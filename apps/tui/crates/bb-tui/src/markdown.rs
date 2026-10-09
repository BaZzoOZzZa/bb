use pulldown_cmark::{Alignment, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use crate::highlight::highlight_code;
use crate::wrap::{WrapMode, spans_width, wrap_spans};

pub const CODE_STYLE: Style = Style::new().fg(Color::Yellow);
const LINK_STYLE: Style = Style::new()
    .fg(Color::LightBlue)
    .add_modifier(Modifier::UNDERLINED);
const MUTED: Style = Style::new().fg(Color::DarkGray);
const MIN_COLUMN: usize = 3;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct RenderedMarkdown {
    pub lines: Vec<Line<'static>>,
    pub links: Vec<String>,
}

#[derive(Debug, Clone)]
enum Container {
    Quote,
    Item { marker: String, pending: bool },
}

#[derive(Debug, Default)]
struct Table {
    alignments: Vec<Alignment>,
    header: Option<Vec<Vec<Span<'static>>>>,
    rows: Vec<Vec<Vec<Span<'static>>>>,
    current: Vec<Vec<Span<'static>>>,
}

struct Renderer {
    width: usize,
    base: Style,
    lines: Vec<Line<'static>>,
    links: Vec<String>,
    inline: Vec<Span<'static>>,
    modifiers: Vec<Style>,
    containers: Vec<Container>,
    lists: Vec<Option<u64>>,
    pending_blank: bool,
    link_starts: Vec<(String, usize)>,
    code_block: Option<(Option<String>, String)>,
    table: Option<Table>,
    heading: Option<HeadingLevel>,
}

fn heading_style(level: HeadingLevel) -> Style {
    match level {
        HeadingLevel::H1 => Style::new()
            .fg(Color::Magenta)
            .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        HeadingLevel::H2 => Style::new().fg(Color::Magenta).add_modifier(Modifier::BOLD),
        HeadingLevel::H3 => Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        _ => Style::new().add_modifier(Modifier::BOLD),
    }
}

fn is_openable_link(dest: &str) -> bool {
    let lower = dest.to_ascii_lowercase();
    lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("file://")
        || lower.starts_with("mailto:")
        || dest.starts_with('/')
        || dest.starts_with("~/")
}

pub fn bare_urls(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(start) = ["https://", "http://"]
        .iter()
        .filter_map(|scheme| rest.find(scheme))
        .min()
    {
        let candidate = &rest[start..];
        let end = candidate
            .find(|ch: char| ch.is_whitespace() || matches!(ch, '<' | '>' | '"' | '`' | ')' | ']'))
            .unwrap_or(candidate.len());
        let url = candidate[..end].trim_end_matches(['.', ',', ';', ':', '!', '?', '\'']);
        if url.len() > "https://".len() {
            out.push(url.to_owned());
        }
        rest = &candidate[end.max(1)..];
    }
    out
}

impl Renderer {
    fn new(width: usize, base: Style) -> Self {
        Self {
            width: width.max(1),
            base,
            lines: Vec::new(),
            links: Vec::new(),
            inline: Vec::new(),
            modifiers: Vec::new(),
            containers: Vec::new(),
            lists: Vec::new(),
            pending_blank: false,
            link_starts: Vec::new(),
            code_block: None,
            table: None,
            heading: None,
        }
    }

    fn style(&self) -> Style {
        let mut style = self.base;
        if let Some(level) = self.heading {
            style = style.patch(heading_style(level));
        }
        for modifier in &self.modifiers {
            style = style.patch(*modifier);
        }
        style
    }

    fn push(&mut self, text: impl Into<String>, style: Style) {
        let text = text.into();
        if !text.is_empty() {
            self.inline.push(Span::styled(text, style));
        }
    }

    fn prefixes(&self) -> (Vec<Span<'static>>, Vec<Span<'static>>) {
        let mut first = Vec::new();
        let mut rest = Vec::new();
        for container in &self.containers {
            match container {
                Container::Quote => {
                    first.push(Span::styled("│ ", MUTED));
                    rest.push(Span::styled("│ ", MUTED));
                }
                Container::Item { marker, pending } => {
                    let pad = " ".repeat(crate::wrap::text_width(marker));
                    if *pending {
                        first.push(Span::styled(marker.clone(), MUTED));
                    } else {
                        first.push(Span::raw(pad.clone()));
                    }
                    rest.push(Span::raw(pad));
                }
            }
        }
        (first, rest)
    }

    fn consume_markers(&mut self) {
        for container in &mut self.containers {
            if let Container::Item { pending, .. } = container {
                *pending = false;
            }
        }
    }

    fn quote_prefix(&self) -> Vec<Span<'static>> {
        self.containers
            .iter()
            .filter(|container| matches!(container, Container::Quote))
            .map(|_| Span::styled("│", MUTED))
            .collect()
    }

    fn start_block(&mut self) {
        if self.pending_blank && !self.lines.is_empty() {
            self.lines.push(Line::from(self.quote_prefix()));
        }
        self.pending_blank = false;
    }

    fn flush_inline(&mut self) {
        if self.inline.is_empty() {
            return;
        }
        let spans = std::mem::take(&mut self.inline);
        let (first, rest) = self.prefixes();
        self.lines.extend(wrap_spans(
            &spans,
            self.width,
            &first,
            &rest,
            WrapMode::Words,
        ));
        self.consume_markers();
    }

    fn push_prefixed(&mut self, content: Vec<Vec<Span<'static>>>, mode: WrapMode) {
        for spans in content {
            let (first, rest) = self.prefixes();
            self.lines
                .extend(wrap_spans(&spans, self.width, &first, &rest, mode));
            self.consume_markers();
        }
    }

    fn start(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Paragraph => {
                self.flush_inline();
                self.start_block();
            }
            Tag::Heading { level, .. } => {
                self.flush_inline();
                self.start_block();
                self.heading = Some(level);
            }
            Tag::BlockQuote(_) => {
                self.flush_inline();
                self.start_block();
                self.containers.push(Container::Quote);
            }
            Tag::CodeBlock(kind) => {
                self.flush_inline();
                self.start_block();
                let lang = match kind {
                    CodeBlockKind::Fenced(info) => info
                        .split_whitespace()
                        .next()
                        .map(str::to_owned)
                        .filter(|lang| !lang.is_empty()),
                    CodeBlockKind::Indented => None,
                };
                self.code_block = Some((lang, String::new()));
            }
            Tag::HtmlBlock => {
                self.flush_inline();
                self.start_block();
            }
            Tag::List(start) => {
                self.flush_inline();
                if self.lists.is_empty() {
                    self.start_block();
                }
                self.lists.push(start);
            }
            Tag::Item => {
                self.flush_inline();
                let marker = match self.lists.last_mut() {
                    Some(Some(number)) => {
                        let marker = format!("{number}. ");
                        *number += 1;
                        marker
                    }
                    _ => "• ".to_owned(),
                };
                self.containers.push(Container::Item {
                    marker,
                    pending: true,
                });
            }
            Tag::FootnoteDefinition(label) => {
                self.flush_inline();
                self.start_block();
                self.push(format!("[^{label}]: "), MUTED);
            }
            Tag::DefinitionList | Tag::DefinitionListTitle | Tag::DefinitionListDefinition => {
                self.flush_inline();
                self.start_block();
            }
            Tag::Table(alignments) => {
                self.flush_inline();
                self.start_block();
                self.table = Some(Table {
                    alignments,
                    ..Table::default()
                });
            }
            Tag::TableHead | Tag::TableRow | Tag::TableCell => {}
            Tag::Emphasis => self
                .modifiers
                .push(Style::new().add_modifier(Modifier::ITALIC)),
            Tag::Strong => self
                .modifiers
                .push(Style::new().add_modifier(Modifier::BOLD)),
            Tag::Strikethrough => self
                .modifiers
                .push(Style::new().add_modifier(Modifier::CROSSED_OUT)),
            Tag::Superscript | Tag::Subscript => self.modifiers.push(Style::new()),
            Tag::Link { dest_url, .. } => {
                self.modifiers.push(LINK_STYLE);
                self.link_starts
                    .push((dest_url.to_string(), self.inline.len()));
            }
            Tag::Image { dest_url, .. } => {
                self.push("[image: ", MUTED);
                self.modifiers.push(MUTED);
                self.link_starts
                    .push((dest_url.to_string(), self.inline.len()));
            }
            Tag::MetadataBlock(_) => {}
        }
    }

    fn end_link(&mut self, image: bool) {
        self.modifiers.pop();
        let Some((dest, start)) = self.link_starts.pop() else {
            return;
        };
        let label: String = self.inline[start.min(self.inline.len())..]
            .iter()
            .map(|span| span.content.as_ref())
            .collect();
        if image {
            self.push("]", MUTED);
        }
        if dest.is_empty() || dest.starts_with('#') {
            return;
        }
        let normalized = dest.strip_prefix("mailto:").unwrap_or(&dest);
        if label.trim() != dest && label.trim() != normalized {
            self.push(format!(" ({dest})"), MUTED);
        }
        if is_openable_link(&dest) && !self.links.contains(&dest) {
            self.links.push(dest);
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => {
                self.flush_inline();
                self.pending_blank = true;
            }
            TagEnd::Heading(_) => {
                self.flush_inline();
                self.heading = None;
                self.pending_blank = true;
            }
            TagEnd::BlockQuote(_) => {
                self.flush_inline();
                self.containers.pop();
                self.pending_blank = true;
            }
            TagEnd::CodeBlock => {
                if let Some((lang, code)) = self.code_block.take() {
                    self.render_code(lang.as_deref(), &code);
                }
                self.pending_blank = true;
            }
            TagEnd::HtmlBlock => {
                self.flush_inline();
                self.pending_blank = true;
            }
            TagEnd::List(_) => {
                self.flush_inline();
                self.lists.pop();
                if self.lists.is_empty() {
                    self.pending_blank = true;
                }
            }
            TagEnd::Item => {
                self.flush_inline();
                if let Some(Container::Item { pending: true, .. }) = self.containers.last() {
                    let (first, _) = self.prefixes();
                    self.lines.push(Line::from(first));
                }
                self.containers.pop();
            }
            TagEnd::FootnoteDefinition
            | TagEnd::DefinitionList
            | TagEnd::DefinitionListTitle
            | TagEnd::DefinitionListDefinition => {
                self.flush_inline();
                self.pending_blank = true;
            }
            TagEnd::Table => {
                if let Some(table) = self.table.take() {
                    self.render_table(table);
                }
                self.pending_blank = true;
            }
            TagEnd::TableHead => {
                if let Some(table) = self.table.as_mut() {
                    table.header = Some(std::mem::take(&mut table.current));
                }
            }
            TagEnd::TableRow => {
                if let Some(table) = self.table.as_mut() {
                    let row = std::mem::take(&mut table.current);
                    table.rows.push(row);
                }
            }
            TagEnd::TableCell => {
                let cell = std::mem::take(&mut self.inline);
                if let Some(table) = self.table.as_mut() {
                    table.current.push(cell);
                }
            }
            TagEnd::Emphasis
            | TagEnd::Strong
            | TagEnd::Strikethrough
            | TagEnd::Superscript
            | TagEnd::Subscript => {
                self.modifiers.pop();
            }
            TagEnd::Link => self.end_link(false),
            TagEnd::Image => self.end_link(true),
            TagEnd::MetadataBlock(_) => {}
        }
    }

    fn render_code(&mut self, lang: Option<&str>, code: &str) {
        let gutter = Span::styled("▏ ", MUTED);
        let highlighted = highlight_code(code, lang, self.base);
        let content: Vec<Vec<Span<'static>>> = highlighted
            .into_iter()
            .map(|spans| {
                let mut line = vec![gutter.clone()];
                line.extend(spans);
                line
            })
            .collect();
        self.push_prefixed(content, WrapMode::Chars);
    }

    fn render_table(&mut self, table: Table) {
        let header = table.header.unwrap_or_default();
        let columns = table
            .rows
            .iter()
            .map(Vec::len)
            .chain(std::iter::once(header.len()))
            .max()
            .unwrap_or(0);
        if columns == 0 {
            return;
        }
        let (_, rest) = self.prefixes();
        let available = self.width.saturating_sub(spans_width(&rest));
        let chrome = columns * 3 + 1;
        if available < chrome + columns * MIN_COLUMN {
            let mut content = Vec::new();
            for row in std::iter::once(&header).chain(table.rows.iter()) {
                let mut spans = Vec::new();
                for (index, cell) in row.iter().enumerate() {
                    if index > 0 {
                        spans.push(Span::styled(" · ", MUTED));
                    }
                    spans.extend(cell.iter().cloned());
                }
                content.push(spans);
            }
            self.push_prefixed(content, WrapMode::Words);
            return;
        }
        let mut widths = vec![MIN_COLUMN; columns];
        for row in std::iter::once(&header).chain(table.rows.iter()) {
            for (index, cell) in row.iter().enumerate() {
                widths[index] = widths[index].max(spans_width(cell));
            }
        }
        let budget = available - chrome;
        while widths.iter().sum::<usize>() > budget {
            let Some((index, _)) = widths
                .iter()
                .enumerate()
                .filter(|(_, width)| **width > MIN_COLUMN)
                .max_by_key(|(_, width)| **width)
            else {
                break;
            };
            widths[index] -= 1;
        }
        let border = |left: &str, mid: &str, right: &str| -> Vec<Span<'static>> {
            let body = widths
                .iter()
                .map(|width| "─".repeat(width + 2))
                .collect::<Vec<_>>()
                .join(mid);
            vec![Span::styled(format!("{left}{body}{right}"), MUTED)]
        };
        let mut content = vec![border("┌", "┬", "┐")];
        let header_style = Style::new().add_modifier(Modifier::BOLD);
        if !header.is_empty() {
            content.extend(self.table_row(&header, &widths, &table.alignments, header_style));
            content.push(border("├", "┼", "┤"));
        }
        for row in &table.rows {
            content.extend(self.table_row(row, &widths, &table.alignments, Style::new()));
        }
        content.push(border("└", "┴", "┘"));
        for spans in content {
            let mut line = rest.clone();
            line.extend(spans);
            self.lines.push(Line::from(line));
        }
    }

    fn table_row(
        &self,
        row: &[Vec<Span<'static>>],
        widths: &[usize],
        alignments: &[Alignment],
        style: Style,
    ) -> Vec<Vec<Span<'static>>> {
        let empty = Vec::new();
        let cells: Vec<Vec<Line<'static>>> = widths
            .iter()
            .enumerate()
            .map(|(index, width)| {
                let spans: Vec<Span<'static>> = row
                    .get(index)
                    .unwrap_or(&empty)
                    .iter()
                    .map(|span| Span::styled(span.content.clone(), span.style.patch(style)))
                    .collect();
                wrap_spans(&spans, *width, &[], &[], WrapMode::Words)
            })
            .collect();
        let height = cells.iter().map(Vec::len).max().unwrap_or(1);
        (0..height)
            .map(|line_index| {
                let mut spans = vec![Span::styled("│", MUTED)];
                for (index, width) in widths.iter().enumerate() {
                    let line = cells[index].get(line_index);
                    let used = line.map_or(0, |line| line.width());
                    let pad = width.saturating_sub(used);
                    let (left, right) = match alignments.get(index) {
                        Some(Alignment::Right) => (pad, 0),
                        Some(Alignment::Center) => (pad / 2, pad - pad / 2),
                        _ => (0, pad),
                    };
                    spans.push(Span::raw(" ".repeat(left + 1)));
                    if let Some(line) = line {
                        spans.extend(line.spans.iter().cloned());
                    }
                    spans.push(Span::raw(" ".repeat(right + 1)));
                    spans.push(Span::styled("│", MUTED));
                }
                spans
            })
            .collect()
    }

    fn task_marker(&mut self, checked: bool) {
        if let Some(Container::Item { marker, .. }) = self.containers.last_mut() {
            let checkbox = if checked { "[x] " } else { "[ ] " };
            if marker == "• " {
                *marker = checkbox.to_owned();
            } else {
                marker.push_str(checkbox);
            }
        }
    }

    fn event(&mut self, event: Event<'_>) {
        if let Some((_, code)) = self.code_block.as_mut() {
            match event {
                Event::Text(text) => {
                    code.push_str(&text);
                    return;
                }
                Event::End(TagEnd::CodeBlock) => {}
                _ => return,
            }
        }
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(text) => {
                let style = self.style();
                self.push(text.to_string(), style);
            }
            Event::Code(code) => {
                let style = self.style().patch(CODE_STYLE);
                self.push(code.to_string(), style);
            }
            Event::InlineMath(math) => {
                let style = self.style().patch(CODE_STYLE);
                self.push(format!("${math}$"), style);
            }
            Event::DisplayMath(math) => {
                self.flush_inline();
                self.start_block();
                self.render_code(None, &math);
                self.pending_blank = true;
            }
            Event::Html(html) | Event::InlineHtml(html) => {
                let style = self.style().patch(MUTED);
                let html = html.trim_end_matches('\n').to_string();
                self.push(html, style);
            }
            Event::FootnoteReference(label) => {
                let style = self.style().patch(MUTED);
                self.push(format!("[^{label}]"), style);
            }
            Event::SoftBreak | Event::HardBreak => {
                if self.table.is_some() {
                    self.push(" ", self.base);
                } else {
                    self.push("\n", self.base);
                }
            }
            Event::Rule => {
                self.flush_inline();
                self.start_block();
                let (first, _) = self.prefixes();
                let width = self.width.saturating_sub(spans_width(&first)).min(80);
                let mut spans = first;
                spans.push(Span::styled("─".repeat(width), MUTED));
                self.lines.push(Line::from(spans));
                self.consume_markers();
                self.pending_blank = true;
            }
            Event::TaskListMarker(checked) => self.task_marker(checked),
        }
    }

    fn finish(mut self) -> RenderedMarkdown {
        self.flush_inline();
        if let Some((lang, code)) = self.code_block.take() {
            self.render_code(lang.as_deref(), &code);
        }
        if let Some(table) = self.table.take() {
            self.render_table(table);
        }
        while self
            .lines
            .last()
            .is_some_and(|line| line.spans.iter().all(|span| span.content.trim().is_empty()))
        {
            self.lines.pop();
        }
        RenderedMarkdown {
            lines: self.lines,
            links: self.links,
        }
    }
}

pub fn markdown_options() -> Options {
    Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_MATH
}

pub fn render_markdown(text: &str, width: usize, base: Style) -> RenderedMarkdown {
    let mut renderer = Renderer::new(width, base);
    for event in Parser::new_ext(text, markdown_options()) {
        renderer.event(event);
    }
    let mut rendered = renderer.finish();
    for url in bare_urls(text) {
        if !rendered.links.contains(&url) {
            rendered.links.push(url);
        }
    }
    rendered
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(text: &str, width: usize) -> Vec<String> {
        render_markdown(text, width, Style::new())
            .lines
            .iter()
            .map(|line| line.to_string().trim_end().to_owned())
            .collect()
    }

    #[test]
    fn paragraphs_breaks_and_emphasis() {
        let rendered =
            render_markdown("Hello **bold** _it_\nnext line\n\nSecond", 40, Style::new());
        let text: Vec<String> = rendered.lines.iter().map(ToString::to_string).collect();
        assert_eq!(text, vec!["Hello bold it", "next line", "", "Second"]);
        let bold = rendered.lines[0]
            .spans
            .iter()
            .find(|span| span.content == "bold")
            .unwrap();
        assert!(bold.style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn lists_nest_and_wrap_under_markers() {
        let text = "- first item that wraps around\n- second\n  1. nested\n  2. [x] done\n\nafter";
        assert_eq!(
            plain(text, 20),
            vec![
                "• first item that",
                "  wraps around",
                "• second",
                "  1. nested",
                "  2. [x] done",
                "",
                "after",
            ]
        );
    }

    #[test]
    fn code_blocks_are_highlighted_and_kept_verbatim() {
        let rendered =
            render_markdown("```rust\nlet x = 1;\n    y();\n```\ntext", 40, Style::new());
        let text: Vec<String> = rendered.lines.iter().map(ToString::to_string).collect();
        assert_eq!(text, vec!["▏ let x = 1;", "▏     y();", "", "text"]);
        assert!(
            rendered.lines[0]
                .spans
                .iter()
                .any(|span| matches!(span.style.fg, Some(Color::Rgb(..))))
        );
    }

    #[test]
    fn links_show_url_and_are_collected() {
        let rendered = render_markdown(
            "See [docs](https://example.com/docs) and <https://a.dev> or https://b.dev/x.",
            80,
            Style::new(),
        );
        assert_eq!(
            rendered.lines[0].to_string(),
            "See docs (https://example.com/docs) and https://a.dev or https://b.dev/x."
        );
        assert_eq!(
            rendered.links,
            vec![
                "https://example.com/docs",
                "https://a.dev",
                "https://b.dev/x"
            ]
        );
    }

    #[test]
    fn tables_fit_the_width() {
        let text =
            "| Name | Value |\n|:-----|------:|\n| alpha | 1 |\n| a much longer cell here | 22 |";
        let lines = plain(text, 30);
        assert_eq!(lines[0], format!("┌{}┬{}┐", "─".repeat(20), "─".repeat(7)));
        assert_eq!(lines[1], format!("│ Name{} │ Value │", " ".repeat(14)));
        assert!(lines.iter().all(|line| crate::wrap::text_width(line) <= 30));
        assert!(lines.iter().any(|line| line.contains("│    22 │")));
        assert_eq!(
            lines.last().unwrap(),
            &format!("└{}┴{}┘", "─".repeat(20), "─".repeat(7))
        );
    }

    #[test]
    fn quotes_headings_and_rules() {
        let lines = plain("# Title\n\n> quoted\n> more\n\n---\n", 20);
        assert_eq!(
            lines,
            vec![
                "Title",
                "",
                "│ quoted",
                "│ more",
                "",
                "────────────────────"
            ]
        );
    }

    #[test]
    fn every_line_respects_width() {
        let text = "## Heading with a fairly long title\n\n- item `code span that is long` text\n\n```\nverylonglinewithoutanyspacesatallthatmustbreak\n```\n";
        for width in [8, 13, 21] {
            for line in render_markdown(text, width, Style::new()).lines {
                assert!(line.width() <= width, "{width}: {line}");
            }
        }
    }
}
