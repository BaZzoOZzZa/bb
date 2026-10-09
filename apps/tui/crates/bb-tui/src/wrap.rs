use ratatui::style::Style;
use ratatui::text::{Line, Span};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

pub const TAB: &str = "    ";

pub fn text_width(text: &str) -> usize {
    text.width()
}

pub fn spans_width(spans: &[Span<'_>]) -> usize {
    spans.iter().map(|span| span.content.width()).sum()
}

fn grapheme_width(grapheme: &str) -> usize {
    if grapheme == "\t" {
        return TAB.len();
    }
    grapheme.width()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WrapMode {
    Words,
    Chars,
}

struct Builder {
    width: usize,
    rest_prefix: Vec<Span<'static>>,
    rest_prefix_width: usize,
    lines: Vec<Line<'static>>,
    current: Vec<Span<'static>>,
    used: usize,
    content: bool,
    prefix_spans: usize,
}

impl Builder {
    fn push_text(&mut self, text: &str, style: Style) {
        if text.is_empty() {
            return;
        }
        self.used += text.width();
        self.content = true;
        let mergeable = self.current.len() > self.prefix_spans;
        if mergeable
            && let Some(last) = self.current.last_mut()
            && last.style == style
        {
            let mut owned = last.content.to_string();
            owned.push_str(text);
            last.content = owned.into();
            return;
        }
        self.current.push(Span::styled(text.to_owned(), style));
    }

    fn remaining(&self) -> usize {
        self.width.saturating_sub(self.used)
    }

    fn break_line(&mut self) {
        let spans = std::mem::take(&mut self.current);
        self.lines.push(Line::from(spans));
        self.current = self.rest_prefix.clone();
        self.prefix_spans = self.current.len();
        self.used = self.rest_prefix_width;
        self.content = false;
    }

    fn finish(mut self) -> Vec<Line<'static>> {
        let spans = std::mem::take(&mut self.current);
        self.lines.push(Line::from(spans));
        self.lines
    }
}

fn tokens(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut in_space: Option<bool> = None;
    for (index, grapheme) in text.grapheme_indices(true) {
        let space = grapheme.chars().all(char::is_whitespace);
        if in_space.is_some_and(|previous| previous != space) {
            out.push(&text[start..index]);
            start = index;
        }
        in_space = Some(space);
    }
    if start < text.len() {
        out.push(&text[start..]);
    }
    out
}

fn push_broken(builder: &mut Builder, token: &str, style: Style) {
    let mut chunk = String::new();
    let mut chunk_width = 0;
    for grapheme in token.graphemes(true) {
        let grapheme = if grapheme == "\t" { TAB } else { grapheme };
        let width = grapheme_width(grapheme);
        if chunk_width + width > builder.remaining() {
            if !chunk.is_empty() {
                builder.push_text(&chunk, style);
                chunk.clear();
                chunk_width = 0;
            }
            if builder.content || builder.remaining() < width {
                builder.break_line();
            }
        }
        chunk.push_str(grapheme);
        chunk_width += width;
    }
    builder.push_text(&chunk, style);
}

pub fn wrap_spans(
    spans: &[Span<'static>],
    width: usize,
    first_prefix: &[Span<'static>],
    rest_prefix: &[Span<'static>],
    mode: WrapMode,
) -> Vec<Line<'static>> {
    let width = width.max(1);
    let mut first_prefix = first_prefix.to_vec();
    let mut rest_prefix = rest_prefix.to_vec();
    if spans_width(&first_prefix) >= width {
        first_prefix.clear();
    }
    if spans_width(&rest_prefix) >= width {
        rest_prefix.clear();
    }
    let rest_prefix_width = spans_width(&rest_prefix);
    let mut builder = Builder {
        width,
        prefix_spans: first_prefix.len(),
        used: spans_width(&first_prefix),
        current: first_prefix,
        rest_prefix,
        rest_prefix_width,
        lines: Vec::new(),
        content: false,
    };
    for span in spans {
        let style = span.style;
        let mut segments = span.content.split('\n').peekable();
        while let Some(segment) = segments.next() {
            match mode {
                WrapMode::Chars => push_broken(&mut builder, segment, style),
                WrapMode::Words => {
                    for token in tokens(segment) {
                        let is_space = token.chars().all(char::is_whitespace);
                        let token_width: usize = token.graphemes(true).map(grapheme_width).sum();
                        if is_space {
                            if !builder.content {
                                continue;
                            }
                            if token_width >= builder.remaining() {
                                builder.break_line();
                                continue;
                            }
                            builder.push_text(&token.replace('\t', TAB), style);
                            continue;
                        }
                        if token_width <= builder.remaining() {
                            builder.push_text(token, style);
                        } else if token_width <= width.saturating_sub(builder.rest_prefix_width)
                            && builder.content
                        {
                            trim_trailing_space(&mut builder);
                            builder.break_line();
                            builder.push_text(token, style);
                        } else {
                            push_broken(&mut builder, token, style);
                        }
                    }
                }
            }
            if segments.peek().is_some() {
                if mode == WrapMode::Words {
                    trim_trailing_space(&mut builder);
                }
                builder.break_line();
            }
        }
    }
    if mode == WrapMode::Words {
        trim_trailing_space(&mut builder);
    }
    builder.finish()
}

fn trim_trailing_space(builder: &mut Builder) {
    if !builder.content {
        return;
    }
    while builder.current.len() > builder.prefix_spans
        && let Some(last) = builder.current.last_mut()
    {
        let trimmed = last.content.trim_end();
        if trimmed.len() == last.content.len() {
            break;
        }
        let removed = last.content.width() - trimmed.width();
        builder.used = builder.used.saturating_sub(removed);
        if trimmed.is_empty() {
            builder.current.pop();
            continue;
        }
        last.content = trimmed.to_owned().into();
        break;
    }
}

pub fn wrap_plain(
    text: &str,
    style: Style,
    width: usize,
    prefix: &[Span<'static>],
    mode: WrapMode,
) -> Vec<Line<'static>> {
    wrap_spans(
        &[Span::styled(text.to_owned(), style)],
        width,
        prefix,
        prefix,
        mode,
    )
}

pub fn truncate_spans(spans: Vec<Span<'static>>, width: usize) -> Vec<Span<'static>> {
    let mut used = 0;
    let mut out = Vec::new();
    for span in spans {
        let span_width = span.content.width();
        if used + span_width <= width {
            used += span_width;
            out.push(span);
            continue;
        }
        let mut text = String::new();
        for grapheme in span.content.graphemes(true) {
            let grapheme_width = grapheme.width();
            if used + grapheme_width + 1 > width {
                break;
            }
            used += grapheme_width;
            text.push_str(grapheme);
        }
        text.push('…');
        out.push(Span::styled(text, span.style));
        break;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Color;

    fn text(lines: &[Line<'_>]) -> Vec<String> {
        lines.iter().map(|line| line.to_string()).collect()
    }

    #[test]
    fn wraps_words_and_keeps_prefixes() {
        let lines = wrap_spans(
            &[Span::raw("the quick brown fox jumps over the lazy dog")],
            16,
            &[Span::raw("- ")],
            &[Span::raw("  ")],
            WrapMode::Words,
        );
        assert_eq!(
            text(&lines),
            vec![
                "- the quick",
                "  brown fox",
                "  jumps over the",
                "  lazy dog"
            ]
        );
    }

    #[test]
    fn breaks_long_words_and_wide_characters() {
        let lines = wrap_spans(
            &[Span::raw("aaaaaaaaaaaa 你好世界你好")],
            5,
            &[],
            &[],
            WrapMode::Words,
        );
        assert_eq!(
            text(&lines),
            vec!["aaaaa", "aaaaa", "aa 你", "好世", "界你", "好"]
        );
        for line in &lines {
            assert!(line.width() <= 5);
        }
    }

    #[test]
    fn preserves_styles_and_newlines() {
        let bold = Style::new().fg(Color::Red);
        let lines = wrap_spans(
            &[Span::raw("one "), Span::styled("two\nthree", bold)],
            40,
            &[],
            &[],
            WrapMode::Words,
        );
        assert_eq!(text(&lines), vec!["one two", "three"]);
        assert_eq!(lines[0].spans[1].style, bold);
        assert_eq!(lines[1].spans[0].style, bold);
    }

    #[test]
    fn char_mode_keeps_spaces() {
        let lines = wrap_plain("a  b\tc", Style::new(), 4, &[], WrapMode::Chars);
        assert_eq!(text(&lines), vec!["a  b", "    ", "c"]);
    }

    #[test]
    fn truncates_with_ellipsis() {
        let spans = truncate_spans(vec![Span::raw("hello "), Span::raw("world")], 8);
        assert_eq!(Line::from(spans).to_string(), "hello w…");
    }
}
