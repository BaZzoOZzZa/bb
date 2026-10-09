use std::fmt;

use bb_wire::system::{AppKeybindingOverride, AppShortcut, KeyboardPlatform, is_app_command_id};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Command {
    Quit,
    Help,
    Back,
    FocusNext,
    FocusPrevious,
    MoveDown,
    MoveUp,
    MoveTop,
    MoveBottom,
    Open,
    Filter,
    Collapse,
    Expand,
    ToggleGroup,
    Rename,
    Archive,
    UndoArchive,
    ToggleRead,
    TogglePin,
    ThreadNext,
    ThreadPrevious,
    Jump(u8),
    SidebarToggle,
    PanelToggle,
    Reload,
    PageDown,
    PageUp,
    HalfPageDown,
    HalfPageUp,
    NextMessage,
    PreviousMessage,
    Copy,
    OpenExternal,
    FocusComposer,
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
}

impl Command {
    pub const ALL: &'static [Command] = &[
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
        Command::OpenExternal,
        Command::FocusComposer,
        Command::StopTurn,
        Command::QueueList,
        Command::ToggleSubmitMode,
        Command::ModelPicker,
        Command::ReasoningPicker,
        Command::PermissionPicker,
        Command::CycleModel,
        Command::CycleModelBackward,
        Command::CycleReasoning,
        Command::CycleReasoningBackward,
        Command::ThreadNext,
        Command::ThreadPrevious,
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
        Command::SidebarToggle,
        Command::PanelToggle,
        Command::Reload,
        Command::Back,
        Command::Help,
        Command::Quit,
    ];

    pub fn app_command_id(self) -> Option<&'static str> {
        Some(match self {
            Self::Back => "app.back",
            Self::FocusNext => "pane.focus.next",
            Self::FocusPrevious => "pane.focus.previous",
            Self::Filter => "thread.search",
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
            Self::Reload => "window.reload",
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

    pub fn description(self) -> String {
        match self {
            Self::Quit => "Quit".into(),
            Self::Help => "Toggle this help".into(),
            Self::Back => "Back / clear filter / close".into(),
            Self::FocusNext => "Focus next pane".into(),
            Self::FocusPrevious => "Focus previous pane".into(),
            Self::MoveDown => "Move down".into(),
            Self::MoveUp => "Move up".into(),
            Self::MoveTop => "Go to top".into(),
            Self::MoveBottom => "Go to bottom".into(),
            Self::Open => "Open thread / expand row".into(),
            Self::Filter => "Filter threads".into(),
            Self::Collapse => "Collapse group / row".into(),
            Self::Expand => "Expand group / row".into(),
            Self::ToggleGroup => "Toggle group / row".into(),
            Self::Rename => "Rename thread".into(),
            Self::Archive => "Archive / unarchive thread".into(),
            Self::UndoArchive => "Undo last archive".into(),
            Self::ToggleRead => "Mark read / unread".into(),
            Self::TogglePin => "Pin / unpin thread".into(),
            Self::ThreadNext => "Open next thread".into(),
            Self::ThreadPrevious => "Open previous thread".into(),
            Self::Jump(n) => format!("Open thread #{n}"),
            Self::SidebarToggle => "Toggle sidebar".into(),
            Self::PanelToggle => "Toggle right panel".into(),
            Self::Reload => "Reload data".into(),
            Self::PageDown => "Page down".into(),
            Self::PageUp => "Page up".into(),
            Self::HalfPageDown => "Half page down".into(),
            Self::HalfPageUp => "Half page up".into(),
            Self::NextMessage => "Next user message".into(),
            Self::PreviousMessage => "Previous user message".into(),
            Self::Copy => "Copy selected row (OSC 52)".into(),
            Self::OpenExternal => "Open link / file of selected row".into(),
            Self::FocusComposer => "Focus the composer".into(),
            Self::StopTurn => "Stop the running turn".into(),
            Self::ModelPicker => "Pick model".into(),
            Self::ReasoningPicker => "Pick reasoning level".into(),
            Self::PermissionPicker => "Pick permission mode".into(),
            Self::CycleModel => "Next model".into(),
            Self::CycleModelBackward => "Previous model".into(),
            Self::CycleReasoning => "Next reasoning level".into(),
            Self::CycleReasoningBackward => "Previous reasoning level".into(),
            Self::QueueList => "Queued messages (edit, delete, send now)".into(),
            Self::ToggleSubmitMode => "Toggle Enter: queue / steer while running".into(),
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
            KeyCode::Char(' ') => f.write_str("Space"),
            KeyCode::Char(ch) => write!(f, "{ch}"),
            KeyCode::Enter => f.write_str("Enter"),
            KeyCode::Esc => f.write_str("Esc"),
            KeyCode::Tab => f.write_str("Tab"),
            KeyCode::BackTab => f.write_str("S-Tab"),
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Keymap {
    bindings: Vec<(KeyChord, Command)>,
}

impl Default for Keymap {
    fn default() -> Self {
        use KeyCode::*;
        let mut bindings = vec![
            (KeyChord::ch('q'), Command::Quit),
            (KeyChord::ctrl('c'), Command::Quit),
            (KeyChord::ch('?'), Command::Help),
            (KeyChord::plain(Esc), Command::Back),
            (KeyChord::plain(Tab), Command::FocusNext),
            (KeyChord::plain(BackTab), Command::FocusPrevious),
            (KeyChord::ch('j'), Command::MoveDown),
            (KeyChord::plain(Down), Command::MoveDown),
            (KeyChord::ch('k'), Command::MoveUp),
            (KeyChord::plain(Up), Command::MoveUp),
            (KeyChord::ch('g'), Command::MoveTop),
            (KeyChord::plain(Home), Command::MoveTop),
            (KeyChord::ch('G'), Command::MoveBottom),
            (KeyChord::plain(End), Command::MoveBottom),
            (KeyChord::plain(Enter), Command::Open),
            (KeyChord::ch('/'), Command::Filter),
            (KeyChord::ch('h'), Command::Collapse),
            (KeyChord::plain(Left), Command::Collapse),
            (KeyChord::ch('l'), Command::Expand),
            (KeyChord::plain(Right), Command::Expand),
            (KeyChord::ch(' '), Command::ToggleGroup),
            (KeyChord::ch('r'), Command::Rename),
            (KeyChord::ch('a'), Command::Archive),
            (KeyChord::ch('u'), Command::UndoArchive),
            (KeyChord::ch('m'), Command::ToggleRead),
            (KeyChord::ch('p'), Command::TogglePin),
            (KeyChord::ch('J'), Command::ThreadNext),
            (KeyChord::ch('K'), Command::ThreadPrevious),
            (KeyChord::ctrl('b'), Command::SidebarToggle),
            (KeyChord::ch('i'), Command::PanelToggle),
            (KeyChord::ctrl('r'), Command::Reload),
            (KeyChord::plain(PageDown), Command::PageDown),
            (KeyChord::plain(PageUp), Command::PageUp),
            (KeyChord::ctrl('f'), Command::PageDown),
            (KeyChord::ctrl('d'), Command::HalfPageDown),
            (KeyChord::ctrl('u'), Command::HalfPageUp),
            (KeyChord::ch(']'), Command::NextMessage),
            (KeyChord::ch('['), Command::PreviousMessage),
            (KeyChord::ch('y'), Command::Copy),
            (KeyChord::ch('o'), Command::OpenExternal),
            (KeyChord::ch('c'), Command::FocusComposer),
            (KeyChord::ch('S'), Command::StopTurn),
            (KeyChord::alt('q'), Command::QueueList),
            (KeyChord::alt('t'), Command::ToggleSubmitMode),
            (KeyChord::alt('m'), Command::ModelPicker),
            (KeyChord::alt('e'), Command::ReasoningPicker),
            (KeyChord::alt('p'), Command::PermissionPicker),
            (KeyChord::alt('.'), Command::CycleModel),
            (KeyChord::alt(','), Command::CycleModelBackward),
            (KeyChord::alt('r'), Command::CycleReasoning),
            (
                KeyChord::new(KeyCode::Char('r'), KeyModifiers::ALT | KeyModifiers::SHIFT),
                Command::CycleReasoningBackward,
            ),
        ];
        for n in 1..=9u8 {
            let digit = char::from(b'0' + n);
            bindings.push((KeyChord::alt(digit), Command::Jump(n)));
        }
        Self { bindings }
    }
}

impl Keymap {
    pub fn is_composer_command(command: Command) -> bool {
        matches!(
            command,
            Command::FocusNext
                | Command::FocusPrevious
                | Command::Jump(_)
                | Command::QueueList
                | Command::ToggleSubmitMode
                | Command::ModelPicker
                | Command::ReasoningPicker
                | Command::PermissionPicker
                | Command::CycleModel
                | Command::CycleModelBackward
                | Command::CycleReasoning
                | Command::CycleReasoningBackward
        )
    }

    pub fn lookup(&self, event: &KeyEvent) -> Option<Command> {
        let chord = KeyChord::from_event(event);
        self.bindings
            .iter()
            .find(|(bound, _)| *bound == chord)
            .map(|(_, command)| *command)
    }

    pub fn chords_for(&self, command: Command) -> Vec<KeyChord> {
        self.bindings
            .iter()
            .filter(|(_, bound)| *bound == command)
            .map(|(chord, _)| *chord)
            .collect()
    }

    pub fn label_for(&self, command: Command) -> String {
        self.chords_for(command)
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("/")
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
            self.bindings.retain(|(_, bound)| *bound != command);
            if let Some(chord) = chord {
                self.bindings.retain(|(bound, _)| *bound != chord);
                self.bindings.push((chord, command));
            }
            applied += 1;
        }
        applied
    }
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
    fn shift_letters_normalize_to_uppercase() {
        let keymap = Keymap::default();
        let shifted = KeyEvent::new(KeyCode::Char('G'), KeyModifiers::SHIFT);
        assert_eq!(keymap.lookup(&shifted), Some(Command::MoveBottom));
        let back_tab = KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT);
        assert_eq!(keymap.lookup(&back_tab), Some(Command::FocusPrevious));
        let question = KeyEvent::new(KeyCode::Char('?'), KeyModifiers::SHIFT);
        assert_eq!(keymap.lookup(&question), Some(Command::Help));
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
                command: "thread.archive".into(),
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
                command: "palette.open".into(),
                shortcut: Some(shortcut("z")),
                platform: None,
            },
        ];
        assert_eq!(keymap.apply_overrides(&overrides), 2);
        assert_eq!(
            keymap.lookup(&KeyEvent::new(KeyCode::Char('e'), KeyModifiers::CONTROL)),
            Some(Command::Rename)
        );
        assert_eq!(keymap.lookup(&KeyEvent::from(KeyCode::Char('r'))), None);
        assert_eq!(keymap.lookup(&KeyEvent::from(KeyCode::Char('a'))), None);
        assert_eq!(
            keymap.lookup(&KeyEvent::from(KeyCode::Char('/'))),
            Some(Command::Filter)
        );
        assert_eq!(
            keymap.lookup(&KeyEvent::from(KeyCode::Char('i'))),
            Some(Command::PanelToggle)
        );
    }

    #[test]
    fn override_steals_chord_from_other_command() {
        let mut keymap = Keymap::default();
        keymap.apply_overrides(&[AppKeybindingOverride {
            command: "thread.archive".into(),
            shortcut: Some(shortcut("j")),
            platform: None,
        }]);
        assert_eq!(
            keymap.lookup(&KeyEvent::from(KeyCode::Char('j'))),
            Some(Command::Archive)
        );
        assert_eq!(keymap.label_for(Command::MoveDown), "↓");
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
}
