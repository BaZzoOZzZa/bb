use std::collections::BTreeSet;

use bb_store::invalidation::{
    Effect, PendingChanges, Target, TimelineRefresh, merge_thread_change_metadata,
    thread_change_effects,
};
use bb_wire::realtime::{ChangedMessage, ThreadChangeKind, ThreadChangeMetadata};
use serde_json::{Value, json};

fn message(value: Value) -> ChangedMessage {
    serde_json::from_value(value).expect("changed message")
}

fn timeline(thread_id: &str, mode: TimelineRefresh) -> Effect {
    Effect::RefreshTimeline {
        thread_id: Some(thread_id.into()),
        mode,
    }
}

#[test]
fn debounces_appends_and_keeps_the_highest_sequence() {
    let mut pending = PendingChanges::default();
    for sequence in [7, 9, 8] {
        let outcome = pending.record(&message(json!({
            "entity": "thread",
            "id": "t1",
            "changes": ["events-appended"],
            "metadata": {"timelineSequence": sequence, "eventTypes": ["item/delta"]}
        })));
        assert!(outcome.scheduled);
        assert!(!outcome.flush_now);
        assert_eq!(
            outcome.immediate,
            [Effect::NoteTimelineSequence {
                thread_id: "t1".into(),
                sequence
            }]
        );
    }
    let effects = pending.take_effects();
    assert_eq!(
        effects,
        [
            Effect::NoteTimelineSequence {
                thread_id: "t1".into(),
                sequence: 9
            },
            timeline("t1", TimelineRefresh::Delta),
        ]
    );
    assert!(pending.is_empty());
}

#[test]
fn applies_immediate_kinds_without_scheduling() {
    let mut pending = PendingChanges::default();
    let outcome = pending.record(&message(json!({
        "entity": "thread",
        "id": "t1",
        "changes": ["history-rewritten"]
    })));
    assert!(!outcome.scheduled);
    assert!(
        outcome
            .immediate
            .contains(&timeline("t1", TimelineRefresh::Full))
    );
    assert!(
        outcome
            .immediate
            .contains(&Effect::Refetch(Target::QueuedMessages(Some("t1".into()))))
    );
    assert!(pending.is_empty());
}

#[test]
fn turn_completion_flushes_now_and_refreshes_terminally() {
    let mut pending = PendingChanges::default();
    let outcome = pending.record(&message(json!({
        "entity": "thread",
        "id": "t1",
        "changes": ["events-appended", "status-changed"],
        "metadata": {"eventTypes": ["turn/completed"], "backgroundActivityChanged": true}
    })));
    assert!(outcome.flush_now);
    let effects = pending.take_effects();
    assert!(effects.contains(&timeline("t1", TimelineRefresh::Terminal)));
    assert!(effects.contains(&Effect::Refetch(Target::Sidebar)));
    assert!(effects.contains(&Effect::Refetch(Target::Thread(Some("t1".into())))));
}

#[test]
fn status_changes_without_metadata_fall_back_to_a_sidebar_refetch() {
    let kinds = BTreeSet::from([ThreadChangeKind::StatusChanged]);
    let effects = thread_change_effects(Some("t1"), &kinds, None);
    assert!(effects.contains(&Effect::Refetch(Target::Sidebar)));
    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, Effect::PatchStatus { .. }))
    );
}

#[test]
fn global_thread_changes_target_every_cached_thread() {
    let kinds = BTreeSet::from([ThreadChangeKind::TitleChanged]);
    assert_eq!(
        thread_change_effects(None, &kinds, None),
        [
            Effect::Refetch(Target::Sidebar),
            Effect::Refetch(Target::Thread(None))
        ]
    );
}

#[test]
fn a_status_change_clears_an_older_status_snapshot() {
    let current = ThreadChangeMetadata {
        status_change: Some(Default::default()),
        project_id: Some("p1".into()),
        ..Default::default()
    };
    let merged =
        merge_thread_change_metadata(Some(&current), &ThreadChangeMetadata::default(), true);
    assert_eq!(merged.status_change, None);
    assert_eq!(merged.project_id.as_deref(), Some("p1"));
}

#[test]
fn environment_project_and_system_changes_are_batched() {
    let mut pending = PendingChanges::default();
    for value in [
        json!({"entity": "environment", "id": "env-1", "changes": ["metadata-changed"]}),
        json!({"entity": "project", "changes": ["project-updated"]}),
        json!({"entity": "system", "changes": ["config-changed", "brand-new-kind"]}),
        json!({"entity": "host", "changes": ["host-connected"]}),
    ] {
        assert!(pending.record(&message(value)).scheduled);
    }
    let effects = pending.take_effects();
    assert!(
        effects.contains(&Effect::Refetch(Target::ThreadsInEnvironment(
            "env-1".into()
        )))
    );
    assert!(effects.contains(&Effect::Refetch(Target::Sidebar)));
    assert!(effects.contains(&Effect::Refetch(Target::Hosts)));
    assert!(effects.contains(&Effect::RefreshTimeline {
        thread_id: None,
        mode: TimelineRefresh::Delta
    }));
    assert_eq!(
        effects
            .iter()
            .filter(|effect| **effect == Effect::Refetch(Target::Sidebar))
            .count(),
        1
    );
}
