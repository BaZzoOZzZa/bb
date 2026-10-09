use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph, Wrap};
use unicode_width::UnicodeWidthStr;

use crate::app::AppState;
use crate::settings::{
    Choice, FieldEdit, RowStyle, SettingsConfirm, SettingsFocus, SettingsRow, SettingsSection,
    SettingsView, Tone, clamp_cursor, settings_rows,
};
use crate::ui::truncate;

const NAV_WIDTH: u16 = 24;
const LABEL_MIN: usize = 18;
const LABEL_MAX: usize = 44;
const MUTED: Style = Style::new().fg(Color::DarkGray);

fn tone_style(tone: Tone) -> Style {
    match tone {
        Tone::Muted => Style::new().fg(Color::DarkGray),
        Tone::Good => Style::new().fg(Color::Green),
        Tone::Warn => Style::new().fg(Color::Yellow),
        Tone::Bad => Style::new().fg(Color::Red),
        Tone::Accent => Style::new().fg(Color::Cyan),
    }
}

fn border(focused: bool) -> Style {
    if focused {
        Style::new().fg(Color::Cyan)
    } else {
        Style::new().fg(Color::DarkGray)
    }
}

pub fn render_settings(state: &AppState, view: &SettingsView, frame: &mut Frame, area: Rect) {
    frame.render_widget(Clear, area);
    let [nav, content] = Layout::horizontal([
        Constraint::Length(NAV_WIDTH.min(area.width / 3)),
        Constraint::Fill(1),
    ])
    .areas(area);
    render_nav(view, frame, nav);
    render_content(state, view, frame, content);
    if let Some(confirm) = &view.confirm {
        render_confirm(confirm, frame, area);
    }
    if let Some(choice) = &view.choice {
        render_choice(choice, frame, area);
    }
    if let Some(edit) = &view.edit {
        render_edit(edit, frame, area);
    }
}

fn render_nav(view: &SettingsView, frame: &mut Frame, area: Rect) {
    let focused = view.focus == SettingsFocus::Nav;
    let width = usize::from(area.width.saturating_sub(2));
    let lines: Vec<Line> = SettingsSection::ALL
        .iter()
        .enumerate()
        .map(|(index, section)| {
            let selected = *section == view.section;
            let text = truncate(&format!(" {} {}", index + 1, section.label()), width);
            let style = match (selected, focused) {
                (true, true) => Style::new().bg(Color::Blue).fg(Color::White),
                (true, false) => Style::new().add_modifier(Modifier::BOLD | Modifier::REVERSED),
                _ => Style::new(),
            };
            let pad = width.saturating_sub(text.width());
            Line::from(Span::styled(format!("{text}{}", " ".repeat(pad)), style))
        })
        .collect();
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::bordered()
                .border_type(if focused {
                    BorderType::Thick
                } else {
                    BorderType::Plain
                })
                .border_style(border(focused))
                .title(" Settings "),
        ),
        area,
    );
}

fn row_line(row: &SettingsRow, label_width: usize, width: usize, selected: bool) -> Line<'static> {
    let indent = " ".repeat(usize::from(row.indent));
    let base = if selected {
        Style::new().add_modifier(Modifier::REVERSED)
    } else {
        Style::new()
    };
    let marker = if selected { "› " } else { "  " };
    match row.style {
        RowStyle::Header => {
            let mut spans = vec![Span::styled(
                truncate(&row.label, width.saturating_sub(2)),
                Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            )];
            if !row.value.is_empty() {
                spans.push(Span::styled(format!("  {}", row.value), MUTED));
            }
            for badge in &row.badges {
                spans.push(Span::raw(" "));
                spans.push(Span::styled(
                    format!("[{}]", badge.text),
                    tone_style(badge.tone),
                ));
            }
            Line::from(spans)
        }
        RowStyle::Note | RowStyle::Error => {
            let style = if row.style == RowStyle::Error {
                Style::new().fg(Color::Red)
            } else {
                MUTED
            };
            Line::from(Span::styled(
                truncate(&format!("  {indent}{}", row.label), width),
                style,
            ))
        }
        RowStyle::Item => {
            let label_room = label_width.saturating_sub(indent.width());
            let label = truncate(&row.label, label_room);
            let label_pad = label_room.saturating_sub(label.width());
            let mut spans = vec![
                Span::styled(marker, base),
                Span::styled(indent, base),
                Span::styled(
                    format!("{label}{}", " ".repeat(label_pad)),
                    if row.target.is_some() {
                        base.add_modifier(Modifier::BOLD)
                    } else {
                        base
                    },
                ),
                Span::styled("  ", base),
            ];
            let badges: String = row
                .badges
                .iter()
                .map(|badge| format!(" [{}]", badge.text))
                .collect();
            let used = 2 + label_width + 2;
            let value_room = width.saturating_sub(used + badges.width());
            spans.push(Span::styled(truncate(&row.value, value_room), base));
            for badge in &row.badges {
                spans.push(Span::raw(" "));
                spans.push(Span::styled(
                    format!("[{}]", badge.text),
                    tone_style(badge.tone),
                ));
            }
            Line::from(spans)
        }
    }
}

fn render_content(state: &AppState, view: &SettingsView, frame: &mut Frame, area: Rect) {
    let focused = view.focus == SettingsFocus::Content
        && view.edit.is_none()
        && view.choice.is_none()
        && view.confirm.is_none();
    let rows = settings_rows(state);
    let filter = view.filter(view.section);
    let mut title = format!(" {} ", view.section.label());
    if view.detail.is_none() && !filter.is_empty() {
        title = format!(" {} · /{} ", view.section.label(), filter);
    }
    let block = Block::bordered()
        .border_type(if focused {
            BorderType::Thick
        } else {
            BorderType::Plain
        })
        .border_style(border(focused))
        .title(title);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let [list_area, footer] = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(u16::from(view.filter_input.is_some())),
    ])
    .areas(inner);
    let width = usize::from(list_area.width);
    let label_width = rows
        .iter()
        .filter(|row| row.style == RowStyle::Item)
        .map(|row| row.label.width() + usize::from(row.indent))
        .max()
        .unwrap_or(LABEL_MIN)
        .clamp(
            LABEL_MIN,
            LABEL_MAX.min(width / 2).max(LABEL_MIN.min(width)),
        );
    let cursor = clamp_cursor(&rows, view.cursor());
    let height = usize::from(list_area.height).max(1);
    let mut offset = view.offset.get();
    if let Some(cursor) = cursor {
        if cursor < offset {
            offset = cursor.saturating_sub(1);
        } else if cursor >= offset + height {
            offset = cursor + 1 - height;
        }
    }
    offset = offset.min(rows.len().saturating_sub(height));
    view.offset.set(offset);
    let lines: Vec<Line> = rows
        .iter()
        .enumerate()
        .skip(offset)
        .take(height)
        .map(|(index, row)| row_line(row, label_width, width, focused && Some(index) == cursor))
        .collect();
    frame.render_widget(Paragraph::new(lines), list_area);
    if let Some(input) = &view.filter_input {
        let text = format!("/{}", input.text());
        frame.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(
                truncate(&text, width),
                Style::new().fg(Color::Yellow),
            )])),
            footer,
        );
        let x = footer.x + 1 + input.cursor_width() as u16;
        if x < footer.x + footer.width {
            frame.set_cursor_position((x, footer.y));
        }
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

fn render_confirm(confirm: &SettingsConfirm, frame: &mut Frame, area: Rect) {
    let mut lines: Vec<Line> = confirm.lines().into_iter().map(Line::from).collect();
    lines.push(Line::default());
    lines.push(Line::styled(
        if confirm.ready() {
            "y/Enter confirm · Esc cancel"
        } else {
            "Esc cancel"
        },
        MUTED,
    ));
    let width = lines
        .iter()
        .map(Line::width)
        .max()
        .unwrap_or(20)
        .clamp(30, 90) as u16
        + 4;
    let rect = centered(area, width, lines.len() as u16 + 2);
    frame.render_widget(Clear, rect);
    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }).block(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .border_style(Style::new().fg(Color::Red))
                .title(Span::styled(
                    format!(" {} ", confirm.title()),
                    Style::new().add_modifier(Modifier::BOLD),
                )),
        ),
        rect,
    );
}

fn render_choice(choice: &Choice, frame: &mut Frame, area: Rect) {
    let inner_width = choice
        .options
        .iter()
        .map(|(_, label)| label.width() + 4)
        .chain(std::iter::once(choice.title.width() + 4))
        .max()
        .unwrap_or(20)
        .min(usize::from(area.width.saturating_sub(4)));
    let lines: Vec<Line> = choice
        .options
        .iter()
        .enumerate()
        .map(|(index, (_, label))| {
            let style = if index == choice.selected {
                Style::new().bg(Color::Blue).fg(Color::White)
            } else {
                Style::new()
            };
            Line::from(vec![
                Span::styled(format!("{} ", index + 1), Style::new().fg(Color::Cyan)),
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

fn render_edit(edit: &FieldEdit, frame: &mut Frame, area: Rect) {
    let width = area.width.saturating_sub(8).clamp(20, 90);
    let inner = usize::from(width.saturating_sub(2));
    let shown = if edit.secret {
        "•".repeat(edit.input.text().chars().count())
    } else {
        edit.input.text().to_owned()
    };
    let (before, _) = edit.input.split_at_cursor();
    let cursor_col = if edit.secret {
        before.chars().count()
    } else {
        edit.input.cursor_width()
    };
    let scroll = cursor_col.saturating_sub(inner.saturating_sub(2));
    let visible: String = shown.chars().skip(scroll).collect();
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
                .title(format!(" {} ", edit.label)),
        ),
        rect,
    );
    let x = rect.x + 1 + (cursor_col - scroll) as u16;
    if x < rect.x + rect.width.saturating_sub(1) {
        frame.set_cursor_position((x, rect.y + 1));
    }
}
