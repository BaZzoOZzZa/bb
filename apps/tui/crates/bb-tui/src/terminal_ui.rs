use bb_client::TerminalConnectionState;
use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::AppState;
use crate::execution::Load;
use crate::terminal::TerminalView;

const MUTED: Style = Style::new().fg(Color::DarkGray);
const WARNING: Style = Style::new().fg(Color::Yellow);

pub fn vt_color(color: vt100::Color) -> Color {
    match color {
        vt100::Color::Default => Color::Reset,
        vt100::Color::Idx(index) => Color::Indexed(index),
        vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
}

pub fn cell_style(cell: &vt100::Cell) -> Style {
    let mut style = Style::new()
        .fg(vt_color(cell.fgcolor()))
        .bg(vt_color(cell.bgcolor()));
    let mut modifiers = Modifier::empty();
    if cell.bold() {
        modifiers |= Modifier::BOLD;
    }
    if cell.dim() {
        modifiers |= Modifier::DIM;
    }
    if cell.italic() {
        modifiers |= Modifier::ITALIC;
    }
    if cell.underline() {
        modifiers |= Modifier::UNDERLINED;
    }
    if cell.inverse() {
        modifiers |= Modifier::REVERSED;
    }
    style.add_modifier = modifiers;
    style
}

pub fn render_screen(view: &TerminalView, buf: &mut Buffer, area: Rect) -> Option<(u16, u16)> {
    let screen = view.screen.screen();
    let (rows, cols) = screen.size();
    let (cursor_row, cursor_col) = screen.cursor_position();
    let scrolled = screen.scrollback() > 0;
    let row_offset = if scrolled || rows <= area.height {
        0
    } else {
        (cursor_row + 1).saturating_sub(area.height)
    };
    let height = area.height.min(rows.saturating_sub(row_offset));
    let width = area.width.min(cols);
    for y in 0..height {
        for x in 0..width {
            let Some(cell) = screen.cell(row_offset + y, x) else {
                continue;
            };
            if cell.is_wide_continuation() {
                continue;
            }
            let Some(target) = buf.cell_mut((area.x + x, area.y + y)) else {
                continue;
            };
            let contents = cell.contents();
            let symbol = if contents.is_empty() || (cell.is_wide() && x + 1 >= width) {
                " "
            } else {
                contents
            };
            target.set_symbol(symbol).set_style(cell_style(cell));
        }
    }
    if scrolled || screen.hide_cursor() {
        return None;
    }
    let row = cursor_row.checked_sub(row_offset)?;
    (row < height && cursor_col < width).then_some((area.x + cursor_col, area.y + row))
}

fn paint_history(state: &AppState, view: &TerminalView, buf: &mut Buffer, area: Rect) {
    let paint = crate::rbuf::paint(state, crate::rbuf::Buf::Terminal);
    let screen = view.screen.screen();
    let (rows, cols) = screen.size();
    let max = view.max_scrollback();
    let top = max - screen.scrollback().min(max);
    let (cursor_row, _) = screen.cursor_position();
    let row_offset = if screen.scrollback() > 0 || rows <= area.height {
        0
    } else {
        (cursor_row + 1).saturating_sub(area.height)
    };
    let height = area.height.min(rows.saturating_sub(row_offset));
    let width = area.width.min(cols);
    for y in 0..height {
        let mut text = String::new();
        for x in 0..cols {
            let Some(cell) = screen.cell(row_offset + y, x) else {
                continue;
            };
            if cell.is_wide_continuation() {
                continue;
            }
            let contents = cell.contents();
            text.push_str(if contents.is_empty() { " " } else { contents });
        }
        let line = top + usize::from(row_offset + y);
        let rect = Rect {
            x: area.x,
            y: area.y + y,
            width,
            height: 1,
        };
        paint.overlay(buf, line, text.trim_end(), rect, 0, true);
    }
}

fn status_mark(view: Option<&TerminalView>, running: bool) -> (&'static str, Style) {
    match view.map(|view| view.connection) {
        _ if !running => ("○", MUTED),
        Some(TerminalConnectionState::Attached) => ("●", Style::new().fg(Color::Green)),
        Some(TerminalConnectionState::Connecting | TerminalConnectionState::Reconnecting) => {
            ("◌", WARNING)
        }
        Some(TerminalConnectionState::Closed) | None => ("●", MUTED),
    }
}

fn tabs_line(state: &AppState, width: u16) -> Line<'static> {
    let Some(sessions) = state.terminals.sessions.loaded() else {
        return Line::from(Span::styled("Terminals", MUTED));
    };
    let mut spans = Vec::new();
    for (index, session) in sessions.iter().enumerate() {
        if index > 0 {
            spans.push(Span::styled(" │ ", MUTED));
        }
        let view = state.terminals.views.get(&session.id);
        let (mark, mark_style) = status_mark(view, session.status.is_active());
        let title = view.map_or_else(
            || {
                if session.title.trim().is_empty() {
                    "Terminal".to_owned()
                } else {
                    session.title.clone()
                }
            },
            TerminalView::title,
        );
        let active = state.terminals.active.as_deref() == Some(session.id.as_str());
        let style = if active {
            Style::new().add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
        } else {
            MUTED
        };
        spans.push(Span::styled(format!("{mark} "), mark_style));
        spans.push(Span::styled(crate::ui::truncate(&title, 28), style));
    }
    if state.terminals.creating {
        spans.push(Span::styled(" · starting…", WARNING));
    }
    let line = Line::from(spans);
    if line.width() > usize::from(width) {
        Line::from(crate::palette_ui::slice_spans(
            &line.spans,
            0,
            usize::from(width),
        ))
    } else {
        line
    }
}

fn footer_text(state: &AppState, view: Option<&TerminalView>) -> (String, Style) {
    if let Some(id) = &state.terminals.confirm_close {
        let title = state
            .terminals
            .views
            .get(id)
            .map_or_else(|| "this terminal".to_owned(), TerminalView::title);
        return (
            format!("Close {title}? It stops the shell. y/Enter close · any key cancel"),
            WARNING,
        );
    }
    let mut parts = Vec::new();
    if let Some(view) = view {
        if let Some(notice) = &view.notice {
            parts.push(notice.clone());
        }
        if view.scrollback() > 0 {
            parts.push(format!("↑ {} lines", view.scrollback()));
        }
    }
    if state.terminal_input_active() {
        parts.push("typing goes to the terminal · C-\\ leave · S-PgUp/PgDn scroll".into());
        return (parts.join(" · "), Style::new().fg(Color::Cyan));
    }
    parts.push(crate::panel_ui::hints(state));
    (parts.join(" · "), MUTED)
}

pub fn render_terminal(state: &AppState, frame: &mut Frame, area: Rect, focused: bool) {
    if area.height < 3 {
        return;
    }
    let [tabs_area, body, footer_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(1),
    ])
    .areas(area);
    frame.render_widget(Paragraph::new(tabs_line(state, tabs_area.width)), tabs_area);
    state
        .terminals
        .viewport
        .set(Some((body.width.max(1), body.height.max(1))));
    let view = state.terminals.active_view();
    let (text, style) = footer_text(state, view);
    frame.render_widget(
        Paragraph::new(crate::ui::truncate(&text, usize::from(footer_area.width))).style(style),
        footer_area,
    );
    let message = match (&state.terminals.sessions, view) {
        (_, Some(_)) => None,
        (Load::Failed(error), None) => Some(format!("Failed to list terminals: {error} · r retry")),
        (Load::Loaded(sessions), None) if sessions.is_empty() && state.read_only => {
            Some("No terminals for this thread.".to_owned())
        }
        (Load::Loaded(sessions), None) if sessions.is_empty() => {
            Some("No terminals for this thread. Press n to start one.".to_owned())
        }
        (Load::Loaded(_), None) => Some("Select a terminal with [ and ].".to_owned()),
        _ => Some("Loading terminals…".to_owned()),
    };
    if let Some(message) = message {
        frame.render_widget(Paragraph::new(message).style(MUTED), body);
        return;
    }
    let Some(view) = view else {
        return;
    };
    let cursor = render_screen(view, frame.buffer_mut(), body);
    if focused && crate::rbuf::focused(state) == Some(crate::rbuf::Buf::Terminal) {
        paint_history(state, view, frame.buffer_mut(), body);
    }
    if let Some((x, y)) = cursor {
        if focused && state.terminal_input_active() {
            frame.set_cursor_position((x, y));
        } else if let Some(cell) = frame.buffer_mut().cell_mut((x, y)) {
            let style = cell.style().add_modifier(Modifier::REVERSED);
            cell.set_style(style);
        }
    }
}
