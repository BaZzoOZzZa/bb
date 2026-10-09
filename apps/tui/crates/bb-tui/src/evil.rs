use std::collections::HashMap;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::{AppState, Effect, Focus, Mode};
use crate::keymap::{Act, Command, KeyChord, Resolved, Scope, format_seq, translate_russian};
use crate::vi::{CharAct, LastFind};

pub const WHICH_KEY_DELAY: Duration = Duration::from_millis(300);
const MAX_COUNT: usize = 9999;
const MAX_REPEAT: usize = 500;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Echo {
    pub text: String,
    pub error: bool,
}

#[derive(Debug, Clone)]
pub struct KeyState {
    pub pending: Vec<KeyChord>,
    pub count: Option<usize>,
    pub register: Option<char>,
    pub await_char: Option<(CharAct, Option<usize>)>,
    pub pending_at: Option<Instant>,
    pub which_key: bool,
    pub which_key_page: usize,
    pub which_key_delay: Duration,
    pub describe: bool,
    pub echo: Option<Echo>,
    pub last_find: LastFind,
}

impl Default for KeyState {
    fn default() -> Self {
        Self {
            pending: Vec::new(),
            count: None,
            register: None,
            await_char: None,
            pending_at: None,
            which_key: false,
            which_key_page: 0,
            which_key_delay: WHICH_KEY_DELAY,
            describe: false,
            echo: None,
            last_find: LastFind::default(),
        }
    }
}

impl KeyState {
    pub fn reset(&mut self) {
        self.pending.clear();
        self.count = None;
        self.register = None;
        self.await_char = None;
        self.pending_at = None;
        self.which_key = false;
        self.which_key_page = 0;
    }

    fn end_sequence(&mut self) {
        self.pending.clear();
        self.pending_at = None;
        self.which_key = false;
        self.which_key_page = 0;
    }

    pub fn is_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    pub fn showcmd(&self) -> String {
        let mut text = String::new();
        if let Some(register) = self.register {
            text.push('"');
            text.push(register);
        }
        if let Some(count) = self.count {
            text.push_str(&count.to_string());
        }
        if !self.pending.is_empty() {
            text.push_str(&format_seq(&self.pending));
        }
        if let Some((act, _)) = self.await_char {
            let tag = match act {
                CharAct::Find {
                    forward: true,
                    till: false,
                } => "f",
                CharAct::Find {
                    forward: false,
                    till: false,
                } => "F",
                CharAct::Find {
                    forward: true,
                    till: true,
                } => "t",
                CharAct::Find {
                    forward: false,
                    till: true,
                } => "T",
                CharAct::ApplyFind { .. } => "…",
                CharAct::Replace | CharAct::VisualReplace => "r",
                CharAct::Register => "\"",
                CharAct::InsertRegister => "C-r",
            };
            if !text.is_empty() && !text.ends_with(' ') {
                text.push(' ');
            }
            text.push_str(tag);
        }
        text
    }

    pub fn tick(&mut self, now: Instant) {
        if self.pending.is_empty() || self.which_key {
            return;
        }
        match self.pending_at {
            None => self.pending_at = Some(now),
            Some(at) if now.duration_since(at) >= self.which_key_delay => self.which_key = true,
            Some(_) => {}
        }
    }

    pub fn deadline(&self) -> Option<Instant> {
        if self.pending.is_empty() || self.which_key {
            return None;
        }
        self.pending_at.map(|at| at + self.which_key_delay)
    }

    pub fn needs_arming(&self) -> bool {
        !self.pending.is_empty() && !self.which_key && self.pending_at.is_none()
    }

    pub fn start_leader(&mut self) {
        self.pending = vec![KeyChord::ch(' ')];
        self.pending_at = None;
    }

    pub fn message(&mut self, text: impl Into<String>) {
        self.echo = Some(Echo {
            text: text.into(),
            error: false,
        });
    }

    pub fn error(&mut self, text: impl Into<String>) {
        self.echo = Some(Echo {
            text: text.into(),
            error: true,
        });
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisterValue {
    pub text: String,
    pub linewise: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Registers {
    values: HashMap<char, RegisterValue>,
    pub clipboard: bool,
}

impl Default for Registers {
    fn default() -> Self {
        Self {
            values: HashMap::new(),
            clipboard: true,
        }
    }
}

impl Registers {
    pub fn get(&self, name: Option<char>) -> Option<&RegisterValue> {
        let name = match name {
            None | Some('"') => '"',
            Some('*') => '+',
            Some(other) => other.to_ascii_lowercase(),
        };
        self.values.get(&name)
    }

    pub fn set(&mut self, name: char, value: RegisterValue) {
        self.values.insert(name, value);
    }

    fn write_named(&mut self, name: char, value: &RegisterValue) {
        if name.is_ascii_uppercase() {
            let lower = name.to_ascii_lowercase();
            let entry = self.values.entry(lower).or_insert(RegisterValue {
                text: String::new(),
                linewise: value.linewise,
            });
            if entry.linewise && !entry.text.ends_with('\n') && !entry.text.is_empty() {
                entry.text.push('\n');
            }
            entry.text.push_str(&value.text);
            entry.linewise |= value.linewise;
        } else {
            self.values.insert(name, value.clone());
        }
    }

    pub fn yank(&mut self, name: Option<char>, value: RegisterValue) -> Option<String> {
        match name {
            Some('_') => None,
            Some('+' | '*') => {
                self.values.insert('+', value.clone());
                self.values.insert('"', value.clone());
                Some(value.text)
            }
            Some(named) if named.is_ascii_alphabetic() => {
                self.write_named(named, &value);
                let stored = self.get(Some(named)).cloned().unwrap_or(value);
                self.values.insert('"', stored);
                None
            }
            _ => {
                self.values.insert('0', value.clone());
                self.values.insert('"', value.clone());
                if self.clipboard {
                    self.values.insert('+', value.clone());
                    Some(value.text)
                } else {
                    None
                }
            }
        }
    }

    pub fn delete(&mut self, name: Option<char>, value: RegisterValue) -> Option<String> {
        match name {
            Some('_') => None,
            Some('+' | '*') => {
                self.values.insert('+', value.clone());
                self.values.insert('"', value.clone());
                Some(value.text)
            }
            Some(named) if named.is_ascii_alphabetic() => {
                self.write_named(named, &value);
                self.values.insert('"', value);
                None
            }
            _ => {
                if value.linewise || value.text.contains('\n') {
                    for index in (1..9).rev() {
                        let from = char::from(b'0' + index);
                        let to = char::from(b'0' + index + 1);
                        if let Some(moved) = self.values.remove(&from) {
                            self.values.insert(to, moved);
                        }
                    }
                    self.values.insert('1', value.clone());
                } else {
                    self.values.insert('-', value.clone());
                }
                self.values.insert('"', value);
                None
            }
        }
    }

    pub fn note_clipboard(&mut self, text: &str) {
        self.values.insert(
            '+',
            RegisterValue {
                text: text.to_owned(),
                linewise: false,
            },
        );
    }

    pub fn listing(&self) -> Vec<(char, String)> {
        let mut names: Vec<char> = self.values.keys().copied().collect();
        names.sort_by_key(|name| match name {
            '"' => (0, *name),
            '0'..='9' => (1, *name),
            'a'..='z' => (2, *name),
            _ => (3, *name),
        });
        names
            .into_iter()
            .map(|name| (name, self.values[&name].text.clone()))
            .collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tag {
    Normal,
    Insert,
    Visual,
    VisualLine,
    Operator,
    Replace,
    Command,
}

impl Tag {
    pub fn label(self) -> &'static str {
        match self {
            Self::Normal => "<N>",
            Self::Insert => "<I>",
            Self::Visual => "<V>",
            Self::VisualLine => "<L>",
            Self::Operator => "<O>",
            Self::Replace => "<R>",
            Self::Command => "<E>",
        }
    }
}

pub fn tag(state: &AppState) -> Tag {
    if matches!(state.mode, Mode::Ex(_)) {
        return Tag::Command;
    }
    if state.palette.is_some()
        || matches!(
            state.mode,
            Mode::Filter(_) | Mode::Rename { .. } | Mode::AttachPrompt { .. }
        )
        || state.terminal_input_active()
        || state
            .settings
            .as_ref()
            .is_some_and(|view| view.captures_keys())
        || state
            .automations
            .as_ref()
            .is_some_and(|view| view.captures_keys())
    {
        return Tag::Insert;
    }
    if let Some(composer) = state.focused_composer() {
        let operator = state.keys.pending.first().is_some_and(|first| {
            matches!(first.code, KeyCode::Char('d' | 'c' | 'y' | '>' | '<'))
                && !first.has_ctrl_or_alt()
                || (state.keys.pending.len() >= 2
                    && first.code == KeyCode::Char('g')
                    && matches!(state.keys.pending[1].code, KeyCode::Char('u' | 'U' | '~')))
        });
        return match composer.mode {
            crate::composer::EditMode::Insert => Tag::Insert,
            crate::composer::EditMode::Visual { line: false, .. } => Tag::Visual,
            crate::composer::EditMode::Visual { line: true, .. } => Tag::VisualLine,
            crate::composer::EditMode::Normal if operator => Tag::Operator,
            crate::composer::EditMode::Normal => Tag::Normal,
        };
    }
    Tag::Normal
}

pub(crate) fn normal_key(state: &mut AppState, key: KeyEvent, scopes: &[Scope]) -> Vec<Effect> {
    let raw = KeyChord::from_event(&key);
    if let Some((act, count)) = state.keys.await_char.take() {
        return match raw.is_plain_char() {
            Some(ch) => crate::edit::char_act(state, act, ch, count),
            None => {
                state.keys.reset();
                Vec::new()
            }
        };
    }
    let chord = translate_russian(raw);
    let mut chain: Vec<Scope> = scopes.to_vec();
    if !chain.contains(&Scope::Global) {
        chain.push(Scope::Global);
    }
    if state.keys.is_pending() {
        let paging = state.keys.which_key
            && matches!(
                (chord.code, chord.modifiers),
                (KeyCode::Char('n' | 'p'), KeyModifiers::CONTROL)
                    | (KeyCode::PageDown | KeyCode::PageUp, _)
            );
        if paging {
            let forward = matches!(chord.code, KeyCode::Char('n') | KeyCode::PageDown);
            let page = state.keys.which_key_page;
            state.keys.which_key_page = if forward {
                page + 1
            } else {
                page.saturating_sub(1)
            };
            return Vec::new();
        }
        if chord == KeyChord::plain(KeyCode::Esc) || chord == KeyChord::ctrl('g') {
            state.keys.reset();
            state.keys.describe = false;
            return Vec::new();
        }
        if chord == KeyChord::plain(KeyCode::Backspace) {
            state.keys.pending.pop();
            state.keys.which_key_page = 0;
            if state.keys.pending.is_empty() {
                state.keys.end_sequence();
            }
            return Vec::new();
        }
    } else {
        if chord == KeyChord::alt(' ') {
            state.keys.start_leader();
            return Vec::new();
        }
        if let Some(command) = state.keymap.lookup_chord(raw) {
            let count = state.keys.count.take();
            state.keys.register = None;
            if state.keys.describe {
                state.keys.describe = false;
                state
                    .keys
                    .message(format!("{raw} runs {}", command.description()));
                return Vec::new();
            }
            return run_counted(state, command, count);
        }
        if let Some(ch) = chord.is_plain_char()
            && ch.is_ascii_digit()
            && (ch != '0' || state.keys.count.is_some())
            && chain.first().is_some_and(|scope| scope.takes_counts())
        {
            let digit = usize::from(ch as u8 - b'0');
            let count = state.keys.count.unwrap_or(0) * 10 + digit;
            state.keys.count = Some(count.min(MAX_COUNT));
            return Vec::new();
        }
    }
    let mut keys = state.keys.pending.clone();
    keys.push(chord);
    match state.keymap.resolve(&chain, &keys) {
        Resolved::Exact(binding) => {
            state.keys.end_sequence();
            let count = state.keys.count.take();
            if state.keys.describe {
                state.keys.describe = false;
                state.keys.register = None;
                state.keys.message(format!(
                    "{} runs {}",
                    format_seq(&binding.keys),
                    binding.description()
                ));
                return Vec::new();
            }
            run_act(state, binding.act, count)
        }
        Resolved::Prefix => {
            state.keys.pending = keys;
            state.keys.which_key_page = 0;
            if !state.keys.which_key {
                state.keys.pending_at = None;
            }
            Vec::new()
        }
        Resolved::Unbound => {
            let was_pending = state.keys.is_pending();
            state.keys.reset();
            state.keys.describe = false;
            if was_pending {
                state
                    .keys
                    .error(format!("{} is undefined", format_seq(&keys)));
            }
            Vec::new()
        }
    }
}

pub(crate) fn run_act(state: &mut AppState, act: Act, count: Option<usize>) -> Vec<Effect> {
    match act {
        Act::Cmd(command) => run_counted(state, command, count),
        Act::Key(chord) => {
            state.keys.register = None;
            let times = count.unwrap_or(1).clamp(1, MAX_REPEAT);
            let mut effects = Vec::new();
            for _ in 0..times {
                effects.extend(crate::app::legacy_key(state, chord.event()));
            }
            effects
        }
        Act::Char(act) => {
            state.keys.await_char = Some((act, count));
            Vec::new()
        }
    }
}

pub(crate) fn run_counted(
    state: &mut AppState,
    command: Command,
    count: Option<usize>,
) -> Vec<Effect> {
    if let Command::Edit(op) = command {
        return crate::edit::apply_edit(state, op, count);
    }
    state.keys.register = None;
    if let Some(effects) = crate::app::counted_command(state, command, count) {
        return effects;
    }
    if command.repeats() {
        let times = count.unwrap_or(1).clamp(1, MAX_REPEAT);
        let mut effects = Vec::new();
        for _ in 0..times {
            effects.extend(crate::app::run_command(state, command));
        }
        return effects;
    }
    crate::app::run_command(state, command)
}

pub(crate) fn insert_intercept(state: &mut AppState, key: &KeyEvent) -> Option<Vec<Effect>> {
    let chord = KeyChord::from_event(key);
    if state.keys.is_pending() {
        return Some(normal_key(state, *key, &[Scope::Insert]));
    }
    if chord == KeyChord::alt(' ') {
        state.keys.start_leader();
        return Some(Vec::new());
    }
    if chord == KeyChord::ctrl('c') {
        state.keys.pending = vec![chord];
        state.keys.pending_at = None;
        return Some(Vec::new());
    }
    let command = state.keymap.lookup_insert(chord)?;
    Some(run_counted(state, command, None))
}

pub(crate) fn transient_intercept(state: &mut AppState, key: &KeyEvent) -> Option<Vec<Effect>> {
    let chord = KeyChord::from_event(key);
    if state.keys.is_pending() {
        return Some(normal_key(state, *key, &[]));
    }
    if chord == KeyChord::alt(' ') {
        state.keys.start_leader();
        return Some(Vec::new());
    }
    if state.focus == Focus::Interaction
        && let Some(direction) = match chord {
            c if c == KeyChord::ctrl('h') => Some(crate::keymap::Direction::Left),
            c if c == KeyChord::ctrl('j') => Some(crate::keymap::Direction::Down),
            c if c == KeyChord::ctrl('k') => Some(crate::keymap::Direction::Up),
            c if c == KeyChord::ctrl('l') => Some(crate::keymap::Direction::Right),
            _ => None,
        }
    {
        return Some(crate::app::run_command(
            state,
            Command::WindowFocus(direction),
        ));
    }
    let command = state.keymap.lookup_insert(chord)?;
    Some(run_counted(state, command, None))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value(text: &str, linewise: bool) -> RegisterValue {
        RegisterValue {
            text: text.into(),
            linewise,
        }
    }

    #[test]
    fn yanks_fill_unnamed_zero_and_clipboard() {
        let mut registers = Registers::default();
        assert_eq!(
            registers.yank(None, value("abc", false)),
            Some("abc".to_owned())
        );
        assert_eq!(registers.get(Some('0')).unwrap().text, "abc");
        assert_eq!(registers.get(None).unwrap().text, "abc");
        assert_eq!(registers.yank(Some('a'), value("x", false)), None);
        registers.yank(Some('A'), value("y", false));
        assert_eq!(registers.get(Some('a')).unwrap().text, "xy");
        assert_eq!(registers.yank(Some('_'), value("gone", false)), None);
        assert_eq!(registers.get(None).unwrap().text, "xy");
    }

    #[test]
    fn deletes_shift_numbered_registers() {
        let mut registers = Registers::default();
        registers.delete(None, value("one\n", true));
        registers.delete(None, value("two\n", true));
        registers.delete(None, value("w", false));
        assert_eq!(registers.get(Some('1')).unwrap().text, "two\n");
        assert_eq!(registers.get(Some('2')).unwrap().text, "one\n");
        assert_eq!(registers.get(Some('-')).unwrap().text, "w");
        assert_eq!(registers.get(None).unwrap().text, "w");
        assert_eq!(registers.get(Some('0')), None);
    }

    #[test]
    fn which_key_arms_on_tick_and_opens_after_delay() {
        let start = Instant::now();
        let mut keys = KeyState::default();
        keys.start_leader();
        assert!(keys.needs_arming());
        keys.tick(start);
        assert_eq!(keys.deadline(), Some(start + WHICH_KEY_DELAY));
        keys.tick(start + Duration::from_millis(100));
        assert!(!keys.which_key);
        keys.tick(start + WHICH_KEY_DELAY);
        assert!(keys.which_key);
        assert_eq!(keys.deadline(), None);
        assert_eq!(keys.showcmd(), "SPC");
    }
}
