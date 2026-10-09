use std::collections::HashMap;
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
    Replace,
    Visual {
        line: bool,
        anchor: usize,
        head: usize,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub op: vi::EditOp,
    pub count: Option<usize>,
    pub register: Option<char>,
    pub inserted: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LineUndo {
    line: usize,
    lines: usize,
    text: String,
}

#[derive(Debug, Clone, PartialEq)]
struct Snapshot {
    draft: PromptDraft,
    cursor: usize,
    marks: HashMap<char, usize>,
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
    pub marks: HashMap<char, usize>,
    pub last_visual_line: Option<bool>,
    pub last_change: Option<Change>,
    pub pending_change: Option<Change>,
    pub finished_insert: Option<String>,
    pub last_inserted: Option<String>,
    pub literal: bool,
    insert_base: Option<(Vec<char>, usize)>,
    replaced: Vec<Option<char>>,
    line_undo: Option<LineUndo>,
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
            marks: HashMap::new(),
            last_visual_line: None,
            last_change: None,
            pending_change: None,
            finished_insert: None,
            last_inserted: None,
            literal: false,
            insert_base: None,
            replaced: Vec::new(),
            line_undo: None,
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
        if self.literal {
            self.literal = false;
            let literal = match key.code {
                KeyCode::Char(ch) => Some(ch),
                KeyCode::Tab => Some('\t'),
                KeyCode::Enter => Some('\n'),
                _ => None,
            };
            if let Some(ch) = literal {
                self.textarea.insert_char(ch);
                self.sync(now);
                return ComposerKey::Edited;
            }
            return ComposerKey::Handled;
        }
        match key.code {
            KeyCode::Char('v') if ctrl && !alt => {
                self.literal = true;
                ComposerKey::Handled
            }
            KeyCode::Char('a') if ctrl && !alt => {
                let chars = self.chars();
                let target = vi::first_non_blank(&chars, self.cursor_char());
                self.set_cursor_char(target);
                ComposerKey::Handled
            }
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
            KeyCode::Char('o' | 'r' | 's' | 'c') if ctrl && !alt => ComposerKey::Unhandled,
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

    pub fn motion_chars(&self) -> Vec<char> {
        let mut chars = self.chars();
        for range in self.mention_ranges() {
            for ch in chars.iter_mut().take(range.end).skip(range.start) {
                *ch = vi::CHIP;
            }
        }
        chars
    }

    pub fn expand_chips(&self, range: vi::Range) -> vi::Range {
        let mut expanded = range;
        for chip in self.mention_ranges() {
            if chip.start < expanded.end && chip.end > expanded.start {
                expanded.start = expanded.start.min(chip.start);
                expanded.end = expanded.end.max(chip.end);
            }
        }
        expanded
    }

    pub fn is_insert(&self) -> bool {
        self.mode == EditMode::Insert
    }

    pub fn checkpoint(&mut self) {
        let cursor = self.cursor_char();
        self.note_line(cursor);
        self.undo.push(Snapshot {
            draft: self.draft.clone(),
            cursor,
            marks: self.marks.clone(),
        });
        if self.undo.len() > MAX_UNDO {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    fn note_line(&mut self, cursor: usize) {
        let chars = self.chars();
        let line = vi::line_index(&chars, cursor);
        let lines = self.line_count();
        if self
            .line_undo
            .as_ref()
            .is_some_and(|undo| undo.line == line && undo.lines == lines)
        {
            return;
        }
        let text = self.textarea.lines().get(line).cloned().unwrap_or_default();
        self.line_undo = Some(LineUndo { line, lines, text });
    }

    pub fn undo_line(&mut self, now: Instant) -> bool {
        let Some(undo) = self.line_undo.clone() else {
            return false;
        };
        if undo.lines != self.line_count() {
            return false;
        }
        let Some(current) = self.textarea.lines().get(undo.line).cloned() else {
            return false;
        };
        if current == undo.text {
            return false;
        }
        self.checkpoint();
        let chars = self.chars();
        let start = vi::line_start(&chars, row_col_to_char(self.textarea.lines(), undo.line, 0));
        let end = vi::line_end(&chars, start);
        self.replace_range(start, end, &undo.text, start, now);
        self.line_undo = Some(LineUndo {
            text: current,
            ..undo
        });
        self.move_to(start, true);
        true
    }

    fn restore(&mut self, snapshot: Snapshot, now: Instant) {
        let placeholder = self.textarea.placeholder_text().to_owned();
        self.textarea = textarea_with(&snapshot.draft.text);
        self.textarea.set_placeholder_text(placeholder);
        self.draft = snapshot.draft;
        self.marks = snapshot.marks;
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
            marks: self.marks.clone(),
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
            marks: self.marks.clone(),
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
        let inserted = text.chars().count();
        self.shift_marks(start, end, inserted);
        self.marks.insert('[', start);
        self.marks.insert(']', start + inserted.saturating_sub(1));
        self.marks.insert('.', start);
        self.reset_text(&next, cursor);
        self.sync(now);
    }

    fn shift_marks(&mut self, start: usize, end: usize, inserted: usize) {
        for mark in self.marks.values_mut() {
            if *mark >= end {
                *mark = *mark + inserted - (end - start);
            } else if *mark > start {
                *mark = start;
            }
        }
    }

    pub fn mark(&self, name: char) -> Option<usize> {
        let position = match name {
            '^' => self.last_insert,
            '`' => self.marks.get(&'\'').copied(),
            other => self.marks.get(&other).copied(),
        }?;
        Some(position.min(char_len(&self.draft.text)))
    }

    pub fn set_mark(&mut self, name: char, position: usize) -> bool {
        let name = if name == '`' { '\'' } else { name };
        if !(name.is_ascii_lowercase() || "'[]<>".contains(name)) {
            return false;
        }
        self.marks.insert(name, position);
        true
    }

    pub fn capture_insert_base(&mut self) {
        self.insert_base = Some((self.chars(), self.cursor_char()));
        self.replaced.clear();
    }

    pub fn enter_insert(&mut self) {
        if !matches!(self.mode, EditMode::Insert | EditMode::Replace) {
            self.checkpoint();
        }
        self.textarea.cancel_selection();
        self.mode = EditMode::Insert;
        self.capture_insert_base();
    }

    pub fn enter_replace(&mut self) {
        if !matches!(self.mode, EditMode::Insert | EditMode::Replace) {
            self.checkpoint();
        }
        self.textarea.cancel_selection();
        self.mode = EditMode::Replace;
        self.capture_insert_base();
    }

    fn inserted_text(&self) -> Option<String> {
        let (base, base_cursor) = self.insert_base.as_ref()?;
        let current = self.chars();
        let limit = base.len().min(current.len());
        let prefix = (0..limit)
            .take_while(|index| base[*index] == current[*index])
            .count()
            .min(*base_cursor);
        let suffix = (0..limit - prefix)
            .take_while(|index| base[base.len() - 1 - index] == current[current.len() - 1 - index])
            .count();
        let text: String = current[prefix..current.len() - suffix].iter().collect();
        Some(text)
    }

    pub fn enter_normal(&mut self) {
        self.textarea.cancel_selection();
        let mut cursor = self.cursor_char();
        match self.mode {
            EditMode::Visual { line, anchor, head } => {
                cursor = head;
                self.marks.insert('<', anchor.min(head));
                self.marks.insert('>', anchor.max(head));
                self.last_visual_line = Some(line);
            }
            EditMode::Insert | EditMode::Replace => {
                self.last_insert = Some(cursor);
                let inserted = self.inserted_text().unwrap_or_default();
                if let Some(mut change) = self.pending_change.take() {
                    change.inserted = Some(inserted.clone());
                    self.last_change = Some(change);
                }
                if !inserted.is_empty() {
                    self.finished_insert = Some(inserted.clone());
                    self.last_inserted = Some(inserted);
                }
                self.insert_base = None;
                self.replaced.clear();
                let chars = self.chars();
                if cursor > vi::line_start(&chars, cursor) {
                    cursor = vi::grapheme_prev(&chars, cursor, 1);
                }
            }
            EditMode::Normal => {}
        }
        self.mode = EditMode::Normal;
        self.one_shot = false;
        self.literal = false;
        let chars = self.chars();
        let cursor = self.normal_position(&chars, vi::clamp_normal(&chars, cursor), true);
        self.set_cursor_char(cursor);
    }

    fn normal_position(&self, chars: &[char], cursor: usize, backward: bool) -> usize {
        let Some(chip) = self
            .mention_ranges()
            .into_iter()
            .find(|range| range.contains_strictly(cursor))
        else {
            return cursor;
        };
        if backward || chip.end >= vi::line_end(chars, chip.start) {
            chip.start
        } else {
            chip.end
        }
    }

    pub fn move_to(&mut self, cursor: usize, backward: bool) {
        let chars = self.chars();
        let cursor = match self.mode {
            EditMode::Insert | EditMode::Replace => {
                let cursor = cursor.min(chars.len());
                chips::snap_cursor(&self.draft.text, &self.draft.mentions, cursor, backward)
            }
            _ => self.normal_position(&chars, vi::clamp_normal(&chars, cursor), backward),
        };
        self.set_cursor_char(cursor);
        if let EditMode::Visual { line, anchor, .. } = self.mode {
            self.mode = EditMode::Visual {
                line,
                anchor,
                head: cursor,
            };
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

    pub fn reselect(&mut self) -> bool {
        let (Some(line), Some(start), Some(end)) = (
            self.last_visual_line,
            self.marks.get(&'<').copied(),
            self.marks.get(&'>').copied(),
        ) else {
            return false;
        };
        let chars = self.chars();
        let start = vi::clamp_normal(&chars, start);
        let end = vi::clamp_normal(&chars, end);
        self.set_visual(line, start, end);
        true
    }

    fn selection_bounds(&self, line: bool, anchor: usize, head: usize) -> vi::Range {
        let chars = self.chars();
        if line {
            return vi::lines_between(&chars, anchor, head);
        }
        let start = anchor.min(head);
        let last = anchor.max(head);
        let end = vi::grapheme_next(&chars, last, 1)
            .max(last + 1)
            .min(chars.len());
        self.expand_chips(vi::Range {
            start,
            end: end.max(start),
            linewise: false,
        })
    }

    pub fn visual_range(&self) -> Option<vi::Range> {
        let EditMode::Visual { line, anchor, head } = self.mode else {
            return None;
        };
        Some(self.selection_bounds(line, anchor, head))
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
            let range = self.selection_bounds(false, anchor, head);
            (range.start, range.end)
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

    pub fn paste(&mut self, text: &str, now: Instant) {
        let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
        if normalized.is_empty() {
            return;
        }
        match self.mode {
            EditMode::Insert | EditMode::Replace => self.insert_text(&normalized, now),
            EditMode::Normal | EditMode::Visual { .. } => {
                let range = self.visual_range();
                if range.is_some() {
                    self.enter_normal();
                }
                self.checkpoint();
                let at = self.cursor_char();
                let (start, end) = range.map_or((at, at), |range| (range.start, range.end));
                let cursor = start + normalized.chars().count() - 1;
                self.replace_range(start, end, &normalized, cursor, now);
                self.move_to(cursor, false);
            }
        }
    }

    pub fn screen_offset(&self, row: u16, column: u16) -> usize {
        let (scroll_row, scroll_col) = self.textarea.scroll_offset();
        let ratatui_textarea::DataCursor(line, col) = self.textarea.screen_to_data(
            usize::from(row) + usize::from(scroll_row),
            usize::from(column) + usize::from(scroll_col),
        );
        let lines = self.textarea.lines();
        let line = line.min(lines.len().saturating_sub(1));
        let col = col.min(lines.get(line).map_or(0, |text| text.chars().count()));
        row_col_to_char(lines, line, col)
    }

    pub fn click_at(&mut self, offset: usize) {
        let cursor = chips::snap_cursor(&self.draft.text, &self.draft.mentions, offset, false);
        self.set_cursor_char(cursor);
        self.capture_insert_base();
    }

    pub fn drag_to(&mut self, offset: usize) {
        let anchor = match self.mode {
            EditMode::Visual { anchor, .. } => anchor,
            _ => {
                let anchor = self.cursor_char();
                self.enter_normal();
                vi::clamp_normal(&self.chars(), anchor)
            }
        };
        let chars = self.chars();
        let head = self.normal_position(&chars, vi::clamp_normal(&chars, offset), false);
        self.set_visual(false, anchor, head);
    }

    pub fn replace_key(&mut self, key: KeyEvent, now: Instant) -> ComposerKey {
        let ctrl = has(&key, KeyModifiers::CONTROL);
        let alt = has(&key, KeyModifiers::ALT);
        match key.code {
            KeyCode::Esc => ComposerKey::Escape,
            KeyCode::Char('[') if ctrl => ComposerKey::Escape,
            KeyCode::Char(ch) if !ctrl && !alt => {
                let chars = self.chars();
                let cursor = self.cursor_char();
                let end = vi::line_end(&chars, cursor);
                if cursor < end {
                    let next = vi::grapheme_next(&chars, cursor, 1);
                    let original: String = chars[cursor..next].iter().collect();
                    let mut original = original.chars();
                    let first = original.next();
                    self.replace_range(cursor, next, &ch.to_string(), cursor + 1, now);
                    self.replaced
                        .push(first.filter(|_| original.next().is_none()));
                } else {
                    self.replace_range(cursor, cursor, &ch.to_string(), cursor + 1, now);
                    self.replaced.push(None);
                }
                ComposerKey::Edited
            }
            KeyCode::Enter => {
                let cursor = self.cursor_char();
                self.replace_range(cursor, cursor, "\n", cursor + 1, now);
                self.replaced.push(None);
                ComposerKey::Edited
            }
            KeyCode::Backspace => {
                let cursor = self.cursor_char();
                if cursor == 0 {
                    return ComposerKey::Handled;
                }
                match self.replaced.pop() {
                    Some(Some(original)) => {
                        self.replace_range(
                            cursor - 1,
                            cursor,
                            &original.to_string(),
                            cursor - 1,
                            now,
                        );
                        ComposerKey::Edited
                    }
                    Some(None) => {
                        self.replace_range(cursor - 1, cursor, "", cursor - 1, now);
                        ComposerKey::Edited
                    }
                    None => {
                        self.set_cursor_char(cursor - 1);
                        ComposerKey::Handled
                    }
                }
            }
            KeyCode::Left
            | KeyCode::Right
            | KeyCode::Up
            | KeyCode::Down
            | KeyCode::Home
            | KeyCode::End => {
                self.replaced.clear();
                self.textarea.input(key);
                self.snap_cursor(matches!(
                    key.code,
                    KeyCode::Left | KeyCode::Up | KeyCode::Home
                ));
                ComposerKey::Handled
            }
            _ => ComposerKey::Unhandled,
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
