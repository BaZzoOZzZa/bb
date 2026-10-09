use std::collections::BTreeMap;

use bb_wire::JsonValue;
use bb_wire::interaction::{
    ASK_USER_QUESTION_RENDERER_ID, ApprovalDecision, ApprovalPayload, ApprovalResolution,
    ApprovalSubject, AskUserQuestionPluginPayload, CommandAction, InteractionOrigin,
    InteractionPayload, InteractionResolution, MacOsAccessLevel, MacOsPermissions,
    NetworkPermissions, PendingInteraction, PendingInteractionStatus, PermissionProfile,
    RequestedPermissionProfile, SECRET_REQUEST_RENDERER_ID, SecretRequestPayload,
    USER_QUESTION_MAX_FREE_TEXT_LENGTH, USER_QUESTION_MAX_OPTIONS, USER_QUESTION_MAX_QUESTIONS,
    USER_QUESTION_MAX_SELECTED, UserAnswer, UserQuestion, UserQuestionOption,
};
use bb_wire::timeline::ItemPresentation;

pub const OTHER_OPTION_LABEL: &str = "Other…";
pub const SECRET_VALUE_MAX_LENGTH: usize = 16 * 1024;
const APPROVAL_DECISION_ORDER: [ApprovalDecision; 3] = [
    ApprovalDecision::Deny,
    ApprovalDecision::AllowForSession,
    ApprovalDecision::AllowOnce,
];

pub fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

fn access_label(level: MacOsAccessLevel) -> &'static str {
    match level {
        MacOsAccessLevel::ReadOnly => "read only",
        MacOsAccessLevel::ReadWrite => "read write",
        MacOsAccessLevel::None | MacOsAccessLevel::Unknown => "none",
    }
}

fn summarize_macos(permissions: &MacOsPermissions) -> Vec<String> {
    let mut summaries = Vec::new();
    if permissions.accessibility {
        summaries.push("macOS accessibility".to_owned());
    }
    if permissions.launch_services {
        summaries.push("macOS launch services".to_owned());
    }
    if permissions.calendar {
        summaries.push("macOS calendar".to_owned());
    }
    if permissions.reminders {
        summaries.push("macOS reminders".to_owned());
    }
    if !matches!(
        permissions.preferences,
        MacOsAccessLevel::None | MacOsAccessLevel::Unknown
    ) {
        summaries.push(format!(
            "macOS preferences ({})",
            access_label(permissions.preferences)
        ));
    }
    if !matches!(
        permissions.contacts,
        MacOsAccessLevel::None | MacOsAccessLevel::Unknown
    ) {
        summaries.push(format!(
            "macOS contacts ({})",
            access_label(permissions.contacts)
        ));
    }
    match permissions.automation_bundle_count() {
        None => summaries.push("macOS automation (all apps)".to_owned()),
        Some(1) => summaries.push("macOS automation (1 app)".to_owned()),
        Some(0) => {}
        Some(count) => summaries.push(format!("macOS automation ({count} apps)")),
    }
    summaries
}

fn count_label(verb: &str, count: usize) -> String {
    if count == 1 {
        format!("{verb} 1 path")
    } else {
        format!("{verb} {count} paths")
    }
}

fn summarize_base(
    network: Option<&NetworkPermissions>,
    file_system: Option<&bb_wire::interaction::FileSystemPermissions>,
) -> Vec<String> {
    let mut summaries = Vec::new();
    if network.and_then(|network| network.enabled) == Some(true) {
        summaries.push("Network access".to_owned());
    }
    if let Some(file_system) = file_system {
        if !file_system.read.is_empty() {
            summaries.push(count_label("Read", file_system.read.len()));
        }
        if !file_system.write.is_empty() {
            summaries.push(count_label("Write", file_system.write.len()));
        }
    }
    summaries
}

pub fn summarize_grantable_permissions(permissions: &PermissionProfile) -> Vec<String> {
    summarize_base(
        permissions.network.as_ref(),
        permissions.file_system.as_ref(),
    )
}

pub fn summarize_requested_permissions(permissions: &RequestedPermissionProfile) -> Vec<String> {
    let mut summaries = summarize_base(
        permissions.network.as_ref(),
        permissions.file_system.as_ref(),
    );
    if let Some(macos) = &permissions.macos {
        summaries.extend(summarize_macos(macos));
    }
    summaries
}

fn summarize_command_actions(actions: &[CommandAction]) -> Vec<String> {
    actions
        .iter()
        .filter_map(|action| match action {
            CommandAction::Read { path, .. } => Some(format!("Read {path}")),
            CommandAction::ListFiles { path, .. } => Some(match path.as_deref() {
                Some(path) if !path.is_empty() => format!("List files in {path}"),
                _ => "List files".to_owned(),
            }),
            CommandAction::Search { query, path, .. } => {
                let path = path.as_deref().filter(|path| !path.is_empty());
                Some(match query.as_deref().filter(|query| !query.is_empty()) {
                    Some(query) => match path {
                        Some(path) => format!("Search for {query} in {path}"),
                        None => format!("Search for {query}"),
                    },
                    None => match path {
                        Some(path) => format!("Search in {path}"),
                        None => "Search files".to_owned(),
                    },
                })
            }
            CommandAction::Generic { command } => Some(command.clone()),
            CommandAction::Unknown(_) => None,
        })
        .collect()
}

fn permission_summary_line(label: &str, permissions: Option<&PermissionProfile>) -> Option<String> {
    let summaries = summarize_grantable_permissions(permissions?);
    (!summaries.is_empty()).then(|| format!("{label}: {}", summaries.join(", ")))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolUseAsk {
    pub title: String,
    pub tool: String,
    pub headline: Option<String>,
    pub detail: Option<String>,
}

fn present_as_text(value: Option<&String>) -> Option<String> {
    value.filter(|value| !value.trim().is_empty()).cloned()
}

pub fn describe_tool_use(
    reason: Option<&str>,
    tool: &str,
    presentation: &ItemPresentation,
) -> ToolUseAsk {
    ToolUseAsk {
        title: reason
            .map(str::to_owned)
            .unwrap_or_else(|| presentation.label.pending.clone()),
        tool: tool.to_owned(),
        headline: present_as_text(presentation.title.as_ref()),
        detail: present_as_text(presentation.detail.as_ref()),
    }
}

pub fn tool_use_detail_lines(ask: &ToolUseAsk) -> Vec<String> {
    let mut lines = vec![format!("Tool: {}", ask.tool)];
    lines.extend(ask.headline.clone());
    lines.extend(ask.detail.clone());
    lines
}

pub fn subject_detail_lines(interaction: &PendingInteraction) -> Vec<String> {
    let payload = match &interaction.payload {
        InteractionPayload::UserQuestion(payload) => {
            return payload
                .questions
                .iter()
                .map(|question| question.prompt.clone())
                .collect();
        }
        InteractionPayload::Approval(payload) => payload,
        _ => return Vec::new(),
    };
    match &payload.subject {
        ApprovalSubject::Command {
            command,
            cwd,
            actions,
            session_grant,
            ..
        } => {
            let mut lines = vec![format!("Command: {command}")];
            if let Some(cwd) = cwd.as_deref().filter(|cwd| !cwd.is_empty()) {
                lines.push(format!("Cwd: {cwd}"));
            }
            lines.extend(
                summarize_command_actions(actions)
                    .into_iter()
                    .map(|action| format!("Action: {action}")),
            );
            lines.extend(permission_summary_line(
                "Session grant",
                session_grant.as_ref(),
            ));
            lines
        }
        ApprovalSubject::FileChange {
            item_id,
            write_scope,
            session_grant,
        } => {
            let mut lines = vec![format!("Item: {item_id}")];
            if let Some(scope) = write_scope.as_deref().filter(|scope| !scope.is_empty()) {
                lines.push(format!("Write root: {scope}"));
            }
            lines.extend(permission_summary_line(
                "Session grant",
                session_grant.as_ref(),
            ));
            lines
        }
        ApprovalSubject::PermissionGrant {
            tool_name,
            permissions,
            ..
        } => {
            let mut lines = Vec::new();
            if let Some(tool) = tool_name.as_deref().filter(|tool| !tool.is_empty()) {
                lines.push(format!("Tool: {tool}"));
            }
            lines.extend(
                summarize_requested_permissions(permissions)
                    .into_iter()
                    .map(|permission| format!("Permission: {permission}")),
            );
            lines
        }
        ApprovalSubject::Plan { plan_file_path, .. } => plan_file_path
            .as_deref()
            .filter(|path| !path.is_empty())
            .map(|path| vec![format!("Plan file: {path}")])
            .unwrap_or_default(),
        ApprovalSubject::ToolUse {
            tool, presentation, ..
        } => tool_use_detail_lines(&describe_tool_use(
            payload.reason.as_deref(),
            tool,
            presentation,
        )),
        ApprovalSubject::Unknown(_) => Vec::new(),
    }
}

pub fn approval_outcome(decision: ApprovalDecision) -> &'static str {
    match decision {
        ApprovalDecision::AllowForSession => "approved for this session",
        ApprovalDecision::Deny => "denied",
        ApprovalDecision::AllowOnce | ApprovalDecision::Unknown => "approved",
    }
}

fn to_granted(permissions: &RequestedPermissionProfile) -> PermissionProfile {
    PermissionProfile {
        network: (permissions
            .network
            .as_ref()
            .and_then(|network| network.enabled)
            == Some(true))
        .then_some(NetworkPermissions {
            enabled: Some(true),
        }),
        file_system: permissions.file_system.clone(),
    }
}

fn granted_permissions(
    interaction: &PendingInteraction,
    decision: ApprovalDecision,
) -> Option<PermissionProfile> {
    let InteractionPayload::Approval(payload) = &interaction.payload else {
        return None;
    };
    match &payload.subject {
        ApprovalSubject::PermissionGrant { permissions, .. } => Some(to_granted(permissions)),
        ApprovalSubject::Command { session_grant, .. }
        | ApprovalSubject::FileChange { session_grant, .. }
            if decision == ApprovalDecision::AllowForSession =>
        {
            session_grant.clone()
        }
        _ => None,
    }
}

pub fn build_approval_resolution(
    interaction: &PendingInteraction,
    decision: ApprovalDecision,
) -> InteractionResolution {
    let granted_permissions = granted_permissions(interaction, decision);
    InteractionResolution::Approval(match decision {
        ApprovalDecision::Deny => ApprovalResolution::Deny,
        ApprovalDecision::AllowForSession => ApprovalResolution::AllowForSession {
            granted_permissions,
        },
        ApprovalDecision::AllowOnce | ApprovalDecision::Unknown => ApprovalResolution::AllowOnce {
            granted_permissions,
        },
    })
}

pub fn ordered_decisions(available: &[ApprovalDecision]) -> Vec<ApprovalDecision> {
    APPROVAL_DECISION_ORDER
        .into_iter()
        .filter(|decision| available.contains(decision))
        .collect()
}

pub fn decision_label(decision: ApprovalDecision, plan: bool) -> &'static str {
    if plan {
        return if decision == ApprovalDecision::Deny {
            "Keep planning"
        } else {
            "Approve plan"
        };
    }
    match decision {
        ApprovalDecision::AllowForSession => "Allow for session",
        ApprovalDecision::Deny => "Deny",
        ApprovalDecision::AllowOnce | ApprovalDecision::Unknown => "Allow once",
    }
}

pub fn interaction_summary(interaction: &PendingInteraction) -> String {
    match &interaction.payload {
        InteractionPayload::UserQuestion(payload) => payload
            .questions
            .first()
            .map(|question| question.prompt.clone())
            .unwrap_or_else(|| "User answer requested".to_owned()),
        InteractionPayload::Plugin(payload) => payload.title.clone(),
        InteractionPayload::Extension(payload) => payload.title.clone(),
        InteractionPayload::Unknown(_) => "Pending request".to_owned(),
        InteractionPayload::Approval(payload) => {
            if let Some(reason) = payload.reason.as_deref() {
                return reason.to_owned();
            }
            match &payload.subject {
                ApprovalSubject::Command { command, .. } => command.clone(),
                ApprovalSubject::FileChange { .. } => "File changes pending approval".to_owned(),
                ApprovalSubject::Plan { .. } => "Plan ready for review".to_owned(),
                ApprovalSubject::ToolUse {
                    tool, presentation, ..
                } => describe_tool_use(None, tool, presentation).title,
                ApprovalSubject::PermissionGrant { tool_name, .. } => tool_name
                    .clone()
                    .unwrap_or_else(|| "Permission request".to_owned()),
                ApprovalSubject::Unknown(_) => "Approval requested".to_owned(),
            }
        }
    }
}

pub fn option_label(question: &Question, value: &str) -> String {
    question
        .options
        .iter()
        .find(|option| option.value == value)
        .map_or_else(|| value.to_owned(), |option| option.label.clone())
}

pub fn unwrap_backticked_command(command: &str) -> String {
    let trimmed = command.trim();
    if trimmed.len() > 2
        && trimmed.starts_with('`')
        && trimmed.ends_with('`')
        && !trimmed[1..trimmed.len() - 1].contains('`')
    {
        trimmed[1..trimmed.len() - 1].to_owned()
    } else {
        command.to_owned()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnswerChannel {
    Resolve,
    Respond,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelAction {
    StopTurn,
    CancelRequest,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RequestKind {
    Approval(ApprovalPayload),
    PlanReview {
        approval: ApprovalPayload,
        plan: String,
        plan_file_path: Option<String>,
    },
    Question {
        questions: Vec<Question>,
        channel: AnswerChannel,
        cancel: CancelAction,
    },
    Secret {
        payload: SecretRequestPayload,
        cancel: CancelAction,
    },
    Plugin {
        plugin_id: String,
        name: String,
        title: String,
        data: JsonValue,
        cancel: CancelAction,
        invalid: bool,
    },
}

impl RequestKind {
    pub fn label(&self) -> String {
        match self {
            Self::Approval(_) => "Approval needed".to_owned(),
            Self::PlanReview { .. } => "Plan review".to_owned(),
            Self::Question { questions, .. } if questions.len() == 1 => "Question".to_owned(),
            Self::Question { questions, .. } => format!("{} questions", questions.len()),
            Self::Secret { .. } => "Secret request".to_owned(),
            Self::Plugin { title, .. } => title.clone(),
        }
    }
}

pub fn classify(interaction: &PendingInteraction) -> RequestKind {
    match &interaction.payload {
        InteractionPayload::Approval(payload) => match &payload.subject {
            ApprovalSubject::Plan {
                plan,
                plan_file_path,
                ..
            } => RequestKind::PlanReview {
                approval: payload.clone(),
                plan: plan.clone(),
                plan_file_path: plan_file_path.clone(),
            },
            _ => RequestKind::Approval(payload.clone()),
        },
        InteractionPayload::UserQuestion(payload) => RequestKind::Question {
            questions: normalize_questions(&payload.questions),
            channel: AnswerChannel::Resolve,
            cancel: CancelAction::StopTurn,
        },
        InteractionPayload::Plugin(payload) => {
            let (plugin_id, name) = match &interaction.origin {
                Some(InteractionOrigin::Plugin {
                    plugin_id,
                    renderer_id,
                }) => (plugin_id.clone(), renderer_id.clone()),
                _ => (String::new(), String::new()),
            };
            plugin_request(
                interaction,
                plugin_id,
                name,
                payload.title.clone(),
                payload.data.clone(),
                CancelAction::CancelRequest,
            )
        }
        InteractionPayload::Extension(payload) => {
            let (plugin_id, name) = payload
                .kind
                .split_once('/')
                .map(|(plugin, name)| (plugin.to_owned(), name.to_owned()))
                .unwrap_or_default();
            let cancel = if matches!(interaction.origin, Some(InteractionOrigin::Plugin { .. })) {
                CancelAction::CancelRequest
            } else {
                CancelAction::StopTurn
            };
            plugin_request(
                interaction,
                plugin_id,
                name,
                payload.title.clone(),
                payload.data.clone(),
                cancel,
            )
        }
        InteractionPayload::Unknown(value) => RequestKind::Plugin {
            plugin_id: String::new(),
            name: value
                .get("kind")
                .and_then(JsonValue::as_str)
                .unwrap_or("unknown")
                .to_owned(),
            title: "Unsupported request".to_owned(),
            data: value.clone(),
            cancel: CancelAction::StopTurn,
            invalid: true,
        },
    }
}

fn plugin_request(
    interaction: &PendingInteraction,
    plugin_id: String,
    name: String,
    title: String,
    data: JsonValue,
    cancel: CancelAction,
) -> RequestKind {
    if name == ASK_USER_QUESTION_RENDERER_ID {
        if let Some(payload) = interaction.plugin_data::<AskUserQuestionPluginPayload>()
            && valid_questions(&payload.questions)
        {
            return RequestKind::Question {
                questions: normalize_questions(&payload.questions),
                channel: AnswerChannel::Respond,
                cancel,
            };
        }
    } else if name == SECRET_REQUEST_RENDERER_ID
        && let Some(payload) = interaction.plugin_data::<SecretRequestPayload>()
        && valid_secret_payload(&payload)
    {
        return RequestKind::Secret { payload, cancel };
    }
    let invalid = name == ASK_USER_QUESTION_RENDERER_ID || name == SECRET_REQUEST_RENDERER_ID;
    RequestKind::Plugin {
        plugin_id,
        name,
        title,
        data,
        cancel,
        invalid,
    }
}

fn valid_questions(questions: &[UserQuestion]) -> bool {
    !questions.is_empty()
        && questions.len() <= USER_QUESTION_MAX_QUESTIONS
        && questions.iter().all(|question| {
            !question.id.is_empty()
                && !question.prompt.is_empty()
                && question
                    .options
                    .as_ref()
                    .is_none_or(|options| options.len() <= USER_QUESTION_MAX_OPTIONS)
        })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Question {
    pub id: String,
    pub prompt: String,
    pub short_label: String,
    pub multi_select: bool,
    pub allow_free_text: bool,
    pub options: Vec<UserQuestionOption>,
}

impl Question {
    pub fn has_other_row(&self) -> bool {
        self.allow_free_text && !self.options.is_empty()
    }

    pub fn row_count(&self) -> usize {
        self.options.len() + usize::from(self.has_other_row())
    }
}

pub fn normalize_questions(questions: &[UserQuestion]) -> Vec<Question> {
    questions
        .iter()
        .take(USER_QUESTION_MAX_QUESTIONS)
        .enumerate()
        .map(|(index, question)| Question {
            id: question.id.clone(),
            prompt: question.prompt.clone(),
            short_label: question
                .short_label
                .clone()
                .filter(|label| !label.trim().is_empty())
                .unwrap_or_else(|| format!("Question {}", index + 1)),
            multi_select: question.multi_select,
            allow_free_text: question.allow_free_text,
            options: question
                .options
                .iter()
                .flatten()
                .take(USER_QUESTION_MAX_OPTIONS)
                .cloned()
                .collect(),
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AnswerState {
    pub selected: Vec<String>,
    pub other_selected: bool,
    pub other_text: String,
}

impl AnswerState {
    pub fn initial(question: &Question) -> Self {
        Self {
            selected: Vec::new(),
            other_selected: question.options.is_empty(),
            other_text: String::new(),
        }
    }
}

fn valid_selected(question: &Question, selected: &[String]) -> Vec<String> {
    selected
        .iter()
        .filter(|value| {
            question
                .options
                .iter()
                .any(|option| &option.value == *value)
        })
        .take(USER_QUESTION_MAX_SELECTED)
        .cloned()
        .collect()
}

pub fn is_question_answered(question: &Question, state: &AnswerState) -> bool {
    if !valid_selected(question, &state.selected).is_empty() {
        return true;
    }
    state.other_selected && !state.other_text.trim().is_empty()
}

pub fn build_question_answer(question: &Question, state: &AnswerState) -> UserAnswer {
    let free_text = state.other_text.trim();
    let free_text = (state.other_selected && !free_text.is_empty()).then(|| free_text.to_owned());
    if question.multi_select {
        return UserAnswer {
            selected: valid_selected(question, &state.selected),
            free_text,
        };
    }
    if state.other_selected {
        return UserAnswer {
            selected: Vec::new(),
            free_text,
        };
    }
    UserAnswer {
        selected: valid_selected(question, &state.selected),
        free_text: None,
    }
}

pub fn build_question_answers(
    questions: &[Question],
    states: &[AnswerState],
) -> BTreeMap<String, UserAnswer> {
    questions
        .iter()
        .enumerate()
        .map(|(index, question)| {
            let state = states
                .get(index)
                .cloned()
                .unwrap_or_else(|| AnswerState::initial(question));
            (question.id.clone(), build_question_answer(question, &state))
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShortcutChoice {
    Option(String),
    Other,
}

pub fn shortcut_choice(question: &Question, index: usize) -> Option<ShortcutChoice> {
    if let Some(option) = question.options.get(index) {
        return Some(ShortcutChoice::Option(option.value.clone()));
    }
    (index == question.options.len() && question.has_other_row()).then_some(ShortcutChoice::Other)
}

pub fn toggle_option(question: &Question, state: &mut AnswerState, value: &str) {
    if question.multi_select {
        if let Some(position) = state.selected.iter().position(|selected| selected == value) {
            state.selected.remove(position);
        } else if state.selected.len() < USER_QUESTION_MAX_SELECTED {
            state.selected.push(value.to_owned());
        }
        return;
    }
    state.selected = vec![value.to_owned()];
    state.other_selected = false;
}

pub fn select_other(question: &Question, state: &mut AnswerState) {
    if question.multi_select {
        state.other_selected = !state.other_selected;
    } else {
        state.selected.clear();
        state.other_selected = true;
    }
}

pub fn clamp_free_text(text: &str) -> String {
    let mut out = String::new();
    let mut units = 0;
    for ch in text.chars() {
        units += ch.len_utf16();
        if units > USER_QUESTION_MAX_FREE_TEXT_LENGTH {
            break;
        }
        out.push(ch);
    }
    out
}

fn valid_secret_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    bytes
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == b'_')
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

pub fn valid_secret_payload(payload: &SecretRequestPayload) -> bool {
    payload.destination.kind == "dotenv"
        && !payload.destination.path.is_empty()
        && !payload.fields.is_empty()
        && payload
            .fields
            .iter()
            .all(|field| valid_secret_name(&field.name))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretValueError {
    Invalid,
    NotRepresentable,
}

impl SecretValueError {
    pub fn message(self) -> &'static str {
        match self {
            Self::Invalid => {
                "Every secret must be a non-empty single-line value no larger than 16 KiB."
            }
            Self::NotRepresentable => {
                "One value cannot be represented safely in a dotenv assignment."
            }
        }
    }
}

fn dotenv_representable(value: &str) -> bool {
    if !value.contains('\'') {
        return true;
    }
    if !value.contains('"') && !value.contains("\\n") && !value.contains("\\r") {
        return true;
    }
    value == value.trim() && !value.contains('#')
}

pub fn validate_secret_values<'a>(
    values: impl IntoIterator<Item = &'a str>,
) -> Result<(), SecretValueError> {
    let values: Vec<&str> = values.into_iter().collect();
    let invalid = |value: &str| {
        value.is_empty()
            || utf16_len(value) > SECRET_VALUE_MAX_LENGTH
            || value.contains(['\n', '\r', '\0'])
    };
    if values.iter().any(|value| invalid(value)) {
        return Err(SecretValueError::Invalid);
    }
    if values.iter().any(|value| !dotenv_representable(value)) {
        return Err(SecretValueError::NotRepresentable);
    }
    Ok(())
}

pub fn active_interactions(interactions: &[PendingInteraction]) -> Vec<&PendingInteraction> {
    let mut active: Vec<&PendingInteraction> = interactions
        .iter()
        .filter(|interaction| {
            matches!(
                interaction.status,
                PendingInteractionStatus::Pending | PendingInteractionStatus::Resolving
            )
        })
        .collect();
    active.sort_by(|left, right| {
        left.created_at
            .cmp(&right.created_at)
            .then_with(|| left.id.cmp(&right.id))
    });
    active
}

pub fn thread_web_url(server_url: &str, thread_id: &str) -> String {
    format!("{}/threads/{}", server_url.trim_end_matches('/'), thread_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn interaction(payload: JsonValue) -> PendingInteraction {
        serde_json::from_value(json!({
            "id": "int_1",
            "threadId": "thr_1",
            "status": "pending",
            "createdAt": 1,
            "payload": payload,
        }))
        .unwrap()
    }

    fn question(options: usize, multi: bool, free: bool) -> Question {
        Question {
            id: "q1".into(),
            prompt: "Pick".into(),
            short_label: "Pick".into(),
            multi_select: multi,
            allow_free_text: free,
            options: (0..options)
                .map(|index| UserQuestionOption {
                    value: format!("v{index}"),
                    label: format!("Label {index}"),
                    ..UserQuestionOption::default()
                })
                .collect(),
        }
    }

    #[test]
    fn command_detail_lines_cover_cwd_actions_and_session_grant() {
        let item = interaction(json!({
            "kind": "approval",
            "reason": null,
            "availableDecisions": ["allow_once", "allow_for_session", "deny"],
            "subject": {
                "kind": "command",
                "itemId": "item_1",
                "command": "cargo test",
                "cwd": "/repo",
                "actions": [
                    {"type": "read", "command": "cat a", "name": "a", "path": "src/a.rs"},
                    {"type": "listFiles", "command": "ls"},
                    {"type": "search", "command": "rg x", "query": "x", "path": "src"},
                    {"type": "search", "command": "rg"},
                    {"type": "unknown", "command": "make"}
                ],
                "sessionGrant": {"network": {"enabled": true}, "fileSystem": {"read": [], "write": ["/repo", "/tmp"]}}
            }
        }));
        assert_eq!(
            subject_detail_lines(&item),
            vec![
                "Command: cargo test",
                "Cwd: /repo",
                "Action: Read src/a.rs",
                "Action: List files",
                "Action: Search for x in src",
                "Action: Search files",
                "Action: make",
                "Session grant: Network access, Write 2 paths",
            ]
        );
        assert_eq!(interaction_summary(&item), "cargo test");
    }

    #[test]
    fn permission_grant_lists_requested_permissions_including_macos() {
        let item = interaction(json!({
            "kind": "approval",
            "availableDecisions": ["allow_once", "deny"],
            "subject": {
                "kind": "permission_grant",
                "itemId": "i",
                "toolName": "Bash",
                "permissions": {
                    "network": null,
                    "fileSystem": {"read": ["/a"], "write": []},
                    "macos": {
                        "preferences": "read_only",
                        "automations": {"kind": "bundle_ids", "bundleIds": ["a", "b"]},
                        "launchServices": false,
                        "accessibility": true,
                        "calendar": false,
                        "reminders": false,
                        "contacts": "none"
                    }
                }
            }
        }));
        assert_eq!(
            subject_detail_lines(&item),
            vec![
                "Tool: Bash",
                "Permission: Read 1 path",
                "Permission: macOS accessibility",
                "Permission: macOS preferences (read only)",
                "Permission: macOS automation (2 apps)",
            ]
        );
        assert_eq!(interaction_summary(&item), "Bash");
    }

    #[test]
    fn approval_resolution_grants_only_what_the_decision_allows() {
        let command = interaction(json!({
            "kind": "approval",
            "availableDecisions": ["allow_once", "allow_for_session", "deny"],
            "subject": {
                "kind": "command", "itemId": "i", "command": "ls",
                "sessionGrant": {"network": null, "fileSystem": {"read": ["/r"], "write": []}}
            }
        }));
        let once = serde_json::to_value(build_approval_resolution(
            &command,
            ApprovalDecision::AllowOnce,
        ))
        .unwrap();
        assert_eq!(
            once,
            json!({"decision": "allow_once", "grantedPermissions": null})
        );
        let session = serde_json::to_value(build_approval_resolution(
            &command,
            ApprovalDecision::AllowForSession,
        ))
        .unwrap();
        assert_eq!(
            session,
            json!({
                "decision": "allow_for_session",
                "grantedPermissions": {"network": null, "fileSystem": {"read": ["/r"], "write": []}}
            })
        );
        let deny =
            serde_json::to_value(build_approval_resolution(&command, ApprovalDecision::Deny))
                .unwrap();
        assert_eq!(deny, json!({"decision": "deny"}));

        let grant = interaction(json!({
            "kind": "approval",
            "availableDecisions": ["allow_once", "deny"],
            "subject": {
                "kind": "permission_grant", "itemId": "i",
                "permissions": {
                    "network": {"enabled": false},
                    "fileSystem": null,
                    "macos": {"preferences": "none", "automations": "all", "launchServices": true,
                              "accessibility": false, "calendar": false, "reminders": false, "contacts": "none"}
                }
            }
        }));
        let once = serde_json::to_value(build_approval_resolution(
            &grant,
            ApprovalDecision::AllowOnce,
        ))
        .unwrap();
        assert_eq!(
            once,
            json!({"decision": "allow_once", "grantedPermissions": {"network": null, "fileSystem": null}})
        );
    }

    #[test]
    fn tool_use_and_plan_titles() {
        let tool = interaction(json!({
            "kind": "approval",
            "availableDecisions": ["allow_once", "deny"],
            "subject": {
                "kind": "tool_use", "itemId": "i", "tool": "web_fetch",
                "presentation": {
                    "label": {"pending": "Fetching page", "completed": "Fetched page"},
                    "icon": {"glyph": "Globe"},
                    "title": "https://example.test",
                    "detail": "  "
                }
            }
        }));
        assert_eq!(interaction_summary(&tool), "Fetching page");
        assert_eq!(
            subject_detail_lines(&tool),
            vec!["Tool: web_fetch", "https://example.test"]
        );
        let plan = interaction(json!({
            "kind": "approval",
            "availableDecisions": ["allow_once", "deny"],
            "subject": {"kind": "plan", "itemId": "i", "plan": "# Plan", "planFilePath": "PLAN.md"}
        }));
        assert_eq!(interaction_summary(&plan), "Plan ready for review");
        assert!(matches!(classify(&plan), RequestKind::PlanReview { .. }));
        assert_eq!(subject_detail_lines(&plan), vec!["Plan file: PLAN.md"]);
    }

    #[test]
    fn decisions_are_ordered_deny_first_and_unknown_dropped() {
        assert_eq!(
            ordered_decisions(&[
                ApprovalDecision::AllowOnce,
                ApprovalDecision::Unknown,
                ApprovalDecision::Deny
            ]),
            vec![ApprovalDecision::Deny, ApprovalDecision::AllowOnce]
        );
        assert_eq!(
            decision_label(ApprovalDecision::AllowOnce, true),
            "Approve plan"
        );
        assert_eq!(
            decision_label(ApprovalDecision::Deny, true),
            "Keep planning"
        );
    }

    #[test]
    fn initial_state_selects_other_only_for_free_text_questions() {
        assert!(AnswerState::initial(&question(0, false, true)).other_selected);
        assert!(!AnswerState::initial(&question(2, false, true)).other_selected);
    }

    #[test]
    fn single_select_replaces_choice_and_clears_other() {
        let q = question(3, false, true);
        let mut state = AnswerState::initial(&q);
        select_other(&q, &mut state);
        state.other_text = "custom".into();
        toggle_option(&q, &mut state, "v1");
        toggle_option(&q, &mut state, "v2");
        assert_eq!(state.selected, vec!["v2"]);
        assert!(!state.other_selected);
        assert_eq!(
            build_question_answer(&q, &state),
            UserAnswer {
                selected: vec!["v2".into()],
                free_text: None
            }
        );
        select_other(&q, &mut state);
        assert!(state.selected.is_empty());
        assert_eq!(
            build_question_answer(&q, &state),
            UserAnswer {
                selected: vec![],
                free_text: Some("custom".into())
            }
        );
    }

    #[test]
    fn multi_select_keeps_choices_and_free_text_together() {
        let q = question(3, true, true);
        let mut state = AnswerState::initial(&q);
        toggle_option(&q, &mut state, "v0");
        toggle_option(&q, &mut state, "v2");
        toggle_option(&q, &mut state, "v0");
        select_other(&q, &mut state);
        state.other_text = "  more  ".into();
        assert_eq!(
            build_question_answer(&q, &state),
            UserAnswer {
                selected: vec!["v2".into()],
                free_text: Some("more".into())
            }
        );
        select_other(&q, &mut state);
        assert_eq!(build_question_answer(&q, &state).free_text, None);
    }

    #[test]
    fn answered_requires_valid_choice_or_non_blank_text() {
        let q = question(2, false, true);
        let mut state = AnswerState::initial(&q);
        assert!(!is_question_answered(&q, &state));
        state.selected = vec!["stale".into()];
        assert!(!is_question_answered(&q, &state));
        state.selected.clear();
        state.other_selected = true;
        state.other_text = "   ".into();
        assert!(!is_question_answered(&q, &state));
        state.other_text = "x".into();
        assert!(is_question_answered(&q, &state));
    }

    #[test]
    fn shortcuts_map_to_options_then_other() {
        let q = question(2, false, true);
        assert_eq!(
            shortcut_choice(&q, 1),
            Some(ShortcutChoice::Option("v1".into()))
        );
        assert_eq!(shortcut_choice(&q, 2), Some(ShortcutChoice::Other));
        assert_eq!(shortcut_choice(&q, 3), None);
        assert_eq!(shortcut_choice(&question(2, false, false), 2), None);
        assert_eq!(shortcut_choice(&question(0, false, true), 0), None);
    }

    #[test]
    fn questions_respect_wire_limits() {
        let raw: Vec<UserQuestion> = (0..6)
            .map(|index| UserQuestion {
                id: format!("q{index}"),
                prompt: "p".into(),
                options: Some(
                    (0..6)
                        .map(|option| UserQuestionOption {
                            value: format!("o{option}"),
                            label: format!("O{option}"),
                            ..UserQuestionOption::default()
                        })
                        .collect(),
                ),
                ..UserQuestion::default()
            })
            .collect();
        let questions = normalize_questions(&raw);
        assert_eq!(questions.len(), USER_QUESTION_MAX_QUESTIONS);
        assert!(
            questions
                .iter()
                .all(|q| q.options.len() == USER_QUESTION_MAX_OPTIONS)
        );
        assert_eq!(questions[1].short_label, "Question 2");
        let long = "é".repeat(5000);
        assert_eq!(
            utf16_len(&clamp_free_text(&long)),
            USER_QUESTION_MAX_FREE_TEXT_LENGTH
        );
        let emoji = "😀".repeat(3000);
        assert!(utf16_len(&clamp_free_text(&emoji)) <= USER_QUESTION_MAX_FREE_TEXT_LENGTH);
    }

    #[test]
    fn plugin_requests_classify_by_renderer() {
        let ask: PendingInteraction = serde_json::from_value(json!({
            "id": "i", "threadId": "t", "status": "pending", "createdAt": 1,
            "origin": {"kind": "plugin", "pluginId": "ask-user-question", "rendererId": "ask-user-question"},
            "payload": {"kind": "plugin", "title": "Ask", "data": {"questions": [
                {"id": "a", "prompt": "Which?", "shortLabel": "Which", "multiSelect": false, "allowFreeText": true,
                 "options": [{"value": "x", "label": "X"}]}
            ]}}
        }))
        .unwrap();
        assert!(matches!(
            classify(&ask),
            RequestKind::Question {
                channel: AnswerChannel::Respond,
                cancel: CancelAction::CancelRequest,
                ..
            }
        ));
        let secret: PendingInteraction = serde_json::from_value(json!({
            "id": "i", "threadId": "t", "status": "pending", "createdAt": 1,
            "origin": {"kind": "plugin", "pluginId": "secrets", "rendererId": "secret-request"},
            "payload": {"kind": "plugin", "title": "Secrets", "data": {
                "purpose": null, "destination": {"kind": "dotenv", "path": ".env"},
                "fields": [{"name": "1BAD", "description": null}]
            }}
        }))
        .unwrap();
        assert!(matches!(
            classify(&secret),
            RequestKind::Plugin { invalid: true, .. }
        ));
        let extension: PendingInteraction = serde_json::from_value(json!({
            "id": "i", "threadId": "t", "status": "pending", "createdAt": 1,
            "origin": {"kind": "provider", "providerId": "p", "providerRequestId": "r"},
            "payload": {"kind": "acme/pick", "title": "Pick a color", "data": {"colors": ["red"]}}
        }))
        .unwrap();
        match classify(&extension) {
            RequestKind::Plugin {
                plugin_id,
                name,
                cancel,
                invalid,
                ..
            } => {
                assert_eq!((plugin_id.as_str(), name.as_str()), ("acme", "pick"));
                assert_eq!(cancel, CancelAction::StopTurn);
                assert!(!invalid);
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn secret_values_follow_plugin_validation() {
        assert_eq!(validate_secret_values(["abc", "with space"]), Ok(()));
        assert_eq!(validate_secret_values([""]), Err(SecretValueError::Invalid));
        assert_eq!(
            validate_secret_values(["a\nb"]),
            Err(SecretValueError::Invalid)
        );
        assert_eq!(validate_secret_values(["it's"]), Ok(()));
        assert_eq!(validate_secret_values(["it's \"x\""]), Ok(()));
        assert_eq!(
            validate_secret_values(["it's \"x\" #"]),
            Err(SecretValueError::NotRepresentable)
        );
        assert_eq!(
            validate_secret_values([" it's \"x\""]),
            Err(SecretValueError::NotRepresentable)
        );
        let big = "a".repeat(SECRET_VALUE_MAX_LENGTH + 1);
        assert_eq!(
            validate_secret_values([big.as_str()]),
            Err(SecretValueError::Invalid)
        );
    }

    #[test]
    fn active_interactions_skip_settled_and_sort_by_creation() {
        let mut first = interaction(json!({"kind": "user_question", "questions": []}));
        first.id = "b".into();
        first.created_at = 5;
        let mut second = first.clone();
        second.id = "a".into();
        second.created_at = 2;
        let mut done = first.clone();
        done.id = "c".into();
        done.status = PendingInteractionStatus::Resolved;
        let all = [first, second, done];
        let ids: Vec<&str> = active_interactions(&all)
            .iter()
            .map(|item| item.id.as_str())
            .collect();
        assert_eq!(ids, vec!["a", "b"]);
        assert_eq!(
            thread_web_url("http://h:1/", "thr_1"),
            "http://h:1/threads/thr_1"
        );
    }
}
