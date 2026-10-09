mod support;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use bb_client::RealtimeEvent;
use bb_store::backend::{FetchResult, SubscriptionGuard};
use bb_store::timeline_merge::row_id;
use bb_store::{
    Backend, ConnectionState, DeferredContent, FetchError, RealtimeSubscriber, Store, StoreConfig,
    StoreSnapshot, TurnDetailsKey,
};
use bb_wire::environment::Host;
use bb_wire::interaction::PendingInteraction;
use bb_wire::project::SidebarBootstrapResponse;
use bb_wire::queue::ThreadQueuedMessage;
use bb_wire::realtime::{ChangedMessage, SubscriptionTarget};
use bb_wire::system::SystemAttentionResponse;
use bb_wire::thread::{ThreadResponse, ThreadStatus};
use bb_wire::timeline::{
    ThreadTimelineQuery, ThreadTimelineResponse, TurnSummaryDetailsQuery,
    TurnSummaryDetailsResponse,
};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use support::{command_row, patch, timeline_response_json, user_row};
use tokio::sync::broadcast;
use tokio::time::Instant;

#[derive(Debug, Clone, PartialEq)]
enum Call {
    Sidebar,
    Hosts,
    Attention,
    Thread(String),
    Timeline(String, ThreadTimelineQuery),
    Details(String, TurnSummaryDetailsQuery),
    Interactions(String),
    Queued(String),
}

type Handler = dyn Fn(&Call) -> Result<Value, FetchError> + Send + Sync;

struct FakeBackend {
    calls: Mutex<Vec<(Instant, Call)>>,
    handler: Box<Handler>,
}

impl FakeBackend {
    fn new(
        handler: impl Fn(&Call) -> Result<Value, FetchError> + Send + Sync + 'static,
    ) -> Arc<Self> {
        Arc::new(Self {
            calls: Mutex::new(Vec::new()),
            handler: Box::new(handler),
        })
    }

    fn respond<T: DeserializeOwned>(&self, call: Call) -> FetchResult<T> {
        let result = (self.handler)(&call);
        self.calls.lock().unwrap().push((Instant::now(), call));
        result.map(|value| serde_json::from_value(value).expect("fake response"))
    }

    fn calls(&self) -> Vec<Call> {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .map(|(_, call)| call.clone())
            .collect()
    }

    fn timed_calls(&self) -> Vec<(Instant, Call)> {
        self.calls.lock().unwrap().clone()
    }

    fn count(&self, predicate: impl Fn(&Call) -> bool) -> usize {
        self.calls().iter().filter(|call| predicate(call)).count()
    }

    fn timeline_queries(&self, thread_id: &str) -> Vec<ThreadTimelineQuery> {
        self.calls()
            .into_iter()
            .filter_map(|call| match call {
                Call::Timeline(id, query) if id == thread_id => Some(query),
                _ => None,
            })
            .collect()
    }
}

impl Backend for FakeBackend {
    async fn sidebar_bootstrap(&self) -> FetchResult<SidebarBootstrapResponse> {
        self.respond(Call::Sidebar)
    }

    async fn hosts(&self) -> FetchResult<Vec<Host>> {
        self.respond(Call::Hosts)
    }

    async fn attention(&self) -> FetchResult<SystemAttentionResponse> {
        self.respond(Call::Attention)
    }

    async fn thread(&self, thread_id: String) -> FetchResult<ThreadResponse> {
        self.respond(Call::Thread(thread_id))
    }

    async fn timeline(
        &self,
        thread_id: String,
        query: ThreadTimelineQuery,
    ) -> FetchResult<ThreadTimelineResponse> {
        self.respond(Call::Timeline(thread_id, query))
    }

    async fn turn_summary_details(
        &self,
        thread_id: String,
        query: TurnSummaryDetailsQuery,
    ) -> FetchResult<TurnSummaryDetailsResponse> {
        self.respond(Call::Details(thread_id, query))
    }

    async fn interactions(&self, thread_id: String) -> FetchResult<Vec<PendingInteraction>> {
        self.respond(Call::Interactions(thread_id))
    }

    async fn queued_messages(&self, thread_id: String) -> FetchResult<Vec<ThreadQueuedMessage>> {
        self.respond(Call::Queued(thread_id))
    }
}

#[derive(Clone, Default)]
struct RecordingSubscriber {
    log: Arc<Mutex<Vec<(SubscriptionTarget, bool)>>>,
}

struct RecordingGuard {
    target: SubscriptionTarget,
    log: Arc<Mutex<Vec<(SubscriptionTarget, bool)>>>,
}

impl Drop for RecordingGuard {
    fn drop(&mut self) {
        self.log.lock().unwrap().push((self.target.clone(), false));
    }
}

impl RealtimeSubscriber for RecordingSubscriber {
    fn subscribe(&self, target: SubscriptionTarget) -> SubscriptionGuard {
        self.log.lock().unwrap().push((target.clone(), true));
        Box::new(RecordingGuard {
            target,
            log: self.log.clone(),
        })
    }
}

fn thread_entry(id: &str, status: &str) -> Value {
    json!({
        "id": id,
        "projectId": "proj-1",
        "environmentId": "env-1",
        "providerId": "codex",
        "title": format!("Thread {id}"),
        "status": status,
        "visibility": "visible",
        "latestAttentionAt": 1,
        "createdAt": 1,
        "updatedAt": 1,
        "runtime": {"displayStatus": status},
        "activity": {"activeWorkflowCount": 0, "activeBackgroundAgentCount": 0, "activeBackgroundCommandCount": 0, "activePlanModeCount": 0, "activeGoalCount": 0},
        "queuedWork": "none",
        "hasPendingInteraction": false
    })
}

fn sidebar_json() -> Value {
    json!({
        "sections": [],
        "projects": [{
            "id": "proj-1",
            "name": "Project",
            "kind": "local",
            "sources": [],
            "threads": [thread_entry("thread-1", "idle"), thread_entry("thread-2", "idle")]
        }],
        "personalProject": {"id": "personal", "name": "Personal", "kind": "personal", "sources": [], "threads": []}
    })
}

fn thread_json(id: &str) -> Value {
    patch(
        &thread_entry(id, "idle"),
        json!({"activeBackgroundAgentCount": 0, "canRestoreEnvironment": false, "canSpawnChild": true, "queuedMessageCount": 0}),
    )
}

fn default_responses(call: &Call) -> Option<Value> {
    Some(match call {
        Call::Sidebar => sidebar_json(),
        Call::Hosts => json!([]),
        Call::Attention => json!({"hasAttention": false}),
        Call::Thread(id) => thread_json(id),
        Call::Interactions(_) | Call::Queued(_) => json!([]),
        _ => return None,
    })
}

fn changed(value: Value) -> RealtimeEvent {
    let message: ChangedMessage = serde_json::from_value(value).expect("changed message");
    RealtimeEvent::Changed(message)
}

fn setup(
    handler: impl Fn(&Call) -> Result<Value, FetchError> + Send + Sync + 'static,
) -> (
    Store,
    Arc<FakeBackend>,
    broadcast::Sender<RealtimeEvent>,
    RecordingSubscriber,
) {
    let backend = FakeBackend::new(handler);
    let (events, receiver) = broadcast::channel(64);
    let subscriber = RecordingSubscriber::default();
    let store = Store::spawn(
        backend.clone(),
        subscriber.clone(),
        Some(receiver),
        StoreConfig::default(),
    );
    (store, backend, events, subscriber)
}

async fn settle() {
    tokio::time::sleep(Duration::from_secs(1)).await;
}

fn timeline_ids(snapshot: &StoreSnapshot, thread_id: &str) -> Vec<String> {
    snapshot
        .timeline(thread_id)
        .map(|entry| {
            entry
                .rows()
                .iter()
                .map(|row| row_id(row).to_owned())
                .collect()
        })
        .unwrap_or_default()
}

fn delta_response(upserts: Vec<Value>, row_order: Option<Vec<&str>>, max_seq: i64) -> Value {
    let mut response = timeline_response_json(&[], None, Some(max_seq));
    let mut delta = json!({"upsertRows": upserts});
    if let Some(order) = row_order {
        delta["rowOrder"] = json!(order);
    }
    response["delta"] = delta;
    response
}

#[tokio::test(start_paused = true)]
async fn watching_the_sidebar_loads_sidebar_resources_and_subscribes() {
    let (store, backend, _events, subscriber) = setup(|call| Ok(default_responses(call).unwrap()));
    let interest = store.watch_sidebar();
    settle().await;
    let snapshot = store.snapshot();
    assert_eq!(snapshot.sidebar_threads().count(), 2);
    assert_eq!(
        snapshot.attention.data,
        Some(SystemAttentionResponse {
            has_attention: false
        })
    );
    assert!(snapshot.hosts.data.is_some());
    assert_eq!(backend.count(|call| *call == Call::Sidebar), 1);
    let subscribed: Vec<_> = subscriber.log.lock().unwrap().clone();
    assert!(subscribed.contains(&(SubscriptionTarget::ThreadList, true)));
    assert!(subscribed.contains(&(SubscriptionTarget::System, true)));
    drop(interest);
    assert!(
        subscriber
            .log
            .lock()
            .unwrap()
            .contains(&(SubscriptionTarget::ThreadList, false))
    );
}

#[tokio::test(start_paused = true)]
async fn coalesces_appended_events_into_one_delta_fetch() {
    let (store, backend, events, _subscriber) =
        setup(|call| match call {
            Call::Timeline(_, query) if query.after_sequence.is_none() => Ok(
                timeline_response_json(&[user_row("u1", 1), command_row("c1", 2)], None, None),
            ),
            Call::Timeline(_, query) => {
                assert_eq!(query.after_sequence, Some(2));
                assert_eq!(query.defer_content, Some(true));
                Ok(delta_response(
                    vec![
                        patch(&command_row("c1", 2), json!({"output": "more"})),
                        command_row("c2", 3),
                    ],
                    Some(vec!["u1", "c1", "c2"]),
                    3,
                ))
            }
            other => Ok(default_responses(other).unwrap()),
        });
    let _thread = store.watch_thread("thread-1");
    settle().await;
    let first = store.snapshot();
    assert_eq!(timeline_ids(&first, "thread-1"), ["u1", "c1"]);
    let user_before = first.timeline("thread-1").unwrap().rows()[0].clone();

    for sequence in [3, 3, 3] {
        events
            .send(changed(json!({
                "type": "changed",
                "entity": "thread",
                "id": "thread-1",
                "changes": ["events-appended"],
                "metadata": {"timelineSequence": sequence}
            })))
            .unwrap();
    }
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert!(
        store
            .snapshot()
            .timeline("thread-1")
            .unwrap()
            .is_catching_up()
    );
    settle().await;

    let queries = backend.timeline_queries("thread-1");
    assert_eq!(queries.len(), 2);
    let snapshot = store.snapshot();
    let entry = snapshot.timeline("thread-1").unwrap();
    assert_eq!(timeline_ids(&snapshot, "thread-1"), ["u1", "c1", "c2"]);
    assert!(Arc::ptr_eq(&entry.rows()[0], &user_before));
    assert!(!entry.is_catching_up());
    assert_eq!(entry.max_seq(), Some(3));
}

#[tokio::test(start_paused = true)]
async fn falls_back_to_a_full_fetch_when_the_delta_base_is_stale() {
    let (store, backend, events, _subscriber) = setup(|call| match call {
        Call::Timeline(_, query) if query.after_sequence.is_some() => {
            Ok(delta_response(vec![], Some(vec!["u1", "missing"]), 5))
        }
        Call::Timeline(..) => Ok(timeline_response_json(&[user_row("u1", 1)], None, None)),
        other => Ok(default_responses(other).unwrap()),
    });
    let _thread = store.watch_thread("thread-1");
    settle().await;
    events
        .send(changed(
            json!({"entity": "thread", "id": "thread-1", "changes": ["events-appended"]}),
        ))
        .unwrap();
    settle().await;
    let queries = backend.timeline_queries("thread-1");
    let after: Vec<Option<i64>> = queries.iter().map(|query| query.after_sequence).collect();
    assert_eq!(after, [None, Some(1), None]);
    assert_eq!(timeline_ids(&store.snapshot(), "thread-1"), ["u1"]);
}

#[tokio::test(start_paused = true)]
async fn status_changes_patch_the_sidebar_without_refetching_it() {
    let (store, backend, events, _subscriber) = setup(|call| match call {
        Call::Timeline(..) => Ok(timeline_response_json(&[], None, Some(0))),
        other => Ok(default_responses(other).unwrap()),
    });
    let _sidebar = store.watch_sidebar();
    settle().await;
    events
        .send(changed(json!({
            "entity": "thread",
            "id": "thread-2",
            "changes": ["status-changed"],
            "metadata": {"statusChange": {
                "status": "active",
                "runtime": {"displayStatus": "active"},
                "activity": {"activeWorkflowCount": 1, "activeBackgroundAgentCount": 0, "activeBackgroundCommandCount": 0, "activePlanModeCount": 0, "activeGoalCount": 0},
                "latestAttentionAt": 50,
                "updatedAt": 60
            }}
        })))
        .unwrap();
    tokio::time::sleep(Duration::from_millis(1)).await;
    let snapshot = store.snapshot();
    let entry = snapshot.sidebar_thread("thread-2").unwrap();
    assert_eq!(entry.thread.status, ThreadStatus::Active);
    assert_eq!(entry.activity.active_workflow_count, 1);
    assert_eq!(entry.thread.updated_at, 60);
    settle().await;
    assert_eq!(backend.count(|call| *call == Call::Sidebar), 1);
    assert_eq!(backend.count(|call| *call == Call::Attention), 2);
}

#[tokio::test(start_paused = true)]
async fn history_rewrites_refetch_the_full_timeline_immediately() {
    let rewritten = Arc::new(Mutex::new(false));
    let flag = rewritten.clone();
    let (store, backend, events, _subscriber) = setup(move |call| match call {
        Call::Timeline(_, query) => {
            assert_eq!(query.after_sequence, None);
            let rows = if *flag.lock().unwrap() {
                vec![user_row("edited", 1)]
            } else {
                vec![user_row("u1", 1), command_row("c1", 2)]
            };
            Ok(timeline_response_json(&rows, None, None))
        }
        other => Ok(default_responses(other).unwrap()),
    });
    let _thread = store.watch_thread("thread-1");
    settle().await;
    *rewritten.lock().unwrap() = true;
    let sent_at = Instant::now();
    events
        .send(changed(
            json!({"entity": "thread", "id": "thread-1", "changes": ["history-rewritten"]}),
        ))
        .unwrap();
    settle().await;
    assert_eq!(timeline_ids(&store.snapshot(), "thread-1"), ["edited"]);
    let refetch = backend
        .timed_calls()
        .into_iter()
        .filter(|(_, call)| matches!(call, Call::Timeline(..)))
        .nth(1)
        .expect("refetch");
    assert!(refetch.0 - sent_at < Duration::from_millis(50));
    assert_eq!(
        backend.count(|call| *call == Call::Queued("thread-1".into())),
        2
    );
    assert_eq!(
        backend.count(|call| *call == Call::Interactions("thread-1".into())),
        2
    );
}

#[tokio::test(start_paused = true)]
async fn turn_completion_flushes_without_waiting_for_the_debounce() {
    let (store, backend, events, _subscriber) = setup(|call| match call {
        Call::Timeline(_, query) if query.after_sequence.is_some() => Ok(delta_response(
            vec![command_row("c2", 3)],
            Some(vec!["u1", "c2"]),
            3,
        )),
        Call::Timeline(..) => Ok(timeline_response_json(&[user_row("u1", 1)], None, None)),
        other => Ok(default_responses(other).unwrap()),
    });
    let _thread = store.watch_thread("thread-1");
    settle().await;
    let sent_at = Instant::now();
    events
        .send(changed(json!({
            "entity": "thread",
            "id": "thread-1",
            "changes": ["events-appended"],
            "metadata": {"eventTypes": ["turn/completed"], "timelineSequence": 3}
        })))
        .unwrap();
    settle().await;
    let (at, _) = backend
        .timed_calls()
        .into_iter()
        .filter(|(_, call)| matches!(call, Call::Timeline(..)))
        .nth(1)
        .expect("refetch");
    assert!(at - sent_at < Duration::from_millis(50));
    assert_eq!(timeline_ids(&store.snapshot(), "thread-1"), ["u1", "c2"]);
}

#[tokio::test(start_paused = true)]
async fn interaction_and_queue_changes_refetch_their_lists() {
    let (store, backend, events, _subscriber) = setup(|call| match call {
        Call::Timeline(..) => Ok(timeline_response_json(&[], None, Some(0))),
        other => Ok(default_responses(other).unwrap()),
    });
    let _sidebar = store.watch_sidebar();
    let _thread = store.watch_thread("thread-1");
    settle().await;
    events
        .send(changed(json!({
            "entity": "thread",
            "id": "thread-1",
            "changes": ["interactions-changed"],
            "metadata": {"hasPendingInteraction": true}
        })))
        .unwrap();
    settle().await;
    assert!(
        store
            .snapshot()
            .sidebar_thread("thread-1")
            .unwrap()
            .has_pending_interaction
    );
    events
        .send(changed(
            json!({"entity": "thread", "id": "thread-1", "changes": ["queue-changed"]}),
        ))
        .unwrap();
    settle().await;
    assert_eq!(
        backend.count(|call| *call == Call::Interactions("thread-1".into())),
        2
    );
    assert_eq!(
        backend.count(|call| *call == Call::Queued("thread-1".into())),
        2
    );
    assert_eq!(backend.count(|call| *call == Call::Sidebar), 2);
}

#[tokio::test(start_paused = true)]
async fn unwatched_threads_are_only_marked_stale() {
    let (store, backend, events, _subscriber) = setup(|call| match call {
        Call::Timeline(..) => Ok(timeline_response_json(&[user_row("u1", 1)], None, None)),
        other => Ok(default_responses(other).unwrap()),
    });
    let thread = store.watch_thread("thread-1");
    settle().await;
    drop(thread);
    settle().await;
    events
        .send(changed(json!({"entity": "thread", "id": "thread-1", "changes": ["events-appended", "title-changed"]})))
        .unwrap();
    settle().await;
    assert_eq!(backend.timeline_queries("thread-1").len(), 1);
    assert_eq!(
        backend.count(|call| *call == Call::Thread("thread-1".into())),
        1
    );
    let snapshot = store.snapshot();
    assert!(snapshot.timeline("thread-1").unwrap().stale);
    assert!(snapshot.threads["thread-1"].stale);

    let _again = store.watch_thread("thread-1");
    settle().await;
    let queries = backend.timeline_queries("thread-1");
    assert_eq!(queries.len(), 2);
    assert_eq!(queries[1].after_sequence, Some(1));
    assert!(!store.snapshot().timeline("thread-1").unwrap().stale);
}

#[tokio::test(start_paused = true)]
async fn reconnects_refetch_every_watched_resource() {
    let (store, backend, events, _subscriber) = setup(|call| match call {
        Call::Timeline(..) => Ok(timeline_response_json(&[user_row("u1", 1)], None, None)),
        other => Ok(default_responses(other).unwrap()),
    });
    let _sidebar = store.watch_sidebar();
    let _thread = store.watch_thread("thread-1");
    settle().await;
    events
        .send(RealtimeEvent::Disconnected {
            retry_in: Some(Duration::from_secs(1)),
            error: None,
        })
        .unwrap();
    settle().await;
    assert!(matches!(
        store.snapshot().connection,
        ConnectionState::Disconnected { .. }
    ));
    events.send(RealtimeEvent::Resumed).unwrap();
    settle().await;
    assert_eq!(store.snapshot().connection, ConnectionState::Connected);
    for call in [
        Call::Sidebar,
        Call::Hosts,
        Call::Attention,
        Call::Thread("thread-1".into()),
        Call::Interactions("thread-1".into()),
        Call::Queued("thread-1".into()),
    ] {
        assert_eq!(backend.count(|candidate| *candidate == call), 2, "{call:?}");
    }
    assert_eq!(backend.timeline_queries("thread-1").len(), 2);
}

#[tokio::test(start_paused = true)]
async fn loads_older_pages_and_recovers_from_stale_cursors() {
    let stale = Arc::new(Mutex::new(false));
    let stale_flag = stale.clone();
    let (store, backend, _events, _subscriber) = setup(move |call| match call {
        Call::Timeline(_, query) if query.before_anchor_id.is_some() => {
            if *stale_flag.lock().unwrap() {
                return Err(FetchError::http(
                    400,
                    Some("invalid_request"),
                    "cursor no longer available",
                ));
            }
            assert_eq!(query.before_anchor_seq, Some(10));
            Ok(timeline_response_json(
                &[user_row("older", 1), command_row("older-work", 2)],
                Some(support::cursor("oldest", 1)),
                Some(9),
            ))
        }
        Call::Timeline(..) => Ok(timeline_response_json(
            &[user_row("latest", 10)],
            Some(support::cursor("latest-page", 10)),
            None,
        )),
        other => Ok(default_responses(other).unwrap()),
    });
    let _thread = store.watch_thread("thread-1");
    settle().await;
    assert!(
        store
            .snapshot()
            .timeline("thread-1")
            .unwrap()
            .has_older_rows()
    );

    store.load_older_timeline_rows("thread-1");
    settle().await;
    let snapshot = store.snapshot();
    assert_eq!(
        timeline_ids(&snapshot, "thread-1"),
        ["older", "older-work", "latest"]
    );
    assert_eq!(
        snapshot
            .timeline("thread-1")
            .unwrap()
            .loaded
            .older_cursor
            .as_ref()
            .map(|cursor| cursor.anchor_id.as_str()),
        Some("oldest")
    );

    *stale.lock().unwrap() = true;
    store.load_older_timeline_rows("thread-1");
    settle().await;
    let snapshot = store.snapshot();
    let entry = snapshot.timeline("thread-1").unwrap();
    assert_eq!(
        timeline_ids(&snapshot, "thread-1"),
        ["older", "older-work", "latest"]
    );
    assert_eq!(
        entry
            .loaded
            .older_cursor
            .as_ref()
            .map(|cursor| cursor.anchor_id.as_str()),
        Some("latest-page")
    );
    assert!(!entry.loading_older);
    assert_eq!(backend.timeline_queries("thread-1").len(), 4);
}

#[tokio::test(start_paused = true)]
async fn loads_deferred_content_through_turn_details() {
    let deferred = patch(
        &command_row("row-1", 4),
        json!({"callId": "call-1", "output": "", "contentDeferred": true}),
    );
    let deferred_for_handler = deferred.clone();
    let (store, backend, _events, _subscriber) = setup(move |call| match call {
        Call::Timeline(..) => Ok(timeline_response_json(
            std::slice::from_ref(&deferred_for_handler),
            None,
            None,
        )),
        Call::Details(_, query) => {
            assert_eq!(query.item_id.as_deref(), Some("call-1"));
            Ok(
                json!({"rows": [patch(&command_row("detail", 4), json!({"callId": "call-1", "output": "the full output"}))]}),
            )
        }
        other => Ok(default_responses(other).unwrap()),
    });
    let _thread = store.watch_thread("thread-1");
    settle().await;
    let row = store.snapshot().timeline("thread-1").unwrap().rows()[0].clone();
    assert_eq!(
        store.snapshot().resolve_deferred_row("thread-1", &row),
        DeferredContent::Pending
    );
    assert!(store.load_deferred_content("thread-1", &row));
    assert!(store.load_deferred_content("thread-1", &row));
    settle().await;
    let snapshot = store.snapshot();
    let DeferredContent::Loaded(resolved) = snapshot.resolve_deferred_row("thread-1", &row) else {
        panic!("expected loaded content");
    };
    let bb_wire::timeline::TimelineRow::Work(bb_wire::timeline::TimelineWorkRow::Command(command)) =
        resolved
    else {
        panic!("expected command");
    };
    assert_eq!(command.output, "the full output");
    assert_eq!(backend.count(|call| matches!(call, Call::Details(..))), 1);
    let key = TurnDetailsKey::for_deferred_row("thread-1", &row).unwrap();
    assert!(snapshot.turn_details[&key].loaded);
}

#[tokio::test(start_paused = true)]
async fn fetch_errors_are_kept_without_dropping_cached_data() {
    let fail = Arc::new(Mutex::new(false));
    let fail_flag = fail.clone();
    let (store, _backend, _events, _subscriber) = setup(move |call| match call {
        Call::Sidebar if *fail_flag.lock().unwrap() => Err(FetchError::new("boom")),
        other => Ok(default_responses(other).unwrap_or(json!(null))),
    });
    let _sidebar = store.watch_sidebar();
    settle().await;
    *fail.lock().unwrap() = true;
    store.refresh(bb_store::ResourceKey::Sidebar);
    settle().await;
    let snapshot = store.snapshot();
    assert_eq!(snapshot.sidebar_threads().count(), 2);
    assert_eq!(
        snapshot
            .sidebar
            .error
            .as_ref()
            .map(|error| error.message.as_str()),
        Some("boom")
    );
}
