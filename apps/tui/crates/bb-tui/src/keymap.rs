use std::fmt;

use bb_wire::system::{AppKeybindingOverride, AppShortcut, KeyboardPlatform, is_app_command_id};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::vi::{CharAct, EditOp, InsertAt};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Direction {
    Left,
    Down,
    Up,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScreenPos {
    Top,
    Middle,
    Bottom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Command {
    Quit,
    Help,
    KeymapOverview,
    Back,
    FocusNext,
    FocusPrevious,
    MoveDown,
    MoveUp,
    MoveTop,
    MoveBottom,
    Open,
    OpenFocus,
    OpenInsert,
    Filter,
    Collapse,
    Expand,
    ToggleGroup,
    FoldAll(bool),
    Rename,
    Archive,
    UndoArchive,
    ToggleRead,
    TogglePin,
    YankTitle,
    YankId,
    ThreadNext,
    ThreadPrevious,
    AlternateThread,
    StepWaiting(bool),
    StepUnread(bool),
    Jump(u8),
    SidebarToggle,
    RevealInSidebar,
    PanelToggle,
    StatusPanel,
    Reload,
    PageDown,
    PageUp,
    HalfPageDown,
    HalfPageUp,
    ScrollLine(i8),
    ScrollCursor(ScreenPos),
    ViewJump(ScreenPos),
    ToggleFollow,
    NextMessage,
    PreviousMessage,
    Copy,
    YankTimeline,
    OpenExternal,
    FocusComposer,
    ComposerInsert(InsertAt),
    ComposerToggle,
    ComposerMention,
    Send,
    Steer,
    StopTurn,
    ModelPicker,
    ReasoningPicker,
    PermissionPicker,
    CycleModel,
    CycleModelBackward,
    CycleReasoning,
    CycleReasoningBackward,
    QueueList,
    ToggleSubmitMode,
    NextPending,
    NewThread,
    NewThreadInEnvironment,
    PaletteOpen,
    PaletteThreadGroup,
    ResumeMinibuffer,
    QuickOpen,
    SearchThreads,
    SwitchProject,
    AttachFile,
    Attachments,
    FileYankPath,
    FileEdit,
    DiffToggle,
    DiffHunk(bool),
    PanelNextTab,
    PanelPreviousTab,
    PanelFullScreen,
    GitCommit,
    PullRequestMerge,
    PullRequestReady,
    PullRequestDraft,
    Notifications,
    ClearNotifications,
    TerminalOpen,
    TerminalNew,
    TerminalAttach,
    Settings,
    SettingsSection(crate::settings::SettingsSection),
    Automations,
    NewAutomation,
    SplitOpen,
    SplitNew,
    PaneLeft,
    PaneRight,
    PaneFocus(u8),
    PaneClose,
    PaneMaximize,
    WindowFocus(Direction),
    WindowLast,
    WindowTop,
    WindowBottom,
    WindowOnly,
    WindowRestore,
    WindowSwap,
    WindowRotate,
    WindowMoveFar(bool),
    WindowResize(i8),
    Zen,
    ExLine,
    CloseWindowOrQuit,
    SendAndClose,
    QuitHint,
    DescribeKey,
    Messages,
    Edit(EditOp),
    View(crate::rbuf::ViewOp),
    TimelineJump(JumpKind, bool),
    PreviewAtCursor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum JumpKind {
    Turn,
    Error,
}

impl Command {
    pub const ALL: &'static [Command] = &[
        Command::PaletteOpen,
        Command::ExLine,
        Command::QuickOpen,
        Command::SearchThreads,
        Command::SwitchProject,
        Command::ResumeMinibuffer,
        Command::MoveDown,
        Command::MoveUp,
        Command::MoveTop,
        Command::MoveBottom,
        Command::PageDown,
        Command::PageUp,
        Command::HalfPageDown,
        Command::HalfPageUp,
        Command::Open,
        Command::NextMessage,
        Command::PreviousMessage,
        Command::Copy,
        Command::YankTimeline,
        Command::OpenExternal,
        Command::ToggleFollow,
        Command::FocusComposer,
        Command::ComposerToggle,
        Command::ComposerMention,
        Command::Send,
        Command::Steer,
        Command::AttachFile,
        Command::Attachments,
        Command::StopTurn,
        Command::QueueList,
        Command::ToggleSubmitMode,
        Command::NextPending,
        Command::Notifications,
        Command::ClearNotifications,
        Command::NewThread,
        Command::NewThreadInEnvironment,
        Command::ModelPicker,
        Command::ReasoningPicker,
        Command::PermissionPicker,
        Command::CycleModel,
        Command::CycleModelBackward,
        Command::CycleReasoning,
        Command::CycleReasoningBackward,
        Command::ThreadNext,
        Command::ThreadPrevious,
        Command::AlternateThread,
        Command::Jump(1),
        Command::Jump(2),
        Command::Jump(3),
        Command::Jump(4),
        Command::Jump(5),
        Command::Jump(6),
        Command::Jump(7),
        Command::Jump(8),
        Command::Jump(9),
        Command::ToggleGroup,
        Command::Collapse,
        Command::Expand,
        Command::Filter,
        Command::Rename,
        Command::Archive,
        Command::UndoArchive,
        Command::ToggleRead,
        Command::TogglePin,
        Command::FocusNext,
        Command::FocusPrevious,
        Command::WindowFocus(Direction::Left),
        Command::WindowFocus(Direction::Down),
        Command::WindowFocus(Direction::Up),
        Command::WindowFocus(Direction::Right),
        Command::WindowLast,
        Command::WindowOnly,
        Command::WindowRestore,
        Command::WindowSwap,
        Command::WindowRotate,
        Command::SidebarToggle,
        Command::RevealInSidebar,
        Command::PanelToggle,
        Command::StatusPanel,
        Command::DiffToggle,
        Command::PanelNextTab,
        Command::PanelPreviousTab,
        Command::PanelFullScreen,
        Command::GitCommit,
        Command::PullRequestMerge,
        Command::PullRequestReady,
        Command::PullRequestDraft,
        Command::TerminalOpen,
        Command::TerminalNew,
        Command::TerminalAttach,
        Command::Settings,
        Command::Automations,
        Command::NewAutomation,
        Command::SplitOpen,
        Command::SplitNew,
        Command::PaneLeft,
        Command::PaneRight,
        Command::PaneFocus(1),
        Command::PaneFocus(2),
        Command::PaneFocus(3),
        Command::PaneFocus(4),
        Command::PaneClose,
        Command::PaneMaximize,
        Command::Zen,
        Command::Reload,
        Command::Back,
        Command::Help,
        Command::KeymapOverview,
        Command::DescribeKey,
        Command::Messages,
        Command::Quit,
    ];

    pub fn app_command_id(self) -> Option<&'static str> {
        Some(match self {
            Self::Back => "app.back",
            Self::NewThread => "thread.new",
            Self::FocusNext => "pane.focus.next",
            Self::FocusPrevious => "pane.focus.previous",
            Self::PaneLeft => "pane.focus.left",
            Self::PaneRight => "pane.focus.right",
            Self::PaneFocus(1) => "pane.focus.1",
            Self::PaneFocus(2) => "pane.focus.2",
            Self::PaneFocus(3) => "pane.focus.3",
            Self::PaneFocus(4) => "pane.focus.4",
            Self::PaneClose => "pane.close",
            Self::PaneMaximize => "pane.maximize.toggle",
            Self::PaletteOpen => "palette.open",
            Self::QuickOpen => "file.quickOpen",
            Self::SearchThreads => "thread.search",
            Self::Rename => "thread.rename",
            Self::Archive => "thread.archive",
            Self::ThreadNext => "thread.next",
            Self::ThreadPrevious => "thread.previous",
            Self::Jump(1) => "thread.jump.1",
            Self::Jump(2) => "thread.jump.2",
            Self::Jump(3) => "thread.jump.3",
            Self::Jump(4) => "thread.jump.4",
            Self::Jump(5) => "thread.jump.5",
            Self::Jump(6) => "thread.jump.6",
            Self::Jump(7) => "thread.jump.7",
            Self::Jump(8) => "thread.jump.8",
            Self::Jump(9) => "thread.jump.9",
            Self::SidebarToggle => "sidebar.toggle",
            Self::PanelToggle => "panel.toggle",
            Self::DiffToggle => "diff.toggle",
            Self::TerminalOpen => "terminal.open",
            Self::PanelNextTab => "panel.nextTab",
            Self::PanelPreviousTab => "panel.previousTab",
            Self::PanelFullScreen => "panel.fullScreen.toggle",
            Self::Reload => "window.reload",
            Self::Settings => "settings.open",
            Self::FocusComposer => "composer.focus",
            Self::ModelPicker => "modelPicker.toggle",
            Self::CycleModel => "modelPicker.cycleModel",
            Self::CycleModelBackward => "modelPicker.cycleModelBackward",
            Self::CycleReasoning => "modelPicker.cycleReasoning",
            Self::CycleReasoningBackward => "modelPicker.cycleReasoningBackward",
            _ => return None,
        })
    }

    pub fn from_app_command_id(id: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|command| command.app_command_id() == Some(id))
    }

    pub fn repeats(self) -> bool {
        matches!(
            self,
            Self::MoveDown
                | Self::MoveUp
                | Self::PageDown
                | Self::PageUp
                | Self::HalfPageDown
                | Self::HalfPageUp
                | Self::ScrollLine(_)
                | Self::NextMessage
                | Self::PreviousMessage
                | Self::ThreadNext
                | Self::ThreadPrevious
                | Self::StepWaiting(_)
                | Self::StepUnread(_)
                | Self::FocusNext
                | Self::FocusPrevious
                | Self::WindowFocus(_)
                | Self::PaneLeft
                | Self::PaneRight
                | Self::PanelPreviousTab
                | Self::CycleModel
                | Self::CycleModelBackward
                | Self::CycleReasoning
                | Self::CycleReasoningBackward
                | Self::DiffHunk(_)
                | Self::TimelineJump(..)
        )
    }

    pub fn description(self) -> String {
        match self {
            Self::Quit => "Quit bb-tui".into(),
            Self::Help => "Bindings of the focused buffer".into(),
            Self::KeymapOverview => "Keymap overview".into(),
            Self::Back => "Keyboard quit / back".into(),
            Self::FocusNext => "Next window".into(),
            Self::FocusPrevious => "Previous window".into(),
            Self::MoveDown => "Move down".into(),
            Self::MoveUp => "Move up".into(),
            Self::MoveTop => "Go to the first line".into(),
            Self::MoveBottom => "Go to the last line".into(),
            Self::Open => "Open thread / toggle row".into(),
            Self::OpenFocus => "Open the thread and focus its timeline".into(),
            Self::OpenInsert => "Open the thread and focus its composer".into(),
            Self::Filter => "Narrow the list to a filter".into(),
            Self::Collapse => "Collapse group / row".into(),
            Self::Expand => "Expand group / row".into(),
            Self::ToggleGroup => "Toggle the fold".into(),
            Self::FoldAll(true) => "Open all folds".into(),
            Self::FoldAll(false) => "Close all folds".into(),
            Self::Rename => "Rename thread".into(),
            Self::Archive => "Archive / unarchive thread".into(),
            Self::UndoArchive => "Undo last archive".into(),
            Self::ToggleRead => "Mark read / unread".into(),
            Self::TogglePin => "Pin / unpin thread".into(),
            Self::YankTitle => "Yank the thread title".into(),
            Self::YankId => "Yank the thread id".into(),
            Self::ThreadNext => "Open next thread".into(),
            Self::ThreadPrevious => "Open previous thread".into(),
            Self::AlternateThread => "Alternate thread".into(),
            Self::StepWaiting(true) => "Next thread waiting for an answer".into(),
            Self::StepWaiting(false) => "Previous thread waiting for an answer".into(),
            Self::StepUnread(true) => "Next unread thread".into(),
            Self::StepUnread(false) => "Previous unread thread".into(),
            Self::Jump(n) => format!("Open thread #{n}"),
            Self::SidebarToggle => "Toggle sidebar".into(),
            Self::RevealInSidebar => "Reveal the thread in the sidebar".into(),
            Self::PanelToggle => "Toggle right panel".into(),
            Self::StatusPanel => "Right panel on the Info tab".into(),
            Self::Reload => "Reload data".into(),
            Self::PageDown => "Page down".into(),
            Self::PageUp => "Page up".into(),
            Self::HalfPageDown => "Half page down".into(),
            Self::HalfPageUp => "Half page up".into(),
            Self::ScrollLine(n) if n > 0 => "Scroll one line down".into(),
            Self::ScrollLine(_) => "Scroll one line up".into(),
            Self::ScrollCursor(ScreenPos::Top) => "Cursor line to the top".into(),
            Self::ScrollCursor(ScreenPos::Middle) => "Cursor line to the middle".into(),
            Self::ScrollCursor(ScreenPos::Bottom) => "Cursor line to the bottom".into(),
            Self::ViewJump(ScreenPos::Top) => "Top of the window".into(),
            Self::ViewJump(ScreenPos::Middle) => "Middle of the window".into(),
            Self::ViewJump(ScreenPos::Bottom) => "Bottom of the window".into(),
            Self::ToggleFollow => "Follow new output".into(),
            Self::NextMessage => "Next user message".into(),
            Self::PreviousMessage => "Previous user message".into(),
            Self::Copy => "Yank the row source".into(),
            Self::YankTimeline => "Yank the loaded timeline".into(),
            Self::OpenExternal => "Open the link or file at the cursor".into(),
            Self::FocusComposer => "Focus the composer (insert)".into(),
            Self::ComposerInsert(InsertAt::LineStart) => "Insert at the start of the draft".into(),
            Self::ComposerInsert(InsertAt::Last) => "Insert where insert was last left".into(),
            Self::ComposerInsert(_) => "Insert at the end of the draft".into(),
            Self::ComposerToggle => "Toggle the composer".into(),
            Self::ComposerMention => "Mention a file".into(),
            Self::Send => "Send the draft".into(),
            Self::Steer => "Steer the running turn".into(),
            Self::StopTurn => "Stop the running turn".into(),
            Self::ModelPicker => "Pick model".into(),
            Self::ReasoningPicker => "Pick reasoning level".into(),
            Self::PermissionPicker => "Pick permission mode".into(),
            Self::CycleModel => "Next model".into(),
            Self::CycleModelBackward => "Previous model".into(),
            Self::CycleReasoning => "Next reasoning level".into(),
            Self::CycleReasoningBackward => "Previous reasoning level".into(),
            Self::QueueList => "Queued messages".into(),
            Self::ToggleSubmitMode => "Toggle sending while running: queue / steer".into(),
            Self::NextPending => "Answer the next waiting request".into(),
            Self::Notifications => "Notification center".into(),
            Self::ClearNotifications => "Clear all notifications".into(),
            Self::NewThread => "New thread".into(),
            Self::NewThreadInEnvironment => "New thread in this thread's environment".into(),
            Self::PaletteOpen => "Command palette".into(),
            Self::PaletteThreadGroup => "Thread commands".into(),
            Self::ResumeMinibuffer => "Reopen the last minibuffer".into(),
            Self::QuickOpen => "Quick open a file".into(),
            Self::SearchThreads => "Search threads".into(),
            Self::SwitchProject => "Switch project".into(),
            Self::AttachFile => "Attach a file".into(),
            Self::Attachments => "Message attachments".into(),
            Self::FileYankPath => "Yank the previewed file path".into(),
            Self::FileEdit => "Open the previewed file in $EDITOR".into(),
            Self::DiffToggle => "Show / hide the git diff panel".into(),
            Self::DiffHunk(true) => "Next hunk".into(),
            Self::DiffHunk(false) => "Previous hunk".into(),
            Self::PanelNextTab => "Next tab".into(),
            Self::PanelPreviousTab => "Previous tab".into(),
            Self::PanelFullScreen => "Maximize / restore the right panel".into(),
            Self::GitCommit => "Commit uncommitted changes".into(),
            Self::PullRequestMerge => "Merge the pull request".into(),
            Self::PullRequestReady => "Mark the pull request ready".into(),
            Self::PullRequestDraft => "Convert the pull request to draft".into(),
            Self::TerminalOpen => "Thread terminals".into(),
            Self::TerminalNew => "Start a new terminal".into(),
            Self::TerminalAttach => "Attach the terminal fullscreen".into(),
            Self::Settings => "Settings".into(),
            Self::SettingsSection(section) => format!("Settings: {}", section.label()),
            Self::Automations => "Automations".into(),
            Self::NewAutomation => "New automation".into(),
            Self::SplitOpen => "Open the thread in a new split".into(),
            Self::SplitNew => "New thread in a new split".into(),
            Self::PaneLeft => "Focus the split pane to the left".into(),
            Self::PaneRight => "Focus the split pane to the right".into(),
            Self::PaneFocus(n) => format!("Focus split pane {n}"),
            Self::PaneClose => "Close the focused window".into(),
            Self::PaneMaximize => "Maximize / restore the focused split pane".into(),
            Self::WindowFocus(Direction::Left) => "Window left".into(),
            Self::WindowFocus(Direction::Down) => "Window below".into(),
            Self::WindowFocus(Direction::Up) => "Window above".into(),
            Self::WindowFocus(Direction::Right) => "Window right".into(),
            Self::WindowLast => "Previously focused window".into(),
            Self::WindowTop => "Top-left window".into(),
            Self::WindowBottom => "Bottom-right window".into(),
            Self::WindowOnly => "Maximize / restore the window".into(),
            Self::WindowRestore => "Restore all windows".into(),
            Self::WindowSwap => "Swap the pane with the next one".into(),
            Self::WindowRotate => "Rotate panes".into(),
            Self::WindowMoveFar(true) => "Move the pane to the far left".into(),
            Self::WindowMoveFar(false) => "Move the pane to the far right".into(),
            Self::WindowResize(n) if n > 0 => "Widen the side window".into(),
            Self::WindowResize(_) => "Narrow the side window".into(),
            Self::Zen => "Zen: only the focused pane".into(),
            Self::ExLine => "Ex command line".into(),
            Self::CloseWindowOrQuit => "Close the window, quit on the last one".into(),
            Self::SendAndClose => "Send, then close the window".into(),
            Self::QuitHint => "Quit with SPC q q or :q".into(),
            Self::DescribeKey => "Describe key".into(),
            Self::Messages => "Message history".into(),
            Self::Edit(op) => op.description(),
            Self::View(op) => op.description(),
            Self::TimelineJump(JumpKind::Turn, true) => "Next turn".into(),
            Self::TimelineJump(JumpKind::Turn, false) => "Previous turn".into(),
            Self::TimelineJump(JumpKind::Error, true) => "Next error".into(),
            Self::TimelineJump(JumpKind::Error, false) => "Previous error".into(),
            Self::PreviewAtCursor => "Preview the file under the cursor".into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KeyChord {
    pub code: KeyCode,
    pub modifiers: KeyModifiers,
}

impl KeyChord {
    pub fn new(code: KeyCode, modifiers: KeyModifiers) -> Self {
        let mut modifiers = modifiers;
        let code = match code {
            KeyCode::Char(ch) if modifiers.contains(KeyModifiers::SHIFT) && ch.is_alphabetic() => {
                modifiers.remove(KeyModifiers::SHIFT);
                KeyCode::Char(ch.to_uppercase().next().unwrap_or(ch))
            }
            KeyCode::Char(_) | KeyCode::BackTab => {
                modifiers.remove(KeyModifiers::SHIFT);
                code
            }
            other => other,
        };
        modifiers &= KeyModifiers::SHIFT | KeyModifiers::CONTROL | KeyModifiers::ALT;
        Self { code, modifiers }
    }

    pub fn plain(code: KeyCode) -> Self {
        Self::new(code, KeyModifiers::NONE)
    }

    pub fn ch(ch: char) -> Self {
        Self::plain(KeyCode::Char(ch))
    }

    pub fn ctrl(ch: char) -> Self {
        Self::new(KeyCode::Char(ch), KeyModifiers::CONTROL)
    }

    pub fn alt(ch: char) -> Self {
        Self::new(KeyCode::Char(ch), KeyModifiers::ALT)
    }

    pub fn from_event(event: &KeyEvent) -> Self {
        Self::new(event.code, event.modifiers)
    }

    pub fn event(self) -> KeyEvent {
        KeyEvent::new(self.code, self.modifiers)
    }

    pub fn is_plain_char(self) -> Option<char> {
        match self.code {
            KeyCode::Char(ch)
                if !self
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                Some(ch)
            }
            _ => None,
        }
    }

    pub fn has_ctrl_or_alt(self) -> bool {
        self.modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
    }

    pub fn parse(token: &str) -> Option<Self> {
        let named = |name: &str| -> Option<KeyCode> {
            Some(match name {
                "SPC" => KeyCode::Char(' '),
                "RET" => KeyCode::Enter,
                "ESC" => KeyCode::Esc,
                "TAB" => KeyCode::Tab,
                "Bksp" => KeyCode::Backspace,
                "Del" => KeyCode::Delete,
                "←" => KeyCode::Left,
                "→" => KeyCode::Right,
                "↑" => KeyCode::Up,
                "↓" => KeyCode::Down,
                "Home" => KeyCode::Home,
                "End" => KeyCode::End,
                "PgUp" => KeyCode::PageUp,
                "PgDn" => KeyCode::PageDown,
                _ => {
                    if let Some(number) = name.strip_prefix('F').and_then(|n| n.parse().ok()) {
                        KeyCode::F(number)
                    } else {
                        let mut chars = name.chars();
                        let ch = chars.next()?;
                        if chars.next().is_some() {
                            return None;
                        }
                        KeyCode::Char(ch)
                    }
                }
            })
        };
        if token == "S-TAB" {
            return Some(Self::plain(KeyCode::BackTab));
        }
        let mut modifiers = KeyModifiers::NONE;
        let mut rest = token;
        loop {
            if rest.chars().count() <= 1 {
                break;
            }
            if let Some(tail) = rest.strip_prefix("C-") {
                modifiers |= KeyModifiers::CONTROL;
                rest = tail;
            } else if let Some(tail) = rest.strip_prefix("M-") {
                modifiers |= KeyModifiers::ALT;
                rest = tail;
            } else if let Some(tail) = rest.strip_prefix("S-") {
                modifiers |= KeyModifiers::SHIFT;
                rest = tail;
            } else {
                break;
            }
        }
        Some(Self::new(named(rest)?, modifiers))
    }

    pub fn from_shortcut(shortcut: &AppShortcut) -> Option<Self> {
        if shortcut.meta {
            return None;
        }
        let mut modifiers = KeyModifiers::NONE;
        if shortcut.primary_modifier || shortcut.control {
            modifiers |= KeyModifiers::CONTROL;
        }
        if shortcut.alt {
            modifiers |= KeyModifiers::ALT;
        }
        if shortcut.shift {
            modifiers |= KeyModifiers::SHIFT;
        }
        let key = shortcut.key.as_str();
        let code = match key {
            "ArrowLeft" => KeyCode::Left,
            "ArrowRight" => KeyCode::Right,
            "ArrowUp" => KeyCode::Up,
            "ArrowDown" => KeyCode::Down,
            "Enter" => KeyCode::Enter,
            "Escape" => KeyCode::Esc,
            "Tab" if shortcut.shift => KeyCode::BackTab,
            "Tab" => KeyCode::Tab,
            "Backspace" => KeyCode::Backspace,
            "Delete" => KeyCode::Delete,
            "Home" => KeyCode::Home,
            "End" => KeyCode::End,
            "PageUp" => KeyCode::PageUp,
            "PageDown" => KeyCode::PageDown,
            "Space" | " " => KeyCode::Char(' '),
            _ => {
                if let Some(number) = key.strip_prefix('F').and_then(|n| n.parse::<u8>().ok()) {
                    KeyCode::F(number)
                } else {
                    let mut chars = key.chars();
                    let ch = chars.next()?;
                    if chars.next().is_some() {
                        return None;
                    }
                    KeyCode::Char(ch.to_lowercase().next().unwrap_or(ch))
                }
            }
        };
        Some(Self::new(code, modifiers))
    }
}

impl fmt::Display for KeyChord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.modifiers.contains(KeyModifiers::CONTROL) {
            f.write_str("C-")?;
        }
        if self.modifiers.contains(KeyModifiers::ALT) {
            f.write_str("M-")?;
        }
        if self.modifiers.contains(KeyModifiers::SHIFT) {
            f.write_str("S-")?;
        }
        match self.code {
            KeyCode::Char(' ') => f.write_str("SPC"),
            KeyCode::Char(ch) => write!(f, "{ch}"),
            KeyCode::Enter => f.write_str("RET"),
            KeyCode::Esc => f.write_str("ESC"),
            KeyCode::Tab => f.write_str("TAB"),
            KeyCode::BackTab => f.write_str("S-TAB"),
            KeyCode::Left => f.write_str("←"),
            KeyCode::Right => f.write_str("→"),
            KeyCode::Up => f.write_str("↑"),
            KeyCode::Down => f.write_str("↓"),
            KeyCode::Home => f.write_str("Home"),
            KeyCode::End => f.write_str("End"),
            KeyCode::PageUp => f.write_str("PgUp"),
            KeyCode::PageDown => f.write_str("PgDn"),
            KeyCode::Backspace => f.write_str("Bksp"),
            KeyCode::Delete => f.write_str("Del"),
            KeyCode::F(n) => write!(f, "F{n}"),
            other => write!(f, "{other:?}"),
        }
    }
}

pub fn seq(text: &str) -> Vec<KeyChord> {
    text.split(' ')
        .filter(|token| !token.is_empty())
        .map(|token| KeyChord::parse(token).unwrap_or_else(|| panic!("bad key token {token}")))
        .collect()
}

pub fn format_seq(keys: &[KeyChord]) -> String {
    keys.iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(" ")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Scope {
    Global,
    Insert,
    Sidebar,
    Timeline,
    Composer,
    Visual,
    Panel,
    PanelInfo,
    PanelDiff,
    PanelFile,
    PanelTerminal,
    Notifications,
    Settings,
    Automations,
    Pane,
    NewThread,
    ViewVisual,
}

impl Scope {
    pub const ALL: &'static [Scope] = &[
        Scope::Global,
        Scope::Sidebar,
        Scope::Timeline,
        Scope::Composer,
        Scope::Visual,
        Scope::ViewVisual,
        Scope::Insert,
        Scope::NewThread,
        Scope::Panel,
        Scope::PanelInfo,
        Scope::PanelDiff,
        Scope::PanelFile,
        Scope::PanelTerminal,
        Scope::Notifications,
        Scope::Settings,
        Scope::Automations,
        Scope::Pane,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::Insert => "composer (insert)",
            Self::Sidebar => "sidebar",
            Self::Timeline => "timeline",
            Self::Composer => "composer (normal)",
            Self::Visual => "composer (visual)",
            Self::Panel => "right panel",
            Self::PanelInfo => "panel: info",
            Self::PanelDiff => "panel: diff",
            Self::PanelFile => "panel: file",
            Self::PanelTerminal => "panel: terminal",
            Self::Notifications => "notifications",
            Self::Settings => "settings",
            Self::Automations => "automations",
            Self::Pane => "output pane",
            Self::NewThread => "new thread form",
            Self::ViewVisual => "visual (read-only buffers)",
        }
    }

    pub fn takes_counts(self) -> bool {
        !matches!(self, Self::Insert)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Act {
    Cmd(Command),
    Key(KeyChord),
    Char(CharAct),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    pub scope: Scope,
    pub keys: Vec<KeyChord>,
    pub act: Act,
    pub desc: Option<String>,
}

impl Binding {
    pub fn description(&self) -> String {
        if let Some(desc) = &self.desc {
            return desc.clone();
        }
        match self.act {
            Act::Cmd(command) => command.description(),
            Act::Key(chord) => chord.to_string(),
            Act::Char(act) => act.label().to_owned(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolved {
    Exact(Binding),
    Prefix,
    Unbound,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Continuation {
    pub key: KeyChord,
    pub label: String,
    pub group: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Keymap {
    direct: Vec<(KeyChord, Command)>,
    bindings: Vec<Binding>,
    groups: Vec<(Scope, Vec<KeyChord>, &'static str)>,
}

pub const INSERT_RESERVED: &[char] = &[
    'w', 'u', 'h', 'r', 'o', 'v', 'j', 'k', 'n', 'p', 'a', 'e', 'c', '[',
];

impl Default for Keymap {
    fn default() -> Self {
        let mut keymap = Self {
            direct: Vec::new(),
            bindings: Vec::new(),
            groups: Vec::new(),
        };
        crate::bindings::install(&mut keymap);
        keymap
    }
}

impl Keymap {
    pub fn direct(&mut self, chord: KeyChord, command: Command) {
        self.direct.push((chord, command));
    }

    pub fn bind(&mut self, scope: Scope, keys: &str, act: Act) {
        self.bind_keys(scope, seq(keys), act, None);
    }

    pub fn bind_desc(&mut self, scope: Scope, keys: &str, act: Act, desc: &str) {
        self.bind_keys(scope, seq(keys), act, Some(desc.to_owned()));
    }

    pub fn bind_keys(&mut self, scope: Scope, keys: Vec<KeyChord>, act: Act, desc: Option<String>) {
        self.bindings
            .retain(|binding| !(binding.scope == scope && binding.keys == keys));
        self.bindings.push(Binding {
            scope,
            keys,
            act,
            desc,
        });
    }

    pub fn group(&mut self, scope: Scope, keys: &str, name: &'static str) {
        self.groups.push((scope, seq(keys), name));
    }

    pub fn lookup(&self, event: &KeyEvent) -> Option<Command> {
        self.lookup_chord(KeyChord::from_event(event))
    }

    pub fn lookup_chord(&self, chord: KeyChord) -> Option<Command> {
        self.direct
            .iter()
            .find(|(bound, _)| *bound == chord)
            .map(|(_, command)| *command)
    }

    pub fn lookup_insert(&self, chord: KeyChord) -> Option<Command> {
        if !chord.has_ctrl_or_alt() {
            return None;
        }
        let reserved = chord.modifiers == KeyModifiers::CONTROL
            && matches!(chord.code, KeyCode::Char(ch) if INSERT_RESERVED.contains(&ch));
        if reserved {
            return None;
        }
        self.lookup_chord(chord)
    }

    pub fn resolve(&self, scopes: &[Scope], keys: &[KeyChord]) -> Resolved {
        for scope in scopes {
            if let Some(binding) = self
                .bindings
                .iter()
                .find(|binding| binding.scope == *scope && binding.keys == keys)
            {
                return Resolved::Exact(binding.clone());
            }
            if self.bindings.iter().any(|binding| {
                binding.scope == *scope
                    && binding.keys.len() > keys.len()
                    && binding.keys.starts_with(keys)
            }) {
                return Resolved::Prefix;
            }
        }
        Resolved::Unbound
    }

    pub fn group_name(&self, scopes: &[Scope], keys: &[KeyChord]) -> Option<&'static str> {
        scopes.iter().find_map(|scope| {
            self.groups
                .iter()
                .find(|(group_scope, prefix, _)| group_scope == scope && prefix == keys)
                .map(|(_, _, name)| *name)
        })
    }

    pub fn continuations(&self, scopes: &[Scope], prefix: &[KeyChord]) -> Vec<Continuation> {
        let mut found: Vec<Continuation> = Vec::new();
        let mut shadowed: Vec<KeyChord> = Vec::new();
        for scope in scopes {
            let mut here: Vec<Continuation> = Vec::new();
            for binding in self.bindings.iter().filter(|binding| {
                binding.scope == *scope
                    && binding.keys.len() > prefix.len()
                    && binding.keys.starts_with(prefix)
            }) {
                let key = binding.keys[prefix.len()];
                if shadowed.contains(&key) || here.iter().any(|entry| entry.key == key) {
                    continue;
                }
                let mut next = prefix.to_vec();
                next.push(key);
                let leaf = self
                    .bindings
                    .iter()
                    .find(|other| other.scope == *scope && other.keys == next);
                here.push(match leaf {
                    Some(leaf) => Continuation {
                        key,
                        label: leaf.description(),
                        group: false,
                    },
                    None => Continuation {
                        key,
                        label: self
                            .group_name(scopes, &next)
                            .map_or_else(|| "+prefix".to_owned(), str::to_owned),
                        group: true,
                    },
                });
            }
            shadowed.extend(here.iter().map(|entry| entry.key));
            found.extend(here);
        }
        found.sort_by_key(|entry| sort_key(entry.key));
        found
    }

    pub fn scope_bindings(&self, scope: Scope) -> Vec<&Binding> {
        let mut bindings: Vec<&Binding> = self
            .bindings
            .iter()
            .filter(|binding| binding.scope == scope)
            .collect();
        bindings.sort_by_key(|binding| {
            binding
                .keys
                .iter()
                .map(|key| sort_key(*key))
                .collect::<Vec<_>>()
        });
        bindings
    }

    pub fn direct_chords(&self) -> &[(KeyChord, Command)] {
        &self.direct
    }

    pub fn chords_for(&self, command: Command) -> Vec<KeyChord> {
        self.direct
            .iter()
            .filter(|(_, bound)| *bound == command)
            .map(|(chord, _)| *chord)
            .collect()
    }

    pub fn sequences_for(&self, command: Command) -> Vec<String> {
        let aliases = hint_aliases(command);
        let mut leader = Vec::new();
        let mut other = Vec::new();
        for binding in &self.bindings {
            let Act::Cmd(bound) = binding.act else {
                continue;
            };
            if bound != command && !aliases.contains(&bound) {
                continue;
            }
            let text = format_seq(&binding.keys);
            let target = if binding.keys.first() == Some(&KeyChord::ch(' ')) {
                &mut leader
            } else {
                &mut other
            };
            if !target.contains(&text) {
                target.push(text);
            }
        }
        leader.sort_by_key(String::len);
        let mut result = leader;
        for chord in self.chords_for(command) {
            let text = chord.to_string();
            if !result.contains(&text) {
                result.push(text);
            }
        }
        for text in other {
            if !result.contains(&text) {
                result.push(text);
            }
        }
        result
    }

    pub fn label_for(&self, command: Command) -> String {
        if let Some(first) = self.sequences_for(command).into_iter().next() {
            return first;
        }
        crate::ex::ex_for(command)
            .map(|name| format!(":{name}"))
            .unwrap_or_default()
    }

    pub fn labels_for(&self, command: Command) -> String {
        let mut labels = self.sequences_for(command);
        if let Some(name) = crate::ex::ex_for(command) {
            labels.push(format!(":{name}"));
        }
        labels.join(", ")
    }

    pub fn apply_overrides(&mut self, overrides: &[AppKeybindingOverride]) -> usize {
        let mut applied = 0;
        for entry in overrides {
            if entry
                .platform
                .is_some_and(|platform| platform != KeyboardPlatform::Linux)
            {
                continue;
            }
            if !is_app_command_id(&entry.command) {
                continue;
            }
            let Some(command) = Command::from_app_command_id(&entry.command) else {
                continue;
            };
            let chord = match &entry.shortcut {
                None => None,
                Some(shortcut) => match KeyChord::from_shortcut(shortcut) {
                    Some(chord) => Some(chord),
                    None => continue,
                },
            };
            self.direct.retain(|(_, bound)| *bound != command);
            if let Some(chord) = chord {
                self.direct.retain(|(bound, _)| *bound != chord);
                self.direct.push((chord, command));
            }
            applied += 1;
        }
        applied
    }
}

fn hint_aliases(command: Command) -> &'static [Command] {
    match command {
        Command::PanelFullScreen | Command::PaneMaximize => &[Command::WindowOnly],
        _ => &[],
    }
}

fn sort_key(chord: KeyChord) -> (u8, u8, String) {
    let modified = chord.has_ctrl_or_alt();
    let class = match chord.code {
        _ if modified => 4,
        KeyCode::Char(ch) if ch.is_lowercase() => 0,
        KeyCode::Char(ch) if ch.is_uppercase() => 1,
        KeyCode::Char(ch) if ch.is_ascii_digit() => 2,
        KeyCode::Char(_) => 3,
        _ => 5,
    };
    let lower = match chord.code {
        KeyCode::Char(ch) => ch.to_lowercase().next().map_or(0, |ch| ch as u32 as u8),
        _ => 0,
    };
    (class, lower, chord.to_string())
}

pub fn translate_russian(chord: KeyChord) -> KeyChord {
    const LOWER: [(char, char); 33] = [
        ('й', 'q'),
        ('ц', 'w'),
        ('у', 'e'),
        ('к', 'r'),
        ('е', 't'),
        ('н', 'y'),
        ('г', 'u'),
        ('ш', 'i'),
        ('щ', 'o'),
        ('з', 'p'),
        ('х', '['),
        ('ъ', ']'),
        ('ф', 'a'),
        ('ы', 's'),
        ('в', 'd'),
        ('а', 'f'),
        ('п', 'g'),
        ('р', 'h'),
        ('о', 'j'),
        ('л', 'k'),
        ('д', 'l'),
        ('ж', ';'),
        ('э', '\''),
        ('я', 'z'),
        ('ч', 'x'),
        ('с', 'c'),
        ('м', 'v'),
        ('и', 'b'),
        ('т', 'n'),
        ('ь', 'm'),
        ('б', ','),
        ('ю', '.'),
        ('ё', '`'),
    ];
    let KeyCode::Char(ch) = chord.code else {
        return chord;
    };
    let upper = ch.is_uppercase();
    let lower = ch.to_lowercase().next().unwrap_or(ch);
    let Some((_, latin)) = LOWER.iter().find(|(cyrillic, _)| *cyrillic == lower) else {
        return chord;
    };
    let latin = if upper {
        match *latin {
            '[' => '{',
            ']' => '}',
            ';' => ':',
            '\'' => '"',
            ',' => '<',
            '.' => '>',
            '`' => '~',
            other => other.to_ascii_uppercase(),
        }
    } else {
        *latin
    };
    KeyChord::new(KeyCode::Char(latin), chord.modifiers)
}

pub fn insert_command(at: InsertAt) -> Command {
    Command::Edit(EditOp::Insert(at))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shortcut(key: &str) -> AppShortcut {
        AppShortcut {
            key: key.into(),
            ..AppShortcut::default()
        }
    }

    #[test]
    fn every_app_command_mapping_is_a_known_app_command_id() {
        for command in Command::ALL {
            if let Some(id) = command.app_command_id() {
                assert!(is_app_command_id(id), "{id}");
                assert_eq!(Command::from_app_command_id(id), Some(*command));
            }
        }
    }

    #[test]
    fn key_tokens_round_trip() {
        for token in [
            "SPC", "RET", "ESC", "TAB", "S-TAB", "C-w", "M-x", "C-RET", "M-SPC", "G", "Bksp",
        ] {
            let chord = KeyChord::parse(token).unwrap();
            assert_eq!(chord.to_string(), token);
        }
        let shifted = KeyChord::from_event(&KeyEvent::new(KeyCode::Char('g'), KeyModifiers::SHIFT));
        assert_eq!(shifted, KeyChord::ch('G'));
        assert_eq!(format_seq(&seq("SPC b k")), "SPC b k");
    }

    #[test]
    fn resolve_prefers_the_first_scope_that_knows_the_sequence() {
        let keymap = Keymap::default();
        assert_eq!(
            keymap.resolve(&[Scope::Sidebar, Scope::Global], &seq("SPC")),
            Resolved::Prefix
        );
        match keymap.resolve(&[Scope::Sidebar, Scope::Global], &seq("SPC b k")) {
            Resolved::Exact(binding) => assert_eq!(binding.act, Act::Cmd(Command::Archive)),
            other => panic!("{other:?}"),
        }
        match keymap.resolve(&[Scope::Visual, Scope::Composer, Scope::Global], &seq("i")) {
            Resolved::Prefix => {}
            other => panic!("{other:?}"),
        }
        match keymap.resolve(&[Scope::Composer, Scope::Global], &seq("i")) {
            Resolved::Exact(binding) => {
                assert_eq!(binding.act, Act::Cmd(insert_command(InsertAt::Before)))
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            keymap.resolve(&[Scope::Sidebar, Scope::Global], &seq("SPC b z")),
            Resolved::Unbound
        );
    }

    #[test]
    fn continuations_list_groups_and_leaves() {
        let keymap = Keymap::default();
        let entries = keymap.continuations(&[Scope::Timeline, Scope::Global], &seq("SPC"));
        let buffer = entries
            .iter()
            .find(|entry| entry.key == KeyChord::ch('b'))
            .unwrap();
        assert!(buffer.group);
        assert_eq!(buffer.label, "+buffer");
        let palette = entries
            .iter()
            .find(|entry| entry.key == KeyChord::ch(':'))
            .unwrap();
        assert!(!palette.group);
        assert_eq!(palette.label, "Command palette");
    }

    #[test]
    fn overrides_rebind_unbind_and_skip_unmappable() {
        let mut keymap = Keymap::default();
        let mut rename = shortcut("e");
        rename.primary_modifier = true;
        let mut meta_only = shortcut("x");
        meta_only.meta = true;
        let overrides = vec![
            AppKeybindingOverride {
                command: "thread.rename".into(),
                shortcut: Some(rename),
                platform: None,
            },
            AppKeybindingOverride {
                command: "palette.open".into(),
                shortcut: None,
                platform: Some(KeyboardPlatform::Linux),
            },
            AppKeybindingOverride {
                command: "thread.search".into(),
                shortcut: Some(shortcut("s")),
                platform: Some(KeyboardPlatform::Mac),
            },
            AppKeybindingOverride {
                command: "panel.toggle".into(),
                shortcut: Some(meta_only),
                platform: None,
            },
            AppKeybindingOverride {
                command: "browser.find".into(),
                shortcut: Some(shortcut("z")),
                platform: None,
            },
        ];
        assert_eq!(keymap.apply_overrides(&overrides), 2);
        assert_eq!(
            keymap.lookup(&KeyEvent::new(KeyCode::Char('e'), KeyModifiers::CONTROL)),
            Some(Command::Rename)
        );
        assert_eq!(
            keymap.lookup(&KeyEvent::new(KeyCode::Char('x'), KeyModifiers::ALT)),
            None
        );
        assert_eq!(keymap.label_for(Command::PaletteOpen), "SPC :");
        assert_eq!(keymap.label_for(Command::Rename), "SPC b R");
        assert!(keymap.labels_for(Command::Rename).contains("C-e"));
    }

    #[test]
    fn override_steals_chord_from_other_command() {
        let mut keymap = Keymap::default();
        keymap.apply_overrides(&[AppKeybindingOverride {
            command: "thread.archive".into(),
            shortcut: Some(shortcut("1")),
            platform: None,
        }]);
        assert_eq!(
            keymap.lookup(&KeyEvent::from(KeyCode::Char('1'))),
            Some(Command::Archive)
        );
        keymap.apply_overrides(&[AppKeybindingOverride {
            command: "thread.jump.1".into(),
            shortcut: Some(shortcut("1")),
            platform: None,
        }]);
        assert_eq!(
            keymap.lookup(&KeyEvent::from(KeyCode::Char('1'))),
            Some(Command::Jump(1))
        );
        assert!(keymap.chords_for(Command::Archive).is_empty());
    }

    #[test]
    fn insert_lookup_skips_reserved_and_plain_chords() {
        let mut keymap = Keymap::default();
        keymap.apply_overrides(&[
            AppKeybindingOverride {
                command: "palette.open".into(),
                shortcut: Some(AppShortcut {
                    key: "k".into(),
                    primary_modifier: true,
                    ..AppShortcut::default()
                }),
                platform: None,
            },
            AppKeybindingOverride {
                command: "file.quickOpen".into(),
                shortcut: Some(AppShortcut {
                    key: "y".into(),
                    primary_modifier: true,
                    ..AppShortcut::default()
                }),
                platform: None,
            },
        ]);
        assert_eq!(keymap.lookup_insert(KeyChord::ctrl('k')), None);
        assert_eq!(
            keymap.lookup_insert(KeyChord::ctrl('y')),
            Some(Command::QuickOpen)
        );
        assert_eq!(
            keymap.lookup_insert(KeyChord::alt('m')),
            Some(Command::ModelPicker)
        );
        assert_eq!(keymap.lookup_insert(KeyChord::ch('j')), None);
    }

    #[test]
    fn shortcut_conversion() {
        let mut arrow = shortcut("ArrowLeft");
        arrow.control = true;
        arrow.shift = true;
        assert_eq!(
            KeyChord::from_shortcut(&arrow),
            Some(KeyChord::new(
                KeyCode::Left,
                KeyModifiers::CONTROL | KeyModifiers::SHIFT
            ))
        );
        let mut upper = shortcut("K");
        upper.shift = true;
        assert_eq!(KeyChord::from_shortcut(&upper), Some(KeyChord::ch('K')));
        assert_eq!(KeyChord::from_shortcut(&shortcut("Unidentified")), None);
        assert_eq!(KeyChord::ctrl('r').to_string(), "C-r");
    }

    #[test]
    fn russian_keys_map_to_latin_positions() {
        assert_eq!(translate_russian(KeyChord::ch('о')), KeyChord::ch('j'));
        assert_eq!(translate_russian(KeyChord::ch('Л')), KeyChord::ch('K'));
        assert_eq!(translate_russian(KeyChord::ctrl('ц')), KeyChord::ctrl('w'));
        assert_eq!(translate_russian(KeyChord::ch('j')), KeyChord::ch('j'));
    }
}
