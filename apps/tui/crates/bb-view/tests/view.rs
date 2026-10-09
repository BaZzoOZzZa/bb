mod support;

use bb_view::view::{
    SummaryKind, ViewRow, build_timeline_view_rows, build_timeline_view_rows_with,
    build_timeline_work_summary_label,
};
use bb_wire::timeline::{ActivityIntent, TimelineRowStatus, TimelineWorkRow};
use serde_json::json;
use support::*;

fn read_paths(intents: &[ActivityIntent]) -> Vec<Option<String>> {
    intents
        .iter()
        .map(|intent| match intent {
            ActivityIntent::Read { path, .. } => path.clone(),
            _ => None,
        })
        .collect()
}

#[test]
fn keeps_a_single_completed_command_as_a_leaf() {
    let rows = build_timeline_view_rows(&rows(vec![command("command-1")]));
    assert_eq!(rows.len(), 1);
    let work = expect_work(rows.first());
    assert!(!work.in_closed_step);
    let TimelineWorkRow::Command(command) = &work.row else {
        panic!("expected command");
    };
    assert_eq!(command.base.id, "command-1");
    assert_eq!(command.completed_at, Some(201));
    assert_eq!(command.base.turn_id.as_deref(), Some("turn-1"));
}

#[test]
fn keeps_single_terminal_work_rows_as_direct_leaves_regardless_of_status() {
    let cases = vec![
        with(command("command-error"), json!({ "status": "error" })),
        with(
            command("command-interrupted"),
            json!({ "status": "interrupted" }),
        ),
        with(
            command("command-denied"),
            json!({ "approvalStatus": "denied" }),
        ),
        with(
            file_change("file-change-error", "src/app.ts"),
            json!({ "status": "error" }),
        ),
        with(
            file_change("file-change-interrupted", "src/app.ts"),
            json!({ "status": "interrupted" }),
        ),
        with(
            file_change("file-change-denied", "src/app.ts"),
            json!({ "approvalStatus": "denied" }),
        ),
        with(tool("tool-error"), json!({ "status": "error" })),
        with(tool("tool-interrupted"), json!({ "status": "interrupted" })),
        with(tool("tool-denied"), json!({ "approvalStatus": "denied" })),
    ];
    for case in cases {
        let input = rows(vec![case]);
        let view = build_timeline_view_rows(&input);
        assert_eq!(view.len(), 1);
        let bb_wire::timeline::TimelineRow::Work(expected) = &input[0] else {
            panic!("expected work input");
        };
        assert_eq!(&expect_work(view.first()).row, expected);
    }
}

#[test]
fn keeps_activity_summary_identity_stable_as_a_run_grows() {
    let first = build_timeline_view_rows(&rows(vec![seq(command("command-1"), 1)]));
    let next = build_timeline_view_rows(&rows(vec![
        seq(command("command-1"), 1),
        seq(command("command-2"), 2),
    ]));
    assert_eq!(first[0].id(), "command-1");
    let summary = expect_summary(next.first(), SummaryKind::Bundle);
    assert_eq!(summary.base.id, "thread-1:turn-1:work-summary:command-1");
    assert_eq!(summary.status, TimelineRowStatus::Completed);
    assert_eq!(summary.base.source_seq_start, 1);
    assert_eq!(summary.base.source_seq_end, 2);
    assert_eq!(child_ids(summary), ["command-1", "command-2"]);
    assert_eq!(
        build_timeline_work_summary_label(summary, false),
        "Ran 2 commands"
    );
}

#[test]
fn keeps_bundle_identity_stable_across_activity_transitions() {
    let pending = build_timeline_view_rows(&rows(vec![
        with(command("command-1"), json!({ "status": "pending" })),
        with(
            command("command-2"),
            json!({ "status": "pending", "sourceSeqStart": 2 }),
        ),
    ]));
    let completed = build_timeline_view_rows(&rows(vec![
        command("command-1"),
        with(command("command-2"), json!({ "sourceSeqStart": 2 })),
    ]));
    let pending_summary = expect_summary(pending.first(), SummaryKind::Bundle);
    let completed_summary = expect_summary(completed.first(), SummaryKind::Bundle);
    assert_eq!(
        pending_summary.base.id,
        "thread-1:turn-1:work-summary:command-1"
    );
    assert_eq!(completed_summary.base.id, pending_summary.base.id);
    assert_eq!(
        build_timeline_work_summary_label(pending_summary, true),
        "Running 2 commands"
    );
    assert_eq!(
        build_timeline_work_summary_label(completed_summary, false),
        "Ran 2 commands"
    );
}

#[test]
fn keeps_single_non_terminal_work_rows_visible_as_leaves() {
    let pending = build_timeline_view_rows(&rows(vec![with(
        command("command-pending"),
        json!({ "status": "pending" }),
    )]));
    let waiting = build_timeline_view_rows(&rows(vec![with(
        command("command-waiting"),
        json!({ "approvalStatus": "waiting_for_approval" }),
    )]));
    assert_eq!(pending.len(), 1);
    assert_eq!(
        expect_work(pending.first()).status(),
        TimelineRowStatus::Pending
    );
    assert_eq!(waiting.len(), 1);
    assert_eq!(waiting[0].id(), "command-waiting");
}

#[test]
fn groups_same_concept_work_into_a_bundle_regardless_of_status_mix() {
    let view = build_timeline_view_rows(&rows(vec![
        command("command-completed"),
        with(
            command("command-pending"),
            json!({ "status": "pending", "sourceSeqStart": 2 }),
        ),
    ]));
    assert_eq!(view.len(), 1);
    let bundle = expect_summary(view.first(), SummaryKind::Bundle);
    assert_eq!(child_ids(bundle), ["command-completed", "command-pending"]);
    assert_eq!(bundle.status, TimelineRowStatus::Pending);
}

#[test]
fn uses_active_labels_for_command_subagent_and_file_edit_runs() {
    let pending = json!({ "status": "pending" });
    let commands = build_timeline_view_rows(&rows(vec![
        with(command("c1"), pending.clone()),
        with(command("c2"), pending.clone()),
    ]));
    let delegations = build_timeline_view_rows(&rows(vec![
        with(delegation("d1", Vec::new()), pending.clone()),
        with(delegation("d2", Vec::new()), pending.clone()),
    ]));
    let edits = build_timeline_view_rows(&rows(vec![
        with(file_change("f1", "src/app.ts"), pending.clone()),
        with(file_change("f2", "src/other.ts"), pending.clone()),
    ]));
    let label = |rows: &[ViewRow]| {
        build_timeline_work_summary_label(expect_summary(rows.first(), SummaryKind::Bundle), true)
    };
    assert_eq!(label(&commands), "Running 2 commands");
    assert_eq!(label(&delegations), "Running 2 subagents");
    assert_eq!(label(&edits), "Editing 2 files");
    assert_eq!(
        expect_summary(delegations.first(), SummaryKind::Bundle).status,
        TimelineRowStatus::Pending
    );
}

#[test]
fn emits_multi_concept_step_summary_once_an_assistant_boundary_closes_the_step() {
    let view = build_timeline_view_rows(&rows(vec![
        with(
            command("read-1"),
            json!({ "activityIntents": [read_intent("src/app.ts")] }),
        ),
        with(command("command-1"), json!({ "sourceSeqStart": 2 })),
        with(assistant("assistant-1"), json!({ "sourceSeqStart": 3 })),
    ]));
    let summary = expect_summary(view.first(), SummaryKind::Step);
    assert_eq!(
        build_timeline_work_summary_label(summary, false),
        "Explored 1 file, ran 1 command"
    );
    assert_eq!(kinds(&view), ["step-summary", "conversation"]);
}

#[test]
fn uses_active_labels_for_tool_only_bundles() {
    let view = build_timeline_view_rows(&rows(vec![
        with(tool("t1"), json!({ "status": "pending" })),
        with(tool("t2"), json!({ "status": "pending" })),
    ]));
    assert_eq!(
        build_timeline_work_summary_label(expect_summary(view.first(), SummaryKind::Bundle), true),
        "Running 2 tools"
    );
}

#[test]
fn collapses_completed_delegation_children_into_a_step_summary() {
    let view = build_timeline_view_rows(&rows(vec![delegation(
        "delegation-1",
        vec![
            with(
                command("child-command-1"),
                json!({ "callId": "child-call-1", "sourceSeqStart": 10, "sourceSeqEnd": 10, "startedAt": 10, "createdAt": 10 }),
            ),
            with(
                command("child-command-2"),
                json!({ "callId": "child-call-2", "sourceSeqStart": 11, "sourceSeqEnd": 11, "startedAt": 11, "createdAt": 11 }),
            ),
        ],
    )]));
    assert_eq!(view.len(), 1);
    let delegation = expect_work(view.first());
    let TimelineWorkRow::Delegation(row) = &delegation.row else {
        panic!("expected delegation");
    };
    assert!(row.child_rows.is_none());
    let children = delegation.child_rows.as_deref().expect("child rows");
    assert_eq!(children.len(), 1);
    let summary = expect_summary(children.first(), SummaryKind::Step);
    assert_eq!(
        build_timeline_work_summary_label(summary, false),
        "Ran 2 commands"
    );
    assert_eq!(summary.status, TimelineRowStatus::Completed);
    assert_eq!(summary.base.source_seq_start, 10);
    assert_eq!(summary.base.source_seq_end, 11);
    assert_eq!(summary.base.started_at, 10);
    assert_eq!(summary.base.created_at, 11);
    assert_eq!(summary.base.turn_id.as_deref(), Some("turn-1"));
    let calls: Vec<(String, String)> = summary
        .work_children()
        .map(|work| match &work.row {
            TimelineWorkRow::Command(command) => (command.call_id.clone(), command.command.clone()),
            _ => panic!("expected command child"),
        })
        .collect();
    assert_eq!(
        calls,
        [
            ("child-call-1".to_owned(), "pnpm test".to_owned()),
            ("child-call-2".to_owned(), "pnpm test".to_owned())
        ]
    );
}

#[test]
fn keeps_pending_delegation_children_as_live_bundles() {
    let view = build_timeline_view_rows(&rows(vec![with(
        delegation("delegation-1", vec![command("c1"), command("c2")]),
        json!({ "status": "pending" }),
    )]));
    let children = expect_work(view.first())
        .child_rows
        .as_deref()
        .expect("child rows");
    assert_eq!(kinds(children), ["bundle-summary"]);
}

#[test]
fn summary_turn_id_is_cleared_when_children_span_turns() {
    let view = build_timeline_view_rows(&rows(vec![
        command("c1"),
        with(command("c2"), json!({ "turnId": "turn-2" })),
    ]));
    let summary = expect_summary(view.first(), SummaryKind::Bundle);
    assert_eq!(summary.base.turn_id, None);
    assert_eq!(summary.base.id, "thread-1:thread:work-summary:c1");
}

#[test]
fn collapses_consecutive_duplicate_read_intents_across_bundle_children() {
    let view = build_timeline_view_rows(&rows(vec![
        command_reading(&["a/b/c"], 1),
        command_reading(&["a/b/c"], 2),
        command_reading(&["a/b/d"], 3),
        command_reading(&["a/b/c"], 4),
    ]));
    let bundle = expect_summary(view.first(), SummaryKind::Bundle);
    assert_eq!(
        read_paths(&command_intents(bundle)),
        [
            Some("a/b/c".to_owned()),
            Some("a/b/d".to_owned()),
            Some("a/b/c".to_owned())
        ]
    );
}

#[test]
fn collapses_consecutive_duplicate_list_and_search_intents() {
    let view = build_timeline_view_rows(&rows(vec![
        seq(
            with(
                command("list-1"),
                json!({ "activityIntents": [list_intent(Some("src"))] }),
            ),
            1,
        ),
        seq(
            with(
                command("list-2"),
                json!({ "activityIntents": [list_intent(Some("src"))] }),
            ),
            2,
        ),
        seq(
            with(
                command("search-1"),
                json!({ "activityIntents": [search_intent("TODO", Some("src"))] }),
            ),
            3,
        ),
        seq(
            with(
                command("search-2"),
                json!({ "activityIntents": [search_intent("TODO", Some("src"))] }),
            ),
            4,
        ),
    ]));
    let bundle = expect_summary(view.first(), SummaryKind::Bundle);
    let types: Vec<&str> = command_intents(bundle)
        .iter()
        .map(|intent| match intent {
            ActivityIntent::ListFiles { .. } => "list_files",
            ActivityIntent::Search { .. } => "search",
            _ => "other",
        })
        .collect();
    assert_eq!(types, ["list_files", "search"]);
    assert_eq!(child_ids(bundle), ["list-1", "search-1"]);
}

#[test]
fn dedupes_consecutive_duplicates_within_a_single_rows_intents() {
    let view = build_timeline_view_rows(&rows(vec![
        seq(
            with(
                command("multi-1"),
                json!({ "activityIntents": [read_intent("a"), read_intent("a"), read_intent("b"), read_intent("a")] }),
            ),
            1,
        ),
        command_reading(&["c"], 2),
    ]));
    let bundle = expect_summary(view.first(), SummaryKind::Bundle);
    assert_eq!(
        read_paths(&command_intents(bundle)),
        ["a", "b", "a", "c"].map(|path| Some(path.to_owned()))
    );
}

#[test]
fn non_exploration_siblings_break_the_dedupe_chain() {
    let view = build_timeline_view_rows(&rows(vec![
        command_reading(&["a"], 1),
        seq(file_change("edit-1", "src/app.ts"), 2),
        command_reading(&["a"], 3),
        seq(assistant("assistant-1"), 4),
    ]));
    let step = expect_summary(view.first(), SummaryKind::Step);
    assert_eq!(
        read_paths(&command_intents(step)),
        [Some("a".to_owned()), Some("a".to_owned())]
    );
}

#[test]
fn does_not_modify_intents_on_standalone_rows() {
    let view = build_timeline_view_rows(&rows(vec![seq(
        with(
            command("solo"),
            json!({ "activityIntents": [read_intent("a"), read_intent("a")] }),
        ),
        1,
    )]));
    assert_eq!(view.len(), 1);
    let TimelineWorkRow::Command(command) = &expect_work(view.first()).row else {
        panic!("expected command");
    };
    assert_eq!(
        read_paths(&command.activity_intents),
        [Some("a".to_owned()), Some("a".to_owned())]
    );
}

#[test]
fn keeps_interleaved_thoughts_inside_a_closed_exploration_and_edit_step() {
    let input = vec![
        thought("before", 1),
        command_reading(&["src/app.ts"], 2),
        thought("between", 3),
        seq(file_change("edit", "src/app.ts"), 4),
        thought("after", 5),
    ];
    let live = build_timeline_view_rows(&rows(input.clone()));
    assert_eq!(kinds(&live), ["bundle-summary", "bundle-summary"]);
    assert_eq!(
        build_timeline_work_summary_label(expect_summary(live.first(), SummaryKind::Bundle), false),
        "Explored 1 file"
    );
    assert_eq!(
        build_timeline_work_summary_label(expect_summary(live.get(1), SummaryKind::Bundle), false),
        "Edited 1 file"
    );
    let mut closed_input = input;
    closed_input.push(seq(assistant("response"), 6));
    let closed = build_timeline_view_rows(&rows(closed_input));
    let summary = expect_summary(closed.first(), SummaryKind::Step);
    assert_eq!(
        child_ids(summary),
        ["before", "read-2", "between", "edit", "after"]
    );
    assert_eq!(
        build_timeline_work_summary_label(summary, false),
        "Explored 1 file, edited 1 file"
    );
    assert_eq!(closed[1].id(), "response");
}

#[test]
fn bundles_live_exploration_across_thoughts_without_counting_them() {
    let view = build_timeline_view_rows(&rows(vec![
        thought("before", 1),
        command_reading(&["a.ts"], 2),
        thought("between", 3),
        command_reading(&["b.ts"], 4),
        thought("after", 5),
    ]));
    assert_eq!(view.len(), 1);
    let summary = expect_summary(view.first(), SummaryKind::Bundle);
    assert_eq!(
        child_ids(summary),
        ["before", "read-2", "between", "read-4", "after"]
    );
    assert_eq!(
        build_timeline_work_summary_label(summary, false),
        "Explored 2 files"
    );
}

#[test]
fn leaves_thought_only_sequences_visible_and_does_not_cross_warnings_or_messages() {
    let first = thought("first", 1);
    let second = thought("second", 2);
    let only = rows(vec![first.clone(), second.clone()]);
    assert_eq!(
        kinds(&build_timeline_view_rows_with(&only, true)),
        ["system", "system"]
    );
    assert_eq!(
        kinds(&build_timeline_view_rows(&only)),
        ["system", "system"]
    );
    let warning = with(
        first.clone(),
        json!({ "id": "warning", "operationKind": "warning" }),
    );
    let view = build_timeline_view_rows(&rows(vec![
        command_reading(&["a.ts"], 0),
        first,
        warning,
        command_reading(&["b.ts"], 3),
        second,
        seq(assistant("response"), 4),
        command_reading(&["c.ts"], 5),
    ]));
    assert_eq!(
        kinds(&view),
        [
            "bundle-summary",
            "system",
            "step-summary",
            "conversation",
            "work"
        ]
    );
    assert_eq!(view[1].id(), "warning");
}

#[test]
fn closed_scope_marks_single_work_rows_as_closed_step_leaves() {
    let view = build_timeline_view_rows_with(&rows(vec![thought("t", 1), command("c1")]), true);
    assert_eq!(kinds(&view), ["step-summary"]);
    let single = build_timeline_view_rows_with(&rows(vec![command("c1")]), true);
    assert!(expect_work(single.first()).in_closed_step);
}

#[test]
fn sent_thread_messages_and_interactions_break_activity_runs() {
    let tell = with(
        command("tell"),
        json!({ "command": "bb thread tell thr_23456789ab 'ship it'" }),
    );
    let question = with(
        base("question-1"),
        json!({
            "kind": "work",
            "workKind": "question",
            "status": "pending",
            "interactionId": "pi-1",
            "lifecycle": "pending",
            "questions": [],
            "answers": null,
            "statusReason": null,
        }),
    );
    let view = build_timeline_view_rows(&rows(vec![
        command("c1"),
        command("c2"),
        tell,
        command("c3"),
        question,
        command("c4"),
    ]));
    assert_eq!(
        kinds(&view),
        ["bundle-summary", "work", "work", "work", "work"]
    );
    assert_eq!(view[1].id(), "tell");
}

#[test]
fn pending_user_steers_do_not_close_the_step() {
    let steer = with(
        user("steer", "also check tests"),
        json!({ "turnRequest": { "isGrouped": false, "kind": "steer", "status": "pending" } }),
    );
    let view = build_timeline_view_rows(&rows(vec![command("c1"), steer, command("c2")]));
    assert_eq!(kinds(&view), ["work", "conversation", "work"]);
    assert!(!expect_work(view.first()).in_closed_step);
}

#[test]
fn unknown_rows_pass_through_without_breaking_grouping_rules() {
    let view = build_timeline_view_rows(&rows(vec![
        command("c1"),
        json!({ "kind": "future-kind", "id": "x" }),
        with(
            base("w"),
            json!({ "kind": "work", "workKind": "hologram", "status": "completed" }),
        ),
        command("c2"),
    ]));
    assert_eq!(kinds(&view), ["work", "unknown", "work", "work"]);
    assert_eq!(view[1].id(), "x");
    assert_eq!(view[2].id(), "w");
}
