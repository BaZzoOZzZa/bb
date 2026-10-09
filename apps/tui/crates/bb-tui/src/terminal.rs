use std::cell::Cell;
use std::collections::HashMap;

use bb_client::{TerminalConnectionState, TerminalEvent};
use bb_wire::terminal::{
    CloseTerminalRequest, CreateTerminalRequest, TERMINAL_COLS_MAX, TERMINAL_ROWS_MAX,
    TerminalCloseMode, TerminalCreateTarget, TerminalListQuery, TerminalServerMessage,
    TerminalSession, TerminalSessionStatus,
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::{AppState, Effect, Focus, ToastLevel};
use crate::execution::Load;
use crate::panel::PanelTab;
use crate::terminal_input::{KeyModes, encode_key, encode_paste, is_detach_key};

pub const SCROLLBACK_LINES: usize = 2_000;
pub const DEFAULT_COLS: u16 = 100;
pub const DEFAULT_ROWS: u16 = 30;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalScope {
    pub thread_id: String,
    pub environment_id: Option<String>,
}

impl TerminalScope {
    pub fn queries(&self) -> Vec<TerminalListQuery> {
        let mut queries = vec![TerminalListQuery {
            thread_id: Some(self.thread_id.clone()),
            ..TerminalListQuery::default()
        }];
        if let Some(environment_id) = &self.environment_id {
            queries.push(TerminalListQuery {
                environment_id: Some(environment_id.clone()),
                ..TerminalListQuery::default()
            });
        }
        queries
    }

    pub fn create_request(&self, cols: u16, rows: u16) -> CreateTerminalRequest {
        CreateTerminalRequest {
            cols,
            rows,
            start: None,
            target: TerminalCreateTarget::Thread {
                thread_id: self.thread_id.clone(),
            },
            title: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum TerminalRequest {
    List {
        scope: TerminalScope,
    },
    Create {
        scope: TerminalScope,
        request: CreateTerminalRequest,
    },
    Close {
        terminal_id: String,
        request: CloseTerminalRequest,
    },
    Connect {
        terminal_id: String,
        link: u64,
        since_seq: u64,
        size: Option<(u16, u16)>,
        visible: bool,
    },
    Disconnect {
        terminal_id: String,
        link: u64,
    },
    Input {
        terminal_id: String,
        link: u64,
        bytes: Vec<u8>,
    },
    Resize {
        terminal_id: String,
        link: u64,
        cols: u16,
        rows: u16,
    },
    Ack {
        terminal_id: String,
        link: u64,
        next_seq: u64,
    },
    Visible {
        terminal_id: String,
        link: u64,
        visible: bool,
    },
    Fullscreen {
        terminal_id: String,
        link: u64,
    },
}

impl TerminalRequest {
    pub fn read_only_refusal(&self) -> Option<Option<&'static str>> {
        match self {
            Self::Create { .. } => Some(Some("create a terminal")),
            Self::Close { .. } => Some(Some("close a terminal")),
            Self::Input { .. } => Some(Some("type into a terminal")),
            Self::Resize { .. } => Some(None),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum TerminalLoaded {
    List {
        scope: TerminalScope,
        result: Result<Vec<TerminalSession>, String>,
    },
    Created {
        scope: TerminalScope,
        result: Result<TerminalSession, String>,
    },
    Closed {
        terminal_id: String,
        result: Result<TerminalSession, String>,
    },
    ConnectFailed {
        terminal_id: String,
        link: u64,
        error: String,
    },
}

#[derive(Debug, Default)]
pub struct ScreenEvents {
    pub title: Option<String>,
    pub bell: bool,
}

impl vt100::Callbacks for ScreenEvents {
    fn audible_bell(&mut self, _: &mut vt100::Screen) {
        self.bell = true;
    }

    fn set_window_title(&mut self, _: &mut vt100::Screen, title: &[u8]) {
        let title = String::from_utf8_lossy(title).trim().to_owned();
        self.title = (!title.is_empty()).then_some(title);
    }
}

pub type Screen = vt100::Parser<ScreenEvents>;

fn new_screen(rows: u16, cols: u16) -> Screen {
    vt100::Parser::new_with_callbacks(
        rows.max(1),
        cols.max(1),
        SCROLLBACK_LINES,
        ScreenEvents::default(),
    )
}

fn session_size(session: &TerminalSession) -> (u16, u16) {
    let cols = if session.cols == 0 {
        DEFAULT_COLS
    } else {
        session.cols
    };
    let rows = if session.rows == 0 {
        DEFAULT_ROWS
    } else {
        session.rows
    };
    (cols, rows)
}

pub fn clamp_size(cols: u16, rows: u16) -> (u16, u16) {
    (
        cols.clamp(2, TERMINAL_COLS_MAX),
        rows.clamp(1, TERMINAL_ROWS_MAX),
    )
}

pub struct TerminalView {
    pub session: TerminalSession,
    pub screen: Screen,
    pub link: Option<u64>,
    pub connection: TerminalConnectionState,
    pub next_seq: u64,
    pub requested: Option<(u16, u16)>,
    pub visible: bool,
    pub notice: Option<String>,
    pub attached_once: bool,
}

impl std::fmt::Debug for TerminalView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TerminalView")
            .field("session", &self.session.id)
            .field("link", &self.link)
            .field("connection", &self.connection)
            .field("next_seq", &self.next_seq)
            .field("requested", &self.requested)
            .field("notice", &self.notice)
            .finish()
    }
}

impl TerminalView {
    pub fn new(session: TerminalSession) -> Self {
        let (cols, rows) = session_size(&session);
        Self {
            session,
            screen: new_screen(rows, cols),
            link: None,
            connection: TerminalConnectionState::Closed,
            next_seq: 0,
            requested: None,
            visible: true,
            notice: None,
            attached_once: false,
        }
    }

    pub fn is_running(&self) -> bool {
        self.session.status == TerminalSessionStatus::Running
            || self.session.status == TerminalSessionStatus::Starting
    }

    pub fn is_connected(&self) -> bool {
        self.link.is_some()
    }

    pub fn title(&self) -> String {
        self.screen
            .callbacks()
            .title
            .clone()
            .filter(|_| self.is_running())
            .unwrap_or_else(|| {
                if self.session.title.trim().is_empty() {
                    "Terminal".into()
                } else {
                    self.session.title.clone()
                }
            })
    }

    pub fn modes(&self) -> KeyModes {
        KeyModes {
            application_cursor: self.screen.screen().application_cursor(),
        }
    }

    pub fn scrollback(&self) -> usize {
        self.screen.screen().scrollback()
    }

    pub fn scroll_by(&mut self, delta: isize) {
        let next = self.scrollback().saturating_add_signed(delta);
        self.screen.screen_mut().set_scrollback(next);
    }

    pub fn scroll_to(&mut self, offset: usize) {
        self.screen.screen_mut().set_scrollback(offset);
    }

    pub fn set_size(&mut self, cols: u16, rows: u16) {
        let (cols, rows) = clamp_size(cols, rows);
        if self.screen.screen().size() != (rows, cols) {
            self.screen.screen_mut().set_size(rows, cols);
        }
    }

    fn write_status(&mut self, text: &str) {
        self.screen
            .process(format!("\r\n\x1b[2m{text}\x1b[0m\r\n").as_bytes());
    }

    fn reset_screen(&mut self) {
        let (rows, cols) = self.screen.screen().size();
        let title = self.screen.callbacks().title.clone();
        self.screen = new_screen(rows, cols);
        self.screen.callbacks_mut().title = title;
    }
}

pub fn exit_text(session: &TerminalSession) -> String {
    match session.exit_code {
        Some(code) => format!("Terminal exited with code {code}"),
        None => "Terminal exited".into(),
    }
}

pub fn is_listed(session: &TerminalSession) -> bool {
    session.status.is_active() || session.status == TerminalSessionStatus::Disconnected
}

#[derive(Debug, Default)]
pub struct TerminalPanel {
    pub scope: Option<TerminalScope>,
    pub sessions: Load<Vec<TerminalSession>>,
    pub listing: bool,
    pub active: Option<String>,
    pub views: HashMap<String, TerminalView>,
    pub input: bool,
    pub creating: bool,
    pub confirm_close: Option<String>,
    pub fullscreen: Option<String>,
    pub viewport: Cell<Option<(u16, u16)>>,
    pub next_link: u64,
}

impl TerminalPanel {
    pub fn active_view(&self) -> Option<&TerminalView> {
        self.views.get(self.active.as_deref()?)
    }

    pub fn active_view_mut(&mut self) -> Option<&mut TerminalView> {
        self.views.get_mut(self.active.as_deref()?)
    }

    pub fn session_ids(&self) -> Vec<String> {
        self.sessions
            .loaded()
            .map(|sessions| sessions.iter().map(|session| session.id.clone()).collect())
            .unwrap_or_default()
    }

    fn upsert_session(&mut self, session: &TerminalSession) {
        if let Load::Loaded(sessions) = &mut self.sessions {
            match sessions.iter_mut().find(|entry| entry.id == session.id) {
                Some(entry) => *entry = session.clone(),
                None if is_listed(session) => sessions.push(session.clone()),
                None => {}
            }
        }
    }
}

fn link_request(
    view: &TerminalView,
    terminal_id: &str,
    make: impl FnOnce(String, u64) -> TerminalRequest,
) -> Option<Effect> {
    view.link
        .map(|link| Effect::Terminal(make(terminal_id.to_owned(), link)))
}

impl AppState {
    pub fn terminal_scope(&self) -> Option<TerminalScope> {
        let thread_id = self.panel_thread()?.to_owned();
        let environment_id = self
            .snapshot
            .thread(&thread_id)
            .and_then(|detail| detail.thread.environment_id.clone())
            .or_else(|| {
                self.sidebar_entry(&thread_id)
                    .and_then(|entry| entry.thread.environment_id.clone())
            });
        Some(TerminalScope {
            thread_id,
            environment_id,
        })
    }

    pub fn terminal_showing(&self) -> bool {
        self.panel_visible && self.panel.tab == PanelTab::Terminal
    }

    pub fn terminal_input_active(&self) -> bool {
        self.terminal_showing()
            && self.focus == Focus::Panel
            && self.terminals.input
            && self.terminals.confirm_close.is_none()
            && self
                .terminals
                .active_view()
                .is_some_and(|view| view.is_connected() && view.is_running())
    }

    fn disconnect_all_terminals(&mut self) -> Vec<Effect> {
        self.terminals
            .views
            .iter_mut()
            .filter_map(|(id, view)| {
                let link = view.link.take()?;
                view.connection = TerminalConnectionState::Closed;
                Some(Effect::Terminal(TerminalRequest::Disconnect {
                    terminal_id: id.clone(),
                    link,
                }))
            })
            .collect()
    }

    pub fn sync_terminals(&mut self) -> Vec<Effect> {
        let scope = self.terminal_scope();
        let mut effects = Vec::new();
        if scope != self.terminals.scope {
            effects.extend(self.disconnect_all_terminals());
            self.terminals = TerminalPanel {
                scope: scope.clone(),
                next_link: self.terminals.next_link,
                ..TerminalPanel::default()
            };
        }
        let Some(scope) = scope else {
            return effects;
        };
        let showing = self.terminal_showing();
        if showing && self.terminals.sessions.is_idle() && !self.terminals.listing {
            self.terminals.listing = true;
            self.terminals.sessions = Load::Loading;
            effects.push(Effect::Terminal(TerminalRequest::List { scope }));
        }
        if showing
            && self.terminals.active.is_none()
            && let Some(first) = self.terminals.session_ids().into_iter().next()
        {
            self.terminals.active = Some(first);
        }
        if showing {
            effects.extend(self.connect_active_terminal());
        }
        let visible = showing || self.terminals.fullscreen.is_some();
        if let Some(id) = self.terminals.active.clone()
            && let Some(view) = self.terminals.views.get_mut(&id)
            && view.visible != visible
        {
            view.visible = visible;
            effects.extend(link_request(view, &id, |terminal_id, link| {
                TerminalRequest::Visible {
                    terminal_id,
                    link,
                    visible,
                }
            }));
        }
        effects
    }

    fn connect_active_terminal(&mut self) -> Vec<Effect> {
        let Some(id) = self.terminals.active.clone() else {
            return Vec::new();
        };
        let Some(session) = self
            .terminals
            .sessions
            .loaded()
            .and_then(|sessions| sessions.iter().find(|session| session.id == id))
            .cloned()
        else {
            return Vec::new();
        };
        let viewport = self.terminals.viewport.get();
        let read_only = self.read_only;
        let link = self.terminals.next_link + 1;
        let view = self
            .terminals
            .views
            .entry(id.clone())
            .or_insert_with(|| TerminalView::new(session.clone()));
        if view.is_connected() || !is_listed(&session) {
            return Vec::new();
        }
        self.terminals.next_link = link;
        view.session = session;
        view.link = Some(link);
        view.visible = true;
        view.connection = TerminalConnectionState::Connecting;
        let size = match viewport {
            Some((cols, rows)) if !read_only => {
                let size = clamp_size(cols, rows);
                view.requested = Some(size);
                Some(size)
            }
            _ => None,
        };
        vec![Effect::Terminal(TerminalRequest::Connect {
            terminal_id: id,
            link,
            since_seq: view.next_seq,
            size,
            visible: true,
        })]
    }

    fn switch_terminal(&mut self, forward: bool) -> Vec<Effect> {
        let ids = self.terminals.session_ids();
        if ids.len() < 2 {
            return Vec::new();
        }
        let index = self
            .terminals
            .active
            .as_ref()
            .and_then(|active| ids.iter().position(|id| id == active))
            .unwrap_or(0);
        let next = if forward {
            (index + 1) % ids.len()
        } else {
            (index + ids.len() - 1) % ids.len()
        };
        self.select_terminal(ids[next].clone())
    }

    pub fn select_terminal(&mut self, terminal_id: String) -> Vec<Effect> {
        let mut effects = Vec::new();
        if self.terminals.active.as_deref() == Some(terminal_id.as_str()) {
            return self.sync_terminals();
        }
        if let Some(previous) = self.terminals.active.take()
            && let Some(view) = self.terminals.views.get_mut(&previous)
            && let Some(link) = view.link.take()
        {
            view.connection = TerminalConnectionState::Closed;
            effects.push(Effect::Terminal(TerminalRequest::Disconnect {
                terminal_id: previous,
                link,
            }));
        }
        self.terminals.active = Some(terminal_id);
        effects.extend(self.sync_terminals());
        effects
    }

    pub fn open_terminal_panel(&mut self) -> Vec<Effect> {
        if self.open_thread.is_none() {
            self.toast("Open a thread to use its terminals", ToastLevel::Info);
            return Vec::new();
        }
        self.panel_visible = true;
        self.panel.tab = PanelTab::Terminal;
        self.focus = Focus::Panel;
        self.terminals.input = true;
        self.sync_panel()
    }

    pub fn terminals_loaded(&mut self, loaded: TerminalLoaded) -> Vec<Effect> {
        match loaded {
            TerminalLoaded::List { scope, result } => {
                if self.terminals.scope.as_ref() != Some(&scope) {
                    return Vec::new();
                }
                self.terminals.listing = false;
                match result {
                    Ok(mut sessions) => {
                        sessions.retain(is_listed);
                        sessions.sort_by_key(|session| (session.created_at, session.id.clone()));
                        sessions.dedup_by(|a, b| a.id == b.id);
                        if self.terminals.active.as_ref().is_some_and(|active| {
                            !sessions.iter().any(|session| &session.id == active)
                        }) {
                            self.terminals.active = None;
                        }
                        for session in &sessions {
                            if let Some(view) = self.terminals.views.get_mut(&session.id) {
                                view.session = session.clone();
                            }
                        }
                        self.terminals.sessions = Load::Loaded(sessions);
                    }
                    Err(error) => self.terminals.sessions = Load::Failed(error),
                }
                self.sync_terminals()
            }
            TerminalLoaded::Created { scope, result } => {
                self.terminals.creating = false;
                if self.terminals.scope.as_ref() != Some(&scope) {
                    return Vec::new();
                }
                match result {
                    Ok(session) => {
                        let id = session.id.clone();
                        if self.terminals.sessions.loaded().is_none() {
                            self.terminals.sessions = Load::Loaded(Vec::new());
                        }
                        self.terminals.upsert_session(&session);
                        self.terminals.input = true;
                        self.select_terminal(id)
                    }
                    Err(error) => {
                        self.toast(
                            format!("Failed to create terminal: {error}"),
                            ToastLevel::Error,
                        );
                        Vec::new()
                    }
                }
            }
            TerminalLoaded::Closed {
                terminal_id,
                result,
            } => match result {
                Ok(_) => {
                    let mut effects = Vec::new();
                    if let Some(mut view) = self.terminals.views.remove(&terminal_id)
                        && let Some(link) = view.link.take()
                    {
                        effects.push(Effect::Terminal(TerminalRequest::Disconnect {
                            terminal_id: terminal_id.clone(),
                            link,
                        }));
                    }
                    if let Load::Loaded(sessions) = &mut self.terminals.sessions {
                        sessions.retain(|session| session.id != terminal_id);
                    }
                    if self.terminals.active.as_deref() == Some(terminal_id.as_str()) {
                        self.terminals.active = None;
                    }
                    self.toast("Terminal closed", ToastLevel::Info);
                    effects.extend(self.sync_terminals());
                    effects
                }
                Err(error) => {
                    self.toast(
                        format!("Failed to close terminal: {error}"),
                        ToastLevel::Error,
                    );
                    Vec::new()
                }
            },
            TerminalLoaded::ConnectFailed {
                terminal_id,
                link,
                error,
            } => {
                if let Some(view) = self.terminals.views.get_mut(&terminal_id)
                    && view.link == Some(link)
                {
                    view.link = None;
                    view.connection = TerminalConnectionState::Closed;
                    view.notice = Some(error);
                }
                Vec::new()
            }
        }
    }

    pub fn terminal_event(
        &mut self,
        terminal_id: String,
        link: u64,
        event: TerminalEvent,
    ) -> Vec<Effect> {
        let Some(view) = self.terminals.views.get_mut(&terminal_id) else {
            return Vec::new();
        };
        if view.link != Some(link) {
            return Vec::new();
        }
        let mut effects = Vec::new();
        let mut session_changed = None;
        match event {
            TerminalEvent::State(state) => {
                view.connection = state;
                match state {
                    TerminalConnectionState::Attached => {
                        view.attached_once = true;
                        if view.notice.as_deref() == Some("Reconnecting…") {
                            view.notice = None;
                        }
                    }
                    TerminalConnectionState::Reconnecting => {
                        view.notice = Some("Reconnecting…".into());
                    }
                    TerminalConnectionState::Closed => view.link = None,
                    TerminalConnectionState::Connecting => {}
                }
            }
            TerminalEvent::Message(message) => match message {
                TerminalServerMessage::Attached { session, .. }
                | TerminalServerMessage::SessionUpdated { session } => {
                    if !session.id.is_empty() {
                        session_changed = Some(session);
                    }
                }
                TerminalServerMessage::Output { chunk } => {
                    if chunk.seq >= view.next_seq {
                        match chunk.decode() {
                            Ok(bytes) => view.screen.process(&bytes),
                            Err(error) => tracing::debug!(%error, "invalid terminal output"),
                        }
                        view.next_seq = chunk.seq + 1;
                        effects.push(Effect::Terminal(TerminalRequest::Ack {
                            terminal_id: terminal_id.clone(),
                            link,
                            next_seq: view.next_seq,
                        }));
                    }
                }
                TerminalServerMessage::Exited { session } => {
                    let session = if session.id.is_empty() {
                        TerminalSession {
                            status: TerminalSessionStatus::Exited,
                            ..view.session.clone()
                        }
                    } else {
                        session
                    };
                    let text = exit_text(&session);
                    view.write_status(&text);
                    view.notice = Some(text);
                    session_changed = Some(session);
                }
                TerminalServerMessage::Error { message, .. } => {
                    let text = format!("Terminal error: {message}");
                    view.write_status(&text);
                    view.notice = Some(text);
                }
                TerminalServerMessage::Pong | TerminalServerMessage::Unknown(_) => {}
            },
            TerminalEvent::SequenceGap { expected, received } => {
                view.reset_screen();
                view.next_seq = received;
                if expected > 0 || view.attached_once {
                    let text = "Some terminal output was unavailable after reconnect";
                    view.write_status(text);
                    view.notice = Some(text.into());
                }
            }
            TerminalEvent::InvalidMessage(text) => {
                tracing::debug!(%text, "invalid terminal message");
            }
            TerminalEvent::InputOverflow { max_bytes } => {
                let text =
                    format!("Terminal input queue is full ({max_bytes} bytes); input was not sent");
                view.notice = Some(text.clone());
                self.toast(text, ToastLevel::Error);
                return effects;
            }
            TerminalEvent::ConnectFailed(error) => {
                view.notice = Some(error);
            }
        }
        if let Some(session) = session_changed {
            let (cols, rows) = session_size(&session);
            view.set_size(cols, rows);
            view.session = session.clone();
            if !view.is_running() {
                self.terminals.input = false;
                if self.terminals.fullscreen.as_deref() == Some(terminal_id.as_str()) {
                    self.terminals.fullscreen = None;
                }
            }
            self.terminals.upsert_session(&session);
        }
        if let Some(view) = self.terminals.views.get(&terminal_id)
            && !view.is_connected()
            && self.terminals.fullscreen.as_deref() == Some(terminal_id.as_str())
        {
            self.terminals.fullscreen = None;
        }
        effects
    }

    pub fn terminals_tick(&mut self) -> Vec<Effect> {
        if self.read_only || self.terminals.fullscreen.is_some() || !self.terminal_showing() {
            return Vec::new();
        }
        let Some((cols, rows)) = self.terminals.viewport.get() else {
            return Vec::new();
        };
        let size = clamp_size(cols, rows);
        let Some(id) = self.terminals.active.clone() else {
            return Vec::new();
        };
        let Some(view) = self.terminals.views.get_mut(&id) else {
            return Vec::new();
        };
        if !view.is_connected() || view.requested == Some(size) {
            return Vec::new();
        }
        view.requested = Some(size);
        link_request(view, &id, |terminal_id, link| TerminalRequest::Resize {
            terminal_id,
            link,
            cols: size.0,
            rows: size.1,
        })
        .into_iter()
        .collect()
    }

    pub fn create_terminal(&mut self) -> Vec<Effect> {
        let Some(scope) = self.terminals.scope.clone() else {
            self.toast("Open a thread to create a terminal", ToastLevel::Info);
            return Vec::new();
        };
        if self.terminals.creating {
            return Vec::new();
        }
        let (cols, rows) = self
            .terminals
            .viewport
            .get()
            .map_or((DEFAULT_COLS, DEFAULT_ROWS), |(cols, rows)| {
                clamp_size(cols, rows)
            });
        if !self.read_only {
            self.terminals.creating = true;
        }
        vec![Effect::Terminal(TerminalRequest::Create {
            request: scope.create_request(cols, rows),
            scope,
        })]
    }

    pub fn start_fullscreen(&mut self) -> Vec<Effect> {
        let Some(id) = self.terminals.active.clone() else {
            self.toast("No terminal to attach", ToastLevel::Info);
            return Vec::new();
        };
        let Some(view) = self.terminals.views.get_mut(&id) else {
            return Vec::new();
        };
        let Some(link) = view.link.filter(|_| view.is_running()) else {
            self.toast("The terminal is not running", ToastLevel::Info);
            return Vec::new();
        };
        view.scroll_to(0);
        self.terminals.fullscreen = Some(id.clone());
        vec![Effect::Terminal(TerminalRequest::Fullscreen {
            terminal_id: id,
            link,
        })]
    }

    pub fn fullscreen_resize(&mut self, cols: u16, rows: u16) -> Vec<Effect> {
        let read_only = self.read_only;
        let Some(id) = self.terminals.fullscreen.clone() else {
            return Vec::new();
        };
        let Some(view) = self.terminals.views.get_mut(&id) else {
            return Vec::new();
        };
        let size = clamp_size(cols, rows);
        if read_only || view.requested == Some(size) {
            return Vec::new();
        }
        view.requested = Some(size);
        link_request(view, &id, |terminal_id, link| TerminalRequest::Resize {
            terminal_id,
            link,
            cols: size.0,
            rows: size.1,
        })
        .into_iter()
        .collect()
    }

    pub fn fullscreen_input(&mut self, bytes: Vec<u8>) -> Vec<Effect> {
        let Some(id) = self.terminals.fullscreen.clone() else {
            return Vec::new();
        };
        match self.terminals.views.get(&id) {
            Some(view) if !bytes.is_empty() => {
                link_request(view, &id, |terminal_id, link| TerminalRequest::Input {
                    terminal_id,
                    link,
                    bytes,
                })
                .into_iter()
                .collect()
            }
            _ => Vec::new(),
        }
    }

    pub fn fullscreen_ended(&mut self, message: String) -> Vec<Effect> {
        if let Some(id) = self.terminals.fullscreen.take()
            && let Some(view) = self.terminals.views.get_mut(&id)
        {
            view.requested = None;
        }
        if let Some(view) = self.terminals.active_view()
            && view.is_running()
            && view.is_connected()
        {
            self.panel_visible = true;
            self.panel.tab = PanelTab::Terminal;
            self.focus = Focus::Panel;
            self.terminals.input = true;
        }
        if !message.is_empty() {
            self.toast(message, ToastLevel::Info);
        }
        let mut effects = self.sync_panel();
        effects.extend(self.terminals_tick());
        effects
    }

    pub fn fullscreen_screen(&self) -> Option<Vec<u8>> {
        let id = self.terminals.fullscreen.as_deref()?;
        Some(
            self.terminals
                .views
                .get(id)?
                .screen
                .screen()
                .state_formatted(),
        )
    }

    pub fn terminal_paste(&mut self, text: &str) -> Option<Vec<Effect>> {
        if !self.terminal_input_active() {
            return None;
        }
        let id = self.terminals.active.clone()?;
        let view = self.terminals.views.get_mut(&id)?;
        let bytes = encode_paste(text, view.screen.screen().bracketed_paste());
        view.scroll_to(0);
        Some(
            link_request(view, &id, |terminal_id, link| TerminalRequest::Input {
                terminal_id,
                link,
                bytes,
            })
            .into_iter()
            .collect(),
        )
    }

    fn copy_terminal_screen(&mut self) -> Vec<Effect> {
        match self.terminals.active_view() {
            Some(view) => {
                let text = view.screen.screen().contents();
                let text = text.trim_end().to_owned();
                if text.is_empty() {
                    self.toast("The terminal screen is empty", ToastLevel::Info);
                    Vec::new()
                } else {
                    vec![Effect::Copy(text)]
                }
            }
            None => Vec::new(),
        }
    }
}

fn page(state: &AppState) -> isize {
    state
        .terminals
        .viewport
        .get()
        .map_or(DEFAULT_ROWS, |(_, rows)| rows)
        .max(2) as isize
        - 1
}

pub(crate) fn handle_terminal_input(state: &mut AppState, key: KeyEvent) -> Vec<Effect> {
    if is_detach_key(&key) {
        state.terminals.input = false;
        return Vec::new();
    }
    let page = page(state);
    let Some(id) = state.terminals.active.clone() else {
        return Vec::new();
    };
    let Some(view) = state.terminals.views.get_mut(&id) else {
        return Vec::new();
    };
    if key.modifiers.contains(KeyModifiers::SHIFT) {
        match key.code {
            KeyCode::PageUp => {
                view.scroll_by(page);
                return Vec::new();
            }
            KeyCode::PageDown => {
                view.scroll_by(-page);
                return Vec::new();
            }
            _ => {}
        }
    }
    let bytes = encode_key(&key, view.modes());
    if bytes.is_empty() {
        return Vec::new();
    }
    view.scroll_to(0);
    link_request(view, &id, |terminal_id, link| TerminalRequest::Input {
        terminal_id,
        link,
        bytes,
    })
    .into_iter()
    .collect()
}

pub(crate) fn handle_terminal_key(state: &mut AppState, key: &KeyEvent) -> Option<Vec<Effect>> {
    if let Some(terminal_id) = state.terminals.confirm_close.take() {
        let confirmed = matches!(key.code, KeyCode::Enter | KeyCode::Char('y'))
            && !key.modifiers.contains(KeyModifiers::CONTROL);
        if confirmed {
            return Some(vec![Effect::Terminal(TerminalRequest::Close {
                terminal_id,
                request: CloseTerminalRequest::by_user(TerminalCloseMode::Force),
            })]);
        }
        return Some(Vec::new());
    }
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let page = page(state);
    let scroll = |state: &mut AppState, delta: Option<isize>| {
        if let Some(view) = state.terminals.active_view_mut() {
            match delta {
                Some(delta) => view.scroll_by(delta),
                None => view.scroll_to(usize::MAX),
            }
        }
        Vec::new()
    };
    let effects = match key.code {
        KeyCode::Enter | KeyCode::Char('i') | KeyCode::Char('a') if !ctrl => {
            match state.terminals.active_view() {
                Some(view) if view.is_running() && view.is_connected() => {
                    state.terminals.input = true;
                }
                Some(view) if !view.is_running() => {
                    state.toast(
                        "The terminal is not running · n starts a new one",
                        ToastLevel::Info,
                    );
                }
                Some(_) => state.toast("The terminal is not connected", ToastLevel::Info),
                None => state.toast("No terminal · n starts one", ToastLevel::Info),
            }
            Vec::new()
        }
        KeyCode::Char('n') if !ctrl => state.create_terminal(),
        KeyCode::Char('x') | KeyCode::Delete if !ctrl => {
            match state.terminals.active.clone() {
                Some(id) => state.terminals.confirm_close = Some(id),
                None => state.toast("No terminal to close", ToastLevel::Info),
            }
            Vec::new()
        }
        KeyCode::Char(']') if !ctrl => state.switch_terminal(true),
        KeyCode::Char('[') if !ctrl => state.switch_terminal(false),
        KeyCode::Char('A') if !ctrl => state.start_fullscreen(),
        KeyCode::Char('y') if !ctrl => state.copy_terminal_screen(),
        KeyCode::Char('k') | KeyCode::Up if !ctrl => scroll(state, Some(1)),
        KeyCode::Char('j') | KeyCode::Down if !ctrl => scroll(state, Some(-1)),
        KeyCode::PageUp => scroll(state, Some(page)),
        KeyCode::PageDown | KeyCode::Char(' ') if !ctrl => scroll(state, Some(-page)),
        KeyCode::Char('u') if ctrl => scroll(state, Some(page / 2)),
        KeyCode::Char('d') if ctrl => scroll(state, Some(-page / 2)),
        KeyCode::Char('g') | KeyCode::Home if !ctrl => scroll(state, None),
        KeyCode::Char('G') | KeyCode::End if !ctrl => {
            if let Some(view) = state.terminals.active_view_mut() {
                view.scroll_to(0);
            }
            Vec::new()
        }
        _ => return None,
    };
    Some(effects)
}
