use std::cell::{Cell, RefCell};

use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Position, Rect};

use crate::app::{AppState, Effect, Focus, Mode};
use crate::panel::PanelTab;
use crate::sidebar::SidebarRow;

pub const WHEEL_LINES: isize = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneArea {
    pub thread_id: String,
    pub pane: Option<usize>,
    pub area: Rect,
    pub timeline: Rect,
    pub composer: Option<Rect>,
    pub interaction: Option<Rect>,
}

#[derive(Debug, Default)]
pub struct MouseAreas {
    pub sidebar: Cell<Option<(Rect, usize)>>,
    pub panes: RefCell<Vec<PaneArea>>,
    pub panel: Cell<Option<Rect>>,
}

impl MouseAreas {
    pub fn clear(&self) {
        self.sidebar.set(None);
        self.panes.borrow_mut().clear();
        self.panel.set(None);
    }

    pub fn record_pane(&self, pane: PaneArea) {
        self.panes.borrow_mut().push(pane);
    }
}

fn contains(rect: Rect, column: u16, row: u16) -> bool {
    rect.contains(Position::new(column, row))
}

fn overlay_open(state: &AppState) -> bool {
    state.result_pane.is_some()
        || state.palette.is_some()
        || state.settings.is_some()
        || state.automations.is_some()
        || state.panel.confirm.is_some()
        || state.mode != Mode::Normal
}

fn wheel_delta(kind: MouseEventKind) -> Option<isize> {
    match kind {
        MouseEventKind::ScrollDown => Some(WHEEL_LINES),
        MouseEventKind::ScrollUp => Some(-WHEEL_LINES),
        _ => None,
    }
}

fn wheel_keys(state: &mut AppState, delta: isize) -> Vec<Effect> {
    let code = if delta > 0 {
        KeyCode::Down
    } else {
        KeyCode::Up
    };
    let mut effects = Vec::new();
    for _ in 0..delta.unsigned_abs() {
        effects.extend(crate::app::handle_key(state, KeyEvent::from(code)));
    }
    effects
}

pub(crate) fn handle_mouse(state: &mut AppState, event: MouseEvent) -> Vec<Effect> {
    let delta = wheel_delta(event.kind);
    let click = matches!(event.kind, MouseEventKind::Down(MouseButton::Left));
    if delta.is_none() && !click {
        return Vec::new();
    }
    if overlay_open(state) {
        return match delta {
            Some(_) if matches!(state.mode, Mode::Ex(_)) => Vec::new(),
            Some(delta) => wheel_keys(state, delta.signum()),
            None => Vec::new(),
        };
    }
    let (column, row) = (event.column, event.row);
    if let Some(panel) = state.mouse_areas.panel.get()
        && contains(panel, column, row)
    {
        return panel_mouse(state, delta);
    }
    if let Some((list, offset)) = state.mouse_areas.sidebar.get()
        && contains(list, column, row)
    {
        return sidebar_mouse(state, delta, offset + usize::from(row - list.y));
    }
    let pane = state
        .mouse_areas
        .panes
        .borrow()
        .iter()
        .find(|pane| contains(pane.area, column, row))
        .cloned();
    match pane {
        Some(pane) => pane_mouse(state, &pane, delta, row),
        None => Vec::new(),
    }
}

fn sidebar_mouse(state: &mut AppState, delta: Option<isize>, index: usize) -> Vec<Effect> {
    if let Some(delta) = delta {
        state.move_selection(delta);
        return Vec::new();
    }
    let Some(row) = state.rows.get(index) else {
        state.focus = Focus::Sidebar;
        return Vec::new();
    };
    let thread_id = match row {
        SidebarRow::Thread(thread) => Some(thread.id.clone()),
        SidebarRow::Group(_) => None,
    };
    state.focus = Focus::Sidebar;
    state.selected = index;
    match thread_id {
        Some(thread_id) => {
            let effects = state.open(thread_id);
            state.focus = Focus::Main;
            effects
        }
        None => {
            state.toggle_group(None);
            Vec::new()
        }
    }
}

fn pane_mouse(
    state: &mut AppState,
    pane: &PaneArea,
    delta: Option<isize>,
    row: u16,
) -> Vec<Effect> {
    if let Some(delta) = delta {
        if !contains(pane.timeline, pane.timeline.x, row) {
            return Vec::new();
        }
        let loads = match state.timelines.get_mut(&pane.thread_id) {
            Some(timeline) => timeline.scroll_by(delta),
            None => return Vec::new(),
        };
        return state.load_effects_for(&pane.thread_id, loads);
    }
    let mut effects = Vec::new();
    if state.open_thread.as_deref() != Some(pane.thread_id.as_str()) {
        match pane.pane {
            Some(index) => effects.extend(state.focus_pane(index)),
            None => return effects,
        }
    }
    if pane
        .composer
        .is_some_and(|area| contains(area, area.x, row))
        && state.composer_available()
    {
        state.focus = Focus::Composer;
        if let Some(composer) = state.composers.get_mut(&pane.thread_id) {
            composer.enter_insert();
        }
        return effects;
    }
    if pane
        .interaction
        .is_some_and(|area| contains(area, area.x, row))
        && state.open_interaction_panel()
    {
        state.focus = Focus::Interaction;
        return effects;
    }
    state.focus = Focus::Main;
    if contains(pane.timeline, pane.timeline.x, row)
        && let Some(timeline) = state.timelines.get_mut(&pane.thread_id)
    {
        timeline.select_at_line(usize::from(row - pane.timeline.y));
    }
    effects
}

fn panel_mouse(state: &mut AppState, delta: Option<isize>) -> Vec<Effect> {
    let Some(delta) = delta else {
        state.focus = Focus::Panel;
        return Vec::new();
    };
    if state.panel.tab == PanelTab::Terminal {
        if let Some(view) = state.terminals.active_view_mut() {
            view.scroll_by(-delta);
        }
        return Vec::new();
    }
    let previous = state.focus;
    state.focus = Focus::Panel;
    let code = if delta > 0 {
        KeyCode::Down
    } else {
        KeyCode::Up
    };
    let mut effects = Vec::new();
    for _ in 0..delta.unsigned_abs() {
        effects.extend(
            crate::panel::handle_panel_key(state, KeyEvent::from(code)).unwrap_or_default(),
        );
    }
    if state.focus == Focus::Panel {
        state.focus = previous;
    }
    effects
}
