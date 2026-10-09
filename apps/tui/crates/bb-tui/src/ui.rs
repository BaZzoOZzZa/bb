use bb_store::ConnectionState;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph, Wrap};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::{AppState, Focus, Mode, ToastLevel};
use crate::composer::EditMode;
use crate::composer_ui;
use crate::evil::Tag;
use crate::indicator::{Indicator, display_status_label};
use crate::input::LineInput;
use crate::keymap::Command;
use crate::sidebar::{GroupRow, SidebarRow, ThreadRow};
use crate::time::compact_relative_time;
use crate::timeline::{Frame as TimelineFrame, Scroll};

const SIDEBAR_MIN: u16 = 24;
const SIDEBAR_MAX: u16 = 40;
const MAIN_MIN: u16 = 40;
const PANEL_MIN: u16 = 28;

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
    state.mouse_areas.clear();
    render_frame(state, frame);
    crate::theme::apply(state.theme, frame.buffer_mut());
}

fn render_frame(state: &AppState, frame: &mut Frame) {
    let area = frame.area();
    let banner = banner_text(state);
    let [banner_area, body, status, echo] = Layout::vertical([
        Constraint::Length(u16::from(banner.is_some())),
        Constraint::Fill(1),
        Constraint::Length(1),
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

    let sidebar_width = adjust(
        (body.width * 3 / 10).clamp(SIDEBAR_MIN, SIDEBAR_MAX),
        state.sidebar_width_delta,
        body.width / 2,
    );
    let panel_width = adjust(
        match state.panel.tab {
            crate::panel::PanelTab::Info => (body.width * 3 / 10).clamp(28, 56),
            _ => (body.width / 2).clamp(40, 120),
        },
        state.panel_width_delta,
        body.width.saturating_sub(MAIN_MIN),
    );
    let maximized = state.panel_visible && state.panel.maximized;
    let crowded = state.panel_visible
        && !maximized
        && state.focus != Focus::Sidebar
        && body.width.saturating_sub(sidebar_width + panel_width) < MAIN_MIN;
    let show_sidebar = state.sidebar_visible && !crowded;
    let panel_width = if state.panel_visible && !maximized {
        let room = body
            .width
            .saturating_sub(if show_sidebar { sidebar_width } else { 0 });
        panel_width.min(room.saturating_sub(MAIN_MIN).max(PANEL_MIN.min(room)))
    } else {
        panel_width
    };
    let [sidebar_area, main_area, panel_area] = Layout::horizontal([
        Constraint::Length(if show_sidebar { sidebar_width } else { 0 }),
        if maximized {
            Constraint::Length(0)
        } else {
            Constraint::Fill(1)
        },
        if maximized {
            Constraint::Fill(1)
        } else {
            Constraint::Length(if state.panel_visible { panel_width } else { 0 })
        },
    ])
    .areas(body);

    if show_sidebar {
        render_sidebar(state, frame, sidebar_area);
    }
    if !maximized {
        render_main(state, frame, main_area);
    }
    if state.panel_visible {
        crate::panel_ui::render_panel(state, frame, panel_area);
        state.mouse_areas.panel.set(Some(panel_area));
    }
    if let Some(view) = &state.settings {
        crate::settings_ui::render_settings(state, view, frame, body);
    }
    if let Some(view) = &state.automations {
        crate::automations_ui::render_automations(state, view, frame, body);
    }
    render_status(state, frame, status);
    render_echo(state, frame, echo);
    render_toasts(state, frame, body);
    if let Mode::OpenPicker { targets, selected } = &state.mode {
        render_open_picker(targets, *selected, frame, area);
    }
    if let Mode::Picker(picker) = &state.mode {
        composer_ui::render_picker(picker, frame, area);
    }
    crate::typeahead_ui::render_attach_modes(state, frame, area);
    if let Some(form) = state.new_thread_form() {
        crate::new_thread_ui::render_form_picker(form, frame, area);
    }
    if let Some(palette) = &state.palette {
        crate::palette_ui::render_palette(palette, frame, body);
    }
    if let Some(pane) = &state.result_pane {
        crate::palette_ui::render_pane(pane, frame, body);
    }
    if let Some(confirm) = &state.panel.confirm {
        crate::panel_ui::render_confirm(confirm, frame, body);
    }
    crate::notifications_ui::render_notifications(state, frame, body);
    crate::which_key::render(state, frame, body);
}

fn adjust(base: u16, delta: i16, max: u16) -> u16 {
    let value = i32::from(base) + i32::from(delta);
    value.clamp(12, i32::from(max.max(12))) as u16
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
    state.mouse_areas.sidebar.set(Some((list_area, offset)));

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
        let mut line = match row {
            SidebarRow::Group(group) => group_line(group, width),
            SidebarRow::Thread(thread) => thread_line(state, thread, width, renaming),
        };
        if selected {
            for span in &mut line.spans {
                if span.style.fg == Some(Color::DarkGray) {
                    span.style.fg = Some(Color::Gray);
                }
            }
        }
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
    let arrow = match (group.thread_count, group.collapsed) {
        (0, _) => "  ",
        (_, true) => "▸ ",
        (_, false) => "▾ ",
    };
    let count = match group.thread_count {
        0 => "no threads".to_owned(),
        count => count.to_string(),
    };
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
    if let Some(form) = state.new_thread_form() {
        crate::new_thread_ui::render_form(state, form, frame, area);
        return;
    }
    let focused = state.focus == Focus::Main;
    if state.is_split() && !state.split_maximized {
        let constraints = vec![Constraint::Fill(1); state.splits.len()];
        let columns = Layout::horizontal(constraints).split(area);
        for (index, (thread_id, column)) in state.splits.iter().zip(columns.iter()).enumerate() {
            render_thread_pane(state, frame, *column, thread_id, Some(index + 1));
        }
        return;
    }
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
    let number = state.is_split().then(|| state.focused_pane() + 1);
    render_thread_pane(state, frame, area, thread_id, number);
}

fn render_thread_pane(
    state: &AppState,
    frame: &mut Frame,
    area: Rect,
    thread_id: &str,
    number: Option<usize>,
) {
    let is_open = state.open_thread.as_deref() == Some(thread_id);
    let focused = is_open && state.focus == Focus::Main;
    let mut title = state.thread_title(thread_id);
    if let Some(number) = number {
        title = format!("{number} · {title}");
        if state.split_maximized {
            title = format!("{title} · {} of {}", number, state.splits.len());
        }
    }
    let block = pane_block(&title, focused);
    let block = if is_open && state.is_split() && !focused {
        block.border_style(Style::new().fg(Color::Blue))
    } else {
        block
    };
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height == 0 || inner.width < 4 {
        return;
    }
    let [meta_area, body] =
        Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).areas(inner);
    let meta = meta_line(state, thread_id, usize::from(meta_area.width));
    frame.render_widget(Paragraph::new(meta), meta_area);
    let record = |timeline: Rect, composer: Option<Rect>, interaction: Option<Rect>| {
        state.mouse_areas.record_pane(crate::mouse::PaneArea {
            thread_id: thread_id.to_owned(),
            pane: number.filter(|_| state.is_split()).map(|number| number - 1),
            area,
            timeline,
            composer,
            interaction,
        });
    };
    if !is_open {
        if state.interaction_panel_visible(thread_id) && body.height >= 3 {
            let [timeline_area, notice] =
                Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(body);
            render_timeline(state, frame, timeline_area, thread_id, false);
            record(timeline_area, None, None);
            frame.render_widget(
                Paragraph::new(truncate(
                    "? Waiting for your answer · focus this pane to respond",
                    usize::from(notice.width),
                ))
                .style(Style::new().fg(Color::Yellow)),
                notice,
            );
            return;
        }
        if state.thread_archived(thread_id)
            || !state.composers.contains_key(thread_id)
            || body.height < 6
        {
            render_timeline(state, frame, body, thread_id, false);
            record(body, None, None);
            return;
        }
        let wanted = composer_ui::composer_height(state, thread_id, usize::from(body.width));
        let height = wanted.min(body.height.saturating_sub(3).max(body.height / 2));
        let [timeline_area, composer_area] =
            Layout::vertical([Constraint::Fill(1), Constraint::Length(height)]).areas(body);
        render_timeline(state, frame, timeline_area, thread_id, false);
        composer_ui::render_composer(state, frame, composer_area, thread_id);
        record(timeline_area, Some(composer_area), None);
        return;
    }
    if state.interaction_panel_visible(thread_id) && body.height >= 6 {
        let wanted = crate::interaction_ui::panel_height(state, thread_id, usize::from(body.width));
        let height = wanted.max(3).min((body.height * 2 / 3).max(5));
        let [timeline_area, panel_area] =
            Layout::vertical([Constraint::Fill(1), Constraint::Length(height)]).areas(body);
        render_timeline(state, frame, timeline_area, thread_id, focused);
        crate::interaction_ui::render_panel(state, frame, panel_area, thread_id);
        record(timeline_area, None, Some(panel_area));
        return;
    }
    if !state.composer_available() || body.height < 6 {
        render_timeline(state, frame, body, thread_id, focused);
        record(body, None, None);
        return;
    }
    let wanted = composer_ui::composer_height(state, thread_id, usize::from(body.width));
    let height = wanted.min(body.height.saturating_sub(3).max(body.height / 2));
    let [timeline_area, composer_area] =
        Layout::vertical([Constraint::Fill(1), Constraint::Length(height)]).areas(body);
    render_timeline(state, frame, timeline_area, thread_id, focused);
    composer_ui::render_composer(state, frame, composer_area, thread_id);
    record(timeline_area, Some(composer_area), None);
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
    if let Some(timeline) = state.timelines.get(thread_id)
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

fn render_timeline(
    state: &AppState,
    frame: &mut Frame,
    area: Rect,
    thread_id: &str,
    focused: bool,
) {
    let muted = Style::new().fg(Color::DarkGray);
    let Some(timeline) = state.timelines.get(thread_id) else {
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

fn hints(state: &AppState) -> String {
    if let Some(hint) = crate::palette_ui::hints(state.palette.as_ref(), state.result_pane.as_ref())
    {
        return hint;
    }
    if let Some(view) = &state.automations {
        return crate::automations::automations_hints(view);
    }
    if let Some(view) = &state.settings {
        return crate::settings::settings_hints(view);
    }
    match &state.mode {
        Mode::Ex(_) => String::new(),
        Mode::Filter(_) => "type to filter · RET keep · ESC clear".into(),
        Mode::Rename { .. } => "RET save · ESC cancel".into(),
        Mode::OpenPicker { .. } => "1-9/RET open · j/k move · ESC cancel".into(),
        Mode::Picker(_) => "1-9/RET choose · j/k move · ESC cancel".into(),
        Mode::Queue { .. } => {
            "j/k select · RET/e edit · s send now · r retry · d delete · ESC close".into()
        }
        Mode::AttachPrompt { .. } => "RET attach · TAB complete path · ESC cancel".into(),
        Mode::Attachments { .. } => "j/k select · d/x/Del remove · ESC close".into(),
        Mode::Notifications { .. } => {
            "RET open thread · j/k move · dd delete · C clear all · q close".into()
        }
        Mode::Normal if state.typeahead_visible() => {
            "↑/↓ select · RET/TAB insert · ESC dismiss".into()
        }
        Mode::Normal if state.focus == Focus::NewThread && state.new_thread_form().is_some() => {
            crate::new_thread_ui::hints(
                state.new_thread_form().expect("form"),
                state.submit_key.submit_label(),
            )
        }
        Mode::Normal if state.panel.confirm.is_some() => "y/RET confirm · ESC cancel".into(),
        Mode::Normal if state.focus == Focus::Panel && state.panel_visible => {
            crate::panel_ui::hints(state)
        }
        Mode::Normal if state.focus == Focus::Interaction => {
            "ESC leave · C-k timeline · M-w next waiting request · M-SPC leader".into()
        }
        Mode::Normal if state.focus == Focus::Composer => composer_hints(state),
        Mode::Normal if state.focus == Focus::Main && state.open_thread.is_some() => {
            "j/k rows · RET toggle · ]m/[m messages · gy yank · gx open · i write · SPC leader"
                .into()
        }
        Mode::Normal if state.focus == Focus::Sidebar => {
            "j/k move · RET open · go focus · R rename · dd archive · g/ filter · SPC leader".into()
        }
        Mode::Normal => "SPC leader · : ex · SPC h b b keys · SPC q q quit".into(),
    }
}

fn composer_hints(state: &AppState) -> String {
    match state.focused_composer().map(|composer| composer.mode) {
        Some(EditMode::Insert) => format!(
            "{} send · {} newline · ESC normal · C-c C-s steer · C-c C-k stop · @ mention",
            state.submit_key.submit_label(),
            state.submit_key.newline_label(),
        ),
        Some(EditMode::Visual { .. }) => {
            "d delete · y yank · c change · > indent · o other end · ESC normal".into()
        }
        _ => "i insert · RET send · u undo · p put · C-k timeline · SPC leader".into(),
    }
}

fn buffer_label(state: &AppState) -> (String, String) {
    if let Some(pane) = &state.result_pane {
        return (pane.title.clone(), String::new());
    }
    if let Some(view) = &state.settings {
        return ("*settings*".into(), view.section.label().to_owned());
    }
    if state.automations.is_some() {
        return ("*automations*".into(), String::new());
    }
    if matches!(state.mode, Mode::Notifications { .. }) {
        return ("*notifications*".into(), String::new());
    }
    let title = |state: &AppState| {
        state
            .open_thread
            .as_deref()
            .map(|thread_id| state.thread_title(thread_id))
            .unwrap_or_default()
    };
    match state.focus {
        Focus::Sidebar if state.sidebar_visible => (
            "*threads*".into(),
            format!(
                "{}/{}",
                (state.selected + 1).min(state.rows.len()),
                state.rows.len()
            ),
        ),
        Focus::Panel if state.panel_visible => (
            format!("*panel: {}*", state.panel.tab.title()),
            String::new(),
        ),
        Focus::NewThread => ("*new thread*".into(), String::new()),
        Focus::Composer => {
            let position = state
                .focused_composer()
                .map(|composer| {
                    let cursor = composer.textarea().cursor();
                    format!("{}:{}", cursor.0 + 1, cursor.1 + 1)
                })
                .unwrap_or_default();
            (format!("{} [message]", title(state)), position)
        }
        Focus::Interaction => (format!("{} [request]", title(state)), String::new()),
        _ => {
            let position = state
                .timeline()
                .map(|timeline| {
                    if timeline.scroll == Scroll::Bottom {
                        "Bot".to_owned()
                    } else {
                        match timeline.selected_index() {
                            Some(index) => format!("{}/{}", index + 1, timeline.items.len()),
                            None => "Top".to_owned(),
                        }
                    }
                })
                .unwrap_or_default();
            (title(state), position)
        }
    }
}

fn tag_color(tag: Tag) -> Color {
    match tag {
        Tag::Normal => Color::Green,
        Tag::Insert => Color::Blue,
        Tag::Visual | Tag::VisualLine => Color::Magenta,
        Tag::Operator => Color::Cyan,
        Tag::Replace => Color::Red,
        Tag::Command => Color::Yellow,
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
            truncate(&server, width / 4),
            Style::new().bg(Color::DarkGray).fg(Color::White),
        ),
        Span::raw(" "),
        connection_span(&state.snapshot.connection),
    ];
    if state.read_only {
        spans.push(Span::styled("  read-only", Style::new().fg(Color::Magenta)));
    }
    if state.waiting > 0 {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            format!("? {} waiting", state.waiting),
            Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        ));
    }
    if state.attention > 0 {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            format!("⚑ {}", state.attention),
            Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        ));
    } else if state
        .snapshot
        .attention
        .data
        .as_ref()
        .is_some_and(|attention| attention.has_attention)
    {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            "⚑",
            Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        ));
    }
    let unread = state.notifications.unread();
    if unread > 0 {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            format!("! {unread} new"),
            Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        ));
    }
    let tag = crate::evil::tag(state);
    let tag_text = format!(" {} ", tag.label());
    let showcmd = state.keys.showcmd();
    let (buffer, position) = buffer_label(state);
    let mut right = Vec::new();
    if !showcmd.is_empty() {
        right.push(Span::styled(
            format!("{showcmd} "),
            Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        ));
    }
    if !position.is_empty() {
        right.push(Span::styled(
            format!("{position} "),
            Style::new().fg(Color::Gray),
        ));
    }
    right.push(Span::styled(
        tag_text,
        Style::new()
            .bg(tag_color(tag))
            .fg(Color::Black)
            .add_modifier(Modifier::BOLD),
    ));
    let right_width: usize = right.iter().map(|span| span.content.width()).sum();
    let used: usize = spans.iter().map(|span| span.content.width()).sum();
    let room = width.saturating_sub(used + right_width + 3);
    if room > 4 && !buffer.is_empty() {
        let buffer = truncate(&buffer, room);
        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            buffer,
            Style::new().fg(Color::White).add_modifier(Modifier::BOLD),
        ));
    }
    let used: usize = spans.iter().map(|span| span.content.width()).sum();
    pad_to(&mut spans, used, width.saturating_sub(right_width));
    spans.extend(right);
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn render_echo(state: &AppState, frame: &mut Frame, area: Rect) {
    let width = usize::from(area.width);
    if let Mode::Ex(line) = &state.mode {
        let text = format!(":{}", line.input.text());
        frame.render_widget(Paragraph::new(truncate(&text, width)), area);
        let x = area.x + 1 + line.input.cursor_width() as u16;
        if x < area.right() {
            frame.set_cursor_position((x, area.y));
        }
        return;
    }
    if let Some(echo) = &state.keys.echo {
        let style = if echo.error {
            Style::new().fg(Color::Red).add_modifier(Modifier::BOLD)
        } else {
            Style::new()
        };
        frame.render_widget(
            Paragraph::new(truncate(&echo.text, width)).style(style),
            area,
        );
        return;
    }
    frame.render_widget(
        Paragraph::new(truncate(&hints(state), width)).style(Style::new().fg(Color::DarkGray)),
        area,
    );
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
            ToastLevel::Attention => Style::new().bg(Color::Yellow).fg(Color::Black),
            ToastLevel::Error => Style::new().bg(Color::Red).fg(Color::White),
        };
        frame.render_widget(Clear, rect);
        frame.render_widget(Paragraph::new(text).style(style), rect);
    }
}
