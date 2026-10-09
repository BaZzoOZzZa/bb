use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::{Color, Modifier, Style};
use ratatui_textarea::{CursorMove, TextArea, WrapMode};

use crate::prompt::PromptDraft;

pub const DRAFT_SAVE_DELAY: Duration = Duration::from_millis(400);
pub const ESC_STOP_WINDOW: Duration = Duration::from_millis(1500);
pub const EDIT_HOLD_RENEWALS_PER_LEASE: u32 = 4;
pub const EDIT_HOLD_RETRY: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SubmitKey {
    #[default]
    Enter,
    ModifierEnter,
}

impl SubmitKey {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "enter" => Some(Self::Enter),
            "ctrl-enter" | "alt-enter" | "modifier-enter" | "mod-enter" => {
                Some(Self::ModifierEnter)
            }
            _ => None,
        }
    }

    pub fn submit_label(self) -> &'static str {
        match self {
            Self::Enter => "Enter",
            Self::ModifierEnter => "M-Enter",
        }
    }

    pub fn newline_label(self) -> &'static str {
        match self {
            Self::Enter => "S-Enter/C-j",
            Self::ModifierEnter => "Enter",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComposerKey {
    Submit,
    Steer,
    Escape,
    Interrupt,
    Edited,
    Handled,
    Unhandled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueuedEdit {
    pub queued_message_id: String,
    pub expected_updated_at: i64,
    pub stash: PromptDraftSnapshot,
    pub renew_at: Option<Instant>,
    pub saving: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptDraftSnapshot(pub Option<String>);

impl PromptDraftSnapshot {
    pub fn of(draft: &PromptDraft) -> Self {
        Self(draft.serialize())
    }

    pub fn draft(&self) -> PromptDraft {
        PromptDraft::parse(self.0.as_deref())
    }
}

#[derive(Debug, Clone)]
pub struct Composer {
    pub thread_id: String,
    textarea: TextArea<'static>,
    draft: PromptDraft,
    pub draft_loaded: bool,
    pub dirty_since: Option<Instant>,
    pub history: Vec<PromptDraft>,
    history_index: Option<usize>,
    history_stash: Option<PromptDraft>,
    pub editing: Option<QueuedEdit>,
    pub esc_armed_at: Option<Instant>,
    pub pending_restore: Option<PromptDraft>,
}

fn configure(textarea: &mut TextArea<'static>) {
    textarea.set_cursor_line_style(Style::default());
    textarea.set_cursor_style(Style::default().add_modifier(Modifier::REVERSED));
    textarea.set_placeholder_style(Style::default().fg(Color::DarkGray));
    textarea.set_wrap_mode(WrapMode::WordOrGlyph);
    textarea.set_max_histories(200);
}

fn textarea_with(text: &str) -> TextArea<'static> {
    let mut textarea = TextArea::new(text.split('\n').map(str::to_owned).collect());
    configure(&mut textarea);
    textarea.move_cursor(CursorMove::Bottom);
    textarea.move_cursor(CursorMove::End);
    textarea
}

fn has(key: &KeyEvent, modifier: KeyModifiers) -> bool {
    key.modifiers.contains(modifier)
}

impl Composer {
    pub fn new(thread_id: impl Into<String>) -> Self {
        Self {
            thread_id: thread_id.into(),
            textarea: textarea_with(""),
            draft: PromptDraft::default(),
            draft_loaded: false,
            dirty_since: None,
            history: Vec::new(),
            history_index: None,
            history_stash: None,
            editing: None,
            esc_armed_at: None,
            pending_restore: None,
        }
    }

    pub fn textarea(&self) -> &TextArea<'static> {
        &self.textarea
    }

    pub fn set_placeholder(&mut self, placeholder: &str) {
        if self.textarea.placeholder_text() != placeholder {
            self.textarea.set_placeholder_text(placeholder);
        }
    }

    pub fn text(&self) -> String {
        self.textarea.lines().join("\n")
    }

    pub fn line_count(&self) -> usize {
        self.textarea.lines().len()
    }

    pub fn draft(&self) -> &PromptDraft {
        &self.draft
    }

    pub fn is_empty(&self) -> bool {
        self.draft.is_empty()
    }

    pub fn has_input(&self) -> bool {
        !self.draft.to_input().is_empty()
    }

    pub fn is_browsing_history(&self) -> bool {
        self.history_index.is_some()
    }

    pub fn history_position(&self) -> Option<(usize, usize)> {
        self.history_index
            .map(|index| (index + 1, self.history.len()))
    }

    fn replace(&mut self, draft: PromptDraft) {
        let placeholder = self.textarea.placeholder_text().to_owned();
        self.textarea = textarea_with(&draft.text);
        self.textarea.set_placeholder_text(placeholder);
        self.draft = draft;
    }

    pub fn set_draft(&mut self, draft: PromptDraft, now: Instant) {
        self.history_index = None;
        self.history_stash = None;
        self.replace(draft);
        self.mark_dirty(now);
    }

    pub fn load_stored(&mut self, raw: Option<&str>) {
        self.draft_loaded = true;
        if !self.draft.is_empty() || self.editing.is_some() {
            return;
        }
        let draft = PromptDraft::parse(raw);
        if !draft.is_empty() {
            self.replace(draft);
        }
    }

    fn mark_dirty(&mut self, now: Instant) {
        if self.editing.is_none() {
            self.dirty_since = Some(now);
        }
    }

    pub fn take_save(&mut self, now: Instant, force: bool) -> Option<Option<String>> {
        let since = self.dirty_since?;
        if !force && now.duration_since(since) < DRAFT_SAVE_DELAY {
            return None;
        }
        self.dirty_since = None;
        Some(self.persisted_draft().serialize())
    }

    pub fn persisted_draft(&self) -> PromptDraft {
        match &self.editing {
            Some(edit) => edit.stash.draft(),
            None => self.draft.clone(),
        }
    }

    pub fn take_submission(&mut self, now: Instant) -> PromptDraft {
        let draft = std::mem::take(&mut self.draft);
        self.history_index = None;
        self.history_stash = None;
        self.replace(PromptDraft::default());
        self.mark_dirty(now);
        draft
    }

    pub fn restore_if_empty(&mut self, draft: PromptDraft, now: Instant) -> bool {
        if !self.draft.is_empty() || self.editing.is_some() {
            return false;
        }
        self.set_draft(draft, now);
        true
    }

    pub fn insert_text(&mut self, text: &str, now: Instant) {
        let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
        self.textarea.insert_str(normalized);
        self.sync(now);
    }

    fn sync(&mut self, now: Instant) {
        let text = self.text();
        if text != self.draft.text {
            self.draft = self.draft.with_text(text);
            self.history_index = None;
            self.history_stash = None;
            self.mark_dirty(now);
        }
    }

    pub fn begin_queued_edit(
        &mut self,
        queued_message_id: String,
        expected_updated_at: i64,
        draft: PromptDraft,
    ) {
        let stash = match self.editing.take() {
            Some(edit) => edit.stash,
            None => PromptDraftSnapshot::of(&self.draft),
        };
        self.history_index = None;
        self.history_stash = None;
        self.editing = Some(QueuedEdit {
            queued_message_id,
            expected_updated_at,
            stash,
            renew_at: None,
            saving: false,
        });
        self.replace(draft);
    }

    pub fn end_queued_edit(&mut self) -> Option<QueuedEdit> {
        let edit = self.editing.take()?;
        let stash = edit.stash.draft();
        self.replace(stash);
        Some(edit)
    }

    fn history_step(&mut self, older: bool) -> bool {
        if self.history.is_empty() {
            return false;
        }
        let next = match (self.history_index, older) {
            (None, true) => Some(0),
            (None, false) => return false,
            (Some(index), true) if index + 1 < self.history.len() => Some(index + 1),
            (Some(_), true) => return true,
            (Some(0), false) => None,
            (Some(index), false) => Some(index - 1),
        };
        if self.history_index.is_none() {
            self.history_stash = Some(self.draft.clone());
        }
        let draft = match next {
            Some(index) => self.history[index].clone(),
            None => self.history_stash.take().unwrap_or_default(),
        };
        self.replace(draft);
        self.history_index = next;
        true
    }

    pub fn arm_escape(&mut self, now: Instant) -> bool {
        match self.esc_armed_at {
            Some(at) if now.duration_since(at) <= ESC_STOP_WINDOW => {
                self.esc_armed_at = None;
                true
            }
            _ => {
                self.esc_armed_at = Some(now);
                false
            }
        }
    }

    pub fn escape_armed(&self, now: Instant) -> bool {
        self.esc_armed_at
            .is_some_and(|at| now.duration_since(at) <= ESC_STOP_WINDOW)
    }

    pub fn handle_key(
        &mut self,
        key: KeyEvent,
        submit_key: SubmitKey,
        now: Instant,
    ) -> ComposerKey {
        let ctrl = has(&key, KeyModifiers::CONTROL);
        let alt = has(&key, KeyModifiers::ALT);
        let shift = has(&key, KeyModifiers::SHIFT);
        if key.code != KeyCode::Esc {
            self.esc_armed_at = None;
        }
        match key.code {
            KeyCode::Enter => {
                let plain = !ctrl && !alt && !shift;
                let action = match submit_key {
                    SubmitKey::Enter if plain => ComposerKey::Submit,
                    SubmitKey::Enter if ctrl => ComposerKey::Steer,
                    SubmitKey::Enter => {
                        self.textarea.insert_newline();
                        ComposerKey::Edited
                    }
                    SubmitKey::ModifierEnter if alt || ctrl => ComposerKey::Submit,
                    SubmitKey::ModifierEnter => {
                        self.textarea.insert_newline();
                        ComposerKey::Edited
                    }
                };
                if action == ComposerKey::Edited {
                    self.sync(now);
                }
                action
            }
            KeyCode::Char('j') if ctrl && !alt => {
                self.textarea.insert_newline();
                self.sync(now);
                ComposerKey::Edited
            }
            KeyCode::Char('s') if ctrl && !alt => ComposerKey::Steer,
            KeyCode::Char('c') if ctrl && !alt => ComposerKey::Interrupt,
            KeyCode::Esc => ComposerKey::Escape,
            KeyCode::Up if !ctrl && !alt && !shift => {
                let at_top = self.textarea.cursor().0 == 0;
                if at_top
                    && (self.draft.is_empty() || self.history_index.is_some())
                    && self.history_step(true)
                {
                    return ComposerKey::Handled;
                }
                self.textarea.input(key);
                ComposerKey::Handled
            }
            KeyCode::Down if !ctrl && !alt && !shift => {
                let at_bottom = self.textarea.cursor().0 + 1 >= self.textarea.lines().len();
                if at_bottom && self.history_index.is_some() && self.history_step(false) {
                    return ComposerKey::Handled;
                }
                self.textarea.input(key);
                ComposerKey::Handled
            }
            KeyCode::Tab | KeyCode::BackTab => ComposerKey::Unhandled,
            KeyCode::Char(_) if alt && !ctrl => ComposerKey::Unhandled,
            KeyCode::F(_) => ComposerKey::Unhandled,
            _ => {
                let modified = self.textarea.input(key);
                if modified {
                    self.sync(now);
                    ComposerKey::Edited
                } else {
                    ComposerKey::Handled
                }
            }
        }
    }

    pub fn clear(&mut self, now: Instant) {
        self.set_draft(PromptDraft::default(), now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::from(code)
    }

    fn type_text(composer: &mut Composer, text: &str, now: Instant) {
        for ch in text.chars() {
            composer.handle_key(key(KeyCode::Char(ch)), SubmitKey::Enter, now);
        }
    }

    #[test]
    fn enter_submits_and_modifiers_insert_newlines() {
        let now = Instant::now();
        let mut composer = Composer::new("thr");
        type_text(&mut composer, "ab", now);
        assert_eq!(
            composer.handle_key(
                KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT),
                SubmitKey::Enter,
                now
            ),
            ComposerKey::Edited
        );
        composer.handle_key(
            KeyEvent::new(KeyCode::Char('j'), KeyModifiers::CONTROL),
            SubmitKey::Enter,
            now,
        );
        type_text(&mut composer, "c", now);
        assert_eq!(composer.text(), "ab\n\nc");
        assert_eq!(
            composer.handle_key(key(KeyCode::Enter), SubmitKey::Enter, now),
            ComposerKey::Submit
        );
        assert_eq!(
            composer.handle_key(
                KeyEvent::new(KeyCode::Enter, KeyModifiers::CONTROL),
                SubmitKey::Enter,
                now
            ),
            ComposerKey::Steer
        );
        assert_eq!(
            composer.handle_key(key(KeyCode::Enter), SubmitKey::ModifierEnter, now),
            ComposerKey::Edited
        );
        assert_eq!(
            composer.handle_key(
                KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT),
                SubmitKey::ModifierEnter,
                now
            ),
            ComposerKey::Submit
        );
        let submitted = composer.take_submission(now);
        assert_eq!(submitted.text, "ab\n\nc\n");
        assert!(composer.is_empty());
    }

    #[test]
    fn history_navigation_restores_the_unsent_draft() {
        let now = Instant::now();
        let mut composer = Composer::new("thr");
        composer.history = vec![
            PromptDraft::from_text("newest"),
            PromptDraft::from_text("older"),
        ];
        composer.handle_key(key(KeyCode::Up), SubmitKey::Enter, now);
        assert_eq!(composer.text(), "newest");
        composer.handle_key(key(KeyCode::Up), SubmitKey::Enter, now);
        assert_eq!(composer.text(), "older");
        assert_eq!(composer.history_position(), Some((2, 2)));
        composer.handle_key(key(KeyCode::Up), SubmitKey::Enter, now);
        assert_eq!(composer.text(), "older");
        composer.handle_key(key(KeyCode::Down), SubmitKey::Enter, now);
        composer.handle_key(key(KeyCode::Down), SubmitKey::Enter, now);
        assert_eq!(composer.text(), "");
        assert!(!composer.is_browsing_history());

        type_text(&mut composer, "typing", now);
        composer.handle_key(key(KeyCode::Up), SubmitKey::Enter, now);
        assert_eq!(composer.text(), "typing");
    }

    #[test]
    fn editing_a_history_entry_detaches_it() {
        let now = Instant::now();
        let mut composer = Composer::new("thr");
        composer.history = vec![PromptDraft::from_text("old")];
        composer.handle_key(key(KeyCode::Up), SubmitKey::Enter, now);
        type_text(&mut composer, "!", now);
        assert!(!composer.is_browsing_history());
        assert_eq!(composer.text(), "old!");
        composer.handle_key(key(KeyCode::Down), SubmitKey::Enter, now);
        assert_eq!(composer.text(), "old!");
    }

    #[test]
    fn queued_edit_stashes_and_restores_the_draft() {
        let now = Instant::now();
        let mut composer = Composer::new("thr");
        type_text(&mut composer, "mine", now);
        composer.dirty_since = None;
        composer.begin_queued_edit("q1".into(), 5, PromptDraft::from_text("queued text"));
        assert_eq!(composer.text(), "queued text");
        type_text(&mut composer, "!", now);
        assert_eq!(composer.dirty_since, None);
        assert_eq!(composer.persisted_draft().text, "mine");
        let edit = composer.end_queued_edit().unwrap();
        assert_eq!(edit.queued_message_id, "q1");
        assert_eq!(composer.text(), "mine");
    }

    #[test]
    fn draft_saves_are_debounced_and_loading_never_clobbers_typing() {
        let start = Instant::now();
        let mut composer = Composer::new("thr");
        type_text(&mut composer, "x", start);
        assert_eq!(composer.take_save(start, false), None);
        let later = start + DRAFT_SAVE_DELAY;
        assert_eq!(
            composer.take_save(later, false),
            Some(Some(r#"{"attachments":[],"text":"x"}"#.to_owned()))
        );
        assert_eq!(composer.take_save(later, true), None);
        composer.load_stored(Some(r#"{"text":"stored"}"#));
        assert_eq!(composer.text(), "x");
        let mut fresh = Composer::new("thr2");
        fresh.load_stored(Some(r#"{"text":"line1\nline2"}"#));
        assert_eq!(fresh.line_count(), 2);
        assert_eq!(fresh.draft().text, "line1\nline2");
    }

    #[test]
    fn double_escape_within_window() {
        let start = Instant::now();
        let mut composer = Composer::new("thr");
        assert!(!composer.arm_escape(start));
        assert!(composer.escape_armed(start));
        assert!(composer.arm_escape(start + Duration::from_millis(300)));
        assert!(!composer.arm_escape(start + Duration::from_secs(1)));
        assert!(!composer.arm_escape(start + Duration::from_secs(4)));
    }

    #[test]
    fn submit_key_parsing() {
        assert_eq!(SubmitKey::parse("Enter"), Some(SubmitKey::Enter));
        assert_eq!(
            SubmitKey::parse("ctrl-enter"),
            Some(SubmitKey::ModifierEnter)
        );
        assert_eq!(SubmitKey::parse("space"), None);
    }
}
