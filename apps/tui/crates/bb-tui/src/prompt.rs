use bb_wire::prompt::{
    ExistingThreadExecutionInputSources, LocalFilePromptInput, LocalImagePromptInput,
    PermissionMode, PromptInput, PromptInputVisibility, PromptTextMention, ReasoningLevel,
};
use bb_wire::queue::{
    CreateQueuedMessageRequest, QueuedMessageWaitingOn, SendMessageMode, SendMessageRequest,
    ThreadQueuedMessage,
};
use bb_wire::thread::ThreadDisplayStatus;
use serde_json::{Map, Value, json};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockedReason {
    LoadingExecutionOptions,
    PendingInteraction,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubmitMode {
    Ready,
    Queue,
    QueueWhileStopping,
    Blocked(BlockedReason),
}

impl SubmitMode {
    pub fn can_stop(self) -> bool {
        self == Self::Queue
    }
}

pub fn should_queue_follow_up(status: ThreadDisplayStatus) -> bool {
    matches!(
        status,
        ThreadDisplayStatus::Active
            | ThreadDisplayStatus::Provisioning
            | ThreadDisplayStatus::Starting
            | ThreadDisplayStatus::Stopping
            | ThreadDisplayStatus::WaitingForHost
    )
}

pub fn should_steer_when_ready(status: ThreadDisplayStatus) -> bool {
    matches!(
        status,
        ThreadDisplayStatus::Provisioning | ThreadDisplayStatus::Starting
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubmitModeInputs {
    pub has_pending_interaction: bool,
    pub defaults_loading: bool,
    pub stop_requested: bool,
    pub status: ThreadDisplayStatus,
}

pub fn build_follow_up_submit_mode(inputs: SubmitModeInputs) -> SubmitMode {
    if inputs.stop_requested {
        return SubmitMode::QueueWhileStopping;
    }
    if inputs.has_pending_interaction {
        return SubmitMode::Blocked(BlockedReason::PendingInteraction);
    }
    if should_queue_follow_up(inputs.status) {
        return SubmitMode::Queue;
    }
    if inputs.defaults_loading {
        return SubmitMode::Blocked(BlockedReason::LoadingExecutionOptions);
    }
    SubmitMode::Ready
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShortcutInputs {
    pub has_draft_input: bool,
    pub submitting: bool,
    pub queue_mutation_pending: bool,
    pub queued_count: usize,
    pub status: ThreadDisplayStatus,
    pub mode: SubmitMode,
}

pub fn can_submit_follow_up_shortcut(inputs: ShortcutInputs) -> bool {
    if inputs.submitting || inputs.queue_mutation_pending {
        return false;
    }
    let can_steer_active_work = matches!(
        inputs.status,
        ThreadDisplayStatus::Active
            | ThreadDisplayStatus::Provisioning
            | ThreadDisplayStatus::Starting
    ) && inputs.mode == SubmitMode::Queue;
    if inputs.has_draft_input {
        return can_steer_active_work
            || (inputs.status == ThreadDisplayStatus::WaitingForHost
                && inputs.mode == SubmitMode::Queue);
    }
    inputs.queued_count > 0
        && (can_steer_active_work
            || (inputs.status == ThreadDisplayStatus::Idle && inputs.mode == SubmitMode::Ready))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefaultsState {
    Available,
    Loading,
    Unavailable,
}

pub fn resolve_default_execution_options_state(
    has_concrete: bool,
    has_resolved: bool,
    is_error: bool,
) -> DefaultsState {
    if has_concrete {
        DefaultsState::Available
    } else if has_resolved || is_error {
        DefaultsState::Unavailable
    } else {
        DefaultsState::Loading
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionSelection {
    pub model: String,
    pub permission_mode: PermissionMode,
    pub reasoning_level: ReasoningLevel,
    pub service_tier: Option<String>,
    pub supports_service_tier: bool,
    pub sources: ExistingThreadExecutionInputSources,
}

struct ExecutionFields {
    model: Option<String>,
    service_tier: Option<String>,
    reasoning_level: Option<ReasoningLevel>,
    permission_mode: Option<PermissionMode>,
    sources: Option<ExistingThreadExecutionInputSources>,
}

fn execution_fields(execution: Option<&ExecutionSelection>) -> ExecutionFields {
    match execution {
        None => ExecutionFields {
            model: None,
            service_tier: None,
            reasoning_level: None,
            permission_mode: None,
            sources: None,
        },
        Some(execution) => ExecutionFields {
            model: Some(execution.model.clone()),
            service_tier: execution
                .service_tier
                .clone()
                .filter(|tier| execution.supports_service_tier && !tier.is_empty()),
            reasoning_level: Some(execution.reasoning_level),
            permission_mode: Some(execution.permission_mode),
            sources: Some(execution.sources.clone()),
        },
    }
}

fn send_request(
    input: Vec<PromptInput>,
    execution: Option<&ExecutionSelection>,
    mode: SendMessageMode,
) -> Option<SendMessageRequest> {
    if input.is_empty() {
        return None;
    }
    let fields = execution_fields(execution);
    let mut request = SendMessageRequest::new(input, mode);
    request.model = fields.model;
    request.service_tier = fields.service_tier;
    request.reasoning_level = fields.reasoning_level;
    request.permission_mode = fields.permission_mode;
    request.execution_input_sources = fields.sources;
    Some(request)
}

pub fn build_auto_follow_up_request(
    input: Vec<PromptInput>,
    execution: Option<&ExecutionSelection>,
) -> Option<SendMessageRequest> {
    send_request(input, execution, SendMessageMode::QueueIfActive)
}

pub fn build_steer_follow_up_request(
    input: Vec<PromptInput>,
    execution: Option<&ExecutionSelection>,
) -> Option<SendMessageRequest> {
    send_request(input, execution, SendMessageMode::SteerIfActive)
}

pub fn build_create_queued_follow_up_request(
    input: Vec<PromptInput>,
    execution: Option<&ExecutionSelection>,
) -> Option<CreateQueuedMessageRequest> {
    if input.is_empty() {
        return None;
    }
    let fields = execution_fields(execution);
    Some(CreateQueuedMessageRequest {
        input,
        model: fields.model,
        service_tier: fields.service_tier,
        reasoning_level: fields.reasoning_level,
        permission_mode: fields.permission_mode,
        execution_input_sources: fields.sources,
        sender_thread_id: None,
    })
}

#[derive(Debug, Clone, PartialEq)]
pub enum ShortcutRequest {
    Draft(SendMessageRequest),
    Queued { queued_message_id: String },
}

pub fn build_follow_up_shortcut_request(
    input: Vec<PromptInput>,
    execution: Option<&ExecutionSelection>,
    queued: &[ThreadQueuedMessage],
) -> Option<ShortcutRequest> {
    if let Some(request) = build_steer_follow_up_request(input, execution) {
        return Some(ShortcutRequest::Draft(request));
    }
    queued.first().map(|message| ShortcutRequest::Queued {
        queued_message_id: message.id.clone(),
    })
}

pub fn placeholder(status: ThreadDisplayStatus) -> &'static str {
    match status {
        ThreadDisplayStatus::Provisioning => "Setting up workspace...",
        ThreadDisplayStatus::Starting => "Starting thread...",
        ThreadDisplayStatus::Stopping => "Queue a message to run after the stop",
        ThreadDisplayStatus::WaitingForHost => "Queue a message for when the host reconnects",
        ThreadDisplayStatus::Error => "Retry by sending a follow-up message",
        _ => "Ask for a follow-up",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PermissionModeOption {
    pub value: PermissionMode,
    pub label: &'static str,
    pub description: &'static str,
    pub warning: bool,
}

pub const PERMISSION_MODE_OPTIONS: &[PermissionModeOption] = &[
    PermissionModeOption {
        value: PermissionMode::AcceptEdits,
        label: "Accept Edits",
        description: "Applies edits inside the workspace automatically. Anything beyond the workspace asks you first.",
        warning: false,
    },
    PermissionModeOption {
        value: PermissionMode::Auto,
        label: "Approve for me",
        description: "Same workspace sandbox, with requests reviewed automatically. High-risk actions can still come back to you.",
        warning: false,
    },
    PermissionModeOption {
        value: PermissionMode::Full,
        label: "Full Access",
        description: "No sandbox and no approvals — the agent can run anything on your machine.",
        warning: true,
    },
];

pub fn permission_mode_label(mode: PermissionMode) -> &'static str {
    PERMISSION_MODE_OPTIONS
        .iter()
        .find(|option| option.value == mode)
        .map_or("Unknown", |option| option.label)
}

pub fn permission_mode_rank(mode: PermissionMode) -> Option<usize> {
    PermissionMode::KNOWN
        .iter()
        .position(|known| *known == mode)
}

pub fn resolve_permission_mode_selection(
    raw: PermissionMode,
    modes: &[PermissionMode],
) -> PermissionMode {
    if modes.contains(&raw) {
        raw
    } else if modes.contains(&PermissionMode::Auto) {
        PermissionMode::Auto
    } else if modes.contains(&PermissionMode::Full) {
        PermissionMode::Full
    } else {
        modes.first().copied().unwrap_or(PermissionMode::Auto)
    }
}

fn reasoning_rank(level: ReasoningLevel) -> isize {
    ReasoningLevel::KNOWN
        .iter()
        .position(|known| *known == level)
        .map_or(-1, |index| index as isize)
}

pub fn reconcile_reasoning_level(
    previous: ReasoningLevel,
    supported: &[ReasoningLevel],
) -> Option<ReasoningLevel> {
    let first = *supported.first()?;
    if supported.contains(&previous) {
        return Some(previous);
    }
    let effective = if previous == ReasoningLevel::Ultracode {
        ReasoningLevel::Xhigh
    } else {
        previous
    };
    if supported.contains(&effective) {
        return Some(effective);
    }
    let rank = reasoning_rank(effective);
    let mut best = first;
    let mut best_distance = (reasoning_rank(best) - rank).abs();
    for candidate in &supported[1..] {
        let distance = (reasoning_rank(*candidate) - rank).abs();
        if distance < best_distance
            || (distance == best_distance && reasoning_rank(*candidate) > reasoning_rank(best))
        {
            best = *candidate;
            best_distance = distance;
        }
    }
    Some(best)
}

pub fn reasoning_fallback_label(level: ReasoningLevel) -> &'static str {
    match level {
        ReasoningLevel::None => "None",
        ReasoningLevel::Low => "Low",
        ReasoningLevel::Medium => "Medium",
        ReasoningLevel::High => "High",
        ReasoningLevel::Xhigh => "Extra High",
        ReasoningLevel::Ultracode => "Ultracode",
        ReasoningLevel::Max => "Max",
        ReasoningLevel::Ultra => "Ultra",
        ReasoningLevel::Unknown => "Unknown",
    }
}

pub fn cycle_value<T: PartialEq + Clone>(options: &[T], current: &T, forward: bool) -> Option<T> {
    if options.is_empty() {
        return None;
    }
    let len = options.len();
    let next = match options.iter().position(|option| option == current) {
        Some(index) if forward => &options[(index + 1) % len],
        Some(index) => &options[(index + len - 1) % len],
        None if forward => &options[0],
        None => &options[len - 1],
    };
    (next != current).then(|| next.clone())
}

pub fn fallback_model_label(model: &str) -> String {
    let lower = model.to_ascii_lowercase();
    let stripped = ["anthropic-claude-", "anthropic/claude-", "claude-"]
        .iter()
        .find(|prefix| lower.starts_with(*prefix))
        .map_or(model, |prefix| &model[prefix.len()..]);
    let parts: Vec<&str> = stripped
        .split('-')
        .filter(|part| !part.is_empty())
        .collect();
    let version_start = parts
        .iter()
        .position(|part| part.chars().all(|ch| ch.is_ascii_digit()));
    let (name_parts, version_parts) = match version_start {
        Some(index) => parts.split_at(index),
        None => (parts.as_slice(), &[][..]),
    };
    let name = name_parts
        .iter()
        .map(|part| {
            let mut chars = part.chars();
            chars
                .next()
                .map(|first| first.to_uppercase().chain(chars).collect::<String>())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ");
    let version = match version_parts {
        [] => String::new(),
        [major] => (*major).to_owned(),
        [major, minor, rest @ ..] => std::iter::once(format!("{major}.{minor}"))
            .chain(rest.iter().map(|part| (*part).to_owned()))
            .collect::<Vec<_>>()
            .join(" "),
    };
    let label = [name, version]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if label.is_empty() {
        model.to_owned()
    } else {
        label
    }
}

pub fn cycle_reasoning_value(
    options: &[ReasoningLevel],
    current: ReasoningLevel,
    forward: bool,
) -> Option<ReasoningLevel> {
    let ordered: Vec<ReasoningLevel> = ReasoningLevel::KNOWN
        .iter()
        .copied()
        .filter(|level| options.contains(level))
        .collect();
    let rank = reasoning_rank(current);
    let candidate = if forward {
        ordered
            .iter()
            .copied()
            .find(|level| reasoning_rank(*level) > rank)
            .or_else(|| ordered.first().copied())
    } else {
        ordered
            .iter()
            .rev()
            .copied()
            .find(|level| reasoning_rank(*level) < rank)
            .or_else(|| ordered.last().copied())
    }?;
    (candidate != current).then_some(candidate)
}

pub fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

pub fn utf16_to_byte(text: &str, offset: usize) -> Option<usize> {
    let mut units = 0;
    for (index, ch) in text.char_indices() {
        if units == offset {
            return Some(index);
        }
        if units > offset {
            return None;
        }
        units += ch.len_utf16();
    }
    (units == offset).then_some(text.len())
}

pub fn utf16_slice(text: &str, start: usize, end: usize) -> Option<&str> {
    let start = utf16_to_byte(text, start)?;
    let end = utf16_to_byte(text, end)?;
    text.get(start..end)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttachmentKind {
    LocalImage,
    LocalFile,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttachmentOwner {
    None,
    Project(String),
    Host(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftAttachment {
    pub kind: AttachmentKind,
    pub path: String,
    pub name: String,
    pub mime_type: Option<String>,
    pub size_bytes: Option<u64>,
    pub owner: AttachmentOwner,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct PromptDraft {
    pub text: String,
    pub mentions: Vec<PromptTextMention>,
    pub attachments: Vec<DraftAttachment>,
}

fn file_name_from_path(path: &str) -> String {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return "Attachment".into();
    }
    match trimmed.rsplit('/').next() {
        Some(last) if !last.is_empty() => last.to_owned(),
        _ => trimmed.to_owned(),
    }
}

fn owner_of(source_project_id: &Option<String>, host_id: &Option<String>) -> AttachmentOwner {
    match (host_id, source_project_id) {
        (Some(host), _) => AttachmentOwner::Host(host.clone()),
        (None, Some(project)) => AttachmentOwner::Project(project.clone()),
        (None, None) => AttachmentOwner::None,
    }
}

fn owner_fields(owner: &AttachmentOwner) -> (Option<String>, Option<String>) {
    match owner {
        AttachmentOwner::None => (None, None),
        AttachmentOwner::Project(project) => (Some(project.clone()), None),
        AttachmentOwner::Host(host) => (None, Some(host.clone())),
    }
}

pub fn normalize_mentions(
    mentions: Vec<PromptTextMention>,
    text_len: usize,
) -> Vec<PromptTextMention> {
    let mut mentions: Vec<PromptTextMention> = mentions
        .into_iter()
        .filter(|mention| mention.end > mention.start && mention.end as usize <= text_len)
        .collect();
    mentions.sort_by_key(|mention| (mention.start, mention.end));
    mentions
}

impl PromptDraft {
    pub fn from_text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            ..Self::default()
        }
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty() && self.mentions.is_empty() && self.attachments.is_empty()
    }

    pub fn to_input(&self) -> Vec<PromptInput> {
        let mut input = Vec::new();
        let trimmed_start = self.text.trim_start();
        let lead = utf16_len(&self.text) - utf16_len(trimmed_start);
        let text = trimmed_start.trim_end();
        let text_len = utf16_len(text);
        let tail = lead + text_len;
        if !text.is_empty() {
            let mentions = self
                .mentions
                .iter()
                .filter_map(|mention| {
                    let start = (mention.start as usize).max(lead);
                    let end = (mention.end as usize).min(tail);
                    (start < end).then(|| PromptTextMention {
                        start: (start - lead) as u32,
                        end: (end - lead) as u32,
                        resource: mention.resource.clone(),
                    })
                })
                .collect();
            input.push(PromptInput::text_with_mentions(
                text,
                normalize_mentions(mentions, text_len),
            ));
        }
        for attachment in &self.attachments {
            let (source_project_id, host_id) = owner_fields(&attachment.owner);
            input.push(match attachment.kind {
                AttachmentKind::LocalImage => PromptInput::LocalImage(LocalImagePromptInput {
                    path: attachment.path.clone(),
                    source_project_id,
                    host_id,
                    visibility: None,
                }),
                AttachmentKind::LocalFile => PromptInput::LocalFile(LocalFilePromptInput {
                    path: attachment.path.clone(),
                    source_project_id,
                    host_id,
                    name: Some(attachment.name.clone()),
                    size_bytes: attachment.size_bytes.filter(|size| *size > 0),
                    mime_type: attachment.mime_type.clone().filter(|mime| !mime.is_empty()),
                    visibility: None,
                }),
            });
        }
        input
    }

    pub fn from_input(input: &[PromptInput]) -> Self {
        let mut segments: Vec<&str> = Vec::new();
        let mut mentions = Vec::new();
        let mut attachments = Vec::new();
        let mut offset = 0usize;
        for chunk in input {
            match chunk {
                PromptInput::Text(text) => {
                    if text.text.trim().is_empty() {
                        continue;
                    }
                    if !segments.is_empty() {
                        offset += 2;
                    }
                    let len = utf16_len(&text.text);
                    for mention in &text.mentions {
                        if mention.end > mention.start && mention.end as usize <= len {
                            mentions.push(PromptTextMention {
                                start: offset as u32 + mention.start,
                                end: offset as u32 + mention.end,
                                resource: mention.resource.clone(),
                            });
                        }
                    }
                    segments.push(&text.text);
                    offset += len;
                }
                PromptInput::LocalImage(image) => attachments.push(DraftAttachment {
                    kind: AttachmentKind::LocalImage,
                    path: image.path.clone(),
                    name: file_name_from_path(&image.path),
                    mime_type: None,
                    size_bytes: None,
                    owner: owner_of(&image.source_project_id, &image.host_id),
                }),
                PromptInput::LocalFile(file) => attachments.push(DraftAttachment {
                    kind: AttachmentKind::LocalFile,
                    path: file.path.clone(),
                    name: file
                        .name
                        .clone()
                        .unwrap_or_else(|| file_name_from_path(&file.path)),
                    mime_type: file.mime_type.clone().filter(|mime| !mime.is_empty()),
                    size_bytes: file.size_bytes,
                    owner: owner_of(&file.source_project_id, &file.host_id),
                }),
                PromptInput::Image(_) | PromptInput::Unknown(_) => {}
            }
        }
        Self {
            text: segments.join("\n\n"),
            mentions,
            attachments,
        }
    }

    pub fn with_attachments(self, source: &PromptDraft) -> Self {
        Self {
            attachments: source.attachments.clone(),
            ..self
        }
    }

    pub fn with_text(&self, text: String) -> Self {
        let mentions = relocate_mentions(&self.text, &text, &self.mentions);
        Self {
            text,
            mentions,
            attachments: self.attachments.clone(),
        }
    }

    pub fn serialize(&self) -> Option<String> {
        if self.is_empty() {
            return None;
        }
        let mut object = Map::new();
        object.insert("text".into(), Value::String(self.text.clone()));
        if !self.mentions.is_empty() {
            object.insert(
                "mentions".into(),
                serde_json::to_value(&self.mentions).unwrap_or(Value::Array(Vec::new())),
            );
        }
        let attachments = self
            .attachments
            .iter()
            .map(|attachment| {
                let mut entry = Map::new();
                entry.insert(
                    "type".into(),
                    json!(match attachment.kind {
                        AttachmentKind::LocalImage => "localImage",
                        AttachmentKind::LocalFile => "localFile",
                    }),
                );
                entry.insert("path".into(), json!(attachment.path));
                entry.insert("name".into(), json!(attachment.name));
                if let Some(mime) = &attachment.mime_type {
                    entry.insert("mimeType".into(), json!(mime));
                }
                if let Some(size) = attachment.size_bytes.filter(|size| *size > 0) {
                    entry.insert("sizeBytes".into(), json!(size));
                }
                match &attachment.owner {
                    AttachmentOwner::None => {}
                    AttachmentOwner::Project(project) => {
                        entry.insert("sourceProjectId".into(), json!(project));
                    }
                    AttachmentOwner::Host(host) => {
                        entry.insert("hostId".into(), json!(host));
                    }
                }
                Value::Object(entry)
            })
            .collect();
        object.insert("attachments".into(), Value::Array(attachments));
        Some(Value::Object(object).to_string())
    }

    pub fn parse(raw: Option<&str>) -> Self {
        let Some(Value::Object(object)) = raw.and_then(|raw| serde_json::from_str(raw).ok()) else {
            return Self::default();
        };
        let text = match object.get("text") {
            None | Some(Value::Null) => String::new(),
            Some(Value::String(text)) => text.clone(),
            Some(_) => return Self::default(),
        };
        let mentions = match object.get("mentions") {
            Some(Value::Array(items)) => items
                .iter()
                .filter_map(|item| serde_json::from_value::<PromptTextMention>(item.clone()).ok())
                .filter(|mention| mention.end > mention.start)
                .collect(),
            _ => Vec::new(),
        };
        let attachments = match object.get("attachments") {
            Some(Value::Array(items)) => items.iter().filter_map(parse_attachment).collect(),
            _ => Vec::new(),
        };
        Self {
            text,
            mentions,
            attachments,
        }
    }
}

fn parse_attachment(value: &Value) -> Option<DraftAttachment> {
    let object = value.as_object()?;
    let kind = match object.get("type")?.as_str()? {
        "localImage" => AttachmentKind::LocalImage,
        "localFile" => AttachmentKind::LocalFile,
        _ => return None,
    };
    let text = |key: &str| object.get(key).and_then(Value::as_str).map(str::to_owned);
    let path = text("path").filter(|path| !path.is_empty())?;
    let name = text("name")?;
    let host = text("hostId").filter(|host| !host.is_empty());
    let project = text("sourceProjectId").filter(|project| !project.is_empty());
    let owner = match (host, project) {
        (Some(_), Some(_)) => return None,
        (Some(host), None) => AttachmentOwner::Host(host),
        (None, Some(project)) => AttachmentOwner::Project(project),
        (None, None) => AttachmentOwner::None,
    };
    Some(DraftAttachment {
        kind,
        path,
        name,
        mime_type: text("mimeType"),
        size_bytes: object
            .get("sizeBytes")
            .and_then(Value::as_u64)
            .filter(|size| *size > 0),
        owner,
    })
}

pub fn relocate_mentions(
    old_text: &str,
    new_text: &str,
    mentions: &[PromptTextMention],
) -> Vec<PromptTextMention> {
    if old_text == new_text {
        return mentions.to_vec();
    }
    mentions
        .iter()
        .filter_map(|mention| {
            let covered = utf16_slice(old_text, mention.start as usize, mention.end as usize)?;
            if covered.is_empty() {
                return None;
            }
            if utf16_slice(new_text, mention.start as usize, mention.end as usize) == Some(covered)
            {
                return Some(mention.clone());
            }
            let mut found = new_text.match_indices(covered);
            let (index, _) = found.next()?;
            if found.next().is_some() {
                return None;
            }
            let start = utf16_len(&new_text[..index]);
            Some(PromptTextMention {
                start: start as u32,
                end: (start + utf16_len(covered)) as u32,
                resource: mention.resource.clone(),
            })
        })
        .collect()
}

pub fn history_drafts(entries: &[bb_wire::prompt::PromptHistoryEntry]) -> Vec<PromptDraft> {
    let mut drafts: Vec<PromptDraft> = Vec::new();
    for entry in entries {
        let draft = PromptDraft::from_input(&entry.input);
        if draft.is_empty() {
            continue;
        }
        if drafts
            .last()
            .is_some_and(|previous| previous.serialize() == draft.serialize())
        {
            continue;
        }
        drafts.push(draft);
    }
    drafts
}

const QUEUED_PREVIEW_MAX_WIDTH: usize = 140;
const QUEUED_PREVIEW_ELLIPSIS: &str = "...";

fn visible_input(input: &[PromptInput]) -> Vec<&PromptInput> {
    input
        .iter()
        .filter(|chunk| {
            let visibility = match chunk {
                PromptInput::Text(text) => text.visibility,
                PromptInput::Image(image) => image.visibility,
                PromptInput::LocalImage(image) => image.visibility,
                PromptInput::LocalFile(file) => file.visibility,
                PromptInput::Unknown(_) => None,
            };
            visibility != Some(PromptInputVisibility::AgentOnly)
        })
        .collect()
}

pub fn count_queued_message_attachments(input: &[PromptInput]) -> usize {
    visible_input(input)
        .into_iter()
        .filter(|chunk| {
            matches!(
                chunk,
                PromptInput::LocalImage(_) | PromptInput::LocalFile(_)
            )
        })
        .count()
}

pub fn truncate_to_width(text: &str, width: usize) -> String {
    let mut out = String::new();
    let mut used = 0;
    for grapheme in text.graphemes(true) {
        let grapheme_width = grapheme.width();
        if used + grapheme_width > width {
            break;
        }
        out.push_str(grapheme);
        used += grapheme_width;
    }
    out
}

pub fn format_queued_message_preview(input: &[PromptInput], truncate: bool) -> String {
    let visible = visible_input(input);
    let text = visible
        .iter()
        .filter_map(|chunk| match chunk {
            PromptInput::Text(text) => Some(text.text.trim()),
            _ => None,
        })
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if !collapsed.is_empty() {
        if !truncate || collapsed.width() <= QUEUED_PREVIEW_MAX_WIDTH {
            return collapsed;
        }
        return format!(
            "{}{QUEUED_PREVIEW_ELLIPSIS}",
            truncate_to_width(
                &collapsed,
                QUEUED_PREVIEW_MAX_WIDTH - QUEUED_PREVIEW_ELLIPSIS.len()
            )
        );
    }
    let attachments: Vec<&&PromptInput> = visible
        .iter()
        .filter(|chunk| {
            matches!(
                chunk,
                PromptInput::LocalImage(_) | PromptInput::LocalFile(_)
            )
        })
        .collect();
    match attachments.as_slice() {
        [] => "(empty message)".into(),
        [single] => match single {
            PromptInput::LocalFile(file) if file.name.as_deref().is_some_and(|n| !n.is_empty()) => {
                format!(
                    "Attachment only ({})",
                    file.name.as_deref().unwrap_or_default()
                )
            }
            PromptInput::LocalFile(file) => {
                format!("Attachment only ({})", file_name_from_path(&file.path))
            }
            PromptInput::LocalImage(image) => {
                format!("Attachment only ({})", file_name_from_path(&image.path))
            }
            _ => "Attachment only (1 file)".into(),
        },
        many => format!("Attachment only ({} files)", many.len()),
    }
}

pub fn queued_input_to_draft(input: &[PromptInput]) -> PromptDraft {
    let visible: Vec<PromptInput> = visible_input(input).into_iter().cloned().collect();
    PromptDraft::from_input(&visible)
}

pub fn waiting_on_label(waiting_on: &QueuedMessageWaitingOn) -> String {
    match waiting_on {
        QueuedMessageWaitingOn::Time => "scheduled".into(),
        QueuedMessageWaitingOn::ThreadBusy => "waiting for the current turn".into(),
        QueuedMessageWaitingOn::Stopping => "waiting for stop".into(),
        QueuedMessageWaitingOn::TurnStarting => "turn starting".into(),
        QueuedMessageWaitingOn::Provisioning => "workspace setting up".into(),
        QueuedMessageWaitingOn::HostOffline { host_name } if host_name.is_empty() => {
            "host offline".into()
        }
        QueuedMessageWaitingOn::HostOffline { host_name } => format!("{host_name} offline"),
        QueuedMessageWaitingOn::Interaction => "waiting for your answer".into(),
        QueuedMessageWaitingOn::Plugin { reason, plugin_id } if reason.is_empty() => {
            format!("held by {plugin_id}")
        }
        QueuedMessageWaitingOn::Plugin { reason, .. } => reason.clone(),
        QueuedMessageWaitingOn::Unknown(_) => "waiting".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bb_wire::prompt::{PromptHistoryEntry, PromptMentionResource, TextPromptInput};

    fn mention(start: u32, end: u32) -> PromptTextMention {
        PromptTextMention {
            start,
            end,
            resource: PromptMentionResource::Thread {
                thread_id: "thr_1".into(),
                project_id: None,
                label: "Other".into(),
            },
        }
    }

    fn inputs(status: ThreadDisplayStatus) -> SubmitModeInputs {
        SubmitModeInputs {
            has_pending_interaction: false,
            defaults_loading: false,
            stop_requested: false,
            status,
        }
    }

    #[test]
    fn submit_mode_priorities_follow_client_core() {
        use ThreadDisplayStatus as S;
        assert_eq!(
            build_follow_up_submit_mode(inputs(S::Idle)),
            SubmitMode::Ready
        );
        assert_eq!(
            build_follow_up_submit_mode(inputs(S::Active)),
            SubmitMode::Queue
        );
        assert_eq!(
            build_follow_up_submit_mode(inputs(S::WaitingForHost)),
            SubmitMode::Queue
        );
        let stopping = SubmitModeInputs {
            stop_requested: true,
            has_pending_interaction: true,
            ..inputs(S::Active)
        };
        assert_eq!(
            build_follow_up_submit_mode(stopping),
            SubmitMode::QueueWhileStopping
        );
        let blocked = SubmitModeInputs {
            has_pending_interaction: true,
            ..inputs(S::Active)
        };
        assert_eq!(
            build_follow_up_submit_mode(blocked),
            SubmitMode::Blocked(BlockedReason::PendingInteraction)
        );
        let loading_idle = SubmitModeInputs {
            defaults_loading: true,
            ..inputs(S::Idle)
        };
        assert_eq!(
            build_follow_up_submit_mode(loading_idle),
            SubmitMode::Blocked(BlockedReason::LoadingExecutionOptions)
        );
        let loading_active = SubmitModeInputs {
            defaults_loading: true,
            ..inputs(S::Active)
        };
        assert_eq!(
            build_follow_up_submit_mode(loading_active),
            SubmitMode::Queue
        );
    }

    #[test]
    fn steer_shortcut_gating() {
        use ThreadDisplayStatus as S;
        let base = ShortcutInputs {
            has_draft_input: true,
            submitting: false,
            queue_mutation_pending: false,
            queued_count: 0,
            status: S::Active,
            mode: SubmitMode::Queue,
        };
        assert!(can_submit_follow_up_shortcut(base));
        assert!(!can_submit_follow_up_shortcut(ShortcutInputs {
            submitting: true,
            ..base
        }));
        assert!(can_submit_follow_up_shortcut(ShortcutInputs {
            status: S::WaitingForHost,
            ..base
        }));
        assert!(!can_submit_follow_up_shortcut(ShortcutInputs {
            status: S::Stopping,
            mode: SubmitMode::QueueWhileStopping,
            ..base
        }));
        assert!(!can_submit_follow_up_shortcut(ShortcutInputs {
            status: S::Idle,
            mode: SubmitMode::Ready,
            ..base
        }));
        let empty_idle = ShortcutInputs {
            has_draft_input: false,
            queued_count: 2,
            status: S::Idle,
            mode: SubmitMode::Ready,
            ..base
        };
        assert!(can_submit_follow_up_shortcut(empty_idle));
        assert!(!can_submit_follow_up_shortcut(ShortcutInputs {
            queued_count: 0,
            ..empty_idle
        }));
        assert!(!can_submit_follow_up_shortcut(ShortcutInputs {
            status: S::WaitingForHost,
            mode: SubmitMode::Queue,
            ..empty_idle
        }));
    }

    #[test]
    fn requests_carry_execution_only_when_known() {
        let execution = ExecutionSelection {
            model: "gpt-5".into(),
            permission_mode: PermissionMode::Auto,
            reasoning_level: ReasoningLevel::High,
            service_tier: Some("fast".into()),
            supports_service_tier: false,
            sources: ExistingThreadExecutionInputSources::default(),
        };
        let request =
            build_auto_follow_up_request(vec![PromptInput::text("hi")], Some(&execution)).unwrap();
        let value = serde_json::to_value(&request).unwrap();
        assert_eq!(
            value,
            json!({
                "input": [{"type": "text", "text": "hi", "mentions": []}],
                "model": "gpt-5",
                "reasoningLevel": "high",
                "permissionMode": "auto",
                "executionInputSources": {},
                "mode": "queue-if-active"
            })
        );
        let bare = build_auto_follow_up_request(vec![PromptInput::text("hi")], None).unwrap();
        assert_eq!(
            serde_json::to_value(&bare).unwrap(),
            json!({"input": [{"type": "text", "text": "hi", "mentions": []}], "mode": "queue-if-active"})
        );
        assert!(build_auto_follow_up_request(Vec::new(), None).is_none());
        let with_tier = ExecutionSelection {
            supports_service_tier: true,
            ..execution
        };
        let queued =
            build_create_queued_follow_up_request(vec![PromptInput::text("x")], Some(&with_tier))
                .unwrap();
        assert_eq!(queued.service_tier.as_deref(), Some("fast"));
    }

    #[test]
    fn shortcut_falls_back_to_first_queued_message() {
        let queued = vec![
            ThreadQueuedMessage {
                id: "q1".into(),
                ..ThreadQueuedMessage::default()
            },
            ThreadQueuedMessage {
                id: "q2".into(),
                ..ThreadQueuedMessage::default()
            },
        ];
        assert_eq!(
            build_follow_up_shortcut_request(Vec::new(), None, &queued),
            Some(ShortcutRequest::Queued {
                queued_message_id: "q1".into()
            })
        );
        match build_follow_up_shortcut_request(vec![PromptInput::text("go")], None, &queued) {
            Some(ShortcutRequest::Draft(request)) => {
                assert_eq!(request.mode, SendMessageMode::SteerIfActive)
            }
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(
            build_follow_up_shortcut_request(Vec::new(), None, &[]),
            None
        );
    }

    #[test]
    fn reasoning_reconciliation_matches_domain() {
        use ReasoningLevel as R;
        assert_eq!(
            reconcile_reasoning_level(R::Ultracode, &[R::Low, R::High, R::Xhigh]),
            Some(R::Xhigh)
        );
        assert_eq!(
            reconcile_reasoning_level(R::Medium, &[R::Low, R::High]),
            Some(R::High)
        );
        assert_eq!(
            reconcile_reasoning_level(R::Max, &[R::Low, R::Medium]),
            Some(R::Medium)
        );
        assert_eq!(reconcile_reasoning_level(R::Max, &[]), None);
        assert_eq!(
            cycle_reasoning_value(&[R::High, R::Low, R::Medium], R::High, true),
            Some(R::Low)
        );
        assert_eq!(
            cycle_reasoning_value(&[R::High, R::Low, R::Medium], R::Low, false),
            Some(R::High)
        );
        assert_eq!(cycle_reasoning_value(&[R::Low], R::Low, true), None);
        assert_eq!(cycle_value(&[1, 2, 3], &3, true), Some(1));
        assert_eq!(cycle_value(&[1, 2, 3], &1, false), Some(3));
        assert_eq!(cycle_value(&[1], &1, false), None);
    }

    #[test]
    fn fallback_labels_match_web_card() {
        assert_eq!(
            fallback_model_label("claude-opus-4-1-20250805"),
            "Opus 4.1 20250805"
        );
        assert_eq!(
            fallback_model_label("anthropic/claude-sonnet-4"),
            "Sonnet 4"
        );
        assert_eq!(fallback_model_label("gpt-5"), "Gpt 5");
        assert_eq!(fallback_model_label("---"), "---");
    }

    #[test]
    fn permission_selection_prefers_auto_then_full() {
        use PermissionMode as P;
        assert_eq!(
            resolve_permission_mode_selection(P::Full, &[P::AcceptEdits, P::Auto]),
            P::Auto
        );
        assert_eq!(
            resolve_permission_mode_selection(P::Auto, &[P::AcceptEdits, P::Full]),
            P::Full
        );
        assert_eq!(
            resolve_permission_mode_selection(P::Auto, &[P::AcceptEdits]),
            P::AcceptEdits
        );
        assert_eq!(permission_mode_label(P::Auto), "Approve for me");
    }

    #[test]
    fn draft_to_input_trims_and_shifts_utf16_mentions() {
        let text = "  👋 @Other  ".to_owned();
        let draft = PromptDraft {
            text,
            mentions: vec![mention(5, 11)],
            attachments: Vec::new(),
        };
        let input = draft.to_input();
        let PromptInput::Text(TextPromptInput { text, mentions, .. }) = &input[0] else {
            panic!("text chunk");
        };
        assert_eq!(text, "👋 @Other");
        assert_eq!((mentions[0].start, mentions[0].end), (3, 9));
        assert_eq!(utf16_slice(text, 3, 9), Some("@Other"));
        assert!(PromptDraft::from_text("   \n ").to_input().is_empty());
    }

    #[test]
    fn input_to_draft_joins_segments_and_offsets_mentions() {
        let input = vec![
            PromptInput::text("first"),
            PromptInput::text("   "),
            PromptInput::text_with_mentions("ёж @Other", vec![mention(3, 9)]),
            PromptInput::LocalFile(LocalFilePromptInput {
                path: "docs/spec.md".into(),
                size_bytes: Some(12),
                ..LocalFilePromptInput::default()
            }),
            PromptInput::LocalImage(LocalImagePromptInput {
                path: "/tmp/shot.png".into(),
                host_id: Some("host_1".into()),
                ..LocalImagePromptInput::default()
            }),
        ];
        let draft = PromptDraft::from_input(&input);
        assert_eq!(draft.text, "first\n\nёж @Other");
        assert_eq!((draft.mentions[0].start, draft.mentions[0].end), (10, 16));
        assert_eq!(utf16_slice(&draft.text, 10, 16), Some("@Other"));
        assert_eq!(draft.attachments[0].name, "spec.md");
        assert_eq!(
            draft.attachments[1].owner,
            AttachmentOwner::Host("host_1".into())
        );
        let round = draft.to_input();
        assert_eq!(round.len(), 3);
    }

    #[test]
    fn storage_round_trip_and_lenient_parse() {
        let draft = PromptDraft {
            text: "hello @Other".into(),
            mentions: vec![mention(6, 12)],
            attachments: vec![DraftAttachment {
                kind: AttachmentKind::LocalFile,
                path: "a.txt".into(),
                name: "a.txt".into(),
                mime_type: None,
                size_bytes: Some(0),
                owner: AttachmentOwner::Project("proj".into()),
            }],
        };
        let raw = draft.serialize().unwrap();
        let parsed = PromptDraft::parse(Some(&raw));
        assert_eq!(parsed.text, draft.text);
        assert_eq!(parsed.mentions, draft.mentions);
        assert_eq!(parsed.attachments[0].size_bytes, None);
        assert_eq!(PromptDraft::default().serialize(), None);
        assert_eq!(PromptDraft::parse(Some("not json")), PromptDraft::default());
        let partial = PromptDraft::parse(Some(
            r#"{"mentions":[{"start":0,"end":2,"resource":{"kind":"thread","threadId":"t"}},7],"attachments":[{"type":"localImage","path":"x.png","name":"x.png","hostId":"h"},{"type":"weird"}]}"#,
        ));
        assert_eq!(partial.text, "");
        assert_eq!(partial.mentions.len(), 1);
        assert_eq!(partial.attachments.len(), 1);
    }

    #[test]
    fn mentions_follow_edits_or_drop() {
        let old = "see @Other now";
        let mentions = vec![mention(4, 10)];
        assert_eq!(
            relocate_mentions(old, "please see @Other now", &mentions)[0].start,
            11
        );
        assert!(relocate_mentions(old, "see @Oth now", &mentions).is_empty());
        assert!(relocate_mentions(old, "@Other and @Other", &mentions).is_empty());
        assert_eq!(
            relocate_mentions(old, "see @Other later", &mentions),
            mentions
        );
    }

    #[test]
    fn queued_preview_matches_client_core() {
        let long = "word ".repeat(40);
        let preview = format_queued_message_preview(&[PromptInput::text(long.clone())], true);
        assert!(preview.ends_with("..."));
        assert_eq!(preview.width(), 140);
        assert_eq!(
            format_queued_message_preview(&[PromptInput::text("  a\n\n  b ")], true),
            "a b"
        );
        let hidden = PromptInput::Text(TextPromptInput {
            text: "secret".into(),
            mentions: Vec::new(),
            visibility: Some(PromptInputVisibility::AgentOnly),
        });
        let file = PromptInput::LocalFile(LocalFilePromptInput {
            path: "dir/report.pdf".into(),
            ..LocalFilePromptInput::default()
        });
        assert_eq!(
            format_queued_message_preview(&[hidden.clone(), file.clone()], true),
            "Attachment only (report.pdf)"
        );
        assert_eq!(
            format_queued_message_preview(&[file.clone(), file], true),
            "Attachment only (2 files)"
        );
        assert_eq!(
            format_queued_message_preview(std::slice::from_ref(&hidden), true),
            "(empty message)"
        );
        assert_eq!(queued_input_to_draft(&[hidden]).text, "");
    }

    #[test]
    fn history_skips_empty_and_adjacent_duplicates() {
        let entry = |id: &str, text: &str| PromptHistoryEntry {
            id: id.into(),
            created_at: 0,
            input: vec![PromptInput::text(text)],
        };
        let drafts = history_drafts(&[
            entry("1", "b"),
            entry("2", "b"),
            entry("3", " "),
            entry("4", "a"),
            entry("5", "b"),
        ]);
        let texts: Vec<&str> = drafts.iter().map(|draft| draft.text.as_str()).collect();
        assert_eq!(texts, ["b", "a", "b"]);
    }
}
