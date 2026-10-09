#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamingMarkdownSplit<'a> {
    pub settled: &'a str,
    pub tail: &'a str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MarkdownFence {
    pub character: char,
    pub length: usize,
}

const MATH_DELIMITER: &str = "$$";

fn any_indent_fence(line: &str) -> Option<(MarkdownFence, usize)> {
    let start = line.len() - line.trim_start().len();
    let rest = &line[start..];
    let character = rest.chars().next().filter(|ch| *ch == '`' || *ch == '~')?;
    let length = rest.chars().take_while(|ch| *ch == character).count();
    (length >= 3).then_some((MarkdownFence { character, length }, start + length))
}

pub fn parse_any_indent_markdown_fence_start(line: &str) -> Option<MarkdownFence> {
    any_indent_fence(line).map(|(fence, _)| fence)
}

pub fn closes_any_indent_markdown_fence(line: &str, fence: MarkdownFence) -> bool {
    any_indent_fence(line).is_some_and(|(marker, end)| {
        marker.character == fence.character
            && marker.length >= fence.length
            && line[end..].trim().is_empty()
    })
}

pub fn is_markdown_indented_continuation(line: &str) -> bool {
    line.starts_with("  ") || line.starts_with('\t')
}

pub fn is_markdown_list_marker(line: &str) -> bool {
    let indent = line
        .chars()
        .take_while(|ch| ch.is_whitespace())
        .count()
        .min(3);
    let mut chars = line.chars().skip(indent).peekable();
    match chars.peek().copied() {
        Some('-' | '*' | '+') => {
            chars.next();
        }
        Some(ch) if ch.is_ascii_digit() => {
            let mut digits = 0;
            while chars.peek().is_some_and(char::is_ascii_digit) {
                chars.next();
                digits += 1;
            }
            if digits > 9 || !matches!(chars.next(), Some('.' | ')')) {
                return false;
            }
        }
        _ => return false,
    }
    chars.next().is_none_or(char::is_whitespace)
}

pub fn is_markdown_list_like_line(line: &str) -> bool {
    is_markdown_list_marker(line) || is_markdown_indented_continuation(line)
}

fn odd_math_delimiters(line: &str) -> bool {
    line.matches(MATH_DELIMITER).count() % 2 == 1
}

pub fn split_streaming_markdown(text: &str) -> Option<StreamingMarkdownSplit<'_>> {
    let lines: Vec<&str> = text.split('\n').collect();
    let last_complete = lines.len().checked_sub(2)?;
    let mut open_fence: Option<MarkdownFence> = None;
    let mut math_open = false;
    let mut last_non_blank: Option<&str> = None;
    let mut boundary: Option<usize> = None;
    for index in 0..=last_complete {
        let line = lines[index];
        if let Some(fence) = open_fence {
            if closes_any_indent_markdown_fence(line, fence) {
                open_fence = None;
            }
            last_non_blank = Some(line);
            continue;
        }
        if math_open {
            if odd_math_delimiters(line) {
                math_open = false;
            }
            last_non_blank = Some(line);
            continue;
        }
        if line.trim().is_empty() {
            if index + 1 > last_complete {
                continue;
            }
            let next = lines[index + 1];
            if is_markdown_indented_continuation(next) {
                continue;
            }
            if last_non_blank.is_some_and(is_markdown_list_like_line)
                && is_markdown_list_marker(next)
            {
                continue;
            }
            if last_non_blank.is_none() {
                continue;
            }
            boundary = Some(index);
            continue;
        }
        last_non_blank = Some(line);
        if let Some(fence) = parse_any_indent_markdown_fence_start(line) {
            open_fence = Some(fence);
            continue;
        }
        if odd_math_delimiters(line) {
            math_open = true;
        }
    }
    let boundary = boundary?;
    let settled_length: usize = lines[..=boundary].iter().map(|line| line.len() + 1).sum();
    Some(StreamingMarkdownSplit {
        settled: &text[..settled_length],
        tail: &text[settled_length..],
    })
}

fn is_escaped(bytes: &[u8], index: usize) -> bool {
    bytes[..index]
        .iter()
        .rev()
        .take_while(|byte| **byte == b'\\')
        .count()
        % 2
        == 1
}

fn scan_code_spans(text: &str) -> (Vec<(usize, usize)>, usize) {
    let bytes = text.as_bytes();
    let mut spans = Vec::new();
    let mut open: Option<(usize, usize)> = None;
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'`' || is_escaped(bytes, index) {
            index += 1;
            continue;
        }
        let mut end = index + 1;
        while end < bytes.len() && bytes[end] == b'`' {
            end += 1;
        }
        let length = end - index;
        match open {
            None => open = Some((index, length)),
            Some((start, open_length)) if open_length == length => {
                spans.push((start, end));
                open = None;
            }
            Some(_) => {}
        }
        index = end;
    }
    if let Some((start, length)) = open {
        spans.push((start, bytes.len()));
        return (spans, length);
    }
    (spans, 0)
}

pub fn close_unterminated_markdown_code_span(text: &str) -> String {
    let (_, open_length) = scan_code_spans(text);
    format!("{text}{}", "`".repeat(open_length))
}

fn is_directive_line(line: &str) -> bool {
    let mut rest = line;
    loop {
        let trimmed = rest.trim_start_matches([' ', '\t']);
        if let Some(after) = trimmed
            .strip_prefix('>')
            .or_else(|| trimmed.strip_prefix(['-', '+', '*']))
        {
            rest = after;
            continue;
        }
        let digits = trimmed.chars().take_while(char::is_ascii_digit).count();
        if (1..=9).contains(&digits)
            && let Some(after) = trimmed[digits..].strip_prefix(['.', ')'])
        {
            rest = after;
            continue;
        }
        rest = trimmed;
        break;
    }
    rest.strip_prefix("::")
        .and_then(|after| after.chars().next())
        .is_some_and(|ch| ch.is_ascii_alphabetic())
}

fn bypasses_repair(tail: &str) -> bool {
    tail.contains("```") || tail.contains("~~~") || tail.split('\n').any(is_directive_line)
}

fn in_spans(spans: &[(usize, usize)], index: usize) -> bool {
    spans
        .iter()
        .any(|(start, end)| index >= *start && index < *end)
}

fn strip_incomplete_link(text: &str, spans: &[(usize, usize)]) -> Option<String> {
    let bytes = text.as_bytes();
    let open = (0..bytes.len()).rev().find(|index| {
        bytes[*index] == b'[' && !in_spans(spans, *index) && !is_escaped(bytes, *index)
    })?;
    let after = &text[open + 1..];
    let is_image = open > 0 && bytes[open - 1] == b'!';
    let prefix_end = if is_image { open - 1 } else { open };
    match after.find(']') {
        None => {
            let label = if is_image { "" } else { after };
            Some(format!("{}{label}", &text[..prefix_end]))
        }
        Some(close) => {
            let rest = &after[close + 1..];
            if rest.starts_with('(') && !rest.contains(')') {
                let label = if is_image { "" } else { &after[..close] };
                Some(format!("{}{label}", &text[..prefix_end]))
            } else {
                None
            }
        }
    }
}

fn is_list_bullet_at(bytes: &[u8], index: usize) -> bool {
    let line_start = bytes[..index]
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |position| position + 1);
    bytes[line_start..index]
        .iter()
        .all(|byte| *byte == b' ' || *byte == b'\t')
        && bytes
            .get(index + 1)
            .is_none_or(|byte| *byte == b' ' || *byte == b'\t')
}

fn close_emphasis(text: &str, spans: &[(usize, usize)]) -> String {
    let bytes = text.as_bytes();
    let mut stack: Vec<(&'static str, usize)> = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if in_spans(spans, index) || !matches!(byte, b'*' | b'_' | b'~') || is_escaped(bytes, index)
        {
            index += 1;
            continue;
        }
        let mut end = index + 1;
        while end < bytes.len() && bytes[end] == byte {
            end += 1;
        }
        let run = end - index;
        let marker: Option<&'static str> = match (byte, run) {
            (b'*', 1) if !is_list_bullet_at(bytes, index) => Some("*"),
            (b'*', 2) => Some("**"),
            (b'*', 3) => Some("***"),
            (b'_', _) => {
                let before = index.checked_sub(1).map(|i| bytes[i]);
                let after = bytes.get(end).copied();
                let intraword = before.is_some_and(|b| b.is_ascii_alphanumeric())
                    && after.is_some_and(|b| b.is_ascii_alphanumeric());
                match (intraword, run) {
                    (false, 1) => Some("_"),
                    (false, 2) => Some("__"),
                    _ => None,
                }
            }
            (b'~', 2) => Some("~~"),
            _ => None,
        };
        if let Some(marker) = marker {
            match stack.iter().rposition(|(open, _)| *open == marker) {
                Some(position) => {
                    stack.truncate(position);
                }
                None => stack.push((marker, index)),
            }
        }
        index = end;
    }
    let mut out = text.to_owned();
    while let Some((marker, position)) = stack.pop() {
        if out[position + marker.len()..].trim().is_empty() {
            out.truncate(position);
            out.truncate(out.trim_end().len());
        } else {
            out.push_str(marker);
        }
    }
    out
}

pub fn repair_streaming_markdown_tail(tail: &str) -> String {
    if bypasses_repair(tail) {
        return tail.to_owned();
    }
    let (spans, open_code) = scan_code_spans(tail);
    if open_code > 0 {
        return close_unterminated_markdown_code_span(tail);
    }
    let text = strip_incomplete_link(tail, &spans).unwrap_or_else(|| tail.to_owned());
    let (spans, _) = scan_code_spans(&text);
    close_emphasis(&text, &spans)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expect_split(text: &str, settled: &str) {
        let split = split_streaming_markdown(text).expect("split");
        assert_eq!(split.settled, settled);
        assert_eq!(split.tail, &text[settled.len()..]);
        assert_eq!(format!("{}{}", split.settled, split.tail), text);
    }

    #[test]
    fn returns_none_without_a_blank_line_followed_by_a_complete_line() {
        assert_eq!(split_streaming_markdown(""), None);
        assert_eq!(split_streaming_markdown("Only one paragraph so far"), None);
        assert_eq!(
            split_streaming_markdown("Para one.\n\nPara two still"),
            None
        );
        assert_eq!(split_streaming_markdown("\n\nleading blanks\n"), None);
    }

    #[test]
    fn splits_at_the_last_blank_line_whose_next_line_is_complete() {
        expect_split("Para one.\n\nPara two.\n\nPara three", "Para one.\n\n");
        expect_split(
            "Para one.\n\nPara two.\n\nPara three.\nmore",
            "Para one.\n\nPara two.\n\n",
        );
    }

    #[test]
    fn only_moves_the_boundary_forward_as_text_streams_in() {
        let chunks = [
            "# Title\n",
            "\n",
            "Intro paragraph.\n",
            "\n",
            "```ts\n",
            "const a = 1;\n",
            "\n",
            "const b = 2;\n",
            "```\n",
            "\n",
            "1. first\n",
            "\n",
            "2. second\n",
            "\n",
            "Closing",
            " words.\n",
            "\n",
            "Done.\n",
        ];
        let mut text = String::new();
        let mut previous = 0;
        for chunk in chunks {
            text.push_str(chunk);
            let split = split_streaming_markdown(&text);
            let settled = split.map_or(0, |split| split.settled.len());
            assert!(settled >= previous, "{text:?}");
            if let Some(split) = split {
                assert!(text.starts_with(split.settled));
            }
            previous = settled;
        }
    }

    #[test]
    fn does_not_split_inside_an_open_fenced_code_block() {
        let text = "Intro.\n\n```js\nline one\n\nline two\n\nline three\n";
        expect_split(text, "Intro.\n\n");
        let closed = format!("{text}```\n\nAfter the fence.\n");
        expect_split(
            &closed,
            "Intro.\n\n```js\nline one\n\nline two\n\nline three\n```\n\n",
        );
    }

    #[test]
    fn requires_the_closing_fence_to_match_the_opening_marker() {
        expect_split("Intro.\n\n````md\n```\ninner\n```\n\ntext\n", "Intro.\n\n");
        expect_split("Intro.\n\n~~~\n```\n\nx\n", "Intro.\n\n");
    }

    #[test]
    fn does_not_split_inside_an_open_math_block() {
        let text = "Formula:\n\n$$\na = b\n\nc = d\n";
        expect_split(text, "Formula:\n\n");
        let closed = format!("{text}$$\n\nDone.\n");
        expect_split(&closed, "Formula:\n\n$$\na = b\n\nc = d\n$$\n\n");
    }

    #[test]
    fn treats_inline_math_spans_as_closed() {
        expect_split(
            "Price is $$x$$ here.\n\nNext para.\n\nTail\n",
            "Price is $$x$$ here.\n\nNext para.\n\n",
        );
    }

    #[test]
    fn does_not_split_between_loose_list_items_or_before_continuations() {
        expect_split(
            "Intro.\n\n- one\n\n- two\n\n- three\n\nAfter list.\n",
            "Intro.\n\n- one\n\n- two\n\n- three\n\n",
        );
        expect_split(
            "Intro.\n\n1. one\n\n   continued\n\n2. two\n\nAfter.\n",
            "Intro.\n\n1. one\n\n   continued\n\n2. two\n\n",
        );
    }

    #[test]
    fn splits_before_a_heading_that_follows_a_blank_line() {
        let split = split_streaming_markdown("Intro text.\n\n## Section\n\nBody").expect("split");
        assert_eq!(split.settled, "Intro text.\n\n");
        assert_eq!(split.tail, "## Section\n\nBody");
        expect_split(
            "Intro text.\n\n## Section\n\nBody paragraph.\n\nMore",
            "Intro text.\n\n## Section\n\n",
        );
    }

    #[test]
    fn closes_unterminated_code_spans() {
        assert_eq!(
            close_unterminated_markdown_code_span("run `cargo"),
            "run `cargo`"
        );
        assert_eq!(
            close_unterminated_markdown_code_span("a ``b` c"),
            "a ``b` c``"
        );
        assert_eq!(
            close_unterminated_markdown_code_span("done `x` y"),
            "done `x` y"
        );
        assert_eq!(
            close_unterminated_markdown_code_span("escaped \\` tick"),
            "escaped \\` tick"
        );
    }

    #[test]
    fn repairs_streaming_tails() {
        assert_eq!(
            repair_streaming_markdown_tail("some **bold"),
            "some **bold**"
        );
        assert_eq!(
            repair_streaming_markdown_tail("*it and **b"),
            "*it and **b***"
        );
        assert_eq!(
            repair_streaming_markdown_tail("snake_case_name"),
            "snake_case_name"
        );
        assert_eq!(repair_streaming_markdown_tail("_lead"), "_lead_");
        assert_eq!(repair_streaming_markdown_tail("~~gone"), "~~gone~~");
        assert_eq!(repair_streaming_markdown_tail("trailing **"), "trailing");
        assert_eq!(
            repair_streaming_markdown_tail("* item\n* two"),
            "* item\n* two"
        );
        assert_eq!(
            repair_streaming_markdown_tail("see [docs](http://x"),
            "see docs"
        );
        assert_eq!(repair_streaming_markdown_tail("see [doc"), "see doc");
        assert_eq!(repair_streaming_markdown_tail("img ![alt](http"), "img ");
        assert_eq!(
            repair_streaming_markdown_tail("ok [a](b) done"),
            "ok [a](b) done"
        );
        assert_eq!(repair_streaming_markdown_tail("`**code"), "`**code`");
        assert_eq!(repair_streaming_markdown_tail("```rust\nfn"), "```rust\nfn");
        assert_eq!(repair_streaming_markdown_tail("- ::note{x}"), "- ::note{x}");
    }
}
