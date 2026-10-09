mod support;

use std::sync::Arc;
use std::time::{Duration, Instant};

use bb_store::{NoRealtime, Resource, Store, StoreConfig, StoreSnapshot};
use bb_tui::api::{InteractionValue, Mutation, MutationReply};
use bb_tui::app::{Action, AppState, Effect, Focus, ServerInfo, update};
use bb_tui::runtime::run;
use bb_wire::interaction::PendingInteraction;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use futures::stream;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use serde_json::{Value, json};
use support::*;
use tokio::sync::mpsc;

const THREAD: &str = "thr_wait";
const WIDTH: u16 = 120;
const HEIGHT: u16 = 32;

fn key(state: &mut AppState, code: KeyCode) -> Vec<Effect> {
    update(state, Action::Key(KeyEvent::from(code)))
}

fn ctrl(state: &mut AppState, ch: char) -> Vec<Effect> {
    update(
        state,
        Action::Key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::CONTROL)),
    )
}

fn type_text(state: &mut AppState, text: &str) {
    for ch in text.chars() {
        key(state, KeyCode::Char(ch));
    }
}

fn pending(id: &str, payload: Value) -> PendingInteraction {
    pending_with(id, payload, Value::Null)
}

fn pending_with(id: &str, payload: Value, origin: Value) -> PendingInteraction {
    let mut raw = json!({
        "id": id,
        "threadId": THREAD,
        "status": "pending",
        "createdAt": NOW_MS,
        "payload": payload,
    });
    if !origin.is_null() {
        raw["origin"] = origin;
    }
    serde_json::from_value(raw).expect("interaction")
}

fn command_approval() -> PendingInteraction {
    pending(
        "int_cmd",
        json!({
            "kind": "approval",
            "reason": null,
            "availableDecisions": ["allow_once", "allow_for_session", "deny"],
            "subject": {
                "kind": "command",
                "itemId": "item_1",
                "command": "/bin/bash -lc 'cargo test --workspace'",
                "cwd": "/work/repo",
                "actions": [],
                "sessionGrant": {"network": {"enabled": true}, "fileSystem": null}
            }
        }),
    )
}

fn set_interactions(state: &mut AppState, items: Vec<PendingInteraction>) {
    let mut snapshot: StoreSnapshot = (*state.snapshot).clone();
    snapshot.interactions.insert(
        THREAD.into(),
        Resource {
            data: Some(Arc::new(items)),
            ..Resource::default()
        },
    );
    update(state, Action::Snapshot(Arc::new(snapshot)));
}

fn waiting_state(items: Vec<PendingInteraction>) -> AppState {
    let mut state = loaded_state(sample_sidebar());
    set_interactions(&mut state, items);
    let effects = press_mod(&mut state, KeyCode::Char('w'), KeyModifiers::ALT);
    assert!(effects.contains(&Effect::WatchThread(Some(THREAD.into()))));
    assert_eq!(state.open_thread.as_deref(), Some(THREAD));
    assert_eq!(state.focus, Focus::Interaction);
    state
}

fn screen(state: &AppState) -> String {
    render_lines(state, WIDTH, HEIGHT).join("\n")
}

fn only_mutation(effects: Vec<Effect>) -> Mutation {
    let mutations: Vec<Mutation> = effects
        .into_iter()
        .filter_map(|effect| match effect {
            Effect::Mutate(mutation) => Some(mutation),
            _ => None,
        })
        .collect();
    assert_eq!(mutations.len(), 1, "{mutations:?}");
    mutations.into_iter().next().unwrap()
}

fn resolution_json(mutation: &Mutation) -> Value {
    match mutation {
        Mutation::ResolveInteraction { resolution, .. } => {
            serde_json::to_value(resolution.as_ref()).unwrap()
        }
        other => panic!("expected resolve, got {other:?}"),
    }
}

fn finish(
    state: &mut AppState,
    mutation: Mutation,
    result: Result<MutationReply, String>,
) -> Vec<Effect> {
    update(state, Action::MutationFinished { mutation, result })
}

#[test]
fn command_approval_renders_and_allow_for_session_sends_the_session_grant() {
    let mut state = waiting_state(vec![command_approval()]);
    let text = screen(&state);
    assert!(text.contains("⚑ Approval needed"), "{text}");
    assert!(text.contains("Do you want to run this command?"), "{text}");
    assert!(text.contains("$ cargo test --workspace"), "{text}");
    assert!(text.contains("/work/repo"), "{text}");
    assert!(text.contains("Session grant: Network access"), "{text}");
    assert!(
        text.contains("[d] Deny  [s] Allow for session  [y] Allow once"),
        "{text}"
    );
    assert!(text.contains("? 1 waiting"), "{text}");

    let mutation = only_mutation(key(&mut state, KeyCode::Char('s')));
    assert_eq!(
        resolution_json(&mutation),
        json!({
            "decision": "allow_for_session",
            "grantedPermissions": {"network": {"enabled": true}, "fileSystem": null}
        })
    );
    assert!(screen(&state).contains("Submitting…"));
    assert!(key(&mut state, KeyCode::Char('y')).is_empty());

    let effects = finish(&mut state, mutation, Ok(MutationReply::Done));
    assert!(
        effects.contains(&Effect::Refresh(bb_store::ResourceKey::Interactions(
            THREAD.into()
        )))
    );
    assert!(screen(&state).contains("Request approved for this session"));

    set_interactions(&mut state, Vec::new());
    assert!(!state.open_interaction_panel());
    assert_eq!(state.focus, Focus::Composer);
    assert!(state.interaction_forms.is_empty());
}

#[test]
fn deny_resolves_with_only_the_decision_like_the_web_banner() {
    let mut state = waiting_state(vec![command_approval()]);
    assert!(key(&mut state, KeyCode::Char('r')).is_empty());
    let mutation = only_mutation(key(&mut state, KeyCode::Char('d')));
    assert_eq!(resolution_json(&mutation), json!({"decision": "deny"}));
    assert!(!screen(&state).contains("Reason"));
    let effects = finish(&mut state, mutation, Ok(MutationReply::Done));
    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, Effect::Mutate(_))),
        "{effects:?}"
    );
}

#[test]
fn failed_resolution_shows_the_error_and_allows_a_retry() {
    let mut state = waiting_state(vec![command_approval()]);
    let mutation = only_mutation(key(&mut state, KeyCode::Char('y')));
    assert_eq!(
        resolution_json(&mutation),
        json!({"decision": "allow_once", "grantedPermissions": null})
    );
    finish(&mut state, mutation, Err("host disconnected".into()));
    let text = screen(&state);
    assert!(text.contains("✗ host disconnected"), "{text}");
    let retry = only_mutation(key(&mut state, KeyCode::Char('3')));
    assert_eq!(
        resolution_json(&retry),
        json!({"decision": "allow_once", "grantedPermissions": null})
    );
}

#[test]
fn plan_review_approves_or_keeps_planning() {
    let plan = pending(
        "int_plan",
        json!({
            "kind": "approval",
            "availableDecisions": ["allow_once", "deny"],
            "subject": {
                "kind": "plan",
                "itemId": "p1",
                "plan": "# Migrate storage\n\n1. Add the table\n2. Backfill rows",
                "planFilePath": "/work/repo/PLAN.md"
            }
        }),
    );
    let mut state = waiting_state(vec![plan]);
    let text = screen(&state);
    assert!(text.contains("⚑ Plan review"), "{text}");
    assert!(text.contains("Ready to code?"), "{text}");
    assert!(text.contains("Migrate storage"), "{text}");
    assert!(text.contains("2. Backfill rows"), "{text}");
    assert!(text.contains("/work/repo/PLAN.md"), "{text}");
    assert!(
        text.contains("[d] Keep planning  [y] Approve plan"),
        "{text}"
    );
    assert!(key(&mut state, KeyCode::Char('s')).is_empty());

    assert!(key(&mut state, KeyCode::Char('f')).is_empty());
    let mutation = only_mutation(key(&mut state, KeyCode::Char('d')));
    assert_eq!(resolution_json(&mutation), json!({"decision": "deny"}));
}

fn questions_payload() -> Value {
    json!({
        "kind": "user_question",
        "questions": [
            {
                "id": "db",
                "prompt": "Which database should we use?",
                "shortLabel": "Database",
                "multiSelect": false,
                "allowFreeText": false,
                "options": [
                    {"value": "pg", "label": "Postgres", "description": "Relational"},
                    {"value": "sqlite", "label": "SQLite"}
                ]
            },
            {
                "id": "features",
                "prompt": "Which features do you want?",
                "multiSelect": true,
                "allowFreeText": true,
                "options": [
                    {"value": "auth", "label": "Auth"},
                    {"value": "billing", "label": "Billing"}
                ]
            }
        ]
    })
}

#[test]
fn question_form_selects_with_digits_and_submits_user_answers() {
    let mut state = waiting_state(vec![pending("int_q", questions_payload())]);
    let text = screen(&state);
    assert!(text.contains("⚑ 2 questions"), "{text}");
    assert!(text.contains("Database"), "{text}");
    assert!(text.contains("Question 2"), "{text}");
    assert!(text.contains("Which database should we use?"), "{text}");
    assert!(text.contains("1 ( ) Postgres — Relational"), "{text}");

    key(&mut state, KeyCode::Char('2'));
    assert!(screen(&state).contains("2 (•) SQLite"));
    key(&mut state, KeyCode::Char('1'));
    assert!(key(&mut state, KeyCode::Enter).is_empty());
    let text = screen(&state);
    assert!(text.contains("✓ Database"), "{text}");
    assert!(text.contains("Which features do you want?"), "{text}");
    assert!(text.contains("3 [ ] Other…"), "{text}");

    key(&mut state, KeyCode::Char('1'));
    key(&mut state, KeyCode::Char('2'));
    key(&mut state, KeyCode::Char('2'));
    key(&mut state, KeyCode::Char('3'));
    type_text(&mut state, "  audit log ");
    assert!(screen(&state).contains("›   audit log"));
    let mutation = only_mutation(key(&mut state, KeyCode::Enter));
    match &mutation {
        Mutation::ResolveInteraction { interaction_id, .. } => assert_eq!(interaction_id, "int_q"),
        other => panic!("unexpected {other:?}"),
    }
    assert_eq!(
        resolution_json(&mutation),
        json!({
            "kind": "user_answer",
            "answers": {
                "db": {"selected": ["pg"]},
                "features": {"selected": ["auth"], "freeText": "audit log"}
            }
        })
    );
}

#[test]
fn question_form_refuses_to_submit_until_every_question_is_answered() {
    let mut state = waiting_state(vec![pending("int_q", questions_payload())]);
    key(&mut state, KeyCode::Tab);
    key(&mut state, KeyCode::Char('1'));
    assert!(key(&mut state, KeyCode::Enter).is_empty());
    let text = screen(&state);
    assert!(text.contains("Answer every question first"), "{text}");
    assert!(text.contains("Which database should we use?"), "{text}");
    let effects = key(&mut state, KeyCode::Enter);
    assert!(effects.is_empty());
    let text = screen(&state);
    assert!(text.contains("✓ Database"), "{text}");
    assert!(text.contains("Which features do you want?"), "{text}");
}

#[test]
fn free_text_only_question_types_immediately_and_caps_length() {
    let item = pending(
        "int_free",
        json!({
            "kind": "user_question",
            "questions": [{"id": "name", "prompt": "Name the service", "multiSelect": false, "allowFreeText": true}]
        }),
    );
    let mut state = waiting_state(vec![item]);
    update(&mut state, Action::Paste("x".repeat(5000)));
    type_text(&mut state, "more");
    let mutation = only_mutation(key(&mut state, KeyCode::Enter));
    let answer = resolution_json(&mutation)["answers"]["name"].clone();
    assert_eq!(answer["selected"], json!([]));
    assert_eq!(answer["freeText"].as_str().unwrap().len(), 4096);
}

#[test]
fn stop_turn_cancels_provider_questions() {
    let mut state = waiting_state(vec![pending("int_q", questions_payload())]);
    let mutation = only_mutation(ctrl(&mut state, 'x'));
    assert_eq!(
        mutation,
        Mutation::Stop {
            thread_id: THREAD.into()
        }
    );
}

#[test]
fn ask_user_question_plugin_responds_and_cancels_through_the_plugin_routes() {
    let item = pending_with(
        "int_plugin_q",
        json!({
            "kind": "plugin",
            "title": "Ask User Question",
            "data": {"questions": [{
                "id": "color", "prompt": "Pick a color", "shortLabel": "Color",
                "multiSelect": false, "allowFreeText": true,
                "options": [{"value": "red", "label": "Red"}, {"value": "blue", "label": "Blue"}]
            }]}
        }),
        json!({"kind": "plugin", "pluginId": "ask-user-question", "rendererId": "ask-user-question"}),
    );
    let mut state = waiting_state(vec![item]);
    assert!(screen(&state).contains("⚑ Question"));
    key(&mut state, KeyCode::Char('j'));
    let mutation = only_mutation(key(&mut state, KeyCode::Enter));
    assert_eq!(
        mutation,
        Mutation::RespondInteraction {
            thread_id: THREAD.into(),
            interaction_id: "int_plugin_q".into(),
            value: InteractionValue {
                value: json!({"answers": {"color": {"selected": ["blue"]}}}),
                sensitive: false,
            },
        }
    );
    finish(&mut state, mutation, Err("boom".into()));
    assert_eq!(
        only_mutation(ctrl(&mut state, 'x')),
        Mutation::CancelInteraction {
            thread_id: THREAD.into(),
            interaction_id: "int_plugin_q".into(),
        }
    );
}

fn secret_request() -> PendingInteraction {
    pending_with(
        "int_secret",
        json!({
            "kind": "plugin",
            "title": "Secrets",
            "data": {
                "purpose": "Deploy needs credentials",
                "destination": {"kind": "dotenv", "path": "/work/repo/.env"},
                "fields": [
                    {"name": "API_KEY", "description": "From the dashboard"},
                    {"name": "REGION", "description": null}
                ]
            }
        }),
        json!({"kind": "plugin", "pluginId": "secrets", "rendererId": "secret-request"}),
    )
}

#[test]
fn secret_request_masks_input_validates_and_never_exposes_values() {
    let mut state = waiting_state(vec![secret_request()]);
    let text = screen(&state);
    assert!(text.contains("⚑ Secret request"), "{text}");
    assert!(text.contains("Deploy needs credentials"), "{text}");
    assert!(
        text.contains("Secrets will be written directly to /work/repo/.env"),
        "{text}"
    );

    type_text(&mut state, "s3cr3t-value");
    let text = screen(&state);
    assert!(!text.contains("s3cr3t"), "{text}");
    assert!(text.contains("••••••••••••"), "{text}");
    assert!(!format!("{state:?}").contains("s3cr3t"));

    assert!(key(&mut state, KeyCode::Enter).is_empty());
    let effects = key(&mut state, KeyCode::Enter);
    assert!(effects.is_empty());
    update(&mut state, Action::Paste("eu-west-1\n".into()));
    ctrl(&mut state, 't');
    assert!(screen(&state).contains("eu-west-1"));
    ctrl(&mut state, 't');

    key(&mut state, KeyCode::Up);
    for _ in 0.."s3cr3t-value".len() {
        key(&mut state, KeyCode::Backspace);
    }
    key(&mut state, KeyCode::Tab);
    let blocked = key(&mut state, KeyCode::Enter);
    assert!(blocked.is_empty());
    key(&mut state, KeyCode::Tab);
    assert!(key(&mut state, KeyCode::Enter).is_empty());
    let text = screen(&state);
    assert!(!text.contains("Every secret must be"), "{text}");

    type_text(&mut state, "top'\"secret #1");
    let text_before = screen(&state);
    assert!(!text_before.contains("secret #1"));
    key(&mut state, KeyCode::Tab);
    assert!(key(&mut state, KeyCode::Enter).is_empty());
    key(&mut state, KeyCode::Tab);
    let invalid = key(&mut state, KeyCode::Enter);
    assert!(invalid.is_empty());
    assert!(screen(&state).contains("cannot be represented safely"));

    for _ in 0.."top'\"secret #1".len() {
        key(&mut state, KeyCode::Backspace);
    }
    type_text(&mut state, "k3y");
    key(&mut state, KeyCode::Tab);
    let mutation = only_mutation(key(&mut state, KeyCode::Enter));
    assert!(!format!("{mutation:?}").contains("k3y"));
    assert_eq!(
        mutation,
        Mutation::RespondInteraction {
            thread_id: THREAD.into(),
            interaction_id: "int_secret".into(),
            value: InteractionValue {
                value: json!({"values": {"API_KEY": "k3y", "REGION": "eu-west-1"}}),
                sensitive: true,
            },
        }
    );
    finish(&mut state, mutation, Ok(MutationReply::Done));
    assert!(!format!("{state:?}").contains("k3y"));
}

#[test]
fn unknown_plugin_request_falls_back_to_json_and_web_link() {
    let item = pending_with(
        "int_ext",
        json!({"kind": "acme/pick-color", "title": "Pick a palette", "data": {"choices": ["teal", "amber"]}}),
        json!({"kind": "provider", "providerId": "codex", "providerRequestId": "r1"}),
    );
    let mut state = waiting_state(vec![item]);
    let text = screen(&state);
    assert!(text.contains("⚑ Pick a palette"), "{text}");
    assert!(text.contains("Asked by the agent through acme"), "{text}");
    assert!(
        text.contains("Open in web: http://127.0.0.1:4000/threads/thr_wait"),
        "{text}"
    );
    assert!(text.contains("\"teal\""), "{text}");
    assert!(text.contains("[C-x] Stop turn"), "{text}");
    assert_eq!(
        key(&mut state, KeyCode::Char('o')),
        vec![Effect::OpenExternal(
            "http://127.0.0.1:4000/threads/thr_wait".into()
        )]
    );
    assert_eq!(key(&mut state, KeyCode::Char('x')), Vec::new());
    assert_eq!(
        only_mutation(ctrl(&mut state, 'x')),
        Mutation::Stop {
            thread_id: THREAD.into()
        }
    );
}

#[test]
fn read_only_mode_never_resolves() {
    let mut state = waiting_state(vec![command_approval()]);
    state.read_only = true;
    assert!(key(&mut state, KeyCode::Char('y')).is_empty());
    let text = screen(&state);
    assert!(
        text.contains("Read-only mode: cannot answer request"),
        "{text}"
    );
    assert!(!text.contains("Submitting…"), "{text}");
}

#[test]
fn panel_takes_over_from_the_composer_and_counts_queued_requests() {
    let mut state = loaded_state(sample_sidebar());
    press_mod(&mut state, KeyCode::Char('w'), KeyModifiers::ALT);
    assert_eq!(state.focus, Focus::Interaction);
    assert!(screen(&state).contains("Loading the request…"));
    key(&mut state, KeyCode::Esc);
    assert_eq!(state.focus, Focus::Main);
    let mut second = command_approval();
    second.id = "int_second".into();
    second.created_at += 1;
    set_interactions(&mut state, vec![second, command_approval()]);
    let text = screen(&state);
    assert!(
        text.contains("⚑ Approval needed · 1 of 2 · i to answer"),
        "{text}"
    );
    assert!(!text.contains("Message"), "{text}");
    key(&mut state, KeyCode::Char('i'));
    assert_eq!(state.focus, Focus::Interaction);
    let mutation = only_mutation(key(&mut state, KeyCode::Char('d')));
    match mutation {
        Mutation::ResolveInteraction { interaction_id, .. } => {
            assert_eq!(interaction_id, "int_cmd")
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn next_pending_cycles_waiting_threads_and_reports_none() {
    let mut sidebar = sample_sidebar();
    sidebar.projects[0].threads[0].has_pending_interaction = false;
    sidebar.projects[0].threads[2].has_pending_interaction = false;
    let mut state = loaded_state(sidebar);
    assert!(press_mod(&mut state, KeyCode::Char('w'), KeyModifiers::ALT).is_empty());
    let text = screen(&state);
    assert!(text.contains("No requests are waiting for you"), "{text}");
    assert!(!text.contains("? 1 waiting"), "{text}");

    let mut state = loaded_state(sample_sidebar());
    let mut snapshot: StoreSnapshot = (*state.snapshot).clone();
    let mut sidebar = (**snapshot.sidebar.data.as_ref().unwrap()).clone();
    sidebar.projects[1].threads[0].has_pending_interaction = true;
    snapshot.sidebar.data = Some(Arc::new(sidebar));
    update(&mut state, Action::Snapshot(Arc::new(snapshot)));
    assert!(screen(&state).contains("? 2 waiting"));
    press_mod(&mut state, KeyCode::Char('w'), KeyModifiers::ALT);
    let first = state.open_thread.clone().unwrap();
    press_mod(&mut state, KeyCode::Char('w'), KeyModifiers::ALT);
    let second = state.open_thread.clone().unwrap();
    assert_ne!(first, second);
    key(&mut state, KeyCode::Esc);
    press_mod(&mut state, KeyCode::Char('w'), KeyModifiers::ALT);
    assert_eq!(state.open_thread.as_deref(), Some(second.as_str()));
    assert_eq!(state.focus, Focus::Interaction);
    press_mod(&mut state, KeyCode::Char('w'), KeyModifiers::ALT);
    assert_eq!(state.open_thread.as_deref(), Some(first.as_str()));
}

async fn wait_until(predicate: impl Fn() -> bool) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while !predicate() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("condition");
}

async fn wait_for(store: &Store, predicate: impl Fn(&StoreSnapshot) -> bool) {
    let mut rx = store.subscribe();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if predicate(&rx.borrow_and_update()) {
                return;
            }
            rx.changed().await.expect("store alive");
        }
    })
    .await
    .expect("store condition");
}

#[tokio::test]
async fn event_loop_answers_a_question_against_the_fake_backend() {
    let backend = FakeBackend::new(sample_sidebar());
    backend
        .interactions
        .lock()
        .unwrap()
        .insert(THREAD.into(), vec![pending("int_q", questions_payload())]);
    let api = MockApi::new(backend.clone());
    let store = Store::spawn(backend.clone(), NoRealtime, None, StoreConfig::default());
    let (tx, mut rx) = mpsc::unbounded_channel::<Action>();
    let events = Box::pin(stream::poll_fn(move |cx| rx.poll_recv(cx)));
    let state = AppState::new(
        ServerInfo {
            url: "http://fake/".into(),
            profile: None,
        },
        Instant::now(),
        NOW_MS,
    );
    let task_store = store.clone();
    let task_api = api.clone();
    let handle = tokio::spawn(async move {
        let mut terminal = Terminal::new(TestBackend::new(WIDTH, HEIGHT)).unwrap();
        let mut state = state;
        run(
            &mut terminal,
            &mut state,
            task_store,
            task_api,
            Arc::new(MockPlatform::default()),
            events,
        )
        .await
        .expect("run");
        (terminal, state)
    });
    let send = |code: KeyCode| tx.send(Action::Key(KeyEvent::from(code))).unwrap();

    wait_for(&store, |snapshot| snapshot.sidebar.data.is_some()).await;
    tx.send(Action::Key(KeyEvent::new(
        KeyCode::Char('w'),
        KeyModifiers::ALT,
    )))
    .unwrap();
    wait_for(&store, |snapshot| {
        !snapshot.pending_interactions(THREAD).is_empty()
    })
    .await;
    tokio::time::sleep(Duration::from_millis(30)).await;
    send(KeyCode::Char('2'));
    send(KeyCode::Enter);
    send(KeyCode::Char('2'));
    send(KeyCode::Enter);
    wait_for(&store, |snapshot| {
        snapshot
            .interactions
            .get(THREAD)
            .and_then(|resource| resource.data.as_deref())
            .is_some_and(Vec::is_empty)
            && snapshot
                .sidebar_thread(THREAD)
                .is_some_and(|entry| !entry.has_pending_interaction)
    })
    .await;
    let recorded = api.clone();
    wait_until(move || {
        recorded
            .recorded()
            .iter()
            .any(|mutation| mutation.is_interaction())
    })
    .await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    send(KeyCode::Esc);
    send(KeyCode::Char(' '));
    send(KeyCode::Char('q'));
    send(KeyCode::Char('q'));
    let (terminal, state) = tokio::time::timeout(Duration::from_secs(5), handle)
        .await
        .expect("loop exits")
        .expect("join");

    let resolved: Vec<Mutation> = api
        .recorded()
        .into_iter()
        .filter(Mutation::is_interaction)
        .collect();
    assert_eq!(resolved.len(), 1);
    assert_eq!(
        resolution_json(&resolved[0]),
        json!({
            "kind": "user_answer",
            "answers": {"db": {"selected": ["sqlite"]}, "features": {"selected": ["billing"]}}
        })
    );
    assert!(state.interaction_forms.is_empty());
    let text = buffer_lines(&terminal).join("\n");
    assert!(!text.contains("⚑ 2 questions"), "{text}");
    assert!(text.contains("Answer sent"), "{text}");
}
