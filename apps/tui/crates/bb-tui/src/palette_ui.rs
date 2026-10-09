use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::palette::{Palette, PaletteItem, PaletteKind, PaneBody, ResultPane};
use crate::wrap::truncate_spans;

const MAX_WIDTH: u16 = 100;
const MAX_HEIGHT: u16 = 24;

fn accent() -> Style {
    Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD)
}

fn muted() -> Style {
    Style::new().fg(Color::DarkGray)
}

pub fn highlight_spans(
    text: &str,
    positions: &[usize],
    base: Style,
    accent: Style,
) -> Vec<Span<'static>> {
    let mut sorted = positions.to_vec();
    sorted.sort_unstable();
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut current = String::new();
    let mut current_hit = false;
    for (index, ch) in text.chars().enumerate() {
        let hit = sorted.binary_search(&index).is_ok();
        if hit != current_hit && !current.is_empty() {
            let style = if current_hit { accent } else { base };
            spans.push(Span::styled(std::mem::take(&mut current), style));
        }
        current_hit = hit;
        if ch == '\n' || ch == '\t' {
            current.push(' ');
        } else {
            current.push(ch);
        }
    }
    if !current.is_empty() {
        spans.push(Span::styled(
            current,
            if current_hit { accent } else { base },
        ));
    }
    spans
}

fn item_line(item: &PaletteItem, width: usize, selected: bool) -> Line<'static> {
    let hint_width = item.hint.width();
    let reserve = if hint_width > 0 { hint_width + 2 } else { 0 };
    let mut spans = vec![Span::raw(if selected { "› " } else { "  " })];
    spans.extend(highlight_spans(
        &item.title,
        &item.positions,
        Style::new(),
        accent(),
    ));
    if let Some(detail) = &item.detail {
        spans.push(Span::raw("  "));
        spans.extend(highlight_spans(
            &detail.text,
            &detail.positions,
            muted(),
            Style::new().fg(Color::Yellow),
        ));
    }
    let mut spans = truncate_spans(spans, width.saturating_sub(reserve));
    if hint_width > 0 {
        let used: usize = spans.iter().map(|span| span.content.width()).sum();
        let pad = width.saturating_sub(used + hint_width);
        spans.push(Span::raw(" ".repeat(pad)));
        spans.push(Span::styled(
            item.hint.clone(),
            Style::new().fg(Color::Cyan),
        ));
    }
    let line = Line::from(spans);
    if selected {
        line.style(Style::new().bg(Color::Blue).fg(Color::White))
    } else {
        line
    }
}

fn centered(area: Rect, width: u16, height: u16, top: bool) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect {
        x: area.x + (area.width - width) / 2,
        y: if top {
            area.y + (area.height - height).min(2)
        } else {
            area.y + (area.height - height) / 2
        },
        width,
        height,
    }
}

pub fn render_palette(palette: &Palette, frame: &mut Frame, area: Rect) {
    let width = area
        .width
        .saturating_sub(4)
        .clamp(30.min(area.width), MAX_WIDTH);
    let height = area
        .height
        .saturating_sub(2)
        .clamp(6.min(area.height), MAX_HEIGHT);
    let rect = centered(area, width, height, true);
    let title = match (&palette.kind, &palette.plugin) {
        (PaletteKind::PluginArgs, Some(plugin)) => format!(" bb {} ", plugin.label),
        (kind, _) => format!(" {} ", kind.title()),
    };
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(Color::Cyan))
        .title(Span::styled(
            title,
            Style::new().add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(rect);
    frame.render_widget(Clear, rect);
    frame.render_widget(block, rect);
    if inner.height < 2 || inner.width < 4 {
        return;
    }
    let width = usize::from(inner.width);
    let prompt = if palette.kind == PaletteKind::PluginArgs {
        "$ "
    } else {
        "› "
    };
    let input_line = if palette.query().is_empty() {
        Line::from(vec![
            Span::styled(prompt, Style::new().fg(Color::Cyan)),
            Span::styled(palette.kind.placeholder(), muted()),
        ])
    } else {
        Line::from(vec![
            Span::styled(prompt, Style::new().fg(Color::Cyan)),
            Span::raw(palette.query().to_owned()),
        ])
    };
    frame.render_widget(
        Paragraph::new(Line::from(truncate_spans(input_line.spans, width))),
        Rect { height: 1, ..inner },
    );
    let cursor_x = inner.x + 2 + palette.input.cursor_width() as u16;
    if cursor_x < inner.right() {
        frame.set_cursor_position((cursor_x, inner.y));
    }
    let mut list = Rect {
        y: inner.y + 1,
        height: inner.height - 1,
        ..inner
    };
    if let Some((notice, error)) = palette.notice() {
        if list.height == 0 {
            return;
        }
        let style = if error {
            Style::new().fg(Color::Red)
        } else {
            muted()
        };
        let notice_area = Rect {
            y: list.bottom() - 1,
            height: 1,
            ..list
        };
        frame.render_widget(
            Paragraph::new(Line::from(truncate_spans(
                vec![Span::styled(notice, style)],
                width,
            ))),
            notice_area,
        );
        list.height -= 1;
    }
    if palette.kind == PaletteKind::PluginArgs {
        if let Some(plugin) = &palette.plugin
            && list.height > 0
            && !plugin.summary.is_empty()
        {
            frame.render_widget(
                Paragraph::new(Line::from(truncate_spans(
                    vec![Span::styled(plugin.summary.clone(), muted())],
                    width,
                ))),
                Rect { height: 1, ..list },
            );
        }
        return;
    }
    let mut rows: Vec<(Option<usize>, Line<'static>)> = Vec::new();
    let mut previous_group: Option<&str> = None;
    for (index, item) in palette.items.iter().enumerate() {
        if previous_group != Some(item.group.as_str()) {
            rows.push((
                None,
                Line::from(Span::styled(
                    item.group.clone(),
                    muted().add_modifier(Modifier::BOLD),
                )),
            ));
            previous_group = Some(item.group.as_str());
        }
        rows.push((
            Some(index),
            item_line(item, width, index == palette.selected),
        ));
    }
    let height = usize::from(list.height);
    if height == 0 {
        return;
    }
    let selected_row = rows
        .iter()
        .position(|(index, _)| *index == Some(palette.selected))
        .unwrap_or(0);
    let mut offset = palette.offset.get();
    if selected_row < offset + 1 {
        offset = selected_row.saturating_sub(1);
    } else if selected_row >= offset + height {
        offset = selected_row + 1 - height;
    }
    offset = offset.min(rows.len().saturating_sub(height));
    palette.offset.set(offset);
    let lines: Vec<Line<'static>> = rows
        .into_iter()
        .skip(offset)
        .take(height)
        .map(|(_, line)| line)
        .collect();
    frame.render_widget(Paragraph::new(lines), list);
}

fn slice_spans(spans: &[Span<'static>], skip: usize, width: usize) -> Vec<Span<'static>> {
    let mut out = Vec::new();
    let mut column = 0;
    let mut used = 0;
    for span in spans {
        let mut text = String::new();
        for ch in span.content.chars() {
            let ch_width = ch.width().unwrap_or(0);
            if column < skip {
                column += ch_width;
                continue;
            }
            if used + ch_width > width {
                break;
            }
            text.push(ch);
            used += ch_width;
            column += ch_width;
        }
        if !text.is_empty() {
            out.push(Span::styled(text, span.style));
        }
        if used >= width {
            break;
        }
    }
    out
}

pub fn render_pane(pane: &ResultPane, frame: &mut Frame, area: Rect) {
    let rect = Rect {
        x: area.x + 1.min(area.width),
        y: area.y + 1.min(area.height),
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    };
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(Color::Cyan))
        .title(Span::styled(
            format!(" {} ", pane.title),
            Style::new().add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(rect);
    frame.render_widget(Clear, rect);
    frame.render_widget(block, rect);
    if inner.height < 2 || inner.width < 4 {
        return;
    }
    let width = usize::from(inner.width);
    let body = Rect {
        height: inner.height - 1,
        ..inner
    };
    let height = usize::from(body.height);
    pane.viewport.set(height);
    let total = pane.line_count();
    let scroll = pane.scroll.min(total.saturating_sub(1));
    match &pane.body {
        PaneBody::Loading => frame.render_widget(Paragraph::new("Loading…").style(muted()), body),
        PaneBody::Error(error) => frame.render_widget(
            Paragraph::new(error.clone()).style(Style::new().fg(Color::Red)),
            body,
        ),
        PaneBody::Lines(lines) => {
            let rendered: Vec<Line<'static>> = lines
                .iter()
                .skip(scroll)
                .take(height)
                .map(|spans| Line::from(slice_spans(spans, pane.column, width)))
                .collect();
            frame.render_widget(Paragraph::new(rendered), body);
        }
    }
    let position = if total == 0 {
        String::new()
    } else {
        format!(
            " {}-{}/{} ",
            scroll + 1,
            (scroll + height).min(total),
            total
        )
    };
    let (footer, style) = pane
        .footer
        .clone()
        .unwrap_or_else(|| (String::new(), muted()));
    let mut keys = String::from("j/k scroll · h/l pan · y copy");
    if pane.open_target.is_some() {
        keys.push_str(" · o open");
    }
    keys.push_str(" · Esc close");
    let left = if footer.is_empty() {
        vec![Span::styled(keys, muted())]
    } else {
        vec![
            Span::styled(footer, style),
            Span::styled(format!("  {keys}"), muted()),
        ]
    };
    let mut spans = truncate_spans(left, width.saturating_sub(position.width()));
    let used: usize = spans.iter().map(|span| span.content.width()).sum();
    spans.push(Span::raw(
        " ".repeat(width.saturating_sub(used + position.width())),
    ));
    spans.push(Span::styled(position, Style::new().fg(Color::Cyan)));
    frame.render_widget(
        Paragraph::new(Line::from(spans)),
        Rect {
            y: inner.bottom() - 1,
            height: 1,
            ..inner
        },
    );
}

pub fn hints(palette: Option<&Palette>, pane: Option<&ResultPane>) -> Option<String> {
    if let Some(pane) = pane {
        return Some(if pane.open_target.is_some() {
            "j/k scroll · h/l pan · y copy · o open · Esc close".into()
        } else {
            "j/k scroll · h/l pan · y copy · Esc close".into()
        });
    }
    let palette = palette?;
    Some(match palette.kind {
        PaletteKind::PluginArgs => "Enter run · Backspace back · Esc cancel".into(),
        _ if palette.kind != palette.origin => {
            "type to filter · ↑/↓ move · Enter open · Backspace back · Esc close".into()
        }
        _ => "type to filter · ↑/↓ move · Enter open · Esc close".into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn highlight_marks_positions_by_char() {
        let spans = highlight_spans("añb c", &[1, 3], Style::new(), accent());
        let texts: Vec<&str> = spans.iter().map(|span| span.content.as_ref()).collect();
        assert_eq!(texts, ["a", "ñ", "b", " ", "c"]);
        assert_eq!(spans[1].style, accent());
        assert_eq!(spans[3].style, accent());
    }

    #[test]
    fn slice_spans_skips_columns_across_spans() {
        let spans = vec![Span::raw("abc"), Span::raw("界def")];
        let sliced = slice_spans(&spans, 2, 4);
        let text: String = sliced.iter().map(|span| span.content.as_ref()).collect();
        assert_eq!(text, "c界d");
    }
}
