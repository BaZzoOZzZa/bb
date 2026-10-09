use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use bb_store::{ConnectionState, ResourceKey, StoreSnapshot, TurnDetailsKey};
use bb_wire::plugin::{PluginCliContribution, PluginMentionProviderContribution};
use bb_wire::project::{ProviderCommand, UploadedPromptAttachment};
use bb_wire::prompt::{PromptHistoryEntry, ResolvedThreadExecutionOptions};
use bb_wire::system::{
    AppKeybindingOverride, SystemExecutionOptionsQuery, SystemExecutionOptionsResponse,
};
use bb_wire::thread::ThreadListEntry;
use bb_wire::timeline::TimelineRow;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::api::{Mutation, MutationReply};
use crate::attach::UploadRequest;
use crate::automations::{AutomationLoaded, AutomationRequest, AutomationsView};
use crate::compose::{
    Inflight, Picker, composer_command, composer_mutation_finished, composer_snapshot,
    composer_tick, handle_composer_key, handle_picker_key, handle_queue_key, history_loaded, paste,
};
use crate::composer::{Composer, EditMode, SubmitKey};
use crate::execution::{ExecutionState, Load};
use crate::indicator::is_thread_read;
use crate::input::{InputOutcome, LineInput};
use crate::interact::{InteractionForm, handle_interaction_key, interaction_mutation_finished};
use crate::keymap::{Command, KeyChord, Keymap, Scope, ScreenPos};
use crate::new_thread::{
    NewThreadForm, NewThreadLoaded, NewThreadRequest, handle_new_thread_key, new_thread_paste,
};
use crate::palette::{
    Palette, PaletteKind, PaletteLoaded, PaletteRequest, ResultPane, handle_palette_key,
    handle_pane_key,
};
use crate::panel::{GitAction, PanelLoaded, PanelRequest, RightPanel};
use crate::prompt::PromptDraft;
use crate::settings::{SettingsLoaded, SettingsRequest, SettingsView};
use crate::sidebar::{RowKey, SidebarRow, SidebarView, attention_count, build_rows, thread_title};
use crate::terminal::{TerminalLoaded, TerminalPanel, TerminalRequest};
use crate::timeline::{LoadRequest, TimelineView};
use crate::typeahead::{
    CommandCatalogKey, ComposerTarget, Typeahead, TypeaheadLoaded, TypeaheadRequest,
};
use crate::vi::InsertAt;

pub const INFO_TOAST_TTL: Duration = Duration::from_secs(4);
pub const ERROR_TOAST_TTL: Duration = Duration::from_secs(8);
const MAX_TOASTS: usize = 4;
const SIDEBAR_PAGE: isize = 10;
const MAX_MESSAGES: usize = 200;

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
    Ex(crate::ex::ExLine),
    OpenPicker {
        targets: Vec<String>,
        selected: usize,
    },
    Picker(Picker),
    Queue {
        thread_id: String,
        selected: usize,
    },
    AttachPrompt {
        target: ComposerTarget,
        input: LineInput,
    },
    Attachments {
        target: ComposerTarget,
        selected: usize,
    },
    Notifications {
        selected: usize,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastLevel {
    Info,
    Attention,
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
    Mouse(crossterm::event::MouseEvent),
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
    PaletteLoaded {
        generation: u64,
        loaded: PaletteLoaded,
    },
    TypeaheadLoaded(TypeaheadLoaded),
    PanelLoaded(PanelLoaded),
    AttachmentUploaded {
        target: ComposerTarget,
        path: std::path::PathBuf,
        result: Result<UploadedPromptAttachment, String>,
    },
    PluginSignal {
        plugin_id: String,
        channel: String,
        payload: bb_wire::common::JsonValue,
    },
    TerminalFocus(bool),
    TerminalLoaded(TerminalLoaded),
    TerminalEvent {
        terminal_id: String,
        link: u64,
        event: bb_client::TerminalEvent,
    },
    TerminalRawInput(Vec<u8>),
    FullscreenResize {
        cols: u16,
        rows: u16,
    },
    FullscreenEnded(String),
    NotificationInteractions {
        thread_id: String,
        result: Result<Vec<bb_wire::interaction::PendingInteraction>, String>,
    },
    SettingsLoaded(SettingsLoaded),
    AutomationsLoaded(AutomationLoaded),
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
    Palette {
        generation: u64,
        request: PaletteRequest,
    },
    SavePaletteRecents(Vec<String>),
    Typeahead(TypeaheadRequest),
    Upload(UploadRequest),
    Panel(PanelRequest),
    EditFile {
        path: String,
        line: Option<u32>,
    },
    Alert(crate::notifications::Alert),
    SetTitle(String),
    FetchNotificationInteractions(String),
    Terminal(TerminalRequest),
    Settings(SettingsRequest),
    Automations(AutomationRequest),
    WatchSplits(Vec<String>),
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
    pub new_thread: Option<NewThreadForm>,
    pub new_thread_generation: u64,
    pub new_thread_draft: Option<PromptDraft>,
    pub palette: Option<Palette>,
    pub palette_generation: u64,
    pub palette_recents: Vec<String>,
    pub plugin_contributions: Option<Vec<PluginCliContribution>>,
    pub result_pane: Option<ResultPane>,
    pub typeahead: Option<Typeahead>,
    pub mention_providers: Load<Vec<PluginMentionProviderContribution>>,
    pub command_catalogs: HashMap<CommandCatalogKey, Load<Vec<ProviderCommand>>>,
    pub stop_requested: HashSet<String>,
    pub history_requested: HashSet<String>,
    pub submit_key: SubmitKey,
    pub enter_steers: bool,
    pub sidebar_visible: bool,
    pub panel_visible: bool,
    pub panel: RightPanel,
    pub terminals: TerminalPanel,
    pub keymap: Keymap,
    pub read_only: bool,
    pub toasts: Vec<Toast>,
    pub last_archived: Option<String>,
    pub attention: usize,
    pub waiting: usize,
    pub notifications: crate::notifications::Notifications,
    pub settings: Option<SettingsView>,
    pub automations: Option<AutomationsView>,
    pub splits: Vec<String>,
    pub split_maximized: bool,
    pub keys: crate::evil::KeyState,
    pub registers: crate::evil::Registers,
    pub prev_window: Option<crate::window::WindowId>,
    pub zoom: Option<crate::window::Zoom>,
    pub sidebar_width_delta: i16,
    pub panel_width_delta: i16,
    pub alternate_thread: Option<String>,
    pub last_palette: Option<(PaletteKind, String, usize)>,
    pub messages: Vec<String>,
    pub ex_history: Vec<String>,
    pub split_new_thread: bool,
    pub theme: crate::theme::Theme,
    pub mouse_areas: crate::mouse::MouseAreas,
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
            new_thread: None,
            new_thread_generation: 0,
            new_thread_draft: None,
            palette: None,
            palette_generation: 0,
            palette_recents: Vec::new(),
            plugin_contributions: None,
            result_pane: None,
            typeahead: None,
            mention_providers: Load::Idle,
            command_catalogs: HashMap::new(),
            stop_requested: HashSet::new(),
            history_requested: HashSet::new(),
            submit_key: SubmitKey::default(),
            enter_steers: false,
            sidebar_visible: true,
            panel_visible: false,
            panel: RightPanel::default(),
            terminals: TerminalPanel::default(),
            keymap: Keymap::default(),
            read_only: false,
            toasts: Vec::new(),
            last_archived: None,
            attention: 0,
            waiting: 0,
            notifications: crate::notifications::Notifications::default(),
            settings: None,
            automations: None,
            splits: Vec::new(),
            split_maximized: false,
            keys: crate::evil::KeyState::default(),
            registers: crate::evil::Registers::default(),
            prev_window: None,
            zoom: None,
            sidebar_width_delta: 0,
            panel_width_delta: 0,
            alternate_thread: None,
            last_palette: None,
            messages: Vec::new(),
            ex_history: Vec::new(),
            split_new_thread: false,
            theme: crate::theme::Theme::default(),
            mouse_areas: crate::mouse::MouseAreas::default(),
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
        match self.open_thread.clone() {
            Some(thread_id) => self.load_effects_for(&thread_id, loads),
            None => Vec::new(),
        }
    }

    pub(crate) fn load_effects_for(&self, thread_id: &str, loads: Vec<LoadRequest>) -> Vec<Effect> {
        let thread_id = thread_id.to_owned();
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
            ToastLevel::Attention | ToastLevel::Error => ERROR_TOAST_TTL,
        };
        let message = message.into();
        self.messages.push(message.clone());
        if self.messages.len() > MAX_MESSAGES {
            self.messages.remove(0);
        }
        self.toasts.push(Toast {
            message,
            level,
            expires_at: self.clock + ttl,
        });
        if self.toasts.len() > MAX_TOASTS {
            self.toasts.remove(0);
        }
    }

    pub(crate) fn target_thread(&self) -> Option<String> {
        match self.focus {
            Focus::Sidebar if self.sidebar_visible => self.selected_thread_id().map(str::to_owned),
            _ => self
                .open_thread
                .clone()
                .or_else(|| self.selected_thread_id().map(str::to_owned)),
        }
    }

    pub(crate) fn rebuild_rows(&mut self) {
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

    pub(crate) fn select_thread_row(&mut self, thread_id: &str) {
        self.select_key(&RowKey::Thread(thread_id.to_owned()));
    }

    pub fn focused_composer(&self) -> Option<&Composer> {
        let target = self.active_composer_target()?;
        self.composer_for(&target)
    }

    pub fn open(&mut self, thread_id: String) -> Vec<Effect> {
        let mut effects = Vec::new();
        self.close_new_thread();
        self.place_in_split(&thread_id);
        if self.open_thread.as_deref() != Some(thread_id.as_str()) {
            effects.push(Effect::WatchThread(Some(thread_id.clone())));
            if let Some(previous) = self.open_thread.clone() {
                effects.extend(self.flush_draft(&previous, true));
                self.alternate_thread = Some(previous);
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
        self.notifications.mark_thread_read(&thread_id);
        self.timelines
            .entry(thread_id.clone())
            .or_insert_with(|| TimelineView::new(thread_id.clone()));
        self.open_thread = Some(thread_id.clone());
        effects.extend(self.ensure_thread_data(&thread_id));
        effects.extend(self.sync_timeline());
        self.sync_interactions();
        effects.extend(self.sync_panel());
        if !self.splits.is_empty() {
            effects.push(self.split_watch());
        }
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

    pub(crate) fn toggle_group(&mut self, expand: Option<bool>) {
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

    pub(crate) fn move_selection(&mut self, delta: isize) {
        if self.rows.is_empty() {
            return;
        }
        let max = self.rows.len() - 1;
        self.selected = self.selected.saturating_add_signed(delta).min(max);
    }

    pub(crate) fn apply_filter(&mut self, text: &str) {
        self.set_filter(text);
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
            Effect::Palette {
                generation,
                request: PaletteRequest::RunCli { .. },
            } => {
                state.close_pane(*generation);
                state.toast(
                    "Read-only mode: cannot run plugin commands",
                    ToastLevel::Error,
                );
                false
            }
            Effect::Terminal(request) => match request.read_only_refusal() {
                None => true,
                Some(refusal) => {
                    if let TerminalRequest::Create { .. } = request {
                        state.terminals.creating = false;
                    }
                    if let Some(verb) = refusal {
                        state.toast(format!("Read-only mode: cannot {verb}"), ToastLevel::Error);
                    }
                    false
                }
            },
            Effect::Settings(SettingsRequest::Mutate(mutation)) => {
                state.settings_refused(mutation);
                false
            }
            Effect::Automations(AutomationRequest::Mutate(mutation)) => {
                state.automations_refused(mutation);
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
        Action::Mouse(mouse) => {
            let mut effects = crate::mouse::handle_mouse(state, mouse);
            effects.extend(sync_active_typeahead(state));
            effects
        }
        Action::Resize => state.with_timeline(TimelineView::auto_load_older),
        Action::Tick { now, now_ms } => {
            state.clock = now;
            state.now_ms = now_ms;
            state.frame = state.frame.wrapping_add(1);
            state.toasts.retain(|toast| toast.expires_at > now);
            state.keys.tick(now);
            let mut effects = composer_tick(state, now);
            effects.extend(state.panel_tick());
            effects.extend(state.notifications_tick());
            effects.extend(state.terminals_tick());
            effects
        }
        Action::Snapshot(snapshot) => {
            state.attention = snapshot.sidebar.data.as_deref().map_or(0, attention_count);
            state.waiting = snapshot
                .sidebar_threads()
                .filter(|entry| entry.has_pending_interaction && !entry.thread.is_archived())
                .count();
            let first_load = state.snapshot.sidebar.data.is_none();
            let previous = std::mem::replace(&mut state.snapshot, snapshot);
            state.notifications_snapshot(&previous);
            state.rebuild_rows();
            if first_load && state.snapshot.sidebar.data.is_some() {
                state.select_first_thread();
            }
            let mut effects = composer_snapshot(state);
            effects.extend(state.sync_timeline());
            effects.extend(state.sync_split_timelines());
            state.sync_interactions();
            effects.extend(state.sync_new_thread());
            effects.extend(state.sync_panel());
            effects.extend(state.sync_title());
            effects
        }
        Action::MutationFinished { mutation, result } if mutation.is_interaction() => {
            interaction_mutation_finished(state, mutation, result)
        }
        Action::MutationFinished { mutation, result } if mutation.is_environment_action() => {
            state.git_action_finished(&mutation, result)
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
        Action::Paste(text) => {
            if let Some(effects) = state.terminal_paste(&text) {
                return effects;
            }
            if let Some(effects) = crate::compose::paste_attachments(state, &text) {
                return effects;
            }
            let mut effects = if state.focus == Focus::NewThread {
                new_thread_paste(state, &text);
                Vec::new()
            } else {
                paste(state, &text)
            };
            effects.extend(sync_active_typeahead(state));
            effects
        }
        Action::NewThreadLoaded { generation, loaded } => {
            state.new_thread_loaded(generation, loaded)
        }
        Action::PaletteLoaded { generation, loaded } => state.palette_loaded(generation, loaded),
        Action::TypeaheadLoaded(loaded) => state.typeahead_loaded(loaded),
        Action::PanelLoaded(loaded) => state.panel_loaded(loaded),
        Action::AttachmentUploaded {
            target,
            path,
            result,
        } => {
            state.attachment_uploaded(target, path, result);
            Vec::new()
        }
        Action::PluginSignal {
            plugin_id,
            channel,
            payload,
        } if plugin_id == crate::automation_data::PLUGIN_ID
            && channel == crate::automation_data::SIGNAL_CHANNEL =>
        {
            state.automations_signal(&payload)
        }
        Action::PluginSignal {
            plugin_id,
            channel,
            payload,
        } => state.plugin_signal(&plugin_id, &channel, payload),
        Action::TerminalFocus(focused) => {
            state.notifications.terminal_focused = Some(focused);
            Vec::new()
        }
        Action::NotificationInteractions { thread_id, result } => {
            state.notification_interactions_loaded(thread_id, result)
        }
        Action::TerminalLoaded(loaded) => state.terminals_loaded(loaded),
        Action::TerminalEvent {
            terminal_id,
            link,
            event,
        } => state.terminal_event(terminal_id, link, event),
        Action::TerminalRawInput(bytes) => state.fullscreen_input(bytes),
        Action::FullscreenResize { cols, rows } => state.fullscreen_resize(cols, rows),
        Action::FullscreenEnded(message) => state.fullscreen_ended(message),
        Action::SettingsLoaded(loaded) => state.settings_loaded(loaded),
        Action::AutomationsLoaded(loaded) => state.automations_loaded(loaded),
    }
}

fn is_ctrl_c(key: &KeyEvent) -> bool {
    key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL)
}

fn sync_active_typeahead(state: &mut AppState) -> Vec<Effect> {
    match state.active_composer_target() {
        Some(target)
            if state.mode == Mode::Normal
                && state.composer_for(&target).is_some_and(Composer::is_insert) =>
        {
            state.sync_typeahead(&target)
        }
        _ => {
            state.typeahead = None;
            Vec::new()
        }
    }
}

pub(crate) fn handle_key(state: &mut AppState, key: KeyEvent) -> Vec<Effect> {
    let before = state.current_window();
    state.keys.echo = None;
    let mut effects = dispatch_key(state, key);
    let after = state.current_window();
    if after != before {
        state.prev_window = Some(before);
    }
    effects.extend(sync_active_typeahead(state));
    effects
}

fn composer_scopes(composer: &Composer) -> Vec<Scope> {
    match composer.mode {
        EditMode::Insert => vec![Scope::Insert],
        EditMode::Visual { .. } => vec![Scope::Visual, Scope::Composer],
        EditMode::Normal => vec![Scope::Composer],
    }
}

pub(crate) fn normal_scopes(state: &AppState) -> Vec<Scope> {
    if state.result_pane.is_some() {
        return vec![Scope::Pane];
    }
    if state.automations.is_some() {
        return vec![Scope::Automations];
    }
    if state.settings.is_some() {
        return vec![Scope::Settings];
    }
    if matches!(state.mode, Mode::Notifications { .. }) {
        return vec![Scope::Notifications];
    }
    match state.focus {
        Focus::NewThread => match state.new_thread.as_ref() {
            Some(form) if form.prompt_active => composer_scopes(&form.composer),
            Some(_) => vec![Scope::NewThread],
            None => Vec::new(),
        },
        Focus::Panel if state.panel_visible => {
            let tab = match state.panel.tab {
                crate::panel::PanelTab::Info => Scope::PanelInfo,
                crate::panel::PanelTab::Diff => Scope::PanelDiff,
                crate::panel::PanelTab::File => Scope::PanelFile,
                crate::panel::PanelTab::Terminal => Scope::PanelTerminal,
            };
            vec![tab, Scope::Panel]
        }
        Focus::Composer => state
            .focused_composer()
            .map(composer_scopes)
            .unwrap_or_default(),
        Focus::Sidebar if state.sidebar_visible => vec![Scope::Sidebar],
        Focus::Main if state.open_thread.is_some() => vec![Scope::Timeline],
        _ => Vec::new(),
    }
}

fn dispatch_key(state: &mut AppState, key: KeyEvent) -> Vec<Effect> {
    if state.keys.is_pending() || state.keys.await_char.is_some() {
        let scopes = normal_scopes(state);
        return crate::evil::normal_key(state, key, &scopes);
    }
    if KeyChord::from_event(&key) == KeyChord::alt(' ') {
        state.keys.start_leader();
        return Vec::new();
    }
    if let Mode::Ex(_) = state.mode {
        let Mode::Ex(line) = std::mem::replace(&mut state.mode, Mode::Normal) else {
            return Vec::new();
        };
        return crate::ex::handle_ex_key(state, line, key);
    }
    if state.palette.is_some() {
        return handle_palette_key(state, key);
    }
    if state.result_pane.is_some() {
        return crate::evil::normal_key(state, key, &[Scope::Pane]);
    }
    if let Some(view) = &state.automations
        && state.mode == Mode::Normal
    {
        if view.captures_keys() {
            return crate::automations::handle_automations_key(state, key);
        }
        return crate::evil::normal_key(state, key, &[Scope::Automations]);
    }
    if let Some(view) = &state.settings {
        if view.captures_keys() {
            return crate::settings::handle_settings_key(state, key);
        }
        return crate::evil::normal_key(state, key, &[Scope::Settings]);
    }
    if state.panel.confirm.is_some() {
        return crate::panel::handle_confirm_key(state, key);
    }
    match &state.mode {
        Mode::Notifications { .. } => {
            return crate::evil::normal_key(state, key, &[Scope::Notifications]);
        }
        Mode::Picker(_)
        | Mode::Queue { .. }
        | Mode::Attachments { .. }
        | Mode::OpenPicker { .. } => {
            if let Some(effects) = crate::evil::transient_intercept(state, &key) {
                return effects;
            }
            return mode_key(state, key);
        }
        Mode::Filter(_) | Mode::Rename { .. } | Mode::AttachPrompt { .. } => {
            return mode_key(state, key);
        }
        Mode::Normal | Mode::Ex(_) => {}
    }
    if state.terminal_input_active() {
        return crate::terminal::handle_terminal_input(state, key);
    }
    match state.focus {
        Focus::NewThread => new_thread_key(state, key),
        Focus::Panel if state.panel_visible => {
            if state.panel.tab == crate::panel::PanelTab::Terminal
                && state.terminals.confirm_close.is_some()
            {
                return crate::panel::handle_panel_key(state, key).unwrap_or_default();
            }
            let scopes = normal_scopes(state);
            crate::evil::normal_key(state, key, &scopes)
        }
        Focus::Interaction => interaction_key(state, key),
        Focus::Composer if state.open_interaction_panel() => {
            state.focus = Focus::Interaction;
            interaction_key(state, key)
        }
        Focus::Composer => composer_key(state, key),
        Focus::Sidebar if state.sidebar_visible => {
            crate::evil::normal_key(state, key, &[Scope::Sidebar])
        }
        Focus::Main if state.open_thread.is_some() => {
            crate::evil::normal_key(state, key, &[Scope::Timeline])
        }
        _ => crate::evil::normal_key(state, key, &[]),
    }
}

fn interaction_key(state: &mut AppState, key: KeyEvent) -> Vec<Effect> {
    if let Some(effects) = crate::evil::transient_intercept(state, &key) {
        return effects;
    }
    handle_interaction_key(state, key)
}

fn new_thread_key(state: &mut AppState, key: KeyEvent) -> Vec<Effect> {
    let Some(form) = state.new_thread.as_ref() else {
        state.focus = Focus::Main;
        return Vec::new();
    };
    if form.picker.is_some() {
        return handle_new_thread_key(state, key);
    }
    if form.prompt_active {
        return composer_key(state, key);
    }
    crate::evil::normal_key(state, key, &[Scope::NewThread])
}

fn composer_key(state: &mut AppState, key: KeyEvent) -> Vec<Effect> {
    let Some(target) = state.active_composer_target() else {
        state.focus = Focus::Main;
        return Vec::new();
    };
    if let ComposerTarget::Thread(_) = target
        && !state.composer_available()
    {
        state.focus = Focus::Main;
        return Vec::new();
    }
    let Some(composer) = state.composer_for(&target) else {
        state.focus = Focus::Main;
        return Vec::new();
    };
    let scopes = composer_scopes(composer);
    if !composer.is_insert() {
        return crate::evil::normal_key(state, key, &scopes);
    }
    if !state.keys.is_pending()
        && let Some(effects) = crate::typeahead::handle_typeahead_key(state, &target, &key)
    {
        return effects;
    }
    if let Some(effects) = crate::evil::insert_intercept(state, &key) {
        return effects;
    }
    match target {
        ComposerTarget::Thread(_) => handle_composer_key(state, key),
        ComposerTarget::NewThread => crate::new_thread::prompt_key(state, key),
    }
}

pub(crate) fn legacy_key(state: &mut AppState, key: KeyEvent) -> Vec<Effect> {
    if state.result_pane.is_some() {
        return handle_pane_key(state, key);
    }
    if state.automations.is_some() {
        return crate::automations::handle_automations_key(state, key);
    }
    if state.settings.is_some() {
        return crate::settings::handle_settings_key(state, key);
    }
    if let Mode::Notifications { selected } = state.mode {
        state.mode = Mode::Normal;
        return crate::notifications::handle_notifications_key(state, selected, key);
    }
    match state.focus {
        Focus::NewThread => handle_new_thread_key(state, key),
        Focus::Panel if state.panel_visible => {
            crate::panel::handle_panel_key(state, key).unwrap_or_default()
        }
        _ => Vec::new(),
    }
}

fn mode_key(state: &mut AppState, key: KeyEvent) -> Vec<Effect> {
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
        Mode::AttachPrompt { target, input } => {
            crate::compose::handle_attach_prompt_key(state, target, input, key)
        }
        Mode::Attachments { target, selected } => {
            crate::compose::handle_attachments_key(state, target, selected, key)
        }
        Mode::OpenPicker {
            targets,
            mut selected,
        } => {
            let choice = match key.code {
                KeyCode::Esc | KeyCode::Char('q') => return Vec::new(),
                _ if is_ctrl_c(&key) => return Vec::new(),
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
        other => {
            state.mode = other;
            Vec::new()
        }
    }
}

fn quit(state: &mut AppState) -> Vec<Effect> {
    let mut effects = state.flush_all_drafts();
    state.quit = true;
    effects.push(Effect::Quit);
    effects
}

fn keyboard_quit(state: &mut AppState) -> Vec<Effect> {
    let describing = state.keys.describe;
    state.keys.reset();
    state.keys.describe = false;
    if describing {
        return Vec::new();
    }
    let escape = KeyEvent::from(KeyCode::Esc);
    if state.result_pane.is_some()
        || state.settings.is_some()
        || state.automations.is_some()
        || matches!(state.mode, Mode::Notifications { .. })
        || (state.focus == Focus::Panel && state.panel_visible)
    {
        return legacy_key(state, escape);
    }
    if state.focus == Focus::NewThread
        && let Some(form) = state.new_thread.as_mut()
    {
        if form.prompt_active {
            if matches!(form.composer.mode, EditMode::Visual { .. }) {
                form.composer.enter_normal();
            } else {
                form.prompt_active = false;
            }
            return Vec::new();
        }
        return legacy_key(state, escape);
    }
    if state.focus == Focus::Composer
        && let Some(thread_id) = state.open_thread.clone()
        && let Some(composer) = state.composers.get_mut(&thread_id)
    {
        if matches!(composer.mode, EditMode::Visual { .. }) {
            composer.enter_normal();
            return Vec::new();
        }
        if composer.editing.is_some() {
            return state.cancel_queued_edit(&thread_id);
        }
    }
    if !state.filter.is_empty() {
        state.set_filter("");
    }
    Vec::new()
}

fn sidebar_viewport(state: &AppState) -> usize {
    state
        .mouse_areas
        .sidebar
        .get()
        .map_or(SIDEBAR_PAGE as usize * 2, |(area, _)| {
            usize::from(area.height)
        })
        .max(1)
}

fn sidebar_scroll(state: &mut AppState, command: Command) {
    let height = sidebar_viewport(state);
    let len = state.rows.len();
    if len == 0 {
        return;
    }
    let offset = state.sidebar_offset.get();
    match command {
        Command::ScrollLine(delta) => {
            let max = len.saturating_sub(height);
            let next = offset.saturating_add_signed(isize::from(delta)).min(max);
            state.sidebar_offset.set(next);
            state.selected = state
                .selected
                .clamp(next, (next + height).saturating_sub(1).min(len - 1));
        }
        Command::ScrollCursor(position) => {
            let next = match position {
                ScreenPos::Top => state.selected,
                ScreenPos::Middle => state.selected.saturating_sub(height / 2),
                ScreenPos::Bottom => (state.selected + 1).saturating_sub(height),
            };
            state.sidebar_offset.set(next);
        }
        Command::ViewJump(position) => {
            let last = (offset + height).min(len).saturating_sub(1);
            state.selected = match position {
                ScreenPos::Top => offset,
                ScreenPos::Middle => (offset + last) / 2,
                ScreenPos::Bottom => last,
            };
        }
        _ => {}
    }
}

fn step_marked(state: &mut AppState, forward: bool, waiting: bool) -> Vec<Effect> {
    let in_sidebar = state.focus == Focus::Sidebar && state.sidebar_visible;
    let anchor = if in_sidebar {
        state.selected
    } else {
        state
            .open_thread
            .as_ref()
            .and_then(|id| {
                state
                    .rows
                    .iter()
                    .position(|row| row.thread().is_some_and(|thread| &thread.id == id))
            })
            .unwrap_or(state.selected)
    };
    let marked: Vec<usize> = state
        .rows
        .iter()
        .enumerate()
        .filter(|(_, row)| {
            row.thread().is_some_and(|thread| {
                state.sidebar_entry(&thread.id).is_some_and(|entry| {
                    if waiting {
                        entry.has_pending_interaction
                    } else {
                        !is_thread_read(&entry.thread)
                    }
                })
            })
        })
        .map(|(index, _)| index)
        .collect();
    let next = if forward {
        marked
            .iter()
            .copied()
            .find(|index| *index > anchor)
            .or_else(|| marked.first().copied())
    } else {
        marked
            .iter()
            .rev()
            .copied()
            .find(|index| *index < anchor)
            .or_else(|| marked.last().copied())
    };
    let Some(next) = next else {
        state.keys.message(if waiting {
            "No thread is waiting for an answer"
        } else {
            "No unread threads"
        });
        return Vec::new();
    };
    state.selected = next;
    if in_sidebar {
        return Vec::new();
    }
    match state.rows[next].thread().map(|row| row.id.clone()) {
        Some(id) => state.open(id),
        None => Vec::new(),
    }
}

fn goto_line(state: &mut AppState, line: usize) -> Option<Vec<Effect>> {
    let index = line.saturating_sub(1);
    match state.focus {
        Focus::Sidebar if state.sidebar_visible => {
            state.selected = index.min(state.rows.len().saturating_sub(1));
            Some(Vec::new())
        }
        Focus::Main if state.open_thread.is_some() => {
            Some(state.with_timeline(|timeline| timeline.select_index(index)))
        }
        _ => None,
    }
}

pub(crate) fn counted_command(
    state: &mut AppState,
    command: Command,
    count: Option<usize>,
) -> Option<Vec<Effect>> {
    let count = count?;
    match command {
        Command::MoveBottom | Command::MoveTop => goto_line(state, count),
        Command::PanelNextTab => {
            if state.settings.is_some() {
                let section = crate::settings::SettingsSection::ALL
                    .get(count.saturating_sub(1))
                    .copied()?;
                return Some(state.set_section(section));
            }
            let tab = crate::panel::PanelTab::ALL
                .get(count.saturating_sub(1))
                .copied()?;
            Some(state.set_panel_tab(tab))
        }
        Command::FocusNext if state.is_split() => {
            Some(state.focus_pane(count.saturating_sub(1).min(state.splits.len() - 1)))
        }
        Command::FocusNext => Some(state.cycle_focus(true, count)),
        Command::FocusPrevious => Some(state.cycle_focus(false, count)),
        Command::WindowResize(sign) => {
            state.resize_side(sign, Some(count));
            Some(Vec::new())
        }
        Command::WindowRotate => Some(state.rotate_panes(count)),
        _ => None,
    }
}

fn timeline_command(state: &mut AppState, command: Command) -> Option<Vec<Effect>> {
    let effects = match command {
        Command::MoveDown => state.with_timeline(|timeline| timeline.move_selection(1)),
        Command::MoveUp => state.with_timeline(|timeline| timeline.move_selection(-1)),
        Command::MoveTop => state.with_timeline(|timeline| timeline.select_edge(true)),
        Command::MoveBottom => state.with_timeline(|timeline| timeline.select_edge(false)),
        Command::Open | Command::ToggleGroup => state.with_timeline(TimelineView::activate),
        Command::Expand => state.with_timeline(|timeline| timeline.set_expanded(Some(true))),
        Command::Collapse => state.with_timeline(|timeline| timeline.set_expanded(Some(false))),
        Command::FoldAll(open) => state.with_timeline(|timeline| timeline.set_all_expanded(open)),
        Command::PageDown => state.with_timeline(|timeline| timeline.page(1, false)),
        Command::PageUp => state.with_timeline(|timeline| timeline.page(-1, false)),
        Command::HalfPageDown => state.with_timeline(|timeline| timeline.page(1, true)),
        Command::HalfPageUp => state.with_timeline(|timeline| timeline.page(-1, true)),
        Command::ScrollLine(delta) => {
            state.with_timeline(|timeline| timeline.scroll_by(isize::from(delta)))
        }
        Command::ScrollCursor(position) => {
            state.with_timeline(|timeline| timeline.align_selected(position))
        }
        Command::ViewJump(position) => {
            state.with_timeline(|timeline| timeline.select_in_view(position))
        }
        Command::NextMessage => state.with_timeline(|timeline| timeline.jump_message(true)),
        Command::PreviousMessage => state.with_timeline(|timeline| timeline.jump_message(false)),
        _ => return None,
    };
    Some(effects)
}

fn copy_effect(state: &mut AppState, text: String) -> Vec<Effect> {
    let value = crate::evil::RegisterValue {
        text: text.clone(),
        linewise: false,
    };
    state.registers.yank(None, value);
    state.registers.note_clipboard(&text);
    vec![Effect::Copy(text)]
}

fn open_targets(state: &mut AppState) -> Vec<Effect> {
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

fn focus_composer_insert(state: &mut AppState, at: InsertAt) -> Vec<Effect> {
    if state.open_interaction_panel() {
        state.focus = Focus::Interaction;
        return Vec::new();
    }
    if !state.composer_available() {
        if state.open_thread.is_some() {
            state.toast("This thread is archived", ToastLevel::Info);
        }
        return Vec::new();
    }
    state.focus = Focus::Composer;
    if let Some(thread_id) = state.open_thread.clone()
        && let Some(composer) = state.composers.get_mut(&thread_id)
    {
        composer.enter_insert();
        let length = composer.chars().len();
        let cursor = match at {
            InsertAt::LineStart => 0,
            InsertAt::Last => composer.last_insert.unwrap_or(length).min(length),
            _ => length,
        };
        composer.move_to(cursor, false);
    }
    Vec::new()
}

fn bindings_pane(state: &mut AppState, scopes: &[Scope], title: &str) {
    use ratatui::style::{Color, Modifier, Style};
    use ratatui::text::Span;
    let mut lines: Vec<Vec<Span<'static>>> = Vec::new();
    let heading = Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD);
    let key_style = Style::new().fg(Color::Cyan);
    for scope in scopes {
        let bindings = state.keymap.scope_bindings(*scope);
        let shown: Vec<_> = bindings
            .into_iter()
            .filter(|binding| {
                !matches!(*scope, Scope::Composer | Scope::Visual)
                    || binding.keys.len() == 1
                    || !matches!(
                        binding.act,
                        crate::keymap::Act::Cmd(Command::Edit(crate::vi::EditOp::Apply(..)))
                            | crate::keymap::Act::Cmd(Command::Edit(
                                crate::vi::EditOp::VisualObject(..)
                            ))
                            | crate::keymap::Act::Char(crate::vi::CharAct::ApplyFind { .. })
                    )
            })
            .collect();
        if shown.is_empty() {
            continue;
        }
        if !lines.is_empty() {
            lines.push(Vec::new());
        }
        lines.push(vec![Span::styled(
            format!("── {} ──", scope.label()),
            heading,
        )]);
        if matches!(*scope, Scope::Composer | Scope::Visual) {
            lines.push(vec![
                Span::styled(format!("{:<18}", "d c y > < gu gU g~"), key_style),
                Span::raw(" operators: then a motion, a text object (iw aw i\" a( ip …) or the same key for a line"),
            ]);
        }
        for binding in shown {
            lines.push(vec![
                Span::styled(
                    format!("{:<18}", crate::keymap::format_seq(&binding.keys)),
                    key_style,
                ),
                Span::raw(format!(" {}", binding.description())),
            ]);
        }
    }
    let direct: Vec<(KeyChord, Command)> = state.keymap.direct_chords().to_vec();
    if !direct.is_empty() {
        lines.push(Vec::new());
        lines.push(vec![Span::styled("── direct chords ──", heading)]);
        for (chord, command) in direct {
            lines.push(vec![
                Span::styled(format!("{:<18}", chord.to_string()), key_style),
                Span::raw(format!(" {}", command.description())),
            ]);
        }
    }
    state.open_text_pane(title.to_owned(), lines);
}

const SETTINGS_KEEP: &[Command] = &[
    Command::Settings,
    Command::PaletteOpen,
    Command::Help,
    Command::KeymapOverview,
    Command::DescribeKey,
    Command::Messages,
    Command::Quit,
    Command::Reload,
    Command::Back,
    Command::WindowFocus(crate::keymap::Direction::Left),
    Command::WindowFocus(crate::keymap::Direction::Right),
    Command::WindowFocus(crate::keymap::Direction::Up),
    Command::WindowFocus(crate::keymap::Direction::Down),
    Command::PanelNextTab,
    Command::PanelPreviousTab,
    Command::ExLine,
    Command::QuitHint,
    Command::CloseWindowOrQuit,
    Command::PaneClose,
];

const AUTOMATIONS_KEEP: &[Command] = &[
    Command::Automations,
    Command::NewAutomation,
    Command::PaletteOpen,
    Command::Help,
    Command::KeymapOverview,
    Command::DescribeKey,
    Command::Messages,
    Command::Quit,
    Command::Reload,
    Command::Back,
    Command::ExLine,
    Command::QuitHint,
    Command::CloseWindowOrQuit,
    Command::PaneClose,
    Command::WindowFocus(crate::keymap::Direction::Left),
    Command::WindowFocus(crate::keymap::Direction::Right),
    Command::WindowFocus(crate::keymap::Direction::Up),
    Command::WindowFocus(crate::keymap::Direction::Down),
];

const PANE_KEEP: &[Command] = &[
    Command::Help,
    Command::KeymapOverview,
    Command::DescribeKey,
    Command::Messages,
    Command::Quit,
    Command::Back,
    Command::ExLine,
    Command::QuitHint,
    Command::CloseWindowOrQuit,
    Command::PaneClose,
    Command::WindowFocus(crate::keymap::Direction::Left),
    Command::WindowFocus(crate::keymap::Direction::Right),
    Command::WindowFocus(crate::keymap::Direction::Up),
    Command::WindowFocus(crate::keymap::Direction::Down),
];

pub(crate) fn run_command(state: &mut AppState, command: Command) -> Vec<Effect> {
    if state.result_pane.is_some() && !PANE_KEEP.contains(&command) {
        state.result_pane = None;
    }
    if state.settings.is_some()
        && !matches!(command, Command::SettingsSection(_))
        && !SETTINGS_KEEP.contains(&command)
    {
        state.close_settings();
    }
    if state.automations.is_some() && !AUTOMATIONS_KEEP.contains(&command) {
        state.close_automations();
    }
    if matches!(state.mode, Mode::Notifications { .. })
        && !matches!(
            command,
            Command::Back | Command::Notifications | Command::DescribeKey | Command::QuitHint
        )
    {
        state.mode = Mode::Normal;
    }
    let in_sidebar = state.focus == Focus::Sidebar && state.sidebar_visible;
    if let Some(effects) = crate::new_thread::new_thread_command(state, command) {
        return effects;
    }
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
        Command::Help => {
            let mut scopes = normal_scopes(state);
            scopes.push(Scope::Global);
            let title = format!(
                "Bindings: {}",
                scopes.first().map_or("global", |scope| scope.label())
            );
            bindings_pane(state, &scopes, &title);
        }
        Command::KeymapOverview => bindings_pane(state, Scope::ALL, "Keymap overview"),
        Command::Back => return keyboard_quit(state),
        Command::FocusNext => return state.cycle_focus(true, 1),
        Command::FocusPrevious => return state.cycle_focus(false, 1),
        Command::MoveDown if in_sidebar => state.move_selection(1),
        Command::MoveUp if in_sidebar => state.move_selection(-1),
        Command::MoveTop if in_sidebar => state.selected = 0,
        Command::MoveBottom if in_sidebar => state.selected = state.rows.len().saturating_sub(1),
        Command::Open if in_sidebar => match state.selected_row() {
            Some(SidebarRow::Group(_)) => state.toggle_group(None),
            Some(SidebarRow::Thread(row)) => {
                let id = row.id.clone();
                return state.open(id);
            }
            None => {}
        },
        Command::OpenFocus | Command::OpenInsert if in_sidebar => {
            let Some(id) = state.selected_thread_id().map(str::to_owned) else {
                if command == Command::OpenFocus {
                    state.toggle_group(None);
                }
                return Vec::new();
            };
            let effects = state.open(id);
            state.focus = Focus::Main;
            if command == Command::OpenInsert {
                let mut effects = effects;
                effects.extend(focus_composer_insert(state, InsertAt::LineEnd));
                return effects;
            }
            return effects;
        }
        Command::Collapse if in_sidebar => state.toggle_group(Some(false)),
        Command::Expand if in_sidebar => match state.selected_row().cloned() {
            Some(SidebarRow::Thread(thread)) if thread.child_count == 0 || thread.expanded => {
                return state.open(thread.id);
            }
            Some(_) => state.toggle_group(Some(true)),
            None => {}
        },
        Command::ToggleGroup if in_sidebar => state.toggle_group(None),
        Command::FoldAll(open) if in_sidebar => {
            let groups: Vec<String> = state
                .rows
                .iter()
                .filter_map(|row| match row {
                    SidebarRow::Group(group) => Some(group.key.clone()),
                    SidebarRow::Thread(_) => None,
                })
                .collect();
            if open {
                state.collapsed.clear();
            } else {
                state.collapsed.extend(groups);
            }
            state.rebuild_rows();
        }
        Command::FoldAll(open) if state.focus == Focus::Panel => {
            return state.diff_fold_all(open);
        }
        Command::ScrollLine(_) | Command::ScrollCursor(_) | Command::ViewJump(_) if in_sidebar => {
            sidebar_scroll(state, command);
        }
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
        Command::YankTitle | Command::YankId => {
            if let Some(thread_id) = state.target_thread() {
                let text = if command == Command::YankId {
                    thread_id
                } else {
                    state.thread_title(&thread_id)
                };
                return copy_effect(state, text);
            }
        }
        Command::NextPending => return state.jump_to_pending(),
        Command::Settings => return state.open_settings(None),
        Command::Automations => return state.open_automations(),
        Command::NewAutomation => return state.new_automation(),
        Command::SplitOpen => return state.open_in_split(),
        Command::SplitNew => {
            let effects = state.open_new_thread();
            state.split_new_thread = state.new_thread.is_some();
            return effects;
        }
        Command::PaneLeft => return state.step_pane(-1),
        Command::PaneRight => return state.step_pane(1),
        Command::PaneFocus(n) => {
            if state.is_split() {
                return state.focus_pane(usize::from(n.saturating_sub(1)));
            }
        }
        Command::PaneClose => return state.close_window(),
        Command::PaneMaximize => state.toggle_pane_maximized(),
        Command::WindowFocus(direction) => return state.focus_direction(direction),
        Command::WindowLast => return state.focus_last_window(),
        Command::WindowTop => return state.focus_edge_window(true),
        Command::WindowBottom => return state.focus_edge_window(false),
        Command::WindowOnly => return state.window_only(),
        Command::WindowRestore => return state.restore_windows(),
        Command::WindowSwap => return state.swap_pane(),
        Command::WindowRotate => return state.rotate_panes(1),
        Command::WindowMoveFar(left) => return state.move_pane_far(left),
        Command::WindowResize(sign) => state.resize_side(sign, None),
        Command::Zen => {
            if state.focus == Focus::Panel {
                state.focus = Focus::Main;
            }
            return state.window_only();
        }
        Command::SettingsSection(section) => return state.open_settings(Some(section)),
        Command::Notifications => state.open_notifications(),
        Command::ClearNotifications => {
            state.notifications.clear();
            state.keys.message("Notifications cleared");
        }
        Command::PaletteOpen => return state.open_palette(PaletteKind::Commands),
        Command::PaletteThreadGroup => {
            return state.open_palette_with(PaletteKind::Commands, "thread");
        }
        Command::ResumeMinibuffer => return state.resume_palette(),
        Command::QuickOpen => return state.open_palette(PaletteKind::Files),
        Command::SearchThreads => return state.open_palette(PaletteKind::Threads),
        Command::SwitchProject => return state.open_palette(PaletteKind::Projects),
        Command::NewThread => return state.open_new_thread(),
        Command::NewThreadInEnvironment => return state.open_new_thread_in_environment(),
        Command::ThreadNext => return state.step_thread(true),
        Command::ThreadPrevious => return state.step_thread(false),
        Command::AlternateThread => match state.alternate_thread.clone() {
            Some(thread_id) => return state.open(thread_id),
            None => state.keys.error("E23: No alternate thread"),
        },
        Command::StepWaiting(forward) => return step_marked(state, forward, true),
        Command::StepUnread(forward) => return step_marked(state, forward, false),
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
        Command::RevealInSidebar => {
            state.sidebar_visible = true;
            state.focus = Focus::Sidebar;
            if let Some(thread_id) = state.open_thread.clone() {
                let key = RowKey::Thread(thread_id);
                if !state.rows.iter().any(|row| row.key() == key) {
                    state.set_filter("");
                    state.collapsed.clear();
                    state.rebuild_rows();
                }
                state.select_key(&key);
            }
        }
        Command::PanelToggle => {
            state.panel_visible = !state.panel_visible;
            if !state.panel_visible {
                state.panel.maximized = false;
                if state.focus == Focus::Panel {
                    state.focus = Focus::Main;
                }
            }
            return state.sync_panel();
        }
        Command::StatusPanel => return state.set_panel_tab(crate::panel::PanelTab::Info),
        Command::DiffToggle => return state.toggle_diff_panel(),
        Command::DiffHunk(forward) => return state.diff_hunk(forward),
        Command::FileYankPath => match state.previewed_path() {
            Some(path) => return copy_effect(state, path),
            None => state.keys.error("No previewed file"),
        },
        Command::FileEdit => return state.edit_previewed(),
        Command::TerminalOpen => return state.open_terminal_panel(),
        Command::TerminalNew => {
            let mut effects = state.open_terminal_panel();
            effects.extend(state.create_terminal());
            return effects;
        }
        Command::TerminalAttach => {
            let mut effects = state.open_terminal_panel();
            if state
                .terminals
                .active_view()
                .is_some_and(|view| view.is_connected())
            {
                effects.extend(state.start_fullscreen());
            } else {
                state.toast(
                    "Connecting to the terminal · press SPC o f again to attach",
                    ToastLevel::Info,
                );
            }
            return effects;
        }
        Command::PanelNextTab | Command::PanelPreviousTab if state.settings.is_some() => {
            let forward = command == Command::PanelNextTab;
            let sections = crate::settings::SettingsSection::ALL;
            let index = state
                .settings
                .as_ref()
                .map_or(0, |view| view.section.index());
            let next = if forward {
                (index + 1) % sections.len()
            } else {
                (index + sections.len() - 1) % sections.len()
            };
            return state.set_section(sections[next]);
        }
        Command::PanelNextTab => return state.cycle_panel_tab(true),
        Command::PanelPreviousTab => return state.cycle_panel_tab(false),
        Command::PanelFullScreen => {
            state.toggle_panel_maximized();
            return state.sync_panel();
        }
        Command::GitCommit => return state.git_command(GitAction::Commit),
        Command::PullRequestMerge => return state.git_command(GitAction::Merge),
        Command::PullRequestReady => return state.git_command(GitAction::MarkReady),
        Command::PullRequestDraft => return state.git_command(GitAction::ConvertToDraft),
        Command::Reload => {
            state.toast("Reloading", ToastLevel::Info);
            return vec![Effect::RefreshAll];
        }
        Command::PageDown if in_sidebar => state.move_selection(SIDEBAR_PAGE),
        Command::PageUp if in_sidebar => state.move_selection(-SIDEBAR_PAGE),
        Command::HalfPageDown if in_sidebar => state.move_selection(SIDEBAR_PAGE / 2),
        Command::HalfPageUp if in_sidebar => state.move_selection(-SIDEBAR_PAGE / 2),
        Command::Copy => match state.timeline().and_then(TimelineView::copy_text) {
            Some(text) => return copy_effect(state, text),
            None => state.toast("Select a row to yank (j/k)", ToastLevel::Info),
        },
        Command::YankTimeline => match state.timeline().map(TimelineView::all_text) {
            Some(text) if !text.is_empty() => return copy_effect(state, text),
            _ => state.keys.error("Nothing to yank"),
        },
        Command::OpenExternal => return open_targets(state),
        Command::ToggleFollow => {
            let following = state
                .timeline()
                .is_some_and(|timeline| timeline.scroll == crate::timeline::Scroll::Bottom);
            if following {
                state.with_timeline(|timeline| {
                    timeline.stop_following();
                    Vec::new()
                });
                state.keys.message("Following off");
            } else {
                let effects = state.with_timeline(|timeline| timeline.select_edge(false));
                state.keys.message("Following new output");
                return effects;
            }
        }
        Command::ComposerInsert(at) => return focus_composer_insert(state, at),
        Command::ComposerToggle => {
            if matches!(state.focus, Focus::Composer | Focus::Interaction) {
                state.focus = Focus::Main;
            } else {
                return focus_composer_insert(state, InsertAt::LineEnd);
            }
        }
        Command::ComposerMention => {
            let mut effects = focus_composer_insert(state, InsertAt::LineEnd);
            let now = state.clock;
            if state.focus == Focus::Composer
                && let Some(thread_id) = state.open_thread.clone()
                && let Some(composer) = state.composers.get_mut(&thread_id)
            {
                let text = composer.text();
                let lead = if text.is_empty() || text.ends_with(char::is_whitespace) {
                    "@"
                } else {
                    " @"
                };
                composer.insert_text(lead, now);
            }
            effects.extend(sync_active_typeahead(state));
            return effects;
        }
        Command::Send | Command::Steer => return send_draft(state, command == Command::Steer),
        Command::ExLine => state.open_ex(),
        Command::CloseWindowOrQuit => {
            if state.can_close_window() {
                return state.close_window();
            }
            return quit(state);
        }
        Command::SendAndClose => {
            let mut effects = send_draft(state, false);
            effects.extend(run_command(state, Command::CloseWindowOrQuit));
            return effects;
        }
        Command::QuitHint => state.keys.message("Quit with SPC q q or :q"),
        Command::DescribeKey => {
            state.keys.describe = true;
            state.keys.message("Describe key: type a key sequence");
        }
        Command::Messages => {
            let lines = state.messages.clone();
            state.show_text_pane("Messages", lines);
        }
        Command::Edit(op) => return crate::edit::apply_edit(state, op, None),
        Command::MoveDown
        | Command::MoveUp
        | Command::MoveTop
        | Command::MoveBottom
        | Command::Open
        | Command::OpenFocus
        | Command::OpenInsert
        | Command::Collapse
        | Command::Expand
        | Command::ToggleGroup
        | Command::FoldAll(_)
        | Command::ScrollLine(_)
        | Command::ScrollCursor(_)
        | Command::ViewJump(_)
        | Command::PageDown
        | Command::PageUp
        | Command::HalfPageDown
        | Command::HalfPageUp
        | Command::NextMessage
        | Command::PreviousMessage
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
        | Command::AttachFile
        | Command::Attachments
        | Command::ToggleSubmitMode => {}
    }
    Vec::new()
}

fn send_draft(state: &mut AppState, steer: bool) -> Vec<Effect> {
    if state.focus == Focus::NewThread
        && state
            .new_thread
            .as_ref()
            .is_some_and(|form| form.prompt_active)
    {
        return match crate::compose::take_attach_command(state, &ComposerTarget::NewThread) {
            Some(effects) => effects,
            None => state.submit_new_thread(),
        };
    }
    let Some(thread_id) = state.open_thread.clone() else {
        state.toast("Open a thread first", ToastLevel::Info);
        return Vec::new();
    };
    if !state.composer_available() {
        state.toast("This thread is archived", ToastLevel::Info);
        return Vec::new();
    }
    if !steer
        && let Some(effects) =
            crate::compose::take_attach_command(state, &ComposerTarget::Thread(thread_id))
    {
        return effects;
    }
    state.submit(steer)
}
