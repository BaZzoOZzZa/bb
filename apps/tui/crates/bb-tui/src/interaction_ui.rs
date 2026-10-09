use bb_wire::interaction::{
    ApprovalDecision, ApprovalPayload, ApprovalSubject, PendingInteraction,
};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};
use unicode_segmentation::UnicodeSegmentation;

use crate::app::{AppState, Focus};
use crate::input::LineInput;
use crate::interact::{ApprovalForm, FormBody, InteractionForm, QuestionForm, SecretForm};
use crate::interaction::{
    CancelAction, OTHER_OPTION_LABEL, Question, RequestKind, decision_label, describe_tool_use,
    is_question_answered, ordered_decisions, subject_detail_lines, thread_web_url,
    unwrap_backticked_command,
};
use crate::keymap::Command;
use crate::markdown::render_markdown;
use crate::wrap::{WrapMode, truncate_spans, wrap_plain};

const COMMAND_PREVIEW_LINES: usize = 4;
const PREVIEW_LINES: usize = 8;
const JSON_LINES: usize = 200;

fn muted() -> Style {
    Style::new().fg(Color::DarkGray)
}

fn key_style() -> Style {
    Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD)
}

fn bold() -> Style {
    Style::new().add_modifier(Modifier::BOLD)
}

fn wrapped(text: &str, style: Style, width: usize, indent: usize) -> Vec<Line<'static>> {
    let prefix = [Span::raw(" ".repeat(indent))];
    text.lines()
        .flat_map(|line| wrap_plain(line, style, width, &prefix, WrapMode::Words))
        .collect()
}

pub struct PanelView {
    pub title: Vec<Span<'static>>,
    pub body: Vec<Line<'static>>,
    pub footer: Vec<Line<'static>>,
    pub scrollable: bool,
}

fn input_spans(input: &LineInput, masked: bool, focused: bool, style: Style) -> Vec<Span<'static>> {
    let (before, after) = input.split_at_cursor();
    let show = |text: &str| -> String {
        if masked {
            "•".repeat(text.graphemes(true).count())
        } else {
            text.to_owned()
        }
    };
    let mut spans = vec![Span::styled(show(before), style)];
    if focused {
        let mut graphemes = after.graphemes(true);
        let at = graphemes.next().unwrap_or(" ");
        let rest: String = graphemes.collect();
        spans.push(Span::styled(
            if masked && at != " " {
                "•".to_owned()
            } else {
                at.to_owned()
            },
            style.add_modifier(Modifier::REVERSED),
        ));
        spans.push(Span::styled(show(&rest), style));
    } else {
        spans.push(Span::styled(show(after), style));
    }
    spans
}

fn button_line(items: &[(String, String)], extra: &str) -> Line<'static> {
    let mut spans = Vec::new();
    for (key, label) in items {
        spans.push(Span::styled(format!("[{key}]"), key_style()));
        spans.push(Span::raw(format!(" {label}  ")));
    }
    if !extra.is_empty() {
        spans.push(Span::styled(extra.to_owned(), muted()));
    }
    Line::from(spans)
}

fn decision_key(decision: ApprovalDecision) -> char {
    match decision {
        ApprovalDecision::Deny => 'd',
        ApprovalDecision::AllowForSession => 's',
        ApprovalDecision::AllowOnce | ApprovalDecision::Unknown => 'y',
    }
}

fn approval_body(
    interaction: &PendingInteraction,
    payload: &ApprovalPayload,
    form: &ApprovalForm,
    width: usize,
) -> (String, Vec<Line<'static>>) {
    let mut body = Vec::new();
    let title = match &payload.subject {
        ApprovalSubject::Command { command, .. } => {
            let display = unwrap_backticked_command(
                &bb_view::shell::extract_shell_command_from_string(command)
                    .unwrap_or_else(|| command.clone()),
            );
            let lines: Vec<&str> = display.split('\n').collect();
            let shown = if form.expanded {
                lines.len()
            } else {
                lines.len().min(COMMAND_PREVIEW_LINES)
            };
            for (index, line) in lines.iter().take(shown).enumerate() {
                let prefix = if index == 0 { "$ " } else { "  " };
                body.extend(wrap_plain(
                    &format!("{prefix}{line}"),
                    Style::new().fg(Color::White),
                    width,
                    &[Span::raw("  ")],
                    WrapMode::Chars,
                ));
            }
            let hidden = lines.len() - shown;
            if hidden > 0 {
                body.push(Line::from(Span::styled(
                    if hidden == 1 {
                        "  … 1 more line (e to show)".to_owned()
                    } else {
                        format!("  … {hidden} more lines (e to show)")
                    },
                    muted(),
                )));
            }
            for line in subject_detail_lines(interaction) {
                if line.starts_with("Command: ")
                    || line == format!("Action: {command}")
                    || line == format!("Action: {display}")
                {
                    continue;
                }
                let line = line.strip_prefix("Cwd: ").unwrap_or(&line).to_owned();
                body.extend(wrapped(&line, muted(), width, 2));
            }
            payload
                .reason
                .clone()
                .unwrap_or_else(|| "Do you want to run this command?".to_owned())
        }
        ApprovalSubject::FileChange { .. } | ApprovalSubject::PermissionGrant { .. } => {
            for line in subject_detail_lines(interaction) {
                body.extend(wrapped(&line, muted(), width, 2));
            }
            payload.reason.clone().unwrap_or_else(|| {
                if matches!(payload.subject, ApprovalSubject::FileChange { .. }) {
                    "Do you want to make these changes?".to_owned()
                } else {
                    "Do you want to grant this permission?".to_owned()
                }
            })
        }
        ApprovalSubject::ToolUse {
            tool, presentation, ..
        } => {
            let ask = describe_tool_use(payload.reason.as_deref(), tool, presentation);
            body.extend(wrapped(
                ask.headline.as_deref().unwrap_or(&ask.tool),
                Style::new().fg(Color::White),
                width,
                2,
            ));
            if ask.headline.is_some() {
                body.extend(wrapped(&format!("Tool: {}", ask.tool), muted(), width, 2));
            }
            if let Some(detail) = &ask.detail {
                let rendered = render_markdown(detail, width.saturating_sub(2).max(1), muted());
                body.extend(rendered.lines.into_iter().map(|line| {
                    let mut spans = vec![Span::raw("  ")];
                    spans.extend(line.spans);
                    Line::from(spans)
                }));
            }
            ask.title
        }
        ApprovalSubject::Plan { .. } => payload.reason.clone().unwrap_or_default(),
        ApprovalSubject::Unknown(value) => {
            body.extend(wrapped(
                &serde_json::to_string_pretty(value).unwrap_or_default(),
                muted(),
                width,
                2,
            ));
            payload
                .reason
                .clone()
                .unwrap_or_else(|| "Approval requested".to_owned())
        }
    };
    (title, body)
}

fn approval_footer(decisions: &[ApprovalDecision], plan: bool) -> Vec<Line<'static>> {
    let items: Vec<(String, String)> = decisions
        .iter()
        .map(|decision| {
            (
                decision_key(*decision).to_string(),
                decision_label(*decision, plan).to_owned(),
            )
        })
        .collect();
    vec![button_line(&items, "Esc leave")]
}

fn question_tabs(questions: &[Question], form: &QuestionForm) -> Line<'static> {
    let mut spans = Vec::new();
    for (index, question) in questions.iter().enumerate() {
        let answered = form
            .answers
            .get(index)
            .is_some_and(|state| is_question_answered(question, state));
        let mut style = if index == form.current {
            Style::new().bg(Color::DarkGray).fg(Color::White)
        } else {
            muted()
        };
        if answered {
            style = style.add_modifier(Modifier::CROSSED_OUT);
        }
        let mark = if answered { "✓ " } else { "" };
        spans.push(Span::styled(
            format!(" {mark}{} ", question.short_label),
            style,
        ));
        spans.push(Span::raw(" "));
    }
    spans.push(Span::styled(
        format!("{} of {}", form.current + 1, questions.len()),
        muted(),
    ));
    Line::from(spans)
}

fn question_body(
    questions: &[Question],
    form: &QuestionForm,
    focused: bool,
    width: usize,
) -> Vec<Line<'static>> {
    let mut body = Vec::new();
    let Some(question) = questions.get(form.current) else {
        return body;
    };
    let state = &form.answers[form.current];
    if questions.len() > 1 {
        body.push(question_tabs(questions, form));
    }
    body.extend(wrapped(&question.prompt, bold(), width, 0));
    let row = |index: usize, checked: bool, label: &str, description: Option<&str>| {
        let cursor = focused && !form.editing && form.cursor == index;
        let mark = match (question.multi_select, checked) {
            (true, true) => "[x]",
            (true, false) => "[ ]",
            (false, true) => "(•)",
            (false, false) => "( )",
        };
        let mut spans = vec![
            Span::styled(
                if cursor { "❯ " } else { "  " }.to_owned(),
                Style::new().fg(Color::Cyan),
            ),
            Span::styled(
                if index < 9 {
                    format!("{} ", index + 1)
                } else {
                    "  ".to_owned()
                },
                key_style(),
            ),
            Span::styled(
                format!("{mark} "),
                if checked {
                    Style::new().fg(Color::Green)
                } else {
                    muted()
                },
            ),
            Span::styled(label.to_owned(), if cursor { bold() } else { Style::new() }),
        ];
        if let Some(description) = description.filter(|text| !text.is_empty()) {
            spans.push(Span::styled(format!(" — {description}"), muted()));
        }
        Line::from(truncate_spans(spans, width))
    };
    for (index, option) in question.options.iter().enumerate() {
        let checked = state.selected.contains(&option.value);
        body.push(row(
            index,
            checked,
            &option.label,
            option.description.as_deref(),
        ));
        if checked && let Some(preview) = option.preview.as_deref() {
            for line in preview.lines().take(PREVIEW_LINES) {
                body.extend(wrapped(line, muted(), width, 8));
            }
        }
    }
    if question.has_other_row() {
        body.push(row(
            question.options.len(),
            state.other_selected,
            OTHER_OPTION_LABEL,
            None,
        ));
    }
    if state.other_selected {
        let input = &form.inputs[form.current];
        let editing = focused && form.editing;
        let mut spans = vec![Span::styled(
            "    › ".to_owned(),
            Style::new().fg(Color::Cyan),
        )];
        if input.text().is_empty() && !editing {
            spans.push(Span::styled("Type your own answer…", muted()));
        } else {
            spans.extend(input_spans(input, false, editing, Style::new()));
        }
        body.push(Line::from(truncate_spans(spans, width)));
    }
    body
}

fn secret_body(
    payload: &bb_wire::interaction::SecretRequestPayload,
    form: &SecretForm,
    focused: bool,
    width: usize,
) -> Vec<Line<'static>> {
    let mut body = Vec::new();
    if let Some(purpose) = payload.purpose.as_deref().filter(|text| !text.is_empty()) {
        body.extend(wrapped(purpose, Style::new(), width, 0));
    }
    body.extend(wrapped(
        &format!(
            "Secrets will be written directly to {}",
            payload.destination.path
        ),
        muted(),
        width,
        0,
    ));
    let name_width = payload
        .fields
        .iter()
        .map(|field| field.name.len())
        .max()
        .unwrap_or(0);
    for (index, field) in payload.fields.iter().enumerate() {
        let current = index == form.field;
        let input = &form.inputs[index];
        let mut spans = vec![
            Span::styled(
                if current && focused { "❯ " } else { "  " }.to_owned(),
                Style::new().fg(Color::Cyan),
            ),
            Span::styled(
                format!("{:<name_width$}  ", field.name),
                if current { bold() } else { Style::new() },
            ),
        ];
        if input.text().is_empty() && !(current && focused) {
            spans.push(Span::styled("(empty)", muted()));
        } else {
            spans.extend(input_spans(
                input,
                !form.revealed,
                current && focused,
                Style::new().fg(Color::Yellow),
            ));
        }
        body.push(Line::from(truncate_spans(spans, width)));
        if let Some(description) = field.description.as_deref().filter(|text| !text.is_empty()) {
            body.extend(wrapped(description, muted(), width, name_width + 4));
        }
    }
    body
}

fn cancel_label(cancel: CancelAction) -> &'static str {
    match cancel {
        CancelAction::StopTurn => "stop turn",
        CancelAction::CancelRequest => "cancel",
    }
}

fn plugin_body(
    state: &AppState,
    form: &InteractionForm,
    width: usize,
) -> (Vec<Line<'static>>, Vec<Line<'static>>) {
    let RequestKind::Plugin {
        plugin_id,
        name,
        data,
        cancel,
        invalid,
        ..
    } = &form.kind
    else {
        return (Vec::new(), Vec::new());
    };
    let mut body = Vec::new();
    let source = if plugin_id.is_empty() {
        name.clone()
    } else {
        plugin_id.clone()
    };
    let origin = match cancel {
        CancelAction::CancelRequest => format!("Requested by {source}"),
        CancelAction::StopTurn => format!("Asked by the agent through {source}"),
    };
    body.extend(wrapped(&origin, muted(), width, 0));
    body.extend(wrapped(
        if *invalid {
            "This request could not be displayed here."
        } else {
            "This request needs the web app to answer."
        },
        Style::new().fg(Color::Yellow),
        width,
        0,
    ));
    let url = thread_web_url(&state.server.url, &form.thread_id);
    let mut link = vec![Span::styled("Open in web: ", bold())];
    link.push(Span::styled(
        url,
        Style::new()
            .fg(Color::Blue)
            .add_modifier(Modifier::UNDERLINED),
    ));
    body.push(Line::from(truncate_spans(link, width)));
    let json = serde_json::to_string_pretty(data).unwrap_or_default();
    for line in json.lines().take(JSON_LINES) {
        body.extend(wrap_plain(
            line,
            muted(),
            width,
            &[Span::raw("  ")],
            WrapMode::Chars,
        ));
    }
    let footer = vec![button_line(
        &[
            ("o".to_owned(), "Open in browser".to_owned()),
            ("y".to_owned(), "Copy link".to_owned()),
            ("C-x".to_owned(), capitalize(cancel_label(*cancel))),
        ],
        "j/k scroll · Esc leave",
    )];
    (body, footer)
}

fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

pub fn panel_view(state: &AppState, thread_id: &str, width: usize) -> PanelView {
    let focused = state.focus == Focus::Interaction;
    let active = state.active_interactions(thread_id);
    let total = active.len();
    let Some((interaction, form)) = active.first().and_then(|interaction| {
        state
            .interaction_forms
            .get(&interaction.id)
            .map(|form| (*interaction, form))
    }) else {
        return PanelView {
            title: vec![Span::styled(
                " ⚑ Waiting for you ".to_owned(),
                Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            )],
            body: vec![Line::from(Span::styled("Loading the request…", muted()))],
            footer: Vec::new(),
            scrollable: false,
        };
    };
    let label = form.kind.label();
    let mut title = vec![Span::styled(
        format!(" ⚑ {label} "),
        Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD),
    )];
    if total > 1 {
        title.push(Span::styled(format!("· 1 of {total} "), muted()));
    }
    if !focused {
        let key = state
            .keymap
            .chords_for(Command::FocusComposer)
            .first()
            .map(ToString::to_string)
            .unwrap_or_else(|| "i".to_owned());
        title.push(Span::styled(format!("· {key} to answer "), muted()));
    }
    let (heading, mut body, mut footer, scrollable) = match (&form.kind, &form.body) {
        (RequestKind::Approval(payload), FormBody::Approval(approval)) => {
            let (heading, body) = approval_body(interaction, payload, approval, width);
            let decisions = ordered_decisions(&payload.available_decisions);
            let footer = approval_footer(&decisions, false);
            (Some(heading), body, footer, true)
        }
        (
            RequestKind::PlanReview {
                approval,
                plan,
                plan_file_path,
            },
            FormBody::Approval(_),
        ) => {
            let mut body = render_markdown(plan, width.max(1), Style::new()).lines;
            if let Some(path) = plan_file_path.as_deref().filter(|path| !path.is_empty()) {
                body.push(Line::from(Span::styled(path.to_owned(), muted())));
            }
            let decisions = ordered_decisions(&approval.available_decisions);
            let footer = approval_footer(&decisions, true);
            let heading = approval
                .reason
                .clone()
                .unwrap_or_else(|| "Ready to code?".to_owned());
            (Some(heading), body, footer, true)
        }
        (
            RequestKind::Question {
                questions, cancel, ..
            },
            FormBody::Question(question_form),
        ) => {
            let body = question_body(questions, question_form, focused, width);
            let last = question_form.current + 1 >= questions.len();
            let footer = vec![Line::from(Span::styled(
                format!(
                    "1-9 choose · Space toggle · Tab next question · Enter {} · C-x {} · Esc leave",
                    if last { "submit" } else { "next" },
                    cancel_label(*cancel)
                ),
                muted(),
            ))];
            (None, body, footer, false)
        }
        (RequestKind::Secret { payload, cancel }, FormBody::Secret(secret)) => {
            let body = secret_body(payload, secret, focused, width);
            let footer = vec![Line::from(Span::styled(
                format!(
                    "Enter next / submit · Tab field · C-t {} · C-x {} · Esc leave",
                    if secret.revealed { "hide" } else { "show" },
                    cancel_label(*cancel)
                ),
                muted(),
            ))];
            (None, body, footer, false)
        }
        _ => {
            let (body, footer) = plugin_body(state, form, width);
            (None, body, footer, true)
        }
    };
    if let Some(heading) = heading.filter(|heading| !heading.is_empty()) {
        let mut lines = wrapped(&heading, bold(), width, 0);
        lines.append(&mut body);
        body = lines;
    }
    if form.submitting || form.submitted {
        footer = vec![Line::from(Span::styled(
            "Submitting…",
            Style::new().fg(Color::Yellow),
        ))];
    } else if interaction.status == bb_wire::interaction::PendingInteractionStatus::Resolving {
        footer = vec![Line::from(Span::styled(
            "Resolving…",
            Style::new().fg(Color::Yellow),
        ))];
    }
    if let Some(error) = &form.error {
        footer.insert(
            0,
            Line::from(truncate_spans(
                vec![Span::styled(
                    format!("✗ {error}"),
                    Style::new().fg(Color::Red),
                )],
                width,
            )),
        );
    }
    PanelView {
        title,
        body,
        footer,
        scrollable,
    }
}

pub fn panel_height(state: &AppState, thread_id: &str, width: usize) -> u16 {
    let view = panel_view(state, thread_id, width);
    (1 + view.body.len() + view.footer.len()) as u16
}

pub fn render_panel(state: &AppState, frame: &mut Frame, area: Rect, thread_id: &str) {
    if area.height < 3 || area.width < 4 {
        return;
    }
    let width = usize::from(area.width);
    let view = panel_view(state, thread_id, width);
    let footer_height = (view.footer.len() as u16).min(area.height.saturating_sub(2));
    let [border_area, body_area, footer_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(footer_height),
    ])
    .areas(area);
    let focused = state.focus == Focus::Interaction;
    let border_style = if focused {
        Style::new().fg(Color::Yellow)
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
            .title(Line::from(view.title)),
        border_area,
    );
    let visible = usize::from(body_area.height);
    let max_scroll = view.body.len().saturating_sub(visible);
    let scroll = match state.active_form() {
        Some(form) if view.scrollable => {
            form.max_scroll.set(max_scroll);
            form.scroll.min(max_scroll)
        }
        _ => 0,
    };
    let mut lines: Vec<Line<'static>> = view.body.into_iter().skip(scroll).take(visible).collect();
    if max_scroll > scroll
        && let Some(last) = lines.last_mut()
    {
        *last = Line::from(Span::styled(
            format!("… {} more lines (j/k to scroll)", max_scroll - scroll),
            muted(),
        ));
    }
    frame.render_widget(Paragraph::new(lines), body_area);
    frame.render_widget(Paragraph::new(view.footer), footer_area);
}
