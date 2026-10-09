use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph, Wrap};
use unicode_width::UnicodeWidthStr;

use crate::app::AppState;
use crate::automations::{
    AutomationForm, AutomationRow, AutomationsView, DeleteConfirm, FormChoice, FormEdit, FormField,
    RowKind, Tone, automation_rows, clamp_cursor, form_value,
};
use crate::ui::truncate;
use crate::wrap::{WrapMode, wrap_plain};

const MUTED: Style = Style::new().fg(Color::DarkGray);
const FIELD_LABEL: usize = 18;
const FORM_LABEL: usize = 16;

fn tone_style(tone: Tone) -> Style {
    match tone {
        Tone::Plain => Style::new(),
        Tone::Muted => MUTED,
        Tone::Good => Style::new().fg(Color::Green),
        Tone::Warn => Style::new().fg(Color::Yellow),
        Tone::Bad => Style::new().fg(Color::Red),
        Tone::Accent => Style::new().fg(Color::Cyan),
    }
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    }
}

fn pad(text: &str, width: usize) -> String {
    let text = truncate(text, width);
    let fill = width.saturating_sub(text.width());
    format!("{text}{}", " ".repeat(fill))
}

fn badge_spans(badges: &[(String, Tone)]) -> Vec<Span<'static>> {
    badges
        .iter()
        .flat_map(|(text, tone)| {
            [
                Span::raw(" "),
                Span::styled(format!("[{text}]"), tone_style(*tone)),
            ]
        })
        .collect()
}

fn row_lines(
    row: &AutomationRow,
    width: usize,
    label_width: usize,
    selected: bool,
) -> Vec<Line<'static>> {
    let base = if selected {
        Style::new().add_modifier(Modifier::REVERSED)
    } else {
        Style::new()
    };
    let marker = Span::styled(if selected { "› " } else { "  " }, base);
    match row.kind {
        RowKind::Header => {
            let mut spans = Vec::new();
            if let Some((glyph, tone)) = &row.glyph {
                spans.push(Span::styled(format!("{glyph} "), tone_style(*tone)));
            }
            spans.push(Span::styled(
                truncate(&row.label, width.saturating_sub(4)),
                Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            ));
            spans.extend(badge_spans(&row.badges));
            vec![Line::from(crate::wrap::truncate_spans(spans, width))]
        }
        RowKind::Note => vec![Line::from(Span::styled(
            truncate(&format!("  {}", row.label), width),
            MUTED,
        ))],
        RowKind::Text | RowKind::Error => {
            let style = if row.kind == RowKind::Error {
                Style::new().fg(Color::Red)
            } else {
                Style::new()
            };
            let text = if row.label.is_empty() {
                " "
            } else {
                &row.label
            };
            let mut lines = wrap_plain(
                text,
                style,
                width.saturating_sub(4).max(1),
                &[],
                WrapMode::Words,
            );
            for (index, line) in lines.iter_mut().enumerate() {
                let lead = if index == 0 {
                    marker.clone()
                } else {
                    Span::raw("  ")
                };
                line.spans.insert(0, Span::raw("  "));
                line.spans.insert(0, lead);
            }
            if selected && let Some(first) = lines.first_mut() {
                first.spans.iter_mut().skip(2).for_each(|span| {
                    span.style = span.style.add_modifier(Modifier::REVERSED);
                });
            }
            lines
        }
        RowKind::Field => {
            let label = pad(&row.label, FIELD_LABEL.min(width / 2));
            let used = 2 + label.width() + 2;
            let mut spans = vec![
                marker,
                Span::styled(label, base.fg(Color::DarkGray)),
                Span::styled("  ", base),
                Span::styled(
                    truncate(&row.value, width.saturating_sub(used)),
                    if row.target.is_some() {
                        base.fg(Color::Cyan)
                    } else {
                        base
                    },
                ),
            ];
            spans.extend(badge_spans(&row.badges));
            vec![Line::from(crate::wrap::truncate_spans(spans, width))]
        }
        RowKind::Item => {
            let mut spans = vec![marker];
            match &row.glyph {
                Some((glyph, tone)) => {
                    spans.push(Span::styled(format!("{glyph} "), tone_style(*tone)));
                }
                None => spans.push(Span::raw("  ")),
            }
            let label = pad(&row.label, label_width.min(width.saturating_sub(6)));
            let used = 4 + label.width() + 2;
            spans.push(Span::styled(label, base.add_modifier(Modifier::BOLD)));
            spans.push(Span::styled("  ", base));
            let badges = badge_spans(&row.badges);
            let badge_width: usize = badges.iter().map(|span| span.content.width()).sum();
            spans.push(Span::styled(
                truncate(&row.value, width.saturating_sub(used + badge_width)),
                if selected { base } else { MUTED },
            ));
            spans.extend(badges);
            vec![Line::from(crate::wrap::truncate_spans(spans, width))]
        }
    }
}

pub fn render_automations(state: &AppState, view: &AutomationsView, frame: &mut Frame, area: Rect) {
    frame.render_widget(Clear, area);
    let focused = view.form.is_none() && view.confirm.is_none() && view.filter_input.is_none();
    let title = match &view.detail {
        Some(detail) => format!(
            " Automations › {} ",
            detail
                .automation
                .loaded()
                .map_or("…", |automation| automation.name.as_str())
        ),
        None => {
            let count = view.overview.loaded().map_or(0, Vec::len);
            let filter = view.filter_text();
            if filter.is_empty() {
                format!(" Automations ({count}) ")
            } else {
                format!(" Automations ({count}) · /{filter} ")
            }
        }
    };
    let block = Block::bordered()
        .border_type(if focused {
            BorderType::Thick
        } else {
            BorderType::Plain
        })
        .border_style(if focused {
            Style::new().fg(Color::Cyan)
        } else {
            MUTED
        })
        .title(Span::styled(
            title,
            Style::new().add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let [list_area, footer] = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(u16::from(view.filter_input.is_some())),
    ])
    .areas(inner);
    let width = usize::from(list_area.width);
    let rows = automation_rows(state);
    let raw_cursor = view
        .detail
        .as_ref()
        .map_or(view.cursor, |detail| detail.cursor);
    let cursor = clamp_cursor(&rows, raw_cursor);
    let label_width = rows
        .iter()
        .filter(|row| row.kind == RowKind::Item)
        .map(|row| row.label.width())
        .max()
        .unwrap_or(12)
        .clamp(12, (width / 3).max(12));
    let mut lines: Vec<Line> = Vec::new();
    let mut cursor_span = (0, 0);
    for (index, row) in rows.iter().enumerate() {
        let selected = focused && Some(index) == cursor;
        let start = lines.len();
        lines.extend(row_lines(row, width, label_width, selected));
        if Some(index) == cursor {
            cursor_span = (start, lines.len());
        }
    }
    let height = usize::from(list_area.height).max(1);
    let offset_cell = match &view.detail {
        Some(detail) => &detail.offset,
        None => &view.offset,
    };
    let mut offset = offset_cell.get();
    if cursor.is_some() {
        if cursor_span.0 < offset {
            offset = cursor_span.0.saturating_sub(1);
        } else if cursor_span.1 > offset + height {
            offset = cursor_span.1 - height;
        }
    }
    offset = offset.min(lines.len().saturating_sub(height));
    offset_cell.set(offset);
    let visible: Vec<Line> = lines.into_iter().skip(offset).take(height).collect();
    frame.render_widget(Paragraph::new(visible), list_area);
    if let Some(input) = &view.filter_input {
        let text = format!("/{}", input.text());
        frame.render_widget(
            Paragraph::new(Span::styled(
                truncate(&text, width),
                Style::new().fg(Color::Yellow),
            )),
            footer,
        );
        let x = footer.x + 1 + input.cursor_width() as u16;
        if x < footer.x + footer.width {
            frame.set_cursor_position((x, footer.y));
        }
    }
    if let Some(form) = &view.form {
        render_form(state, form, frame, area);
    }
    if let Some(confirm) = &view.confirm {
        render_confirm(confirm, frame, area);
    }
}

fn render_form(state: &AppState, form: &AutomationForm, frame: &mut Frame, area: Rect) {
    let fields = form.fields();
    let width = area.width.saturating_sub(4).clamp(30, 100);
    let inner_width = usize::from(width.saturating_sub(2));
    let cursor = form.cursor.min(fields.len() - 1);
    let active = form.edit.is_none() && form.choice.is_none();
    let mut lines: Vec<Line> = Vec::new();
    for (index, field) in fields.iter().enumerate() {
        let selected = active && index == cursor;
        let base = if selected {
            Style::new().add_modifier(Modifier::REVERSED)
        } else {
            Style::new()
        };
        let marker = if index == cursor { "› " } else { "  " };
        let value = form_value(state, form, *field);
        if *field == FormField::Save {
            lines.push(Line::default());
            let text = pad(&value, value.width());
            let lead = inner_width.saturating_sub(text.width()) / 2;
            lines.push(Line::from(vec![
                Span::raw(" ".repeat(lead)),
                Span::styled(text, base.fg(Color::Green).add_modifier(Modifier::BOLD)),
            ]));
            continue;
        }
        let locked = *field == FormField::Project && !form.is_create();
        let label = pad(field.label(), FORM_LABEL);
        let used = 2 + FORM_LABEL + 2;
        let value_style = if value.is_empty() {
            MUTED
        } else if locked {
            base.fg(Color::DarkGray)
        } else {
            base
        };
        let shown = if value.is_empty() {
            "(required)".to_owned()
        } else {
            value
        };
        lines.push(Line::from(vec![
            Span::styled(marker, Style::new().fg(Color::Cyan)),
            Span::styled(label, Style::new().fg(Color::DarkGray)),
            Span::raw("  "),
            Span::styled(
                truncate(&shown, inner_width.saturating_sub(used)),
                value_style,
            ),
        ]));
    }
    lines.push(Line::default());
    if let Some(error) = &form.error {
        lines.extend(wrap_plain(
            error,
            Style::new().fg(Color::Red),
            inner_width.max(1),
            &[],
            WrapMode::Words,
        ));
    }
    lines.push(Line::styled(
        truncate(
            "j/k field · Enter edit · C-s save · Esc discard",
            inner_width,
        ),
        MUTED,
    ));
    let title = if form.is_create() {
        " New automation ".to_owned()
    } else {
        format!(" Edit {} ", form.name)
    };
    let rect = centered(area, width, lines.len() as u16 + 2);
    frame.render_widget(Clear, rect);
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .border_style(Style::new().fg(Color::Cyan))
                .title(Span::styled(
                    title,
                    Style::new().add_modifier(Modifier::BOLD),
                )),
        ),
        rect,
    );
    if let Some(choice) = &form.choice {
        render_choice(choice, frame, area);
    }
    if let Some(edit) = &form.edit {
        render_edit(edit, frame, area);
    }
}

fn render_choice(choice: &FormChoice, frame: &mut Frame, area: Rect) {
    let inner_width = choice
        .options
        .iter()
        .map(|(_, label)| label.width() + 4)
        .chain(std::iter::once(choice.title.width() + 4))
        .max()
        .unwrap_or(20)
        .min(usize::from(area.width.saturating_sub(4)));
    let height = usize::from(area.height.saturating_sub(2)).max(1);
    let offset = choice.selected.saturating_sub(height.saturating_sub(1));
    let lines: Vec<Line> = choice
        .options
        .iter()
        .enumerate()
        .skip(offset)
        .take(height)
        .map(|(index, (_, label))| {
            let style = if index == choice.selected {
                Style::new().bg(Color::Blue).fg(Color::White)
            } else {
                Style::new()
            };
            Line::from(vec![
                Span::styled(
                    if index < 9 {
                        format!("{} ", index + 1)
                    } else {
                        "  ".into()
                    },
                    Style::new().fg(Color::Cyan),
                ),
                Span::styled(truncate(label, inner_width.saturating_sub(2)), style),
            ])
        })
        .collect();
    let rect = centered(area, inner_width as u16 + 2, lines.len() as u16 + 2);
    frame.render_widget(Clear, rect);
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .title(format!(" {} ", choice.title)),
        ),
        rect,
    );
}

fn render_edit(edit: &FormEdit, frame: &mut Frame, area: Rect) {
    let width = area.width.saturating_sub(8).clamp(20, 100);
    let inner = usize::from(width.saturating_sub(2));
    let cursor_col = edit.input.cursor_width();
    let scroll = cursor_col.saturating_sub(inner.saturating_sub(2));
    let visible: String = edit.input.text().chars().skip(scroll).collect();
    let mut lines = vec![Line::from(truncate(&visible, inner))];
    if let Some(error) = &edit.error {
        lines.push(Line::styled(
            truncate(error, inner),
            Style::new().fg(Color::Red),
        ));
    } else if let Some(hint) = &edit.hint {
        lines.push(Line::styled(truncate(hint, inner), MUTED));
    }
    lines.push(Line::styled("Enter save · Esc cancel", MUTED));
    let rect = centered(area, width, lines.len() as u16 + 2);
    frame.render_widget(Clear, rect);
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .border_style(Style::new().fg(Color::Yellow))
                .title(format!(" {} ", edit.field.label())),
        ),
        rect,
    );
    let x = rect.x + 1 + (cursor_col - scroll) as u16;
    if x < rect.x + rect.width.saturating_sub(1) {
        frame.set_cursor_position((x, rect.y + 1));
    }
}

fn render_confirm(confirm: &DeleteConfirm, frame: &mut Frame, area: Rect) {
    let lines = vec![
        Line::from(format!(
            "\u{201c}{}\u{201d} and its run history will be permanently removed.",
            confirm.name
        )),
        Line::default(),
        Line::styled("y/Enter delete · Esc cancel", MUTED),
    ];
    let width = lines
        .iter()
        .map(Line::width)
        .max()
        .unwrap_or(30)
        .clamp(30, 90) as u16
        + 4;
    let rect = centered(area, width, lines.len() as u16 + 3);
    frame.render_widget(Clear, rect);
    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }).block(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .border_style(Style::new().fg(Color::Red))
                .title(Span::styled(
                    " Delete automation? ",
                    Style::new().add_modifier(Modifier::BOLD),
                )),
        ),
        rect,
    );
}
