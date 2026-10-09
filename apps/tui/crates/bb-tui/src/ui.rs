use bb_store::ConnectionState;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph, Wrap};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::{AppState, Focus, Mode, ToastLevel};
use crate::indicator::{Indicator, display_status_label};
use crate::input::LineInput;
use crate::keymap::Command;
use crate::sidebar::{GroupRow, SidebarRow, ThreadRow};
use crate::time::compact_relative_time;

const SIDEBAR_MIN: u16 = 24;
const SIDEBAR_MAX: u16 = 40;

pub fn truncate(text: &str, width: usize) -> String {
    if text.width() <= width {
        return text.to_owned();
    }
    if width == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut used = 0;
    for ch in text.chars() {
        let w = ch.width().unwrap_or(0);
        if used + w + 1 > width {
            break;
        }
        out.push(ch);
        used += w;
    }
    out.push('…');
    out
}

fn pad_to(spans: &mut Vec<Span<'static>>, used: usize, target: usize) {
    if target > used {
        spans.push(Span::raw(" ".repeat(target - used)));
    }
}

fn pane_block(title: &str, focused: bool) -> Block<'static> {
    let style = if focused {
        Style::new().fg(Color::Cyan)
    } else {
        Style::new().fg(Color::DarkGray)
    };
    Block::bordered()
        .border_type(if focused {
            BorderType::Thick
        } else {
            BorderType::Plain
        })
        .border_style(style)
        .title(Span::styled(
            format!(" {title} "),
            if focused {
                style.add_modifier(Modifier::BOLD)
            } else {
                Style::new()
            },
        ))
}

pub fn render(state: &AppState, frame: &mut Frame) {
    let area = frame.area();
    let banner = banner_text(state);
    let [banner_area, body, status] = Layout::vertical([
        Constraint::Length(u16::from(banner.is_some())),
        Constraint::Fill(1),
        Constraint::Length(1),
    ])
    .areas(area);
    if let Some(text) = banner {
        frame.render_widget(
            Paragraph::new(truncate(&text, usize::from(banner_area.width)))
                .style(Style::new().bg(Color::Red).fg(Color::White)),
            banner_area,
        );
    }

    let sidebar_width = (body.width * 3 / 10).clamp(SIDEBAR_MIN, SIDEBAR_MAX);
    let panel_width = (body.width * 3 / 10).clamp(20, 48);
    let [sidebar_area, main_area, panel_area] = Layout::horizontal([
        Constraint::Length(if state.sidebar_visible {
            sidebar_width
        } else {
            0
        }),
        Constraint::Fill(1),
        Constraint::Length(if state.panel_visible { panel_width } else { 0 }),
    ])
    .areas(body);

    if state.sidebar_visible {
        render_sidebar(state, frame, sidebar_area);
    }
    render_main(state, frame, main_area);
    if state.panel_visible {
        render_panel(state, frame, panel_area);
    }
    render_status(state, frame, status);
    render_toasts(state, frame, body);
    if state.mode == Mode::Help {
        render_help(state, frame, area);
    }
}

fn banner_text(state: &AppState) -> Option<String> {
    if let ConnectionState::Disconnected { retry_in, error } = &state.snapshot.connection {
        let mut text = String::from(" Disconnected from server");
        if let Some(error) = error {
            text.push_str(": ");
            text.push_str(error);
        }
        match retry_in {
            Some(delay) => text.push_str(&format!(" · retrying in {}s", delay.as_secs().max(1))),
            None => text.push_str(" · reconnecting"),
        }
        return Some(text);
    }
    if state.snapshot.sidebar.data.is_none()
        && let Some(error) = &state.snapshot.sidebar.error
    {
        return Some(format!(" Failed to load threads: {error}"));
    }
    None
}

fn render_sidebar(state: &AppState, frame: &mut Frame, area: Rect) {
    let focused = state.focus == Focus::Sidebar;
    let block = pane_block("Threads", focused);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height == 0 || inner.width == 0 {
        return;
    }
    let filter_line = match &state.mode {
        Mode::Filter(input) => Some((input.clone(), true)),
        _ if !state.filter.is_empty() => Some((LineInput::new(state.filter.clone()), false)),
        _ => None,
    };
    let list_area = if let Some((input, editing)) = filter_line {
        let [filter_area, rest] =
            Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).areas(inner);
        let line = Line::from(vec![
            Span::styled("/", Style::new().fg(Color::Yellow)),
            Span::raw(input.text().to_owned()),
        ]);
        frame.render_widget(Paragraph::new(line), filter_area);
        if editing {
            let x = filter_area.x + 1 + input.cursor_width() as u16;
            if x < filter_area.right() {
                frame.set_cursor_position((x, filter_area.y));
            }
        }
        rest
    } else {
        inner
    };

    if state.rows.is_empty() {
        let message = if state.snapshot.sidebar.is_loading() {
            "Loading…"
        } else if !state.filter.is_empty() {
            "No matching threads"
        } else {
            "No threads"
        };
        frame.render_widget(
            Paragraph::new(message).style(Style::new().fg(Color::DarkGray)),
            list_area,
        );
        return;
    }

    let height = usize::from(list_area.height);
    let mut offset = state.sidebar_offset.get();
    if state.selected < offset {
        offset = state.selected;
    } else if state.selected >= offset + height {
        offset = state.selected + 1 - height;
    }
    offset = offset.min(state.rows.len().saturating_sub(height));
    state.sidebar_offset.set(offset);

    let width = usize::from(list_area.width);
    for (line_index, (index, row)) in state
        .rows
        .iter()
        .enumerate()
        .skip(offset)
        .take(height)
        .enumerate()
    {
        let selected = index == state.selected;
        let rect = Rect {
            x: list_area.x,
            y: list_area.y + line_index as u16,
            width: list_area.width,
            height: 1,
        };
        let renaming = match (&state.mode, row) {
            (Mode::Rename { thread_id, input }, SidebarRow::Thread(thread))
                if *thread_id == thread.id =>
            {
                Some(input)
            }
            _ => None,
        };
        let line = match row {
            SidebarRow::Group(group) => group_line(group, width),
            SidebarRow::Thread(thread) => thread_line(state, thread, width, renaming),
        };
        let mut style = Style::new();
        if selected {
            style = if focused {
                Style::new().bg(Color::Blue).fg(Color::White)
            } else {
                Style::new().bg(Color::DarkGray)
            };
        } else if state.open_thread.as_deref() == row.thread().map(|thread| thread.id.as_str()) {
            style = Style::new().add_modifier(Modifier::UNDERLINED);
        }
        frame.render_widget(Paragraph::new(line).style(style), rect);
        if let (Some(input), SidebarRow::Thread(thread)) = (renaming, row) {
            let x = rect.x + (thread.depth * 2 + 3 + input.cursor_width()) as u16;
            if x < rect.right() {
                frame.set_cursor_position((x, rect.y));
            }
        }
    }
}

fn group_line(group: &GroupRow, width: usize) -> Line<'static> {
    let arrow = if group.collapsed { "▸ " } else { "▾ " };
    let count = group.thread_count.to_string();
    let attention = if group.collapsed && group.attention > 0 {
        format!("●{} ", group.attention)
    } else {
        String::new()
    };
    let tail = attention.width() + count.width();
    let label = truncate(&group.label, width.saturating_sub(2 + tail + 1));
    let mut spans = vec![
        Span::styled(arrow, Style::new().fg(Color::DarkGray)),
        Span::styled(label.clone(), Style::new().add_modifier(Modifier::BOLD)),
    ];
    let used = 2 + label.width();
    pad_to(&mut spans, used, width.saturating_sub(tail));
    if !attention.is_empty() {
        spans.push(Span::styled(attention, Style::new().fg(Color::Yellow)));
    }
    spans.push(Span::styled(count, Style::new().fg(Color::DarkGray)));
    Line::from(spans)
}

fn thread_line(
    state: &AppState,
    thread: &ThreadRow,
    width: usize,
    renaming: Option<&LineInput>,
) -> Line<'static> {
    let indent = "  ".repeat(thread.depth);
    let disclosure = match (thread.child_count, thread.expanded) {
        (0, _) => ' ',
        (_, true) => '▾',
        (_, false) => '▸',
    };
    let glyph = thread.indicator.glyph(state.frame);
    let mut spans = vec![
        Span::raw(indent.clone()),
        Span::styled(disclosure.to_string(), Style::new().fg(Color::DarkGray)),
        Span::styled(glyph.to_string(), thread.indicator.style()),
        Span::raw(" "),
    ];
    let prefix = indent.width() + 3;
    if let Some(input) = renaming {
        let text = truncate(input.text(), width.saturating_sub(prefix));
        spans.push(Span::styled(
            text,
            Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        ));
        return Line::from(spans);
    }
    let time = compact_relative_time(thread.attention_at, state.now_ms);
    let collapsed_children = if thread.child_count > 0 && !thread.expanded {
        format!("+{} ", thread.child_count)
    } else {
        String::new()
    };
    let tail = time.width() + collapsed_children.width() + 1;
    let title = truncate(&thread.title, width.saturating_sub(prefix + tail));
    let title_style = if thread.unread || thread.indicator.needs_attention() {
        Style::new().add_modifier(Modifier::BOLD)
    } else {
        Style::new()
    };
    let used = prefix + title.width();
    spans.push(Span::styled(title, title_style));
    pad_to(&mut spans, used, width.saturating_sub(tail - 1));
    if !collapsed_children.is_empty() {
        spans.push(Span::styled(
            collapsed_children,
            Style::new().fg(Color::DarkGray),
        ));
    }
    spans.push(Span::styled(time, Style::new().fg(Color::DarkGray)));
    Line::from(spans)
}

fn render_main(state: &AppState, frame: &mut Frame, area: Rect) {
    let focused = state.focus == Focus::Main;
    let Some(thread_id) = state.open_thread.as_deref() else {
        let block = pane_block("bb", focused);
        let hint = format!(
            "Select a thread and press {} to open it.\nPress {} for help.",
            state.keymap.label_for(Command::Open),
            state.keymap.label_for(Command::Help),
        );
        frame.render_widget(
            Paragraph::new(hint)
                .style(Style::new().fg(Color::DarkGray))
                .block(block),
            area,
        );
        return;
    };
    let title = state.thread_title(thread_id);
    let block = pane_block(&title, focused);
    let entry = state.sidebar_entry(thread_id);
    let detail = state.snapshot.thread(thread_id);
    let mut lines = Vec::new();
    if let Mode::Rename {
        thread_id: renaming,
        input,
    } = &state.mode
        && renaming == thread_id
        && !state
            .rows
            .iter()
            .any(|row| row.thread().is_some_and(|thread| thread.id == *thread_id))
    {
        lines.push(Line::from(vec![
            Span::styled("Rename: ", Style::new().fg(Color::Yellow)),
            Span::raw(input.text().to_owned()),
        ]));
    }
    lines.push(Line::from(Span::styled(
        title.clone(),
        Style::new().add_modifier(Modifier::BOLD),
    )));
    let display_status = entry
        .map(|entry| entry.runtime.display_status)
        .or_else(|| detail.map(|detail| detail.runtime.display_status));
    let indicator = entry.map(Indicator::for_entry);
    let mut status_spans = vec![Span::styled("status: ", Style::new().fg(Color::DarkGray))];
    match display_status {
        Some(status) => status_spans.push(Span::raw(display_status_label(status))),
        None => status_spans.push(Span::raw("loading…")),
    }
    if let Some(indicator) = indicator
        && let Some(label) = indicator.label()
    {
        status_spans.push(Span::raw("  "));
        status_spans.push(Span::styled(
            format!("{} {label}", indicator.glyph(state.frame)),
            indicator.style(),
        ));
    }
    lines.push(Line::from(status_spans));
    let thread = entry
        .map(|entry| &entry.thread)
        .or_else(|| detail.map(|detail| &detail.thread));
    if let Some(thread) = thread {
        let project = state
            .snapshot
            .sidebar
            .data
            .as_deref()
            .and_then(|sidebar| {
                sidebar
                    .projects
                    .iter()
                    .chain(std::iter::once(&sidebar.personal_project))
                    .find(|project| project.project.id == thread.project_id)
            })
            .map_or(thread.project_id.clone(), |project| {
                project.project.name.clone()
            });
        lines.push(Line::from(vec![
            Span::styled("project: ", Style::new().fg(Color::DarkGray)),
            Span::raw(project),
        ]));
        lines.push(Line::from(vec![
            Span::styled("provider: ", Style::new().fg(Color::DarkGray)),
            Span::raw(thread.provider_id.clone()),
        ]));
        let mut flags = Vec::new();
        if thread.is_pinned() {
            flags.push("pinned");
        }
        if thread.is_archived() {
            flags.push("archived");
        }
        if !crate::indicator::is_thread_read(thread) {
            flags.push("unread");
        }
        if !flags.is_empty() {
            lines.push(Line::from(Span::styled(
                flags.join(" · "),
                Style::new().fg(Color::DarkGray),
            )));
        }
        lines.push(Line::from(Span::styled(
            format!(
                "updated {} ago",
                compact_relative_time(thread.updated_at, state.now_ms)
            ),
            Style::new().fg(Color::DarkGray),
        )));
    }
    if let Some(branch) = entry.and_then(|entry| entry.environment_branch_name.as_deref()) {
        lines.push(Line::from(vec![
            Span::styled("branch: ", Style::new().fg(Color::DarkGray)),
            Span::raw(branch.to_owned()),
        ]));
    }
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(block),
        area,
    );
}

fn render_panel(state: &AppState, frame: &mut Frame, area: Rect) {
    let focused = state.focus == Focus::Panel;
    let block = pane_block("Panel", focused);
    let text = match state.open_thread.as_deref() {
        Some(id) => format!("thread {id}"),
        None => "Nothing to show".into(),
    };
    frame.render_widget(
        Paragraph::new(text)
            .style(Style::new().fg(Color::DarkGray))
            .block(block),
        area,
    );
}

fn connection_span(connection: &ConnectionState) -> Span<'static> {
    match connection {
        ConnectionState::Idle => Span::styled("○ idle", Style::new().fg(Color::DarkGray)),
        ConnectionState::Connecting { reconnect: false } => {
            Span::styled("◌ connecting", Style::new().fg(Color::Yellow))
        }
        ConnectionState::Connecting { reconnect: true } => {
            Span::styled("◌ reconnecting", Style::new().fg(Color::Yellow))
        }
        ConnectionState::Connected => Span::styled("● connected", Style::new().fg(Color::Green)),
        ConnectionState::Disconnected { .. } => {
            Span::styled("✗ disconnected", Style::new().fg(Color::Red))
        }
    }
}

fn first_label(state: &AppState, command: Command) -> String {
    state
        .keymap
        .chords_for(command)
        .first()
        .map(ToString::to_string)
        .unwrap_or_default()
}

fn hints(state: &AppState) -> String {
    let key = |command| first_label(state, command);
    match &state.mode {
        Mode::Filter(_) => "type to filter · Enter keep · Esc clear".into(),
        Mode::Rename { .. } => "Enter save · Esc cancel".into(),
        Mode::Help => "any key closes help".into(),
        Mode::Normal => format!(
            "{}/{} move · {} open · {} filter · {} help · {} quit",
            key(Command::MoveDown),
            key(Command::MoveUp),
            key(Command::Open),
            key(Command::Filter),
            key(Command::Help),
            key(Command::Quit),
        ),
    }
}

fn render_status(state: &AppState, frame: &mut Frame, area: Rect) {
    let width = usize::from(area.width);
    let server = match &state.server.profile {
        Some(profile) => format!(" {profile} "),
        None => format!(" {} ", state.server.url),
    };
    let mut spans = vec![
        Span::styled(
            truncate(&server, width / 3),
            Style::new().bg(Color::DarkGray).fg(Color::White),
        ),
        Span::raw(" "),
        connection_span(&state.snapshot.connection),
    ];
    if state.attention > 0 {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            format!("⚑ {}", state.attention),
            Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        ));
    }
    let used: usize = spans.iter().map(|span| span.content.width()).sum();
    let hint = hints(state);
    let available = width.saturating_sub(used + 2);
    if available > 8 {
        let hint = truncate(&hint, available);
        pad_to(&mut spans, used, width.saturating_sub(hint.width() + 1));
        spans.push(Span::styled(hint, Style::new().fg(Color::DarkGray)));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn render_toasts(state: &AppState, frame: &mut Frame, area: Rect) {
    if state.toasts.is_empty() || area.height < 3 {
        return;
    }
    let max_width = usize::from(area.width.saturating_sub(4)).min(60);
    let mut y = area.bottom().saturating_sub(1);
    for toast in state.toasts.iter().rev() {
        if y <= area.y {
            break;
        }
        y -= 1;
        let text = truncate(&format!(" {} ", toast.message), max_width);
        let width = text.width() as u16;
        let rect = Rect {
            x: area.right().saturating_sub(width + 2),
            y,
            width,
            height: 1,
        };
        let style = match toast.level {
            ToastLevel::Info => Style::new().bg(Color::Blue).fg(Color::White),
            ToastLevel::Error => Style::new().bg(Color::Red).fg(Color::White),
        };
        frame.render_widget(Clear, rect);
        frame.render_widget(Paragraph::new(text).style(style), rect);
    }
}

fn help_entries(state: &AppState) -> Vec<(String, String)> {
    Command::ALL
        .iter()
        .filter_map(|command| match command {
            Command::Jump(1) => Some((
                format!(
                    "{}…{}",
                    state.keymap.label_for(Command::Jump(1)),
                    state.keymap.label_for(Command::Jump(9))
                ),
                "Open thread #1-9".to_owned(),
            )),
            Command::Jump(_) => None,
            command => {
                let label = state.keymap.label_for(*command);
                (!label.is_empty()).then(|| (label, command.description()))
            }
        })
        .collect()
}

fn render_help(state: &AppState, frame: &mut Frame, area: Rect) {
    let entries = help_entries(state);
    let key_width = entries
        .iter()
        .map(|(key, _)| key.width())
        .max()
        .unwrap_or(0);
    let desc_width = entries
        .iter()
        .map(|(_, desc)| desc.width())
        .max()
        .unwrap_or(0);
    let width = ((key_width + desc_width + 6) as u16).min(area.width);
    let height = (entries.len() as u16 + 2).min(area.height);
    let rect = Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    };
    let lines: Vec<Line> = entries
        .into_iter()
        .map(|(key, desc)| {
            Line::from(vec![
                Span::styled(
                    format!(" {key:<key_width$}  "),
                    Style::new().fg(Color::Cyan),
                ),
                Span::raw(desc),
            ])
        })
        .collect();
    frame.render_widget(Clear, rect);
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .title(" Keys "),
        ),
        rect,
    );
}
