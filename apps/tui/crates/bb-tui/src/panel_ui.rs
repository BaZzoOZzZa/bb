use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph, Wrap};
use unicode_width::UnicodeWidthStr;

use crate::app::{AppState, Focus};
use crate::execution::Load;
use crate::git_diff::{DiffDocument, DiffRow, LineKind, PatchState};
use crate::palette_ui::slice_spans;
use crate::panel::{
    GitAction, GitConfirm, InfoEntry, MERGE_METHODS, PanelTab, StatusView, change_style,
};
use crate::rbuf::Buf;

const MUTED: Style = Style::new().fg(Color::DarkGray);
const CURSOR: Style = Style::new().bg(Color::DarkGray);
const ADDED_BG: Color = Color::Rgb(18, 48, 24);
const REMOVED_BG: Color = Color::Rgb(56, 20, 22);
const LABEL_WIDTH: usize = 13;

fn tab_title(state: &AppState, focused: bool) -> Line<'static> {
    let mut spans = vec![Span::raw(" ")];
    for (index, tab) in PanelTab::ALL.iter().enumerate() {
        if index > 0 {
            spans.push(Span::styled(" │ ", MUTED));
        }
        let label = format!("{} {}", index + 1, tab.title());
        let style = if *tab == state.panel.tab {
            let style = Style::new().add_modifier(Modifier::BOLD);
            if focused {
                style.fg(Color::Cyan)
            } else {
                style
            }
        } else {
            MUTED
        };
        spans.push(Span::styled(label, style));
    }
    if state.panel.maximized {
        spans.push(Span::styled(" · max", MUTED));
    }
    spans.push(Span::raw(" "));
    Line::from(spans)
}

pub fn render_panel(state: &AppState, frame: &mut Frame, area: Rect) {
    let focused = state.focus == Focus::Panel;
    let border = if focused {
        Style::new().fg(Color::Cyan)
    } else {
        Style::new().fg(Color::DarkGray)
    };
    let block = Block::bordered()
        .border_type(if focused {
            BorderType::Thick
        } else {
            BorderType::Plain
        })
        .border_style(border)
        .title(tab_title(state, focused));
    let inner = block.inner(area);
    frame.render_widget(Clear, area);
    frame.render_widget(block, area);
    if inner.height == 0 || inner.width < 4 {
        return;
    }
    if state.panel_thread().is_none() {
        frame.render_widget(
            Paragraph::new("Open a thread to see its details.").style(MUTED),
            inner,
        );
        return;
    }
    match state.panel.tab {
        PanelTab::Info => render_info(state, frame, inner, focused),
        PanelTab::Diff => render_diff(state, frame, inner, focused),
        PanelTab::File => render_file(state, frame, inner, focused),
        PanelTab::Terminal => crate::terminal_ui::render_terminal(state, frame, inner, focused),
    }
}

fn split_footer(area: Rect) -> (Rect, Rect) {
    if area.height < 3 {
        return (area, Rect { height: 0, ..area });
    }
    let [body, footer] = Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(area);
    (body, footer)
}

fn footer(frame: &mut Frame, area: Rect, text: String) {
    if area.height == 0 {
        return;
    }
    frame.render_widget(
        Paragraph::new(crate::ui::truncate(&text, usize::from(area.width))).style(MUTED),
        area,
    );
}

fn action_hint(state: &AppState) -> Option<String> {
    let actions = state.available_git_actions();
    if actions.is_empty() {
        return None;
    }
    Some(
        actions
            .iter()
            .map(|action| match action {
                GitAction::Commit => "SPC g c c commit",
                GitAction::Merge => "SPC g p m merge PR",
                GitAction::MarkReady => "SPC g p r ready PR",
                GitAction::ConvertToDraft => "SPC g p d draft PR",
            })
            .collect::<Vec<_>>()
            .join(" · "),
    )
}

fn info_line(entry: &InfoEntry, selected: bool, width: usize) -> Line<'static> {
    let mut spans = Vec::new();
    if let Some(label) = &entry.label {
        let padded = format!("{label:<LABEL_WIDTH$}");
        spans.push(Span::styled(padded, MUTED));
    } else if entry.target.is_some() {
        spans.push(Span::raw("  "));
    }
    spans.extend(entry.spans.iter().cloned());
    if entry.heading {
        for span in &mut spans {
            span.style = span.style.add_modifier(Modifier::BOLD);
        }
    }
    let mut spans = slice_spans(&spans, 0, width);
    if selected {
        let used: usize = spans.iter().map(|span| span.content.width()).sum();
        if used < width {
            spans.push(Span::raw(" ".repeat(width - used)));
        }
        for span in &mut spans {
            span.style = span.style.patch(CURSOR);
        }
    }
    Line::from(spans)
}

fn render_info(state: &AppState, frame: &mut Frame, area: Rect, focused: bool) {
    let (body, footer_area) = split_footer(area);
    let entries = state.info_entries();
    let height = usize::from(body.height).max(1);
    state.panel.info_viewport.set(height);
    let cursor = state.panel.info_cursor.min(entries.len().saturating_sub(1));
    let mut scroll = state.panel.info_scroll.min(entries.len().saturating_sub(1));
    if cursor < scroll {
        scroll = cursor;
    } else if cursor >= scroll + height {
        scroll = cursor + 1 - height;
    }
    let width = usize::from(body.width);
    let lines: Vec<Line<'static>> = entries
        .iter()
        .enumerate()
        .skip(scroll)
        .take(height)
        .map(|(index, entry)| info_line(entry, focused && index == cursor, width))
        .collect();
    frame.render_widget(Paragraph::new(lines), body);
    let mut hint = vec!["Enter open".to_owned(), "p preview".into(), "y copy".into()];
    if let Some(actions) = action_hint(state) {
        hint.insert(0, actions);
    }
    footer(frame, footer_area, hint.join(" · "));
}

fn diff_unavailable(state: &AppState) -> Option<String> {
    if state.panel.environment_id.is_none() {
        return Some("This thread has no environment.".into());
    }
    match &state.panel.status {
        Load::Loaded(StatusView::NotApplicable(message)) => return Some(message.clone()),
        Load::Loaded(StatusView::Unavailable(message)) if state.panel.document().is_none() => {
            return Some(message.clone());
        }
        _ => {}
    }
    match &state.panel.diff {
        Load::Failed(error) => Some(error.clone()),
        Load::Idle | Load::Loading => Some("Loading diff…".into()),
        Load::Loaded(_) => None,
    }
}

fn scope_line(state: &AppState, document: Option<&DiffDocument>) -> Line<'static> {
    let mut spans = vec![Span::styled(
        state.panel.scope.label(),
        Style::new().add_modifier(Modifier::BOLD),
    )];
    if let Some(base) = state.panel_merge_base() {
        spans.push(Span::styled(format!(" vs {base}"), MUTED));
    }
    if let Some(document) = document {
        let (added, removed) = document.files.iter().fold((0, 0), |(a, d), file| {
            (a + file.entry.additions, d + file.entry.deletions)
        });
        spans.push(Span::styled(
            format!(
                " · {} file{} ",
                document.files.len(),
                if document.files.len() == 1 { "" } else { "s" }
            ),
            MUTED,
        ));
        spans.push(Span::styled(
            format!("+{added}"),
            Style::new().fg(Color::Green),
        ));
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            format!("-{removed}"),
            Style::new().fg(Color::Red),
        ));
        if document.truncated {
            spans.push(Span::styled(
                " · list truncated",
                Style::new().fg(Color::Yellow),
            ));
        }
    }
    Line::from(spans)
}

pub(crate) fn diff_row_text(document: &DiffDocument, index: usize) -> (String, usize, bool) {
    let plain = |spans: &[Span<'static>]| -> String {
        spans.iter().map(|span| span.content.as_ref()).collect()
    };
    match document.row(index) {
        Some(DiffRow::Header(file)) => (plain(&header_line(document, file, 10_000)), 0, false),
        Some(DiffRow::Line(file, line)) => {
            let number_width = document.number_width(file);
            match document.line(file, line) {
                Some(diff_line) if diff_line.kind == LineKind::Hunk => {
                    (diff_line.text(), number_width * 2 + 2, true)
                }
                Some(diff_line) => (diff_line.text(), number_width * 2 + 4, true),
                None => (String::new(), 0, false),
            }
        }
        Some(DiffRow::Notice(file)) => (
            format!("    {}", document.files[file].notice().unwrap_or_default()),
            0,
            false,
        ),
        None => (String::new(), 0, false),
    }
}

fn header_line(document: &DiffDocument, file: usize, width: usize) -> Vec<Span<'static>> {
    let diff_file = &document.files[file];
    let marker = if diff_file.collapsed { "▸ " } else { "▾ " };
    let letter = diff_file.status_letter();
    let mut spans = vec![
        Span::styled(marker, MUTED),
        Span::styled(format!("{letter} "), change_style(letter)),
    ];
    let path = match &diff_file.entry.previous_path {
        Some(previous) if previous != &diff_file.entry.path => {
            format!("{previous} → {}", diff_file.entry.path)
        }
        _ => diff_file.entry.path.clone(),
    };
    spans.push(Span::styled(
        path,
        Style::new().add_modifier(Modifier::BOLD),
    ));
    if diff_file.entry.binary {
        spans.push(Span::styled(" binary", MUTED));
    } else {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            format!("+{}", diff_file.entry.additions),
            Style::new().fg(Color::Green),
        ));
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            format!("-{}", diff_file.entry.deletions),
            Style::new().fg(Color::Red),
        ));
    }
    if matches!(diff_file.patch, PatchState::Loading) && !diff_file.collapsed {
        spans.push(Span::styled(" …", MUTED));
    }
    slice_spans(&spans, 0, width)
}

fn code_line(
    document: &DiffDocument,
    file: usize,
    line: usize,
    width: usize,
    selected: bool,
) -> Vec<Span<'static>> {
    let Some(diff_line) = document.line(file, line) else {
        return Vec::new();
    };
    let number_width = document.number_width(file);
    let gutter_style = if selected {
        CURSOR.fg(Color::White)
    } else {
        MUTED
    };
    if diff_line.kind == LineKind::Hunk {
        let mut spans = vec![Span::styled(" ".repeat(number_width * 2 + 2), gutter_style)];
        let code_width = width.saturating_sub(number_width * 2 + 2);
        spans.extend(slice_spans(&diff_line.spans, document.column, code_width));
        return spans;
    }
    let number = |value: Option<u32>| {
        value
            .map(|value| format!("{value:>number_width$}"))
            .unwrap_or_else(|| " ".repeat(number_width))
    };
    let (sign, sign_style, background) = match diff_line.kind {
        LineKind::Added => ("+", Style::new().fg(Color::Green), Some(ADDED_BG)),
        LineKind::Removed => ("-", Style::new().fg(Color::Red), Some(REMOVED_BG)),
        LineKind::NoNewline => (" ", MUTED, None),
        _ => (" ", MUTED, None),
    };
    let mut spans = vec![
        Span::styled(
            format!("{} {} ", number(diff_line.old), number(diff_line.new)),
            gutter_style,
        ),
        Span::styled(sign, sign_style),
        Span::raw(" "),
    ];
    let code_width = width.saturating_sub(number_width * 2 + 4);
    let mut code = slice_spans(&diff_line.spans, document.column, code_width);
    if let Some(background) = background {
        let used: usize = code.iter().map(|span| span.content.width()).sum();
        if used < code_width {
            code.push(Span::raw(" ".repeat(code_width - used)));
        }
        for span in &mut code {
            span.style = span.style.bg(background);
        }
    }
    spans.extend(code);
    spans
}

fn render_diff(state: &AppState, frame: &mut Frame, area: Rect, focused: bool) {
    let (body, footer_area) = split_footer(area);
    let document = state.panel.document();
    let [scope_area, rows_area] =
        Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).areas(body);
    frame.render_widget(Paragraph::new(scope_line(state, document)), scope_area);
    let hint =
        "s scope · RET fold/load · ]c/[c hunk · ]f/[f file · zR/zM all · gf preview · e edit";
    let hint = match action_hint(state) {
        Some(actions) => format!("{actions} · {hint}"),
        None => hint.to_owned(),
    };
    footer(frame, footer_area, hint);
    let Some(document) = document.filter(|_| diff_unavailable(state).is_none()) else {
        frame.render_widget(
            Paragraph::new(diff_unavailable(state).unwrap_or_default())
                .style(MUTED)
                .wrap(Wrap { trim: false }),
            rows_area,
        );
        return;
    };
    if document.files.is_empty() {
        frame.render_widget(Paragraph::new("No changes.").style(MUTED), rows_area);
        return;
    }
    let height = usize::from(rows_area.height).max(1);
    document.viewport.set(height);
    let width = usize::from(rows_area.width);
    let total = document.row_count();
    let scroll = document.scroll.min(total.saturating_sub(1));
    let lines: Vec<Line<'static>> = (scroll..(scroll + height).min(total))
        .map(|index| {
            let selected = focused && index == document.cursor;
            let mut spans = match document.row(index) {
                Some(DiffRow::Header(file)) => header_line(document, file, width),
                Some(DiffRow::Line(file, line)) => code_line(document, file, line, width, selected),
                Some(DiffRow::Notice(file)) => {
                    let notice = document.files[file].notice().unwrap_or_default();
                    let style = if matches!(document.files[file].patch, PatchState::Failed(_)) {
                        Style::new().fg(Color::Red)
                    } else {
                        Style::new().fg(Color::Yellow)
                    };
                    slice_spans(&[Span::raw("    "), Span::styled(notice, style)], 0, width)
                }
                None => Vec::new(),
            };
            if selected && !matches!(document.row(index), Some(DiffRow::Line(..))) {
                let used: usize = spans.iter().map(|span| span.content.width()).sum();
                if used < width {
                    spans.push(Span::raw(" ".repeat(width - used)));
                }
                for span in &mut spans {
                    span.style = span.style.patch(CURSOR);
                }
            }
            Line::from(spans)
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), rows_area);
    let gutter = document
        .cursor_file()
        .map_or(0, |file| document.number_width(file) * 2 + 4);
    crate::rbuf::note_width(state, Buf::Diff, width.saturating_sub(gutter));
    let paint = crate::rbuf::paint(state, Buf::Diff);
    let buffer = frame.buffer_mut();
    for (row, index) in (scroll..(scroll + height).min(total)).enumerate() {
        let (text, prefix, panned) = diff_row_text(document, index);
        let prefix = prefix.min(width) as u16;
        let rect = Rect {
            x: rows_area.x + prefix,
            y: rows_area.y + row as u16,
            width: rows_area.width - prefix,
            height: 1,
        };
        let hscroll = if panned { document.column } else { 0 };
        paint.overlay(buffer, index, &text, rect, hscroll, focused);
    }
}

fn render_file(state: &AppState, frame: &mut Frame, area: Rect, focused: bool) {
    let Some(preview) = &state.panel.preview else {
        frame.render_widget(
            Paragraph::new("No file open. Press p on a changed file in Info or Diff.").style(MUTED),
            area,
        );
        return;
    };
    let (body, footer_area) = split_footer(area);
    let [title_area, text_area] =
        Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).areas(body);
    let mut title = vec![Span::styled(
        preview.label.clone(),
        Style::new().add_modifier(Modifier::BOLD),
    )];
    if let Some(line) = preview.line {
        title.push(Span::styled(format!(" · line {line}"), MUTED));
    }
    frame.render_widget(
        Paragraph::new(Line::from(slice_spans(
            &title,
            0,
            usize::from(title_area.width),
        ))),
        title_area,
    );
    let mut hint = "j/k move · / search · zh/zl pan · e edit · gy yank path · q back".to_owned();
    if let Some(note) = &preview.note {
        hint = format!("{note} · {hint}");
    }
    footer(frame, footer_area, hint);
    let height = usize::from(text_area.height).max(1);
    preview.viewport.set(height);
    match &preview.body {
        Load::Idle | Load::Loading => {
            frame.render_widget(Paragraph::new("Loading…").style(MUTED), text_area);
        }
        Load::Failed(error) => frame.render_widget(
            Paragraph::new(error.clone())
                .style(Style::new().fg(Color::Red))
                .wrap(Wrap { trim: false }),
            text_area,
        ),
        Load::Loaded(lines) => {
            let total = lines.len();
            let number_width = total.max(1).to_string().len();
            let width = usize::from(text_area.width);
            let code_width = width.saturating_sub(number_width + 1);
            let scroll = preview.scroll.min(total.saturating_sub(1));
            let rendered: Vec<Line<'static>> = lines
                .iter()
                .enumerate()
                .skip(scroll)
                .take(height)
                .map(|(index, spans)| {
                    let current = preview.line == u32::try_from(index + 1).ok();
                    let gutter_style = if current {
                        Style::new().fg(Color::Yellow)
                    } else {
                        MUTED
                    };
                    let mut out = vec![Span::styled(
                        format!("{:>number_width$} ", index + 1),
                        gutter_style,
                    )];
                    out.extend(slice_spans(spans, preview.column, code_width));
                    Line::from(out)
                })
                .collect();
            frame.render_widget(Paragraph::new(rendered), text_area);
            crate::rbuf::note_width(state, Buf::File, code_width);
            let paint = crate::rbuf::paint(state, Buf::File);
            let buffer = frame.buffer_mut();
            for (row, (index, spans)) in lines
                .iter()
                .enumerate()
                .skip(scroll)
                .take(height)
                .enumerate()
            {
                let y = text_area.y + row as u16;
                if focused && paint.cursor_line() == Some(index) {
                    buffer.set_style(
                        Rect {
                            x: text_area.x,
                            y,
                            width: text_area.width,
                            height: 1,
                        },
                        crate::rbuf::CURSOR_LINE,
                    );
                }
                let text: String = spans.iter().map(|span| span.content.as_ref()).collect();
                let gutter = (number_width + 1).min(width) as u16;
                let rect = Rect {
                    x: text_area.x + gutter,
                    y,
                    width: text_area.width - gutter,
                    height: 1,
                };
                paint.overlay(buffer, index, &text, rect, preview.column, focused);
            }
        }
    }
}

pub fn render_confirm(confirm: &GitConfirm, frame: &mut Frame, area: Rect) {
    let mut lines: Vec<Line<'static>> = confirm
        .lines
        .iter()
        .map(|line| Line::from(line.clone()))
        .collect();
    if confirm.action == GitAction::Merge {
        lines.push(Line::default());
        let mut spans = vec![Span::styled("Method: ", MUTED)];
        for (index, method) in MERGE_METHODS.iter().enumerate() {
            let label = format!(" {} {} ", index + 1, method.as_str());
            let style = if index == confirm.method {
                Style::new()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::new()
            };
            spans.push(Span::styled(label, style));
            spans.push(Span::raw(" "));
        }
        lines.push(Line::from(spans));
    }
    lines.push(Line::default());
    let mut keys = "y/Enter confirm · Esc cancel".to_owned();
    if confirm.action == GitAction::Merge {
        keys.push_str(" · ←/→ method");
    }
    lines.push(Line::from(Span::styled(keys, MUTED)));
    let content_width = lines
        .iter()
        .map(|line| line.width())
        .chain(std::iter::once(confirm.action.title().width() + 4))
        .max()
        .unwrap_or(20);
    let width = (content_width as u16 + 4).clamp(30, area.width.max(1));
    let height = (lines.len() as u16 + 2).min(area.height.max(1));
    let rect = Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    };
    frame.render_widget(Clear, rect);
    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }).block(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .border_style(Style::new().fg(Color::Yellow))
                .title(Span::styled(
                    format!(" {} ", confirm.action.title()),
                    Style::new().add_modifier(Modifier::BOLD),
                )),
        ),
        rect,
    );
}

pub fn hints(state: &AppState) -> String {
    match state.panel.tab {
        PanelTab::Info => {
            "j/k move · RET open diff/link · gf preview · e edit · yy yank · gt/gT tab · C-w o maximize · q leave".into()
        }
        PanelTab::Diff => {
            "s/S scope · ]c/[c hunk · ]f/[f file · RET fold · / search · v/V select · y yank · gf preview · e edit · q leave".into()
        }
        PanelTab::File => "j/k move · / search · v/V select · y yank · zh/zl pan · e edit · gy yank path · q back".into(),
        PanelTab::Terminal => {
            "i type · o new · dd close · ]t/[t switch · SPC o f fullscreen · j/k/C-u scrollback · / search · yy yank · q leave".into()
        }
    }
}
