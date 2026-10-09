use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LineInput {
    text: String,
    cursor: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputOutcome {
    Edited,
    Moved,
    Submit,
    Cancel,
    Ignored,
}

impl LineInput {
    pub fn new(text: impl Into<String>) -> Self {
        let text = text.into();
        let cursor = text.len();
        Self { text, cursor }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn cursor_width(&self) -> usize {
        self.text[..self.cursor].width()
    }

    fn prev_boundary(&self) -> usize {
        self.text[..self.cursor]
            .grapheme_indices(true)
            .next_back()
            .map_or(0, |(index, _)| index)
    }

    fn next_boundary(&self) -> usize {
        self.text[self.cursor..]
            .graphemes(true)
            .next()
            .map_or(self.cursor, |grapheme| self.cursor + grapheme.len())
    }

    fn prev_word_boundary(&self) -> usize {
        let head = &self.text[..self.cursor];
        let trimmed = head.trim_end();
        trimmed
            .unicode_word_indices()
            .next_back()
            .map_or(0, |(index, _)| index)
    }

    pub fn insert(&mut self, ch: char) {
        self.text.insert(self.cursor, ch);
        self.cursor += ch.len_utf8();
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> InputOutcome {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Enter => InputOutcome::Submit,
            KeyCode::Esc => InputOutcome::Cancel,
            KeyCode::Char('c') if ctrl => InputOutcome::Cancel,
            KeyCode::Char('a') if ctrl => {
                self.cursor = 0;
                InputOutcome::Moved
            }
            KeyCode::Char('e') if ctrl => {
                self.cursor = self.text.len();
                InputOutcome::Moved
            }
            KeyCode::Char('u') if ctrl => {
                self.text.drain(..self.cursor);
                self.cursor = 0;
                InputOutcome::Edited
            }
            KeyCode::Char('w') if ctrl => {
                let start = self.prev_word_boundary();
                self.text.drain(start..self.cursor);
                self.cursor = start;
                InputOutcome::Edited
            }
            KeyCode::Char(ch) if !ctrl => {
                self.insert(ch);
                InputOutcome::Edited
            }
            KeyCode::Backspace => {
                if self.cursor == 0 {
                    return InputOutcome::Ignored;
                }
                let start = self.prev_boundary();
                self.text.drain(start..self.cursor);
                self.cursor = start;
                InputOutcome::Edited
            }
            KeyCode::Delete => {
                let end = self.next_boundary();
                if end == self.cursor {
                    return InputOutcome::Ignored;
                }
                self.text.drain(self.cursor..end);
                InputOutcome::Edited
            }
            KeyCode::Left => {
                self.cursor = self.prev_boundary();
                InputOutcome::Moved
            }
            KeyCode::Right => {
                self.cursor = self.next_boundary();
                InputOutcome::Moved
            }
            KeyCode::Home => {
                self.cursor = 0;
                InputOutcome::Moved
            }
            KeyCode::End => {
                self.cursor = self.text.len();
                InputOutcome::Moved
            }
            _ => InputOutcome::Ignored,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::from(code)
    }

    #[test]
    fn edits_respect_graphemes() {
        let mut input = LineInput::new("héllo👍🏽");
        assert_eq!(
            input.handle_key(key(KeyCode::Backspace)),
            InputOutcome::Edited
        );
        assert_eq!(input.text(), "héllo");
        input.handle_key(key(KeyCode::Home));
        input.handle_key(key(KeyCode::Right));
        input.handle_key(key(KeyCode::Right));
        input.handle_key(key(KeyCode::Delete));
        assert_eq!(input.text(), "hélo");
        input.handle_key(key(KeyCode::Char('L')));
        assert_eq!(input.text(), "héLlo");
        assert_eq!(input.cursor_width(), 3);
    }

    #[test]
    fn ctrl_w_deletes_previous_word_and_ctrl_u_clears_head() {
        let mut input = LineInput::new("fix the  bug ");
        input.handle_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL));
        assert_eq!(input.text(), "fix the  ");
        input.handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        assert_eq!(input.text(), "");
        assert_eq!(
            input.handle_key(key(KeyCode::Backspace)),
            InputOutcome::Ignored
        );
    }

    #[test]
    fn submit_and_cancel() {
        let mut input = LineInput::default();
        assert_eq!(input.handle_key(key(KeyCode::Enter)), InputOutcome::Submit);
        assert_eq!(input.handle_key(key(KeyCode::Esc)), InputOutcome::Cancel);
    }
}
