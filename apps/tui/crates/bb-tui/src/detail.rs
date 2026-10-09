use bb_view::glyph::WorkRowGlyph;
use bb_view::patch::{PatchLineKind, plain_diff_fallback, renderable_diff};
use bb_view::styled::{SpanRole, StyledSpan};
use bb_wire::JsonValue;
use bb_wire::timeline::{
    BackgroundTaskStatus, CommandWorkRow, DelegationWorkRow, FileChangeWorkRow,
    FullOutputAvailability, OutputPreview, PlanStepStatus, QuestionWorkRow, SystemRow,
    SystemRowKind, TimelineApprovalStatus, TimelineWorkRow, ToolWorkRow, WorkflowAgentState,
    WorkflowWorkRow,
};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use crate::ansi::ansi_lines;
use crate::markdown::{bare_urls, render_markdown};
use crate::wrap::{WrapMode, wrap_plain, wrap_spans};

pub const MAX_OUTPUT_LINES: usize = 400;
pub const MAX_JSON_LINES: usize = 80;

pub const MUTED: Style = Style::new().fg(Color::DarkGray);
pub const OUTPUT: Style = Style::new().fg(Color::Gray);
pub const LABEL: Style = Style::new().fg(Color::DarkGray);
pub const ERROR: Style = Style::new().fg(Color::Red);
pub const WARNING: Style = Style::new().fg(Color::Yellow);
pub const ADDED: Style = Style::new().fg(Color::Green);
pub const REMOVED: Style = Style::new().fg(Color::Red);
pub const HUNK: Style = Style::new().fg(Color::Cyan);

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Body {
    pub lines: Vec<Line<'static>>,
    pub links: Vec<String>,
    pub paths: Vec<String>,
}

impl Body {
    fn text(&mut self, text: &str, style: Style, width: usize) {
        self.lines
            .extend(wrap_plain(text, style, width, &[], WrapMode::Words));
    }

    fn field(&mut self, label: &str, value: &str, width: usize) {
        if value.trim().is_empty() {
            return;
        }
        let label = format!("{label}: ");
        let pad = " ".repeat(crate::wrap::text_width(&label));
        self.lines.extend(wrap_spans(
            &[Span::raw(value.to_owned())],
            width,
            &[Span::styled(label, LABEL)],
            &[Span::raw(pad)],
            WrapMode::Chars,
        ));
    }

    fn markdown(&mut self, text: &str, style: Style, width: usize) {
        let rendered = render_markdown(text, width, style);
        self.lines.extend(rendered.lines);
        self.add_links(rendered.links);
    }

    fn add_links(&mut self, links: impl IntoIterator<Item = String>) {
        for link in links {
            if !self.links.contains(&link) {
                self.links.push(link);
            }
        }
    }

    fn add_path(&mut self, path: &str) {
        let path = path.trim();
        if !path.is_empty() && !self.paths.iter().any(|known| known == path) {
            self.paths.push(path.to_owned());
        }
    }

    fn output(&mut self, output: &str, width: usize) {
        let output = output.trim_end_matches(['\n', '\r']);
        if output.trim().is_empty() {
            return;
        }
        self.add_links(bare_urls(output));
        let lines = ansi_lines(output, OUTPUT);
        let hidden = lines.len().saturating_sub(MAX_OUTPUT_LINES);
        if hidden > 0 {
            self.lines.push(Line::from(Span::styled(
                format!("… {hidden} earlier lines hidden"),
                MUTED,
            )));
        }
        for spans in lines.into_iter().skip(hidden) {
            self.lines
                .extend(wrap_spans(&spans, width, &[], &[], WrapMode::Chars));
        }
    }

    fn preview_notice(&mut self, preview: Option<&OutputPreview>, shown: usize) {
        let Some(preview) = preview else {
            return;
        };
        if preview.total_chars as usize <= shown {
            return;
        }
        let reason = match preview.full_output_availability {
            FullOutputAvailability::RetentionExpired => " (full output expired)",
            FullOutputAvailability::DetailLimit => " (detail limit)",
            _ => "",
        };
        self.lines.push(Line::from(Span::styled(
            format!(
                "… output truncated, {} chars total{reason}",
                preview.total_chars
            ),
            MUTED,
        )));
    }

    fn json(&mut self, value: &JsonValue, width: usize) {
        let text = serde_json::to_string_pretty(value).unwrap_or_default();
        let lines: Vec<&str> = text.lines().collect();
        for line in lines.iter().take(MAX_JSON_LINES) {
            self.lines
                .extend(wrap_plain(line, OUTPUT, width, &[], WrapMode::Chars));
        }
        if lines.len() > MAX_JSON_LINES {
            self.lines.push(Line::from(Span::styled(
                format!("… {} more lines", lines.len() - MAX_JSON_LINES),
                MUTED,
            )));
        }
    }

    fn notice(&mut self, text: &str, style: Style) {
        self.lines
            .push(Line::from(Span::styled(text.to_owned(), style)));
    }
}

pub fn glyph_char(glyph: WorkRowGlyph) -> &'static str {
    match glyph {
        WorkRowGlyph::CircleQuestion => "?",
        WorkRowGlyph::EditFile => "✎",
        WorkRowGlyph::File => "□",
        WorkRowGlyph::FileText => "≡",
        WorkRowGlyph::Folder => "⊞",
        WorkRowGlyph::Globe => "⊕",
        WorkRowGlyph::ListTodo => "☰",
        WorkRowGlyph::Lock => "⊡",
        WorkRowGlyph::Palette => "◐",
        WorkRowGlyph::Puzzle => "◈",
        WorkRowGlyph::Search => "⌕",
        WorkRowGlyph::Terminal => "$",
        WorkRowGlyph::UserRoundPlus => "↳",
        WorkRowGlyph::Zap => "ϟ",
    }
}

pub fn role_style(span: &StyledSpan) -> Style {
    let style = match span.role {
        SpanRole::Text => Style::new(),
        SpanRole::Emphasis => Style::new().add_modifier(Modifier::BOLD),
        SpanRole::Muted | SpanRole::Decoration => MUTED,
        SpanRole::Subtle => Style::new().fg(Color::Gray),
        SpanRole::File => Style::new().fg(Color::Cyan),
        SpanRole::Link => Style::new()
            .fg(Color::LightBlue)
            .add_modifier(Modifier::UNDERLINED),
        SpanRole::Error => ERROR,
        SpanRole::Warning => WARNING,
        SpanRole::Badge => Style::new().fg(Color::Magenta),
        SpanRole::BadgeDestructive => ERROR.add_modifier(Modifier::BOLD),
        SpanRole::DiffAdded => ADDED,
        SpanRole::DiffRemoved => REMOVED,
    };
    if span.shimmer {
        style.fg(Color::LightYellow)
    } else {
        style
    }
}

fn command_body(row: &CommandWorkRow, width: usize, body: &mut Body) {
    body.lines.extend(wrap_spans(
        &[Span::styled(
            row.command.clone(),
            Style::new().fg(Color::Cyan),
        )],
        width,
        &[Span::styled("$ ", MUTED)],
        &[Span::raw("  ")],
        WrapMode::Chars,
    ));
    if let Some(cwd) = row.cwd.as_deref() {
        body.field("cwd", cwd, width);
    }
    body.output(&row.output, width);
    body.preview_notice(row.output_preview.as_ref(), row.output.chars().count());
    if row.status != bb_wire::timeline::TimelineRowStatus::Pending
        && let Some(code) = row.exit_code
    {
        if code != 0 {
            body.notice(&format!("exit {code}"), ERROR);
        } else if row.output.trim().is_empty() {
            body.notice("exit code 0", MUTED);
        }
    }
}

fn json_inline(value: &JsonValue) -> Option<String> {
    match value {
        JsonValue::String(text) => Some(text.clone()),
        JsonValue::Null => Some("null".into()),
        JsonValue::Bool(_) | JsonValue::Number(_) => Some(value.to_string()),
        JsonValue::Array(_) | JsonValue::Object(_) => None,
    }
}

fn tool_body(row: &ToolWorkRow, width: usize, body: &mut Body) {
    body.lines.push(Line::from(vec![
        Span::styled("tool: ", LABEL),
        Span::styled(
            row.tool_name.clone(),
            Style::new().add_modifier(Modifier::BOLD),
        ),
    ]));
    if let Some(args) = row.tool_args.as_ref().filter(|args| !args.is_empty()) {
        for (key, value) in args {
            match json_inline(value) {
                Some(text) if !text.contains('\n') => body.field(key, &text, width),
                Some(text) => {
                    body.lines
                        .push(Line::from(Span::styled(format!("{key}:"), LABEL)));
                    for line in text.lines().take(MAX_JSON_LINES) {
                        body.lines.extend(wrap_plain(
                            line,
                            OUTPUT,
                            width,
                            &[Span::raw("  ")],
                            WrapMode::Chars,
                        ));
                    }
                }
                None => {
                    body.lines
                        .push(Line::from(Span::styled(format!("{key}:"), LABEL)));
                    body.json(value, width);
                }
            }
            if let JsonValue::String(text) = value {
                body.add_links(bare_urls(text));
            }
        }
    }
    if let Some(status) = row.approval_status {
        approval_notice(status, body);
    }
    if !row.output.trim().is_empty() {
        body.notice("output:", LABEL);
        body.output(&row.output, width);
        body.preview_notice(row.output_preview.as_ref(), row.output.chars().count());
    }
}

fn approval_notice(status: TimelineApprovalStatus, body: &mut Body) {
    match status {
        TimelineApprovalStatus::WaitingForApproval => body.notice("waiting for approval", WARNING),
        TimelineApprovalStatus::Denied => body.notice("denied", ERROR),
        TimelineApprovalStatus::Unknown => {}
    }
}

fn file_change_body(row: &FileChangeWorkRow, width: usize, body: &mut Body) {
    body.add_path(row.change.move_path.as_deref().unwrap_or(&row.change.path));
    if let Some(status) = row.approval_status {
        approval_notice(status, body);
    }
    let diff = renderable_diff(&row.change);
    match &diff {
        Some(diff) if diff.files.iter().any(|file| !file.hunks.is_empty()) => {
            for file in &diff.files {
                diff_file(file, diff.disable_line_numbers, width, body);
            }
        }
        _ => {
            if let Some(text) = plain_diff_fallback(&row.change, false) {
                for line in text.lines() {
                    let style = if line.starts_with('+') {
                        ADDED
                    } else if line.starts_with('-') {
                        REMOVED
                    } else {
                        OUTPUT
                    };
                    body.lines
                        .extend(wrap_plain(line, style, width, &[], WrapMode::Chars));
                }
            } else {
                body.notice("no diff available", MUTED);
            }
        }
    }
    if let Some(stdout) = row.stdout.as_deref() {
        body.output(stdout, width);
    }
    if let Some(stderr) = row.stderr.as_deref().filter(|text| !text.trim().is_empty()) {
        body.notice("stderr:", LABEL);
        body.output(stderr, width);
    }
}

fn diff_file(file: &bb_view::patch::PatchFile, no_numbers: bool, width: usize, body: &mut Body) {
    let (added, removed) = file.stats();
    let mut header = vec![Span::styled(
        file.display_path().unwrap_or("file").to_owned(),
        Style::new().add_modifier(Modifier::BOLD),
    )];
    if let (Some(old), Some(new)) = (file.old_path.as_deref(), file.new_path.as_deref())
        && old != new
    {
        header.insert(0, Span::styled(format!("{old} → "), MUTED));
    }
    if added > 0 {
        header.push(Span::styled(format!(" +{added}"), ADDED));
    }
    if removed > 0 {
        header.push(Span::styled(format!(" -{removed}"), REMOVED));
    }
    if file.is_binary {
        header.push(Span::styled(" (binary)", MUTED));
    }
    body.lines
        .extend(wrap_spans(&header, width, &[], &[], WrapMode::Chars));
    let max_line = file
        .hunks
        .iter()
        .flat_map(|hunk| &hunk.lines)
        .flat_map(|line| [line.old_line, line.new_line])
        .flatten()
        .max()
        .unwrap_or(0);
    let number_width = if no_numbers {
        0
    } else {
        max_line.to_string().len()
    };
    let mut emitted = 0usize;
    for hunk in &file.hunks {
        let mut text = hunk.header.clone();
        if text.is_empty() {
            text = format!(
                "@@ -{},{} +{},{} @@",
                hunk.old_start, hunk.old_count, hunk.new_start, hunk.new_count
            );
        }
        body.lines
            .extend(wrap_plain(&text, HUNK, width, &[], WrapMode::Chars));
        for line in &hunk.lines {
            if emitted >= MAX_OUTPUT_LINES * 2 {
                body.notice("… diff truncated", MUTED);
                return;
            }
            emitted += 1;
            let (sign, style) = match line.kind {
                PatchLineKind::Added => ("+", ADDED),
                PatchLineKind::Removed => ("-", REMOVED),
                PatchLineKind::Context => (" ", OUTPUT),
                PatchLineKind::NoNewline => ("", MUTED),
            };
            let mut prefix = Vec::new();
            if number_width > 0 {
                let number = line
                    .new_line
                    .or(line.old_line)
                    .map(|number| number.to_string())
                    .unwrap_or_default();
                prefix.push(Span::styled(format!("{number:>number_width$} "), MUTED));
            }
            prefix.push(Span::styled(sign.to_owned(), style));
            let rest_pad = " ".repeat(crate::wrap::spans_width(&prefix));
            body.lines.extend(wrap_spans(
                &[Span::styled(
                    line.text.replace('\t', crate::wrap::TAB),
                    style,
                )],
                width,
                &prefix,
                &[Span::raw(rest_pad)],
                WrapMode::Chars,
            ));
        }
    }
}

fn question_body(row: &QuestionWorkRow, width: usize, body: &mut Body) {
    for question in &row.questions {
        body.lines.extend(wrap_plain(
            &question.prompt,
            Style::new().add_modifier(Modifier::BOLD),
            width,
            &[],
            WrapMode::Words,
        ));
        let answer = row
            .answers
            .as_ref()
            .and_then(|answers| answers.get(&question.id));
        for option in question.options.iter().flatten() {
            let selected = answer.is_some_and(|answer| answer.selected.contains(&option.value));
            let marker = if selected { "● " } else { "○ " };
            let mut spans = vec![Span::styled(
                option.label.clone(),
                if selected {
                    Style::new().add_modifier(Modifier::BOLD)
                } else {
                    Style::new()
                },
            )];
            if let Some(description) = option.description.as_deref() {
                spans.push(Span::styled(format!(" — {description}"), MUTED));
            }
            body.lines.extend(wrap_spans(
                &spans,
                width,
                &[Span::styled(format!("  {marker}"), MUTED)],
                &[Span::raw("    ")],
                WrapMode::Words,
            ));
        }
        if let Some(text) = answer.and_then(|answer| answer.free_text.as_deref()) {
            body.lines.extend(wrap_spans(
                &[Span::raw(text.to_owned())],
                width,
                &[Span::styled("  ✎ ", MUTED)],
                &[Span::raw("    ")],
                WrapMode::Words,
            ));
        }
    }
    if let Some(reason) = row.status_reason.as_deref() {
        body.field("status", reason, width);
    }
}

fn delegation_body(row: &DelegationWorkRow, width: usize, body: &mut Body) {
    if let Some(description) = row.description.as_deref() {
        body.text(description, Style::new(), width);
    }
    if let Some(kind) = row.subagent_type.as_deref() {
        body.field("agent", kind, width);
    }
    if let Some(child) = row.child_ref.as_deref() {
        body.field("thread", child, width);
    }
    if row.background {
        body.notice("runs in background", MUTED);
    }
    if !row.output.trim().is_empty() {
        body.notice("result:", LABEL);
        body.markdown(&row.output, OUTPUT, width);
    }
}

fn agent_state_marker(state: WorkflowAgentState) -> (&'static str, Style) {
    match state {
        WorkflowAgentState::Done => ("✓", ADDED),
        WorkflowAgentState::Failed => ("✗", ERROR),
        WorkflowAgentState::Running => ("●", WARNING),
        WorkflowAgentState::Skipped => ("-", MUTED),
        WorkflowAgentState::Queued | WorkflowAgentState::Unknown => ("○", MUTED),
    }
}

fn task_status_label(status: BackgroundTaskStatus) -> &'static str {
    match status {
        BackgroundTaskStatus::Pending => "pending",
        BackgroundTaskStatus::Running => "running",
        BackgroundTaskStatus::Paused => "paused",
        BackgroundTaskStatus::Completed => "completed",
        BackgroundTaskStatus::Failed => "failed",
        BackgroundTaskStatus::Killed => "killed",
        BackgroundTaskStatus::Stopped => "stopped",
        BackgroundTaskStatus::Unknown => "unknown",
    }
}

fn workflow_body(row: &WorkflowWorkRow, width: usize, body: &mut Body) {
    if let Some(name) = row.workflow_name.as_deref() {
        body.field("workflow", name, width);
    }
    if !row.description.trim().is_empty() {
        body.text(&row.description, Style::new(), width);
    }
    body.field("status", task_status_label(row.task_status), width);
    if let Some(model) = row.model.as_deref() {
        body.field("model", model, width);
    }
    if let Some(workflow) = row.workflow.as_ref() {
        for agent in &workflow.agents {
            let (marker, style) = agent_state_marker(agent.state);
            let mut spans = vec![Span::raw(agent.label.clone())];
            if let Some(phase) = agent.phase_title.as_deref() {
                spans.push(Span::styled(format!(" · {phase}"), MUTED));
            }
            if let Some(summary) = agent
                .last_tool_summary
                .as_deref()
                .or(agent.error.as_deref())
            {
                spans.push(Span::styled(format!(" — {summary}"), MUTED));
            }
            body.lines.extend(wrap_spans(
                &spans,
                width,
                &[Span::styled(format!("{marker} "), style)],
                &[Span::raw("  ")],
                WrapMode::Words,
            ));
        }
    }
    if let Some(summary) = row
        .summary
        .as_deref()
        .filter(|text| !text.trim().is_empty())
    {
        body.markdown(summary, OUTPUT, width);
    }
    if let Some(error) = row.error.as_deref().filter(|text| !text.trim().is_empty()) {
        body.text(error, ERROR, width);
    }
    if let Some(usage) = row.usage.as_ref() {
        body.notice(
            &format!(
                "{} tokens · {} tool uses · {}",
                usage.total_tokens as u64,
                usage.tool_uses as u64,
                bb_view::format::duration_ms_to_compact_string(usage.duration_ms as i64)
            ),
            MUTED,
        );
    }
}

pub fn work_body(row: &TimelineWorkRow, width: usize) -> Body {
    let mut body = Body::default();
    match row {
        TimelineWorkRow::Command(row) => command_body(row, width, &mut body),
        TimelineWorkRow::Tool(row) => tool_body(row, width, &mut body),
        TimelineWorkRow::FileChange(row) => file_change_body(row, width, &mut body),
        TimelineWorkRow::WebSearch(row) => {
            for query in &row.queries {
                body.field("query", query, width);
            }
        }
        TimelineWorkRow::WebFetch(row) => {
            body.field("url", &row.url, width);
            body.add_links([row.url.clone()]);
            if let Some(prompt) = row.prompt.as_deref() {
                body.field("prompt", prompt, width);
            }
            if let Some(pattern) = row.pattern.as_deref() {
                body.field("pattern", pattern, width);
            }
        }
        TimelineWorkRow::ImageGeneration(row) => {
            if let Some(prompt) = row.prompt.as_deref() {
                body.field("prompt", prompt, width);
            }
            if let Some(path) = row.path.as_deref() {
                body.field("path", path, width);
                body.add_path(path);
            }
            if let Some(error) = row.error.as_deref() {
                body.text(error, ERROR, width);
            }
        }
        TimelineWorkRow::ImageView(row) => {
            body.field("path", &row.path, width);
            body.add_path(&row.path);
        }
        TimelineWorkRow::FileRead(row) => {
            body.field("path", &row.path, width);
            body.add_path(&row.path);
            if let Some(cmd) = row.cmd.as_deref() {
                body.field("cmd", cmd, width);
            }
        }
        TimelineWorkRow::Search(row) => {
            body.field("query", &row.query, width);
            if let Some(path) = row.path.as_deref() {
                body.field("path", path, width);
            }
            if let Some(cmd) = row.cmd.as_deref() {
                body.field("cmd", cmd, width);
            }
        }
        TimelineWorkRow::PlanSteps(row) => {
            if let Some(explanation) = row.explanation.as_deref() {
                body.markdown(explanation, OUTPUT, width);
            }
            for step in &row.steps {
                let (marker, style) = match step.status {
                    Some(PlanStepStatus::Completed) => ("[x] ", ADDED),
                    Some(PlanStepStatus::Active) => ("[>] ", WARNING),
                    Some(PlanStepStatus::Failed) => ("[!] ", ERROR),
                    _ => ("[ ] ", MUTED),
                };
                body.lines.extend(wrap_spans(
                    &[Span::raw(step.step.clone())],
                    width,
                    &[Span::styled(marker, style)],
                    &[Span::raw("    ")],
                    WrapMode::Words,
                ));
            }
        }
        TimelineWorkRow::Extension(row) => {
            if let Some(detail) = row.presentation.detail.as_deref() {
                body.markdown(detail, OUTPUT, width);
            }
            body.field("kind", &row.extension_kind, width);
            if !row.payload.is_null() {
                body.json(&row.payload, width);
            }
        }
        TimelineWorkRow::Approval(row) => {
            body.field("kind", row.approval_kind.as_str(), width);
            body.field("state", row.lifecycle.as_str(), width);
            if let Some(tool) = row.target.tool_name.as_deref() {
                body.field("tool", tool, width);
            }
            if let Some(scope) = row.grant_scope {
                body.field("scope", scope.as_str(), width);
            }
            if let Some(reason) = row.status_reason.as_deref() {
                body.field("reason", reason, width);
            }
        }
        TimelineWorkRow::Question(row) => question_body(row, width, &mut body),
        TimelineWorkRow::Form(row) => {
            body.field("form", &row.title, width);
            body.field("plugin", &row.plugin_id, width);
            body.field("state", row.lifecycle.as_str(), width);
            if let Some(reason) = row.status_reason.as_deref() {
                body.field("reason", reason, width);
            }
            if !row.payload.is_null() {
                body.json(&row.payload, width);
            }
        }
        TimelineWorkRow::Delegation(row) => delegation_body(row, width, &mut body),
        TimelineWorkRow::Workflow(row) => workflow_body(row, width, &mut body),
        TimelineWorkRow::Unknown(value) => body.json(value, width),
    }
    body
}

pub fn work_has_body(row: &TimelineWorkRow) -> bool {
    !matches!(row, TimelineWorkRow::WebSearch(row) if row.queries.is_empty())
}

pub fn system_has_body(row: &SystemRow) -> bool {
    row.content_deferred
        || row
            .detail
            .as_deref()
            .is_some_and(|detail| !detail.trim().is_empty())
        || (row.system_kind == SystemRowKind::Error && !row.title.trim().is_empty())
}

pub fn system_body(row: &SystemRow, width: usize) -> Body {
    let mut body = Body::default();
    if row.system_kind == SystemRowKind::Error {
        body.text(&row.title, ERROR, width);
    }
    if let Some(detail) = row
        .detail
        .as_deref()
        .filter(|detail| !detail.trim().is_empty())
    {
        let style = if row.system_kind == SystemRowKind::Error {
            OUTPUT
        } else {
            Style::new().fg(Color::Gray).add_modifier(Modifier::ITALIC)
        };
        body.markdown(detail, style, width);
    }
    body
}

#[cfg(test)]
mod tests {
    use super::*;
    use bb_wire::timeline::{DiffStats, FileChange};

    fn text(body: &Body) -> Vec<String> {
        body.lines.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn command_body_shows_output_and_exit_code() {
        let row = CommandWorkRow {
            command: "cargo test".into(),
            output: "running 2 tests\n\u{1b}[31mFAILED\u{1b}[0m see https://ci.dev/run".into(),
            exit_code: Some(101),
            status: bb_wire::timeline::TimelineRowStatus::Error,
            ..CommandWorkRow::default()
        };
        let body = work_body(&TimelineWorkRow::Command(Box::new(row)), 40);
        assert_eq!(
            text(&body),
            vec![
                "$ cargo test",
                "running 2 tests",
                "FAILED see https://ci.dev/run",
                "exit 101"
            ]
        );
        assert_eq!(body.links, vec!["https://ci.dev/run"]);
        assert_eq!(body.lines[2].spans[0].style.fg, Some(Color::Red));
    }

    #[test]
    fn file_change_body_renders_colored_hunks() {
        let row = FileChangeWorkRow {
            change: FileChange {
                path: "src/lib.rs".into(),
                diff: Some("@@ -1,2 +1,2 @@\n fn a() {}\n-let x = 1;\n+let x = 2;\n".into()),
                diff_stats: DiffStats {
                    added: 1,
                    removed: 1,
                },
                ..FileChange::default()
            },
            ..FileChangeWorkRow::default()
        };
        let body = work_body(&TimelineWorkRow::FileChange(Box::new(row)), 40);
        assert_eq!(
            text(&body),
            vec![
                "src/lib.rs +1 -1",
                "@@ -1,2 +1,2 @@",
                "1  fn a() {}",
                "2 -let x = 1;",
                "2 +let x = 2;"
            ]
        );
        assert_eq!(body.lines[3].spans[1].style, REMOVED);
        assert_eq!(body.lines[4].spans[1].style, ADDED);
        assert_eq!(body.paths, vec!["src/lib.rs"]);
    }
}
