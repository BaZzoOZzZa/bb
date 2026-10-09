mod support;

use std::sync::Arc;
use std::time::Duration;

use bb_store::{Resource, StoreSnapshot};
use bb_tui::api::{Mutation, MutationReply};
use bb_tui::app::{Action, AppState, Effect, Focus, Mode, update};
use bb_tui::composer::DRAFT_SAVE_DELAY;
use bb_wire::prompt::{
    ExecutionInputSource, PermissionMode, PromptHistoryEntry, PromptInput, ReasoningLevel,
    ResolvedThreadExecutionOptions, ThreadExecutionSource,
};
use bb_wire::queue::{
    QueuedMessageWaitingOn, SendMessageMode, SendQueuedMessageMode, ThreadQueuedMessage,
};
use bb_wire::system::{
    AvailableModel, ModelReasoningEffort, ProviderCapabilities, ProviderInfo,
    SystemExecutionOptionsResponse,
};
use bb_wire::thread::ThreadContextWindowUsage;
use bb_wire::thread::{ThreadDisplayStatus, ThreadStatus};
use bb_wire::timeline::{
    ActivePromptMode, GoalStatus, ModelFallback, PendingTodoItem, PendingTodoStatus, PendingTodos,
    ThreadGoal,
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use support::*;

const IDLE: &str = "thr_other";
const WIDTH: u16 = 140;
const HEIGHT: u16 = 30;

fn key(state: &mut AppState, code: KeyCode) -> Vec<Effect> {
    update(state, Action::Key(KeyEvent::from(code)))
}

fn chord(state: &mut AppState, code: KeyCode, modifiers: KeyModifiers) -> Vec<Effect> {
    update(state, Action::Key(KeyEvent::new(code, modifiers)))
}

fn type_text(state: &mut AppState, text: &str) {
    for ch in text.chars() {
        key(state, KeyCode::Char(ch));
    }
}

fn model(id: &str, name: &str, default: bool) -> AvailableModel {
    AvailableModel {
        id: id.into(),
        model: id.into(),
        display_name: name.into(),
        supported_reasoning_efforts: [
            ReasoningLevel::Low,
            ReasoningLevel::Medium,
            ReasoningLevel::High,
        ]
        .into_iter()
        .map(|level| ModelReasoningEffort {
            reasoning_effort: level,
            description: String::new(),
        })
        .collect(),
        default_reasoning_effort: ReasoningLevel::Medium,
        is_default: default,
        ..AvailableModel::default()
    }
}

fn execution_options() -> SystemExecutionOptionsResponse {
    SystemExecutionOptionsResponse {
        providers: vec![ProviderInfo {
            id: "codex".into(),
            capabilities: ProviderCapabilities {
                permission_modes: vec![
                    PermissionMode::AcceptEdits,
                    PermissionMode::Auto,
                    PermissionMode::Full,
                ],
                ..ProviderCapabilities::default()
            },
            ..ProviderInfo::default()
        }],
        permission_ceiling: PermissionMode::Auto,
        models: vec![
            model("gpt-5", "GPT-5", true),
            model("gpt-5-mini", "GPT-5 Mini", false),
        ],
        selected_only_models: Vec::new(),
        model_load_error: None,
    }
}

fn defaults() -> ResolvedThreadExecutionOptions {
    ResolvedThreadExecutionOptions {
        model: "gpt-5".into(),
        service_tier: String::new(),
        reasoning_level: ReasoningLevel::High,
        permission_mode: PermissionMode::Auto,
        source: ThreadExecutionSource::TurnRequested,
        seq: None,
    }
}

fn with_status(status: ThreadStatus, display: ThreadDisplayStatus) -> AppState {
    let mut sidebar = sample_sidebar();
    for project in &mut sidebar.projects {
        for entry in &mut project.threads {
            if entry.thread.id == IDLE {
                entry.thread.status = status;
                entry.runtime.display_status = display;
            }
        }
    }
    let mut state = loaded_state(sidebar);
    open(&mut state);
    load_execution(&mut state);
    state
}

fn open(state: &mut AppState) -> Vec<Effect> {
    let index = state
        .rows
        .iter()
        .position(|row| row.thread().is_some_and(|thread| thread.id == IDLE))
        .expect("thread row");
    state.selected = index;
    key(state, KeyCode::Enter)
}

fn load_execution(state: &mut AppState) {
    update(
        state,
        Action::ExecutionOptionsLoaded {
            thread_id: IDLE.into(),
            result: Ok(execution_options()),
        },
    );
    update(
        state,
        Action::DefaultsLoaded {
            thread_id: IDLE.into(),
            result: Ok(Some(defaults())),
        },
    );
}

fn idle() -> AppState {
    with_status(ThreadStatus::Idle, ThreadDisplayStatus::Idle)
}

fn active() -> AppState {
    with_status(ThreadStatus::Active, ThreadDisplayStatus::Active)
}

fn focus_composer(state: &mut AppState) {
    key(state, KeyCode::Char('c'));
    assert_eq!(state.focus, Focus::Composer);
}

fn mutations(effects: &[Effect]) -> Vec<Mutation> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Mutate(mutation) => Some(mutation.clone()),
            _ => None,
        })
        .collect()
}

fn composer_text(state: &AppState) -> String {
    state.composer().expect("composer").text()
}

fn screen(state: &AppState) -> String {
    render_lines(state, WIDTH, HEIGHT).join("\n")
}

fn queued(id: &str, text: &str) -> ThreadQueuedMessage {
    ThreadQueuedMessage {
        id: id.into(),
        thread_id: IDLE.into(),
        content: vec![PromptInput::text(text)],
        editable: true,
        updated_at: 77,
        waiting_on: Some(QueuedMessageWaitingOn::ThreadBusy),
        ..ThreadQueuedMessage::default()
    }
}

fn with_queue(state: &mut AppState, messages: Vec<ThreadQueuedMessage>) {
    let mut snapshot: StoreSnapshot = (*state.snapshot).clone();
    snapshot.queued_messages.insert(
        IDLE.into(),
        Resource {
            data: Some(Arc::new(messages)),
            ..Resource::default()
        },
    );
    snapshot.version += 1;
    update(state, Action::Snapshot(Arc::new(snapshot)));
}

fn finish(
    state: &mut AppState,
    mutation: Mutation,
    result: Result<MutationReply, String>,
) -> Vec<Effect> {
    update(state, Action::MutationFinished { mutation, result })
}

#[test]
fn opening_a_thread_loads_draft_history_and_execution_options() {
    let mut state = loaded_state(sample_sidebar());
    let effects = open(&mut state);
    assert!(effects.contains(&Effect::LoadDraft(IDLE.into())));
    assert!(effects.contains(&Effect::FetchPromptHistory(IDLE.into())));
    assert!(effects.contains(&Effect::FetchDefaults(IDLE.into())));
    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::FetchExecutionOptions { thread_id, query }
            if thread_id == IDLE && query.provider_id.as_deref() == Some("codex")
    )));
    let again = open(&mut state);
    assert!(
        !again
            .iter()
            .any(|effect| matches!(effect, Effect::LoadDraft(_) | Effect::FetchDefaults(_)))
    );
}

#[test]
fn enter_sends_idle_follow_up_with_execution_selection() {
    let mut state = idle();
    focus_composer(&mut state);
    type_text(&mut state, "Fix the tests");
    let effects = key(&mut state, KeyCode::Enter);
    let sent = mutations(&effects);
    let [Mutation::SendMessage { thread_id, request }] = sent.as_slice() else {
        panic!("expected send, got {effects:?}");
    };
    assert_eq!(thread_id, IDLE);
    assert_eq!(request.mode, SendMessageMode::QueueIfActive);
    assert_eq!(request.input, vec![PromptInput::text("Fix the tests")]);
    assert_eq!(request.model.as_deref(), Some("gpt-5"));
    assert_eq!(request.reasoning_level, Some(ReasoningLevel::High));
    assert_eq!(request.permission_mode, Some(PermissionMode::Auto));
    assert_eq!(request.execution_input_sources, Some(Default::default()));
    assert!(effects.contains(&Effect::SaveDraft {
        thread_id: IDLE.into(),
        raw: None
    }));
    assert_eq!(composer_text(&state), "");
}

#[test]
fn picked_options_become_explicit_execution_inputs() {
    let mut state = idle();
    chord(&mut state, KeyCode::Char('m'), KeyModifiers::ALT);
    let Mode::Picker(picker) = &state.mode else {
        panic!("model picker expected");
    };
    assert_eq!(picker.items.len(), 2);
    assert!(picker.items[0].current);
    key(&mut state, KeyCode::Char('2'));
    assert_eq!(state.mode, Mode::Normal);
    chord(&mut state, KeyCode::Char('r'), KeyModifiers::ALT);
    chord(&mut state, KeyCode::Char('p'), KeyModifiers::ALT);
    let Mode::Picker(picker) = &state.mode else {
        panic!("permission picker expected");
    };
    assert!(picker.items[2].disabled);
    key(&mut state, KeyCode::Char('3'));
    assert!(matches!(state.mode, Mode::Picker(_)));
    key(&mut state, KeyCode::Char('1'));
    focus_composer(&mut state);
    type_text(&mut state, "go");
    let sent = mutations(&key(&mut state, KeyCode::Enter));
    let [Mutation::SendMessage { request, .. }] = sent.as_slice() else {
        panic!("expected send");
    };
    assert_eq!(request.model.as_deref(), Some("gpt-5-mini"));
    assert_eq!(request.reasoning_level, Some(ReasoningLevel::Low));
    assert_eq!(request.permission_mode, Some(PermissionMode::AcceptEdits));
    let sources = request.execution_input_sources.clone().unwrap();
    assert_eq!(sources.model, Some(ExecutionInputSource::Explicit));
    assert_eq!(
        sources.reasoning_level,
        Some(ExecutionInputSource::Explicit)
    );
}

#[test]
fn active_thread_queues_on_enter_and_steers_with_ctrl_s() {
    let mut state = active();
    focus_composer(&mut state);
    type_text(&mut state, "after this");
    let sent = mutations(&key(&mut state, KeyCode::Enter));
    let [Mutation::QueueMessage { request, .. }] = sent.as_slice() else {
        panic!("expected queue, got {sent:?}");
    };
    assert_eq!(request.input, vec![PromptInput::text("after this")]);
    finish(&mut state, sent[0].clone(), Ok(MutationReply::Done));

    type_text(&mut state, "change course");
    let sent = mutations(&chord(
        &mut state,
        KeyCode::Char('s'),
        KeyModifiers::CONTROL,
    ));
    let [Mutation::SendMessage { request, .. }] = sent.as_slice() else {
        panic!("expected steer, got {sent:?}");
    };
    assert_eq!(request.mode, SendMessageMode::SteerIfActive);
    finish(&mut state, sent[0].clone(), Ok(MutationReply::Done));

    with_queue(
        &mut state,
        vec![queued("q1", "first"), queued("q2", "second")],
    );
    let sent = mutations(&chord(
        &mut state,
        KeyCode::Char('s'),
        KeyModifiers::CONTROL,
    ));
    assert_eq!(
        sent,
        vec![Mutation::SendQueued {
            thread_id: IDLE.into(),
            queued_message_id: "q1".into(),
            mode: SendQueuedMessageMode::Steer,
        }]
    );
    assert!(
        mutations(&chord(
            &mut state,
            KeyCode::Char('s'),
            KeyModifiers::CONTROL
        ))
        .is_empty()
    );
}

#[test]
fn submit_mode_toggle_makes_enter_steer() {
    let mut state = active();
    chord(&mut state, KeyCode::Char('t'), KeyModifiers::ALT);
    assert!(state.enter_steers);
    focus_composer(&mut state);
    type_text(&mut state, "now");
    let sent = mutations(&key(&mut state, KeyCode::Enter));
    let [Mutation::SendMessage { request, .. }] = sent.as_slice() else {
        panic!("expected steer");
    };
    assert_eq!(request.mode, SendMessageMode::SteerIfActive);
    finish(&mut state, sent[0].clone(), Ok(MutationReply::Done));
    type_text(&mut state, "later");
    let sent = mutations(&chord(
        &mut state,
        KeyCode::Char('s'),
        KeyModifiers::CONTROL,
    ));
    assert!(matches!(sent.as_slice(), [Mutation::QueueMessage { .. }]));

    let mut idle = idle();
    chord(&mut idle, KeyCode::Char('t'), KeyModifiers::ALT);
    focus_composer(&mut idle);
    type_text(&mut idle, "hello");
    let sent = mutations(&key(&mut idle, KeyCode::Enter));
    let [Mutation::SendMessage { request, .. }] = sent.as_slice() else {
        panic!("expected plain send");
    };
    assert_eq!(request.mode, SendMessageMode::QueueIfActive);
}

#[test]
fn double_escape_and_ctrl_c_stop_the_turn() {
    let mut state = active();
    focus_composer(&mut state);
    assert!(mutations(&key(&mut state, KeyCode::Esc)).is_empty());
    assert!(screen(&state).contains("Esc again to stop"));
    assert_eq!(
        mutations(&key(&mut state, KeyCode::Esc)),
        vec![Mutation::Stop {
            thread_id: IDLE.into()
        }]
    );
    assert!(state.stop_requested(IDLE));
    assert!(screen(&state).contains("stopping"));
    finish(
        &mut state,
        Mutation::Stop {
            thread_id: IDLE.into(),
        },
        Ok(MutationReply::Done),
    );

    type_text(&mut state, "draft");
    chord(&mut state, KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert_eq!(composer_text(&state), "");
    assert!(!state.quit);
    assert_eq!(
        mutations(&chord(
            &mut state,
            KeyCode::Char('c'),
            KeyModifiers::CONTROL
        )),
        vec![Mutation::Stop {
            thread_id: IDLE.into()
        }]
    );

    let mut idle = idle();
    focus_composer(&mut idle);
    key(&mut idle, KeyCode::Esc);
    assert_eq!(idle.focus, Focus::Main);
    assert!(mutations(&key(&mut idle, KeyCode::Char('S'))).is_empty());
    assert!(
        idle.toasts
            .iter()
            .any(|toast| toast.message == "Nothing to stop")
    );
}

#[test]
fn blocked_modes_keep_the_draft() {
    let mut state = idle();
    let mut snapshot: StoreSnapshot = (*state.snapshot).clone();
    let sidebar = Arc::make_mut(snapshot.sidebar.data.as_mut().unwrap());
    for project in &mut sidebar.projects {
        for entry in &mut project.threads {
            if entry.thread.id == IDLE {
                entry.has_pending_interaction = true;
            }
        }
    }
    update(&mut state, Action::Snapshot(Arc::new(snapshot)));
    focus_composer(&mut state);
    type_text(&mut state, "hello");
    assert!(mutations(&key(&mut state, KeyCode::Enter)).is_empty());
    assert_eq!(composer_text(&state), "hello");
    assert!(screen(&state).contains("Waiting for you"));

    let mut loading = loaded_state(sample_sidebar());
    open(&mut loading);
    focus_composer(&mut loading);
    type_text(&mut loading, "x");
    assert!(mutations(&key(&mut loading, KeyCode::Enter)).is_empty());
    assert!(
        loading
            .toasts
            .iter()
            .any(|toast| toast.message.contains("still loading"))
    );

    let mut read_only = idle_read_only();
    focus_composer(&mut read_only);
    type_text(&mut read_only, "keep me");
    assert!(key(&mut read_only, KeyCode::Enter).is_empty());
    assert_eq!(composer_text(&read_only), "keep me");
}

fn idle_read_only() -> AppState {
    let mut state = idle();
    state.read_only = true;
    state
}

#[test]
fn failed_send_restores_the_draft_and_success_extends_history() {
    let mut state = idle();
    focus_composer(&mut state);
    type_text(&mut state, "try this");
    let sent = mutations(&key(&mut state, KeyCode::Enter));
    let effects = finish(&mut state, sent[0].clone(), Err("boom".into()));
    assert_eq!(composer_text(&state), "try this");
    assert!(
        state
            .toasts
            .iter()
            .any(|toast| toast.message == "Failed to send message: boom")
    );
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::SaveDraft { raw: Some(_), .. }))
    );

    let sent = mutations(&key(&mut state, KeyCode::Enter));
    let effects = finish(&mut state, sent[0].clone(), Ok(MutationReply::Done));
    assert!(effects.contains(&Effect::FetchPromptHistory(IDLE.into())));
    assert_eq!(composer_text(&state), "");
    key(&mut state, KeyCode::Up);
    assert_eq!(composer_text(&state), "try this");
}

#[test]
fn prompt_history_browses_server_entries() {
    let mut state = idle();
    update(
        &mut state,
        Action::PromptHistoryLoaded {
            thread_id: IDLE.into(),
            result: Ok(vec![
                PromptHistoryEntry {
                    id: "h2".into(),
                    created_at: 2,
                    input: vec![PromptInput::text("second\nline")],
                },
                PromptHistoryEntry {
                    id: "h1".into(),
                    created_at: 1,
                    input: vec![PromptInput::text("first")],
                },
            ]),
        },
    );
    focus_composer(&mut state);
    key(&mut state, KeyCode::Up);
    assert_eq!(composer_text(&state), "second\nline");
    assert!(screen(&state).contains("History 1/2"));
    key(&mut state, KeyCode::Up);
    key(&mut state, KeyCode::Up);
    assert_eq!(composer_text(&state), "first");
    key(&mut state, KeyCode::Down);
    key(&mut state, KeyCode::Down);
    key(&mut state, KeyCode::Down);
    key(&mut state, KeyCode::Down);
    assert_eq!(composer_text(&state), "");
}

#[test]
fn drafts_load_once_and_save_after_a_pause() {
    let mut state = idle();
    update(
        &mut state,
        Action::DraftLoaded {
            thread_id: IDLE.into(),
            raw: Some(r#"{"text":"saved draft","attachments":[]}"#.into()),
        },
    );
    assert_eq!(composer_text(&state), "saved draft");
    focus_composer(&mut state);
    type_text(&mut state, "!");
    let soon = state.clock + Duration::from_millis(10);
    let effects = update(
        &mut state,
        Action::Tick {
            now: soon,
            now_ms: NOW_MS,
        },
    );
    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, Effect::SaveDraft { .. }))
    );
    let later = soon + DRAFT_SAVE_DELAY;
    let effects = update(
        &mut state,
        Action::Tick {
            now: later,
            now_ms: NOW_MS,
        },
    );
    assert!(effects.contains(&Effect::SaveDraft {
        thread_id: IDLE.into(),
        raw: Some(r#"{"attachments":[],"text":"saved draft!"}"#.into()),
    }));
}

#[test]
fn queue_panel_edits_deletes_and_sends_now() {
    let mut state = active();
    with_queue(
        &mut state,
        vec![queued("q1", "first"), queued("q2", "second")],
    );
    let text = screen(&state);
    assert!(text.contains("Queued · 2"), "{text}");
    assert!(
        text.contains("1. first · waiting for the current turn"),
        "{text}"
    );

    focus_composer(&mut state);
    type_text(&mut state, "my draft");
    chord(&mut state, KeyCode::Char('q'), KeyModifiers::ALT);
    assert!(matches!(state.mode, Mode::Queue { .. }));
    key(&mut state, KeyCode::Char('j'));
    assert_eq!(
        mutations(&key(&mut state, KeyCode::Char('d'))),
        vec![Mutation::DeleteQueued {
            thread_id: IDLE.into(),
            queued_message_id: "q2".into()
        }]
    );
    assert_eq!(
        mutations(&key(&mut state, KeyCode::Char('s'))),
        vec![Mutation::SendQueued {
            thread_id: IDLE.into(),
            queued_message_id: "q2".into(),
            mode: SendQueuedMessageMode::Auto
        }]
    );
    key(&mut state, KeyCode::Char('k'));
    let effects = key(&mut state, KeyCode::Char('e'));
    assert_eq!(
        mutations(&effects),
        vec![Mutation::HoldQueued {
            thread_id: IDLE.into(),
            queued_message_id: "q1".into()
        }]
    );
    assert_eq!(state.focus, Focus::Composer);
    assert_eq!(composer_text(&state), "first");
    assert!(screen(&state).contains("Editing queued message"));
    finish(
        &mut state,
        Mutation::HoldQueued {
            thread_id: IDLE.into(),
            queued_message_id: "q1".into(),
        },
        Ok(MutationReply::Lease { lease_ms: 20_000 }),
    );
    let renew_at = state.clock + Duration::from_secs(6);
    let renew = update(
        &mut state,
        Action::Tick {
            now: renew_at,
            now_ms: NOW_MS,
        },
    );
    assert!(
        mutations(&renew)
            .iter()
            .any(|mutation| matches!(mutation, Mutation::HoldQueued { .. }))
    );

    type_text(&mut state, " edited");
    let sent = mutations(&key(&mut state, KeyCode::Enter));
    let [
        Mutation::UpdateQueued {
            request,
            queued_message_id,
            ..
        },
    ] = sent.as_slice()
    else {
        panic!("expected update, got {sent:?}");
    };
    assert_eq!(queued_message_id, "q1");
    assert_eq!(request.expected_updated_at, 77);
    assert_eq!(request.input, vec![PromptInput::text("first edited")]);
    let effects = finish(&mut state, sent[0].clone(), Ok(MutationReply::Done));
    assert!(
        mutations(&effects)
            .iter()
            .any(|mutation| matches!(mutation, Mutation::ReleaseQueued { .. }))
    );
    assert_eq!(composer_text(&state), "my draft");

    chord(&mut state, KeyCode::Char('q'), KeyModifiers::ALT);
    key(&mut state, KeyCode::Enter);
    assert_eq!(composer_text(&state), "first");
    let cancel = mutations(&key(&mut state, KeyCode::Esc));
    assert_eq!(
        cancel,
        vec![Mutation::ReleaseQueued {
            thread_id: IDLE.into(),
            queued_message_id: "q1".into()
        }]
    );
    assert_eq!(composer_text(&state), "my draft");
}

#[test]
fn edit_ends_when_the_queued_message_is_sent() {
    let mut state = active();
    with_queue(&mut state, vec![queued("q1", "first")]);
    chord(&mut state, KeyCode::Char('q'), KeyModifiers::ALT);
    key(&mut state, KeyCode::Enter);
    assert_eq!(composer_text(&state), "first");
    with_queue(&mut state, Vec::new());
    assert_eq!(composer_text(&state), "");
    assert!(state.composer().unwrap().editing.is_none());
}

#[test]
fn banners_and_footer_render_thread_state() {
    let mut state = active();
    let mut response = timeline_response(vec![user("u1", 1, "hello")], false);
    response.active_prompt_mode = Some(ActivePromptMode {
        mode: "plan".into(),
        provider_id: "codex".into(),
        prompt: "Design the cache".into(),
    });
    response.goal = Some(ThreadGoal {
        objective: "Ship the composer".into(),
        status: GoalStatus::Active,
        token_budget: Some(50_000.0),
        tokens_used: 12_345.0,
        time_used_seconds: 125.0,
        ..ThreadGoal::default()
    });
    response.pending_todos = Some(PendingTodos {
        source_seq: 1,
        updated_at: 1,
        items: vec![
            PendingTodoItem {
                id: "1".into(),
                text: "Write tests".into(),
                status: PendingTodoStatus::Completed,
            },
            PendingTodoItem {
                id: "2".into(),
                text: "Render banners".into(),
                status: PendingTodoStatus::InProgress,
            },
        ],
    });
    response.model_fallback = Some(ModelFallback {
        source_seq: 4,
        original_model: "claude-opus-4-1".into(),
        fallback_model: "claude-sonnet-4-5".into(),
        ..ModelFallback::default()
    });
    response.context_window_usage = Some(ThreadContextWindowUsage {
        used_tokens: 84_000.0,
        model_context_window: 200_000.0,
        ..ThreadContextWindowUsage::default()
    });
    let mut snapshot: StoreSnapshot = (*state.snapshot).clone();
    let mut response_for = response.clone();
    for row in &mut response_for.rows {
        if let bb_wire::timeline::TimelineRow::Conversation(row) = row {
            row.base.thread_id = IDLE.into();
        }
    }
    snapshot
        .timelines
        .insert(IDLE.into(), timeline_entry(response_for));
    snapshot.version += 1;
    update(&mut state, Action::Snapshot(Arc::new(snapshot)));
    let text = screen(&state);
    assert!(text.contains("◆ Plan mode · Design the cache"), "{text}");
    assert!(
        text.contains("◎ Goal · Ship the composer · 12,345 / 50,000 tokens · 2m 5s"),
        "{text}"
    );
    assert!(
        text.contains("☐ Todos 1/2 complete · ▸ Render banners"),
        "{text}"
    );
    assert!(
        text.contains("⚠ Model fallback · Switched from Opus 4.1 to Sonnet 4.5"),
        "{text}"
    );
    assert!(text.contains("Claude-Sonnet-4-5 · High · Plan"), "{text}");
    assert!(
        text.contains("Enter queue · C-s steer · Esc Esc stop · ctx 42% 84k/200k"),
        "{text}"
    );

    with_queue(
        &mut state,
        vec![queued("q1", "first queued message"), queued("q2", "second")],
    );
    focus_composer(&mut state);
    type_text(&mut state, "multi");
    chord(&mut state, KeyCode::Enter, KeyModifiers::SHIFT);
    type_text(&mut state, "line draft");
    if std::env::var_os("BB_TUI_PRINT").is_some() {
        eprintln!("{}", screen(&state));
    }
    chord(&mut state, KeyCode::Char('m'), KeyModifiers::ALT);
    if std::env::var_os("BB_TUI_PRINT").is_some() {
        eprintln!("{}", screen(&state));
    }
    key(&mut state, KeyCode::Char('1'));
    let text = screen(&state);
    assert!(!text.contains("Model fallback"), "{text}");
    assert!(text.contains("GPT-5 · High"), "{text}");
}
