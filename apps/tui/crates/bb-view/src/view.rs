use std::collections::HashSet;

use bb_wire::JsonValue;
use bb_wire::timeline::{
    ActivityIntent, ConversationRole, ConversationRow, SystemOperationKind, SystemRow,
    SystemRowKind, TimelineApprovalStatus, TimelineRow, TimelineRowBase, TimelineRowStatus,
    TimelineWorkRow, TurnRequestStatus, TurnRow,
};

use crate::activity_intents::{
    ExplorationRow, activity_intent_dedupe_key, has_timeline_exploration_intent, intent_type,
    timeline_row_activity_intents, work_row_has_exploration_intent,
};
use crate::file_change::{FileChangeAction, file_change_action, file_change_identity};
use crate::format::{lower_first, plural, plural_with};
use crate::shell::parse_sent_thread_message;

static EMPTY_BASE: TimelineRowBase = TimelineRowBase {
    id: String::new(),
    thread_id: String::new(),
    turn_id: None,
    source_seq_start: 0,
    source_seq_end: 0,
    started_at: 0,
    created_at: 0,
};

#[derive(Debug, Clone, PartialEq)]
pub struct ViewWorkRow {
    pub row: TimelineWorkRow,
    pub child_rows: Option<Vec<ViewRow>>,
    pub in_closed_step: bool,
}

impl ViewWorkRow {
    pub fn base(&self) -> &TimelineRowBase {
        self.row.base().unwrap_or(&EMPTY_BASE)
    }

    pub fn id(&self) -> &str {
        match &self.row {
            TimelineWorkRow::Unknown(value) => value
                .get("id")
                .and_then(JsonValue::as_str)
                .unwrap_or_default(),
            _ => &self.base().id,
        }
    }

    pub fn status(&self) -> TimelineRowStatus {
        self.row.status()
    }

    pub fn exploration(&self) -> Option<ExplorationRow<'_>> {
        ExplorationRow::from_work(&self.row)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum SummaryChild {
    Work(ViewWorkRow),
    Reasoning(Box<SystemRow>),
}

impl SummaryChild {
    pub fn base(&self) -> &TimelineRowBase {
        match self {
            Self::Work(row) => row.base(),
            Self::Reasoning(row) => &row.base,
        }
    }

    pub fn id(&self) -> &str {
        match self {
            Self::Work(row) => row.id(),
            Self::Reasoning(row) => &row.base.id,
        }
    }

    pub fn status(&self) -> TimelineRowStatus {
        match self {
            Self::Work(row) => row.status(),
            Self::Reasoning(row) => row.status.unwrap_or(TimelineRowStatus::Completed),
        }
    }

    pub fn work(&self) -> Option<&ViewWorkRow> {
        match self {
            Self::Work(row) => Some(row),
            Self::Reasoning(_) => None,
        }
    }

    fn into_view_row(self) -> ViewRow {
        match self {
            Self::Work(row) => ViewRow::Work(Box::new(row)),
            Self::Reasoning(row) => ViewRow::System(row),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SummaryKind {
    Step,
    Bundle,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WorkSummaryRow {
    pub kind: SummaryKind,
    pub base: TimelineRowBase,
    pub status: TimelineRowStatus,
    pub children: Vec<SummaryChild>,
}

impl WorkSummaryRow {
    pub fn work_children(&self) -> impl Iterator<Item = &ViewWorkRow> {
        self.children.iter().filter_map(SummaryChild::work)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ViewTurnRow {
    pub row: TurnRow,
    pub children: Option<Vec<ViewRow>>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ViewRow {
    Conversation(Box<ConversationRow>),
    System(Box<SystemRow>),
    Work(Box<ViewWorkRow>),
    Summary(Box<WorkSummaryRow>),
    Turn(Box<ViewTurnRow>),
    Unknown(JsonValue),
}

impl ViewRow {
    pub fn base(&self) -> Option<&TimelineRowBase> {
        match self {
            Self::Conversation(row) => Some(&row.base),
            Self::System(row) => Some(&row.base),
            Self::Work(row) => row.row.base(),
            Self::Summary(row) => Some(&row.base),
            Self::Turn(row) => Some(&row.row.base),
            Self::Unknown(_) => None,
        }
    }

    pub fn id(&self) -> &str {
        match self {
            Self::Work(row) => row.id(),
            Self::Unknown(value) => value
                .get("id")
                .and_then(JsonValue::as_str)
                .unwrap_or_default(),
            _ => self.base().map(|base| base.id.as_str()).unwrap_or_default(),
        }
    }

    pub fn is_bundle_summary(&self) -> bool {
        matches!(self, Self::Summary(row) if row.kind == SummaryKind::Bundle)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum ExplorationKind {
    Files,
    Searches,
    Lists,
}

#[derive(Debug, Default)]
struct WorkSummaryCounts {
    commands: usize,
    created_files: usize,
    deleted_files: usize,
    delegations: usize,
    edited_files: usize,
    extensions: usize,
    files: usize,
    lists: usize,
    plan_updates: usize,
    renamed_files: usize,
    searches: usize,
    tools: usize,
    web_fetches: usize,
    web_searches: usize,
    image_generations: usize,
    image_views: usize,
    exploration_kind_order: Vec<ExplorationKind>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SummaryCategory {
    Commands,
    Delegations,
    Exploration,
    Extensions,
    FileChanges,
    ImageGenerations,
    ImageViews,
    PlanUpdates,
    Tools,
    WebResearch,
}

fn note_exploration_kind(counts: &mut WorkSummaryCounts, kind: ExplorationKind) {
    if !counts.exploration_kind_order.contains(&kind) {
        counts.exploration_kind_order.push(kind);
    }
}

fn count_exploration_intents(
    row: ExplorationRow<'_>,
    counts: &mut WorkSummaryCounts,
    explored_files: &mut HashSet<String>,
) {
    for intent in timeline_row_activity_intents(row).iter() {
        match intent {
            ActivityIntent::Read { path, name, .. } => {
                let identity = path.as_deref().unwrap_or(name);
                if !identity.is_empty() {
                    explored_files.insert(identity.to_owned());
                    note_exploration_kind(counts, ExplorationKind::Files);
                }
            }
            ActivityIntent::ListFiles { .. } => {
                counts.lists += 1;
                note_exploration_kind(counts, ExplorationKind::Lists);
            }
            ActivityIntent::Search { .. } => {
                counts.searches += 1;
                note_exploration_kind(counts, ExplorationKind::Searches);
            }
            ActivityIntent::Generic { .. } | ActivityIntent::Unknown(_) => {}
        }
    }
}

fn summarize_timeline_work<'a>(rows: impl Iterator<Item = &'a ViewWorkRow>) -> WorkSummaryCounts {
    let mut counts = WorkSummaryCounts::default();
    let mut explored = HashSet::new();
    let mut created = HashSet::new();
    let mut deleted = HashSet::new();
    let mut edited = HashSet::new();
    let mut renamed = HashSet::new();
    for view in rows {
        match &view.row {
            TimelineWorkRow::Command(row) => {
                let exploration = ExplorationRow::Command(row);
                if has_timeline_exploration_intent(exploration) {
                    count_exploration_intents(exploration, &mut counts, &mut explored);
                } else {
                    counts.commands += 1;
                }
            }
            TimelineWorkRow::Tool(_) => counts.tools += 1,
            TimelineWorkRow::FileRead(row) => {
                count_exploration_intents(ExplorationRow::FileRead(row), &mut counts, &mut explored)
            }
            TimelineWorkRow::Search(row) => {
                count_exploration_intents(ExplorationRow::Search(row), &mut counts, &mut explored)
            }
            TimelineWorkRow::PlanSteps(_) => counts.plan_updates += 1,
            TimelineWorkRow::Extension(_) => counts.extensions += 1,
            TimelineWorkRow::FileChange(row) => {
                let identity = file_change_identity(&row.change).to_owned();
                match file_change_action(&row.change) {
                    FileChangeAction::Created => created.insert(identity),
                    FileChangeAction::Deleted => deleted.insert(identity),
                    FileChangeAction::Edited => edited.insert(identity),
                    FileChangeAction::Renamed => renamed.insert(identity),
                };
            }
            TimelineWorkRow::WebFetch(_) => counts.web_fetches += 1,
            TimelineWorkRow::WebSearch(row) => counts.web_searches += row.queries.len().max(1),
            TimelineWorkRow::ImageGeneration(_) => counts.image_generations += 1,
            TimelineWorkRow::ImageView(_) => counts.image_views += 1,
            TimelineWorkRow::Delegation(_) => counts.delegations += 1,
            TimelineWorkRow::Question(_)
            | TimelineWorkRow::Form(_)
            | TimelineWorkRow::Approval(_)
            | TimelineWorkRow::Workflow(_)
            | TimelineWorkRow::Unknown(_) => {}
        }
    }
    counts.files = explored.len();
    counts.created_files = created.len();
    counts.deleted_files = deleted.len();
    counts.edited_files = edited.len();
    counts.renamed_files = renamed.len();
    counts
}

fn exploration_detail(counts: &WorkSummaryCounts) -> Option<String> {
    let parts: Vec<String> = counts
        .exploration_kind_order
        .iter()
        .filter_map(|kind| match kind {
            ExplorationKind::Files => (counts.files > 0).then(|| plural(counts.files, "file")),
            ExplorationKind::Searches => (counts.searches > 0)
                .then(|| plural_with(counts.searches, "search", Some("searches"))),
            ExplorationKind::Lists => (counts.lists > 0).then(|| plural(counts.lists, "list")),
        })
        .collect();
    (!parts.is_empty()).then(|| parts.join(", "))
}

fn approval_status_summary_label<'a>(
    rows: impl Iterator<Item = &'a ViewWorkRow>,
) -> Option<String> {
    let mut status: Option<TimelineApprovalStatus> = None;
    let mut commands = 0;
    let mut file_changes = 0;
    let mut tools = 0;
    for view in rows {
        let row_status = match &view.row {
            TimelineWorkRow::Command(row) => {
                commands += 1;
                row.approval_status
            }
            TimelineWorkRow::FileChange(row) => {
                file_changes += 1;
                row.approval_status
            }
            TimelineWorkRow::Tool(row) => {
                tools += 1;
                row.approval_status
            }
            _ => return None,
        };
        let row_status = row_status.filter(|status| status.is_known())?;
        match status {
            None => status = Some(row_status),
            Some(existing) if existing != row_status => return None,
            Some(_) => {}
        }
    }
    let status = status?;
    let details: Vec<String> = [
        (commands > 0).then(|| plural(commands, "command")),
        (file_changes > 0).then(|| plural(file_changes, "file change")),
        (tools > 0).then(|| plural(tools, "tool")),
    ]
    .into_iter()
    .flatten()
    .collect();
    let detail = details.join(", ");
    Some(if status == TimelineApprovalStatus::Denied {
        format!("Denied {detail}")
    } else {
        format!("Waiting for approval on {detail}")
    })
}

fn summary_category(row: &TimelineWorkRow) -> Option<SummaryCategory> {
    Some(match row {
        TimelineWorkRow::Command(_) if work_row_has_exploration_intent(row) => {
            SummaryCategory::Exploration
        }
        TimelineWorkRow::Command(_) => SummaryCategory::Commands,
        TimelineWorkRow::Tool(_) => SummaryCategory::Tools,
        TimelineWorkRow::FileRead(_) | TimelineWorkRow::Search(_) => SummaryCategory::Exploration,
        TimelineWorkRow::PlanSteps(_) => SummaryCategory::PlanUpdates,
        TimelineWorkRow::Extension(_) => SummaryCategory::Extensions,
        TimelineWorkRow::FileChange(_) => SummaryCategory::FileChanges,
        TimelineWorkRow::WebFetch(_) | TimelineWorkRow::WebSearch(_) => {
            SummaryCategory::WebResearch
        }
        TimelineWorkRow::ImageGeneration(_) => SummaryCategory::ImageGenerations,
        TimelineWorkRow::ImageView(_) => SummaryCategory::ImageViews,
        TimelineWorkRow::Delegation(_) => SummaryCategory::Delegations,
        TimelineWorkRow::Approval(_)
        | TimelineWorkRow::Question(_)
        | TimelineWorkRow::Form(_)
        | TimelineWorkRow::Workflow(_)
        | TimelineWorkRow::Unknown(_) => return None,
    })
}

fn row_concept(row: &TimelineWorkRow) -> SummaryCategory {
    summary_category(row).unwrap_or(SummaryCategory::Tools)
}

fn ordered_summary_categories<'a>(
    rows: impl Iterator<Item = &'a ViewWorkRow>,
) -> Vec<SummaryCategory> {
    let mut categories = Vec::new();
    for row in rows {
        if let Some(category) = summary_category(&row.row)
            && !categories.contains(&category)
        {
            categories.push(category);
        }
    }
    categories
}

fn file_change_summary_phrase(counts: &WorkSummaryCounts, active: bool) -> Option<String> {
    let present: Vec<(FileChangeAction, usize)> = [
        (FileChangeAction::Created, counts.created_files),
        (FileChangeAction::Deleted, counts.deleted_files),
        (FileChangeAction::Edited, counts.edited_files),
        (FileChangeAction::Renamed, counts.renamed_files),
    ]
    .into_iter()
    .filter(|(_, count)| *count > 0)
    .collect();
    match present.as_slice() {
        [] => None,
        [(action, count)] => {
            let verb = if active {
                action.present_tense()
            } else {
                action.past_tense()
            };
            Some(format!("{verb} {}", plural(*count, "file")))
        }
        _ => {
            let total = present.iter().map(|(_, count)| count).sum();
            let verb = if active { "Editing" } else { "Edited" };
            Some(format!("{verb} {}", plural(total, "file")))
        }
    }
}

fn web_research_summary_phrase(counts: &WorkSummaryCounts, active: bool) -> Option<String> {
    let mut parts = Vec::new();
    if counts.web_searches > 0 {
        parts.push(plural_with(
            counts.web_searches,
            "search query",
            Some("search queries"),
        ));
    }
    if counts.web_fetches > 0 {
        parts.push(plural(counts.web_fetches, "web page"));
    }
    if parts.is_empty() {
        return None;
    }
    let verb = if active { "Researching" } else { "Researched" };
    Some(format!("{verb} {}", parts.join(", ")))
}

fn counted_phrase(count: usize, verb: &str, noun: &str) -> Option<String> {
    (count > 0).then(|| format!("{verb} {}", plural(count, noun)))
}

fn summary_phrase(
    category: SummaryCategory,
    counts: &WorkSummaryCounts,
    exploration: Option<&str>,
    active: bool,
) -> Option<String> {
    let run = if active { "Running" } else { "Ran" };
    match category {
        SummaryCategory::Exploration => exploration
            .map(|detail| format!("{} {detail}", if active { "Exploring" } else { "Explored" })),
        SummaryCategory::Commands => counted_phrase(counts.commands, run, "command"),
        SummaryCategory::FileChanges => file_change_summary_phrase(counts, active),
        SummaryCategory::WebResearch => web_research_summary_phrase(counts, active),
        SummaryCategory::ImageGenerations => counted_phrase(
            counts.image_generations,
            if active { "Generating" } else { "Generated" },
            "image",
        ),
        SummaryCategory::ImageViews => counted_phrase(
            counts.image_views,
            if active { "Viewing" } else { "Viewed" },
            "image",
        ),
        SummaryCategory::Delegations => counted_phrase(counts.delegations, run, "subagent"),
        SummaryCategory::Tools => counted_phrase(counts.tools, run, "tool"),
        SummaryCategory::PlanUpdates => (counts.plan_updates > 0).then(|| {
            if active {
                "Updating plan".to_owned()
            } else {
                format!("Updated plan {}", plural(counts.plan_updates, "time"))
            }
        }),
        SummaryCategory::Extensions => counted_phrase(counts.extensions, run, "plugin step"),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SummaryLabelParts {
    pub verb: String,
    pub rest: String,
}

fn split_verb_and_rest(label: &str) -> SummaryLabelParts {
    match label.split_once(' ') {
        Some((verb, rest)) => SummaryLabelParts {
            verb: verb.to_owned(),
            rest: rest.to_owned(),
        },
        None => SummaryLabelParts {
            verb: label.to_owned(),
            rest: String::new(),
        },
    }
}

pub fn build_timeline_work_summary_label_parts(
    row: &WorkSummaryRow,
    active: bool,
) -> SummaryLabelParts {
    if let Some(label) = approval_status_summary_label(row.work_children()) {
        return split_verb_and_rest(&label);
    }
    let counts = summarize_timeline_work(row.work_children());
    let exploration = exploration_detail(&counts);
    let phrases: Vec<String> = ordered_summary_categories(row.work_children())
        .into_iter()
        .filter_map(|category| summary_phrase(category, &counts, exploration.as_deref(), active))
        .collect();
    if phrases.is_empty() {
        return SummaryLabelParts {
            verb: if active { "Working" } else { "Worked" }.to_owned(),
            rest: String::new(),
        };
    }
    let joined = phrases
        .iter()
        .enumerate()
        .map(|(index, phrase)| {
            if index == 0 {
                phrase.clone()
            } else {
                lower_first(phrase)
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    split_verb_and_rest(&joined)
}

pub fn build_timeline_work_summary_label(row: &WorkSummaryRow, active: bool) -> String {
    let parts = build_timeline_work_summary_label_parts(row, active);
    if parts.rest.is_empty() {
        parts.verb
    } else {
        format!("{} {}", parts.verb, parts.rest)
    }
}

pub fn merge_timeline_status(
    left: TimelineRowStatus,
    right: TimelineRowStatus,
) -> TimelineRowStatus {
    use TimelineRowStatus::{Completed, Error, Interrupted, Pending};
    if left == Error || right == Error {
        Error
    } else if left == Pending || right == Pending {
        Pending
    } else if left == Interrupted || right == Interrupted {
        Interrupted
    } else {
        Completed
    }
}

fn summarize_range(children: &[SummaryChild]) -> (TimelineRowBase, TimelineRowStatus) {
    let first = children[0].base();
    let mut base = TimelineRowBase {
        id: String::new(),
        thread_id: first.thread_id.clone(),
        turn_id: first.turn_id.clone(),
        source_seq_start: first.source_seq_start,
        source_seq_end: first.source_seq_end,
        started_at: first.started_at,
        created_at: first.created_at,
    };
    let mut status = children[0].status();
    for child in children {
        let child_base = child.base();
        base.source_seq_start = base.source_seq_start.min(child_base.source_seq_start);
        base.source_seq_end = base.source_seq_end.max(child_base.source_seq_end);
        base.started_at = base.started_at.min(child_base.started_at);
        base.created_at = base.created_at.max(child_base.created_at);
        status = merge_timeline_status(status, child.status());
        if base.turn_id != child_base.turn_id {
            base.turn_id = None;
        }
    }
    base.id = [
        first.thread_id.as_str(),
        base.turn_id.as_deref().unwrap_or("thread"),
        "work-summary",
        children[0].id(),
    ]
    .join(":");
    (base, status)
}

fn is_completed_reasoning(row: &SystemRow) -> bool {
    row.system_kind == SystemRowKind::Operation
        && row.operation_kind == Some(SystemOperationKind::Reasoning)
        && row.status == Some(TimelineRowStatus::Completed)
}

fn is_summarizable_work(row: &TimelineWorkRow) -> bool {
    match row {
        TimelineWorkRow::Approval(_)
        | TimelineWorkRow::Question(_)
        | TimelineWorkRow::Workflow(_)
        | TimelineWorkRow::Unknown(_) => false,
        TimelineWorkRow::Command(command) => parse_sent_thread_message(command).is_none(),
        _ => true,
    }
}

fn into_summary_child(row: ViewRow) -> Result<SummaryChild, ViewRow> {
    match row {
        ViewRow::System(system) if is_completed_reasoning(&system) => {
            Ok(SummaryChild::Reasoning(system))
        }
        ViewRow::Work(work) if is_summarizable_work(&work.row) => Ok(SummaryChild::Work(*work)),
        other => Err(other),
    }
}

fn is_timeline_step_boundary(row: &ViewRow) -> bool {
    match row {
        ViewRow::Conversation(row) => {
            !(row.role == ConversationRole::User
                && row
                    .turn_request
                    .as_ref()
                    .is_some_and(|request| request.status == TurnRequestStatus::Pending))
        }
        _ => false,
    }
}

fn dedupe_bundle_child_intents(children: Vec<SummaryChild>) -> Vec<SummaryChild> {
    let mut last_key: Option<String> = None;
    let mut out = Vec::with_capacity(children.len());
    for child in children {
        let SummaryChild::Work(mut work) = child else {
            last_key = None;
            out.push(child);
            continue;
        };
        if let TimelineWorkRow::FileRead(_) | TimelineWorkRow::Search(_) = &work.row {
            let key = work.exploration().and_then(|row| {
                timeline_row_activity_intents(row)
                    .first()
                    .and_then(activity_intent_dedupe_key)
            });
            if key.is_some() && key == last_key {
                continue;
            }
            last_key = key;
            out.push(SummaryChild::Work(work));
            continue;
        }
        let TimelineWorkRow::Command(command) = &mut work.row else {
            last_key = None;
            out.push(SummaryChild::Work(work));
            continue;
        };
        if command.activity_intents.is_empty() {
            last_key = None;
            out.push(SummaryChild::Work(work));
            continue;
        }
        let was_exploration = has_timeline_exploration_intent(ExplorationRow::Command(command));
        let original_len = command.activity_intents.len();
        let mut filtered = Vec::with_capacity(original_len);
        for intent in command.activity_intents.drain(..) {
            if intent_type(&intent).is_none() {
                filtered.push(intent);
                continue;
            }
            let key = activity_intent_dedupe_key(&intent);
            if key.is_some() && key == last_key {
                continue;
            }
            filtered.push(intent);
            last_key = key;
        }
        let changed = filtered.len() != original_len;
        let has_known = filtered.iter().any(|intent| intent_type(intent).is_some());
        command.activity_intents = filtered;
        if changed && was_exploration && !has_known {
            continue;
        }
        out.push(SummaryChild::Work(work));
    }
    out
}

fn build_summary_row(kind: SummaryKind, children: Vec<SummaryChild>) -> ViewRow {
    let children = dedupe_bundle_child_intents(children);
    let (base, status) = summarize_range(&children);
    ViewRow::Summary(Box::new(WorkSummaryRow {
        kind,
        base,
        status,
        children,
    }))
}

fn close_open_step_at_boundary(work: Vec<SummaryChild>) -> Vec<ViewRow> {
    let work_count = work.iter().filter(|row| row.work().is_some()).count();
    if work_count == 0 {
        return work.into_iter().map(SummaryChild::into_view_row).collect();
    }
    if work.len() == 1 {
        return work
            .into_iter()
            .map(|row| match row {
                SummaryChild::Work(mut work) => {
                    work.in_closed_step = true;
                    ViewRow::Work(Box::new(work))
                }
                other => other.into_view_row(),
            })
            .collect();
    }
    vec![build_summary_row(SummaryKind::Step, work)]
}

fn flush_open_step_as_bundles(work: Vec<SummaryChild>) -> Vec<ViewRow> {
    let mut groups: Vec<(SummaryCategory, Vec<SummaryChild>)> = Vec::new();
    let mut leading_thoughts: Vec<SummaryChild> = Vec::new();
    for row in work {
        let concept = match &row {
            SummaryChild::Reasoning(_) => {
                match groups.last_mut() {
                    Some((_, rows)) => rows.push(row),
                    None => leading_thoughts.push(row),
                }
                continue;
            }
            SummaryChild::Work(work) => row_concept(&work.row),
        };
        match groups.last_mut() {
            Some((last_concept, rows)) if *last_concept == concept => rows.push(row),
            _ => {
                let mut rows: Vec<SummaryChild> = std::mem::take(&mut leading_thoughts);
                rows.push(row);
                groups.push((concept, rows));
            }
        }
    }
    let mut out: Vec<ViewRow> = leading_thoughts
        .into_iter()
        .map(SummaryChild::into_view_row)
        .collect();
    for (_, mut rows) in groups {
        if rows.len() == 1 {
            out.push(rows.remove(0).into_view_row());
        } else {
            out.push(build_summary_row(SummaryKind::Bundle, rows));
        }
    }
    out
}

fn to_view_work_row(row: &TimelineWorkRow) -> ViewWorkRow {
    if let TimelineWorkRow::Delegation(delegation) = row {
        let closed_scope = delegation.status != TimelineRowStatus::Pending;
        let child_rows = delegation
            .child_rows
            .as_ref()
            .map(|rows| build_timeline_view_rows_with(rows, closed_scope));
        let mut stripped = delegation.clone();
        stripped.child_rows = None;
        return ViewWorkRow {
            row: TimelineWorkRow::Delegation(stripped),
            child_rows,
            in_closed_step: false,
        };
    }
    ViewWorkRow {
        row: row.clone(),
        child_rows: None,
        in_closed_step: false,
    }
}

fn to_view_row(row: &TimelineRow) -> ViewRow {
    match row {
        TimelineRow::Conversation(row) => ViewRow::Conversation(row.clone()),
        TimelineRow::System(row) => ViewRow::System(row.clone()),
        TimelineRow::Work(row) => ViewRow::Work(Box::new(to_view_work_row(row))),
        TimelineRow::Turn(turn) => {
            let children = turn
                .children
                .as_ref()
                .map(|rows| build_timeline_view_rows_with(rows, true));
            let mut stripped = (**turn).clone();
            stripped.children = None;
            ViewRow::Turn(Box::new(ViewTurnRow {
                row: stripped,
                children,
            }))
        }
        TimelineRow::Unknown(value) => ViewRow::Unknown(value.clone()),
    }
}

pub fn build_timeline_view_rows(rows: &[TimelineRow]) -> Vec<ViewRow> {
    build_timeline_view_rows_with(rows, false)
}

pub fn build_timeline_view_rows_with(rows: &[TimelineRow], closed_scope: bool) -> Vec<ViewRow> {
    let mut result = Vec::with_capacity(rows.len());
    let mut open_step: Vec<SummaryChild> = Vec::new();
    for row in rows.iter().map(to_view_row) {
        let row = match into_summary_child(row) {
            Ok(child) => {
                open_step.push(child);
                continue;
            }
            Err(row) => row,
        };
        if is_timeline_step_boundary(&row) {
            result.extend(close_open_step_at_boundary(std::mem::take(&mut open_step)));
        } else {
            result.extend(flush_open_step_as_bundles(std::mem::take(&mut open_step)));
        }
        result.push(row);
    }
    if closed_scope {
        result.extend(close_open_step_at_boundary(open_step));
    } else {
        result.extend(flush_open_step_as_bundles(open_step));
    }
    result
}

fn is_user_conversation(row: &ViewRow) -> bool {
    matches!(row, ViewRow::Conversation(row) if row.role == ConversationRole::User)
}

pub fn find_timeline_frontier_row(rows: &[ViewRow]) -> Option<&ViewRow> {
    rows.iter().rev().find(|row| !is_user_conversation(row))
}

pub fn find_active_latest_bundle_id(rows: &[ViewRow]) -> Option<String> {
    find_timeline_frontier_row(rows)
        .filter(|row| row.is_bundle_summary())
        .map(|row| row.id().to_owned())
}
