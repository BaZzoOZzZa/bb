use bb_store::ConnectionState;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph, Wrap};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::{AppState, Focus, Mode, ToastLevel};
use crate::composer_ui;
use crate::indicator::{Indicator, display_status_label};
use crate::input::LineInput;
use crate::keymap::Command;
use crate::sidebar::{GroupRow, SidebarRow, ThreadRow};
use crate::time::compact_relative_time;
use crate::timeline::{Frame as TimelineFrame, Scroll};

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
    if let Mode::OpenPicker { targets, selected } = &state.mode {
        render_open_picker(targets, *selected, frame, area);
    }
    if let Mode::Picker(picker) = &state.mode {
        composer_ui::render_picker(picker, frame, area);
    }
}

fn render_open_picker(targets: &[String], selected: usize, frame: &mut Frame, area: Rect) {
    let width = targets
        .iter()
        .map(|target| target.width() + 6)
        .max()
        .unwrap_or(20)
        .clamp(24, usize::from(area.width).saturating_sub(4).max(24)) as u16;
    let width = width.min(area.width);
    let height = (targets.len().min(9) as u16 + 2).min(area.height);
    let rect = Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    };
    let inner_width = usize::from(width.saturating_sub(2));
    let lines: Vec<Line> = targets
        .iter()
        .take(9)
        .enumerate()
        .map(|(index, target)| {
            let style = if index == selected {
                Style::new().bg(Color::Blue).fg(Color::White)
            } else {
                Style::new()
            };
            Line::from(vec![
                Span::styled(format!("{} ", index + 1), Style::new().fg(Color::Cyan)),
                Span::styled(truncate(target, inner_width.saturating_sub(2)), style),
            ])
        })
        .collect();
    frame.render_widget(Clear, rect);
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .title(" Open "),
        ),
        rect,
    );
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
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height == 0 || inner.width < 4 {
        return;
    }
    let [meta_area, body] =
        Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).areas(inner);
    let meta = meta_line(state, thread_id, usize::from(meta_area.width));
    frame.render_widget(Paragraph::new(meta), meta_area);
    if !state.composer_available() || body.height < 6 {
        render_timeline(state, frame, body, focused);
        return;
    }
    let wanted = composer_ui::composer_height(state, thread_id, usize::from(body.width));
    let height = wanted.min(body.height.saturating_sub(3).max(body.height / 2));
    let [timeline_area, composer_area] =
        Layout::vertical([Constraint::Fill(1), Constraint::Length(height)]).areas(body);
    render_timeline(state, frame, timeline_area, focused);
    composer_ui::render_composer(state, frame, composer_area, thread_id);
}

fn meta_line(state: &AppState, thread_id: &str, width: usize) -> Line<'static> {
    let muted = Style::new().fg(Color::DarkGray);
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
        return Line::from(vec![
            Span::styled("Rename: ", Style::new().fg(Color::Yellow)),
            Span::raw(truncate(input.text(), width.saturating_sub(8))),
        ]);
    }
    let entry = state.sidebar_entry(thread_id);
    let detail = state.snapshot.thread(thread_id);
    let mut spans: Vec<Span<'static>> = Vec::new();
    let display_status = entry
        .map(|entry| entry.runtime.display_status)
        .or_else(|| detail.map(|detail| detail.runtime.display_status));
    match entry.map(Indicator::for_entry) {
        Some(indicator) if indicator.label().is_some() => spans.push(Span::styled(
            format!(
                "{} {}",
                indicator.glyph(state.frame),
                indicator.label().unwrap_or_default()
            ),
            indicator.style(),
        )),
        _ => spans.push(Span::styled(
            display_status
                .map_or("loading…", display_status_label)
                .to_owned(),
            muted,
        )),
    }
    let thread = entry
        .map(|entry| &entry.thread)
        .or_else(|| detail.map(|detail| &detail.thread));
    let mut parts: Vec<String> = Vec::new();
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
        parts.push(project);
        parts.push(thread.provider_id.clone());
    }
    if let Some(branch) = entry.and_then(|entry| entry.environment_branch_name.as_deref()) {
        parts.push(format!("⎇ {branch}"));
    }
    if let Some(thread) = thread {
        if thread.is_pinned() {
            parts.push("pinned".into());
        }
        if thread.is_archived() {
            parts.push("archived".into());
        }
        if !crate::indicator::is_thread_read(thread) {
            parts.push("unread".into());
        }
        parts.push(format!(
            "updated {} ago",
            compact_relative_time(thread.updated_at, state.now_ms)
        ));
    }
    if let Some(timeline) = state.timeline()
        && timeline.scroll != Scroll::Bottom
    {
        parts.push("scrolled".into());
    }
    for part in parts {
        spans.push(Span::styled(" · ", muted));
        spans.push(Span::styled(part, muted));
    }
    Line::from(crate::wrap::truncate_spans(spans, width))
}

fn render_timeline(state: &AppState, frame: &mut Frame, area: Rect, focused: bool) {
    let muted = Style::new().fg(Color::DarkGray);
    let Some(timeline) = state.timeline() else {
        return;
    };
    if area.height == 0 || area.width < 2 {
        return;
    }
    if !timeline.has_rows() && timeline.items.is_empty() {
        let (text, style) = if let Some(error) = timeline.error() {
            (
                format!("Failed to load timeline: {error}"),
                Style::new().fg(Color::Red),
            )
        } else if timeline.is_loading() {
            ("Loading…".to_owned(), muted)
        } else {
            ("No messages yet".to_owned(), muted)
        };
        frame.render_widget(
            Paragraph::new(text).style(style).wrap(Wrap { trim: false }),
            area,
        );
        return;
    }
    let width = usize::from(area.width) - 1;
    let height = usize::from(area.height);
    timeline.viewport.set((width, height));
    let placements = timeline.layout(width, height);
    let clock = TimelineFrame {
        now_ms: state.now_ms,
        tick: state.frame,
    };
    let selected = timeline.selected_index();
    let gutter_style = if focused {
        Style::new().fg(Color::Cyan)
    } else {
        muted
    };
    let mut y = area.y;
    for placement in placements {
        let lines = timeline.item_lines(placement.index, width, clock);
        let is_selected = selected == Some(placement.index);
        let gap = usize::from(timeline.items[placement.index].gap);
        for (line_index, line) in lines
            .into_iter()
            .enumerate()
            .skip(placement.skip)
            .take(placement.take)
        {
            let marked = is_selected && line_index >= gap;
            let gutter = if marked { "▌" } else { " " };
            let buffer = frame.buffer_mut();
            buffer.set_string(area.x, y, gutter, gutter_style);
            buffer.set_line(area.x + 1, y, &line, area.width - 1);
            if marked && focused && line_index == gap {
                buffer.set_style(
                    Rect {
                        x: area.x + 1,
                        y,
                        width: area.width - 1,
                        height: 1,
                    },
                    Style::new().bg(Color::Indexed(236)),
                );
            }
            y += 1;
        }
    }
    if timeline.unseen && timeline.scroll != Scroll::Bottom {
        let label = format!(
            " ↓ new activity · {} to follow ",
            state.keymap.label_for(Command::MoveBottom)
        );
        let label = truncate(&label, usize::from(area.width));
        let label_width = label.width() as u16;
        let rect = Rect {
            x: area.right().saturating_sub(label_width),
            y: area.bottom() - 1,
            width: label_width,
            height: 1,
        };
        frame.render_widget(Clear, rect);
        frame.render_widget(
            Paragraph::new(label).style(Style::new().bg(Color::Yellow).fg(Color::Black)),
            rect,
        );
    }
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
        Mode::OpenPicker { .. } => "1-9/Enter open · j/k move · Esc cancel".into(),
        Mode::Picker(_) => "1-9/Enter choose · j/k move · Esc cancel".into(),
        Mode::Queue { .. } => {
            "j/k select · Enter/e edit · s send now · r retry · d delete · Esc close".into()
        }
        Mode::Normal if state.focus == Focus::Composer => format!(
            "{} send · {} newline · ↑/↓ history · C-s steer · Esc Esc stop · {} model · {} queue",
            state.submit_key.submit_label(),
            state.submit_key.newline_label(),
            key(Command::ModelPicker),
            key(Command::QueueList),
        ),
        Mode::Normal if state.focus == Focus::Main && state.open_thread.is_some() => format!(
            "{}/{} select · {} expand · {} copy · {} open · {}/{} messages · {} follow",
            key(Command::MoveDown),
            key(Command::MoveUp),
            key(Command::Open),
            key(Command::Copy),
            key(Command::OpenExternal),
            key(Command::PreviousMessage),
            key(Command::NextMessage),
            key(Command::MoveBottom),
        ),
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
    if state.read_only {
        spans.push(Span::styled("  read-only", Style::new().fg(Color::Magenta)));
    }
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
    let composer = [
        (
            state.submit_key.submit_label().to_owned(),
            "Composer: send (queues while running)".to_owned(),
        ),
        (
            state.submit_key.newline_label().to_owned(),
            "Composer: new line".to_owned(),
        ),
        (
            "C-s".to_owned(),
            "Composer: steer the running turn".to_owned(),
        ),
        ("↑/↓".to_owned(), "Composer: prompt history".to_owned()),
        (
            "Esc Esc".to_owned(),
            "Composer: stop the running turn".to_owned(),
        ),
        ("C-c".to_owned(), "Composer: clear, then stop".to_owned()),
    ];
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
        .chain(composer)
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
    let per_column = usize::from(area.height.saturating_sub(2)).max(1);
    let columns = entries.len().div_ceil(per_column).max(1);
    let rows = entries.len().div_ceil(columns);
    let fit = usize::from(area.width.saturating_sub(2)) / columns;
    let desc_width = desc_width.min(fit.saturating_sub(key_width + 4)).max(1);
    let column_width = key_width + desc_width + 4;
    let width = ((column_width * columns + 2) as u16).min(area.width);
    let height = (rows as u16 + 2).min(area.height);
    let rect = Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    };
    let lines: Vec<Line> = (0..rows)
        .map(|row| {
            let mut spans = Vec::new();
            for column in 0..columns {
                let Some((key, desc)) = entries.get(column * rows + row) else {
                    continue;
                };
                spans.push(Span::styled(
                    format!(" {key:<key_width$}  "),
                    Style::new().fg(Color::Cyan),
                ));
                spans.push(Span::raw(format!(
                    "{:<desc_width$} ",
                    truncate(desc, desc_width)
                )));
            }
            Line::from(spans)
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
