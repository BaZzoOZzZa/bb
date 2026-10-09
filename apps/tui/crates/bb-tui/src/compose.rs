use std::time::{Duration, Instant};

use bb_store::ResourceKey;
use bb_wire::interaction::PendingInteractionStatus;
use bb_wire::prompt::{PermissionMode, ReasoningLevel};
use bb_wire::queue::{
    SendMessageResponse, SendQueuedMessageMode, ThreadQueuedMessage, UpdateQueuedMessageRequest,
};
use bb_wire::system::SystemExecutionOptionsQuery;
use bb_wire::thread::{ThreadDisplayStatus, ThreadStatus};
use bb_wire::timeline::{ModelFallback, ThreadTimelineResponse};
use std::path::Path;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::api::{Mutation, MutationReply};
use crate::app::{AppState, Effect, Focus, Mode, ToastLevel};
use crate::attach::{attach_command_argument, complete_path, list_dir, pasted_file_paths};
use crate::composer::{Composer, ComposerKey, EDIT_HOLD_RENEWALS_PER_LEASE, EDIT_HOLD_RETRY};
use crate::execution::{ExecutionState, Load, PickerOption, ResolvedExecution};
use crate::input::{InputOutcome, LineInput};
use crate::keymap::Command;
use crate::prompt::{
    ExecutionSelection, PromptDraft, ShortcutInputs, ShortcutRequest, SubmitMode, SubmitModeInputs,
    build_auto_follow_up_request, build_create_queued_follow_up_request,
    build_follow_up_shortcut_request, build_follow_up_submit_mode, can_submit_follow_up_shortcut,
    cycle_reasoning_value, cycle_value, history_drafts, placeholder, queued_input_to_draft,
    should_queue_follow_up, should_steer_when_ready,
};
use crate::typeahead::ComposerTarget;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Inflight {
    pub submitting: usize,
    pub queue_ops: usize,
    pub processing: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerKind {
    Model,
    Reasoning,
    Permission,
}

impl PickerKind {
    pub fn title(self) -> &'static str {
        match self {
            Self::Model => "Model",
            Self::Reasoning => "Reasoning",
            Self::Permission => "Permissions",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PickerValues {
    Model(Vec<String>),
    Reasoning(Vec<ReasoningLevel>),
    Permission(Vec<PermissionMode>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickerItem {
    pub label: String,
    pub detail: Option<String>,
    pub disabled: bool,
    pub current: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picker {
    pub kind: PickerKind,
    pub thread_id: String,
    pub items: Vec<PickerItem>,
    pub values: PickerValues,
    pub selected: usize,
}

fn picker_items<T: PartialEq>(options: &[PickerOption<T>], current: &T) -> Vec<PickerItem> {
    options
        .iter()
        .map(|option| PickerItem {
            label: option.label.clone(),
            detail: option.detail.clone(),
            disabled: option.disabled,
            current: option.value == *current,
        })
        .collect()
}

impl AppState {
    pub fn thread_display_status(&self, thread_id: &str) -> ThreadDisplayStatus {
        self.sidebar_entry(thread_id)
            .map(|entry| entry.runtime.display_status)
            .or_else(|| {
                self.snapshot
                    .thread(thread_id)
                    .map(|thread| thread.runtime.display_status)
            })
            .unwrap_or(ThreadDisplayStatus::Idle)
    }

    fn thread_status(&self, thread_id: &str) -> Option<ThreadStatus> {
        self.sidebar_entry(thread_id)
            .map(|entry| entry.thread.status)
            .or_else(|| {
                self.snapshot
                    .thread(thread_id)
                    .map(|thread| thread.thread.status)
            })
    }

    pub fn thread_archived(&self, thread_id: &str) -> bool {
        self.sidebar_entry(thread_id)
            .map(|entry| entry.thread.is_archived())
            .or_else(|| {
                self.snapshot
                    .thread(thread_id)
                    .map(|thread| thread.thread.is_archived())
            })
            .unwrap_or(false)
    }

    pub fn has_pending_interaction(&self, thread_id: &str) -> bool {
        match self.snapshot.interactions.get(thread_id) {
            Some(resource) if resource.data.is_some() => self
                .snapshot
                .pending_interactions(thread_id)
                .iter()
                .any(|interaction| interaction.status == PendingInteractionStatus::Pending),
            _ => self
                .sidebar_entry(thread_id)
                .is_some_and(|entry| entry.has_pending_interaction),
        }
    }

    pub fn timeline_head(&self, thread_id: &str) -> Option<&ThreadTimelineResponse> {
        self.snapshot.timeline(thread_id)?.head()
    }

    pub fn model_fallback(&self, thread_id: &str) -> Option<&ModelFallback> {
        self.timeline_head(thread_id)?.model_fallback.as_ref()
    }

    pub fn queued_messages(&self, thread_id: &str) -> &[ThreadQueuedMessage] {
        self.snapshot.queued(thread_id)
    }

    pub fn stop_requested(&self, thread_id: &str) -> bool {
        self.stop_requested.contains(thread_id)
            || self.thread_status(thread_id) == Some(ThreadStatus::Stopping)
    }

    pub fn execution_state(&self, thread_id: &str) -> Option<&ExecutionState> {
        self.executions.get(thread_id)
    }

    pub fn resolved_execution(&self, thread_id: &str) -> Option<ResolvedExecution> {
        let execution = self.executions.get(thread_id)?;
        Some(execution.resolve(self.model_fallback(thread_id)))
    }

    fn execution_selection(&self, thread_id: &str) -> Option<ExecutionSelection> {
        self.executions
            .get(thread_id)?
            .selection(self.model_fallback(thread_id))
    }

    pub fn submit_mode(&self, thread_id: &str) -> SubmitMode {
        let defaults_loading = self.executions.get(thread_id).is_none_or(|execution| {
            execution.defaults_state() == crate::prompt::DefaultsState::Loading
        });
        build_follow_up_submit_mode(SubmitModeInputs {
            has_pending_interaction: self.has_pending_interaction(thread_id),
            defaults_loading,
            stop_requested: self.stop_requested(thread_id),
            status: self.thread_display_status(thread_id),
        })
    }

    fn shortcut_allowed(&self, thread_id: &str, has_draft_input: bool) -> bool {
        let inflight = self.inflight.get(thread_id).cloned().unwrap_or_default();
        can_submit_follow_up_shortcut(ShortcutInputs {
            has_draft_input,
            submitting: inflight.submitting > 0,
            queue_mutation_pending: inflight.queue_ops > 0,
            queued_count: self.queued_messages(thread_id).len(),
            status: self.thread_display_status(thread_id),
            mode: self.submit_mode(thread_id),
        })
    }

    pub fn composer(&self) -> Option<&Composer> {
        self.composers.get(self.open_thread.as_deref()?)
    }

    pub fn composer_available(&self) -> bool {
        self.open_thread
            .as_deref()
            .is_some_and(|thread_id| !self.thread_archived(thread_id))
    }

    pub(crate) fn ensure_thread_data(&mut self, thread_id: &str) -> Vec<Effect> {
        let mut effects = Vec::new();
        if !self.composers.contains_key(thread_id) {
            self.composers
                .insert(thread_id.to_owned(), Composer::new(thread_id));
            effects.push(Effect::LoadDraft(thread_id.to_owned()));
        }
        if !self.history_requested.contains(thread_id) {
            self.history_requested.insert(thread_id.to_owned());
            effects.push(Effect::FetchPromptHistory(thread_id.to_owned()));
        }
        let provider = self
            .sidebar_entry(thread_id)
            .map(|entry| entry.thread.provider_id.clone())
            .or_else(|| {
                self.snapshot
                    .thread(thread_id)
                    .map(|thread| thread.thread.provider_id.clone())
            })
            .filter(|provider| !provider.is_empty());
        let environment = self
            .sidebar_entry(thread_id)
            .and_then(|entry| entry.thread.environment_id.clone())
            .or_else(|| {
                self.snapshot
                    .thread(thread_id)
                    .and_then(|thread| thread.thread.environment_id.clone())
            });
        let execution = self
            .executions
            .entry(thread_id.to_owned())
            .or_insert_with(|| ExecutionState::new(provider.clone().unwrap_or_default()));
        if execution.defaults.is_idle() {
            execution.defaults = Load::Loading;
            effects.push(Effect::FetchDefaults(thread_id.to_owned()));
        }
        if let Some(provider) = provider {
            if execution.provider_id.is_empty() {
                execution.provider_id = provider.clone();
            }
            if execution.options.is_idle() {
                execution.options = Load::Loading;
                effects.push(Effect::FetchExecutionOptions {
                    thread_id: thread_id.to_owned(),
                    query: SystemExecutionOptionsQuery {
                        host_id: None,
                        environment_id: environment,
                        provider_id: Some(provider),
                    },
                });
            }
        }
        effects
    }

    pub(crate) fn sync_composer_placeholder(&mut self) {
        let Some(thread_id) = self.open_thread.clone() else {
            return;
        };
        let status = if self.stop_requested(&thread_id) {
            ThreadDisplayStatus::Stopping
        } else {
            self.thread_display_status(&thread_id)
        };
        if let Some(composer) = self.composers.get_mut(&thread_id) {
            composer.set_placeholder(placeholder(status));
        }
    }

    pub(crate) fn flush_draft(&mut self, thread_id: &str, force: bool) -> Option<Effect> {
        let now = self.clock;
        let raw = self.composers.get_mut(thread_id)?.take_save(now, force)?;
        Some(Effect::SaveDraft {
            thread_id: thread_id.to_owned(),
            raw,
        })
    }

    pub(crate) fn flush_all_drafts(&mut self) -> Vec<Effect> {
        let ids: Vec<String> = self.composers.keys().cloned().collect();
        ids.iter()
            .filter_map(|id| self.flush_draft(id, true))
            .collect()
    }

    fn inflight_mut(&mut self, thread_id: &str) -> &mut Inflight {
        self.inflight.entry(thread_id.to_owned()).or_default()
    }

    fn take_submission(&mut self, thread_id: &str) -> Option<(PromptDraft, Vec<Effect>)> {
        let now = self.clock;
        let composer = self.composers.get_mut(thread_id)?;
        let draft = composer.take_submission(now);
        composer.pending_restore = Some(draft.clone());
        let effects = self.flush_draft(thread_id, true).into_iter().collect();
        Some((draft, effects))
    }

    pub(crate) fn submit(&mut self, steer: bool) -> Vec<Effect> {
        let Some(thread_id) = self.open_thread.clone() else {
            return Vec::new();
        };
        let Some(composer) = self.composers.get(&thread_id) else {
            return Vec::new();
        };
        if composer.editing.is_some() {
            return self.save_queued_edit(&thread_id);
        }
        let input = composer.draft().to_input();
        if self.read_only {
            if !input.is_empty() {
                self.toast("Read-only mode: cannot send message", ToastLevel::Error);
            }
            return Vec::new();
        }
        let wants_steer = steer != self.enter_steers;
        if wants_steer {
            if self.shortcut_allowed(&thread_id, !input.is_empty()) {
                return self.submit_shortcut(&thread_id, input);
            }
            if steer {
                self.toast("Nothing to steer right now", ToastLevel::Info);
                return Vec::new();
            }
        }
        let mode = self.submit_mode(&thread_id);
        match mode {
            SubmitMode::Blocked(crate::prompt::BlockedReason::PendingInteraction) => {
                self.toast(
                    "The agent is waiting for your answer to a pending request",
                    ToastLevel::Info,
                );
                return Vec::new();
            }
            SubmitMode::Blocked(_)
                if !should_queue_follow_up(self.thread_display_status(&thread_id)) =>
            {
                self.toast("Model options are still loading", ToastLevel::Info);
                return Vec::new();
            }
            _ => {}
        }
        if input.is_empty() {
            return Vec::new();
        }
        let execution = self.execution_selection(&thread_id);
        let queuing = should_queue_follow_up(self.thread_display_status(&thread_id));
        let mutation = if queuing {
            build_create_queued_follow_up_request(input, execution.as_ref()).map(|request| {
                Mutation::QueueMessage {
                    thread_id: thread_id.clone(),
                    request: Box::new(request),
                }
            })
        } else {
            build_auto_follow_up_request(input, execution.as_ref()).map(|request| {
                Mutation::SendMessage {
                    thread_id: thread_id.clone(),
                    request: Box::new(request),
                }
            })
        };
        let Some(mutation) = mutation else {
            return Vec::new();
        };
        let Some((_, mut effects)) = self.take_submission(&thread_id) else {
            return Vec::new();
        };
        self.inflight_mut(&thread_id).submitting += 1;
        effects.push(Effect::Mutate(mutation));
        effects
    }

    fn submit_shortcut(
        &mut self,
        thread_id: &str,
        input: Vec<bb_wire::prompt::PromptInput>,
    ) -> Vec<Effect> {
        let execution = self.execution_selection(thread_id);
        let queued = self.queued_messages(thread_id).to_vec();
        match build_follow_up_shortcut_request(input, execution.as_ref(), &queued) {
            Some(ShortcutRequest::Draft(request)) => {
                let Some((_, mut effects)) = self.take_submission(thread_id) else {
                    return Vec::new();
                };
                self.inflight_mut(thread_id).submitting += 1;
                effects.push(Effect::Mutate(Mutation::SendMessage {
                    thread_id: thread_id.to_owned(),
                    request: Box::new(request),
                }));
                effects
            }
            Some(ShortcutRequest::Queued { queued_message_id }) => {
                self.send_queued(thread_id, &queued_message_id, SendQueuedMessageMode::Steer)
            }
            None => Vec::new(),
        }
    }

    fn send_queued(
        &mut self,
        thread_id: &str,
        queued_message_id: &str,
        mode: SendQueuedMessageMode,
    ) -> Vec<Effect> {
        if !self
            .queued_messages(thread_id)
            .iter()
            .any(|message| message.id == queued_message_id)
        {
            return Vec::new();
        }
        let inflight = self.inflight_mut(thread_id);
        inflight.queue_ops += 1;
        inflight.processing = Some(queued_message_id.to_owned());
        vec![Effect::Mutate(Mutation::SendQueued {
            thread_id: thread_id.to_owned(),
            queued_message_id: queued_message_id.to_owned(),
            mode,
        })]
    }

    pub(crate) fn stop_turn(&mut self) -> Vec<Effect> {
        let Some(thread_id) = self.open_thread.clone() else {
            return Vec::new();
        };
        if !self.submit_mode(&thread_id).can_stop() {
            self.toast("Nothing to stop", ToastLevel::Info);
            return Vec::new();
        }
        if self.read_only {
            self.toast("Read-only mode: cannot stop", ToastLevel::Error);
            return Vec::new();
        }
        self.stop_requested.insert(thread_id.clone());
        vec![Effect::Mutate(Mutation::Stop { thread_id })]
    }

    fn save_queued_edit(&mut self, thread_id: &str) -> Vec<Effect> {
        let Some(composer) = self.composers.get_mut(thread_id) else {
            return Vec::new();
        };
        let input = composer.draft().to_input();
        let Some(edit) = composer.editing.as_mut() else {
            return Vec::new();
        };
        if input.is_empty() || edit.saving {
            return Vec::new();
        }
        edit.saving = true;
        let queued_message_id = edit.queued_message_id.clone();
        let expected_updated_at = edit.expected_updated_at;
        self.inflight_mut(thread_id).queue_ops += 1;
        vec![Effect::Mutate(Mutation::UpdateQueued {
            thread_id: thread_id.to_owned(),
            queued_message_id,
            request: Box::new(UpdateQueuedMessageRequest {
                expected_updated_at,
                input,
            }),
        })]
    }

    pub(crate) fn cancel_queued_edit(&mut self, thread_id: &str) -> Vec<Effect> {
        let Some(edit) = self
            .composers
            .get_mut(thread_id)
            .and_then(Composer::end_queued_edit)
        else {
            return Vec::new();
        };
        vec![Effect::Mutate(Mutation::ReleaseQueued {
            thread_id: thread_id.to_owned(),
            queued_message_id: edit.queued_message_id,
        })]
    }

    fn begin_queued_edit(&mut self, thread_id: &str, message: &ThreadQueuedMessage) -> Vec<Effect> {
        if self.read_only {
            self.toast(
                "Read-only mode: cannot edit queued messages",
                ToastLevel::Error,
            );
            return Vec::new();
        }
        if !message.editable {
            self.toast("This queued message cannot be edited", ToastLevel::Info);
            return Vec::new();
        }
        let mut effects = Vec::new();
        if let Some(previous) = self
            .composers
            .get(thread_id)
            .and_then(|composer| composer.editing.as_ref())
            .map(|edit| edit.queued_message_id.clone())
            .filter(|previous| *previous != message.id)
        {
            effects.push(Effect::Mutate(Mutation::ReleaseQueued {
                thread_id: thread_id.to_owned(),
                queued_message_id: previous,
            }));
        }
        let Some(composer) = self.composers.get_mut(thread_id) else {
            return effects;
        };
        composer.begin_queued_edit(
            message.id.clone(),
            message.updated_at,
            queued_input_to_draft(&message.content),
        );
        self.focus = Focus::Composer;
        effects.push(Effect::Mutate(Mutation::HoldQueued {
            thread_id: thread_id.to_owned(),
            queued_message_id: message.id.clone(),
        }));
        effects
    }

    fn open_picker(&mut self, kind: PickerKind) -> Vec<Effect> {
        let Some(thread_id) = self.open_thread.clone() else {
            return Vec::new();
        };
        let Some(execution) = self.executions.get(&thread_id) else {
            return Vec::new();
        };
        if let Load::Failed(error) = &execution.options {
            let message = format!("Model options failed to load: {error}");
            self.toast(message, ToastLevel::Error);
            return Vec::new();
        }
        let resolved = execution.resolve(self.model_fallback(&thread_id));
        let (items, values) = match kind {
            PickerKind::Model => (
                picker_items(&resolved.model_options, &resolved.model),
                PickerValues::Model(
                    resolved
                        .model_options
                        .iter()
                        .map(|option| option.value.clone())
                        .collect(),
                ),
            ),
            PickerKind::Reasoning => (
                picker_items(&resolved.reasoning_options, &resolved.reasoning),
                PickerValues::Reasoning(
                    resolved
                        .reasoning_options
                        .iter()
                        .map(|option| option.value)
                        .collect(),
                ),
            ),
            PickerKind::Permission => {
                if self.plan_mode_active(&thread_id) {
                    self.toast(
                        "Permissions are fixed while plan mode is active",
                        ToastLevel::Info,
                    );
                    return Vec::new();
                }
                (
                    picker_items(&resolved.permission_options, &resolved.permission),
                    PickerValues::Permission(
                        resolved
                            .permission_options
                            .iter()
                            .map(|option| option.value)
                            .collect(),
                    ),
                )
            }
        };
        if items.is_empty() {
            let message = if execution.options.loaded().is_none() {
                "Model options are still loading"
            } else {
                "No options available for this provider"
            };
            self.toast(message, ToastLevel::Info);
            return Vec::new();
        }
        let selected = items.iter().position(|item| item.current).unwrap_or(0);
        self.mode = Mode::Picker(Picker {
            kind,
            thread_id,
            items,
            values,
            selected,
        });
        Vec::new()
    }

    pub fn plan_mode_active(&self, thread_id: &str) -> bool {
        self.timeline_head(thread_id)
            .and_then(|head| head.active_prompt_mode.as_ref())
            .is_some_and(|mode| mode.mode == "plan")
    }

    fn apply_picker(&mut self, picker: &Picker, index: usize) {
        let fallback = self.model_fallback(&picker.thread_id).cloned();
        let Some(execution) = self.executions.get_mut(&picker.thread_id) else {
            return;
        };
        match &picker.values {
            PickerValues::Model(values) => {
                if let Some(value) = values.get(index) {
                    execution.select_model(value.clone(), fallback.as_ref());
                }
            }
            PickerValues::Reasoning(values) => execution.reasoning = values.get(index).copied(),
            PickerValues::Permission(values) => execution.permission = values.get(index).copied(),
        }
    }

    fn cycle(&mut self, kind: PickerKind, forward: bool) {
        let Some(thread_id) = self.open_thread.clone() else {
            return;
        };
        let fallback = self.model_fallback(&thread_id).cloned();
        let Some(execution) = self.executions.get_mut(&thread_id) else {
            return;
        };
        let resolved = execution.resolve(fallback.as_ref());
        let message = match kind {
            PickerKind::Model => {
                let values: Vec<String> = resolved
                    .model_options
                    .iter()
                    .map(|option| option.value.clone())
                    .collect();
                cycle_value(&values, &resolved.model, forward).map(|next| {
                    execution.select_model(next, fallback.as_ref());
                    let label = execution.resolve(fallback.as_ref()).model_label;
                    format!("Model: {label}")
                })
            }
            PickerKind::Reasoning => {
                let values: Vec<ReasoningLevel> = resolved
                    .reasoning_options
                    .iter()
                    .map(|option| option.value)
                    .collect();
                cycle_reasoning_value(&values, resolved.reasoning, forward).map(|next| {
                    execution.reasoning = Some(next);
                    let label = resolved
                        .reasoning_options
                        .iter()
                        .find(|option| option.value == next)
                        .map_or_else(|| next.to_string(), |option| option.label.clone());
                    format!("Reasoning: {label}")
                })
            }
            PickerKind::Permission => None,
        };
        match message {
            Some(message) => self.toast(message, ToastLevel::Info),
            None => self.toast("No other option available", ToastLevel::Info),
        }
    }
}

pub(crate) fn composer_command(state: &mut AppState, command: Command) -> Option<Vec<Effect>> {
    let has_thread = state.open_thread.is_some();
    let effects = match command {
        Command::FocusComposer => {
            if state.open_interaction_panel() {
                state.focus = Focus::Interaction;
            } else if state.composer_available() {
                state.focus = Focus::Composer;
                if let Some(composer) = state
                    .open_thread
                    .clone()
                    .and_then(|thread_id| state.composers.get_mut(&thread_id))
                {
                    composer.enter_insert();
                }
            } else if has_thread {
                state.toast("This thread is archived", ToastLevel::Info);
            }
            Vec::new()
        }
        Command::StopTurn if has_thread => state.stop_turn(),
        Command::ModelPicker if has_thread => state.open_picker(PickerKind::Model),
        Command::ReasoningPicker if has_thread => state.open_picker(PickerKind::Reasoning),
        Command::PermissionPicker if has_thread => state.open_picker(PickerKind::Permission),
        Command::CycleModel if has_thread => {
            state.cycle(PickerKind::Model, true);
            Vec::new()
        }
        Command::CycleModelBackward if has_thread => {
            state.cycle(PickerKind::Model, false);
            Vec::new()
        }
        Command::CycleReasoning if has_thread => {
            state.cycle(PickerKind::Reasoning, true);
            Vec::new()
        }
        Command::CycleReasoningBackward if has_thread => {
            state.cycle(PickerKind::Reasoning, false);
            Vec::new()
        }
        Command::QueueList if has_thread => {
            let thread_id = state.open_thread.clone().unwrap_or_default();
            if state.queued_messages(&thread_id).is_empty() {
                state.toast("No queued messages", ToastLevel::Info);
            } else {
                state.mode = Mode::Queue {
                    thread_id,
                    selected: 0,
                };
            }
            Vec::new()
        }
        Command::AttachFile => state.open_attach_prompt(),
        Command::Attachments => state.open_attachment_list(),
        Command::ToggleSubmitMode => {
            state.enter_steers = !state.enter_steers;
            let message = if state.enter_steers {
                "While running, Enter steers the current turn"
            } else {
                "While running, Enter queues the message"
            };
            state.toast(message, ToastLevel::Info);
            Vec::new()
        }
        Command::StopTurn
        | Command::ModelPicker
        | Command::ReasoningPicker
        | Command::PermissionPicker
        | Command::CycleModel
        | Command::CycleModelBackward
        | Command::CycleReasoning
        | Command::CycleReasoningBackward
        | Command::QueueList => {
            state.toast("Open a thread first", ToastLevel::Info);
            Vec::new()
        }
        _ => return None,
    };
    Some(effects)
}

pub(crate) fn handle_composer_key(state: &mut AppState, key: KeyEvent) -> Vec<Effect> {
    let Some(thread_id) = state.open_thread.clone() else {
        state.focus = Focus::Main;
        return Vec::new();
    };
    if !state.composer_available() {
        state.focus = Focus::Main;
        return Vec::new();
    }
    let now = state.clock;
    let submit_key = state.submit_key;
    let Some(composer) = state.composers.get_mut(&thread_id) else {
        state.focus = Focus::Main;
        return Vec::new();
    };
    match composer.handle_key(key, submit_key, now) {
        ComposerKey::Submit => {
            let target = ComposerTarget::Thread(thread_id.clone());
            match take_attach_command(state, &target) {
                Some(effects) => effects,
                None => state.submit(false),
            }
        }
        ComposerKey::Escape => {
            composer.enter_normal();
            Vec::new()
        }
        ComposerKey::Edited | ComposerKey::Handled => Vec::new(),
        ComposerKey::Unhandled => {
            insert_unhandled(state, &key);
            Vec::new()
        }
    }
}

pub(crate) fn insert_unhandled(state: &mut AppState, key: &KeyEvent) {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Char('o') if ctrl => {
            let target = state.active_composer_target();
            if let Some(composer) = target.and_then(|target| state.composer_for_mut(&target)) {
                composer.enter_normal();
                composer.one_shot = true;
            }
        }
        KeyCode::Char('r') if ctrl => {
            state.keys.await_char = Some((crate::vi::CharAct::InsertRegister, None));
        }
        _ => {}
    }
}

pub(crate) fn handle_picker_key(
    state: &mut AppState,
    mut picker: Picker,
    key: KeyEvent,
) -> Vec<Effect> {
    let count = picker.items.len();
    let choice = match key.code {
        KeyCode::Esc | KeyCode::Char('q') => return Vec::new(),
        KeyCode::Enter | KeyCode::Char(' ') => Some(picker.selected),
        KeyCode::Char(ch) if ch.is_ascii_digit() && ch != '0' => Some(usize::from(ch as u8 - b'1')),
        KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => {
            picker.selected = (picker.selected + 1) % count.max(1);
            None
        }
        KeyCode::Up | KeyCode::Char('k') | KeyCode::BackTab => {
            picker.selected = (picker.selected + count.max(1) - 1) % count.max(1);
            None
        }
        KeyCode::Home | KeyCode::Char('g') => {
            picker.selected = 0;
            None
        }
        KeyCode::End | KeyCode::Char('G') => {
            picker.selected = count.saturating_sub(1);
            None
        }
        _ => None,
    };
    if let Some(index) = choice
        && let Some(item) = picker.items.get(index)
    {
        if item.disabled {
            let detail = item
                .detail
                .clone()
                .unwrap_or_else(|| "Not available".into());
            state.toast(detail, ToastLevel::Info);
        } else {
            let label = item.label.clone();
            state.apply_picker(&picker, index);
            state.toast(
                format!("{}: {label}", picker.kind.title()),
                ToastLevel::Info,
            );
            return Vec::new();
        }
    }
    state.mode = Mode::Picker(picker);
    Vec::new()
}

pub(crate) fn handle_queue_key(
    state: &mut AppState,
    thread_id: String,
    mut selected: usize,
    key: KeyEvent,
) -> Vec<Effect> {
    let queued = state.queued_messages(&thread_id).to_vec();
    if queued.is_empty() {
        return Vec::new();
    }
    selected = selected.min(queued.len() - 1);
    let message = &queued[selected];
    let effects = match key.code {
        KeyCode::Esc | KeyCode::Char('q') => return Vec::new(),
        KeyCode::Down | KeyCode::Char('j') => {
            selected = (selected + 1).min(queued.len() - 1);
            Vec::new()
        }
        KeyCode::Up | KeyCode::Char('k') => {
            selected = selected.saturating_sub(1);
            Vec::new()
        }
        KeyCode::Enter | KeyCode::Char('e') => return state.begin_queued_edit(&thread_id, message),
        KeyCode::Char('d') | KeyCode::Delete | KeyCode::Backspace => {
            let inflight = state.inflight_mut(&thread_id);
            inflight.queue_ops += 1;
            inflight.processing = Some(message.id.clone());
            vec![Effect::Mutate(Mutation::DeleteQueued {
                thread_id: thread_id.clone(),
                queued_message_id: message.id.clone(),
            })]
        }
        KeyCode::Char('s') => {
            let mode = if should_steer_when_ready(state.thread_display_status(&thread_id)) {
                SendQueuedMessageMode::Steer
            } else {
                SendQueuedMessageMode::Auto
            };
            state.send_queued(&thread_id, &message.id, mode)
        }
        KeyCode::Char('r') => {
            if message.failure_reason.is_none() {
                state.toast("Only failed messages can be retried", ToastLevel::Info);
                Vec::new()
            } else {
                state.send_queued(&thread_id, &message.id, SendQueuedMessageMode::Auto)
            }
        }
        _ => Vec::new(),
    };
    state.mode = Mode::Queue {
        thread_id,
        selected,
    };
    effects
}

fn refresh_thread_effects(thread_id: &str) -> Vec<Effect> {
    vec![
        Effect::Refresh(ResourceKey::QueuedMessages(thread_id.to_owned())),
        Effect::Refresh(ResourceKey::Thread(thread_id.to_owned())),
        Effect::RefreshSidebar,
    ]
}

pub(crate) fn composer_mutation_finished(
    state: &mut AppState,
    mutation: Mutation,
    result: Result<MutationReply, String>,
) -> Vec<Effect> {
    let thread_id = mutation.thread_id().to_owned();
    let now = state.clock;
    match &mutation {
        Mutation::SendMessage { .. } | Mutation::QueueMessage { .. } => {
            let inflight = state.inflight_mut(&thread_id);
            inflight.submitting = inflight.submitting.saturating_sub(1);
        }
        Mutation::SendQueued { .. }
        | Mutation::UpdateQueued { .. }
        | Mutation::DeleteQueued { .. } => {
            let inflight = state.inflight_mut(&thread_id);
            inflight.queue_ops = inflight.queue_ops.saturating_sub(1);
            if inflight.processing.as_deref() == mutation.queued_message_id() {
                inflight.processing = None;
            }
        }
        Mutation::Stop { .. } => {
            state.stop_requested.remove(&thread_id);
        }
        _ => {}
    }
    match (mutation, result) {
        (Mutation::SendMessage { .. } | Mutation::QueueMessage { .. }, Ok(reply)) => {
            if let Some(composer) = state.composers.get_mut(&thread_id)
                && let Some(draft) = composer.pending_restore.take()
                && composer.history.first() != Some(&draft)
            {
                composer.history.insert(0, draft);
            }
            let queued = matches!(
                reply,
                MutationReply::Delivery(SendMessageResponse::Queued { .. })
            ) || matches!(reply, MutationReply::Done);
            if queued {
                state.toast("Queued — runs after the current turn", ToastLevel::Info);
            }
            let mut effects = refresh_thread_effects(&thread_id);
            effects.push(Effect::FetchDefaults(thread_id.clone()));
            effects.push(Effect::FetchPromptHistory(thread_id));
            effects
        }
        (mutation @ (Mutation::SendMessage { .. } | Mutation::QueueMessage { .. }), Err(error)) => {
            if let Some(composer) = state.composers.get_mut(&thread_id)
                && let Some(draft) = composer.pending_restore.take()
            {
                composer.restore_if_empty(draft, now);
            }
            state.toast(
                format!("Failed to {}: {error}", mutation.failure_verb()),
                ToastLevel::Error,
            );
            state.flush_draft(&thread_id, true).into_iter().collect()
        }
        (
            Mutation::UpdateQueued {
                queued_message_id, ..
            },
            Ok(_),
        ) => {
            let mut effects = Vec::new();
            if let Some(composer) = state.composers.get_mut(&thread_id)
                && composer
                    .editing
                    .as_ref()
                    .is_some_and(|edit| edit.queued_message_id == queued_message_id)
            {
                composer.end_queued_edit();
                effects.push(Effect::Mutate(Mutation::ReleaseQueued {
                    thread_id: thread_id.clone(),
                    queued_message_id,
                }));
            }
            state.toast("Queued message updated", ToastLevel::Info);
            effects.extend(refresh_thread_effects(&thread_id));
            effects
        }
        (
            Mutation::UpdateQueued {
                queued_message_id, ..
            },
            Err(error),
        ) => {
            if let Some(edit) = state
                .composers
                .get_mut(&thread_id)
                .and_then(|composer| composer.editing.as_mut())
                .filter(|edit| edit.queued_message_id == queued_message_id)
            {
                edit.saving = false;
            }
            state.toast(
                format!("Failed to update queued message: {error}"),
                ToastLevel::Error,
            );
            refresh_thread_effects(&thread_id)
        }
        (
            Mutation::HoldQueued {
                queued_message_id, ..
            },
            Ok(MutationReply::Lease { lease_ms }),
        ) => {
            if let Some(edit) = state
                .composers
                .get_mut(&thread_id)
                .and_then(|composer| composer.editing.as_mut())
                .filter(|edit| edit.queued_message_id == queued_message_id)
            {
                let renew =
                    Duration::from_millis(lease_ms.max(1_000)) / EDIT_HOLD_RENEWALS_PER_LEASE;
                edit.renew_at = Some(now + renew);
            }
            Vec::new()
        }
        (Mutation::HoldQueued { .. }, Ok(_)) => Vec::new(),
        (
            Mutation::HoldQueued {
                queued_message_id, ..
            },
            Err(error),
        ) => {
            let editing = state
                .composers
                .get(&thread_id)
                .and_then(|composer| composer.editing.as_ref())
                .is_some_and(|edit| edit.queued_message_id == queued_message_id);
            if editing {
                let rejected = error.contains("404") || error.contains("409");
                if rejected {
                    if let Some(composer) = state.composers.get_mut(&thread_id) {
                        composer.end_queued_edit();
                    }
                    state.toast(
                        format!("Cannot edit queued message: {error}"),
                        ToastLevel::Error,
                    );
                } else if let Some(edit) = state
                    .composers
                    .get_mut(&thread_id)
                    .and_then(|composer| composer.editing.as_mut())
                {
                    edit.renew_at = Some(now + EDIT_HOLD_RETRY);
                }
            }
            Vec::new()
        }
        (Mutation::ReleaseQueued { .. }, _) => Vec::new(),
        (Mutation::DeleteQueued { .. }, Ok(_)) => {
            state.toast("Queued message deleted", ToastLevel::Info);
            refresh_thread_effects(&thread_id)
        }
        (Mutation::SendQueued { .. }, Ok(_)) => refresh_thread_effects(&thread_id),
        (Mutation::Stop { .. }, Ok(_)) => {
            state.toast("Stopping…", ToastLevel::Info);
            refresh_thread_effects(&thread_id)
        }
        (mutation, Err(error)) => {
            state.toast(
                format!("Failed to {}: {error}", mutation.failure_verb()),
                ToastLevel::Error,
            );
            refresh_thread_effects(&thread_id)
        }
        (_, Ok(_)) => Vec::new(),
    }
}

pub(crate) fn composer_tick(state: &mut AppState, now: Instant) -> Vec<Effect> {
    let mut effects = Vec::new();
    let ids: Vec<String> = state.composers.keys().cloned().collect();
    for thread_id in ids {
        if let Some(effect) = state.flush_draft(&thread_id, false) {
            effects.push(effect);
        }
        let Some(edit) = state
            .composers
            .get_mut(&thread_id)
            .and_then(|composer| composer.editing.as_mut())
        else {
            continue;
        };
        if edit.renew_at.is_some_and(|at| at <= now) {
            edit.renew_at = None;
            effects.push(Effect::Mutate(Mutation::HoldQueued {
                thread_id: thread_id.clone(),
                queued_message_id: edit.queued_message_id.clone(),
            }));
        }
    }
    effects
}

pub(crate) fn composer_snapshot(state: &mut AppState) -> Vec<Effect> {
    let mut effects = Vec::new();
    let mut vanished = Vec::new();
    for (thread_id, composer) in &state.composers {
        let Some(edit) = &composer.editing else {
            continue;
        };
        let loaded = state
            .snapshot
            .queued_messages
            .get(thread_id)
            .is_some_and(|resource| resource.data.is_some());
        if loaded
            && !state
                .snapshot
                .queued(thread_id)
                .iter()
                .any(|message| message.id == edit.queued_message_id)
        {
            vanished.push(thread_id.clone());
        }
    }
    for thread_id in vanished {
        if let Some(composer) = state.composers.get_mut(&thread_id)
            && composer.editing.as_ref().is_some_and(|edit| !edit.saving)
        {
            composer.end_queued_edit();
            state.toast(
                "The queued message was sent before your edit was saved",
                ToastLevel::Info,
            );
        }
    }
    if let Some(thread_id) = state.open_thread.clone() {
        effects.extend(state.ensure_thread_data(&thread_id));
        if let Mode::Queue {
            thread_id: queue_thread,
            ..
        } = &state.mode
            && state.queued_messages(queue_thread).is_empty()
        {
            state.mode = Mode::Normal;
        }
    }
    effects
}

pub(crate) fn history_loaded(
    state: &mut AppState,
    thread_id: &str,
    result: Result<Vec<bb_wire::prompt::PromptHistoryEntry>, String>,
) {
    match result {
        Ok(entries) => {
            if let Some(composer) = state.composers.get_mut(thread_id) {
                composer.history = history_drafts(&entries);
            }
        }
        Err(error) => tracing::debug!(%error, thread_id, "prompt history unavailable"),
    }
}

pub(crate) fn take_attach_command(
    state: &mut AppState,
    target: &ComposerTarget,
) -> Option<Vec<Effect>> {
    let now = state.clock;
    let composer = state.composer_for_mut(target)?;
    if composer.editing.is_some() || !composer.draft().mentions.is_empty() {
        return None;
    }
    let argument = attach_command_argument(&composer.draft().text)?.to_owned();
    if argument.is_empty() {
        composer.set_draft(
            PromptDraft::default().with_attachments(composer.draft()),
            now,
        );
        return Some(state.open_attach_prompt());
    }
    if state.read_only {
        state.toast(
            "Read-only mode: cannot upload attachments",
            ToastLevel::Error,
        );
        return Some(Vec::new());
    }
    let single = crate::attach::normalize_path_word(&argument).filter(|path| path.is_file());
    let paths = match single
        .map(|path| vec![path])
        .or_else(|| crate::attach::parse_path_list(&argument))
    {
        Some(paths) => paths,
        None => {
            state.toast(
                "Give an absolute path (or ~/…) to attach",
                ToastLevel::Error,
            );
            return Some(Vec::new());
        }
    };
    if let Some(composer) = state.composer_for_mut(target) {
        composer.set_draft(
            PromptDraft::default().with_attachments(composer.draft()),
            now,
        );
    }
    Some(state.attach_paths(target, paths))
}

pub(crate) fn paste_attachments(state: &mut AppState, text: &str) -> Option<Vec<Effect>> {
    let target = state.active_composer_target()?;
    let paths = pasted_file_paths(text, Path::is_file)?;
    Some(state.attach_paths(&target, paths))
}

pub(crate) fn handle_attach_prompt_key(
    state: &mut AppState,
    target: ComposerTarget,
    mut input: LineInput,
    key: KeyEvent,
) -> Vec<Effect> {
    if key.code == KeyCode::Tab {
        if let Some(completed) = complete_path(input.text(), list_dir) {
            input = LineInput::new(completed);
        }
        state.mode = Mode::AttachPrompt { target, input };
        return Vec::new();
    }
    match input.handle_key(key) {
        InputOutcome::Submit => {
            let text = input.text().trim().to_owned();
            if text.is_empty() {
                return Vec::new();
            }
            state.attach_from_text(&target, &text)
        }
        InputOutcome::Cancel => Vec::new(),
        _ => {
            state.mode = Mode::AttachPrompt { target, input };
            Vec::new()
        }
    }
}

pub(crate) fn handle_attachments_key(
    state: &mut AppState,
    target: ComposerTarget,
    selected: usize,
    key: KeyEvent,
) -> Vec<Effect> {
    let now = state.clock;
    let count = state
        .composer_for(&target)
        .map_or(0, |composer| composer.draft().attachments.len());
    if count == 0 {
        return Vec::new();
    }
    let selected = selected.min(count - 1);
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => return Vec::new(),
        KeyCode::Down | KeyCode::Char('j') => {
            state.mode = Mode::Attachments {
                target,
                selected: (selected + 1).min(count - 1),
            };
        }
        KeyCode::Up | KeyCode::Char('k') => {
            state.mode = Mode::Attachments {
                target,
                selected: selected.saturating_sub(1),
            };
        }
        KeyCode::Char('d' | 'x') | KeyCode::Delete | KeyCode::Backspace => {
            let removed = state
                .composer_for_mut(&target)
                .and_then(|composer| composer.remove_attachment(selected, now));
            if let Some(removed) = removed {
                state.toast(format!("Removed {}", removed.name), ToastLevel::Info);
            }
            if count > 1 {
                state.mode = Mode::Attachments {
                    target,
                    selected: selected.min(count - 2),
                };
            }
        }
        _ => {
            state.mode = Mode::Attachments { target, selected };
        }
    }
    Vec::new()
}

pub(crate) fn paste(state: &mut AppState, text: &str) -> Vec<Effect> {
    if state.focus == Focus::Interaction {
        crate::interact::interaction_paste(state, text);
        return Vec::new();
    }
    if state.focus != Focus::Composer {
        return Vec::new();
    }
    let now = state.clock;
    if let Some(composer) = state
        .open_thread
        .clone()
        .and_then(|thread_id| state.composers.get_mut(&thread_id))
    {
        composer.insert_text(text, now);
    }
    Vec::new()
}
