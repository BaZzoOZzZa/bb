use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph};
use unicode_width::UnicodeWidthStr;

use crate::app::AppState;
use crate::keymap::{Continuation, Scope, format_seq};

const LABEL_MAX: usize = 32;

pub fn entries(state: &AppState) -> Vec<Continuation> {
    let mut scopes = crate::app::normal_scopes(state);
    scopes.push(Scope::Global);
    state.keymap.continuations(&scopes, &state.keys.pending)
}

pub struct Page {
    pub title: String,
    pub columns: usize,
    pub rows: usize,
    pub entries: Vec<Continuation>,
}

pub fn layout(state: &AppState, width: u16, max_height: u16) -> Option<Page> {
    if !state.keys.which_key || state.keys.pending.is_empty() {
        return None;
    }
    let entries = entries(state);
    if entries.is_empty() {
        return None;
    }
    let key_width = entries
        .iter()
        .map(|entry| entry.key.to_string().width())
        .max()
        .unwrap_or(1);
    let label_width = entries
        .iter()
        .map(|entry| entry.label.width().min(LABEL_MAX))
        .max()
        .unwrap_or(1);
    let cell = key_width + 3 + label_width + 2;
    let inner = usize::from(width.saturating_sub(2)).max(cell);
    let columns = (inner / cell).max(1);
    let rows_max = usize::from(max_height.saturating_sub(2)).max(1);
    let rows = entries.len().div_ceil(columns).min(rows_max).max(1);
    let per_page = columns * rows;
    let pages = entries.len().div_ceil(per_page).max(1);
    let page = state.keys.which_key_page.min(pages - 1);
    let mut title = format!(" {}- ", format_seq(&state.keys.pending));
    if pages > 1 {
        title.push_str(&format!("page {}/{} · C-n/C-p ", page + 1, pages));
    }
    let entries = entries
        .into_iter()
        .skip(page * per_page)
        .take(per_page)
        .collect();
    Some(Page {
        title,
        columns,
        rows,
        entries,
    })
}

pub fn render(state: &AppState, frame: &mut Frame, area: Rect) {
    let Some(page) = layout(state, area.width, area.height / 2) else {
        return;
    };
    let height = (page.rows as u16 + 2).min(area.height);
    let rect = Rect {
        x: area.x,
        y: area.bottom().saturating_sub(height),
        width: area.width,
        height,
    };
    let inner_width = usize::from(rect.width.saturating_sub(2));
    let cell = (inner_width / page.columns).max(1);
    let key_width = page
        .entries
        .iter()
        .map(|entry| entry.key.to_string().width())
        .max()
        .unwrap_or(1);
    let lines: Vec<Line> = (0..page.rows)
        .map(|row| {
            let mut spans = Vec::new();
            for column in 0..page.columns {
                let Some(entry) = page.entries.get(column * page.rows + row) else {
                    continue;
                };
                let key = entry.key.to_string();
                let label_room = cell.saturating_sub(key_width + 4);
                let label = crate::ui::truncate(&entry.label, label_room.min(LABEL_MAX));
                let label_style = if entry.group {
                    Style::new().fg(Color::Magenta).add_modifier(Modifier::BOLD)
                } else {
                    Style::new()
                };
                spans.push(Span::styled(
                    format!("{key:>key_width$}"),
                    Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                ));
                spans.push(Span::styled(" → ", Style::new().fg(Color::DarkGray)));
                let used = key_width + 3 + label.width();
                spans.push(Span::styled(label, label_style));
                if column + 1 < page.columns && cell > used {
                    spans.push(Span::raw(" ".repeat(cell - used)));
                }
            }
            Line::from(spans)
        })
        .collect();
    frame.render_widget(Clear, rect);
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .border_style(Style::new().fg(Color::DarkGray))
                .title(Span::styled(
                    page.title,
                    Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD),
                )),
        ),
        rect,
    );
}
