use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use bb_wire::timeline::{
    ConversationRole, ConversationTurnRequest, DelegationWorkRow, ThreadTimelineResponse,
    TimelineDelta, TimelinePageMetadata, TimelinePaginationCursor, TimelineRow, TimelineRowBase,
    TimelineWorkRow, TurnRow,
};

pub type RowRef = Arc<TimelineRow>;

pub const OPTIMISTIC_TIMELINE_ROW_ID_PREFIX: &str = "optimistic-user-";

pub fn is_optimistic_timeline_row_id(id: &str) -> bool {
    id.starts_with(OPTIMISTIC_TIMELINE_ROW_ID_PREFIX)
}

pub fn row_id(row: &TimelineRow) -> &str {
    row.id().unwrap_or_default()
}

fn row_source_seq_end(row: &TimelineRow) -> i64 {
    row.source_seq_end().unwrap_or(0)
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct LatestTimeline {
    pub response: ThreadTimelineResponse,
    pub rows: Vec<RowRef>,
}

impl LatestTimeline {
    pub fn from_response(mut response: ThreadTimelineResponse) -> Self {
        let rows = std::mem::take(&mut response.rows)
            .into_iter()
            .map(Arc::new)
            .collect();
        response.delta = None;
        Self { response, rows }
    }

    pub fn with_rows(mut response: ThreadTimelineResponse, rows: Vec<RowRef>) -> Self {
        response.rows.clear();
        response.delta = None;
        Self { response, rows }
    }

    pub fn max_seq(&self) -> i64 {
        self.response.max_seq
    }

    pub fn page(&self) -> &TimelinePageMetadata {
        &self.response.timeline_page
    }

    pub fn window_start_sequence(&self) -> i64 {
        self.page()
            .older_cursor
            .as_ref()
            .map_or(0, |cursor| cursor.anchor_seq)
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct LoadedTimelineState {
    pub history_snapshot: Option<String>,
    pub latest_window_end_sequence: Option<i64>,
    pub older_cursor: Option<TimelinePaginationCursor>,
    pub rows: Vec<RowRef>,
    pub surface_key: String,
}

impl LoadedTimelineState {
    pub fn empty(surface_key: impl Into<String>) -> Self {
        Self {
            surface_key: surface_key.into(),
            ..Self::default()
        }
    }

    pub fn from_latest(
        latest: &LatestTimeline,
        surface_key: &str,
        rows: Option<Vec<RowRef>>,
    ) -> Self {
        Self {
            history_snapshot: latest.page().history_snapshot.clone(),
            latest_window_end_sequence: Some(latest.max_seq()),
            older_cursor: latest.page().older_cursor.clone(),
            rows: rows.unwrap_or_else(|| latest.rows.clone()),
            surface_key: surface_key.to_owned(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MergeLatestResult {
    pub can_merge: bool,
    pub rows: Vec<RowRef>,
}

pub fn resolve_loaded_timeline_surface_key(
    base_surface_key: &str,
    latest: Option<&ThreadTimelineResponse>,
) -> String {
    let Some(latest) = latest else {
        return base_surface_key.to_owned();
    };
    let display = format!(
        "{base_surface_key}:completed-turns:{}",
        latest.completed_turn_display.as_str()
    );
    match latest.context_boundary_seq {
        None => display,
        Some(boundary) => format!("{display}:context-boundary:{boundary}"),
    }
}

pub fn are_timeline_pagination_cursors_equal(
    left: Option<&TimelinePaginationCursor>,
    right: Option<&TimelinePaginationCursor>,
) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => {
            left.anchor_seq == right.anchor_seq && left.anchor_id == right.anchor_id
        }
        (None, None) => true,
        _ => false,
    }
}

pub fn same_row_refs(left: &[RowRef], right: &[RowRef]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(left, right)| Arc::ptr_eq(left, right))
}

trait RowHandle: Clone {
    fn row(&self) -> &TimelineRow;
    fn wrap(row: TimelineRow) -> Self;
}

impl RowHandle for RowRef {
    fn row(&self) -> &TimelineRow {
        self
    }

    fn wrap(row: TimelineRow) -> Self {
        Arc::new(row)
    }
}

impl RowHandle for TimelineRow {
    fn row(&self) -> &TimelineRow {
        self
    }

    fn wrap(row: TimelineRow) -> Self {
        row
    }
}

fn prepend_rows<T: RowHandle>(loaded_rows: &[T], older_rows: &[T]) -> (Vec<T>, bool) {
    let mut changed = false;
    let mut loaded_ids: HashSet<&str> = HashSet::with_capacity(loaded_rows.len());
    let mut unique_loaded: Vec<&T> = Vec::with_capacity(loaded_rows.len());
    for row in loaded_rows {
        if loaded_ids.insert(row_id(row.row())) {
            unique_loaded.push(row);
        } else {
            changed = true;
        }
    }
    let mut older_by_id: HashMap<&str, &T> = HashMap::with_capacity(older_rows.len());
    let mut rows: Vec<T> = Vec::with_capacity(older_rows.len() + unique_loaded.len());
    for row in older_rows {
        let id = row_id(row.row());
        if older_by_id.contains_key(id) {
            continue;
        }
        older_by_id.insert(id, row);
        if !loaded_ids.contains(id) {
            rows.push(row.clone());
            changed = true;
        }
    }
    for loaded in unique_loaded {
        let joined = older_by_id
            .get(row_id(loaded.row()))
            .and_then(|older| join_older_row_children(older.row(), loaded.row()));
        match joined {
            Some(joined) => {
                rows.push(T::wrap(joined));
                changed = true;
            }
            None => rows.push(loaded.clone()),
        }
    }
    (rows, changed)
}

fn join_older_row_children(older: &TimelineRow, loaded: &TimelineRow) -> Option<TimelineRow> {
    match (older, loaded) {
        (TimelineRow::Turn(older), TimelineRow::Turn(loaded)) => {
            let (Some(older_children), Some(loaded_children)) = (&older.children, &loaded.children)
            else {
                return None;
            };
            let (children, changed) = prepend_rows(loaded_children, older_children);
            if !changed {
                return None;
            }
            Some(TimelineRow::Turn(Box::new(TurnRow {
                base: loaded.base.clone(),
                status: loaded.status,
                summary_count: loaded.summary_count,
                completed_at: loaded.completed_at,
                children: Some(children),
            })))
        }
        (
            TimelineRow::Work(TimelineWorkRow::Delegation(older)),
            TimelineRow::Work(TimelineWorkRow::Delegation(loaded)),
        ) => {
            let (Some(older_children), Some(loaded_children)) =
                (&older.child_rows, &loaded.child_rows)
            else {
                return None;
            };
            let (child_rows, changed) = prepend_rows(loaded_children, older_children);
            if !changed {
                return None;
            }
            let joined = DelegationWorkRow {
                child_rows: Some(child_rows),
                base: loaded.base.clone(),
                status: loaded.status,
                call_id: loaded.call_id.clone(),
                tool_name: loaded.tool_name.clone(),
                child_ref: loaded.child_ref.clone(),
                background: loaded.background,
                subagent_type: loaded.subagent_type.clone(),
                description: loaded.description.clone(),
                output: loaded.output.clone(),
                completed_at: loaded.completed_at,
                presentation: loaded.presentation.clone(),
            };
            Some(TimelineRow::Work(TimelineWorkRow::Delegation(Box::new(
                joined,
            ))))
        }
        _ => None,
    }
}

pub fn prepend_older_timeline_rows(loaded_rows: &[RowRef], older_rows: &[RowRef]) -> Vec<RowRef> {
    prepend_rows(loaded_rows, older_rows).0
}

pub fn prepend_older_rows(
    loaded_rows: &[TimelineRow],
    older_rows: &[TimelineRow],
) -> Vec<TimelineRow> {
    prepend_rows(loaded_rows, older_rows).0
}

fn join_single(older: &RowRef, loaded: &RowRef) -> RowRef {
    match join_older_row_children(older, loaded) {
        Some(joined) => Arc::new(joined),
        None => loaded.clone(),
    }
}

type IdentitySignature<'a> = (
    &'static str,
    &'a TimelineRowBase,
    Option<&'a ConversationTurnRequest>,
);

fn identity_signature(row: &TimelineRow) -> Option<IdentitySignature<'_>> {
    let kind = match row {
        TimelineRow::Conversation(_) => "conversation",
        TimelineRow::Work(_) => "work",
        TimelineRow::System(_) => "system",
        TimelineRow::Turn(_) => "turn",
        TimelineRow::Unknown(_) => return None,
    };
    let turn_request = match row {
        TimelineRow::Conversation(row) if row.role == ConversationRole::User => {
            row.turn_request.as_ref()
        }
        _ => None,
    };
    Some((kind, row.base()?, turn_request))
}

fn rows_have_same_content(previous: &TimelineRow, next: &TimelineRow) -> bool {
    match (identity_signature(previous), identity_signature(next)) {
        (Some(left), Some(right)) if left != right => false,
        _ => previous == next,
    }
}

fn preserve_timeline_row_identity(next_rows: &[RowRef], previous_rows: &[RowRef]) -> Vec<RowRef> {
    let previous_by_id: HashMap<&str, &RowRef> =
        previous_rows.iter().map(|row| (row_id(row), row)).collect();
    next_rows
        .iter()
        .map(|row| match previous_by_id.get(row_id(row)) {
            Some(previous) if Arc::ptr_eq(previous, row) => row.clone(),
            Some(previous) if rows_have_same_content(previous, row) => (*previous).clone(),
            _ => row.clone(),
        })
        .collect()
}

pub fn merge_latest_timeline_rows(
    latest_rows: &[RowRef],
    latest_window_start_sequence: i64,
    loaded_rows: &[RowRef],
) -> MergeLatestResult {
    let filtered: Vec<RowRef>;
    let loaded_rows: &[RowRef] = if loaded_rows
        .iter()
        .any(|row| is_optimistic_timeline_row_id(row_id(row)))
    {
        filtered = loaded_rows
            .iter()
            .filter(|row| !is_optimistic_timeline_row_id(row_id(row)))
            .cloned()
            .collect();
        &filtered
    } else {
        loaded_rows
    };

    let latest = preserve_timeline_row_identity(latest_rows, loaded_rows);
    if loaded_rows.is_empty() {
        return MergeLatestResult {
            can_merge: true,
            rows: latest,
        };
    }

    let latest_by_id: HashMap<&str, &RowRef> =
        latest.iter().map(|row| (row_id(row), row)).collect();
    let rows_to_retain: Vec<&RowRef> = loaded_rows
        .iter()
        .filter(|row| {
            row_source_seq_end(row) < latest_window_start_sequence
                || latest_by_id.contains_key(row_id(row))
        })
        .collect();
    let retained_ids: HashSet<&str> = rows_to_retain.iter().map(|row| row_id(row)).collect();
    let loaded_common_ids = rows_to_retain
        .iter()
        .map(|row| row_id(row))
        .filter(|id| latest_by_id.contains_key(id));
    let latest_common_ids = latest
        .iter()
        .map(|row| row_id(row))
        .filter(|id| retained_ids.contains(id));
    if !loaded_common_ids.eq(latest_common_ids) {
        return MergeLatestResult {
            can_merge: false,
            rows: latest,
        };
    }

    let mut rows_before_shared_id: HashMap<&str, Vec<RowRef>> = HashMap::new();
    let mut pending_rows: Vec<RowRef> = Vec::new();
    for row in &latest {
        let id = row_id(row);
        if !retained_ids.contains(id) {
            pending_rows.push(row.clone());
            continue;
        }
        if !pending_rows.is_empty() {
            rows_before_shared_id.insert(id, std::mem::take(&mut pending_rows));
        }
    }

    let mut rows: Vec<RowRef> = Vec::with_capacity(rows_to_retain.len() + latest.len());
    for row in rows_to_retain {
        let id = row_id(row);
        if let Some(before) = rows_before_shared_id.remove(id) {
            rows.extend(before);
        }
        rows.push(
            latest_by_id
                .get(id)
                .map_or_else(|| row.clone(), |row| (*row).clone()),
        );
    }
    rows.extend(pending_rows);
    MergeLatestResult {
        can_merge: true,
        rows,
    }
}

fn timeline_windows_are_contiguous(current: &LoadedTimelineState, latest: &LatestTimeline) -> bool {
    current
        .latest_window_end_sequence
        .is_some_and(|end| latest.max_seq() >= end && latest.window_start_sequence() <= end + 1)
}

fn merge_loaded_timeline_older_cursor(
    current: Option<&TimelinePaginationCursor>,
    latest: Option<&TimelinePaginationCursor>,
) -> Option<TimelinePaginationCursor> {
    match (current, latest) {
        (Some(current), Some(latest)) => Some(if latest.anchor_seq < current.anchor_seq {
            latest.clone()
        } else {
            current.clone()
        }),
        _ => None,
    }
}

fn apply_older_row_updates(rows: &[RowRef], updates: Option<&[TimelineRow]>) -> Vec<RowRef> {
    let Some(updates) = updates else {
        return rows.to_vec();
    };
    let updates_by_id: HashMap<&str, &TimelineRow> =
        updates.iter().map(|row| (row_id(row), row)).collect();
    rows.iter()
        .map(|row| match updates_by_id.get(row_id(row)) {
            None => row.clone(),
            Some(update) => join_single(row, &Arc::new((*update).clone())),
        })
        .collect()
}

fn merge_snapshot_timeline_rows(
    current: &LoadedTimelineState,
    latest_rows: &[RowRef],
    latest: &LatestTimeline,
) -> MergeLatestResult {
    let latest_window_start_sequence = latest.window_start_sequence();
    let page = latest.page();
    let current_end = current.latest_window_end_sequence.unwrap_or(0);
    let cannot_merge = || MergeLatestResult {
        can_merge: false,
        rows: latest_rows.to_vec(),
    };
    match page.older_rows_source_seq_end {
        None => return cannot_merge(),
        Some(Some(end)) if end > current_end => return cannot_merge(),
        Some(_) => {}
    }
    let current_rows = apply_older_row_updates(&current.rows, page.older_row_updates.as_deref());
    if page.older_cursor.is_none() {
        return merge_latest_timeline_rows(
            latest_rows,
            latest_window_start_sequence,
            &current_rows,
        );
    }
    let loaded_rows: Vec<RowRef> = current_rows
        .into_iter()
        .filter(|row| !is_optimistic_timeline_row_id(row_id(row)))
        .collect();
    let latest_row_ids: HashSet<&str> = latest_rows.iter().map(|row| row_id(row)).collect();
    let Some(first_covered_index) = loaded_rows
        .iter()
        .position(|row| latest_row_ids.contains(row_id(row)))
    else {
        if latest_window_start_sequence > current_end {
            let mut rows = loaded_rows;
            rows.extend(latest_rows.iter().cloned());
            return MergeLatestResult {
                can_merge: true,
                rows,
            };
        }
        return cannot_merge();
    };
    let covered = merge_latest_timeline_rows(latest_rows, 0, &loaded_rows[first_covered_index..]);
    if !covered.can_merge {
        return covered;
    }
    let mut rows = loaded_rows[..first_covered_index].to_vec();
    rows.extend(covered.rows);
    MergeLatestResult {
        can_merge: true,
        rows,
    }
}

pub fn merge_loaded_timeline_with_latest(
    current: &LoadedTimelineState,
    latest: &LatestTimeline,
    surface_key: &str,
) -> LoadedTimelineState {
    let latest_snapshot = latest.page().history_snapshot.as_ref();
    if current.surface_key != surface_key
        || current.history_snapshot.is_none() != latest_snapshot.is_none()
        || !timeline_windows_are_contiguous(current, latest)
    {
        return LoadedTimelineState::from_latest(latest, surface_key, None);
    }

    let latest_rows: Vec<RowRef> = if current.history_snapshot.is_none() {
        latest.rows.clone()
    } else {
        let current_by_id: HashMap<&str, &RowRef> =
            current.rows.iter().map(|row| (row_id(row), row)).collect();
        latest
            .rows
            .iter()
            .map(|row| match current_by_id.get(row_id(row)) {
                None => row.clone(),
                Some(loaded) => join_single(loaded, row),
            })
            .collect()
    };
    let merge = if current.history_snapshot.as_ref() == latest_snapshot
        && latest.page().older_row_updates.is_none()
    {
        merge_latest_timeline_rows(&latest_rows, latest.window_start_sequence(), &current.rows)
    } else {
        merge_snapshot_timeline_rows(current, &latest_rows, latest)
    };
    if !merge.can_merge {
        return LoadedTimelineState::from_latest(latest, surface_key, None);
    }
    LoadedTimelineState {
        history_snapshot: latest_snapshot.cloned(),
        latest_window_end_sequence: Some(latest.max_seq()),
        older_cursor: merge_loaded_timeline_older_cursor(
            current.older_cursor.as_ref(),
            latest.page().older_cursor.as_ref(),
        ),
        rows: merge.rows,
        surface_key: current.surface_key.clone(),
    }
}

pub fn recover_loaded_timeline_after_stale_cursor(
    current: &LoadedTimelineState,
    latest: &LatestTimeline,
    surface_key: &str,
) -> LoadedTimelineState {
    if current.surface_key != surface_key
        || current.history_snapshot != latest.page().history_snapshot
    {
        return LoadedTimelineState::from_latest(latest, surface_key, None);
    }
    let merge =
        merge_latest_timeline_rows(&latest.rows, latest.window_start_sequence(), &current.rows);
    if !merge.can_merge {
        return LoadedTimelineState::from_latest(latest, surface_key, None);
    }
    LoadedTimelineState::from_latest(latest, surface_key, Some(merge.rows))
}

pub fn apply_timeline_delta(prev_rows: &[RowRef], delta: TimelineDelta) -> Option<Vec<RowRef>> {
    let mut by_id: HashMap<String, RowRef> = prev_rows
        .iter()
        .map(|row| (row_id(row).to_owned(), row.clone()))
        .collect();
    for row in delta.upsert_rows {
        by_id.insert(row_id(&row).to_owned(), Arc::new(row));
    }
    match delta.row_order {
        Some(order) => order.iter().map(|id| by_id.get(id).cloned()).collect(),
        None => prev_rows
            .iter()
            .map(|row| by_id.get(row_id(row)).cloned())
            .collect(),
    }
}

pub fn compute_timeline_row_delta(prev_rows: &[RowRef], current_rows: &[RowRef]) -> TimelineDelta {
    let prev_by_id: HashMap<&str, &RowRef> =
        prev_rows.iter().map(|row| (row_id(row), row)).collect();
    let mut upsert_rows = Vec::new();
    let mut row_order = Vec::with_capacity(current_rows.len());
    let mut order_changed = prev_rows.len() != current_rows.len();
    for (index, row) in current_rows.iter().enumerate() {
        let id = row_id(row);
        row_order.push(id.to_owned());
        if prev_rows.get(index).map(|prev| row_id(prev)) != Some(id) {
            order_changed = true;
        }
        let unchanged = prev_by_id
            .get(id)
            .is_some_and(|prev| Arc::ptr_eq(prev, row) || ***prev == **row);
        if !unchanged {
            upsert_rows.push((**row).clone());
        }
    }
    TimelineDelta {
        upsert_rows,
        row_order: order_changed.then_some(row_order),
    }
}
