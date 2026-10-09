use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use bb_store::{DeferredContent, StoreSnapshot, TimelineEntry, TurnDetailsKey};
use bb_view::glyph::{ResolvedGlyph, work_row_glyph};
use bb_view::markdown_split::{repair_streaming_markdown_tail, split_streaming_markdown};
use bb_view::styled::{fit_spans_to_width, title_spans};
use bb_view::view::{
    SummaryChild, ViewRow, ViewWorkRow, build_timeline_view_rows, build_timeline_view_rows_with,
};
use bb_view::{TimelineTextOptions, TitleOptions, build_timeline_row_title, format_view_rows_text};
use bb_wire::prompt::TurnInitiator;
use bb_wire::timeline::{
    ActiveThinking, ConversationRole, ConversationRow, SystemOperationKind, SystemRow,
    SystemRowKind, TimelineRow, TimelineRowStatus, TimelineWorkRow, TurnRequestKind,
    TurnRequestStatus,
};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use crate::detail::{
    Body, ERROR, MUTED, glyph_char, host_glyph_char, role_style, system_body, system_has_body,
    work_body, work_has_body,
};
use crate::markdown::{bare_urls, render_markdown};
use crate::wrap::{WrapMode, truncate_spans, wrap_spans};

pub const LOAD_OLDER_KEY: &str = "::load-older";
pub const ACTIVITY_KEY: &str = "::activity";
pub const SPINNER: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const MIN_CONTENT_WIDTH: usize = 12;
const THINKING_LINES: usize = 3;
const USER_PREFIX: &str = "❯ ";
const ASSISTANT_PREFIX: &str = "● ";

#[derive(Debug, Clone, PartialEq)]
pub enum DeferredStatus {
    Ready,
    Loading,
    Missing,
    Failed(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum LoadRequest {
    Older,
    Turn(TurnDetailsKey),
    OlderTurn(TurnDetailsKey),
    Deferred(Box<TimelineRow>),
}

impl LoadRequest {
    fn dedupe_key(&self) -> String {
        match self {
            Self::Older => LOAD_OLDER_KEY.to_owned(),
            Self::Turn(key) => format!("turn:{key:?}"),
            Self::OlderTurn(key) => format!("older-turn:{key:?}"),
            Self::Deferred(row) => format!("deferred:{}", row.id().unwrap_or_default()),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RowItem {
    pub row: Arc<ViewRow>,
    pub expandable: bool,
    pub expanded: bool,
    pub streaming: bool,
    pub active_bundle: bool,
    pub deferred: DeferredStatus,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ItemKind {
    LoadOlder {
        loading: bool,
        error: Option<String>,
    },
    Row(RowItem),
    Notice {
        text: String,
        error: bool,
        retry: Option<LoadRequest>,
    },
    LoadMore {
        key: TurnDetailsKey,
        loading: bool,
    },
    Activity {
        label: String,
        thinking: Option<ActiveThinking>,
        started_at: Option<i64>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub key: String,
    pub parent: Option<String>,
    pub depth: usize,
    pub gap: bool,
    pub kind: ItemKind,
}

impl Item {
    pub fn row(&self) -> Option<&RowItem> {
        match &self.kind {
            ItemKind::Row(row) => Some(row),
            _ => None,
        }
    }

    fn is_conversation(&self) -> bool {
        matches!(&self.kind, ItemKind::Row(row) if matches!(*row.row, ViewRow::Conversation(_)))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Scroll {
    #[default]
    Bottom,
    Anchor {
        key: String,
        offset: usize,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placement {
    pub index: usize,
    pub skip: usize,
    pub take: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Frame {
    pub now_ms: i64,
    pub tick: u64,
}

#[derive(Debug)]
struct CacheEntry {
    row: Arc<ViewRow>,
    width: usize,
    variant: (bool, bool, u8),
    body: Arc<Body>,
}

#[derive(Debug, Default)]
struct SettledCache {
    key: String,
    width: usize,
    settled: String,
    body: Body,
}

type TurnRowsCache = HashMap<TurnDetailsKey, (Arc<Vec<TimelineRow>>, Vec<Arc<ViewRow>>)>;

#[derive(Debug, Clone, Default)]
pub struct ThreadActivity {
    pub label: Option<String>,
}

#[derive(Debug, Default)]
pub struct TimelineView {
    pub thread_id: String,
    pub items: Vec<Item>,
    pub selected: Option<String>,
    pub scroll: Scroll,
    pub expanded: HashMap<String, bool>,
    pub unseen: bool,
    pub viewport: Cell<(usize, usize)>,
    snapshot: Arc<StoreSnapshot>,
    activity: ThreadActivity,
    source: Option<Arc<TimelineEntry>>,
    view_rows: Vec<Arc<ViewRow>>,
    turn_rows: TurnRowsCache,
    requested: HashSet<String>,
    cache: RefCell<HashMap<String, CacheEntry>>,
    settled: RefCell<SettledCache>,
}

fn timeline_rows(entry: &TimelineEntry) -> Vec<TimelineRow> {
    entry.rows().iter().map(|row| (**row).clone()).collect()
}

fn row_key(parent: Option<&str>, row: &ViewRow, index: usize) -> String {
    let id = row.id();
    let id = if id.is_empty() {
        format!("#{index}")
    } else {
        id.to_owned()
    };
    match parent {
        Some(parent) => format!("{parent}/{id}"),
        None => id,
    }
}

fn is_interaction_row(row: &TimelineWorkRow) -> bool {
    matches!(
        row,
        TimelineWorkRow::Question(_) | TimelineWorkRow::Approval(_) | TimelineWorkRow::Form(_)
    )
}

fn default_expanded(row: &ViewRow, active_bundle: bool) -> bool {
    match row {
        ViewRow::Summary(_) => active_bundle,
        ViewRow::Turn(turn) => {
            turn.row.status == TimelineRowStatus::Pending && turn.children.is_some()
        }
        ViewRow::Work(work) => {
            is_interaction_row(&work.row) && work.status() == TimelineRowStatus::Pending
        }
        _ => false,
    }
}

fn row_expandable(row: &ViewRow) -> bool {
    match row {
        ViewRow::Conversation(_) | ViewRow::Unknown(_) => false,
        ViewRow::System(system) => system_has_body(system),
        ViewRow::Work(work) => {
            work_has_body(&work.row)
                || work
                    .child_rows
                    .as_ref()
                    .is_some_and(|rows| !rows.is_empty())
        }
        ViewRow::Summary(_) | ViewRow::Turn(_) => true,
    }
}

fn summary_child_row(child: &SummaryChild) -> ViewRow {
    match child {
        SummaryChild::Work(work) => ViewRow::Work(Box::new(work.clone())),
        SummaryChild::Reasoning(system) => ViewRow::System(system.clone()),
    }
}

fn source_row(row: &ViewRow) -> Option<TimelineRow> {
    match row {
        ViewRow::Work(work) => Some(TimelineRow::Work(work.row.clone())),
        ViewRow::System(system) => Some(TimelineRow::System(system.clone())),
        _ => None,
    }
}

fn is_content_deferred(row: &ViewRow) -> bool {
    match row {
        ViewRow::Work(work) => work.row.content_deferred(),
        ViewRow::System(system) => system.content_deferred,
        _ => false,
    }
}

fn with_resolved(row: &ViewRow, resolved: TimelineRow) -> ViewRow {
    match (row, resolved) {
        (ViewRow::Work(work), TimelineRow::Work(resolved)) => {
            ViewRow::Work(Box::new(ViewWorkRow {
                row: resolved,
                child_rows: work.child_rows.clone(),
                in_closed_step: work.in_closed_step,
            }))
        }
        (ViewRow::System(_), TimelineRow::System(resolved)) => ViewRow::System(resolved),
        (row, _) => row.clone(),
    }
}

pub fn busy_label(snapshot: &StoreSnapshot, thread_id: &str) -> Option<String> {
    use bb_wire::thread::ThreadDisplayStatus as Status;
    let status = snapshot
        .sidebar_thread(thread_id)
        .map(|entry| entry.runtime.display_status)
        .or_else(|| {
            snapshot
                .thread(thread_id)
                .map(|thread| thread.runtime.display_status)
        })?;
    let label = match status {
        Status::Active => "Working",
        Status::Starting => "Starting",
        Status::Provisioning => "Provisioning",
        Status::Stopping => "Stopping",
        Status::WaitingForHost => "Waiting for host",
        _ => return None,
    };
    Some(label.to_owned())
}

struct Builder<'a> {
    snapshot: &'a StoreSnapshot,
    thread_id: &'a str,
    expanded: &'a HashMap<String, bool>,
    turn_rows: &'a mut TurnRowsCache,
    items: Vec<Item>,
    loads: Vec<LoadRequest>,
    busy: bool,
}

impl Builder<'_> {
    fn is_expanded(&self, key: &str, default: bool) -> bool {
        self.expanded.get(key).copied().unwrap_or(default)
    }

    fn push(&mut self, key: String, parent: Option<&str>, depth: usize, kind: ItemKind) {
        self.items.push(Item {
            key,
            parent: parent.map(str::to_owned),
            depth,
            gap: false,
            kind,
        });
    }

    fn rows(&mut self, rows: &[Arc<ViewRow>], parent: Option<&str>, depth: usize) {
        let active_bundle = if self.busy && depth == 0 {
            rows.iter()
                .rev()
                .find(|row| !matches!(&***row, ViewRow::Conversation(conversation) if conversation.role == ConversationRole::User))
                .filter(|row| row.is_bundle_summary())
                .map(|row| row.id().to_owned())
        } else {
            None
        };
        for (index, row) in rows.iter().enumerate() {
            let active = row.is_bundle_summary() && active_bundle.as_deref() == Some(row.id());
            self.row(row.clone(), parent, depth, index, active);
        }
    }

    fn resolve_deferred(&mut self, row: &Arc<ViewRow>) -> (Arc<ViewRow>, DeferredStatus) {
        let Some(source) = source_row(row) else {
            return (row.clone(), DeferredStatus::Ready);
        };
        match self.snapshot.resolve_deferred_row(self.thread_id, &source) {
            DeferredContent::NotDeferred => (row.clone(), DeferredStatus::Ready),
            DeferredContent::Loaded(resolved) => (
                Arc::new(with_resolved(row, resolved)),
                DeferredStatus::Ready,
            ),
            DeferredContent::Pending => {
                self.loads.push(LoadRequest::Deferred(Box::new(source)));
                (row.clone(), DeferredStatus::Loading)
            }
            DeferredContent::Missing => (row.clone(), DeferredStatus::Missing),
            DeferredContent::Failed(error) => {
                (row.clone(), DeferredStatus::Failed(error.message.clone()))
            }
        }
    }

    fn row(
        &mut self,
        row: Arc<ViewRow>,
        parent: Option<&str>,
        depth: usize,
        index: usize,
        active_bundle: bool,
    ) {
        let key = row_key(parent, &row, index);
        let expandable = row_expandable(&row);
        let expanded = expandable && self.is_expanded(&key, default_expanded(&row, active_bundle));
        let (row, deferred) = if expanded && is_content_deferred(&row) {
            self.resolve_deferred(&row)
        } else {
            (row, DeferredStatus::Ready)
        };
        self.push(
            key.clone(),
            parent,
            depth,
            ItemKind::Row(RowItem {
                row: row.clone(),
                expandable,
                expanded,
                streaming: false,
                active_bundle,
                deferred,
            }),
        );
        if !expanded {
            return;
        }
        match &*row {
            ViewRow::Summary(summary) => {
                let children: Vec<Arc<ViewRow>> = summary
                    .children
                    .iter()
                    .map(|child| Arc::new(summary_child_row(child)))
                    .collect();
                self.rows(&children, Some(&key), depth + 1);
            }
            ViewRow::Work(work) => {
                if let Some(children) = work.child_rows.as_ref().filter(|rows| !rows.is_empty()) {
                    let children: Vec<Arc<ViewRow>> =
                        children.iter().cloned().map(Arc::new).collect();
                    self.rows(&children, Some(&key), depth + 1);
                }
            }
            ViewRow::Turn(turn) => match &turn.children {
                Some(children) => {
                    let children: Vec<Arc<ViewRow>> =
                        children.iter().cloned().map(Arc::new).collect();
                    self.rows(&children, Some(&key), depth + 1);
                }
                None => self.turn_details(&turn.row, &key, depth + 1),
            },
            _ => {}
        }
    }

    fn turn_details(&mut self, turn: &bb_wire::timeline::TurnRow, key: &str, depth: usize) {
        let Some(turn_id) = turn.base.turn_id.clone() else {
            self.push(
                format!("{key}/::empty"),
                Some(key),
                depth,
                ItemKind::Notice {
                    text: "No details for this turn".into(),
                    error: false,
                    retry: None,
                },
            );
            return;
        };
        let details_key = TurnDetailsKey {
            thread_id: self.thread_id.to_owned(),
            turn_id,
            source_seq_start: turn.base.source_seq_start,
            source_seq_end: turn.base.source_seq_end,
            item_id: None,
        };
        let entry = self.snapshot.turn_details.get(&details_key).cloned();
        let Some(entry) = entry.filter(|entry| entry.loaded) else {
            let failed = self
                .snapshot
                .turn_details
                .get(&details_key)
                .and_then(|entry| entry.error.as_ref())
                .map(|error| error.message.clone());
            let (text, error) = match failed {
                Some(message) => (format!("Failed to load: {message} (Enter to retry)"), true),
                None => {
                    self.loads.push(LoadRequest::Turn(details_key.clone()));
                    ("Loading…".to_owned(), false)
                }
            };
            self.push(
                format!("{key}/::status"),
                Some(key),
                depth,
                ItemKind::Notice {
                    text,
                    error,
                    retry: Some(LoadRequest::Turn(details_key)),
                },
            );
            return;
        };
        if entry.older_cursor.is_some() {
            self.push(
                format!("{key}/::more"),
                Some(key),
                depth,
                ItemKind::LoadMore {
                    key: details_key.clone(),
                    loading: entry.loading_older,
                },
            );
        }
        let rows = match self.turn_rows.get(&details_key) {
            Some((source, rows)) if Arc::ptr_eq(source, &entry.rows) => rows.clone(),
            _ => {
                let rows: Vec<Arc<ViewRow>> = build_timeline_view_rows_with(&entry.rows, true)
                    .into_iter()
                    .map(Arc::new)
                    .collect();
                self.turn_rows
                    .insert(details_key, (entry.rows.clone(), rows.clone()));
                rows
            }
        };
        if rows.is_empty() {
            self.push(
                format!("{key}/::empty"),
                Some(key),
                depth,
                ItemKind::Notice {
                    text: "No steps recorded".into(),
                    error: false,
                    retry: None,
                },
            );
        }
        self.rows(&rows, Some(key), depth);
    }
}

fn streaming_index(items: &[Item]) -> Option<usize> {
    let index = items
        .iter()
        .rposition(|item| !matches!(item.kind, ItemKind::Activity { .. }))?;
    let row = items[index].row()?;
    match &*row.row {
        ViewRow::Conversation(row) if row.role == ConversationRole::Assistant => Some(index),
        _ => None,
    }
}

fn last_user_message_at(items: &[Item]) -> Option<i64> {
    items.iter().rev().find_map(|item| {
        let row = item.row()?;
        match &*row.row {
            ViewRow::Conversation(conversation) if conversation.role == ConversationRole::User => {
                Some(
                    conversation
                        .base
                        .started_at
                        .max(conversation.base.created_at),
                )
            }
            _ => None,
        }
    })
}

fn assign_gaps(items: &mut [Item]) {
    let mut previous_conversation = false;
    let mut first = true;
    for item in items.iter_mut() {
        if item.depth > 0 || matches!(item.kind, ItemKind::LoadOlder { .. }) {
            if item.depth == 0 {
                previous_conversation = false;
            }
            continue;
        }
        let conversation = item.is_conversation();
        item.gap = !first && (conversation || previous_conversation);
        previous_conversation = conversation;
        first = false;
    }
}

impl TimelineView {
    pub fn new(thread_id: impl Into<String>) -> Self {
        Self {
            thread_id: thread_id.into(),
            ..Self::default()
        }
    }

    pub fn entry(&self) -> Option<&TimelineEntry> {
        self.snapshot.timeline(&self.thread_id)
    }

    pub fn is_loading(&self) -> bool {
        self.entry().is_none_or(TimelineEntry::is_loading)
    }

    pub fn error(&self) -> Option<String> {
        let entry = self.entry()?;
        if !entry.rows().is_empty() {
            return None;
        }
        entry.error.as_ref().map(|error| error.message.clone())
    }

    pub fn activity_label(&self) -> Option<&str> {
        self.activity.label.as_deref()
    }

    pub fn sync(&mut self, snapshot: Arc<StoreSnapshot>) -> Vec<LoadRequest> {
        let entry = snapshot.timelines.get(&self.thread_id).cloned();
        let changed = match (&self.source, &entry) {
            (Some(old), Some(new)) => !Arc::ptr_eq(old, new),
            (None, None) => false,
            _ => true,
        };
        if changed {
            self.view_rows = entry
                .as_deref()
                .map(|entry| {
                    build_timeline_view_rows(&timeline_rows(entry))
                        .into_iter()
                        .map(Arc::new)
                        .collect()
                })
                .unwrap_or_default();
            self.source = entry;
        }
        self.activity = ThreadActivity {
            label: busy_label(&snapshot, &self.thread_id),
        };
        self.snapshot = snapshot;
        self.rebuild(true)
    }

    fn rebuild(&mut self, from_snapshot: bool) -> Vec<LoadRequest> {
        let previous_tail = self.items.last().cloned();
        let previous_first_row = match self.items.first().map(|item| &item.kind) {
            Some(ItemKind::LoadOlder { .. }) => self.items.get(1).map(|item| item.key.clone()),
            _ => None,
        };
        let mut builder = Builder {
            snapshot: &self.snapshot,
            thread_id: &self.thread_id,
            expanded: &self.expanded,
            turn_rows: &mut self.turn_rows,
            items: Vec::new(),
            loads: Vec::new(),
            busy: self.activity.label.is_some(),
        };
        let rows = self.view_rows.clone();
        builder.rows(&rows, None, 0);
        let mut items = builder.items;
        let mut loads = builder.loads;
        if let Some(entry) = self.snapshot.timeline(&self.thread_id) {
            if entry.has_older_rows() || entry.loading_older || entry.older_error.is_some() {
                items.insert(
                    0,
                    Item {
                        key: LOAD_OLDER_KEY.into(),
                        parent: None,
                        depth: 0,
                        gap: false,
                        kind: ItemKind::LoadOlder {
                            loading: entry.loading_older,
                            error: entry
                                .older_error
                                .as_ref()
                                .map(|error| error.message.clone()),
                        },
                    },
                );
            }
            let thinking = entry.head().and_then(|head| head.active_thinking.clone());
            let label = match (&thinking, self.activity.label.as_deref()) {
                (Some(_), Some("Working") | None) => Some("Thinking".to_owned()),
                (_, label) => label.map(str::to_owned),
            };
            let background = entry
                .head()
                .map(|head| {
                    head.active_workflows
                        .iter()
                        .chain(&head.active_background_commands)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let activity = match label {
                Some(label) => {
                    if let Some(index) = streaming_index(&items)
                        && let ItemKind::Row(row) = &mut items[index].kind
                    {
                        row.streaming = true;
                    }
                    let started_at = thinking
                        .as_ref()
                        .map(|thinking| thinking.started_at)
                        .or_else(|| last_user_message_at(&items));
                    Some((label, started_at))
                }
                None if !background.is_empty() => {
                    let label = match background.as_slice() {
                        [task] => format!(
                            "Running {}",
                            task.workflow_name
                                .as_deref()
                                .filter(|name| !name.is_empty())
                                .unwrap_or(&task.description)
                        ),
                        tasks => format!("Running {} background tasks", tasks.len()),
                    };
                    let started_at = background.iter().map(|task| task.base.started_at).min();
                    Some((label, started_at))
                }
                None => None,
            };
            if let Some((label, started_at)) = activity {
                items.push(Item {
                    key: ACTIVITY_KEY.into(),
                    parent: None,
                    depth: 0,
                    gap: false,
                    kind: ItemKind::Activity {
                        label,
                        thinking,
                        started_at,
                    },
                });
            }
        }
        assign_gaps(&mut items);
        self.items = items;
        let keys: HashSet<&str> = self.items.iter().map(|item| item.key.as_str()).collect();
        self.cache
            .borrow_mut()
            .retain(|key, _| keys.contains(key.as_str()));
        if let Some(previous) = previous_first_row
            && self
                .items
                .get(1)
                .is_some_and(|item| item.key != previous || self.items[0].key != LOAD_OLDER_KEY)
        {
            if matches!(&self.scroll, Scroll::Anchor { key, .. } if key == LOAD_OLDER_KEY) {
                self.scroll = Scroll::Anchor {
                    key: previous.clone(),
                    offset: 0,
                };
            }
            if self.selected.as_deref() == Some(LOAD_OLDER_KEY) {
                self.selected = Some(previous);
            }
        }
        if let Some(selected) = self.selected.clone() {
            self.selected = self
                .resolve_key(&selected)
                .map(|index| self.items[index].key.clone());
        }
        if let Scroll::Anchor { key, .. } = self.scroll.clone() {
            match self.resolve_key(&key) {
                Some(index) if self.items[index].key == key => {}
                Some(index) => {
                    self.scroll = Scroll::Anchor {
                        key: self.items[index].key.clone(),
                        offset: 0,
                    };
                }
                None => self.scroll = Scroll::Bottom,
            }
        }
        let grew = previous_tail.is_some() && self.items.last() != previous_tail.as_ref();
        if from_snapshot && self.scroll != Scroll::Bottom && grew {
            self.unseen = true;
        }
        if self.scroll == Scroll::Bottom {
            self.unseen = false;
        }
        loads.retain(|load| self.requested.insert(load.dedupe_key()));
        loads.extend(self.auto_load_older());
        loads
    }

    fn resolve_key(&self, key: &str) -> Option<usize> {
        let mut key = key;
        loop {
            if let Some(index) = self.items.iter().position(|item| item.key == key) {
                return Some(index);
            }
            let (parent, _) = key.rsplit_once('/')?;
            key = parent;
        }
    }

    pub fn selected_index(&self) -> Option<usize> {
        let key = self.selected.as_deref()?;
        self.items.iter().position(|item| item.key == key)
    }

    pub fn selected_item(&self) -> Option<&Item> {
        self.selected_index().map(|index| &self.items[index])
    }

    fn content_width(&self, width: usize, depth: usize) -> (usize, usize) {
        let indent = (depth * 2).min(width.saturating_sub(MIN_CONTENT_WIDTH));
        (indent, width.saturating_sub(indent).max(1))
    }

    fn conversation_body(
        &self,
        key: &str,
        row: &ConversationRow,
        width: usize,
        streaming: bool,
    ) -> Body {
        let (prefix, style) = match row.role {
            ConversationRole::User => (
                USER_PREFIX,
                Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            ),
            _ => (ASSISTANT_PREFIX, Style::new().fg(Color::White)),
        };
        let inner = width.saturating_sub(2).max(1);
        let mut body = Body::default();
        if row.role == ConversationRole::User {
            let origin = match (row.initiator, row.sender_thread_id.as_deref()) {
                (Some(TurnInitiator::Agent), Some(sender)) => Some(format!("from thread {sender}")),
                (Some(TurnInitiator::Agent), None) => Some("from an agent".to_owned()),
                (Some(TurnInitiator::System), _) => Some("system message".to_owned()),
                _ => None,
            };
            if let Some(origin) = origin {
                body.lines.push(Line::from(vec![
                    Span::raw("  "),
                    Span::styled(origin, MUTED),
                ]));
            }
        }
        let rendered = if streaming {
            self.streaming_markdown(key, &row.text, inner)
        } else {
            let rendered = render_markdown(&row.text, inner, Style::new());
            Body {
                lines: rendered.lines,
                links: rendered.links,
                paths: Vec::new(),
            }
        };
        let mut first = true;
        for line in rendered.lines {
            let lead = if first {
                Span::styled(prefix, style)
            } else {
                Span::raw("  ")
            };
            first = false;
            let mut spans = vec![lead];
            spans.extend(line.spans);
            body.lines.push(Line::from(spans));
        }
        if first {
            body.lines.push(Line::from(Span::styled(prefix, style)));
        }
        body.links = rendered.links;
        if let Some(attachments) = row.attachments.as_ref() {
            let paths: Vec<&String> = attachments
                .local_file_paths
                .iter()
                .chain(&attachments.local_image_paths)
                .collect();
            let count = paths.len() + attachments.image_urls.len();
            if count > 0 {
                let names: Vec<String> = paths
                    .iter()
                    .map(|path| (*path).clone())
                    .chain(attachments.image_urls.iter().cloned())
                    .collect();
                body.lines.extend(wrap_spans(
                    &[Span::styled(
                        format!("{count} attached: {}", names.join(", ")),
                        MUTED,
                    )],
                    width,
                    &[Span::raw("  ")],
                    &[Span::raw("  ")],
                    WrapMode::Words,
                ));
                body.paths.extend(paths.into_iter().cloned());
                body.links.extend(attachments.image_urls.iter().cloned());
            }
        }
        if let Some(request) = row.turn_request.as_ref()
            && row.role == ConversationRole::User
            && request.kind == TurnRequestKind::Steer
        {
            let (label, style) = match request.status {
                TurnRequestStatus::Pending => ("steer pending", crate::detail::WARNING),
                TurnRequestStatus::Rejected => ("steer failed", ERROR),
                _ => ("steer", MUTED),
            };
            body.lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(label, style),
            ]));
        }
        body
    }

    fn streaming_markdown(&self, key: &str, text: &str, width: usize) -> Body {
        let Some(split) = split_streaming_markdown(text) else {
            let rendered =
                render_markdown(&repair_streaming_markdown_tail(text), width, Style::new());
            return Body {
                lines: rendered.lines,
                links: rendered.links,
                paths: Vec::new(),
            };
        };
        let mut cache = self.settled.borrow_mut();
        if cache.key != key || cache.width != width || cache.settled != split.settled {
            let rendered = render_markdown(split.settled, width, Style::new());
            *cache = SettledCache {
                key: key.to_owned(),
                width,
                settled: split.settled.to_owned(),
                body: Body {
                    lines: rendered.lines,
                    links: rendered.links,
                    paths: Vec::new(),
                },
            };
        }
        let mut body = cache.body.clone();
        let tail = repair_streaming_markdown_tail(split.tail);
        if !tail.trim().is_empty() {
            let rendered = render_markdown(&tail, width, Style::new());
            if !body.lines.is_empty() {
                body.lines.push(Line::default());
            }
            body.lines.extend(rendered.lines);
            body.links.extend(rendered.links);
        }
        body
    }

    fn row_body(&self, item: &Item, row: &RowItem, width: usize) -> Body {
        match &*row.row {
            ViewRow::Conversation(conversation) => {
                self.conversation_body(&item.key, conversation, width, row.streaming)
            }
            _ if !row.expanded => Body::default(),
            _ => {
                let mut body = match &row.deferred {
                    DeferredStatus::Loading => {
                        let mut body = Body::default();
                        body.lines.push(Line::from(Span::styled("Loading…", MUTED)));
                        return self.indent_body(body);
                    }
                    DeferredStatus::Failed(error) => {
                        let mut body = Body::default();
                        body.lines.push(Line::from(Span::styled(
                            format!("Failed to load content: {error}"),
                            ERROR,
                        )));
                        return self.indent_body(body);
                    }
                    DeferredStatus::Missing => {
                        let mut body = Body::default();
                        body.lines
                            .push(Line::from(Span::styled("Content unavailable", MUTED)));
                        return self.indent_body(body);
                    }
                    DeferredStatus::Ready => match &*row.row {
                        ViewRow::Work(work) => work_body(&work.row, width.saturating_sub(2).max(1)),
                        ViewRow::System(system) => {
                            system_body(system, width.saturating_sub(2).max(1))
                        }
                        _ => Body::default(),
                    },
                };
                body = self.indent_body(body);
                body
            }
        }
    }

    fn indent_body(&self, mut body: Body) -> Body {
        for line in &mut body.lines {
            line.spans.insert(0, Span::raw("  "));
        }
        body
    }

    fn body(&self, item: &Item, width: usize) -> Arc<Body> {
        let ItemKind::Row(row) = &item.kind else {
            return Arc::new(Body::default());
        };
        let (_, content) = self.content_width(width, item.depth);
        let variant = (
            row.expanded,
            row.streaming,
            match row.deferred {
                DeferredStatus::Ready => 0,
                DeferredStatus::Loading => 1,
                DeferredStatus::Missing => 2,
                DeferredStatus::Failed(_) => 3,
            },
        );
        if let Some(entry) = self.cache.borrow().get(&item.key)
            && entry.width == content
            && entry.variant == variant
            && (Arc::ptr_eq(&entry.row, &row.row) || entry.row == row.row)
        {
            return entry.body.clone();
        }
        let body = Arc::new(self.row_body(item, row, content));
        self.cache.borrow_mut().insert(
            item.key.clone(),
            CacheEntry {
                row: row.row.clone(),
                width: content,
                variant,
                body: body.clone(),
            },
        );
        body
    }

    fn thinking_lines(&self, thinking: &ActiveThinking, width: usize) -> Vec<Line<'static>> {
        let style = Style::new()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::ITALIC);
        let text = thinking.text.trim();
        if text.is_empty() {
            return Vec::new();
        }
        let tail: Vec<&str> = text
            .lines()
            .filter(|line| !line.trim().is_empty())
            .collect();
        let start = tail.len().saturating_sub(THINKING_LINES);
        let mut lines = Vec::new();
        for line in &tail[start..] {
            lines.extend(wrap_spans(
                &[Span::styled((*line).to_owned(), style)],
                width,
                &[Span::raw("  ")],
                &[Span::raw("  ")],
                WrapMode::Words,
            ));
        }
        let keep = lines.len().saturating_sub(THINKING_LINES);
        lines.split_off(keep)
    }

    pub fn item_height(&self, index: usize, width: usize) -> usize {
        let item = &self.items[index];
        let gap = usize::from(item.gap);
        match &item.kind {
            ItemKind::Row(row) => {
                let header = usize::from(!matches!(*row.row, ViewRow::Conversation(_)));
                gap + header + self.body(item, width).lines.len()
            }
            ItemKind::Activity { thinking, .. } => {
                let (_, content) = self.content_width(width, item.depth);
                gap + 1
                    + thinking
                        .as_ref()
                        .map_or(0, |thinking| self.thinking_lines(thinking, content).len())
            }
            _ => gap + 1,
        }
    }

    fn header(&self, item: &Item, width: usize, frame: Frame) -> Option<Line<'static>> {
        let spinner = SPINNER[(frame.tick as usize) % SPINNER.len()];
        let (glyph, title): (Span<'static>, Vec<Span<'static>>) = match &item.kind {
            ItemKind::Row(row) => {
                if matches!(*row.row, ViewRow::Conversation(_)) {
                    return None;
                }
                let disclosure = if !row.expandable {
                    "  "
                } else if row.expanded {
                    "▾ "
                } else {
                    "▸ "
                };
                let title = build_timeline_row_title(
                    &row.row,
                    TitleOptions {
                        is_active_latest_bundle: row.active_bundle,
                        ..TitleOptions::default()
                    },
                );
                let (indent, _) = self.content_width(width, item.depth);
                let available = width.saturating_sub(indent + 4);
                let spans: Vec<Span<'static>> =
                    fit_spans_to_width(&title_spans(&title, frame.now_ms), available)
                        .iter()
                        .map(|span| Span::styled(span.text.clone(), role_style(span)))
                        .collect();
                let glyph = self.row_glyph(&row.row, spinner);
                let mut prefix = vec![Span::styled(disclosure, MUTED), glyph, Span::raw(" ")];
                prefix.extend(spans);
                (Span::raw(""), prefix)
            }
            ItemKind::LoadOlder { loading, error } => {
                let text = match (loading, error) {
                    (true, _) => "Loading older messages…".to_owned(),
                    (false, Some(error)) => {
                        format!("Failed to load older messages: {error} (Enter to retry)")
                    }
                    (false, None) => "Older messages (Enter to load)".to_owned(),
                };
                let style = if error.is_some() && !loading {
                    ERROR
                } else {
                    MUTED
                };
                (Span::styled("↑ ", MUTED), vec![Span::styled(text, style)])
            }
            ItemKind::LoadMore { loading, .. } => {
                let text = if *loading {
                    "Loading earlier steps…"
                } else {
                    "Earlier steps (Enter to load)"
                };
                (Span::styled("↑ ", MUTED), vec![Span::styled(text, MUTED)])
            }
            ItemKind::Notice { text, error, .. } => (
                Span::raw("  "),
                vec![Span::styled(
                    text.clone(),
                    if *error { ERROR } else { MUTED },
                )],
            ),
            ItemKind::Activity {
                label, started_at, ..
            } => {
                let mut spans = vec![Span::styled(
                    format!("{label}…"),
                    Style::new().fg(Color::LightYellow),
                )];
                if let Some(started_at) = started_at {
                    let elapsed = frame.now_ms - started_at;
                    if elapsed >= 1_000 {
                        spans.push(Span::styled(
                            format!(
                                " {}",
                                bb_view::format::duration_ms_to_compact_string(elapsed)
                            ),
                            MUTED,
                        ));
                    }
                }
                (
                    Span::styled(format!("{spinner} "), Style::new().fg(Color::Yellow)),
                    spans,
                )
            }
        };
        let (indent, _) = self.content_width(width, item.depth);
        let mut spans = vec![Span::raw(" ".repeat(indent))];
        if !glyph.content.is_empty() {
            spans.push(glyph);
        }
        spans.extend(title);
        Some(Line::from(truncate_spans(spans, width)))
    }

    fn row_glyph(&self, row: &ViewRow, spinner: &'static str) -> Span<'static> {
        let pending = Style::new().fg(Color::Yellow);
        let status = match row {
            ViewRow::Work(work) => work.status(),
            ViewRow::Summary(summary) => summary.status,
            ViewRow::Turn(turn) => turn.row.status,
            ViewRow::System(system) => system.status.unwrap_or(TimelineRowStatus::Completed),
            _ => TimelineRowStatus::Completed,
        };
        if status == TimelineRowStatus::Pending {
            return Span::styled(spinner, pending);
        }
        let style = match status {
            TimelineRowStatus::Error => ERROR,
            TimelineRowStatus::Interrupted => crate::detail::WARNING,
            _ => MUTED,
        };
        let glyph = match row {
            ViewRow::Work(work) => {
                match work_row_glyph(&work.row, |name| host_glyph_char(name).is_some()) {
                    ResolvedGlyph::Builtin(glyph) => glyph_char(glyph),
                    ResolvedGlyph::Host(name) => host_glyph_char(&name).unwrap_or("◈"),
                }
            }
            ViewRow::Summary(_) => "≡",
            ViewRow::Turn(_) => "◦",
            ViewRow::System(system) => system_glyph(system),
            _ => "?",
        };
        let style = match row {
            ViewRow::System(system) if system.system_kind == SystemRowKind::Error => ERROR,
            _ => style,
        };
        Span::styled(glyph, style)
    }

    pub fn item_lines(&self, index: usize, width: usize, frame: Frame) -> Vec<Line<'static>> {
        let item = &self.items[index];
        let mut lines = Vec::new();
        if item.gap {
            lines.push(Line::default());
        }
        if let Some(header) = self.header(item, width, frame) {
            lines.push(header);
        }
        match &item.kind {
            ItemKind::Row(_) => {
                let (indent, _) = self.content_width(width, item.depth);
                let body = self.body(item, width);
                let pad = " ".repeat(indent);
                for line in body.lines.iter() {
                    if indent == 0 {
                        lines.push(line.clone());
                    } else {
                        let mut spans = vec![Span::raw(pad.clone())];
                        spans.extend(line.spans.iter().cloned());
                        lines.push(Line::from(spans));
                    }
                }
            }
            ItemKind::Activity {
                thinking: Some(thinking),
                ..
            } => {
                let (_, content) = self.content_width(width, item.depth);
                lines.extend(self.thinking_lines(thinking, content));
            }
            _ => {}
        }
        lines
    }

    fn bottom_top(&self, width: usize, height: usize) -> (usize, usize) {
        let mut used = 0;
        for index in (0..self.items.len()).rev() {
            let item_height = self.item_height(index, width);
            if used + item_height >= height {
                return (index, used + item_height - height);
            }
            used += item_height;
        }
        (0, 0)
    }

    fn fills_from(&self, index: usize, skip: usize, width: usize, height: usize) -> bool {
        let mut used = 0usize;
        for current in index..self.items.len() {
            let item_height = self.item_height(current, width);
            used += if current == index {
                item_height.saturating_sub(skip)
            } else {
                item_height
            };
            if used >= height {
                return true;
            }
        }
        false
    }

    pub fn top_position(&self, width: usize, height: usize) -> (usize, usize) {
        if self.items.is_empty() {
            return (0, 0);
        }
        match &self.scroll {
            Scroll::Bottom => self.bottom_top(width, height),
            Scroll::Anchor { key, offset } => {
                let Some(index) = self.items.iter().position(|item| &item.key == key) else {
                    return self.bottom_top(width, height);
                };
                let skip = (*offset).min(self.item_height(index, width).saturating_sub(1));
                if self.fills_from(index, skip, width, height) {
                    (index, skip)
                } else {
                    self.bottom_top(width, height)
                }
            }
        }
    }

    pub fn layout(&self, width: usize, height: usize) -> Vec<Placement> {
        let mut placements = Vec::new();
        if self.items.is_empty() || height == 0 {
            return placements;
        }
        let (mut index, mut skip) = self.top_position(width, height);
        let mut remaining = height;
        while remaining > 0 && index < self.items.len() {
            let item_height = self.item_height(index, width);
            let take = item_height.saturating_sub(skip).min(remaining);
            if take > 0 {
                placements.push(Placement { index, skip, take });
            }
            remaining -= take;
            index += 1;
            skip = 0;
        }
        placements
    }

    fn set_position(&mut self, index: usize, skip: usize) {
        let (width, height) = self.viewport_size();
        if self.items.is_empty() || !self.fills_from(index, skip, width, height) || {
            let bottom = self.bottom_top(width, height);
            (index, skip) >= bottom
        } {
            self.scroll = Scroll::Bottom;
            self.unseen = false;
            return;
        }
        self.scroll = Scroll::Anchor {
            key: self.items[index].key.clone(),
            offset: skip,
        };
    }

    pub fn viewport_size(&self) -> (usize, usize) {
        let (width, height) = self.viewport.get();
        if width == 0 || height == 0 {
            (80, 24)
        } else {
            (width, height)
        }
    }

    pub fn at_top(&self) -> bool {
        let (width, height) = self.viewport_size();
        self.top_position(width, height) == (0, 0)
    }

    pub fn auto_load_older(&mut self) -> Vec<LoadRequest> {
        let Some(entry) = self.entry() else {
            return Vec::new();
        };
        if !entry.has_older_rows() || entry.loading_older || entry.older_error.is_some() {
            return Vec::new();
        }
        if self.at_top() {
            return vec![LoadRequest::Older];
        }
        Vec::new()
    }

    pub fn scroll_by(&mut self, delta: isize) -> Vec<LoadRequest> {
        let (width, height) = self.viewport_size();
        if self.items.is_empty() {
            return Vec::new();
        }
        let (mut index, mut skip) = self.top_position(width, height);
        let mut remaining = delta.unsigned_abs();
        if delta < 0 {
            while remaining > 0 {
                if skip > 0 {
                    let step = skip.min(remaining);
                    skip -= step;
                    remaining -= step;
                    continue;
                }
                if index == 0 {
                    break;
                }
                index -= 1;
                skip = self.item_height(index, width);
            }
        } else {
            while remaining > 0 && index < self.items.len() {
                let item_height = self.item_height(index, width);
                if skip + remaining < item_height {
                    skip += remaining;
                    remaining = 0;
                } else {
                    remaining -= item_height - skip;
                    index += 1;
                    skip = 0;
                }
            }
            if index >= self.items.len() {
                self.scroll = Scroll::Bottom;
                self.unseen = false;
                return Vec::new();
            }
        }
        self.set_position(index, skip);
        self.auto_load_older()
    }

    pub fn page(&mut self, direction: isize, half: bool) -> Vec<LoadRequest> {
        let (_, height) = self.viewport_size();
        let amount = if half {
            (height / 2).max(1)
        } else {
            height.saturating_sub(2).max(1)
        };
        self.scroll_by(direction * amount as isize)
    }

    pub fn ensure_visible(&mut self, index: usize) {
        let (width, height) = self.viewport_size();
        let (top_index, top_skip) = self.top_position(width, height);
        let item_height = self.item_height(index, width);
        let gap = usize::from(self.items[index].gap);
        if index < top_index || (index == top_index && top_skip > gap) {
            self.set_position(index, gap.min(item_height.saturating_sub(1)));
            return;
        }
        let mut used = self.item_height(top_index, width).saturating_sub(top_skip);
        for current in top_index + 1..=index {
            used += self.item_height(current, width);
        }
        if used <= height {
            return;
        }
        if item_height >= height {
            self.set_position(index, gap.min(item_height.saturating_sub(1)));
            return;
        }
        let mut used = 0;
        let mut position = (index, 0);
        for current in (0..=index).rev() {
            let current_height = self.item_height(current, width);
            if used + current_height >= height {
                position = (current, used + current_height - height);
                break;
            }
            used += current_height;
        }
        self.set_position(position.0, position.1);
    }

    pub fn move_selection(&mut self, delta: isize) -> Vec<LoadRequest> {
        if self.items.is_empty() {
            return Vec::new();
        }
        let current = match self.selected_index() {
            Some(index) => index,
            None => {
                let (width, height) = self.viewport_size();
                let placements = self.layout(width, height);
                let index = if delta < 0 {
                    placements.last().map_or(self.items.len() - 1, |p| p.index)
                } else {
                    placements.first().map_or(0, |p| p.index)
                };
                self.selected = Some(self.items[index].key.clone());
                self.ensure_visible(index);
                return self.auto_load_older();
            }
        };
        let next = current
            .saturating_add_signed(delta)
            .min(self.items.len() - 1);
        self.selected = Some(self.items[next].key.clone());
        self.ensure_visible(next);
        if next == self.items.len() - 1 {
            self.scroll = Scroll::Bottom;
            self.unseen = false;
        }
        self.auto_load_older()
    }

    pub fn select_edge(&mut self, top: bool) -> Vec<LoadRequest> {
        if self.items.is_empty() {
            return Vec::new();
        }
        if top {
            self.selected = Some(self.items[0].key.clone());
            self.set_position(0, 0);
            self.auto_load_older()
        } else {
            self.selected = Some(self.items[self.items.len() - 1].key.clone());
            self.scroll = Scroll::Bottom;
            self.unseen = false;
            Vec::new()
        }
    }

    pub fn jump_message(&mut self, forward: bool) -> Vec<LoadRequest> {
        let start = self.selected_index();
        let is_user = |item: &Item| {
            item.depth == 0
                && matches!(&item.kind, ItemKind::Row(row) if matches!(&*row.row, ViewRow::Conversation(c) if c.role == ConversationRole::User))
        };
        let found = if forward {
            let from = start.map_or(0, |index| index + 1);
            (from..self.items.len()).find(|index| is_user(&self.items[*index]))
        } else {
            let to = start.unwrap_or(self.items.len());
            (0..to).rev().find(|index| is_user(&self.items[*index]))
        };
        match found {
            Some(index) => {
                self.selected = Some(self.items[index].key.clone());
                let gap = usize::from(self.items[index].gap);
                self.set_position(index, gap);
                self.auto_load_older()
            }
            None if forward => self.select_edge(false),
            None => self.select_edge(true),
        }
    }

    pub fn set_expanded(&mut self, expand: Option<bool>) -> Vec<LoadRequest> {
        let Some(index) = self.selected_index() else {
            return Vec::new();
        };
        let item = self.items[index].clone();
        let ItemKind::Row(row) = &item.kind else {
            return Vec::new();
        };
        let target = expand.unwrap_or(!row.expanded);
        if !row.expandable || target == row.expanded {
            if expand == Some(false)
                && let Some(parent) = item.parent.clone()
            {
                self.selected = Some(parent);
                if let Some(index) = self.selected_index() {
                    self.ensure_visible(index);
                }
            }
            return Vec::new();
        }
        self.expanded.insert(item.key.clone(), target);
        let mut loads = self.rebuild(false);
        if let Some(index) = self.selected_index() {
            self.ensure_visible(index);
        }
        loads.extend(self.auto_load_older());
        loads
    }

    pub fn activate(&mut self) -> Vec<LoadRequest> {
        let Some(item) = self.selected_item().cloned() else {
            return Vec::new();
        };
        match item.kind {
            ItemKind::LoadOlder { loading: false, .. } => vec![LoadRequest::Older],
            ItemKind::LoadMore {
                key,
                loading: false,
            } => vec![LoadRequest::OlderTurn(key)],
            ItemKind::Notice {
                retry: Some(retry),
                error: true,
                ..
            } => vec![retry],
            ItemKind::Row(_) => self.set_expanded(None),
            _ => Vec::new(),
        }
    }

    pub fn copy_text(&self) -> Option<String> {
        let item = self.selected_item()?;
        let row = item.row()?;
        match &*row.row {
            ViewRow::Conversation(conversation) => Some(conversation.text.clone()),
            other => Some(format_view_rows_text(
                std::slice::from_ref(other),
                TimelineTextOptions {
                    verbose: true,
                    color: false,
                },
            )),
        }
    }

    pub fn open_targets(&self, workspace: Option<&str>) -> Vec<String> {
        let Some(item) = self.selected_item() else {
            return Vec::new();
        };
        let Some(row) = item.row() else {
            return Vec::new();
        };
        let (width, _) = self.viewport_size();
        let body = match &*row.row {
            ViewRow::Conversation(_) => (*self.body(item, width)).clone(),
            ViewRow::Work(work) => work_body(&work.row, width),
            ViewRow::System(system) => system_body(system, width),
            _ => Body::default(),
        };
        let mut targets: Vec<String> = Vec::new();
        for path in &body.paths {
            let resolved = resolve_path(path, workspace);
            if !targets.contains(&resolved) {
                targets.push(resolved);
            }
        }
        for link in body.links.iter().cloned().chain(match &*row.row {
            ViewRow::Conversation(conversation) => bare_urls(&conversation.text),
            _ => Vec::new(),
        }) {
            if !targets.contains(&link) {
                targets.push(link);
            }
        }
        targets
    }

    pub fn cached_rows(&self) -> usize {
        self.cache.borrow().len()
    }

    pub fn has_rows(&self) -> bool {
        self.items
            .iter()
            .any(|item| matches!(item.kind, ItemKind::Row(_)))
    }
}

fn system_glyph(system: &SystemRow) -> &'static str {
    match (system.system_kind, system.operation_kind) {
        (SystemRowKind::Error, _) => "✗",
        (SystemRowKind::Reconnect, _) => "↻",
        (_, Some(SystemOperationKind::Reasoning)) => "✻",
        (_, Some(SystemOperationKind::Compaction | SystemOperationKind::ContextClear)) => "⇣",
        (_, Some(SystemOperationKind::ParentChange)) => "↳",
        (_, Some(SystemOperationKind::Warning | SystemOperationKind::Deprecation)) => "!",
        (_, Some(SystemOperationKind::ThreadInterrupted)) => "■",
        _ => "·",
    }
}

pub fn resolve_path(path: &str, workspace: Option<&str>) -> String {
    if path.starts_with('/') || path.starts_with("~/") || path.contains("://") {
        return path.to_owned();
    }
    match workspace {
        Some(root) => format!(
            "{}/{}",
            root.trim_end_matches('/'),
            path.trim_start_matches("./")
        ),
        None => path.to_owned(),
    }
}
