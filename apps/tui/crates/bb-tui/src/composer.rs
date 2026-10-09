use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::{Color, Modifier, Style};
use ratatui_textarea::{CursorMove, TextArea, WrapMode};

use crate::chips::{
    self, CharRange, char_len, char_to_row_col, mention_char_ranges, row_col_to_char,
};
use crate::mentions::{ActiveTrigger, TypeaheadTrigger, find_active_trigger};
use crate::prompt::{DraftAttachment, PromptDraft};
use crate::vi;
use bb_wire::prompt::{PromptMentionResource, PromptTextMention};

pub const DRAFT_SAVE_DELAY: Duration = Duration::from_millis(400);
const MAX_UNDO: usize = 200;
pub const EDIT_HOLD_RENEWALS_PER_LEASE: u32 = 4;
pub const EDIT_HOLD_RETRY: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SubmitKey {
    Enter,
    #[default]
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
            Self::Enter => "RET",
            Self::ModifierEnter => "C-RET",
        }
    }

    pub fn newline_label(self) -> &'static str {
        match self {
            Self::Enter => "S-RET/C-j",
            Self::ModifierEnter => "RET",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComposerKey {
    Submit,
    Escape,
    Edited,
    Handled,
    Unhandled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EditMode {
    #[default]
    Normal,
    Insert,
    Visual {
        line: bool,
        anchor: usize,
        head: usize,
    },
}

#[derive(Debug, Clone, PartialEq)]
struct Snapshot {
    draft: PromptDraft,
    cursor: usize,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DismissedTrigger {
    pub start: usize,
    pub end: usize,
    pub has_left: bool,
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
    pub pending_restore: Option<PromptDraft>,
    pub dismissed_trigger: Option<DismissedTrigger>,
    pub uploads: Vec<String>,
    pub mode: EditMode,
    pub last_insert: Option<usize>,
    pub one_shot: bool,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
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

fn valid_mentions(text: &str, mentions: &[PromptTextMention]) -> Vec<PromptTextMention> {
    let mut ranged: Vec<(CharRange, &PromptTextMention)> = mentions
        .iter()
        .filter_map(|mention| {
            chips::mention_char_range(text, mention).map(|range| (range, mention))
        })
        .collect();
    ranged.sort_by_key(|(range, _)| (range.start, range.end));
    let mut kept: Vec<PromptTextMention> = Vec::new();
    let mut last_end = 0;
    for (range, mention) in ranged {
        if range.start < last_end {
            continue;
        }
        last_end = range.end;
        kept.push(mention.clone());
    }
    kept
}

fn is_backward_motion(key: &KeyEvent) -> bool {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    matches!(
        key.code,
        KeyCode::Left | KeyCode::Home | KeyCode::Up | KeyCode::PageUp
    ) || (ctrl && matches!(key.code, KeyCode::Char('b' | 'a' | 'p')))
        || (alt && matches!(key.code, KeyCode::Char('b' | '<')))
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
            pending_restore: None,
            dismissed_trigger: None,
            uploads: Vec::new(),
            mode: EditMode::Normal,
            last_insert: None,
            one_shot: false,
            undo: Vec::new(),
            redo: Vec::new(),
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

    fn replace(&mut self, mut draft: PromptDraft) {
        let placeholder = self.textarea.placeholder_text().to_owned();
        self.textarea = textarea_with(&draft.text);
        self.textarea.set_placeholder_text(placeholder);
        draft.mentions = valid_mentions(&draft.text, &draft.mentions);
        self.draft = draft;
        self.dismissed_trigger = None;
    }

    fn reset_text(&mut self, text: &str, cursor: usize) {
        let placeholder = self.textarea.placeholder_text().to_owned();
        self.textarea = textarea_with(text);
        self.textarea.set_placeholder_text(placeholder);
        self.set_cursor_char(cursor);
    }

    pub fn cursor_char(&self) -> usize {
        let ratatui_textarea::DataCursor(row, col) = self.textarea.cursor();
        row_col_to_char(self.textarea.lines(), row, col)
    }

    pub fn set_cursor_char(&mut self, offset: usize) {
        let (row, col) = char_to_row_col(&self.text(), offset);
        self.textarea.move_cursor(CursorMove::Jump(
            u16::try_from(row).unwrap_or(u16::MAX),
            u16::try_from(col).unwrap_or(u16::MAX),
        ));
    }

    pub fn mention_ranges(&self) -> Vec<CharRange> {
        mention_char_ranges(&self.draft.text, &self.draft.mentions)
    }

    pub fn scan_text(&self) -> Vec<char> {
        let mut chars: Vec<char> = self.draft.text.chars().collect();
        for range in self.mention_ranges() {
            for ch in chars.iter_mut().take(range.end).skip(range.start) {
                *ch = '\n';
            }
        }
        chars
    }

    pub fn sync_trigger(&mut self, triggers: &[TypeaheadTrigger]) -> Option<ActiveTrigger> {
        let caret = self.cursor_char();
        let detected = if self.textarea.is_selecting() {
            None
        } else {
            find_active_trigger(&self.scan_text(), caret, triggers)
        };
        if let Some(mut dismissed) = self.dismissed_trigger {
            if !dismissed.has_left
                && detected.as_ref().map(|trigger| trigger.from) == Some(dismissed.start)
            {
                dismissed.end = dismissed.end.max(caret);
            }
            let within = caret >= dismissed.start && caret <= dismissed.end;
            if !within {
                dismissed.has_left = true;
                self.dismissed_trigger = Some(dismissed);
            } else if dismissed.has_left {
                self.dismissed_trigger = None;
            } else {
                self.dismissed_trigger = Some(dismissed);
            }
        }
        let suppressed = self.dismissed_trigger.is_some_and(|dismissed| {
            !dismissed.has_left && caret >= dismissed.start && caret <= dismissed.end
        });
        if suppressed { None } else { detected }
    }

    pub fn dismiss_trigger(&mut self, trigger: &ActiveTrigger) {
        self.dismissed_trigger = Some(DismissedTrigger {
            start: trigger.from,
            end: trigger.to,
            has_left: false,
        });
    }

    pub fn apply_mention(
        &mut self,
        trigger: &ActiveTrigger,
        serialized: &str,
        resource: PromptMentionResource,
        dismissed_end: impl FnOnce(&str) -> usize,
        now: Instant,
    ) {
        let trailing = chips::trailing_text_after(&self.draft.text, trigger.to);
        let inserted = chips::insert_mention(
            &self.draft.text,
            &self.draft.mentions,
            CharRange {
                start: trigger.from,
                end: trigger.to,
            },
            serialized,
            resource,
            trailing,
        );
        self.reset_text(&inserted.text, inserted.cursor);
        self.draft = PromptDraft {
            text: inserted.text,
            mentions: inserted.mentions,
            attachments: self.draft.attachments.clone(),
        };
        self.dismissed_trigger = Some(DismissedTrigger {
            start: trigger.from,
            end: dismissed_end(trailing),
            has_left: false,
        });
        self.history_index = None;
        self.history_stash = None;
        self.mark_dirty(now);
    }

    pub fn add_attachment(&mut self, attachment: DraftAttachment, now: Instant) -> bool {
        if self
            .draft
            .attachments
            .iter()
            .any(|existing| existing.path == attachment.path)
        {
            return false;
        }
        self.draft.attachments.push(attachment);
        self.mark_dirty(now);
        true
    }

    pub fn remove_attachment(&mut self, index: usize, now: Instant) -> Option<DraftAttachment> {
        if index >= self.draft.attachments.len() {
            return None;
        }
        let removed = self.draft.attachments.remove(index);
        self.mark_dirty(now);
        Some(removed)
    }

    fn snap_cursor(&mut self, backward: bool) {
        let cursor = self.cursor_char();
        let snapped = chips::snap_cursor(&self.draft.text, &self.draft.mentions, cursor, backward);
        if snapped != cursor {
            self.set_cursor_char(snapped);
        }
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
        if text == self.draft.text {
            return;
        }
        let cursor = self.cursor_char();
        let reconciled =
            chips::reconcile(&self.draft.text, &text, &self.draft.mentions, Some(cursor));
        if reconciled.removed_leftovers {
            let cursor = reconciled.cursor.unwrap_or(char_len(&reconciled.text));
            self.reset_text(&reconciled.text, cursor);
        }
        self.draft = PromptDraft {
            text: reconciled.text,
            mentions: reconciled.mentions,
            attachments: std::mem::take(&mut self.draft.attachments),
        };
        self.history_index = None;
        self.history_stash = None;
        self.mark_dirty(now);
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

    pub fn history_step(&mut self, older: bool) -> bool {
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

    pub fn handle_key(
        &mut self,
        key: KeyEvent,
        submit_key: SubmitKey,
        now: Instant,
    ) -> ComposerKey {
        let ctrl = has(&key, KeyModifiers::CONTROL);
        let alt = has(&key, KeyModifiers::ALT);
        let shift = has(&key, KeyModifiers::SHIFT);
        match key.code {
            KeyCode::Enter => {
                let plain = !ctrl && !alt && !shift;
                if ctrl || alt || (plain && submit_key == SubmitKey::Enter) {
                    return ComposerKey::Submit;
                }
                self.textarea.insert_newline();
                self.sync(now);
                ComposerKey::Edited
            }
            KeyCode::Char('j') if ctrl && !alt => {
                self.textarea.insert_newline();
                self.sync(now);
                ComposerKey::Edited
            }
            KeyCode::Char('u') if ctrl && !alt => {
                let head = self.textarea.lines()[self.textarea.cursor().0].clone();
                let column = self.textarea.cursor().1;
                let indent = head.chars().take_while(|ch| ch.is_whitespace()).count();
                let target = if column > indent { indent } else { 0 };
                for _ in target..column {
                    self.textarea.delete_char();
                }
                self.sync(now);
                ComposerKey::Edited
            }
            KeyCode::Char('n') if ctrl && !alt => {
                self.history_step(false);
                ComposerKey::Handled
            }
            KeyCode::Char('p') if ctrl && !alt => {
                self.history_step(true);
                ComposerKey::Handled
            }
            KeyCode::Char('o' | 'r' | 'v' | 's' | 'c') if ctrl && !alt => ComposerKey::Unhandled,
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
                self.snap_cursor(true);
                ComposerKey::Handled
            }
            KeyCode::Down if !ctrl && !alt && !shift => {
                let at_bottom = self.textarea.cursor().0 + 1 >= self.textarea.lines().len();
                if at_bottom && self.history_index.is_some() && self.history_step(false) {
                    return ComposerKey::Handled;
                }
                self.textarea.input(key);
                self.snap_cursor(false);
                ComposerKey::Handled
            }
            KeyCode::Tab | KeyCode::BackTab => ComposerKey::Unhandled,
            KeyCode::Char(_) if alt && !ctrl => ComposerKey::Unhandled,
            KeyCode::F(_) => ComposerKey::Unhandled,
            _ => {
                let before = self.cursor_char();
                let modified = self.textarea.input(key);
                if modified {
                    self.sync(now);
                    ComposerKey::Edited
                } else {
                    let after = self.cursor_char();
                    self.snap_cursor(
                        after < before || (after == before && is_backward_motion(&key)),
                    );
                    ComposerKey::Handled
                }
            }
        }
    }

    pub fn clear(&mut self, now: Instant) {
        self.set_draft(PromptDraft::default(), now);
    }

    pub fn chars(&self) -> Vec<char> {
        self.text().chars().collect()
    }

    pub fn is_insert(&self) -> bool {
        self.mode == EditMode::Insert
    }

    pub fn checkpoint(&mut self) {
        self.undo.push(Snapshot {
            draft: self.draft.clone(),
            cursor: self.cursor_char(),
        });
        if self.undo.len() > MAX_UNDO {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    fn restore(&mut self, snapshot: Snapshot, now: Instant) {
        let placeholder = self.textarea.placeholder_text().to_owned();
        self.textarea = textarea_with(&snapshot.draft.text);
        self.textarea.set_placeholder_text(placeholder);
        self.draft = snapshot.draft;
        self.dismissed_trigger = None;
        self.history_index = None;
        self.history_stash = None;
        let cursor = vi::clamp_normal(&self.chars(), snapshot.cursor);
        self.set_cursor_char(cursor);
        self.mark_dirty(now);
    }

    pub fn undo(&mut self, now: Instant) -> bool {
        let Some(snapshot) = self.undo.pop() else {
            return false;
        };
        self.redo.push(Snapshot {
            draft: self.draft.clone(),
            cursor: self.cursor_char(),
        });
        self.restore(snapshot, now);
        true
    }

    pub fn redo(&mut self, now: Instant) -> bool {
        let Some(snapshot) = self.redo.pop() else {
            return false;
        };
        self.undo.push(Snapshot {
            draft: self.draft.clone(),
            cursor: self.cursor_char(),
        });
        self.restore(snapshot, now);
        true
    }

    pub fn replace_range(
        &mut self,
        start: usize,
        end: usize,
        text: &str,
        cursor: usize,
        now: Instant,
    ) {
        let chars = self.chars();
        let start = start.min(chars.len());
        let end = end.clamp(start, chars.len());
        let mut next: String = chars[..start].iter().collect();
        next.push_str(text);
        next.extend(chars[end..].iter());
        self.reset_text(&next, cursor);
        self.sync(now);
    }

    pub fn enter_insert(&mut self) {
        if self.mode != EditMode::Insert {
            self.checkpoint();
        }
        self.textarea.cancel_selection();
        self.mode = EditMode::Insert;
    }

    pub fn enter_normal(&mut self) {
        self.textarea.cancel_selection();
        let mut cursor = self.cursor_char();
        if let EditMode::Visual { head, .. } = self.mode {
            cursor = head;
        }
        if self.mode == EditMode::Insert {
            self.last_insert = Some(cursor);
            let chars = self.chars();
            if cursor > vi::line_start(&chars, cursor) {
                cursor -= 1;
            }
        }
        self.mode = EditMode::Normal;
        self.one_shot = false;
        let cursor = vi::clamp_normal(&self.chars(), cursor);
        self.set_cursor_char(cursor);
    }

    pub fn move_to(&mut self, cursor: usize, backward: bool) {
        let chars = self.chars();
        let cursor = match self.mode {
            EditMode::Insert => cursor.min(chars.len()),
            _ => vi::clamp_normal(&chars, cursor),
        };
        self.set_cursor_char(cursor);
        self.snap_cursor(backward);
        if let EditMode::Visual { line, anchor, .. } = self.mode {
            let head = self.cursor_char();
            self.mode = EditMode::Visual { line, anchor, head };
            self.refresh_selection();
        }
    }

    pub fn start_visual(&mut self, line: bool) {
        let cursor = self.cursor_char();
        self.mode = EditMode::Visual {
            line,
            anchor: cursor,
            head: cursor,
        };
        self.refresh_selection();
    }

    pub fn set_visual(&mut self, line: bool, anchor: usize, head: usize) {
        self.mode = EditMode::Visual { line, anchor, head };
        self.refresh_selection();
    }

    pub fn visual_range(&self) -> Option<vi::Range> {
        let EditMode::Visual { line, anchor, head } = self.mode else {
            return None;
        };
        let chars = self.chars();
        if line {
            return Some(vi::lines_between(&chars, anchor, head));
        }
        let start = anchor.min(head);
        let end = (anchor.max(head) + 1).min(chars.len());
        Some(vi::Range {
            start,
            end: end.max(start),
            linewise: false,
        })
    }

    pub fn refresh_selection(&mut self) {
        let EditMode::Visual { line, anchor, head } = self.mode else {
            return;
        };
        let chars = self.chars();
        let (start, end) = if line {
            let low = anchor.min(head);
            let high = anchor.max(head);
            (vi::line_start(&chars, low), vi::line_end(&chars, high))
        } else {
            (anchor.min(head), (anchor.max(head) + 1).min(chars.len()))
        };
        self.textarea.cancel_selection();
        self.set_cursor_char(start);
        self.textarea.start_selection();
        self.set_cursor_char(end);
    }

    pub fn cursor(&self) -> usize {
        match self.mode {
            EditMode::Visual { head, .. } => head,
            _ => self.cursor_char(),
        }
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
            ComposerKey::Submit
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
    fn ctrl_u_deletes_to_the_indentation_then_the_line_start() {
        let now = Instant::now();
        let mut composer = Composer::new("thr");
        type_text(&mut composer, "  foo bar", now);
        let ctrl_u = KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL);
        composer.handle_key(ctrl_u, SubmitKey::Enter, now);
        assert_eq!(composer.text(), "  ");
        composer.handle_key(ctrl_u, SubmitKey::Enter, now);
        assert_eq!(composer.text(), "");
    }

    #[test]
    fn undo_restores_snapshots_and_redo_reapplies() {
        let now = Instant::now();
        let mut composer = Composer::new("thr");
        composer.enter_insert();
        type_text(&mut composer, "hello world", now);
        composer.enter_normal();
        assert_eq!(composer.cursor_char(), 10);
        composer.checkpoint();
        composer.replace_range(5, 11, "", 4, now);
        assert_eq!(composer.text(), "hello");
        assert!(composer.undo(now));
        assert_eq!(composer.text(), "hello world");
        assert!(composer.undo(now));
        assert_eq!(composer.text(), "");
        assert!(!composer.undo(now));
        assert!(composer.redo(now));
        assert!(composer.redo(now));
        assert_eq!(composer.text(), "hello");
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
            Some(Some(r#"{"text":"x","attachments":[]}"#.to_owned()))
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
    fn submit_key_parsing() {
        assert_eq!(SubmitKey::parse("Enter"), Some(SubmitKey::Enter));
        assert_eq!(
            SubmitKey::parse("ctrl-enter"),
            Some(SubmitKey::ModifierEnter)
        );
        assert_eq!(SubmitKey::parse("space"), None);
    }
}
