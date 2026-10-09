mod support;

use std::sync::Arc;

use bb_store::timeline_merge::{
    apply_timeline_delta, compute_timeline_row_delta, merge_latest_timeline_rows,
    merge_loaded_timeline_with_latest, prepend_older_rows, prepend_older_timeline_rows,
    recover_loaded_timeline_after_stale_cursor, resolve_loaded_timeline_surface_key, row_id,
};
use bb_wire::timeline::{TimelineDelta, TimelineRow, TimelineWorkRow};
use serde_json::{Value, json};
use support::*;

fn rows(values: &[Value]) -> Vec<bb_store::RowRef> {
    values.iter().map(row).collect()
}

fn children_ids(rows: &[bb_store::RowRef]) -> Vec<String> {
    rows.iter()
        .flat_map(|row| match &**row {
            TimelineRow::Turn(turn) => turn
                .children
                .iter()
                .flatten()
                .map(|child| row_id(child).to_owned())
                .collect::<Vec<_>>(),
            _ => Vec::new(),
        })
        .collect()
}

fn text_of(row: &TimelineRow) -> &str {
    match row {
        TimelineRow::Conversation(row) => &row.text,
        _ => panic!("expected conversation row"),
    }
}

#[test]
fn prepends_older_server_ordered_rows_without_sorting_by_source_sequence() {
    let merged = prepend_older_timeline_rows(
        &rows(&[user_row("latest-user", 20)]),
        &rows(&[user_row("older-user", 10), command_row("older-command", 1)]),
    );
    assert_eq!(ids(&merged), ["older-user", "older-command", "latest-user"]);
}

#[test]
fn keeps_server_ordered_worked_for_rows_after_the_first_user() {
    let merged = prepend_older_timeline_rows(
        &rows(&[user_row("latest-user", 20)]),
        &rows(&[
            user_row("first-user", 10),
            turn_summary_row("worked-for-summary", 1, 1, None),
        ]),
    );
    assert_eq!(
        ids(&merged),
        ["first-user", "worked-for-summary", "latest-user"]
    );
}

#[test]
fn keeps_distinct_byte_budget_slices_of_one_finished_turn() {
    let older_slice = turn_summary_row(
        "turn-1:sequence-page:10",
        10,
        10,
        Some(vec![
            command_row("command-1", 10),
            command_row("command-2", 11),
        ]),
    );
    let latest_slice = turn_summary_row(
        "turn-1:sequence-page:20",
        20,
        20,
        Some(vec![
            command_row("command-3", 20),
            command_row("command-4", 21),
        ]),
    );
    let merged = prepend_older_timeline_rows(&rows(&[latest_slice]), &rows(&[older_slice]));
    assert_eq!(
        ids(&merged),
        ["turn-1:sequence-page:10", "turn-1:sequence-page:20"]
    );
    assert_eq!(
        children_ids(&merged),
        ["command-1", "command-2", "command-3", "command-4"]
    );
}

#[test]
fn replaces_a_byte_cut_latest_page_while_an_unfinished_turn_grows() {
    let loaded: Vec<Value> = [15, 16, 17, 18]
        .iter()
        .map(|seq| command_row(&format!("command-{seq}"), *seq))
        .collect();
    let latest: Vec<Value> = [15, 16, 17, 18, 19, 20]
        .iter()
        .map(|seq| command_row(&format!("command-{seq}"), *seq))
        .collect();
    let merge = merge_latest_timeline_rows(&rows(&latest), 15, &rows(&loaded));
    let call_ids: std::collections::HashSet<String> = merge
        .rows
        .iter()
        .filter_map(|row| match &**row {
            TimelineRow::Work(TimelineWorkRow::Command(command)) => Some(command.call_id.clone()),
            _ => None,
        })
        .collect();
    assert!(merge.can_merge);
    assert_eq!(merge.rows.len(), 6);
    assert_eq!(call_ids.len(), 6);
}

#[test]
fn replaces_the_overlapping_latest_tail_while_preserving_loaded_history() {
    let old_tail = user_row("live-tail", 20);
    let updated_tail = patch(
        &old_tail,
        json!({"sourceSeqEnd": 21, "text": "updated tail"}),
    );
    let merge = merge_latest_timeline_rows(
        &rows(&[updated_tail, command_row("new-streaming-row", 21)]),
        20,
        &rows(&[user_row("older-user", 1), old_tail]),
    );
    assert_eq!(
        ids(&merge.rows),
        ["older-user", "live-tail", "new-streaming-row"]
    );
    assert_eq!(text_of(&merge.rows[1]), "updated tail");
}

#[test]
fn retains_every_loaded_row_before_the_latest_raw_window_boundary() {
    let prompt = row(&user_row("prompt", 1));
    let straddling = command_row_until("straddling-work", 20, 210);
    let steer = row(&user_row("accepted-steer", 100));
    let older_work = row(&command_row("older-work", 120));
    let covered_stale = row(&command_row("covered-stale-row", 180));
    let tail = command_row("tail", 200);
    let updated_straddling = row(&patch(
        &straddling,
        json!({"sourceSeqEnd": 300, "output": "updated straddling output"}),
    ));
    let updated_tail = row(&patch(
        &tail,
        json!({"sourceSeqEnd": 300, "output": "updated tail output"}),
    ));
    let new_tail = row(&command_row("new-tail", 250));

    let merge = merge_latest_timeline_rows(
        &[updated_straddling.clone(), updated_tail.clone(), new_tail],
        150,
        &[
            prompt,
            row(&straddling),
            steer.clone(),
            older_work.clone(),
            covered_stale,
            row(&tail),
        ],
    );
    assert_eq!(
        ids(&merge.rows),
        [
            "prompt",
            "straddling-work",
            "accepted-steer",
            "older-work",
            "tail",
            "new-tail"
        ]
    );
    assert!(merge.can_merge);
    assert!(Arc::ptr_eq(&merge.rows[1], &updated_straddling));
    assert!(Arc::ptr_eq(&merge.rows[2], &steer));
    assert!(Arc::ptr_eq(&merge.rows[3], &older_work));
    assert!(Arc::ptr_eq(&merge.rows[4], &updated_tail));
}

#[test]
fn preserves_unchanged_overlapping_row_references_after_a_latest_refetch() {
    let older_user = row(&user_row("older-user", 1));
    let old_tail_value = user_row("live-tail", 20);
    let old_tail = row(&old_tail_value);
    let loaded = vec![older_user.clone(), old_tail.clone()];
    let merge = merge_latest_timeline_rows(&[row(&old_tail_value)], 20, &loaded);
    assert_eq!(merge.rows.len(), 2);
    assert!(bb_store::timeline_merge::same_row_refs(
        &merge.rows,
        &loaded
    ));
    assert!(Arc::ptr_eq(&merge.rows[0], &older_user));
    assert!(Arc::ptr_eq(&merge.rows[1], &old_tail));
}

#[test]
fn replaces_a_pending_message_row_when_the_server_accepts_it() {
    let pending = row(&user_row_with("submitted-message", 1, 1, "pending"));
    let accepted = row(&user_row_with("submitted-message", 1, 1, "accepted"));
    let merge = merge_latest_timeline_rows(std::slice::from_ref(&accepted), 0, &[pending]);
    assert_eq!(plain(&merge.rows), vec![(*accepted).clone()]);
    assert!(Arc::ptr_eq(&merge.rows[0], &accepted));
}

#[test]
fn drops_optimistic_rows_when_merging_the_latest_page() {
    let optimistic = row(&user_row("optimistic-user-1", 5));
    let merge = merge_latest_timeline_rows(
        &rows(&[user_row("accepted", 5)]),
        5,
        &[row(&user_row("older", 1)), optimistic],
    );
    assert_eq!(ids(&merge.rows), ["older", "accepted"]);
}

#[test]
fn rebuilds_when_latest_advances_past_the_loaded_rows_with_a_gap_between() {
    let oldest_cursor = cursor("oldest", 1);
    let latest_cursor = cursor("latest-page", 40);
    let current = loaded_state(rows(&[user_row("oldest", 1)]), Some(oldest_cursor), None);
    let latest = latest(&timeline_response_json(
        &[user_row("latest", 50)],
        Some(latest_cursor.clone()),
        None,
    ));
    let next = merge_loaded_timeline_with_latest(&current, &latest, "thread-1:default");
    assert_eq!(ids(&next.rows), ["latest"]);
    assert_eq!(next.older_cursor, Some(parse_cursor(&latest_cursor)));
}

#[test]
fn rebuilds_across_a_raw_sequence_gap_even_when_a_projected_row_overlaps() {
    let latest_cursor = cursor("latest-page", 150);
    let straddling = command_row_until("straddling-work", 20, 100);
    let updated = patch(
        &straddling,
        json!({"sourceSeqEnd": 300, "output": "updated output"}),
    );
    let current = loaded_state(
        rows(std::slice::from_ref(&straddling)),
        Some(cursor("old-page", 1)),
        Some(100),
    );
    let latest = latest(&timeline_response_json(
        std::slice::from_ref(&updated),
        Some(latest_cursor.clone()),
        Some(300),
    ));
    let next = merge_loaded_timeline_with_latest(&current, &latest, "thread-1:default");
    assert_eq!(plain(&next.rows), plain_values(&[updated]));
    assert_eq!(next.older_cursor, Some(parse_cursor(&latest_cursor)));
}

#[test]
fn keeps_loaded_rows_when_the_latest_window_abuts_them_without_overlapping() {
    let oldest_cursor = cursor("oldest", 1);
    let current = loaded_state(
        rows(&[user_row("oldest", 1)]),
        Some(oldest_cursor.clone()),
        None,
    );
    let latest = latest(&timeline_response_json(
        &[user_row("latest", 2)],
        Some(cursor("latest-page", 2)),
        None,
    ));
    let next = merge_loaded_timeline_with_latest(&current, &latest, "thread-1:default");
    assert_eq!(ids(&next.rows), ["oldest", "latest"]);
    assert_eq!(next.older_cursor, Some(parse_cursor(&oldest_cursor)));
}

#[test]
fn reconciles_when_a_finished_turn_reaches_back_past_loaded_in_turn_rows() {
    let live_tail = command_row("live-tail", 520);
    let current = loaded_state(
        rows(&[command_row("live-work", 500), live_tail.clone()]),
        Some(cursor("thread-1:in-turn:500", 500)),
        None,
    );
    let finished_cursor = cursor("older-turn", 1);
    let latest = latest(&timeline_response_json(
        &[
            user_row("turn-prompt", 10),
            turn_summary_row("turn-summary", 11, 11, None),
            live_tail,
        ],
        Some(finished_cursor.clone()),
        None,
    ));
    let next = merge_loaded_timeline_with_latest(&current, &latest, "thread-1:default");
    assert_eq!(
        ids(&next.rows),
        ["turn-prompt", "turn-summary", "live-tail"]
    );
    assert_eq!(next.older_cursor, Some(parse_cursor(&finished_cursor)));
}

#[test]
fn keeps_loaded_rows_when_unprojected_events_separate_them_from_the_follow_up_window() {
    let oldest_cursor = cursor("oldest", 1);
    let current = loaded_state(
        rows(&[
            user_row("oldest", 1),
            turn_summary_row("turn-summary", 10, 100, None),
            command_row("late-error", 99),
        ]),
        Some(oldest_cursor.clone()),
        Some(100),
    );
    let latest = latest(&timeline_response_json(
        &[user_row("follow-up", 101)],
        Some(cursor("follow-up", 101)),
        Some(104),
    ));
    let next = merge_loaded_timeline_with_latest(&current, &latest, "thread-1:default");
    assert_eq!(
        ids(&next.rows),
        ["oldest", "turn-summary", "late-error", "follow-up"]
    );
    assert_eq!(next.older_cursor, Some(parse_cursor(&oldest_cursor)));
}

#[test]
fn keeps_loaded_rows_when_a_windows_first_row_is_backfilled_from_below_the_cut() {
    let current = loaded_state(
        rows(&[command_row("loaded-work", 40)]),
        Some(cursor("thread-1:in-turn:30", 30)),
        Some(59),
    );
    let latest = latest(&timeline_response_json(
        &[
            turn_summary_row("turn-summary", 20, 70, None),
            command_row("live-work", 65),
        ],
        Some(cursor("thread-1:in-turn:60", 60)),
        Some(70),
    ));
    let next = merge_loaded_timeline_with_latest(&current, &latest, "thread-1:default");
    assert_eq!(
        ids(&next.rows),
        ["loaded-work", "turn-summary", "live-work"]
    );
}

#[test]
fn recovers_from_a_stale_cursor_with_a_fresh_latest_cursor_without_dropping_loaded_rows() {
    let fresh_cursor = cursor("fresh-cursor", 40);
    let old_tail = user_row("live-tail", 20);
    let updated_tail = patch(
        &old_tail,
        json!({"sourceSeqEnd": 21, "text": "updated tail"}),
    );
    let latest = latest(&timeline_response_json(
        &[updated_tail],
        Some(fresh_cursor.clone()),
        None,
    ));
    let next = recover_loaded_timeline_after_stale_cursor(
        &loaded_state(
            rows(&[user_row("older-user", 1), old_tail]),
            Some(cursor("stale-cursor", 1)),
            None,
        ),
        &latest,
        "thread-1:default",
    );
    assert_eq!(ids(&next.rows), ["older-user", "live-tail"]);
    assert_eq!(text_of(&next.rows[1]), "updated tail");
    assert_eq!(next.older_cursor, Some(parse_cursor(&fresh_cursor)));
}

fn snapshot_state(
    values: Vec<bb_store::RowRef>,
    older_cursor: Option<Value>,
    end: i64,
    snapshot: &str,
) -> bb_store::LoadedTimelineState {
    let mut state = loaded_state(values, older_cursor, Some(end));
    state.history_snapshot = Some(snapshot.into());
    state
}

#[test]
fn joins_stable_summary_children_and_preserves_them_across_an_unchanged_latest_refresh() {
    let first = command_row("first", 2);
    let last = command_row("last", 3);
    let older = turn_summary_row("summary", 1, 4, Some(vec![first.clone()]));
    let latest_summary = turn_summary_row("summary", 1, 4, Some(vec![last.clone()]));
    let merged = prepend_older_timeline_rows(
        &rows(std::slice::from_ref(&latest_summary)),
        &rows(&[older]),
    );
    let expected = turn_summary_row("summary", 1, 4, Some(vec![first, last]));
    assert_eq!(plain(&merged), plain_values(&[expected]));

    let mut response =
        timeline_response_json(&[latest_summary], Some(cursor("contents", 1)), Some(4));
    set_page(&mut response, "historySnapshot", json!("snapshot-1"));
    let current = snapshot_state(merged.clone(), None, 4, "snapshot-1");
    let next =
        merge_loaded_timeline_with_latest(&current, &latest(&response), &current.surface_key);
    assert_eq!(plain(&next.rows), plain(&merged));
}

#[test]
fn discards_old_grouping_when_a_new_snapshot_arrives_at_the_same_sequence() {
    let current = snapshot_state(rows(&[user_row("old", 1)]), None, 4, "snapshot-1");
    let replacement = user_row("replacement", 1);
    let mut response = timeline_response_json(std::slice::from_ref(&replacement), None, Some(4));
    set_page(&mut response, "historySnapshot", json!("snapshot-2"));
    let result =
        merge_loaded_timeline_with_latest(&current, &latest(&response), &current.surface_key);
    assert_eq!(plain(&result.rows), plain_values(&[replacement]));
    assert_eq!(result.history_snapshot.as_deref(), Some("snapshot-2"));
}

#[test]
fn keeps_loaded_older_pages_when_a_streaming_refresh_advances_the_snapshot() {
    let walk_cursor = cursor("walk-cursor", 1);
    let older_user = user_row("older-user", 1);
    let older_summary = turn_summary_row("older-summary", 2, 9, None);
    let latest_user = user_row("latest-user", 10);
    let streaming = command_row("streaming", 11);
    let current = snapshot_state(
        rows(&[
            older_user.clone(),
            older_summary.clone(),
            latest_user.clone(),
            streaming.clone(),
        ]),
        Some(walk_cursor.clone()),
        11,
        "snapshot-1",
    );
    let updated_streaming = patch(
        &streaming,
        json!({"sourceSeqEnd": 12, "output": "more output"}),
    );
    let mut response = timeline_response_json(
        &[latest_user.clone(), updated_streaming.clone()],
        Some(cursor("latest-cursor", 10)),
        Some(12),
    );
    set_page(&mut response, "historySnapshot", json!("snapshot-2"));
    set_page(&mut response, "olderRowsSourceSeqEnd", Value::Null);
    let next =
        merge_loaded_timeline_with_latest(&current, &latest(&response), &current.surface_key);
    assert_eq!(
        plain(&next.rows),
        plain_values(&[older_user, older_summary, latest_user, updated_streaming])
    );
    assert_eq!(next.older_cursor, Some(parse_cursor(&walk_cursor)));
    assert_eq!(next.history_snapshot.as_deref(), Some("snapshot-2"));
    assert_eq!(next.latest_window_end_sequence, Some(12));
}

#[test]
fn keeps_the_in_flight_walk_cursor_when_a_refresh_returns_the_same_window_start() {
    let first_cursor = cursor("snapshot-1-cursor", 10);
    let latest_user = user_row("latest-user", 10);
    let current = snapshot_state(
        rows(std::slice::from_ref(&latest_user)),
        Some(first_cursor.clone()),
        10,
        "snapshot-1",
    );
    let mut response = timeline_response_json(
        &[latest_user, command_row("streaming", 11)],
        Some(cursor("snapshot-2-cursor", 10)),
        Some(11),
    );
    set_page(&mut response, "historySnapshot", json!("snapshot-2"));
    set_page(&mut response, "olderRowsSourceSeqEnd", Value::Null);
    let next =
        merge_loaded_timeline_with_latest(&current, &latest(&response), &current.surface_key);
    assert_eq!(next.older_cursor, Some(parse_cursor(&first_cursor)));
    assert_eq!(ids(&next.rows), ["latest-user", "streaming"]);
}

#[test]
fn keeps_the_head_of_a_running_turn_while_its_content_cut_latest_page_slides_forward() {
    let turn_cursor = cursor("turn-cursor-1", 10);
    let prompt = user_row("prompt", 10);
    let commands: Vec<Value> = [11, 12, 13, 14]
        .iter()
        .map(|seq| command_row(&format!("command-{seq}"), *seq))
        .collect();
    let mut loaded = vec![prompt];
    loaded.extend(commands[..3].iter().cloned());
    let current = snapshot_state(rows(&loaded), Some(turn_cursor.clone()), 13, "snapshot-1");
    let mut response =
        timeline_response_json(&commands[1..], Some(cursor("turn-cursor-2", 10)), Some(14));
    set_page(&mut response, "historySnapshot", json!("snapshot-2"));
    set_page(&mut response, "olderRowsSourceSeqEnd", Value::Null);
    set_page(
        &mut response,
        "contentPage",
        json!({"anchorSeq": 10, "start": 2, "end": 5, "total": 5}),
    );
    let next =
        merge_loaded_timeline_with_latest(&current, &latest(&response), &current.surface_key);
    assert_eq!(
        ids(&next.rows),
        [
            "prompt",
            "command-11",
            "command-12",
            "command-13",
            "command-14"
        ]
    );
    assert_eq!(next.older_cursor, Some(parse_cursor(&turn_cursor)));
}

#[test]
fn appends_a_new_conversation_group_that_starts_right_after_the_loaded_tip() {
    let walk_cursor = cursor("walk-cursor", 1);
    let current = snapshot_state(
        rows(&[user_row("older-user", 1), user_row("previous-user", 10)]),
        Some(walk_cursor.clone()),
        20,
        "snapshot-1",
    );
    let mut response = timeline_response_json(
        &[user_row("follow-up", 21)],
        Some(cursor("follow-up", 21)),
        Some(24),
    );
    set_page(&mut response, "historySnapshot", json!("snapshot-2"));
    set_page(&mut response, "olderRowsSourceSeqEnd", Value::Null);
    let next =
        merge_loaded_timeline_with_latest(&current, &latest(&response), &current.surface_key);
    assert_eq!(
        ids(&next.rows),
        ["older-user", "previous-user", "follow-up"]
    );
    assert_eq!(next.older_cursor, Some(parse_cursor(&walk_cursor)));
}

#[test]
fn rebuilds_when_an_advanced_snapshot_shares_no_rows_with_a_window_it_overlaps() {
    let current = snapshot_state(
        rows(&[user_row("older-user", 1), user_row("regrouped", 10)]),
        Some(cursor("walk-cursor", 1)),
        20,
        "snapshot-1",
    );
    let latest_cursor = cursor("latest-cursor", 10);
    let mut response = timeline_response_json(
        &[user_row("replacement", 10)],
        Some(latest_cursor.clone()),
        Some(24),
    );
    set_page(&mut response, "historySnapshot", json!("snapshot-2"));
    set_page(&mut response, "olderRowsSourceSeqEnd", Value::Null);
    let next =
        merge_loaded_timeline_with_latest(&current, &latest(&response), &current.surface_key);
    assert_eq!(ids(&next.rows), ["replacement"]);
    assert_eq!(next.older_cursor, Some(parse_cursor(&latest_cursor)));
}

#[test]
fn rebuilds_when_an_event_after_the_loaded_tip_changes_an_older_conversation_group() {
    let latest_user = user_row("latest-user", 10);
    let current = snapshot_state(
        rows(&[user_row("older-user", 1), latest_user.clone()]),
        Some(cursor("walk-cursor", 1)),
        11,
        "snapshot-1",
    );
    let latest_cursor = cursor("latest-cursor", 10);
    let mut response =
        timeline_response_json(&[latest_user], Some(latest_cursor.clone()), Some(12));
    set_page(&mut response, "historySnapshot", json!("snapshot-2"));
    set_page(&mut response, "olderRowsSourceSeqEnd", json!(12));
    let next =
        merge_loaded_timeline_with_latest(&current, &latest(&response), &current.surface_key);
    assert_eq!(ids(&next.rows), ["latest-user"]);
    assert_eq!(next.older_cursor, Some(parse_cursor(&latest_cursor)));
    assert_eq!(next.history_snapshot.as_deref(), Some("snapshot-2"));
}

#[test]
fn keeps_omitted_row_updates_through_streaming_and_repeat_refreshes() {
    let older_user = user_row("older-user", 1);
    let early_child = command_row("early-command", 3);
    let live_child = command_row("live-command", 4);
    let summary = turn_summary_row(
        "summary",
        2,
        4,
        Some(vec![early_child.clone(), live_child.clone()]),
    );
    let latest_user = user_row("latest-user", 10);
    let walk_cursor = cursor("walk-cursor", 1);
    let current = snapshot_state(
        rows(&[older_user.clone(), summary.clone(), latest_user.clone()]),
        Some(walk_cursor.clone()),
        11,
        "snapshot-1",
    );
    let updated_child = patch(
        &live_child,
        json!({"sourceSeqEnd": 12, "output": "progress"}),
    );
    let new_child = command_row("new-command", 12);
    let mut response = timeline_response_json(
        std::slice::from_ref(&latest_user),
        Some(cursor("latest-cursor", 10)),
        Some(12),
    );
    set_page(&mut response, "historySnapshot", json!("snapshot-2"));
    set_page(&mut response, "olderRowsSourceSeqEnd", json!(1));
    set_page(
        &mut response,
        "olderRowUpdates",
        json!([
            patch(
                &summary,
                json!({"sourceSeqEnd": 12, "children": [updated_child.clone(), new_child.clone()]})
            ),
            command_row_until("unloaded", 5, 12)
        ]),
    );
    let latest = latest(&response);
    let next = merge_loaded_timeline_with_latest(&current, &latest, &current.surface_key);
    let expected = plain_values(&[
        older_user,
        patch(
            &summary,
            json!({"sourceSeqEnd": 12, "children": [early_child, updated_child, new_child]}),
        ),
        latest_user,
    ]);
    assert_eq!(plain(&next.rows), expected);
    let refreshed = merge_loaded_timeline_with_latest(&next, &latest, &current.surface_key);
    assert_eq!(plain(&refreshed.rows), expected);
    assert_eq!(next.older_cursor, Some(parse_cursor(&walk_cursor)));
    assert_eq!(next.latest_window_end_sequence, Some(12));
}

#[test]
fn keeps_loaded_order_when_an_older_page_repeats_the_loaded_conversation_group() {
    let user = row(&user_row("user", 13));
    let assistant = row(&command_row("assistant", 16));
    let merged = prepend_older_timeline_rows(
        &[
            row(&command_row("provider-env", 15)),
            user.clone(),
            assistant.clone(),
        ],
        &[
            row(&command_row("older-assistant", 5)),
            user.clone(),
            assistant,
        ],
    );
    assert_eq!(
        ids(&merged),
        ["older-assistant", "provider-env", "user", "assistant"]
    );
    assert!(Arc::ptr_eq(&merged[2], &user));
}

#[test]
fn replaces_changed_content_even_when_row_identity_and_sequence_are_unchanged() {
    let old = command_row("command", 2);
    let updated = patch(&old, json!({"output": "updated output"}));
    let merge = merge_latest_timeline_rows(&rows(std::slice::from_ref(&updated)), 1, &rows(&[old]));
    assert_eq!(plain(&merge.rows), plain_values(&[updated]));
}

#[test]
fn reuses_delta_row_references_under_same_and_advanced_snapshots() {
    for snapshot in ["snapshot-1", "snapshot-2"] {
        let summary = row(&turn_summary_row(
            "summary",
            2,
            2,
            Some(vec![patch(
                &command_row("unchanged-command", 2),
                json!({"output": "large unchanged tool output"}),
            )]),
        ));
        let tail = command_row("live-command", 3);
        let updated_tail = parse_row(&patch(
            &tail,
            json!({"output": "changed at the same sequence"}),
        ));
        let current = snapshot_state(vec![summary.clone(), row(&tail)], None, 3, "snapshot-1");
        let delta_rows = apply_timeline_delta(
            &current.rows,
            TimelineDelta {
                upsert_rows: vec![updated_tail.clone()],
                row_order: None,
            },
        )
        .expect("delta applies");
        let mut response = timeline_response_json(&[], None, Some(3));
        set_page(&mut response, "historySnapshot", json!(snapshot));
        set_page(&mut response, "olderRowsSourceSeqEnd", Value::Null);
        let latest = latest_with_rows(&response, delta_rows.clone());
        let next = merge_loaded_timeline_with_latest(&current, &latest, &current.surface_key);
        assert!(Arc::ptr_eq(&next.rows[0], &summary), "{snapshot}");
        assert!(Arc::ptr_eq(&next.rows[1], &delta_rows[1]), "{snapshot}");
        assert_eq!(*next.rows[1], updated_tail);
    }
}

#[test]
fn retains_older_children_when_a_delta_updates_a_partially_loaded_summary() {
    let older_child = command_row("older-command", 2);
    let child = command_row("live-command", 3);
    let partial = turn_summary_row("summary", 2, 3, Some(vec![child.clone()]));
    let current = snapshot_state(
        rows(&[patch(
            &partial,
            json!({"children": [older_child.clone(), child.clone()]}),
        )]),
        None,
        3,
        "snapshot-1",
    );
    let updated_child = patch(&child, json!({"output": "new output"}));
    let updated = patch(&partial, json!({"children": [updated_child.clone()]}));
    let delta_rows = apply_timeline_delta(
        &rows(&[partial]),
        TimelineDelta {
            upsert_rows: vec![parse_row(&updated)],
            row_order: None,
        },
    )
    .expect("delta applies");
    let mut response = timeline_response_json(&[], None, Some(3));
    set_page(&mut response, "historySnapshot", json!("snapshot-1"));
    let next = merge_loaded_timeline_with_latest(
        &current,
        &latest_with_rows(&response, delta_rows),
        &current.surface_key,
    );
    assert_eq!(
        plain(&next.rows),
        plain_values(&[patch(
            &updated,
            json!({"children": [older_child, updated_child]})
        )])
    );
}

#[test]
fn still_deduplicates_children_when_both_pages_contain_the_same_summary_object() {
    let child = command_row("repeated-command", 2);
    let summary = row(&turn_summary_row(
        "summary",
        2,
        2,
        Some(vec![child.clone(), child.clone()]),
    ));
    let merged = prepend_older_timeline_rows(
        std::slice::from_ref(&summary),
        std::slice::from_ref(&summary),
    );
    assert_eq!(
        plain(&merged),
        plain_values(&[turn_summary_row("summary", 2, 2, Some(vec![child]))])
    );
}

#[test]
fn prepends_plain_rows_for_turn_detail_pages() {
    let merged = prepend_older_rows(
        &plain_values(&[command_row("b", 2)]),
        &plain_values(&[command_row("a", 1), command_row("b", 2)]),
    );
    let merged_ids: Vec<&str> = merged.iter().map(row_id).collect();
    assert_eq!(merged_ids, ["a", "b"]);
}

#[test]
fn folds_display_and_context_boundary_into_the_surface_key() {
    assert_eq!(
        resolve_loaded_timeline_surface_key("thread-1", None),
        "thread-1"
    );
    let mut response = timeline_response_json(&[], None, Some(0));
    let parsed = parse_response(&response);
    assert_eq!(
        resolve_loaded_timeline_surface_key("thread-1", Some(&parsed)),
        "thread-1:completed-turns:collapse"
    );
    response["completedTurnDisplay"] = json!("flat");
    response["contextBoundarySeq"] = json!(42);
    let parsed = parse_response(&response);
    assert_eq!(
        resolve_loaded_timeline_surface_key("thread-1", Some(&parsed)),
        "thread-1:completed-turns:flat:context-boundary:42"
    );
}

mod delta {
    use super::*;

    fn system(id: &str, seq: i64, title: &str) -> bb_store::RowRef {
        row(&system_row(id, seq, title))
    }

    #[test]
    fn round_trips_an_upsert_and_insert() {
        let prev = vec![system("a", 1, "t"), system("b", 2, "t")];
        let current = vec![
            system("a", 1, "t"),
            system("b", 2, "changed"),
            system("c", 3, "t"),
        ];
        let delta = compute_timeline_row_delta(&prev, &current);
        let upserted: Vec<&str> = delta.upsert_rows.iter().map(row_id).collect();
        assert_eq!(upserted, ["b", "c"]);
        assert_eq!(
            plain(&apply_timeline_delta(&prev, delta).unwrap()),
            plain(&current)
        );
    }

    #[test]
    fn round_trips_a_removal() {
        let prev = vec![
            system("a", 1, "t"),
            system("b", 2, "t"),
            system("c", 3, "t"),
        ];
        let current = vec![system("a", 1, "t"), system("c", 3, "t")];
        let delta = compute_timeline_row_delta(&prev, &current);
        assert!(delta.upsert_rows.is_empty());
        assert_eq!(
            plain(&apply_timeline_delta(&prev, delta).unwrap()),
            plain(&current)
        );
    }

    #[test]
    fn preserves_unchanged_row_identity() {
        let a = system("a", 1, "t");
        let prev = vec![a.clone(), system("b", 2, "t")];
        let current = vec![a.clone(), system("b", 2, "changed")];
        let merged =
            apply_timeline_delta(&prev, compute_timeline_row_delta(&prev, &current)).unwrap();
        assert!(Arc::ptr_eq(&merged[0], &a));
    }

    #[test]
    fn omits_row_order_when_membership_and_ordering_are_unchanged() {
        let prev = vec![system("a", 1, "t"), system("b", 2, "t")];
        let current = vec![system("a", 1, "t"), system("b", 2, "changed")];
        let delta = compute_timeline_row_delta(&prev, &current);
        assert!(delta.row_order.is_none());
        assert_eq!(
            plain(&apply_timeline_delta(&prev, delta).unwrap()),
            plain(&current)
        );
    }

    #[test]
    fn returns_none_when_the_base_is_stale() {
        assert!(
            apply_timeline_delta(
                &[system("a", 1, "t")],
                TimelineDelta {
                    upsert_rows: Vec::new(),
                    row_order: Some(vec!["a".into(), "z".into()]),
                },
            )
            .is_none()
        );
    }
}
