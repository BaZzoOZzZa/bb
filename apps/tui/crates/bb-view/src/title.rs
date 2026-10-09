use bb_wire::JsonValue;
use bb_wire::interaction::UserQuestion;
use bb_wire::timeline::{
    ActivityIntent, ApprovalKind, ApprovalLifecycle, ApprovalWorkRow, CommandWorkRow,
    ConversationRole, ConversationRow, DelegationWorkRow, ExtensionWorkRow, FileChangeWorkRow,
    FileReadWorkRow, FormLifecycle, FormWorkRow, GrantScope, ImageGenerationWorkRow,
    ImageViewWorkRow, ItemPresentation, ParentChange, ParentChangeAction, PlanStepStatus,
    PlanStepsWorkRow, PresentationBadgeTone, QuestionLifecycle, QuestionWorkRow, SearchMode,
    SearchWorkRow, SystemOperationKind, SystemRow, SystemRowKind, TimelineApprovalStatus,
    TimelineRowStatus, TimelineWorkRow, ToolWorkRow, TurnRow, WebFetchWorkRow, WebSearchWorkRow,
    WorkflowWorkRow,
};

use crate::activity_intents::{
    ExplorationRow, IntentType, activity_intent_dedupe_key,
    format_timeline_activity_intent_detail_parts, has_timeline_exploration_intent, intent_type,
    timeline_row_activity_intents,
};
use crate::domain::{
    format_tool_call_command, is_background_agent_task_type, is_background_command_task_type,
    is_settled_workflow_agent_state,
};
use crate::file_change::{file_change_action, format_file_change_path, non_empty};
use crate::format::{duration_ms_to_compact_string, format_diff_stats_text};
use crate::path_display::{PathDisplayMode, file_name_from_path, format_timeline_path};
use crate::view::{
    SummaryKind, ViewRow, ViewTurnRow, ViewWorkRow, WorkSummaryRow,
    build_timeline_work_summary_label_parts,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TitleTone {
    #[default]
    Default,
    Summary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegmentAccent {
    Muted,
    Subtle,
    File,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TitleLink {
    Thread { thread_id: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TitleSegment {
    pub text: String,
    pub plain_text: Option<String>,
    pub em: bool,
    pub shimmer: bool,
    pub truncate: bool,
    pub accent: Option<SegmentAccent>,
    pub link: Option<TitleLink>,
}

impl TitleSegment {
    fn new(text: impl AsRef<str>) -> Self {
        Self {
            text: collapse_title_newlines(text.as_ref()),
            ..Self::default()
        }
    }

    fn em(mut self, em: bool) -> Self {
        self.em = em;
        self
    }

    fn shimmer(mut self, shimmer: bool) -> Self {
        self.shimmer = shimmer;
        self
    }

    fn truncate(mut self, truncate: bool) -> Self {
        self.truncate = truncate;
        self
    }

    fn plain(mut self, plain: impl AsRef<str>) -> Self {
        self.plain_text = Some(collapse_title_newlines(plain.as_ref()));
        self
    }

    fn accent(mut self, accent: Option<SegmentAccent>) -> Self {
        self.accent = accent;
        self
    }

    fn link(mut self, link: Option<TitleLink>) -> Self {
        self.link = link;
        self
    }

    pub fn plain_text(&self) -> &str {
        self.plain_text.as_deref().unwrap_or(&self.text)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusDecorationStatus {
    Denied,
    Error,
    Interrupted,
}

impl StatusDecorationStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Denied => "denied",
            Self::Error => "error",
            Self::Interrupted => "interrupted",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TitleDecoration {
    Duration {
        started_at: i64,
        completed_at: Option<i64>,
        em: bool,
    },
    Status {
        status: StatusDecorationStatus,
        duration_ms: Option<i64>,
        emphasis: bool,
    },
    SummaryStatus {
        error_count: usize,
        interrupted_count: usize,
    },
    DiffStats {
        added: u64,
        removed: u64,
    },
    Badge {
        glyph: String,
        label: String,
        hint: String,
        tone: PresentationBadgeTone,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TitleAction {
    OpenFileDiff { path: String },
    OpenPluginSideChat { thread_id: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimelineTitle {
    pub segments: Vec<TitleSegment>,
    pub decorations: Vec<TitleDecoration>,
    pub tone: TitleTone,
    pub action: Option<TitleAction>,
    pub plain: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SummaryStyle {
    #[default]
    Bundle,
    Background,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WorkStyle {
    #[default]
    Default,
    Summary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TitleOptions {
    pub summary_style: SummaryStyle,
    pub work_style: WorkStyle,
    pub is_active_latest_bundle: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ActivityIntentTitle {
    pub id: String,
    pub intent: ActivityIntent,
    pub title: TimelineTitle,
    pub intent_type: IntentType,
}

pub fn collapse_title_newlines(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_break = false;
    for ch in text.chars() {
        if ch == '\r' || ch == '\n' {
            if !in_break {
                out.push(' ');
                in_break = true;
            }
        } else {
            in_break = false;
            out.push(ch);
        }
    }
    out
}

fn visible_duration_ms(duration_ms: Option<i64>) -> Option<i64> {
    duration_ms.filter(|ms| *ms > 1_000)
}

fn duration_decoration(
    started_at: i64,
    completed_at: Option<i64>,
    em: bool,
) -> Option<TitleDecoration> {
    if let Some(completed) = completed_at
        && visible_duration_ms(Some(completed - started_at)).is_none()
    {
        return None;
    }
    Some(TitleDecoration::Duration {
        started_at,
        completed_at,
        em,
    })
}

fn badge_decorations(presentation: Option<&ItemPresentation>) -> Vec<TitleDecoration> {
    presentation
        .and_then(|presentation| presentation.badge.as_ref())
        .map(|badge| TitleDecoration::Badge {
            glyph: badge.glyph.clone(),
            label: badge.label.clone(),
            hint: badge.hint.clone(),
            tone: badge.tone,
        })
        .into_iter()
        .collect()
}

fn status_decoration(
    status: StatusDecorationStatus,
    duration_ms: Option<i64>,
    emphasis: bool,
) -> TitleDecoration {
    TitleDecoration::Status {
        status,
        duration_ms: visible_duration_ms(duration_ms),
        emphasis,
    }
}

fn elapsed(started_at: i64, completed_at: Option<i64>) -> Option<i64> {
    completed_at.map(|completed| completed - started_at)
}

pub fn format_timeline_decoration_text(decoration: &TitleDecoration) -> String {
    match decoration {
        TitleDecoration::Duration {
            started_at,
            completed_at,
            ..
        } => match completed_at {
            Some(completed) => format!(
                "({})",
                duration_ms_to_compact_string(completed - started_at)
            ),
            None => String::new(),
        },
        TitleDecoration::Status {
            status,
            duration_ms,
            ..
        } => match duration_ms {
            Some(ms) => format!(
                "({}, {})",
                duration_ms_to_compact_string(*ms),
                status.as_str()
            ),
            None => format!("({})", status.as_str()),
        },
        TitleDecoration::SummaryStatus {
            error_count,
            interrupted_count,
        } => {
            let mut parts = Vec::new();
            if *error_count > 0 {
                parts.push(format!(
                    "{error_count} error{}",
                    if *error_count > 1 { "s" } else { "" }
                ));
            }
            if *interrupted_count > 0 {
                parts.push(format!("{interrupted_count} interrupted"));
            }
            if parts.is_empty() {
                String::new()
            } else {
                format!("({})", parts.join(", "))
            }
        }
        TitleDecoration::DiffStats { added, removed } => {
            format_diff_stats_text(*added, *removed, true)
        }
        TitleDecoration::Badge { label, .. } => format!("({label})"),
    }
}

fn render_title_plain(segments: &[TitleSegment], decorations: &[TitleDecoration]) -> String {
    let segments_text = segments
        .iter()
        .map(TitleSegment::plain_text)
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let decorations_text = decorations
        .iter()
        .map(format_timeline_decoration_text)
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    match (segments_text.is_empty(), decorations_text.is_empty()) {
        (_, true) => segments_text,
        (true, false) => decorations_text,
        (false, false) => format!("{segments_text} {decorations_text}"),
    }
}

fn make_title(segments: Vec<TitleSegment>, decorations: Vec<TitleDecoration>) -> TimelineTitle {
    make_title_full(segments, decorations, TitleTone::Default, None)
}

fn make_title_full(
    segments: Vec<TitleSegment>,
    decorations: Vec<TitleDecoration>,
    tone: TitleTone,
    action: Option<TitleAction>,
) -> TimelineTitle {
    let plain = render_title_plain(&segments, &decorations);
    TimelineTitle {
        segments,
        decorations,
        tone,
        action,
        plain,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DisplayStatus {
    Waiting,
    Denied,
    Pending,
    Completed,
    Error,
    Interrupted,
}

fn row_display_status(status: TimelineRowStatus) -> DisplayStatus {
    match status {
        TimelineRowStatus::Pending => DisplayStatus::Pending,
        TimelineRowStatus::Error => DisplayStatus::Error,
        TimelineRowStatus::Interrupted => DisplayStatus::Interrupted,
        TimelineRowStatus::Completed | TimelineRowStatus::Unknown => DisplayStatus::Completed,
    }
}

fn display_status(
    approval: Option<TimelineApprovalStatus>,
    status: TimelineRowStatus,
) -> DisplayStatus {
    match approval {
        Some(TimelineApprovalStatus::WaitingForApproval) => DisplayStatus::Waiting,
        Some(TimelineApprovalStatus::Denied) => DisplayStatus::Denied,
        _ => row_display_status(status),
    }
}

fn failure_or_duration(
    status: TimelineRowStatus,
    started_at: i64,
    completed_at: Option<i64>,
    with_elapsed: bool,
) -> Vec<TitleDecoration> {
    let duration = if with_elapsed {
        elapsed(started_at, completed_at)
    } else {
        None
    };
    match status {
        TimelineRowStatus::Error => vec![status_decoration(
            StatusDecorationStatus::Error,
            duration,
            false,
        )],
        TimelineRowStatus::Interrupted => {
            vec![status_decoration(
                StatusDecorationStatus::Interrupted,
                duration,
                false,
            )]
        }
        _ => duration_decoration(started_at, completed_at, false)
            .into_iter()
            .collect(),
    }
}

struct Presented<'a> {
    presentation: &'a ItemPresentation,
    status: TimelineRowStatus,
    started_at: i64,
    completed_at: Option<i64>,
    content: Option<String>,
    plain_content: Option<String>,
    em: bool,
}

fn presented_title(args: Presented<'_>) -> TimelineTitle {
    let pending = args.status == TimelineRowStatus::Pending;
    let label = if pending {
        &args.presentation.label.pending
    } else {
        &args.presentation.label.completed
    };
    let content = args.content.filter(|content| !content.is_empty());
    let mut segments = vec![
        TitleSegment::new(label)
            .shimmer(pending)
            .truncate(content.is_none()),
    ];
    if let Some(content) = content {
        let mut segment = TitleSegment::new(content).em(args.em).truncate(true);
        if let Some(plain) = args.plain_content {
            segment = segment.plain(plain);
        }
        segments.push(segment);
    }
    let mut decorations = badge_decorations(Some(args.presentation));
    decorations.extend(failure_or_duration(
        args.status,
        args.started_at,
        args.completed_at,
        true,
    ));
    make_title(segments, decorations)
}

enum ExecutionRow<'a> {
    Command(&'a CommandWorkRow),
    Tool(&'a ToolWorkRow),
}

fn map_execution_title(row: ExecutionRow<'_>) -> TimelineTitle {
    let (approval, row_status, started_at, completed_at, presentation, is_command) = match &row {
        ExecutionRow::Command(row) => (
            row.approval_status,
            row.status,
            row.base.started_at,
            row.completed_at,
            row.presentation.as_ref(),
            true,
        ),
        ExecutionRow::Tool(row) => (
            row.approval_status,
            row.status,
            row.base.started_at,
            row.completed_at,
            row.presentation.as_ref(),
            false,
        ),
    };
    let status = display_status(approval, row_status);
    if let (Some(presentation), false) = (presentation, is_command)
        && status != DisplayStatus::Waiting
        && status != DisplayStatus::Denied
    {
        return presented_title(Presented {
            presentation,
            status: row_status,
            started_at,
            completed_at,
            content: presentation.title.clone(),
            plain_content: None,
            em: true,
        });
    }
    let content = match &row {
        ExecutionRow::Command(row) => row.command.clone(),
        ExecutionRow::Tool(row) => format_tool_call_command(&row.tool_name, row.tool_args.as_ref()),
    };
    if let ExecutionRow::Command(command) = &row
        && let Some(title) = map_single_exploration_intent_title(ExplorationRow::Command(command))
    {
        return title;
    }
    let badges = badge_decorations(presentation);
    let content_segment = || TitleSegment::new(&content).em(true).truncate(true);
    let with_badges = |extra: Vec<TitleDecoration>| {
        let mut decorations = badges.clone();
        decorations.extend(extra);
        decorations
    };
    let ran = if is_command { "Ran" } else { "Ran tool" };
    let duration = || {
        duration_decoration(started_at, completed_at, false)
            .into_iter()
            .collect()
    };
    match status {
        DisplayStatus::Waiting => make_title(
            vec![
                TitleSegment::new("Waiting for approval").shimmer(true),
                TitleSegment::new(if is_command { "to run" } else { "to use" }),
                content_segment(),
            ],
            badges.clone(),
        ),
        DisplayStatus::Denied => make_title(
            vec![TitleSegment::new("Permission denied:"), content_segment()],
            with_badges(duration()),
        ),
        DisplayStatus::Pending => make_title(
            vec![
                TitleSegment::new(if is_command {
                    "Running"
                } else {
                    "Running tool:"
                })
                .shimmer(true),
                content_segment(),
            ],
            with_badges(duration()),
        ),
        DisplayStatus::Completed => make_title(
            vec![TitleSegment::new(ran), content_segment()],
            with_badges(duration()),
        ),
        DisplayStatus::Error => make_title(
            vec![TitleSegment::new(ran), content_segment()],
            with_badges(vec![status_decoration(
                StatusDecorationStatus::Error,
                elapsed(started_at, completed_at),
                false,
            )]),
        ),
        DisplayStatus::Interrupted => make_title(
            vec![TitleSegment::new(ran), content_segment()],
            with_badges(vec![status_decoration(
                StatusDecorationStatus::Interrupted,
                elapsed(started_at, completed_at),
                false,
            )]),
        ),
    }
}

fn exploration_presentation<'a>(row: ExplorationRow<'a>) -> Option<&'a ItemPresentation> {
    match row {
        ExplorationRow::Command(row) => row.presentation.as_ref(),
        ExplorationRow::FileRead(row) => row.presentation.as_ref(),
        ExplorationRow::Search(row) => row.presentation.as_ref(),
    }
}

fn map_single_exploration_intent_title(row: ExplorationRow<'_>) -> Option<TimelineTitle> {
    if !has_timeline_exploration_intent(row) {
        return None;
    }
    let intents = timeline_row_activity_intents(row);
    let known: Vec<&ActivityIntent> = intents
        .iter()
        .filter(|intent| intent_type(intent).is_some())
        .collect();
    let [intent] = known.as_slice() else {
        return None;
    };
    let approval = match row {
        ExplorationRow::Command(row) => row.approval_status,
        _ => None,
    };
    let status = display_status(approval, row.status());
    let pending = status == DisplayStatus::Pending;
    let detail =
        format_timeline_activity_intent_detail_parts(intent, PathDisplayMode::Compact, pending);
    let plain_detail =
        format_timeline_activity_intent_detail_parts(intent, PathDisplayMode::Full, pending);
    let badges = badge_decorations(exploration_presentation(row));
    let verb_segment = || {
        TitleSegment::new(detail.joined())
            .em(true)
            .truncate(true)
            .plain(plain_detail.joined())
    };
    match status {
        DisplayStatus::Denied => {
            return Some(make_title(
                vec![TitleSegment::new("Permission denied:"), verb_segment()],
                badges,
            ));
        }
        DisplayStatus::Waiting => {
            return Some(make_title(
                vec![
                    TitleSegment::new("Waiting for approval").shimmer(true),
                    TitleSegment::new("to use"),
                    verb_segment(),
                ],
                badges,
            ));
        }
        _ => {}
    }
    let mut segments = Vec::new();
    if let Some(prefix) = detail.prefix.as_deref().filter(|prefix| !prefix.is_empty()) {
        segments.push(TitleSegment::new(prefix).shimmer(pending));
    }
    segments.push(
        TitleSegment::new(&detail.content)
            .truncate(true)
            .plain(&plain_detail.content),
    );
    let mut decorations = badges;
    match status {
        DisplayStatus::Error => decorations.push(status_decoration(
            StatusDecorationStatus::Error,
            None,
            false,
        )),
        DisplayStatus::Interrupted => decorations.push(status_decoration(
            StatusDecorationStatus::Interrupted,
            None,
            false,
        )),
        _ => {}
    }
    Some(make_title(segments, decorations))
}

fn diff_stats_decoration(row: &FileChangeWorkRow) -> Option<TitleDecoration> {
    let stats = &row.change.diff_stats;
    (stats.added != 0 || stats.removed != 0).then_some(TitleDecoration::DiffStats {
        added: stats.added,
        removed: stats.removed,
    })
}

fn map_file_change_title(row: &FileChangeWorkRow) -> TimelineTitle {
    let status = display_status(row.approval_status, row.status);
    let action = file_change_action(&row.change);
    let compact = format_file_change_path(&row.change, PathDisplayMode::Compact);
    let full = format_file_change_path(&row.change, PathDisplayMode::Full);
    let title_action = Some(TitleAction::OpenFileDiff {
        path: row
            .change
            .move_path
            .clone()
            .unwrap_or_else(|| row.change.path.clone()),
    });
    let path_segment = TitleSegment::new(compact)
        .em(true)
        .truncate(true)
        .plain(full)
        .accent(Some(SegmentAccent::File));
    let stats = || diff_stats_decoration(row).into_iter().collect::<Vec<_>>();
    let (segments, decorations) = match status {
        DisplayStatus::Waiting => (
            vec![
                TitleSegment::new("Waiting for approval").shimmer(true),
                TitleSegment::new("to edit"),
                path_segment,
            ],
            stats(),
        ),
        DisplayStatus::Denied => (
            vec![TitleSegment::new("Permission denied:"), path_segment],
            stats(),
        ),
        DisplayStatus::Pending => (
            vec![
                TitleSegment::new(action.present_tense()).shimmer(true),
                path_segment,
            ],
            stats(),
        ),
        DisplayStatus::Completed => (
            vec![TitleSegment::new(action.past_tense()), path_segment],
            stats(),
        ),
        DisplayStatus::Error => (
            vec![
                TitleSegment::new(format!("Failed to {}", action.infinitive())),
                path_segment,
            ],
            vec![status_decoration(
                StatusDecorationStatus::Error,
                None,
                false,
            )],
        ),
        DisplayStatus::Interrupted => (
            vec![
                TitleSegment::new(format!(
                    "Interrupted while {}",
                    action.present_tense().to_lowercase()
                )),
                path_segment,
            ],
            Vec::new(),
        ),
    };
    make_title_full(segments, decorations, TitleTone::Default, title_action)
}

struct SimpleLifecycle<'a> {
    pending: &'a str,
    completed: &'a str,
    error: &'a str,
    interrupted: &'a str,
}

fn simple_status_title(
    labels: SimpleLifecycle<'_>,
    status: TimelineRowStatus,
    content: TitleSegment,
    started_at: i64,
    completed_at: Option<i64>,
) -> TimelineTitle {
    let duration = elapsed(started_at, completed_at);
    match status {
        TimelineRowStatus::Pending => make_title(
            vec![TitleSegment::new(labels.pending).shimmer(true), content],
            Vec::new(),
        ),
        TimelineRowStatus::Error => make_title(
            vec![TitleSegment::new(labels.error), content],
            vec![status_decoration(
                StatusDecorationStatus::Error,
                duration,
                false,
            )],
        ),
        TimelineRowStatus::Interrupted => make_title(
            vec![TitleSegment::new(labels.interrupted), content],
            vec![status_decoration(
                StatusDecorationStatus::Interrupted,
                duration,
                false,
            )],
        ),
        TimelineRowStatus::Completed | TimelineRowStatus::Unknown => make_title(
            vec![TitleSegment::new(labels.completed), content],
            duration_decoration(started_at, completed_at, false)
                .into_iter()
                .collect(),
        ),
    }
}

fn map_web_search_title(row: &WebSearchWorkRow) -> TimelineTitle {
    if let Some(presentation) = &row.presentation {
        return presented_title(Presented {
            presentation,
            status: row.status,
            started_at: row.base.started_at,
            completed_at: row.completed_at,
            content: Some(row.queries.join(", ")),
            plain_content: None,
            em: false,
        });
    }
    let joined = row.queries.join(", ");
    let query = if joined.is_empty() {
        "web search".to_owned()
    } else {
        joined
    };
    simple_status_title(
        SimpleLifecycle {
            pending: "Running web search:",
            completed: "Ran web search:",
            error: "Ran web search:",
            interrupted: "Interrupted web search:",
        },
        row.status,
        TitleSegment::new(query).truncate(true),
        row.base.started_at,
        row.completed_at,
    )
}

fn map_web_fetch_title(row: &WebFetchWorkRow) -> TimelineTitle {
    if let Some(presentation) = &row.presentation {
        return presented_title(Presented {
            presentation,
            status: row.status,
            started_at: row.base.started_at,
            completed_at: row.completed_at,
            content: Some(row.url.clone()),
            plain_content: None,
            em: false,
        });
    }
    simple_status_title(
        SimpleLifecycle {
            pending: "Fetching:",
            completed: "Fetched:",
            error: "Fetched:",
            interrupted: "Interrupted fetch:",
        },
        row.status,
        TitleSegment::new(&row.url).truncate(true),
        row.base.started_at,
        row.completed_at,
    )
}

fn map_image_view_title(row: &ImageViewWorkRow) -> TimelineTitle {
    if let Some(presentation) = &row.presentation {
        return presented_title(Presented {
            presentation,
            status: row.status,
            started_at: row.base.started_at,
            completed_at: row.completed_at,
            content: Some(file_name_from_path(&row.path)),
            plain_content: Some(row.path.clone()),
            em: true,
        });
    }
    simple_status_title(
        SimpleLifecycle {
            pending: "Viewing image:",
            completed: "Viewed image:",
            error: "Viewed image:",
            interrupted: "Interrupted image view:",
        },
        row.status,
        TitleSegment::new(file_name_from_path(&row.path))
            .plain(&row.path)
            .truncate(true),
        row.base.started_at,
        row.completed_at,
    )
}

fn map_image_generation_title(row: &ImageGenerationWorkRow) -> TimelineTitle {
    if let Some(presentation) = &row.presentation {
        return presented_title(Presented {
            presentation,
            status: row.status,
            started_at: row.base.started_at,
            completed_at: row.completed_at,
            content: presentation.title.clone(),
            plain_content: None,
            em: true,
        });
    }
    let label = match row.status {
        TimelineRowStatus::Pending => "Generating image",
        TimelineRowStatus::Error => "Image generation failed",
        TimelineRowStatus::Interrupted => "Interrupted image generation",
        TimelineRowStatus::Completed | TimelineRowStatus::Unknown => "Generated image",
    };
    make_title(
        vec![TitleSegment::new(label).shimmer(row.status == TimelineRowStatus::Pending)],
        failure_or_duration(row.status, row.base.started_at, row.completed_at, false),
    )
}

fn lifecycle_verb(status: TimelineRowStatus, labels: [&'static str; 4]) -> (&'static str, bool) {
    match status {
        TimelineRowStatus::Pending => (labels[0], true),
        TimelineRowStatus::Error => (labels[2], false),
        TimelineRowStatus::Interrupted => (labels[3], false),
        TimelineRowStatus::Completed | TimelineRowStatus::Unknown => (labels[1], false),
    }
}

fn map_delegation_title(row: &DelegationWorkRow) -> TimelineTitle {
    let description = row.description.clone().unwrap_or_else(|| {
        let trimmed = row.output.trim();
        if trimmed.is_empty() {
            row.tool_name.clone()
        } else {
            trimmed.to_owned()
        }
    });
    if let Some(presentation) = &row.presentation {
        return presented_title(Presented {
            presentation,
            status: row.status,
            started_at: row.base.started_at,
            completed_at: row.completed_at,
            content: Some(presentation.title.clone().unwrap_or(description)),
            plain_content: None,
            em: true,
        });
    }
    let (verb, shimmer) = lifecycle_verb(
        row.status,
        [
            "Running subagent:",
            "Ran subagent:",
            "Failed subagent:",
            "Interrupted subagent:",
        ],
    );
    let mut segments = vec![
        TitleSegment::new(verb).shimmer(shimmer),
        TitleSegment::new(description).em(true).truncate(true),
    ];
    if let Some(subagent_type) = non_empty(row.subagent_type.as_ref()) {
        segments.push(TitleSegment::new(format!("({subagent_type})")).truncate(true));
    }
    make_title(
        segments,
        duration_decoration(row.base.started_at, row.completed_at, false)
            .into_iter()
            .collect(),
    )
}

fn map_workflow_title(row: &WorkflowWorkRow) -> TimelineTitle {
    let duration = || {
        duration_decoration(row.base.started_at, row.completed_at, false)
            .into_iter()
            .collect::<Vec<_>>()
    };
    let labels = if is_background_command_task_type(&row.task_type) {
        Some([
            "Running background command:",
            "Ran background command:",
            "Background command failed:",
            "Background command interrupted:",
        ])
    } else if is_background_agent_task_type(&row.task_type) {
        Some([
            "Running background agent:",
            "Ran background agent:",
            "Background agent failed:",
            "Background agent interrupted:",
        ])
    } else {
        None
    };
    if let Some(labels) = labels {
        let (verb, shimmer) = lifecycle_verb(row.status, labels);
        return make_title(
            vec![
                TitleSegment::new(verb).shimmer(shimmer),
                TitleSegment::new(&row.description).em(true).truncate(true),
            ],
            duration(),
        );
    }
    let (verb, shimmer) = lifecycle_verb(
        row.status,
        [
            "Running workflow:",
            "Ran workflow:",
            "Failed workflow:",
            "Interrupted workflow:",
        ],
    );
    let name = row.workflow_name.as_deref().unwrap_or(&row.description);
    let mut segments = vec![
        TitleSegment::new(verb).shimmer(shimmer),
        TitleSegment::new(name).em(true).truncate(true),
    ];
    let agents = row
        .workflow
        .as_ref()
        .map(|workflow| workflow.agents.as_slice())
        .unwrap_or_default();
    if !agents.is_empty() {
        let done = agents
            .iter()
            .filter(|agent| is_settled_workflow_agent_state(agent.state))
            .count();
        segments.push(TitleSegment::new(format!(
            "({done}/{} agents)",
            agents.len()
        )));
    }
    make_title(segments, duration())
}

fn map_file_edit_approval_title(row: &ApprovalWorkRow) -> TimelineTitle {
    match row.lifecycle {
        ApprovalLifecycle::Denied | ApprovalLifecycle::Interrupted => make_title(
            vec![
                TitleSegment::new("Permission denied:"),
                TitleSegment::new("file changes").em(true).truncate(true),
            ],
            Vec::new(),
        ),
        ApprovalLifecycle::Granted => make_title(
            vec![
                TitleSegment::new("Permission granted:"),
                TitleSegment::new("file changes").em(true).truncate(true),
            ],
            Vec::new(),
        ),
        ApprovalLifecycle::Waiting
        | ApprovalLifecycle::Pending
        | ApprovalLifecycle::Resolving
        | ApprovalLifecycle::Unknown => make_title(
            vec![
                TitleSegment::new("Waiting for approval to edit").shimmer(true),
                TitleSegment::new("files").em(true).truncate(true),
            ],
            Vec::new(),
        ),
    }
}

fn map_permission_grant_approval_title(row: &ApprovalWorkRow) -> TimelineTitle {
    let tool = row.target.tool_name.as_deref();
    let tool_segment = |name: &str| TitleSegment::new(name).em(true).truncate(true);
    let reason = row
        .status_reason
        .as_deref()
        .map(str::trim)
        .filter(|reason| !reason.is_empty());
    let segments = match row.lifecycle {
        ApprovalLifecycle::Pending | ApprovalLifecycle::Waiting | ApprovalLifecycle::Unknown => {
            match tool {
                Some(tool) => vec![
                    TitleSegment::new("Waiting for permission").shimmer(true),
                    TitleSegment::new("to use"),
                    tool_segment(tool),
                ],
                None => vec![TitleSegment::new("Waiting for permissions").shimmer(true)],
            }
        }
        ApprovalLifecycle::Resolving => match tool {
            Some(tool) => vec![
                TitleSegment::new("Delivering permission").shimmer(true),
                TitleSegment::new("to use"),
                tool_segment(tool),
            ],
            None => vec![TitleSegment::new("Delivering permissions").shimmer(true)],
        },
        ApprovalLifecycle::Granted => {
            let scope = match row.grant_scope {
                Some(GrantScope::Turn) => Some("for this turn"),
                Some(GrantScope::Session) => Some("for this session"),
                _ => None,
            };
            let base = match scope {
                Some(scope) => format!("Permission granted {scope}"),
                None => "Permission granted".to_owned(),
            };
            match tool {
                Some(tool) => vec![TitleSegment::new(format!("{base}:")), tool_segment(tool)],
                None => vec![TitleSegment::new(base)],
            }
        }
        ApprovalLifecycle::Denied => match tool {
            Some(tool) => vec![TitleSegment::new("Permission denied:"), tool_segment(tool)],
            None => vec![TitleSegment::new("Permission denied")],
        },
        ApprovalLifecycle::Interrupted => {
            let mut segments = match tool {
                Some(tool) => vec![
                    TitleSegment::new("Permission grant interrupted:"),
                    tool_segment(tool),
                ],
                None => vec![TitleSegment::new("Permission grant interrupted")],
            };
            if let Some(reason) = reason {
                segments.push(TitleSegment::new(format!("({reason})")).truncate(true));
            }
            segments
        }
    };
    make_title(segments, Vec::new())
}

fn map_approval_title(row: &ApprovalWorkRow) -> TimelineTitle {
    match row.approval_kind {
        ApprovalKind::FileEdit => map_file_edit_approval_title(row),
        ApprovalKind::PermissionGrant | ApprovalKind::Unknown => {
            map_permission_grant_approval_title(row)
        }
    }
}

fn single_question_answer_summary(
    row: &QuestionWorkRow,
    question: &UserQuestion,
) -> Option<String> {
    let answer = row.answers.as_ref()?.get(&question.id)?;
    let mut parts: Vec<String> = answer
        .selected
        .iter()
        .map(|value| {
            question
                .options
                .as_ref()
                .and_then(|options| options.iter().find(|option| &option.value == value))
                .map_or_else(|| value.clone(), |option| option.label.clone())
        })
        .collect();
    if let Some(free_text) = non_empty(answer.free_text.as_ref()) {
        parts.push(free_text.to_owned());
    }
    let text = parts.join(", ");
    (!text.is_empty()).then_some(text)
}

fn map_question_title(row: &QuestionWorkRow) -> TimelineTitle {
    let question = match row.questions.as_slice() {
        [question] => Some(question),
        _ => None,
    };
    let subject = match question {
        Some(question) => TitleSegment::new(&question.prompt).em(true).truncate(true),
        None => TitleSegment::new(format!("{} questions", row.questions.len())).em(true),
    };
    match row.lifecycle {
        QuestionLifecycle::Pending => make_title(
            vec![
                TitleSegment::new(if question.is_some() {
                    "Waiting for answer"
                } else {
                    "Waiting for answers to"
                })
                .shimmer(true),
                subject,
            ],
            Vec::new(),
        ),
        QuestionLifecycle::Resolving => make_title(
            vec![
                TitleSegment::new(if question.is_some() {
                    "Delivering answer"
                } else {
                    "Delivering answers to"
                })
                .shimmer(true),
                subject,
            ],
            Vec::new(),
        ),
        QuestionLifecycle::Interrupted => make_title(
            vec![TitleSegment::new("Asked"), subject],
            vec![status_decoration(
                StatusDecorationStatus::Interrupted,
                None,
                false,
            )],
        ),
        QuestionLifecycle::Answered | QuestionLifecycle::Unknown => {
            let mut segments = vec![TitleSegment::new("Answered"), subject];
            if let Some(summary) =
                question.and_then(|question| single_question_answer_summary(row, question))
            {
                segments.push(TitleSegment::new(format!("— {summary}")).truncate(true));
            }
            make_title(segments, Vec::new())
        }
    }
}

fn map_form_title(row: &FormWorkRow) -> TimelineTitle {
    let presentation = &row.presentation;
    match row.lifecycle {
        FormLifecycle::Pending => make_title(
            vec![TitleSegment::new(&presentation.label.pending).shimmer(true)],
            Vec::new(),
        ),
        FormLifecycle::Cancelled => make_title(
            vec![TitleSegment::new(&presentation.label.completed).truncate(true)],
            vec![status_decoration(
                StatusDecorationStatus::Interrupted,
                None,
                false,
            )],
        ),
        FormLifecycle::Submitted | FormLifecycle::Unknown => make_title(
            vec![
                TitleSegment::new(
                    presentation
                        .title
                        .as_deref()
                        .unwrap_or(&presentation.label.completed),
                )
                .truncate(true),
            ],
            Vec::new(),
        ),
    }
}

fn map_file_read_title(row: &FileReadWorkRow) -> TimelineTitle {
    if let Some(presentation) = &row.presentation {
        return presented_title(Presented {
            presentation,
            status: row.status,
            started_at: row.base.started_at,
            completed_at: row.completed_at,
            content: Some(format_timeline_path(&row.path, PathDisplayMode::Compact)),
            plain_content: Some(row.path.clone()),
            em: true,
        });
    }
    map_single_exploration_intent_title(ExplorationRow::FileRead(row)).unwrap_or_else(|| {
        let pending = row.status == TimelineRowStatus::Pending;
        make_title(
            vec![
                TitleSegment::new(if pending { "Reading" } else { "Read" }).shimmer(pending),
                TitleSegment::new(&row.path).em(true).truncate(true),
            ],
            Vec::new(),
        )
    })
}

fn search_content(row: &SearchWorkRow) -> String {
    let root = row
        .path
        .as_deref()
        .filter(|path| !path.is_empty())
        .map(|path| format!(" in {path}"))
        .unwrap_or_default();
    match row.mode {
        SearchMode::Content => format!("for {}{root}", row.query),
        SearchMode::Path => format!("matching {}{root}", row.query),
        SearchMode::List | SearchMode::Unknown => match non_empty(row.path.as_ref()) {
            Some(path) => format!("in {path}"),
            None if !row.query.is_empty() => format!("in {}", row.query),
            None => String::new(),
        },
    }
}

fn map_search_title(row: &SearchWorkRow) -> TimelineTitle {
    if let Some(presentation) = &row.presentation {
        return presented_title(Presented {
            presentation,
            status: row.status,
            started_at: row.base.started_at,
            completed_at: row.completed_at,
            content: Some(search_content(row)),
            plain_content: None,
            em: false,
        });
    }
    map_single_exploration_intent_title(ExplorationRow::Search(row)).unwrap_or_else(|| {
        let pending = row.status == TimelineRowStatus::Pending;
        make_title(
            vec![
                TitleSegment::new(if pending { "Searching" } else { "Searched" }).shimmer(pending),
                TitleSegment::new(search_content(row)).truncate(true),
            ],
            Vec::new(),
        )
    })
}

fn map_plan_steps_title(row: &PlanStepsWorkRow) -> TimelineTitle {
    let content = row
        .steps
        .iter()
        .find(|step| step.status == Some(PlanStepStatus::Active))
        .map(|step| step.step.clone())
        .or_else(|| row.explanation.clone());
    if let Some(presentation) = &row.presentation {
        return presented_title(Presented {
            presentation,
            status: row.status,
            started_at: row.base.started_at,
            completed_at: row.completed_at,
            content: presentation.title.clone().or(content),
            plain_content: None,
            em: false,
        });
    }
    let completed = row
        .steps
        .iter()
        .filter(|step| step.status == Some(PlanStepStatus::Completed))
        .count();
    let pending = row.status == TimelineRowStatus::Pending;
    make_title(
        vec![
            TitleSegment::new(if pending {
                "Updating plan"
            } else {
                "Updated plan"
            })
            .shimmer(pending),
            TitleSegment::new(
                content.unwrap_or_else(|| format!("{completed}/{} steps done", row.steps.len())),
            )
            .truncate(true),
        ],
        duration_decoration(row.base.started_at, row.completed_at, false)
            .into_iter()
            .collect(),
    )
}

fn map_extension_title(row: &ExtensionWorkRow) -> TimelineTitle {
    presented_title(Presented {
        presentation: &row.presentation,
        status: row.status,
        started_at: row.base.started_at,
        completed_at: row.completed_at,
        content: row.presentation.title.clone(),
        plain_content: None,
        em: false,
    })
}

fn unknown_kind_label(value: &JsonValue, default_kind: &str) -> String {
    let kind = value
        .get("kind")
        .and_then(JsonValue::as_str)
        .unwrap_or(default_kind);
    match value.get("workKind").and_then(JsonValue::as_str) {
        Some(work_kind) => format!("Unsupported {kind} row ({work_kind})"),
        None => format!("Unsupported {kind} row"),
    }
}

pub fn build_work_row_title(row: &TimelineWorkRow, options: TitleOptions) -> TimelineTitle {
    let mut title = match row {
        TimelineWorkRow::Command(row) => map_execution_title(ExecutionRow::Command(row)),
        TimelineWorkRow::Tool(row) => map_execution_title(ExecutionRow::Tool(row)),
        TimelineWorkRow::FileRead(row) => map_file_read_title(row),
        TimelineWorkRow::Search(row) => map_search_title(row),
        TimelineWorkRow::PlanSteps(row) => map_plan_steps_title(row),
        TimelineWorkRow::Extension(row) => map_extension_title(row),
        TimelineWorkRow::FileChange(row) => map_file_change_title(row),
        TimelineWorkRow::WebSearch(row) => map_web_search_title(row),
        TimelineWorkRow::WebFetch(row) => map_web_fetch_title(row),
        TimelineWorkRow::ImageGeneration(row) => map_image_generation_title(row),
        TimelineWorkRow::ImageView(row) => map_image_view_title(row),
        TimelineWorkRow::Delegation(row) => map_delegation_title(row),
        TimelineWorkRow::Workflow(row) => map_workflow_title(row),
        TimelineWorkRow::Approval(row) => map_approval_title(row),
        TimelineWorkRow::Question(row) => map_question_title(row),
        TimelineWorkRow::Form(row) => map_form_title(row),
        TimelineWorkRow::Unknown(value) => make_title(
            vec![TitleSegment::new(unknown_kind_label(value, "work")).truncate(true)],
            Vec::new(),
        ),
    };
    if options.work_style == WorkStyle::Summary {
        title.tone = TitleTone::Summary;
    }
    title
}

fn summary_status_decoration(row: &WorkSummaryRow) -> Option<TitleDecoration> {
    let error_count = row
        .children
        .iter()
        .filter(|child| child.status() == TimelineRowStatus::Error)
        .count();
    let interrupted_count = row
        .children
        .iter()
        .filter(|child| child.status() == TimelineRowStatus::Interrupted)
        .count();
    (error_count > 0 || interrupted_count > 0).then_some(TitleDecoration::SummaryStatus {
        error_count,
        interrupted_count,
    })
}

fn map_work_summary_title(row: &WorkSummaryRow, options: TitleOptions) -> TimelineTitle {
    let active = row.kind == SummaryKind::Bundle && options.is_active_latest_bundle;
    let parts = build_timeline_work_summary_label_parts(row, active);
    let decorations: Vec<TitleDecoration> = summary_status_decoration(row).into_iter().collect();
    if options.summary_style == SummaryStyle::Background {
        let label = if parts.rest.is_empty() {
            parts.verb
        } else {
            format!("{} {}", parts.verb, parts.rest)
        };
        return make_title_full(
            vec![TitleSegment::new(label).truncate(true)],
            decorations,
            TitleTone::Summary,
            None,
        );
    }
    let accent = (!active).then_some(SegmentAccent::Subtle);
    let verb = TitleSegment::new(&parts.verb)
        .shimmer(active)
        .accent(accent);
    if parts.rest.is_empty() {
        return make_title(vec![verb.truncate(true)], decorations);
    }
    make_title(
        vec![
            verb,
            TitleSegment::new(&parts.rest)
                .em(true)
                .truncate(true)
                .accent(accent),
        ],
        decorations,
    )
}

fn map_turn_title(row: &TurnRow) -> TimelineTitle {
    let pending = row.status == TimelineRowStatus::Pending;
    let duration = if pending {
        duration_decoration(row.base.started_at, row.completed_at, true)
    } else {
        row.completed_at
            .map(|completed_at| TitleDecoration::Duration {
                started_at: row.base.started_at,
                completed_at: Some(completed_at),
                em: false,
            })
    };
    if !pending && let Some(duration) = duration.clone() {
        return make_title(
            vec![TitleSegment::new("Worked for").accent(Some(SegmentAccent::Subtle))],
            vec![duration],
        );
    }
    make_title(
        vec![
            TitleSegment::new(if pending { "Working" } else { "Worked" })
                .shimmer(pending)
                .accent((!pending).then_some(SegmentAccent::Subtle)),
        ],
        if pending {
            duration.into_iter().collect()
        } else {
            Vec::new()
        },
    )
}

pub fn ownership_change_verb(action: ParentChangeAction) -> &'static str {
    match action {
        ParentChangeAction::Release => "released from",
        ParentChangeAction::Transfer => "transferred to",
        ParentChangeAction::Assign | ParentChangeAction::Unknown => "assigned to",
    }
}

fn parent_change_thread_name(row: &SystemRow, change: &ParentChange) -> String {
    let verb = ownership_change_verb(change.action);
    let parent_title = if change.action == ParentChangeAction::Release {
        change.previous_parent_thread_title.as_deref()
    } else {
        change.next_parent_thread_title.as_deref()
    };
    let parent = parent_title
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .unwrap_or("parent");
    let suffix = format!(" {verb} {parent}");
    if row.title.ends_with(&suffix) && row.title.len() > suffix.len() {
        return row.title[..row.title.len() - suffix.len()].to_owned();
    }
    row.base.thread_id.clone()
}

fn map_parent_change_title(row: &SystemRow, change: &ParentChange) -> TimelineTitle {
    let status = row.status.unwrap_or(TimelineRowStatus::Completed);
    let shimmer = status == TimelineRowStatus::Pending;
    let verb = ownership_change_verb(change.action);
    let thread_link = (!row.base.thread_id.is_empty()).then(|| TitleLink::Thread {
        thread_id: row.base.thread_id.clone(),
    });
    let thread_segment = TitleSegment::new(parent_change_thread_name(row, change))
        .em(true)
        .truncate(true)
        .shimmer(shimmer)
        .link(thread_link);
    let (parent_id, parent_title) = if change.action == ParentChangeAction::Release {
        (
            change.previous_parent_thread_id.as_ref(),
            change.previous_parent_thread_title.as_ref(),
        )
    } else {
        (
            change.next_parent_thread_id.as_ref(),
            change.next_parent_thread_title.as_ref(),
        )
    };
    let parent_segment = match parent_id {
        None => TitleSegment::new("parent"),
        Some(id) => TitleSegment::new(parent_title.unwrap_or(id))
            .em(true)
            .truncate(true)
            .link(Some(TitleLink::Thread {
                thread_id: id.clone(),
            })),
    };
    let decorations = match status {
        TimelineRowStatus::Error => {
            vec![status_decoration(StatusDecorationStatus::Error, None, true)]
        }
        TimelineRowStatus::Interrupted => {
            vec![status_decoration(
                StatusDecorationStatus::Interrupted,
                None,
                false,
            )]
        }
        _ => Vec::new(),
    };
    make_title(
        vec![
            thread_segment,
            TitleSegment::new(verb)
                .shimmer(shimmer)
                .accent(Some(SegmentAccent::Muted)),
            parent_segment,
        ],
        decorations,
    )
}

pub fn build_system_row_title(row: &SystemRow) -> TimelineTitle {
    let is_operation = row.system_kind == SystemRowKind::Operation;
    if is_operation
        && row.operation_kind == Some(SystemOperationKind::ParentChange)
        && let Some(change) = &row.parent_change
    {
        return map_parent_change_title(row, change);
    }
    let has_error =
        row.system_kind == SystemRowKind::Error || row.status == Some(TimelineRowStatus::Error);
    let is_reasoning = is_operation && row.operation_kind == Some(SystemOperationKind::Reasoning);
    let is_compaction = is_operation && row.operation_kind == Some(SystemOperationKind::Compaction);
    let pending = row.status == Some(TimelineRowStatus::Pending);
    let title_text = if is_reasoning {
        if pending {
            "Thinking…".to_owned()
        } else {
            "Thought".to_owned()
        }
    } else if is_compaction && pending {
        format!("{}…", row.title)
    } else {
        row.title.clone()
    };
    let decorations = if has_error {
        vec![status_decoration(StatusDecorationStatus::Error, None, true)]
    } else if is_reasoning
        || (is_compaction && (pending || row.status == Some(TimelineRowStatus::Completed)))
    {
        duration_decoration(row.base.started_at, row.completed_at, false)
            .into_iter()
            .collect()
    } else {
        Vec::new()
    };
    make_title(
        vec![
            TitleSegment::new(title_text)
                .shimmer(pending)
                .truncate(true),
        ],
        decorations,
    )
}

fn map_conversation_title(row: &ConversationRow) -> TimelineTitle {
    make_title(
        vec![TitleSegment::new(if row.role == ConversationRole::User {
            "User"
        } else {
            "Assistant"
        })],
        Vec::new(),
    )
}

pub fn build_timeline_row_title(row: &ViewRow, options: TitleOptions) -> TimelineTitle {
    match row {
        ViewRow::Conversation(row) => map_conversation_title(row),
        ViewRow::System(row) => build_system_row_title(row),
        ViewRow::Work(row) => build_view_work_row_title(row, options),
        ViewRow::Summary(row) => map_work_summary_title(row, options),
        ViewRow::Turn(row) => build_turn_row_title(row),
        ViewRow::Unknown(value) => make_title(
            vec![TitleSegment::new(unknown_kind_label(value, "timeline")).truncate(true)],
            Vec::new(),
        ),
    }
}

pub fn build_view_work_row_title(row: &ViewWorkRow, options: TitleOptions) -> TimelineTitle {
    build_work_row_title(&row.row, options)
}

pub fn build_turn_row_title(row: &ViewTurnRow) -> TimelineTitle {
    map_turn_title(&row.row)
}

pub fn build_summary_row_title(row: &WorkSummaryRow, options: TitleOptions) -> TimelineTitle {
    map_work_summary_title(row, options)
}

fn map_activity_intent_title(
    intent: &ActivityIntent,
    pending: bool,
    failure: Option<StatusDecorationStatus>,
    badges: Vec<TitleDecoration>,
) -> TimelineTitle {
    let detail =
        format_timeline_activity_intent_detail_parts(intent, PathDisplayMode::Compact, pending);
    let plain_detail =
        format_timeline_activity_intent_detail_parts(intent, PathDisplayMode::Full, pending);
    let mut segments = Vec::new();
    if let Some(prefix) = detail.prefix.as_deref().filter(|prefix| !prefix.is_empty()) {
        segments.push(TitleSegment::new(prefix).shimmer(pending));
    }
    segments.push(
        TitleSegment::new(&detail.content)
            .truncate(true)
            .plain(&plain_detail.content),
    );
    let mut decorations = badges;
    if let Some(failure) = failure {
        decorations.push(status_decoration(failure, None, false));
    }
    make_title(segments, decorations)
}

pub fn build_timeline_activity_intent_titles(row: ExplorationRow<'_>) -> Vec<ActivityIntentTitle> {
    if !has_timeline_exploration_intent(row) {
        return Vec::new();
    }
    let status = row.status();
    let failure = match status {
        TimelineRowStatus::Error => Some(StatusDecorationStatus::Error),
        TimelineRowStatus::Interrupted => Some(StatusDecorationStatus::Interrupted),
        _ => None,
    };
    let badges = badge_decorations(exploration_presentation(row));
    let mut last_key: Option<String> = None;
    let mut titles = Vec::new();
    for (index, intent) in timeline_row_activity_intents(row).iter().enumerate() {
        let Some(kind) = intent_type(intent) else {
            continue;
        };
        let key = activity_intent_dedupe_key(intent);
        if key.is_some() && key == last_key {
            continue;
        }
        let title = map_activity_intent_title(
            intent,
            status == TimelineRowStatus::Pending,
            failure,
            if titles.is_empty() {
                badges.clone()
            } else {
                Vec::new()
            },
        );
        titles.push(ActivityIntentTitle {
            id: format!("{}:activity-intent:{index}", row.id()),
            intent: intent.clone(),
            title,
            intent_type: kind,
        });
        last_key = key;
    }
    titles
}
