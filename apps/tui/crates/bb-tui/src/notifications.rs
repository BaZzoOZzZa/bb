use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use bb_store::StoreSnapshot;
use bb_wire::common::JsonValue;
use bb_wire::interaction::{
    ApprovalSubject, InteractionPayload, PendingInteraction, PendingInteractionStatus,
};
use bb_wire::plugin::{
    ClientNotification, PUSH_NOTIFICATION_CHANNEL, PUSH_NOTIFICATIONS_PLUGIN_ID,
};
use bb_wire::thread::{ThreadListEntry, ThreadStatus, ThreadVisibility};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::{AppState, Effect, Focus, Mode, ToastLevel};
use crate::config::{DesktopMethod, DesktopWhen, NotificationPrefs};

pub const COALESCE: Duration = Duration::from_secs(2);
pub const SIGNAL_DEDUPE: Duration = Duration::from_secs(30);
pub const MAX_ENTRIES: usize = 200;
const SEEN_SIGNALS: usize = 100;
const TITLE_MAX_CHARS: usize = 80;
const BODY_MAX_CHARS: usize = 180;
const APP_TITLE: &str = "bb";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NoticeKind {
    NeedsInput,
    Error,
    TurnFinished,
    Plugin,
}

const PRIORITY: [NoticeKind; 3] = [
    NoticeKind::NeedsInput,
    NoticeKind::Error,
    NoticeKind::TurnFinished,
];

impl NoticeKind {
    pub fn fallback_body(self) -> &'static str {
        match self {
            Self::NeedsInput => "Waiting for your input",
            Self::Error => "The thread hit an error",
            Self::TurnFinished => "Finished and waiting for you",
            Self::Plugin => "",
        }
    }

    pub fn glyph(self) -> char {
        match self {
            Self::NeedsInput => '?',
            Self::Error => '✗',
            Self::TurnFinished => '✓',
            Self::Plugin => '•',
        }
    }

    pub fn toast_level(self) -> ToastLevel {
        match self {
            Self::NeedsInput => ToastLevel::Attention,
            Self::Error => ToastLevel::Error,
            Self::TurnFinished | Self::Plugin => ToastLevel::Info,
        }
    }

    fn enabled(self, prefs: &NotificationPrefs) -> bool {
        match self {
            Self::NeedsInput => prefs.needs_input,
            Self::Error => prefs.errors,
            Self::TurnFinished => prefs.turn_finished,
            Self::Plugin => prefs.plugin_signals,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub id: u64,
    pub kind: NoticeKind,
    pub thread_id: Option<String>,
    pub title: String,
    pub body: String,
    pub at_ms: i64,
    pub unread: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopNotice {
    pub method: DesktopMethod,
    pub title: String,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Alert {
    pub bell: bool,
    pub desktop: Option<DesktopNotice>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingNotice {
    kinds: Vec<NoticeKind>,
    event_at_ms: i64,
    due: Instant,
}

#[derive(Debug, Default)]
pub struct Notifications {
    pub prefs: NotificationPrefs,
    entries: VecDeque<Notice>,
    pending: HashMap<String, PendingNotice>,
    awaiting: HashMap<String, i64>,
    seen_signals: VecDeque<String>,
    handled: HashMap<String, Instant>,
    next_id: u64,
    pub terminal_focused: Option<bool>,
    pub title: Option<String>,
}

impl Notifications {
    pub fn new(prefs: NotificationPrefs) -> Self {
        Self {
            prefs,
            ..Self::default()
        }
    }

    pub fn newest_first(&self) -> impl Iterator<Item = &Notice> {
        self.entries.iter().rev()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn unread(&self) -> usize {
        self.entries.iter().filter(|notice| notice.unread).count()
    }

    pub fn pending_threads(&self) -> usize {
        self.pending.len() + self.awaiting.len()
    }

    pub fn mark_all_read(&mut self) {
        for notice in &mut self.entries {
            notice.unread = false;
        }
    }

    pub fn mark_thread_read(&mut self, thread_id: &str) {
        for notice in &mut self.entries {
            if notice.thread_id.as_deref() == Some(thread_id) {
                notice.unread = false;
            }
        }
    }

    pub fn remove_newest(&mut self, index: usize) -> Option<Notice> {
        let position = self.entries.len().checked_sub(index + 1)?;
        self.entries.remove(position)
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    fn schedule(&mut self, thread_id: String, kind: NoticeKind, now: Instant, now_ms: i64) {
        let entry = self.pending.entry(thread_id).or_insert(PendingNotice {
            kinds: Vec::new(),
            event_at_ms: now_ms,
            due: now + COALESCE,
        });
        if !entry.kinds.contains(&kind) {
            entry.kinds.push(kind);
        }
        entry.event_at_ms = entry.event_at_ms.max(now_ms);
    }

    fn take_due(&mut self, now: Instant) -> Vec<(String, PendingNotice)> {
        let due: Vec<String> = self
            .pending
            .iter()
            .filter(|(_, pending)| pending.due <= now)
            .map(|(id, _)| id.clone())
            .collect();
        let mut taken: Vec<(String, PendingNotice)> = due
            .into_iter()
            .filter_map(|id| self.pending.remove_entry(&id))
            .collect();
        for (thread_id, _) in &taken {
            self.handled.insert(thread_id.clone(), now);
        }
        taken.sort_by_key(|(_, pending)| pending.due);
        taken
    }

    fn push(&mut self, notice: Notice, now: Instant) {
        if let Some(thread_id) = &notice.thread_id {
            self.handled.insert(thread_id.clone(), now);
        }
        self.entries.push_back(notice);
        while self.entries.len() > MAX_ENTRIES {
            self.entries.pop_front();
        }
        self.handled
            .retain(|_, at| now.saturating_duration_since(*at) < SIGNAL_DEDUPE);
    }

    fn claim_signal(&mut self, id: &str) -> bool {
        if self.seen_signals.iter().any(|seen| seen == id) {
            return false;
        }
        self.seen_signals.push_back(id.to_owned());
        while self.seen_signals.len() > SEEN_SIGNALS {
            self.seen_signals.pop_front();
        }
        true
    }

    fn recently_handled(&self, thread_id: &str, now: Instant) -> bool {
        self.handled
            .get(thread_id)
            .is_some_and(|at| now.saturating_duration_since(*at) < SIGNAL_DEDUPE)
    }

    fn alert(&self, title: &str, body: &str) -> Option<Alert> {
        let desktop_allowed = self.prefs.desktop != DesktopMethod::Off
            && self.prefs.desktop != DesktopMethod::Auto
            && (self.prefs.desktop_when == DesktopWhen::Always
                || self.terminal_focused != Some(true));
        let desktop = desktop_allowed.then(|| DesktopNotice {
            method: self.prefs.desktop,
            title: title.to_owned(),
            body: body.to_owned(),
        });
        (self.prefs.bell || desktop.is_some()).then_some(Alert {
            bell: self.prefs.bell,
            desktop,
        })
    }
}

fn is_turn_running(status: ThreadStatus) -> bool {
    matches!(
        status,
        ThreadStatus::Active | ThreadStatus::Starting | ThreadStatus::Stopping
    )
}

fn is_notifiable(entry: &ThreadListEntry) -> bool {
    let thread = &entry.thread;
    thread.deleted_at.is_none()
        && thread.archived_at.is_none()
        && thread.visibility != ThreadVisibility::Hidden
}

pub fn detect(previous: &StoreSnapshot, next: &StoreSnapshot) -> Vec<(String, NoticeKind)> {
    if previous.sidebar.data.is_none() {
        return Vec::new();
    }
    let before: HashMap<&str, &ThreadListEntry> = previous
        .sidebar_threads()
        .map(|entry| (entry.thread.id.as_str(), entry))
        .collect();
    let mut changes = Vec::new();
    for entry in next.sidebar_threads() {
        let Some(old) = before.get(entry.thread.id.as_str()) else {
            continue;
        };
        if !is_notifiable(entry) {
            continue;
        }
        let id = &entry.thread.id;
        if entry.has_pending_interaction && !old.has_pending_interaction {
            changes.push((id.clone(), NoticeKind::NeedsInput));
        }
        if entry.thread.status == ThreadStatus::Error && old.thread.status != ThreadStatus::Error {
            changes.push((id.clone(), NoticeKind::Error));
        }
        if entry.thread.status == ThreadStatus::Idle
            && is_turn_running(old.thread.status)
            && entry.thread.parent_thread_id.is_none()
        {
            changes.push((id.clone(), NoticeKind::TurnFinished));
        }
    }
    changes
}

fn first_line(text: &str) -> &str {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("")
}

pub fn describe_interaction(interaction: &PendingInteraction) -> String {
    match &interaction.payload {
        InteractionPayload::UserQuestion(payload) => {
            let prompt = payload
                .questions
                .first()
                .map(|question| first_line(&question.prompt))
                .unwrap_or("");
            if prompt.is_empty() {
                "The agent has a question for you".into()
            } else {
                prompt.into()
            }
        }
        InteractionPayload::Approval(payload) => match &payload.subject {
            ApprovalSubject::Command { command, .. } => {
                format!("Approve command: {}", first_line(command))
            }
            ApprovalSubject::FileChange { .. } => "Approve file changes".into(),
            ApprovalSubject::PermissionGrant { tool_name, .. } => match tool_name {
                Some(tool) => format!("Grant permissions to {tool}"),
                None => "Grant additional permissions".into(),
            },
            _ => "Review the plan before the agent continues".into(),
        },
        InteractionPayload::Plugin(payload) => payload.title.clone(),
        InteractionPayload::Extension(payload) => payload.title.clone(),
        InteractionPayload::Unknown(_) => NoticeKind::NeedsInput.fallback_body().into(),
    }
}

pub fn truncate_chars(text: &str, max: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out = out.trim_end().to_owned();
    out.push('…');
    out
}

fn pick(kinds: &[NoticeKind], prefs: &NotificationPrefs) -> Option<NoticeKind> {
    PRIORITY
        .into_iter()
        .find(|kind| kinds.contains(kind) && kind.enabled(prefs))
}

impl AppState {
    fn is_watching(&self, thread_id: &str) -> bool {
        self.open_thread.as_deref() == Some(thread_id)
            && self.notifications.terminal_focused != Some(false)
    }

    pub(crate) fn notifications_snapshot(&mut self, previous: &StoreSnapshot) {
        if !self.notifications.prefs.enabled {
            return;
        }
        let changes = detect(previous, &self.snapshot);
        for (thread_id, kind) in changes {
            self.notifications
                .schedule(thread_id, kind, self.clock, self.now_ms);
        }
    }

    pub(crate) fn notifications_tick(&mut self) -> Vec<Effect> {
        let mut effects = Vec::new();
        for (thread_id, pending) in self.notifications.take_due(self.clock) {
            effects.extend(self.flush_notice(thread_id, pending));
        }
        effects
    }

    fn flush_notice(&mut self, thread_id: String, pending: PendingNotice) -> Vec<Effect> {
        let Some(entry) = self.snapshot.sidebar_thread(&thread_id) else {
            return Vec::new();
        };
        if !is_notifiable(entry) {
            return Vec::new();
        }
        let last_read = entry.thread.last_read_at.unwrap_or(0);
        if last_read >= entry.thread.latest_attention_at && last_read >= pending.event_at_ms {
            return Vec::new();
        }
        let current: Vec<NoticeKind> = pending
            .kinds
            .iter()
            .copied()
            .filter(|kind| match kind {
                NoticeKind::NeedsInput => entry.has_pending_interaction,
                NoticeKind::Error => entry.thread.status == ThreadStatus::Error,
                NoticeKind::TurnFinished => entry.thread.status == ThreadStatus::Idle,
                NoticeKind::Plugin => false,
            })
            .collect();
        let Some(kind) = pick(&current, &self.notifications.prefs) else {
            return Vec::new();
        };
        if self.is_watching(&thread_id) {
            return Vec::new();
        }
        if kind == NoticeKind::NeedsInput {
            let body = self
                .snapshot
                .pending_interactions(&thread_id)
                .iter()
                .find(|interaction| interaction.status == PendingInteractionStatus::Pending)
                .map(describe_interaction);
            if body.is_none() {
                self.notifications
                    .awaiting
                    .insert(thread_id.clone(), pending.event_at_ms);
                return vec![Effect::FetchNotificationInteractions(thread_id)];
            }
            return self.deliver_notice(kind, Some(thread_id), body);
        }
        self.deliver_notice(kind, Some(thread_id), None)
    }

    pub(crate) fn notification_interactions_loaded(
        &mut self,
        thread_id: String,
        result: Result<Vec<PendingInteraction>, String>,
    ) -> Vec<Effect> {
        if self.notifications.awaiting.remove(&thread_id).is_none() {
            return Vec::new();
        }
        if self.is_watching(&thread_id) {
            return Vec::new();
        }
        let interactions = match result {
            Ok(interactions) => interactions,
            Err(error) => {
                tracing::debug!(%error, thread_id, "notification interactions unavailable");
                Vec::new()
            }
        };
        let pending = interactions
            .iter()
            .find(|interaction| interaction.status == PendingInteractionStatus::Pending);
        let still_waiting = self
            .snapshot
            .sidebar_thread(&thread_id)
            .is_some_and(|entry| entry.has_pending_interaction);
        if pending.is_none() && !still_waiting {
            return Vec::new();
        }
        let body = pending.map(describe_interaction);
        self.deliver_notice(NoticeKind::NeedsInput, Some(thread_id), body)
    }

    fn deliver_notice(
        &mut self,
        kind: NoticeKind,
        thread_id: Option<String>,
        body: Option<String>,
    ) -> Vec<Effect> {
        let title = match &thread_id {
            Some(id) => self.thread_title(id),
            None => APP_TITLE.to_owned(),
        };
        let body = body
            .filter(|body| !body.trim().is_empty())
            .unwrap_or_else(|| kind.fallback_body().to_owned());
        self.publish(kind, thread_id, title, body)
    }

    fn publish(
        &mut self,
        kind: NoticeKind,
        thread_id: Option<String>,
        title: String,
        body: String,
    ) -> Vec<Effect> {
        let title = truncate_chars(&title, TITLE_MAX_CHARS);
        let body = truncate_chars(&body, BODY_MAX_CHARS);
        let id = self.notifications.next_id;
        self.notifications.next_id += 1;
        self.notifications.push(
            Notice {
                id,
                kind,
                thread_id,
                title: title.clone(),
                body: body.clone(),
                at_ms: self.now_ms,
                unread: true,
            },
            self.clock,
        );
        if self.notifications.prefs.toast {
            let message = if body.is_empty() {
                format!("{} {title}", kind.glyph())
            } else {
                format!("{} {title}: {body}", kind.glyph())
            };
            self.toast(message, kind.toast_level());
        }
        self.notifications
            .alert(&title, &body)
            .map(Effect::Alert)
            .into_iter()
            .collect()
    }

    pub(crate) fn plugin_signal(
        &mut self,
        plugin_id: &str,
        channel: &str,
        payload: JsonValue,
    ) -> Vec<Effect> {
        if plugin_id != PUSH_NOTIFICATIONS_PLUGIN_ID
            || channel != PUSH_NOTIFICATION_CHANNEL
            || !self.notifications.prefs.enabled
            || !self.notifications.prefs.plugin_signals
        {
            return Vec::new();
        }
        let notification: ClientNotification = match serde_json::from_value(payload) {
            Ok(notification) => notification,
            Err(error) => {
                tracing::debug!(%error, "ignoring malformed notification signal");
                return Vec::new();
            }
        };
        if notification.id.is_empty()
            || notification.channels.is_empty()
            || !self.notifications.claim_signal(&notification.id)
        {
            return Vec::new();
        }
        if let Some(thread_id) = notification.thread_id.as_deref()
            && (self.is_watching(thread_id)
                || self.notifications.recently_handled(thread_id, self.clock)
                || self.notifications.pending.contains_key(thread_id)
                || self.notifications.awaiting.contains_key(thread_id))
        {
            return Vec::new();
        }
        let title = if notification.title.trim().is_empty() {
            APP_TITLE.to_owned()
        } else {
            notification.title
        };
        self.publish(
            NoticeKind::Plugin,
            notification.thread_id,
            title,
            notification.body,
        )
    }

    pub fn desired_title(&self) -> Option<String> {
        if !self.notifications.prefs.terminal_title {
            return None;
        }
        let has_attention = self
            .snapshot
            .attention
            .data
            .as_ref()
            .is_some_and(|attention| attention.has_attention);
        Some(if self.attention > 0 {
            format!("({}) {APP_TITLE}", self.attention)
        } else if has_attention {
            format!("● {APP_TITLE}")
        } else {
            APP_TITLE.to_owned()
        })
    }

    pub(crate) fn sync_title(&mut self) -> Vec<Effect> {
        let desired = self.desired_title();
        if desired.is_none() || desired == self.notifications.title {
            return Vec::new();
        }
        self.notifications.title = desired.clone();
        desired.map(Effect::SetTitle).into_iter().collect()
    }

    pub(crate) fn open_notifications(&mut self) {
        self.notifications.mark_all_read();
        self.mode = Mode::Notifications { selected: 0 };
    }

    fn jump_to_notice(&mut self, index: usize) -> Vec<Effect> {
        let Some(notice) = self.notifications.newest_first().nth(index).cloned() else {
            return Vec::new();
        };
        let Some(thread_id) = notice.thread_id else {
            self.mode = Mode::Notifications { selected: index };
            return Vec::new();
        };
        let known =
            self.sidebar_entry(&thread_id).is_some() || self.snapshot.thread(&thread_id).is_some();
        if !known {
            self.mode = Mode::Notifications { selected: index };
            self.toast("That thread is no longer listed", ToastLevel::Info);
            return Vec::new();
        }
        let waiting = self
            .sidebar_entry(&thread_id)
            .is_some_and(|entry| entry.has_pending_interaction);
        let effects = self.open(thread_id);
        self.focus = if waiting {
            Focus::Interaction
        } else {
            Focus::Main
        };
        effects
    }
}

pub(crate) fn handle_notifications_key(
    state: &mut AppState,
    selected: usize,
    key: KeyEvent,
) -> Vec<Effect> {
    let count = state.notifications.len();
    let last = count.saturating_sub(1);
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let selected = match key.code {
        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('!') => return Vec::new(),
        KeyCode::Char('c') if ctrl => return Vec::new(),
        KeyCode::Enter | KeyCode::Char('o') => return state.jump_to_notice(selected),
        KeyCode::Down | KeyCode::Char('j') => (selected + 1).min(last),
        KeyCode::Up | KeyCode::Char('k') => selected.saturating_sub(1),
        KeyCode::Home | KeyCode::Char('g') => 0,
        KeyCode::End | KeyCode::Char('G') => last,
        KeyCode::Char('d') | KeyCode::Char('x') | KeyCode::Delete => {
            state.notifications.remove_newest(selected);
            selected.min(state.notifications.len().saturating_sub(1))
        }
        KeyCode::Char('C') => {
            state.notifications.clear();
            0
        }
        _ => selected,
    };
    state.mode = Mode::Notifications { selected };
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use bb_wire::interaction::{ApprovalPayload, UserQuestion, UserQuestionPayload};

    fn interaction(payload: InteractionPayload) -> PendingInteraction {
        PendingInteraction {
            payload,
            ..PendingInteraction::default()
        }
    }

    #[test]
    fn describes_interactions_like_the_push_sender() {
        let question = interaction(InteractionPayload::UserQuestion(UserQuestionPayload {
            questions: vec![UserQuestion {
                prompt: "\n  Which branch?\nmore".into(),
                ..UserQuestion::default()
            }],
        }));
        assert_eq!(describe_interaction(&question), "Which branch?");
        let empty = interaction(InteractionPayload::UserQuestion(
            UserQuestionPayload::default(),
        ));
        assert_eq!(
            describe_interaction(&empty),
            "The agent has a question for you"
        );
        let command = interaction(InteractionPayload::Approval(ApprovalPayload {
            subject: ApprovalSubject::Command {
                item_id: "i".into(),
                command: "rm -rf build\necho done".into(),
                cwd: None,
                actions: Vec::new(),
                session_grant: None,
            },
            ..ApprovalPayload::default()
        }));
        assert_eq!(
            describe_interaction(&command),
            "Approve command: rm -rf build"
        );
        let grant = interaction(InteractionPayload::Approval(ApprovalPayload {
            subject: ApprovalSubject::PermissionGrant {
                item_id: "i".into(),
                tool_name: None,
                permissions: Default::default(),
            },
            ..ApprovalPayload::default()
        }));
        assert_eq!(describe_interaction(&grant), "Grant additional permissions");
    }

    #[test]
    fn truncation_counts_characters() {
        assert_eq!(truncate_chars("  short  ", 10), "short");
        assert_eq!(truncate_chars("абвгдеёжзи", 5), "абвг…");
    }

    #[test]
    fn priority_skips_disabled_kinds() {
        let mut prefs = NotificationPrefs::default();
        let kinds = [NoticeKind::TurnFinished, NoticeKind::NeedsInput];
        assert_eq!(pick(&kinds, &prefs), Some(NoticeKind::NeedsInput));
        prefs.needs_input = false;
        assert_eq!(pick(&kinds, &prefs), Some(NoticeKind::TurnFinished));
        prefs.turn_finished = false;
        assert_eq!(pick(&kinds, &prefs), None);
    }

    #[test]
    fn desktop_alert_respects_focus_and_method() {
        let mut center = Notifications::new(NotificationPrefs {
            desktop: DesktopMethod::Osc9,
            ..NotificationPrefs::default()
        });
        assert!(center.alert("t", "b").unwrap().desktop.is_some());
        center.terminal_focused = Some(true);
        assert_eq!(
            center.alert("t", "b"),
            Some(Alert {
                bell: true,
                desktop: None
            })
        );
        center.prefs.desktop_when = DesktopWhen::Always;
        assert!(center.alert("t", "b").unwrap().desktop.is_some());
        center.prefs.bell = false;
        center.prefs.desktop = DesktopMethod::Off;
        assert_eq!(center.alert("t", "b"), None);
    }
}
