use bb_wire::timeline::{
    CommandWorkRow, ConversationRole, ConversationRow, PlanStepStatus, SystemRow, SystemRowKind,
    TimelineApprovalStatus, TimelineRow, TimelineRowStatus, TimelineWorkRow, TurnRequestKind,
    TurnRequestStatus,
};

use crate::activity_intents::{
    ExplorationRow, activity_intent_dedupe_key, format_timeline_activity_intent_detail,
    has_timeline_exploration_intent, intent_type, timeline_row_activity_intents,
};
use crate::domain::display_width;
use crate::path_display::PathDisplayMode;
use crate::title::{TitleOptions, build_timeline_row_title, build_work_row_title};
use crate::view::{
    SummaryChild, SummaryKind, ViewRow, ViewTurnRow, ViewWorkRow, WorkSummaryRow,
    build_timeline_view_rows, build_timeline_work_summary_label, find_active_latest_bundle_id,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TimelineTextOptions {
    pub verbose: bool,
    pub color: bool,
}

#[derive(Debug, Clone)]
struct Context {
    verbose: bool,
    color: bool,
    depth: usize,
    active_latest_bundle_id: Option<String>,
}

fn wrap(text: &str, color: bool, open: &str, close: &str) -> String {
    if color {
        format!("\x1b[{open}m{text}\x1b[{close}m")
    } else {
        text.to_owned()
    }
}

fn dim(text: &str, color: bool) -> String {
    wrap(text, color, "2", "22")
}

fn green(text: &str, color: bool) -> String {
    wrap(text, color, "32", "39")
}

fn yellow(text: &str, color: bool) -> String {
    wrap(text, color, "33", "39")
}

fn red(text: &str, color: bool) -> String {
    wrap(text, color, "31", "39")
}

fn cyan(text: &str, color: bool) -> String {
    wrap(text, color, "36", "39")
}

fn separator(label: &str, color: bool) -> String {
    let pad = 60usize.saturating_sub(display_width(label) + 4);
    let text = if pad > 0 {
        format!("── {label} {}", "─".repeat(pad))
    } else {
        format!("── {label}")
    };
    dim(&text, color)
}

fn row_header(label: &str, context: &Context) -> String {
    if context.depth == 0 {
        separator(label, context.color)
    } else {
        dim(&format!("── {label}"), context.color)
    }
}

fn nested_context(context: &Context, nested_rows: Option<&[ViewRow]>) -> Context {
    Context {
        depth: context.depth + 1,
        active_latest_bundle_id: nested_rows.and_then(find_active_latest_bundle_id),
        ..context.clone()
    }
}

pub fn indent_block(text: &str, prefix: &str) -> String {
    text.split('\n')
        .map(|line| {
            if line.is_empty() {
                String::new()
            } else {
                format!("{prefix}{line}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn status_label(status: TimelineRowStatus, color: bool) -> String {
    match status {
        TimelineRowStatus::Pending => yellow("running", color),
        TimelineRowStatus::Error => red("error", color),
        TimelineRowStatus::Interrupted => yellow("interrupted", color),
        TimelineRowStatus::Completed | TimelineRowStatus::Unknown => green("completed", color),
    }
}

fn work_approval_status(row: &TimelineWorkRow) -> Option<TimelineApprovalStatus> {
    match row {
        TimelineWorkRow::Command(row) => row.approval_status,
        TimelineWorkRow::Tool(row) => row.approval_status,
        TimelineWorkRow::FileChange(row) => row.approval_status,
        _ => None,
    }
}

fn work_status_label(row: &TimelineWorkRow, color: bool) -> String {
    match work_approval_status(row) {
        Some(TimelineApprovalStatus::Denied) => red("denied", color),
        Some(TimelineApprovalStatus::WaitingForApproval) => yellow("waiting", color),
        _ => status_label(row.status(), color),
    }
}

fn title_options(row: &ViewRow, context: &Context) -> TitleOptions {
    TitleOptions {
        is_active_latest_bundle: row.is_bundle_summary()
            && context.active_latest_bundle_id.as_deref() == Some(row.id()),
        ..TitleOptions::default()
    }
}

fn format_work_title(row: &ViewWorkRow) -> String {
    build_work_row_title(&row.row, TitleOptions::default()).plain
}

fn format_work_output(output: &str, color: bool) -> String {
    dim(&indent_block(output.trim(), "  "), color)
}

fn format_command_exit_code_line(row: &CommandWorkRow, color: bool) -> Option<String> {
    let exit_code = row.exit_code?;
    if row.status == TimelineRowStatus::Pending {
        return None;
    }
    if exit_code == 0 && !row.output.trim().is_empty() {
        return None;
    }
    if exit_code == 0 {
        return Some(dim("  exit code 0", color));
    }
    Some(dim(&format!("  exit {exit_code}"), color))
}

fn format_work_body(row: &ViewWorkRow, context: &Context) -> Vec<String> {
    let mut lines = Vec::new();
    match &row.row {
        TimelineWorkRow::Command(command) => {
            lines.push(format!("  $ {}", cyan(&command.command, context.color)));
            if context.verbose
                && !command.output.trim().is_empty()
                && !has_timeline_exploration_intent(ExplorationRow::Command(command))
            {
                lines.push(format_work_output(&command.output, context.color));
            }
            if let Some(line) = format_command_exit_code_line(command, context.color) {
                lines.push(line);
            }
        }
        TimelineWorkRow::Tool(tool) => {
            if tool.approval_status.is_some() {
                lines.push(format!("  {}", work_status_label(&row.row, context.color)));
            }
            if context.verbose && !tool.output.trim().is_empty() {
                lines.push(format_work_output(&tool.output, context.color));
            }
        }
        TimelineWorkRow::FileChange(change) => {
            if change.approval_status.is_some() {
                lines.push(format!("  {}", work_status_label(&row.row, context.color)));
            }
            if context.verbose
                && let Some(diff) = change
                    .change
                    .diff
                    .as_deref()
                    .filter(|diff| !diff.is_empty())
            {
                lines.push(dim(&indent_block(diff.trim_end(), "  "), context.color));
            }
        }
        TimelineWorkRow::PlanSteps(plan) => {
            if context.verbose {
                for step in &plan.steps {
                    let marker = match step.status {
                        Some(PlanStepStatus::Completed) => "[x]",
                        Some(PlanStepStatus::Active) => "[>]",
                        Some(PlanStepStatus::Failed) => "[!]",
                        _ => "[ ]",
                    };
                    lines.push(dim(&format!("  {marker} {}", step.step), context.color));
                }
            }
        }
        TimelineWorkRow::Extension(extension) => {
            if let Some(detail) = extension
                .presentation
                .detail
                .as_deref()
                .filter(|detail| !detail.trim().is_empty())
            {
                lines.push(dim(&indent_block(detail, "  "), context.color));
            }
        }
        TimelineWorkRow::Delegation(_) => {
            if let Some(children) = row.child_rows.as_deref().filter(|rows| !rows.is_empty()) {
                lines.push(indent_block(
                    &format_rows(children, &nested_context(context, Some(children))),
                    "  ",
                ));
            }
        }
        TimelineWorkRow::WebSearch(_)
        | TimelineWorkRow::WebFetch(_)
        | TimelineWorkRow::ImageGeneration(_)
        | TimelineWorkRow::ImageView(_)
        | TimelineWorkRow::FileRead(_)
        | TimelineWorkRow::Search(_)
        | TimelineWorkRow::Approval(_)
        | TimelineWorkRow::Question(_)
        | TimelineWorkRow::Form(_)
        | TimelineWorkRow::Workflow(_)
        | TimelineWorkRow::Unknown(_) => {}
    }
    lines
}

fn format_work_row(row: &ViewWorkRow, context: &Context) -> String {
    let mut lines = vec![row_header(&format_work_title(row), context)];
    lines.extend(format_work_body(row, context));
    lines.join("\n")
}

fn format_exploration_work_details(row: ExplorationRow<'_>) -> Vec<String> {
    let mut last_key: Option<String> = None;
    let mut details = Vec::new();
    let pending = row.status() == TimelineRowStatus::Pending;
    for intent in timeline_row_activity_intents(row).iter() {
        if intent_type(intent).is_none() {
            continue;
        }
        let key = activity_intent_dedupe_key(intent);
        if key.is_some() && key == last_key {
            continue;
        }
        details.push(format_timeline_activity_intent_detail(
            intent,
            PathDisplayMode::Full,
            pending,
        ));
        last_key = key;
    }
    details
}

fn format_work_summary_details(row: &WorkSummaryRow, context: &Context) -> Vec<String> {
    let mut lines = Vec::new();
    let child_context = nested_context(context, None);
    for child in &row.children {
        let work = match child {
            SummaryChild::Reasoning(system) => {
                lines.push(format_system_row(system, &child_context));
                continue;
            }
            SummaryChild::Work(work) => work,
        };
        if let Some(exploration) = work.exploration()
            && has_timeline_exploration_intent(exploration)
        {
            lines.extend(
                format_exploration_work_details(exploration)
                    .iter()
                    .map(|detail| row_header(detail, &child_context)),
            );
            continue;
        }
        if matches!(
            work.row,
            TimelineWorkRow::WebSearch(_)
                | TimelineWorkRow::WebFetch(_)
                | TimelineWorkRow::ImageView(_)
                | TimelineWorkRow::PlanSteps(_)
                | TimelineWorkRow::Extension(_)
        ) {
            lines.push(row_header(&format_work_title(work), &child_context));
            continue;
        }
        lines.push(format_work_row(work, &child_context));
    }
    lines
}

fn format_work_summary(row: &WorkSummaryRow, view_row: &ViewRow, context: &Context) -> String {
    let active = row.kind == SummaryKind::Bundle
        && context.active_latest_bundle_id.as_deref() == Some(view_row.id());
    let mut lines = vec![row_header(
        &build_timeline_work_summary_label(row, active),
        context,
    )];
    if context.verbose || row.kind == SummaryKind::Bundle {
        let details = format_work_summary_details(row, context);
        if !details.is_empty() {
            lines.push(indent_block(&details.join("\n"), "  "));
        }
    }
    lines.join("\n")
}

fn conversation_request_label(row: &ConversationRow) -> Option<&'static str> {
    let request = row.turn_request.as_ref()?;
    if row.role != ConversationRole::User || request.kind != TurnRequestKind::Steer {
        return None;
    }
    Some(match request.status {
        TurnRequestStatus::Pending => "steer pending",
        TurnRequestStatus::Rejected => "steer failed",
        _ => "steer",
    })
}

fn format_system_row(row: &SystemRow, context: &Context) -> String {
    if row.system_kind == SystemRowKind::Reconnect {
        return row_header(&row.title, context);
    }
    let detail = row
        .detail
        .as_deref()
        .filter(|detail| !detail.is_empty())
        .map(|detail| dim(&indent_block(detail, "  "), context.color))
        .unwrap_or_default();
    let lines = if row.system_kind == SystemRowKind::Error {
        vec![
            row_header("Error", context),
            dim(&indent_block(&row.title, "  "), context.color),
            detail,
        ]
    } else {
        vec![row_header(&row.title, context), detail]
    };
    lines
        .into_iter()
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn format_turn(row: &ViewTurnRow, view_row: &ViewRow, context: &Context) -> String {
    let label = build_timeline_row_title(view_row, title_options(view_row, context)).plain;
    match row.children.as_deref().filter(|rows| !rows.is_empty()) {
        None => row_header(&label, context),
        Some(children) => [
            row_header(&label, context),
            indent_block(&format_rows(children, &nested_context(context, None)), "  "),
        ]
        .join("\n"),
    }
}

fn format_row(row: &ViewRow, context: &Context) -> String {
    match row {
        ViewRow::Conversation(conversation) => {
            let header = row_header(
                if conversation.role == ConversationRole::User {
                    "User"
                } else {
                    "Assistant"
                },
                context,
            );
            let mut lines = vec![header, conversation.text.clone()];
            if let Some(label) = conversation_request_label(conversation) {
                lines.push(label.to_owned());
            }
            lines.join("\n")
        }
        ViewRow::Work(work) => format_work_row(work, context),
        ViewRow::System(system) => format_system_row(system, context),
        ViewRow::Summary(summary) => format_work_summary(summary, row, context),
        ViewRow::Turn(turn) => format_turn(turn, row, context),
        ViewRow::Unknown(_) => row_header(
            &build_timeline_row_title(row, title_options(row, context)).plain,
            context,
        ),
    }
}

fn format_rows(rows: &[ViewRow], context: &Context) -> String {
    rows.iter()
        .map(|row| format_row(row, context))
        .collect::<Vec<_>>()
        .join("\n\n")
}

pub fn format_view_rows_text(rows: &[ViewRow], options: TimelineTextOptions) -> String {
    format_rows(
        rows,
        &Context {
            verbose: options.verbose,
            color: options.color,
            depth: 0,
            active_latest_bundle_id: find_active_latest_bundle_id(rows),
        },
    )
}

pub fn format_thread_timeline_text(rows: &[TimelineRow], options: TimelineTextOptions) -> String {
    format_view_rows_text(&build_timeline_view_rows(rows), options)
}
