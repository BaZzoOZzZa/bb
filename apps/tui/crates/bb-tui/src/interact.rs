use std::cell::Cell;
use std::collections::BTreeMap;

use bb_store::ResourceKey;
use bb_wire::interaction::{
    ApprovalDecision, InteractionPayload, InteractionResolution, PendingInteraction,
    SecretRequestResponse, USER_QUESTION_MAX_FREE_TEXT_LENGTH,
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::api::{InteractionValue, Mutation, MutationReply};
use crate::app::{AppState, Effect, Focus, ToastLevel};
use crate::input::{InputOutcome, LineInput};
use crate::interaction::{
    AnswerChannel, AnswerState, CancelAction, RequestKind, ShortcutChoice, active_interactions,
    approval_outcome, build_approval_resolution, build_question_answers, classify,
    is_question_answered, ordered_decisions, select_other, shortcut_choice, thread_web_url,
    toggle_option, utf16_len, validate_secret_values,
};
use crate::keymap::Command;
use crate::sidebar::SidebarRow;

#[derive(Debug, Default)]
pub struct ApprovalForm {
    pub expanded: bool,
    pub reason: Option<LineInput>,
}

#[derive(Debug, Default)]
pub struct QuestionForm {
    pub answers: Vec<AnswerState>,
    pub inputs: Vec<LineInput>,
    pub current: usize,
    pub cursor: usize,
    pub editing: bool,
}

#[derive(Default)]
pub struct SecretForm {
    pub inputs: Vec<LineInput>,
    pub field: usize,
    pub revealed: bool,
}

impl std::fmt::Debug for SecretForm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SecretForm")
            .field("fields", &self.inputs.len())
            .field("field", &self.field)
            .field("revealed", &self.revealed)
            .finish_non_exhaustive()
    }
}

impl SecretForm {
    pub fn clear(&mut self) {
        for input in &mut self.inputs {
            *input = LineInput::default();
        }
    }
}

#[derive(Debug)]
pub enum FormBody {
    Approval(ApprovalForm),
    Question(QuestionForm),
    Secret(SecretForm),
    Plugin,
}

#[derive(Debug)]
pub struct InteractionForm {
    pub interaction_id: String,
    pub thread_id: String,
    pub kind: RequestKind,
    pub body: FormBody,
    pub submitting: bool,
    pub submitted: bool,
    pub error: Option<String>,
    pub scroll: usize,
    pub max_scroll: Cell<usize>,
}

impl InteractionForm {
    pub fn new(interaction: &PendingInteraction) -> Self {
        let kind = classify(interaction);
        let body = match &kind {
            RequestKind::Approval(_) | RequestKind::PlanReview { .. } => {
                FormBody::Approval(ApprovalForm::default())
            }
            RequestKind::Question { questions, .. } => {
                let mut form = QuestionForm {
                    answers: questions.iter().map(AnswerState::initial).collect(),
                    inputs: questions.iter().map(|_| LineInput::default()).collect(),
                    ..QuestionForm::default()
                };
                form.editing = questions
                    .first()
                    .is_some_and(|question| question.options.is_empty());
                FormBody::Question(form)
            }
            RequestKind::Secret { payload, .. } => FormBody::Secret(SecretForm {
                inputs: payload
                    .fields
                    .iter()
                    .map(|_| LineInput::default())
                    .collect(),
                ..SecretForm::default()
            }),
            RequestKind::Plugin { .. } => FormBody::Plugin,
        };
        Self {
            interaction_id: interaction.id.clone(),
            thread_id: interaction.thread_id.clone(),
            kind,
            body,
            submitting: false,
            submitted: false,
            error: None,
            scroll: 0,
            max_scroll: Cell::new(0),
        }
    }

    pub fn is_busy(&self) -> bool {
        self.submitting || self.submitted
    }

    pub fn is_typing(&self) -> bool {
        match &self.body {
            FormBody::Approval(form) => form.reason.is_some(),
            FormBody::Question(form) => form.editing,
            FormBody::Secret(_) => true,
            FormBody::Plugin => false,
        }
    }

    fn scroll_by(&mut self, delta: isize) {
        let max = self.max_scroll.get();
        self.scroll = self.scroll.min(max).saturating_add_signed(delta).min(max);
    }
}

pub fn interaction_resolution_message(resolution: &InteractionResolution) -> String {
    match resolution {
        InteractionResolution::Approval(approval) => {
            format!("Request {}", approval_outcome(approval.decision()))
        }
        _ => "Answer sent".to_owned(),
    }
}

impl AppState {
    pub fn active_interactions(&self, thread_id: &str) -> Vec<&PendingInteraction> {
        active_interactions(self.snapshot.pending_interactions(thread_id))
    }

    pub fn interactions_loaded(&self, thread_id: &str) -> bool {
        self.snapshot
            .interactions
            .get(thread_id)
            .is_some_and(|resource| resource.data.is_some())
    }

    pub fn interaction_panel_visible(&self, thread_id: &str) -> bool {
        if self.interactions_loaded(thread_id) {
            !self.active_interactions(thread_id).is_empty()
        } else {
            self.sidebar_entry(thread_id)
                .is_some_and(|entry| entry.has_pending_interaction)
        }
    }

    pub fn open_interaction_panel(&self) -> bool {
        self.open_thread
            .as_deref()
            .is_some_and(|thread_id| self.interaction_panel_visible(thread_id))
    }

    pub fn active_interaction(&self, thread_id: &str) -> Option<&PendingInteraction> {
        self.active_interactions(thread_id).into_iter().next()
    }

    pub fn active_form(&self) -> Option<&InteractionForm> {
        let thread_id = self.open_thread.as_deref()?;
        let interaction = self.active_interaction(thread_id)?;
        self.interaction_forms.get(&interaction.id)
    }

    fn active_form_mut(&mut self) -> Option<&mut InteractionForm> {
        let thread_id = self.open_thread.clone()?;
        let id = self.active_interaction(&thread_id)?.id.clone();
        self.interaction_forms.get_mut(&id)
    }

    pub(crate) fn sync_interactions(&mut self) {
        let snapshot = self.snapshot.clone();
        self.interaction_forms.retain(|id, form| {
            let Some(resource) = snapshot.interactions.get(&form.thread_id) else {
                return true;
            };
            let Some(items) = resource.data.as_deref() else {
                return true;
            };
            active_interactions(items)
                .iter()
                .any(|interaction| &interaction.id == id)
        });
        let Some(thread_id) = self.open_thread.clone() else {
            return;
        };
        for interaction in active_interactions(snapshot.pending_interactions(&thread_id)) {
            self.interaction_forms
                .entry(interaction.id.clone())
                .or_insert_with(|| InteractionForm::new(interaction));
        }
        let visible = self.interaction_panel_visible(&thread_id);
        if visible && self.focus == Focus::Composer {
            self.focus = Focus::Interaction;
        } else if !visible && self.focus == Focus::Interaction {
            self.focus = if self.composer_available() {
                Focus::Composer
            } else {
                Focus::Main
            };
        }
    }

    pub fn waiting_threads(&self) -> Vec<String> {
        let mut ordered: Vec<String> = self
            .rows
            .iter()
            .filter_map(SidebarRow::thread)
            .map(|row| row.id.clone())
            .collect();
        for entry in self.snapshot.sidebar_threads() {
            if !ordered.contains(&entry.thread.id) {
                ordered.push(entry.thread.id.clone());
            }
        }
        ordered
            .into_iter()
            .filter(|id| {
                self.sidebar_entry(id).is_some_and(|entry| {
                    entry.has_pending_interaction && !entry.thread.is_archived()
                })
            })
            .collect()
    }

    pub(crate) fn jump_to_pending(&mut self) -> Vec<Effect> {
        if let Some(open) = self.open_thread.clone()
            && self.interaction_panel_visible(&open)
            && self.focus != Focus::Interaction
        {
            self.focus = Focus::Interaction;
            return Vec::new();
        }
        let waiting = self.waiting_threads();
        let current = self.open_thread.clone();
        let start = current
            .as_ref()
            .and_then(|id| waiting.iter().position(|candidate| candidate == id))
            .map_or(0, |index| index + 1);
        let next = (0..waiting.len())
            .map(|offset| &waiting[(start + offset) % waiting.len()])
            .find(|id| Some(*id) != current.as_ref())
            .cloned();
        match next {
            Some(thread_id) => {
                let effects = self.open(thread_id);
                self.focus = Focus::Interaction;
                effects
            }
            None => {
                self.toast("No requests are waiting for you", ToastLevel::Info);
                Vec::new()
            }
        }
    }

    fn writable(&mut self) -> bool {
        if self.read_only {
            self.toast("Read-only mode: cannot answer request", ToastLevel::Error);
        }
        !self.read_only
    }

    fn decide(&mut self, decision: ApprovalDecision, follow_up: Option<String>) -> Vec<Effect> {
        let Some(thread_id) = self.open_thread.clone() else {
            return Vec::new();
        };
        let Some(interaction) = self.active_interaction(&thread_id).cloned() else {
            return Vec::new();
        };
        let available = match &interaction.payload {
            InteractionPayload::Approval(payload) => {
                ordered_decisions(&payload.available_decisions)
            }
            _ => Vec::new(),
        };
        if !available.contains(&decision) {
            self.toast(
                "That choice is not offered for this request",
                ToastLevel::Info,
            );
            return Vec::new();
        }
        if !self.writable() {
            return Vec::new();
        }
        let resolution = build_approval_resolution(&interaction, decision);
        if let Some(text) = follow_up.filter(|text| !text.trim().is_empty()) {
            self.interaction_follow_ups
                .insert(interaction.id.clone(), text.trim().to_owned());
        }
        if let Some(form) = self.interaction_forms.get_mut(&interaction.id) {
            form.submitting = true;
            form.error = None;
            if let FormBody::Approval(approval) = &mut form.body {
                approval.reason = None;
            }
        }
        vec![Effect::Mutate(Mutation::ResolveInteraction {
            thread_id,
            interaction_id: interaction.id,
            resolution: Box::new(resolution),
        })]
    }

    fn cancel_interaction(&mut self) -> Vec<Effect> {
        let Some(form) = self.active_form() else {
            return Vec::new();
        };
        let cancel = match &form.kind {
            RequestKind::Question { cancel, .. }
            | RequestKind::Secret { cancel, .. }
            | RequestKind::Plugin { cancel, .. } => *cancel,
            RequestKind::Approval(_) | RequestKind::PlanReview { .. } => {
                self.toast("Choose an answer, or Esc to leave it", ToastLevel::Info);
                return Vec::new();
            }
        };
        let thread_id = form.thread_id.clone();
        let interaction_id = form.interaction_id.clone();
        match cancel {
            CancelAction::StopTurn => {
                if self.read_only {
                    self.toast("Read-only mode: cannot stop", ToastLevel::Error);
                    return Vec::new();
                }
                self.stop_requested.insert(thread_id.clone());
                vec![Effect::Mutate(Mutation::Stop { thread_id })]
            }
            CancelAction::CancelRequest => {
                if !self.writable() {
                    return Vec::new();
                }
                if let Some(form) = self.active_form_mut() {
                    form.submitting = true;
                    form.error = None;
                }
                vec![Effect::Mutate(Mutation::CancelInteraction {
                    thread_id,
                    interaction_id,
                })]
            }
        }
    }

    fn submit_questions(&mut self) -> Vec<Effect> {
        let Some(form) = self.active_form() else {
            return Vec::new();
        };
        let (
            RequestKind::Question {
                questions, channel, ..
            },
            FormBody::Question(body),
        ) = (&form.kind, &form.body)
        else {
            return Vec::new();
        };
        let unanswered = questions
            .iter()
            .zip(&body.answers)
            .position(|(question, state)| !is_question_answered(question, state));
        if let Some(index) = unanswered {
            if let Some(form) = self.active_form_mut()
                && let FormBody::Question(body) = &mut form.body
            {
                body.current = index;
                body.cursor = 0;
                body.editing = false;
            }
            self.toast("Answer every question first", ToastLevel::Info);
            return Vec::new();
        }
        let answers = build_question_answers(questions, &body.answers);
        let channel = *channel;
        let thread_id = form.thread_id.clone();
        let interaction_id = form.interaction_id.clone();
        if !self.writable() {
            return Vec::new();
        }
        if let Some(form) = self.active_form_mut() {
            form.submitting = true;
            form.error = None;
        }
        let mutation = match channel {
            AnswerChannel::Resolve => Mutation::ResolveInteraction {
                thread_id,
                interaction_id,
                resolution: Box::new(InteractionResolution::user_answer(answers)),
            },
            AnswerChannel::Respond => Mutation::RespondInteraction {
                thread_id,
                interaction_id,
                value: InteractionValue {
                    value: serde_json::json!({ "answers": answers }),
                    sensitive: false,
                },
            },
        };
        vec![Effect::Mutate(mutation)]
    }

    fn submit_secrets(&mut self) -> Vec<Effect> {
        let Some(form) = self.active_form() else {
            return Vec::new();
        };
        let (RequestKind::Secret { payload, .. }, FormBody::Secret(body)) =
            (&form.kind, &form.body)
        else {
            return Vec::new();
        };
        if let Err(error) = validate_secret_values(body.inputs.iter().map(LineInput::text)) {
            let message = error.message().to_owned();
            if let Some(form) = self.active_form_mut() {
                form.error = Some(message);
            }
            return Vec::new();
        }
        let values: BTreeMap<String, String> = payload
            .fields
            .iter()
            .zip(&body.inputs)
            .map(|(field, input)| (field.name.clone(), input.text().to_owned()))
            .collect();
        let thread_id = form.thread_id.clone();
        let interaction_id = form.interaction_id.clone();
        if !self.writable() {
            return Vec::new();
        }
        let Ok(value) = serde_json::to_value(SecretRequestResponse { values }) else {
            return Vec::new();
        };
        if let Some(form) = self.active_form_mut() {
            form.submitting = true;
            form.error = None;
        }
        vec![Effect::Mutate(Mutation::RespondInteraction {
            thread_id,
            interaction_id,
            value: InteractionValue {
                value,
                sensitive: true,
            },
        })]
    }

    fn open_in_web(&mut self, copy: bool) -> Vec<Effect> {
        let Some(thread_id) = self.open_thread.clone() else {
            return Vec::new();
        };
        let url = thread_web_url(&self.server.url, &thread_id);
        if copy {
            vec![Effect::Copy(url)]
        } else {
            vec![Effect::OpenExternal(url)]
        }
    }
}

fn is_ctrl(key: &KeyEvent, ch: char) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char(ch)
}

fn fallback(state: &mut AppState, key: KeyEvent) -> Vec<Effect> {
    match state.keymap.lookup(&key) {
        Some(
            command @ (Command::FocusNext
            | Command::FocusPrevious
            | Command::Jump(_)
            | Command::Help
            | Command::Quit
            | Command::NextPending
            | Command::SidebarToggle
            | Command::PanelToggle
            | Command::ThreadNext
            | Command::ThreadPrevious
            | Command::Reload),
        ) => crate::app::run_command(state, command),
        _ => Vec::new(),
    }
}

fn leave(state: &mut AppState) -> Vec<Effect> {
    state.focus = Focus::Main;
    Vec::new()
}

pub(crate) fn handle_interaction_key(state: &mut AppState, key: KeyEvent) -> Vec<Effect> {
    let Some(form) = state.active_form() else {
        if key.code == KeyCode::Esc {
            return leave(state);
        }
        return fallback(state, key);
    };
    if form.is_busy() {
        return match key.code {
            KeyCode::Esc => leave(state),
            _ => fallback(state, key),
        };
    }
    if is_ctrl(&key, 'x') {
        return state.cancel_interaction();
    }
    match form.body {
        FormBody::Approval(_) => approval_key(state, key),
        FormBody::Question(_) => question_key(state, key),
        FormBody::Secret(_) => secret_key(state, key),
        FormBody::Plugin => plugin_key(state, key),
    }
}

fn scroll_key(state: &mut AppState, key: &KeyEvent) -> bool {
    let delta = match key.code {
        KeyCode::Char('j') | KeyCode::Down => 1,
        KeyCode::Char('k') | KeyCode::Up => -1,
        KeyCode::PageDown => 10,
        KeyCode::PageUp => -10,
        _ => return false,
    };
    if let Some(form) = state.active_form_mut() {
        form.scroll_by(delta);
    }
    true
}

fn approval_key(state: &mut AppState, key: KeyEvent) -> Vec<Effect> {
    let Some(form) = state.active_form_mut() else {
        return Vec::new();
    };
    let FormBody::Approval(approval) = &mut form.body else {
        return Vec::new();
    };
    if let Some(input) = approval.reason.as_mut() {
        return match input.handle_key(key) {
            InputOutcome::Submit => {
                let text = input.text().to_owned();
                state.decide(ApprovalDecision::Deny, Some(text))
            }
            InputOutcome::Cancel => {
                approval.reason = None;
                Vec::new()
            }
            _ => Vec::new(),
        };
    }
    let available = match &form.kind {
        RequestKind::Approval(payload) => ordered_decisions(&payload.available_decisions),
        RequestKind::PlanReview { approval, .. } => {
            ordered_decisions(&approval.available_decisions)
        }
        _ => Vec::new(),
    };
    match key.code {
        KeyCode::Char('y') => state.decide(ApprovalDecision::AllowOnce, None),
        KeyCode::Char('s') => state.decide(ApprovalDecision::AllowForSession, None),
        KeyCode::Char('d') => state.decide(ApprovalDecision::Deny, None),
        KeyCode::Char(ch @ '1'..='9') => {
            match available.get(usize::from(ch as u8 - b'1')).copied() {
                Some(decision) => state.decide(decision, None),
                None => Vec::new(),
            }
        }
        KeyCode::Char('r' | 'f') => {
            if available.contains(&ApprovalDecision::Deny) {
                approval.reason = Some(LineInput::default());
            } else {
                state.toast("This request cannot be denied", ToastLevel::Info);
            }
            Vec::new()
        }
        KeyCode::Char('e') => {
            approval.expanded = !approval.expanded;
            Vec::new()
        }
        KeyCode::Esc => leave(state),
        _ if scroll_key(state, &key) => Vec::new(),
        _ => fallback(state, key),
    }
}

fn question_key(state: &mut AppState, key: KeyEvent) -> Vec<Effect> {
    let Some(form) = state.active_form_mut() else {
        return Vec::new();
    };
    let (RequestKind::Question { questions, .. }, FormBody::Question(body)) =
        (&form.kind, &mut form.body)
    else {
        return Vec::new();
    };
    let count = questions.len();
    if count == 0 {
        return fallback(state, key);
    }
    let current = body.current.min(count - 1);
    let question = questions[current].clone();
    let last = current + 1 == count;
    let goto = |body: &mut QuestionForm, index: usize| {
        body.current = index;
        body.cursor = 0;
        body.editing = questions[index].options.is_empty();
    };

    if body.editing {
        let input = &mut body.inputs[current];
        match key.code {
            KeyCode::Tab => {
                goto(body, (current + 1) % count);
                return Vec::new();
            }
            KeyCode::BackTab => {
                goto(body, (current + count - 1) % count);
                return Vec::new();
            }
            KeyCode::Up if !question.options.is_empty() => {
                body.editing = false;
                body.cursor = question.row_count().saturating_sub(1);
                return Vec::new();
            }
            KeyCode::Char(ch)
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && utf16_len(input.text()) + ch.len_utf16()
                        > USER_QUESTION_MAX_FREE_TEXT_LENGTH =>
            {
                return Vec::new();
            }
            _ => {}
        }
        match input.handle_key(key) {
            InputOutcome::Submit => {}
            InputOutcome::Cancel => {
                if question.options.is_empty() {
                    return leave(state);
                }
                body.editing = false;
                return Vec::new();
            }
            InputOutcome::Edited => {
                body.answers[current].other_text = input.text().to_owned();
                return Vec::new();
            }
            InputOutcome::Moved => return Vec::new(),
            InputOutcome::Ignored => return fallback(state, key),
        }
        if !is_question_answered(&question, &body.answers[current]) {
            return Vec::new();
        }
        if last {
            return state.submit_questions();
        }
        goto(body, current + 1);
        return Vec::new();
    }

    let rows = question.row_count();
    let choose = |body: &mut QuestionForm, choice: ShortcutChoice| match choice {
        ShortcutChoice::Option(value) => {
            toggle_option(&question, &mut body.answers[current], &value)
        }
        ShortcutChoice::Other => {
            select_other(&question, &mut body.answers[current]);
            body.editing = body.answers[current].other_selected;
        }
    };
    match key.code {
        KeyCode::Char(ch @ '1'..='9') => {
            let index = usize::from(ch as u8 - b'1');
            if let Some(choice) = shortcut_choice(&question, index) {
                body.cursor = index;
                choose(body, choice);
            }
            Vec::new()
        }
        KeyCode::Char('j') | KeyCode::Down => {
            if body.cursor + 1 < rows {
                body.cursor += 1;
            } else if body.answers[current].other_selected {
                body.editing = true;
            }
            Vec::new()
        }
        KeyCode::Char('k') | KeyCode::Up => {
            body.cursor = body.cursor.saturating_sub(1);
            Vec::new()
        }
        KeyCode::Char(' ') | KeyCode::Char('x') => {
            if let Some(choice) = shortcut_choice(&question, body.cursor) {
                choose(body, choice);
            }
            Vec::new()
        }
        KeyCode::Tab | KeyCode::Right | KeyCode::Char('l') => {
            goto(body, (current + 1) % count);
            Vec::new()
        }
        KeyCode::BackTab | KeyCode::Left | KeyCode::Char('h') => {
            goto(body, (current + count - 1) % count);
            Vec::new()
        }
        KeyCode::Enter => {
            if !is_question_answered(&question, &body.answers[current]) {
                if let Some(choice) = shortcut_choice(&question, body.cursor) {
                    choose(body, choice);
                }
                if body.editing || !is_question_answered(&question, &body.answers[current]) {
                    return Vec::new();
                }
            }
            if last {
                return state.submit_questions();
            }
            goto(body, current + 1);
            Vec::new()
        }
        KeyCode::Esc => leave(state),
        _ => fallback(state, key),
    }
}

fn secret_key(state: &mut AppState, key: KeyEvent) -> Vec<Effect> {
    let Some(form) = state.active_form_mut() else {
        return Vec::new();
    };
    let FormBody::Secret(body) = &mut form.body else {
        return Vec::new();
    };
    let count = body.inputs.len();
    if count == 0 {
        return fallback(state, key);
    }
    let field = body.field.min(count - 1);
    match key.code {
        KeyCode::Tab | KeyCode::Down => {
            body.field = (field + 1) % count;
            return Vec::new();
        }
        KeyCode::BackTab | KeyCode::Up => {
            body.field = (field + count - 1) % count;
            return Vec::new();
        }
        _ if is_ctrl(&key, 't') => {
            body.revealed = !body.revealed;
            return Vec::new();
        }
        _ => {}
    }
    match body.inputs[field].handle_key(key) {
        InputOutcome::Submit => {
            let next_empty = (0..count)
                .map(|offset| (field + 1 + offset) % count)
                .find(|index| body.inputs[*index].text().is_empty());
            match next_empty {
                Some(index) if index != field || body.inputs[field].text().is_empty() => {
                    body.field = index;
                    Vec::new()
                }
                _ => state.submit_secrets(),
            }
        }
        InputOutcome::Cancel => leave(state),
        InputOutcome::Edited => {
            form.error = None;
            Vec::new()
        }
        InputOutcome::Moved => Vec::new(),
        InputOutcome::Ignored => fallback(state, key),
    }
}

fn plugin_key(state: &mut AppState, key: KeyEvent) -> Vec<Effect> {
    match key.code {
        KeyCode::Char('o') => state.open_in_web(false),
        KeyCode::Char('y') => state.open_in_web(true),
        KeyCode::Char('x') => state.cancel_interaction(),
        KeyCode::Esc => leave(state),
        _ if scroll_key(state, &key) => Vec::new(),
        _ => fallback(state, key),
    }
}

pub(crate) fn interaction_paste(state: &mut AppState, text: &str) {
    let Some(form) = state.active_form_mut() else {
        return;
    };
    if form.is_busy() {
        return;
    }
    let current_question = match &form.kind {
        RequestKind::Question { .. } => {
            let FormBody::Question(body) = &form.body else {
                return;
            };
            Some(body.current)
        }
        _ => None,
    };
    match &mut form.body {
        FormBody::Approval(ApprovalForm {
            reason: Some(input),
            ..
        }) => {
            for ch in text.chars().filter(|ch| !ch.is_control()) {
                input.insert(ch);
            }
        }
        FormBody::Question(body) if body.editing => {
            let index = current_question.unwrap_or(0);
            let Some(input) = body.inputs.get_mut(index) else {
                return;
            };
            for ch in text.chars().filter(|ch| *ch != '\r') {
                if utf16_len(input.text()) + ch.len_utf16() > USER_QUESTION_MAX_FREE_TEXT_LENGTH {
                    break;
                }
                input.insert(if ch == '\n' { ' ' } else { ch });
            }
            body.answers[index].other_text = input.text().to_owned();
        }
        FormBody::Secret(body) => {
            let field = body.field;
            if let Some(input) = body.inputs.get_mut(field) {
                for ch in text.trim_end_matches(['\n', '\r']).chars() {
                    input.insert(ch);
                }
            }
            form.error = None;
        }
        _ => {}
    }
}

pub(crate) fn interaction_mutation_finished(
    state: &mut AppState,
    mutation: Mutation,
    result: Result<MutationReply, String>,
) -> Vec<Effect> {
    let thread_id = mutation.thread_id().to_owned();
    let interaction_id = mutation.interaction_id().unwrap_or_default().to_owned();
    let follow_up = state.interaction_follow_ups.remove(&interaction_id);
    let form = state.interaction_forms.get_mut(&interaction_id);
    match result {
        Ok(_) => {
            if let Some(form) = form {
                form.submitting = false;
                form.submitted = true;
                form.error = None;
                if let FormBody::Secret(body) = &mut form.body {
                    body.clear();
                }
            }
            let message = match &mutation {
                Mutation::ResolveInteraction { resolution, .. } => {
                    interaction_resolution_message(resolution)
                }
                other => other.success_message().to_owned(),
            };
            state.toast(message, ToastLevel::Info);
            let mut effects = vec![
                Effect::Refresh(ResourceKey::Interactions(thread_id.clone())),
                Effect::Refresh(ResourceKey::Thread(thread_id.clone())),
                Effect::RefreshSidebar,
            ];
            if let Some(text) = follow_up {
                effects.extend(state.send_follow_up_text(&thread_id, text));
            }
            effects
        }
        Err(error) => {
            if let Some(form) = form {
                form.submitting = false;
                form.error = Some(error.clone());
            }
            state.toast(
                format!("Failed to {}: {error}", mutation.failure_verb()),
                ToastLevel::Error,
            );
            vec![Effect::Refresh(ResourceKey::Interactions(thread_id))]
        }
    }
}
