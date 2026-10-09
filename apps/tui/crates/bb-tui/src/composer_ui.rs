use bb_wire::timeline::{GoalStatus, PendingTodoStatus};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};
use unicode_width::UnicodeWidthStr;

use crate::app::{AppState, Focus, Mode};
use crate::compose::{Picker, PickerItem};
use crate::execution::Load;
use crate::keymap::Command;
use crate::prompt::{
    BlockedReason, SubmitMode, fallback_model_label, format_queued_message_preview,
    waiting_on_label,
};
use crate::typeahead::ComposerTarget;
use crate::ui::truncate;
use crate::wrap::truncate_spans;

pub const MAX_EDITOR_LINES: usize = 8;
const QUEUE_ROWS: usize = 3;
const QUEUE_ROWS_FOCUSED: usize = 6;

fn muted() -> Style {
    Style::new().fg(Color::DarkGray)
}

fn first_line(text: &str) -> &str {
    text.trim().lines().next().unwrap_or_default()
}

pub fn compact_tokens(value: f64) -> String {
    let value = value.max(0.0);
    if value >= 1_000_000.0 {
        format!("{:.1}M", value / 1_000_000.0)
    } else if value >= 1_000.0 {
        format!("{}k", (value / 1_000.0).round() as u64)
    } else {
        format!("{}", value.round() as u64)
    }
}

pub fn format_duration(seconds: f64) -> String {
    if seconds < 60.0 {
        return format!("{}s", seconds.round() as u64);
    }
    let minutes = (seconds / 60.0).floor() as u64;
    if minutes < 60 {
        let rest = (seconds % 60.0).round() as u64;
        return if rest > 0 {
            format!("{minutes}m {rest}s")
        } else {
            format!("{minutes}m")
        };
    }
    let hours = minutes / 60;
    let rest = minutes % 60;
    if rest > 0 {
        format!("{hours}h {rest}m")
    } else {
        format!("{hours}h")
    }
}

fn group_thousands(value: f64) -> String {
    let digits = (value.max(0.0).round() as u64).to_string();
    let mut out = String::new();
    for (index, ch) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

fn label(text: &str, color: Color) -> Span<'static> {
    Span::styled(
        text.to_owned(),
        Style::new().fg(color).add_modifier(Modifier::BOLD),
    )
}

pub fn banner_lines(state: &AppState, thread_id: &str, width: usize) -> Vec<Line<'static>> {
    let mut lines: Vec<Vec<Span<'static>>> = Vec::new();
    let head = state.timeline_head(thread_id);
    if state.has_pending_interaction(thread_id) {
        lines.push(vec![
            label("⚑ Waiting for you", Color::Yellow),
            Span::styled(
                " · the agent needs an answer before it can continue",
                muted(),
            ),
        ]);
    }
    if let Some(mode) = head
        .and_then(|head| head.active_prompt_mode.as_ref())
        .filter(|mode| mode.mode == "plan")
    {
        let prompt = first_line(&mode.prompt);
        lines.push(vec![
            label("◆ Plan mode", Color::Magenta),
            Span::styled(
                format!(
                    " · {}",
                    if prompt.is_empty() {
                        "No prompt text."
                    } else {
                        prompt
                    }
                ),
                muted(),
            ),
        ]);
    }
    if let Some(goal) = head
        .and_then(|head| head.goal.as_ref())
        .filter(|goal| goal.status == GoalStatus::Active)
    {
        let objective = first_line(&goal.objective);
        let tokens = match goal.token_budget {
            Some(budget) => format!(
                "{} / {} tokens",
                group_thousands(goal.tokens_used),
                group_thousands(budget)
            ),
            None => format!("{} tokens", group_thousands(goal.tokens_used)),
        };
        lines.push(vec![
            label("◎ Goal", Color::Cyan),
            Span::raw(format!(
                " · {}",
                if objective.is_empty() {
                    "No goal objective."
                } else {
                    objective
                }
            )),
            Span::styled(
                format!(" · {tokens} · {}", format_duration(goal.time_used_seconds)),
                muted(),
            ),
        ]);
    }
    if let Some(todos) = head.and_then(|head| head.pending_todos.as_ref()) {
        let total = todos.items.len();
        let done = todos
            .items
            .iter()
            .filter(|item| item.status == PendingTodoStatus::Completed)
            .count();
        if total > 0 && done < total {
            let current = todos
                .items
                .iter()
                .find(|item| item.status == PendingTodoStatus::InProgress)
                .or_else(|| {
                    todos
                        .items
                        .iter()
                        .find(|item| item.status != PendingTodoStatus::Completed)
                });
            let mut spans = vec![
                label("☐ Todos", Color::Blue),
                Span::styled(format!(" {done}/{total} complete"), muted()),
            ];
            if let Some(item) = current {
                spans.push(Span::raw(format!(" · ▸ {}", first_line(&item.text))));
            }
            lines.push(spans);
        }
    }
    if let (Some(fallback), Some(execution)) = (
        state.model_fallback(thread_id),
        state.execution_state(thread_id),
    ) && execution.fallback_active(Some(fallback))
    {
        lines.push(vec![
            label("⚠ Model fallback", Color::Yellow),
            Span::styled(
                format!(
                    " · Switched from {} to {}",
                    fallback_model_label(&fallback.original_model),
                    fallback_model_label(&fallback.fallback_model)
                ),
                muted(),
            ),
        ]);
    }
    lines
        .into_iter()
        .map(|spans| Line::from(truncate_spans(spans, width)))
        .collect()
}

pub fn queue_lines(state: &AppState, thread_id: &str, width: usize) -> Vec<Line<'static>> {
    let queued = state.queued_messages(thread_id);
    if queued.is_empty() {
        return Vec::new();
    }
    let focused = match &state.mode {
        Mode::Queue {
            thread_id: queue_thread,
            selected,
        } if queue_thread == thread_id => Some((*selected).min(queued.len() - 1)),
        _ => None,
    };
    let capacity = if focused.is_some() {
        QUEUE_ROWS_FOCUSED
    } else {
        QUEUE_ROWS
    };
    let start = focused.map_or(0, |selected| (selected + 1).saturating_sub(capacity));
    let inflight = state.inflight.get(thread_id);
    let editing = state
        .composers
        .get(thread_id)
        .and_then(|composer| composer.editing.as_ref())
        .map(|edit| edit.queued_message_id.as_str());
    let mut header = vec![label(&format!("Queued · {}", queued.len()), Color::Cyan)];
    if focused.is_none() {
        header.push(Span::styled(
            format!(
                " · {} to manage",
                state.keymap.label_for(Command::QueueList)
            ),
            muted(),
        ));
    }
    let mut lines = vec![Line::from(truncate_spans(header, width))];
    for (index, message) in queued.iter().enumerate().skip(start).take(capacity) {
        let selected = focused == Some(index);
        let marker = if selected { "▸ " } else { "  " };
        let preview = format_queued_message_preview(&message.content, false);
        let mut spans = vec![
            Span::raw(marker),
            Span::styled(format!("{}. ", index + 1), muted()),
        ];
        let status = if editing == Some(message.id.as_str()) {
            Some(("✎ editing".to_owned(), Color::Magenta))
        } else if inflight.and_then(|inflight| inflight.processing.as_deref())
            == Some(message.id.as_str())
        {
            Some(("sending…".to_owned(), Color::Cyan))
        } else if let Some(reason) = &message.failure_reason {
            Some((format!("✗ failed: {}", first_line(reason)), Color::Red))
        } else {
            message
                .waiting_on
                .as_ref()
                .map(|waiting| (waiting_on_label(waiting), Color::DarkGray))
        };
        let status_width = status.as_ref().map_or(0, |(text, _)| text.width() + 3);
        let available = width
            .saturating_sub(marker.width() + format!("{}. ", index + 1).width() + status_width);
        spans.push(Span::raw(truncate(&preview, available.max(4))));
        if let Some((text, color)) = status {
            spans.push(Span::styled(" · ", muted()));
            spans.push(Span::styled(text, Style::new().fg(color)));
        }
        let mut line = Line::from(truncate_spans(spans, width));
        if selected {
            line = line.style(Style::new().bg(Color::Blue).fg(Color::White));
        }
        lines.push(line);
    }
    let hidden = queued.len().saturating_sub(start + capacity);
    if hidden > 0 {
        lines.push(Line::from(Span::styled(
            format!("  … {hidden} more"),
            muted(),
        )));
    }
    lines
}

pub fn editor_height(state: &AppState, thread_id: &str, width: usize) -> usize {
    let Some(composer) = state.composers.get(thread_id) else {
        return 1;
    };
    let width = width.max(1);
    let rows: usize = composer
        .textarea()
        .lines()
        .iter()
        .map(|line| line.width().div_ceil(width).max(1))
        .sum();
    rows.clamp(1, MAX_EDITOR_LINES)
}

pub fn composer_height(state: &AppState, thread_id: &str, width: usize) -> u16 {
    let attachments = state.composers.get(thread_id).is_some_and(|composer| {
        !composer.draft().attachments.is_empty() || !composer.uploads.is_empty()
    });
    let total = banner_lines(state, thread_id, width).len()
        + queue_lines(state, thread_id, width).len()
        + usize::from(attachments)
        + 1
        + editor_height(state, thread_id, width.saturating_sub(1))
        + 1;
    total as u16
}

fn submit_hint(state: &AppState, thread_id: &str) -> Vec<Span<'static>> {
    let key = state.submit_key.submit_label();
    let now = state.clock;
    let composer = state.composers.get(thread_id);
    if composer.is_some_and(|composer| composer.escape_armed(now)) {
        return vec![label("Esc again to stop", Color::Yellow)];
    }
    if composer.is_some_and(|composer| composer.editing.is_some()) {
        return vec![Span::styled(format!("{key} save · Esc cancel"), muted())];
    }
    let text = match state.submit_mode(thread_id) {
        SubmitMode::Ready => format!("{key} send"),
        SubmitMode::Queue if state.enter_steers => {
            format!("{key} steer · C-s queue · Esc Esc stop")
        }
        SubmitMode::Queue => format!("{key} queue · C-s steer · Esc Esc stop"),
        SubmitMode::QueueWhileStopping => format!("{key} queue · stopping"),
        SubmitMode::Blocked(BlockedReason::PendingInteraction) => {
            return vec![Span::styled(
                "waiting for your answer",
                Style::new().fg(Color::Yellow),
            )];
        }
        SubmitMode::Blocked(BlockedReason::LoadingExecutionOptions) => "loading options…".into(),
        SubmitMode::Blocked(BlockedReason::Unavailable) => "unavailable".into(),
    };
    vec![Span::styled(text, muted())]
}

fn context_span(state: &AppState, thread_id: &str) -> Option<Span<'static>> {
    let usage = state
        .timeline_head(thread_id)?
        .context_window_usage
        .as_ref()?;
    if usage.model_context_window <= 0.0 {
        return None;
    }
    let percent = (usage.used_tokens / usage.model_context_window * 100.0).clamp(0.0, 999.0);
    let color = if percent >= 95.0 {
        Color::Red
    } else if percent >= 80.0 {
        Color::Yellow
    } else {
        Color::DarkGray
    };
    Some(Span::styled(
        format!(
            "ctx {}{:.0}% {}/{}",
            if usage.estimated { "~" } else { "" },
            percent,
            compact_tokens(usage.used_tokens),
            compact_tokens(usage.model_context_window)
        ),
        Style::new().fg(color),
    ))
}

pub fn footer_line(state: &AppState, thread_id: &str, width: usize) -> Line<'static> {
    let mut left: Vec<Span<'static>> = Vec::new();
    match state.execution_state(thread_id) {
        Some(execution) => {
            let resolved = execution.resolve(state.model_fallback(thread_id));
            let model = if resolved.model.is_empty() {
                match &execution.options {
                    Load::Failed(_) => "models unavailable".to_owned(),
                    Load::Loaded(_) => "default model".to_owned(),
                    _ => "loading models…".to_owned(),
                }
            } else {
                resolved.model_label.clone()
            };
            left.push(Span::styled(model, Style::new().fg(Color::Cyan)));
            if !resolved.reasoning_options.is_empty() {
                let reasoning = resolved
                    .reasoning_options
                    .iter()
                    .find(|option| option.value == resolved.reasoning)
                    .map_or_else(
                        || resolved.reasoning.to_string(),
                        |option| option.label.clone(),
                    );
                left.push(Span::styled(" · ", muted()));
                left.push(Span::raw(reasoning));
            }
            left.push(Span::styled(" · ", muted()));
            if state.plan_mode_active(thread_id) {
                left.push(Span::styled("Plan", Style::new().fg(Color::Magenta)));
            } else {
                let option = resolved
                    .permission_options
                    .iter()
                    .find(|option| option.value == resolved.permission);
                let label = option.map_or_else(
                    || crate::prompt::permission_mode_label(resolved.permission).to_owned(),
                    |option| option.label.clone(),
                );
                let style = if resolved.permission == bb_wire::prompt::PermissionMode::Full {
                    Style::new().fg(Color::Red)
                } else {
                    Style::new()
                };
                left.push(Span::styled(label, style));
            }
            if let Some(tier) = resolved
                .service_tier
                .as_deref()
                .filter(|tier| *tier != "default")
            {
                left.push(Span::styled(" · ", muted()));
                left.push(Span::styled(
                    tier.to_owned(),
                    Style::new().fg(Color::Yellow),
                ));
            }
        }
        None => left.push(Span::styled("loading…", muted())),
    }
    let mut right = submit_hint(state, thread_id);
    if let Some(context) = context_span(state, thread_id) {
        right.push(Span::styled(" · ", muted()));
        right.push(context);
    }
    let right_width: usize = right.iter().map(|span| span.content.width()).sum();
    let left = truncate_spans(left, width.saturating_sub(right_width + 2));
    let left_width: usize = left.iter().map(|span| span.content.width()).sum();
    let mut spans = left;
    if left_width + right_width < width {
        spans.push(Span::raw(" ".repeat(width - left_width - right_width)));
        spans.extend(right);
    }
    Line::from(spans)
}

fn editor_title(state: &AppState, thread_id: &str) -> String {
    let composer = state.composers.get(thread_id);
    if composer.is_some_and(|composer| composer.editing.is_some()) {
        return " Editing queued message ".into();
    }
    if let Some((index, total)) = composer.and_then(|composer| composer.history_position()) {
        return format!(" History {index}/{total} ");
    }
    if state.focus == Focus::Composer {
        " Message ".into()
    } else {
        format!(
            " Message · {} to type ",
            state.keymap.label_for(Command::FocusComposer)
        )
    }
}

pub fn render_composer(state: &AppState, frame: &mut Frame, area: Rect, thread_id: &str) {
    if area.height < 3 || area.width < 4 {
        return;
    }
    let width = usize::from(area.width);
    let banners = banner_lines(state, thread_id, width);
    let queue = queue_lines(state, thread_id, width);
    let editor = editor_height(state, thread_id, width.saturating_sub(1)) as u16;
    let attachments = state
        .composers
        .get(thread_id)
        .and_then(|composer| crate::typeahead_ui::attachment_line(state, composer, width));
    let [
        banner_area,
        queue_area,
        attachment_area,
        border_area,
        editor_area,
        footer_area,
    ] = Layout::vertical([
        Constraint::Length(banners.len() as u16),
        Constraint::Length(queue.len() as u16),
        Constraint::Length(u16::from(attachments.is_some())),
        Constraint::Length(1),
        Constraint::Min(editor.min(area.height.saturating_sub(2))),
        Constraint::Length(1),
    ])
    .areas(area);
    frame.render_widget(Paragraph::new(banners), banner_area);
    frame.render_widget(Paragraph::new(queue), queue_area);
    let focused = state.focus == Focus::Composer;
    let border_style = if focused {
        Style::new().fg(Color::Cyan)
    } else {
        muted()
    };
    frame.render_widget(
        Block::new()
            .borders(Borders::TOP)
            .border_type(if focused {
                BorderType::Thick
            } else {
                BorderType::Plain
            })
            .border_style(border_style)
            .title(Span::styled(editor_title(state, thread_id), border_style)),
        border_area,
    );
    if let Some(composer) = state.composers.get(thread_id) {
        crate::typeahead_ui::render_editor(composer, focused, frame, editor_area);
        if let Some(line) = attachments {
            frame.render_widget(Paragraph::new(line), attachment_area);
        }
        if focused {
            crate::typeahead_ui::render_typeahead(
                state,
                &ComposerTarget::Thread(thread_id.to_owned()),
                frame,
                editor_area,
            );
        }
    }
    frame.render_widget(
        Paragraph::new(footer_line(state, thread_id, width)),
        footer_area,
    );
}

pub fn render_picker(picker: &Picker, frame: &mut Frame, area: Rect) {
    let items: Vec<&PickerItem> = picker.items.iter().collect();
    render_item_list(
        picker.kind.title(),
        &items,
        picker.selected,
        None,
        frame,
        area,
    );
}

pub fn render_item_list(
    title: &str,
    items: &[&PickerItem],
    selected: usize,
    filter: Option<&str>,
    frame: &mut Frame,
    area: Rect,
) {
    let label_width = items
        .iter()
        .map(|item| item.label.width())
        .max()
        .unwrap_or(8);
    let wanted = items
        .iter()
        .map(|item| label_width + item.detail.as_ref().map_or(0, |detail| detail.width() + 3) + 6)
        .max()
        .unwrap_or(20)
        .max(filter.map_or(0, |text| text.width() + 12));
    let width = (wanted as u16 + 2)
        .clamp(24, area.width.saturating_sub(4).max(24))
        .min(area.width);
    let filter_rows = u16::from(filter.is_some());
    let visible = usize::from(area.height.saturating_sub(4 + filter_rows)).max(1);
    let height = (items.len().max(1).min(visible) as u16 + 2 + filter_rows).min(area.height);
    let rect = Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    };
    let inner_width = usize::from(width.saturating_sub(2));
    let start = (selected + 1).saturating_sub(visible);
    let mut lines: Vec<Line> = Vec::new();
    if let Some(text) = filter {
        let shown = if text.is_empty() {
            "type to filter"
        } else {
            text
        };
        lines.push(Line::from(vec![
            Span::styled("/ ", Style::new().fg(Color::Cyan)),
            Span::styled(
                truncate(shown, inner_width.saturating_sub(2)),
                if text.is_empty() {
                    muted()
                } else {
                    Style::new()
                },
            ),
        ]));
    }
    if items.is_empty() {
        lines.push(Line::styled("  no matches", muted()));
    }
    lines.extend(
        items
            .iter()
            .enumerate()
            .skip(start)
            .take(visible)
            .map(|(index, item)| {
                let mut style = Style::new();
                if item.disabled {
                    style = style.fg(Color::DarkGray);
                }
                if index == selected {
                    style = style.bg(Color::Blue).fg(Color::White);
                }
                let mark = if item.current { "● " } else { "  " };
                let number = if index < 9 && filter.is_none() {
                    format!("{} ", index + 1)
                } else {
                    "  ".into()
                };
                let mut spans = vec![
                    Span::styled(number, Style::new().fg(Color::Cyan)),
                    Span::styled(mark.to_owned(), Style::new().fg(Color::Green)),
                    Span::styled(format!("{:<label_width$}", item.label), style),
                ];
                if let Some(detail) = &item.detail {
                    spans.push(Span::styled(format!("  {detail}"), muted()));
                }
                Line::from(truncate_spans(spans, inner_width))
            }),
    );
    frame.render_widget(Clear, rect);
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .title(format!(" {title} ")),
        ),
        rect,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn number_formatting() {
        assert_eq!(compact_tokens(84_321.0), "84k");
        assert_eq!(compact_tokens(1_250_000.0), "1.2M");
        assert_eq!(compact_tokens(512.0), "512");
        assert_eq!(format_duration(42.0), "42s");
        assert_eq!(format_duration(125.0), "2m 5s");
        assert_eq!(format_duration(3_600.0), "1h");
        assert_eq!(format_duration(7_500.0), "2h 5m");
        assert_eq!(group_thousands(1_234_567.0), "1,234,567");
    }
}
