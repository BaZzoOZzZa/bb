use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use bb_store::{ConnectionState, ResourceKey, StoreSnapshot, TurnDetailsKey};
use bb_wire::prompt::{PromptHistoryEntry, ResolvedThreadExecutionOptions};
use bb_wire::system::{
    AppKeybindingOverride, SystemExecutionOptionsQuery, SystemExecutionOptionsResponse,
};
use bb_wire::thread::ThreadListEntry;
use bb_wire::timeline::TimelineRow;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::api::{Mutation, MutationReply};
use crate::compose::{
    Inflight, Picker, composer_command, composer_mutation_finished, composer_snapshot,
    composer_tick, handle_composer_key, handle_picker_key, handle_queue_key, history_loaded, paste,
};
use crate::composer::{Composer, SubmitKey};
use crate::execution::{ExecutionState, Load};
use crate::indicator::is_thread_read;
use crate::input::{InputOutcome, LineInput};
use crate::interact::{InteractionForm, handle_interaction_key, interaction_mutation_finished};
use crate::keymap::{Command, Keymap};
use crate::new_thread::{
    NewThreadForm, NewThreadLoaded, NewThreadRequest, handle_new_thread_key, new_thread_paste,
};
use crate::prompt::PromptDraft;
use crate::sidebar::{RowKey, SidebarRow, SidebarView, attention_count, build_rows, thread_title};
use crate::timeline::{LoadRequest, TimelineView};

pub const INFO_TOAST_TTL: Duration = Duration::from_secs(4);
pub const ERROR_TOAST_TTL: Duration = Duration::from_secs(8);
const MAX_TOASTS: usize = 4;
const SIDEBAR_PAGE: isize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Sidebar,
    Main,
    Composer,
    Interaction,
    Panel,
    NewThread,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Filter(LineInput),
    Rename {
        thread_id: String,
        input: LineInput,
    },
    Help,
    OpenPicker {
        targets: Vec<String>,
        selected: usize,
    },
    Picker(Picker),
    Queue {
        thread_id: String,
        selected: usize,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastLevel {
    Info,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Toast {
    pub message: String,
    pub level: ToastLevel,
    pub expires_at: Instant,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ServerInfo {
    pub url: String,
    pub profile: Option<String>,
}

#[derive(Debug, Clone)]
pub enum Action {
    Key(KeyEvent),
    Resize,
    Tick {
        now: Instant,
        now_ms: i64,
    },
    Snapshot(Arc<StoreSnapshot>),
    MutationFinished {
        mutation: Mutation,
        result: Result<MutationReply, String>,
    },
    DraftLoaded {
        thread_id: String,
        raw: Option<String>,
    },
    ExecutionOptionsLoaded {
        thread_id: String,
        result: Result<SystemExecutionOptionsResponse, String>,
    },
    DefaultsLoaded {
        thread_id: String,
        result: Result<Option<ResolvedThreadExecutionOptions>, String>,
    },
    PromptHistoryLoaded {
        thread_id: String,
        result: Result<Vec<PromptHistoryEntry>, String>,
    },
    Paste(String),
    KeymapLoaded(Result<Vec<AppKeybindingOverride>, String>),
    Notify {
        message: String,
        level: ToastLevel,
    },
    NewThreadLoaded {
        generation: u64,
        loaded: NewThreadLoaded,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    Mutate(Mutation),
    WatchThread(Option<String>),
    RefreshSidebar,
    RefreshAll,
    LoadOlder(String),
    LoadTurnDetails(TurnDetailsKey),
    LoadOlderTurnDetails(TurnDetailsKey),
    LoadDeferred {
        thread_id: String,
        row: Box<TimelineRow>,
    },
    Copy(String),
    OpenExternal(String),
    Refresh(ResourceKey),
    LoadDraft(String),
    SaveDraft {
        thread_id: String,
        raw: Option<String>,
    },
    FetchExecutionOptions {
        thread_id: String,
        query: SystemExecutionOptionsQuery,
    },
    FetchDefaults(String),
    FetchPromptHistory(String),
    NewThread {
        generation: u64,
        request: NewThreadRequest,
    },
    Quit,
}

#[derive(Debug)]
pub struct AppState {
    pub server: ServerInfo,
    pub snapshot: Arc<StoreSnapshot>,
    pub rows: Vec<SidebarRow>,
    pub selected: usize,
    pub sidebar_offset: Cell<usize>,
    pub collapsed: HashSet<String>,
    pub expanded_threads: HashSet<String>,
    pub filter: String,
    pub focus: Focus,
    pub mode: Mode,
    pub open_thread: Option<String>,
    pub timelines: HashMap<String, TimelineView>,
    pub composers: HashMap<String, Composer>,
    pub executions: HashMap<String, ExecutionState>,
    pub inflight: HashMap<String, Inflight>,
    pub interaction_forms: HashMap<String, InteractionForm>,
    pub interaction_follow_ups: HashMap<String, String>,
    pub new_thread: Option<NewThreadForm>,
    pub new_thread_generation: u64,
    pub new_thread_draft: Option<PromptDraft>,
    pub stop_requested: HashSet<String>,
    pub history_requested: HashSet<String>,
    pub submit_key: SubmitKey,
    pub enter_steers: bool,
    pub sidebar_visible: bool,
    pub panel_visible: bool,
    pub keymap: Keymap,
    pub read_only: bool,
    pub toasts: Vec<Toast>,
    pub last_archived: Option<String>,
    pub attention: usize,
    pub waiting: usize,
    pub clock: Instant,
    pub now_ms: i64,
    pub frame: u64,
    pub quit: bool,
}

impl AppState {
    pub fn new(server: ServerInfo, now: Instant, now_ms: i64) -> Self {
        Self {
            server,
            snapshot: Arc::new(StoreSnapshot::default()),
            rows: Vec::new(),
            selected: 0,
            sidebar_offset: Cell::new(0),
            collapsed: HashSet::new(),
            expanded_threads: HashSet::new(),
            filter: String::new(),
            focus: Focus::Sidebar,
            mode: Mode::Normal,
            open_thread: None,
            timelines: HashMap::new(),
            composers: HashMap::new(),
            executions: HashMap::new(),
            inflight: HashMap::new(),
            interaction_forms: HashMap::new(),
            interaction_follow_ups: HashMap::new(),
            new_thread: None,
            new_thread_generation: 0,
            new_thread_draft: None,
            stop_requested: HashSet::new(),
            history_requested: HashSet::new(),
            submit_key: SubmitKey::default(),
            enter_steers: false,
            sidebar_visible: true,
            panel_visible: false,
            keymap: Keymap::default(),
            read_only: false,
            toasts: Vec::new(),
            last_archived: None,
            attention: 0,
            waiting: 0,
            clock: now,
            now_ms,
            frame: 0,
            quit: false,
        }
    }

    pub fn selected_row(&self) -> Option<&SidebarRow> {
        self.rows.get(self.selected)
    }

    pub fn selected_thread_id(&self) -> Option<&str> {
        self.selected_row()
            .and_then(SidebarRow::thread)
            .map(|row| row.id.as_str())
    }

    pub fn timeline(&self) -> Option<&TimelineView> {
        self.timelines.get(self.open_thread.as_deref()?)
    }

    fn timeline_mut(&mut self) -> Option<&mut TimelineView> {
        self.timelines.get_mut(self.open_thread.as_deref()?)
    }

    fn load_effects(&self, loads: Vec<LoadRequest>) -> Vec<Effect> {
        let Some(thread_id) = self.open_thread.clone() else {
            return Vec::new();
        };
        loads
            .into_iter()
            .map(|load| match load {
                LoadRequest::Older => Effect::LoadOlder(thread_id.clone()),
                LoadRequest::Turn(key) => Effect::LoadTurnDetails(key),
                LoadRequest::OlderTurn(key) => Effect::LoadOlderTurnDetails(key),
                LoadRequest::Deferred(row) => Effect::LoadDeferred {
                    thread_id: thread_id.clone(),
                    row,
                },
            })
            .collect()
    }

    fn with_timeline(
        &mut self,
        run: impl FnOnce(&mut TimelineView) -> Vec<LoadRequest>,
    ) -> Vec<Effect> {
        let loads = self.timeline_mut().map(run).unwrap_or_default();
        self.load_effects(loads)
    }

    fn sync_timeline(&mut self) -> Vec<Effect> {
        let snapshot = self.snapshot.clone();
        self.with_timeline(|timeline| timeline.sync(snapshot))
    }

    pub fn workspace_path(&self) -> Option<&str> {
        let thread_id = self.open_thread.as_deref()?;
        self.snapshot
            .thread(thread_id)?
            .environment
            .as_ref()?
            .path
            .as_deref()
    }

    pub fn sidebar_entry(&self, thread_id: &str) -> Option<&ThreadListEntry> {
        self.snapshot.sidebar_thread(thread_id)
    }

    pub fn thread_title(&self, thread_id: &str) -> String {
        if let Some(entry) = self.sidebar_entry(thread_id) {
            return thread_title(entry);
        }
        self.snapshot
            .thread(thread_id)
            .and_then(|thread| thread.thread.title_or_fallback())
            .unwrap_or("Untitled thread")
            .to_owned()
    }

    pub fn is_disconnected(&self) -> bool {
        matches!(
            self.snapshot.connection,
            ConnectionState::Disconnected { .. }
        )
    }

    pub fn toast(&mut self, message: impl Into<String>, level: ToastLevel) {
        let ttl = match level {
            ToastLevel::Info => INFO_TOAST_TTL,
            ToastLevel::Error => ERROR_TOAST_TTL,
        };
        self.toasts.push(Toast {
            message: message.into(),
            level,
            expires_at: self.clock + ttl,
        });
        if self.toasts.len() > MAX_TOASTS {
            self.toasts.remove(0);
        }
    }

    fn target_thread(&self) -> Option<String> {
        match self.focus {
            Focus::Sidebar if self.sidebar_visible => self.selected_thread_id().map(str::to_owned),
            _ => self
                .open_thread
                .clone()
                .or_else(|| self.selected_thread_id().map(str::to_owned)),
        }
    }

    fn rebuild_rows(&mut self) {
        let previous = self.selected_row().map(SidebarRow::key);
        self.rows = match self.snapshot.sidebar.data.as_deref() {
            Some(sidebar) => build_rows(
                sidebar,
                SidebarView {
                    collapsed: &self.collapsed,
                    expanded_threads: &self.expanded_threads,
                    filter: &self.filter,
                },
            ),
            None => Vec::new(),
        };
        match previous.and_then(|key| self.rows.iter().position(|row| row.key() == key)) {
            Some(index) => self.selected = index,
            None if !self.filter.is_empty() => self.select_first_thread(),
            None => self.selected = self.selected.min(self.rows.len().saturating_sub(1)),
        }
    }

    fn select_first_thread(&mut self) {
        self.selected = self
            .rows
            .iter()
            .position(|row| row.thread().is_some())
            .unwrap_or(0);
    }

    fn select_key(&mut self, key: &RowKey) {
        if let Some(index) = self.rows.iter().position(|row| &row.key() == key) {
            self.selected = index;
        }
    }

    fn panes(&self) -> Vec<Focus> {
        let mut panes = Vec::new();
        if self.sidebar_visible {
            panes.push(Focus::Sidebar);
        }
        if self.new_thread.is_some() {
            panes.push(Focus::NewThread);
            if self.panel_visible {
                panes.push(Focus::Panel);
            }
            return panes;
        }
        panes.push(Focus::Main);
        if self.open_interaction_panel() {
            panes.push(Focus::Interaction);
        } else if self.composer_available() {
            panes.push(Focus::Composer);
        }
        if self.panel_visible {
            panes.push(Focus::Panel);
        }
        panes
    }

    fn cycle_focus(&mut self, forward: bool) {
        let panes = self.panes();
        let index = panes
            .iter()
            .position(|pane| *pane == self.focus)
            .unwrap_or(0);
        let next = if forward {
            (index + 1) % panes.len()
        } else {
            (index + panes.len() - 1) % panes.len()
        };
        self.focus = panes[next];
    }

    pub(crate) fn open(&mut self, thread_id: String) -> Vec<Effect> {
        let mut effects = Vec::new();
        self.close_new_thread();
        if self.open_thread.as_deref() != Some(thread_id.as_str()) {
            effects.push(Effect::WatchThread(Some(thread_id.clone())));
            if let Some(previous) = self.open_thread.clone() {
                effects.extend(self.flush_draft(&previous, true));
            }
            if matches!(self.focus, Focus::Composer | Focus::Interaction) {
                self.focus = Focus::Main;
            }
        }
        if let Some(entry) = self.sidebar_entry(&thread_id)
            && !is_thread_read(&entry.thread)
        {
            effects.push(Effect::Mutate(Mutation::MarkRead {
                thread_id: thread_id.clone(),
            }));
        }
        self.select_key(&RowKey::Thread(thread_id.clone()));
        self.timelines
            .entry(thread_id.clone())
            .or_insert_with(|| TimelineView::new(thread_id.clone()));
        self.open_thread = Some(thread_id.clone());
        effects.extend(self.ensure_thread_data(&thread_id));
        effects.extend(self.sync_timeline());
        self.sync_interactions();
        effects
    }

    fn thread_rows(&self) -> Vec<usize> {
        self.rows
            .iter()
            .enumerate()
            .filter(|(_, row)| row.thread().is_some())
            .map(|(index, _)| index)
            .collect()
    }

    fn step_thread(&mut self, forward: bool) -> Vec<Effect> {
        let rows = self.thread_rows();
        if rows.is_empty() {
            return Vec::new();
        }
        let anchor = self
            .open_thread
            .as_ref()
            .and_then(|id| {
                self.rows
                    .iter()
                    .position(|row| row.thread().is_some_and(|thread| &thread.id == id))
            })
            .unwrap_or(self.selected);
        let next = if forward {
            rows.iter().copied().find(|index| *index > anchor)
        } else {
            rows.iter().rev().copied().find(|index| *index < anchor)
        };
        let next = next.unwrap_or(if forward {
            rows[0]
        } else {
            rows[rows.len() - 1]
        });
        self.selected = next;
        match self.rows[next].thread().map(|row| row.id.clone()) {
            Some(id) => self.open(id),
            None => Vec::new(),
        }
    }

    fn toggle_group(&mut self, expand: Option<bool>) {
        let Some(row) = self.selected_row().cloned() else {
            return;
        };
        match row {
            SidebarRow::Group(group) => {
                let collapse = expand.map_or(!group.collapsed, |expand| !expand);
                if collapse {
                    self.collapsed.insert(group.key);
                } else {
                    self.collapsed.remove(&group.key);
                }
                self.rebuild_rows();
            }
            SidebarRow::Thread(thread) => {
                if thread.child_count > 0 {
                    let open = expand.unwrap_or(!thread.expanded);
                    if open == thread.expanded {
                        if !open {
                            self.select_parent();
                        }
                        return;
                    }
                    if open {
                        self.expanded_threads.insert(thread.id);
                    } else {
                        self.expanded_threads.remove(&thread.id);
                    }
                    self.rebuild_rows();
                } else if expand != Some(true) {
                    self.select_parent();
                }
            }
        }
    }

    fn select_parent(&mut self) {
        let Some(SidebarRow::Thread(current)) = self.selected_row() else {
            return;
        };
        let depth = current.depth;
        let parent = self.rows[..self.selected]
            .iter()
            .rposition(|row| match row {
                SidebarRow::Group(_) => true,
                SidebarRow::Thread(thread) => depth > 0 && thread.depth + 1 == depth,
            });
        if let Some(index) = parent {
            self.selected = index;
        }
    }

    fn move_selection(&mut self, delta: isize) {
        if self.rows.is_empty() {
            return;
        }
        let max = self.rows.len() - 1;
        self.selected = self.selected.saturating_add_signed(delta).min(max);
    }

    fn set_filter(&mut self, text: &str) {
        if self.filter != text {
            self.filter = text.to_owned();
            self.rebuild_rows();
            if !self.filter.is_empty() && self.selected_thread_id().is_none() {
                self.select_first_thread();
            }
        }
    }
}

pub fn update(state: &mut AppState, action: Action) -> Vec<Effect> {
    let effects = apply(state, action);
    state.sync_composer_placeholder();
    if !state.read_only {
        return effects;
    }
    effects
        .into_iter()
        .filter(|effect| match effect {
            Effect::NewThread {
                request: NewThreadRequest::Create(_),
                ..
            } => {
                state.toast("Read-only mode: cannot create thread", ToastLevel::Error);
                false
            }
            Effect::Mutate(mutation) => {
                if !mutation.is_silent() {
                    state.toast(
                        format!("Read-only mode: cannot {}", mutation.failure_verb()),
                        ToastLevel::Error,
                    );
                }
                false
            }
            _ => true,
        })
        .collect()
}

fn apply(state: &mut AppState, action: Action) -> Vec<Effect> {
    match action {
        Action::Key(key) => handle_key(state, key),
        Action::Resize => state.with_timeline(TimelineView::auto_load_older),
        Action::Tick { now, now_ms } => {
            state.clock = now;
            state.now_ms = now_ms;
            state.frame = state.frame.wrapping_add(1);
            state.toasts.retain(|toast| toast.expires_at > now);
            composer_tick(state, now)
        }
        Action::Snapshot(snapshot) => {
            state.attention = snapshot.sidebar.data.as_deref().map_or(0, attention_count);
            state.waiting = snapshot
                .sidebar_threads()
                .filter(|entry| entry.has_pending_interaction && !entry.thread.is_archived())
                .count();
            let first_load = state.snapshot.sidebar.data.is_none();
            state.snapshot = snapshot;
            state.rebuild_rows();
            if first_load && state.snapshot.sidebar.data.is_some() {
                state.select_first_thread();
            }
            let mut effects = composer_snapshot(state);
            effects.extend(state.sync_timeline());
            state.sync_interactions();
            effects.extend(state.sync_new_thread());
            effects
        }
        Action::MutationFinished { mutation, result } if mutation.is_interaction() => {
            interaction_mutation_finished(state, mutation, result)
        }
        Action::MutationFinished { mutation, result } if mutation.is_composer() => {
            composer_mutation_finished(state, mutation, result)
        }
        Action::MutationFinished { mutation, result } => match result {
            Ok(_) => {
                if !mutation.is_silent() {
                    let title = state.thread_title(mutation.thread_id());
                    state.toast(
                        format!("{}: {title}", mutation.success_message()),
                        ToastLevel::Info,
                    );
                }
                vec![Effect::RefreshSidebar]
            }
            Err(error) => {
                if matches!(mutation, Mutation::Archive { .. })
                    && state.last_archived.as_deref() == Some(mutation.thread_id())
                {
                    state.last_archived = None;
                }
                state.toast(
                    format!("Failed to {}: {error}", mutation.failure_verb()),
                    ToastLevel::Error,
                );
                Vec::new()
            }
        },
        Action::KeymapLoaded(Ok(overrides)) => {
            let applied = state.keymap.apply_overrides(&overrides);
            if applied > 0 {
                state.toast(
                    format!("Applied {applied} keybinding override(s)"),
                    ToastLevel::Info,
                );
            }
            Vec::new()
        }
        Action::KeymapLoaded(Err(error)) => {
            tracing::debug!(%error, "keybinding overrides unavailable");
            Vec::new()
        }
        Action::Notify { message, level } => {
            state.toast(message, level);
            Vec::new()
        }
        Action::DraftLoaded { thread_id, raw } => {
            if let Some(composer) = state.composers.get_mut(&thread_id) {
                composer.load_stored(raw.as_deref());
            }
            Vec::new()
        }
        Action::ExecutionOptionsLoaded { thread_id, result } => {
            if let Some(execution) = state.executions.get_mut(&thread_id) {
                execution.options = Load::from_result(result);
            }
            Vec::new()
        }
        Action::DefaultsLoaded { thread_id, result } => {
            if let Some(execution) = state.executions.get_mut(&thread_id) {
                if result.is_err() && execution.defaults.loaded().is_some() {
                    return Vec::new();
                }
                execution.defaults = Load::from_result(result);
            }
            Vec::new()
        }
        Action::PromptHistoryLoaded { thread_id, result } => {
            history_loaded(state, &thread_id, result);
            Vec::new()
        }
        Action::Paste(text) if state.focus == Focus::NewThread => {
            new_thread_paste(state, &text);
            Vec::new()
        }
        Action::Paste(text) => paste(state, &text),
        Action::NewThreadLoaded { generation, loaded } => {
            state.new_thread_loaded(generation, loaded)
        }
    }
}

fn is_ctrl_c(key: &KeyEvent) -> bool {
    key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL)
}

fn handle_key(state: &mut AppState, key: KeyEvent) -> Vec<Effect> {
    match std::mem::replace(&mut state.mode, Mode::Normal) {
        Mode::Picker(picker) => {
            if is_ctrl_c(&key) {
                return Vec::new();
            }
            handle_picker_key(state, picker, key)
        }
        Mode::Queue {
            thread_id,
            selected,
        } => {
            if is_ctrl_c(&key) {
                return Vec::new();
            }
            handle_queue_key(state, thread_id, selected, key)
        }
        Mode::Normal if state.focus == Focus::NewThread => handle_new_thread_key(state, key),
        Mode::Normal if state.focus == Focus::Interaction => handle_interaction_key(state, key),
        Mode::Normal if state.focus == Focus::Composer && state.open_interaction_panel() => {
            state.focus = Focus::Interaction;
            handle_interaction_key(state, key)
        }
        Mode::Normal if state.focus == Focus::Composer => handle_composer_key(state, key),
        Mode::Help => {
            if is_ctrl_c(&key) {
                return quit(state);
            }
            Vec::new()
        }
        Mode::OpenPicker {
            targets,
            mut selected,
        } => {
            let choice = match key.code {
                KeyCode::Esc | KeyCode::Char('q') => return Vec::new(),
                KeyCode::Enter => Some(selected),
                KeyCode::Char(ch) if ch.is_ascii_digit() && ch != '0' => {
                    Some(usize::from(ch as u8 - b'1'))
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    selected = (selected + 1).min(targets.len().saturating_sub(1));
                    None
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    selected = selected.saturating_sub(1);
                    None
                }
                _ if is_ctrl_c(&key) => return quit(state),
                _ => None,
            };
            if let Some(choice) = choice
                && let Some(target) = targets.get(choice)
            {
                return vec![Effect::OpenExternal(target.clone())];
            }
            state.mode = Mode::OpenPicker { targets, selected };
            Vec::new()
        }
        Mode::Filter(mut input) => {
            match key.code {
                KeyCode::Down => state.move_selection(1),
                KeyCode::Up => state.move_selection(-1),
                _ => match input.handle_key(key) {
                    InputOutcome::Submit => {
                        state.set_filter(input.text());
                        return Vec::new();
                    }
                    InputOutcome::Cancel => {
                        state.set_filter("");
                        return Vec::new();
                    }
                    InputOutcome::Edited => state.set_filter(input.text()),
                    InputOutcome::Moved | InputOutcome::Ignored => {}
                },
            }
            state.mode = Mode::Filter(input);
            Vec::new()
        }
        Mode::Rename {
            thread_id,
            mut input,
        } => match input.handle_key(key) {
            InputOutcome::Submit => {
                let title = input.text().trim().to_owned();
                if title.is_empty() || title == state.thread_title(&thread_id) {
                    return Vec::new();
                }
                vec![Effect::Mutate(Mutation::Rename { thread_id, title })]
            }
            InputOutcome::Cancel => Vec::new(),
            _ => {
                state.mode = Mode::Rename { thread_id, input };
                Vec::new()
            }
        },
        Mode::Normal => match state.keymap.lookup(&key) {
            Some(command) => run_command(state, command),
            None => Vec::new(),
        },
    }
}

fn quit(state: &mut AppState) -> Vec<Effect> {
    let mut effects = state.flush_all_drafts();
    state.quit = true;
    effects.push(Effect::Quit);
    effects
}

fn timeline_command(state: &mut AppState, command: Command) -> Option<Vec<Effect>> {
    let effects = match command {
        Command::MoveDown => state.with_timeline(|timeline| timeline.move_selection(1)),
        Command::MoveUp => state.with_timeline(|timeline| timeline.move_selection(-1)),
        Command::MoveTop => state.with_timeline(|timeline| timeline.select_edge(true)),
        Command::MoveBottom => state.with_timeline(|timeline| timeline.select_edge(false)),
        Command::Open => state.with_timeline(TimelineView::activate),
        Command::Expand => state.with_timeline(|timeline| timeline.set_expanded(Some(true))),
        Command::Collapse => state.with_timeline(|timeline| timeline.set_expanded(Some(false))),
        Command::ToggleGroup => state.with_timeline(|timeline| timeline.set_expanded(None)),
        Command::PageDown => state.with_timeline(|timeline| timeline.page(1, false)),
        Command::PageUp => state.with_timeline(|timeline| timeline.page(-1, false)),
        Command::HalfPageDown => state.with_timeline(|timeline| timeline.page(1, true)),
        Command::HalfPageUp => state.with_timeline(|timeline| timeline.page(-1, true)),
        Command::NextMessage => state.with_timeline(|timeline| timeline.jump_message(true)),
        Command::PreviousMessage => state.with_timeline(|timeline| timeline.jump_message(false)),
        Command::Copy => match state.timeline().and_then(TimelineView::copy_text) {
            Some(text) => vec![Effect::Copy(text)],
            None => {
                state.toast("Select a row to copy (j/k)", ToastLevel::Info);
                Vec::new()
            }
        },
        Command::OpenExternal => {
            let workspace = state.workspace_path().map(str::to_owned);
            let targets = state
                .timeline()
                .map(|timeline| timeline.open_targets(workspace.as_deref()))
                .unwrap_or_default();
            match targets.len() {
                0 => {
                    state.toast("Nothing to open in the selected row", ToastLevel::Info);
                    Vec::new()
                }
                1 => vec![Effect::OpenExternal(targets[0].clone())],
                _ => {
                    state.mode = Mode::OpenPicker {
                        targets,
                        selected: 0,
                    };
                    Vec::new()
                }
            }
        }
        _ => return None,
    };
    Some(effects)
}

pub(crate) fn run_command(state: &mut AppState, command: Command) -> Vec<Effect> {
    let in_sidebar = state.focus == Focus::Sidebar && state.sidebar_visible;
    if let Some(effects) = composer_command(state, command) {
        return effects;
    }
    if state.focus == Focus::Main
        && state.open_thread.is_some()
        && let Some(effects) = timeline_command(state, command)
    {
        return effects;
    }
    match command {
        Command::Quit => return quit(state),
        Command::Help => state.mode = Mode::Help,
        Command::Back => {
            if !state.filter.is_empty() {
                state.set_filter("");
            } else if state.focus != Focus::Sidebar && state.sidebar_visible {
                state.focus = Focus::Sidebar;
            } else {
                state.toasts.clear();
            }
        }
        Command::FocusNext => state.cycle_focus(true),
        Command::FocusPrevious => state.cycle_focus(false),
        Command::MoveDown if in_sidebar => state.move_selection(1),
        Command::MoveUp if in_sidebar => state.move_selection(-1),
        Command::MoveTop if in_sidebar => state.selected = 0,
        Command::MoveBottom if in_sidebar => state.selected = state.rows.len().saturating_sub(1),
        Command::Open if in_sidebar => match state.selected_row() {
            Some(SidebarRow::Group(_)) => state.toggle_group(None),
            Some(SidebarRow::Thread(row)) => {
                let id = row.id.clone();
                let effects = state.open(id);
                state.focus = Focus::Main;
                return effects;
            }
            None => {}
        },
        Command::Collapse if in_sidebar => state.toggle_group(Some(false)),
        Command::Expand if in_sidebar => state.toggle_group(Some(true)),
        Command::ToggleGroup if in_sidebar => state.toggle_group(None),
        Command::Filter => {
            state.sidebar_visible = true;
            state.focus = Focus::Sidebar;
            state.mode = Mode::Filter(LineInput::new(state.filter.clone()));
        }
        Command::Rename => {
            if let Some(thread_id) = state.target_thread() {
                let input = LineInput::new(state.thread_title(&thread_id));
                state.mode = Mode::Rename { thread_id, input };
            }
        }
        Command::Archive => {
            if let Some(thread_id) = state.target_thread() {
                let archived = state
                    .snapshot
                    .thread(&thread_id)
                    .is_some_and(|thread| thread.thread.is_archived());
                if archived {
                    return vec![Effect::Mutate(Mutation::Unarchive { thread_id })];
                }
                state.last_archived = Some(thread_id.clone());
                return vec![Effect::Mutate(Mutation::Archive { thread_id })];
            }
        }
        Command::UndoArchive => match state.last_archived.take() {
            Some(thread_id) => return vec![Effect::Mutate(Mutation::Unarchive { thread_id })],
            None => state.toast("Nothing to undo", ToastLevel::Info),
        },
        Command::ToggleRead => {
            if let Some(thread_id) = state.target_thread()
                && let Some(entry) = state.sidebar_entry(&thread_id)
            {
                let mutation = if is_thread_read(&entry.thread) {
                    Mutation::MarkUnread { thread_id }
                } else {
                    Mutation::MarkRead { thread_id }
                };
                return vec![Effect::Mutate(mutation)];
            }
        }
        Command::TogglePin => {
            if let Some(thread_id) = state.target_thread() {
                let pinned = state
                    .sidebar_entry(&thread_id)
                    .map(|entry| entry.thread.is_pinned())
                    .or_else(|| {
                        state
                            .snapshot
                            .thread(&thread_id)
                            .map(|thread| thread.thread.is_pinned())
                    })
                    .unwrap_or(false);
                let mutation = if pinned {
                    Mutation::Unpin { thread_id }
                } else {
                    Mutation::Pin { thread_id }
                };
                return vec![Effect::Mutate(mutation)];
            }
        }
        Command::NextPending => return state.jump_to_pending(),
        Command::NewThread => return state.open_new_thread(),
        Command::NewThreadInEnvironment => return state.open_new_thread_in_environment(),
        Command::ThreadNext => return state.step_thread(true),
        Command::ThreadPrevious => return state.step_thread(false),
        Command::Jump(n) => {
            let rows = state.thread_rows();
            if let Some(index) = rows.get(usize::from(n.saturating_sub(1))).copied()
                && let Some(id) = state.rows[index].thread().map(|row| row.id.clone())
            {
                return state.open(id);
            }
        }
        Command::SidebarToggle => {
            state.sidebar_visible = !state.sidebar_visible;
            if !state.sidebar_visible && state.focus == Focus::Sidebar {
                state.focus = Focus::Main;
            }
        }
        Command::PanelToggle => {
            state.panel_visible = !state.panel_visible;
            if !state.panel_visible && state.focus == Focus::Panel {
                state.focus = Focus::Main;
            }
        }
        Command::Reload => {
            state.toast("Reloading", ToastLevel::Info);
            return vec![Effect::RefreshAll];
        }
        Command::PageDown if in_sidebar => state.move_selection(SIDEBAR_PAGE),
        Command::PageUp if in_sidebar => state.move_selection(-SIDEBAR_PAGE),
        Command::HalfPageDown if in_sidebar => state.move_selection(SIDEBAR_PAGE / 2),
        Command::HalfPageUp if in_sidebar => state.move_selection(-SIDEBAR_PAGE / 2),
        Command::MoveDown
        | Command::MoveUp
        | Command::MoveTop
        | Command::MoveBottom
        | Command::Open
        | Command::Collapse
        | Command::Expand
        | Command::ToggleGroup
        | Command::PageDown
        | Command::PageUp
        | Command::HalfPageDown
        | Command::HalfPageUp
        | Command::NextMessage
        | Command::PreviousMessage
        | Command::Copy
        | Command::OpenExternal
        | Command::FocusComposer
        | Command::StopTurn
        | Command::ModelPicker
        | Command::ReasoningPicker
        | Command::PermissionPicker
        | Command::CycleModel
        | Command::CycleModelBackward
        | Command::CycleReasoning
        | Command::CycleReasoningBackward
        | Command::QueueList
        | Command::ToggleSubmitMode => {}
    }
    Vec::new()
}
