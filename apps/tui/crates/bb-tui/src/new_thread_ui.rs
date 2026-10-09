use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};
use unicode_width::UnicodeWidthStr;

use crate::app::{AppState, Focus};
use crate::compose::PickerItem;
use crate::composer_ui::{MAX_EDITOR_LINES, render_item_list};
use crate::execution::Load;
use crate::new_thread::{FormField, NewThreadForm, Resolved};
use crate::ui::truncate;

const LABEL_WIDTH: usize = 13;

fn muted() -> Style {
    Style::new().fg(Color::DarkGray)
}

fn field_value(
    form: &NewThreadForm,
    resolved: &Resolved,
    field: FormField,
) -> (String, Option<String>) {
    let options = &resolved.options;
    let loading_models = options.loading_models && options.provider_id.is_empty();
    match field {
        FormField::Project => (resolved.project_label.clone(), None),
        FormField::Environment => {
            if resolved.environment_providers.is_none() {
                let text = match &form.registered {
                    Load::Failed(error) => format!("failed to load: {error}"),
                    _ => "loading…".into(),
                };
                return (text, None);
            }
            let note = if resolved.machine_unavailable {
                Some("not available on this machine".to_owned())
            } else {
                resolved.inputs_blocker.clone()
            };
            (resolved.environment_label(), note)
        }
        FormField::Branch => {
            let label = resolved.branch_label(form).unwrap_or_default();
            let note = match form.branches.as_ref().map(|(_, load)| load) {
                Some(Load::Loading) => Some("loading branches…".to_owned()),
                Some(Load::Failed(error)) => Some(format!("branches failed: {error}")),
                _ => resolved
                    .checkout_state
                    .as_ref()
                    .and_then(crate::creation::checkout_blocker)
                    .map(|blocker| blocker.label),
            };
            (label, note)
        }
        FormField::Provider if loading_models => ("loading…".into(), None),
        FormField::Provider => (options.provider_label.clone(), None),
        FormField::Model if options.model.is_empty() && options.loading_models => {
            ("loading…".into(), None)
        }
        FormField::Model => {
            let note = options
                .unavailable_model_recovery
                .then(|| "previous model unavailable".to_owned());
            (options.model_label.clone(), note)
        }
        FormField::Reasoning => {
            let label = options
                .reasoning_options
                .iter()
                .find(|option| option.value == options.reasoning)
                .map_or_else(
                    || options.reasoning.to_string(),
                    |option| option.label.clone(),
                );
            (label, None)
        }
        FormField::Permission => (
            crate::prompt::permission_mode_label(options.permission).to_owned(),
            None,
        ),
        FormField::Prompt => {
            let lines = form.composer.line_count();
            let text = if form.composer.is_empty() {
                "empty".to_owned()
            } else {
                format!("{lines} line(s)")
            };
            (text, None)
        }
    }
}

fn field_line(
    form: &NewThreadForm,
    resolved: &Resolved,
    field: FormField,
    focused: bool,
    width: usize,
) -> Line<'static> {
    let selected =
        focused && form.field == field && !(field == FormField::Prompt && form.prompt_active);
    let (value, note) = field_value(form, resolved, field);
    let marker = if selected { "▸ " } else { "  " };
    let label_style = if selected {
        Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD)
    } else {
        muted()
    };
    let value_style = if selected {
        Style::new().add_modifier(Modifier::REVERSED)
    } else {
        Style::new()
    };
    let mut spans = vec![
        Span::styled(marker.to_owned(), Style::new().fg(Color::Cyan)),
        Span::styled(format!("{:<LABEL_WIDTH$}", field.label()), label_style),
    ];
    let used = 2 + LABEL_WIDTH;
    let available = width.saturating_sub(used);
    let value = truncate(&value, available);
    let value_width = value.width();
    spans.push(Span::styled(value, value_style));
    if let Some(note) = note {
        let room = available.saturating_sub(value_width + 2);
        if room > 3 {
            spans.push(Span::styled(
                format!("  {}", truncate(&note, room)),
                Style::new().fg(Color::Yellow),
            ));
        }
    }
    Line::from(spans)
}

pub fn render_form(state: &AppState, form: &NewThreadForm, frame: &mut Frame, area: Rect) {
    let focused = state.focus == Focus::NewThread;
    let resolved = form.resolve(&state.snapshot);
    let style = if focused {
        Style::new().fg(Color::Cyan)
    } else {
        muted()
    };
    let block = Block::bordered()
        .border_type(if focused {
            BorderType::Thick
        } else {
            BorderType::Plain
        })
        .border_style(style)
        .title(Span::styled(
            " New thread ",
            if focused {
                style.add_modifier(Modifier::BOLD)
            } else {
                Style::new()
            },
        ));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height < 4 || inner.width < 10 {
        return;
    }
    let width = usize::from(inner.width);
    let fields: Vec<FormField> = FormField::ALL
        .into_iter()
        .filter(|field| *field != FormField::Prompt && form.field_visible(&resolved, *field))
        .collect();
    let mut lines: Vec<Line> = fields
        .iter()
        .map(|field| field_line(form, &resolved, *field, focused, width))
        .collect();
    lines.push(Line::raw(""));
    let status = if let Some(error) = &form.error {
        Line::styled(
            truncate(&format!("✗ {error}"), width),
            Style::new().fg(Color::Red),
        )
    } else if let Some(reason) = &resolved.submit_reason {
        Line::styled(
            truncate(&format!("• {reason}"), width),
            Style::new().fg(Color::Yellow),
        )
    } else {
        Line::styled(
            truncate(
                &format!(
                    "✓ Ready — {} in the prompt creates the thread",
                    state.submit_key.submit_label()
                ),
                width,
            ),
            Style::new().fg(Color::Green),
        )
    };
    lines.push(status);
    let header = lines.len() as u16;
    let editor = (form.composer.line_count().clamp(3, MAX_EDITOR_LINES)) as u16;
    let [fields_area, border_area, editor_area, _] = Layout::vertical([
        Constraint::Length(header.min(inner.height.saturating_sub(2))),
        Constraint::Length(1),
        Constraint::Min(editor.min(inner.height.saturating_sub(header + 1).max(1))),
        Constraint::Fill(1),
    ])
    .areas(inner);
    frame.render_widget(Paragraph::new(lines), fields_area);
    let prompt_focused = focused && form.prompt_active && form.picker.is_none();
    let prompt_selected = focused && form.field == FormField::Prompt;
    let border_style = if prompt_focused || prompt_selected {
        Style::new().fg(Color::Cyan)
    } else {
        muted()
    };
    frame.render_widget(
        Block::new()
            .borders(Borders::TOP)
            .border_type(if prompt_focused {
                BorderType::Thick
            } else {
                BorderType::Plain
            })
            .border_style(border_style)
            .title(Span::styled(
                if prompt_selected && !prompt_focused {
                    " ▸ Prompt (Enter to edit) "
                } else {
                    " Prompt "
                },
                border_style,
            )),
        border_area,
    );
    if prompt_focused {
        frame.render_widget(form.composer.textarea(), editor_area);
    } else {
        let mut textarea = form.composer.textarea().clone();
        textarea.set_cursor_style(Style::default());
        frame.render_widget(&textarea, editor_area);
    }
}

pub fn render_form_picker(form: &NewThreadForm, frame: &mut Frame, area: Rect) {
    let Some(picker) = &form.picker else {
        return;
    };
    let visible = picker.visible();
    let items: Vec<&PickerItem> = visible.iter().map(|index| &picker.items[*index]).collect();
    render_item_list(
        &picker.title,
        &items,
        picker.selected,
        Some(picker.filter.text()),
        frame,
        area,
    );
}

pub fn hints(form: &NewThreadForm, submit_label: &str) -> String {
    if form.picker.is_some() {
        return "type to filter · ↑/↓ move · Enter choose · Esc cancel".into();
    }
    if form.prompt_active {
        return format!(
            "{submit_label} create · Tab/S-Tab fields · M-m model · M-e reasoning · M-p permissions · Esc fields"
        );
    }
    "j/k field · Enter pick · i prompt · Esc close".into()
}
