use bb_wire::thread::{
    Thread, ThreadDisplayStatus, ThreadListEntry, ThreadQueuedWork, ThreadStatus,
};
use ratatui::style::{Color, Modifier, Style};

const SPINNER: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Indicator {
    Archived,
    UnreadError,
    WaitingForInput,
    PlanMode,
    Goal,
    Runtime,
    Workflow,
    BackgroundAgent,
    BackgroundCommand,
    QueuedFailed,
    UnreadSuccess,
    QueuedWaiting,
    None,
}

pub fn is_thread_read(thread: &Thread) -> bool {
    thread.last_read_at.unwrap_or(0) >= thread.latest_attention_at
}

pub fn is_unread_done(thread: &Thread) -> bool {
    if thread.parent_thread_id.is_some() {
        return false;
    }
    matches!(thread.status, ThreadStatus::Idle | ThreadStatus::Error) && !is_thread_read(thread)
}

pub fn is_runtime_busy(status: ThreadDisplayStatus) -> bool {
    matches!(
        status,
        ThreadDisplayStatus::Active
            | ThreadDisplayStatus::Provisioning
            | ThreadDisplayStatus::Starting
            | ThreadDisplayStatus::Stopping
    )
}

impl Indicator {
    pub fn for_entry(entry: &ThreadListEntry) -> Self {
        let thread = &entry.thread;
        if thread.is_archived() {
            return Self::Archived;
        }
        let unread_done = is_unread_done(thread);
        let activity = &entry.activity;
        if unread_done && thread.status == ThreadStatus::Error {
            return Self::UnreadError;
        }
        if entry.has_pending_interaction {
            return Self::WaitingForInput;
        }
        if activity.active_plan_mode_count > 0 {
            return Self::PlanMode;
        }
        if activity.active_goal_count > 0 {
            return Self::Goal;
        }
        if is_runtime_busy(entry.runtime.display_status) {
            return Self::Runtime;
        }
        if activity.active_workflow_count > 0 {
            return Self::Workflow;
        }
        if activity.active_background_agent_count > 0 {
            return Self::BackgroundAgent;
        }
        if activity.active_background_command_count > 0 {
            return Self::BackgroundCommand;
        }
        if entry.queued_work == ThreadQueuedWork::Failed {
            return Self::QueuedFailed;
        }
        if unread_done {
            return Self::UnreadSuccess;
        }
        if entry.queued_work == ThreadQueuedWork::Waiting {
            return Self::QueuedWaiting;
        }
        Self::None
    }

    pub fn label(self) -> Option<&'static str> {
        Some(match self {
            Self::Archived => "Archived thread",
            Self::UnreadError => "Unread thread failed",
            Self::WaitingForInput => "Thread needs user input",
            Self::PlanMode => "Plan mode active",
            Self::Goal => "Goal active",
            Self::Runtime => "Thread working",
            Self::Workflow => "Workflow running",
            Self::BackgroundAgent => "Background agent running",
            Self::BackgroundCommand => "Background command running",
            Self::QueuedFailed => "Queued message failed to send",
            Self::UnreadSuccess => "Unread thread succeeded",
            Self::QueuedWaiting => "Thread has a message waiting to send",
            Self::None => return None,
        })
    }

    pub fn needs_attention(self) -> bool {
        matches!(
            self,
            Self::UnreadError | Self::WaitingForInput | Self::UnreadSuccess | Self::QueuedFailed
        )
    }

    pub fn is_working(self) -> bool {
        matches!(
            self,
            Self::PlanMode
                | Self::Goal
                | Self::Runtime
                | Self::Workflow
                | Self::BackgroundAgent
                | Self::BackgroundCommand
        )
    }

    pub fn glyph(self, frame: u64) -> char {
        match self {
            Self::Archived => '▤',
            Self::UnreadError | Self::QueuedFailed => '✗',
            Self::WaitingForInput => '?',
            Self::PlanMode => '≣',
            Self::Goal => '◎',
            Self::Runtime => SPINNER[(frame as usize) % SPINNER.len()],
            Self::Workflow => '≋',
            Self::BackgroundAgent => '⊕',
            Self::BackgroundCommand => '$',
            Self::UnreadSuccess => '●',
            Self::QueuedWaiting => '◷',
            Self::None => ' ',
        }
    }

    pub fn style(self) -> Style {
        match self {
            Self::UnreadError | Self::QueuedFailed => Style::new().fg(Color::Red),
            Self::WaitingForInput => Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            Self::UnreadSuccess => Style::new().fg(Color::Green),
            Self::Archived | Self::QueuedWaiting | Self::None => Style::new().fg(Color::DarkGray),
            Self::PlanMode
            | Self::Goal
            | Self::Runtime
            | Self::Workflow
            | Self::BackgroundAgent
            | Self::BackgroundCommand => Style::new().fg(Color::Cyan),
        }
    }
}

pub fn display_status_label(status: ThreadDisplayStatus) -> &'static str {
    match status {
        ThreadDisplayStatus::Pending => "pending",
        ThreadDisplayStatus::Idle => "idle",
        ThreadDisplayStatus::Starting => "starting",
        ThreadDisplayStatus::Active => "working",
        ThreadDisplayStatus::Stopping => "stopping",
        ThreadDisplayStatus::Error => "error",
        ThreadDisplayStatus::Provisioning => "provisioning",
        ThreadDisplayStatus::WaitingForHost => "waiting for host",
        ThreadDisplayStatus::Unknown => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bb_wire::thread::ThreadActivityState;

    fn entry(status: ThreadStatus, last_read: Option<i64>, attention: i64) -> ThreadListEntry {
        let mut entry = ThreadListEntry::default();
        entry.thread.id = "thr_a".into();
        entry.thread.status = status;
        entry.thread.last_read_at = last_read;
        entry.thread.latest_attention_at = attention;
        entry
    }

    #[test]
    fn unread_error_outranks_pending_interaction_and_work() {
        let mut e = entry(ThreadStatus::Error, None, 10);
        e.has_pending_interaction = true;
        e.runtime.display_status = ThreadDisplayStatus::Active;
        assert_eq!(Indicator::for_entry(&e), Indicator::UnreadError);
        e.thread.last_read_at = Some(10);
        assert_eq!(Indicator::for_entry(&e), Indicator::WaitingForInput);
    }

    #[test]
    fn activity_precedence_matches_web_resolver() {
        let mut e = entry(ThreadStatus::Active, Some(0), 0);
        e.runtime.display_status = ThreadDisplayStatus::Active;
        e.activity = ThreadActivityState {
            active_workflow_count: 1,
            active_goal_count: 1,
            ..ThreadActivityState::default()
        };
        assert_eq!(Indicator::for_entry(&e), Indicator::Goal);
        e.activity.active_goal_count = 0;
        assert_eq!(Indicator::for_entry(&e), Indicator::Runtime);
        e.runtime.display_status = ThreadDisplayStatus::WaitingForHost;
        assert_eq!(Indicator::for_entry(&e), Indicator::Workflow);
    }

    #[test]
    fn queued_failed_beats_unread_success_which_beats_queued_waiting() {
        let mut e = entry(ThreadStatus::Idle, Some(1), 5);
        e.queued_work = ThreadQueuedWork::Waiting;
        assert_eq!(Indicator::for_entry(&e), Indicator::UnreadSuccess);
        e.queued_work = ThreadQueuedWork::Failed;
        assert_eq!(Indicator::for_entry(&e), Indicator::QueuedFailed);
        e.queued_work = ThreadQueuedWork::Waiting;
        e.thread.last_read_at = Some(5);
        assert_eq!(Indicator::for_entry(&e), Indicator::QueuedWaiting);
    }

    #[test]
    fn child_and_pending_threads_are_never_unread() {
        let mut e = entry(ThreadStatus::Idle, None, 5);
        e.thread.parent_thread_id = Some("thr_parent".into());
        assert_eq!(Indicator::for_entry(&e), Indicator::None);
        let e = entry(ThreadStatus::Pending, None, 5);
        assert_eq!(Indicator::for_entry(&e), Indicator::None);
    }

    #[test]
    fn archived_wins() {
        let mut e = entry(ThreadStatus::Error, None, 5);
        e.thread.archived_at = Some(1);
        assert_eq!(Indicator::for_entry(&e), Indicator::Archived);
    }
}
