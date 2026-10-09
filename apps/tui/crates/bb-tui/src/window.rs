use crate::app::{AppState, Effect, Focus, ToastLevel};
use crate::keymap::Direction;
use crate::settings::SettingsFocus;

pub type WindowId = (Focus, Option<String>);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Zoom {
    pub sidebar: bool,
    pub panel: bool,
    pub split_maximized: bool,
}

pub const RESIZE_STEP: i16 = 4;

impl AppState {
    pub fn windows(&self) -> Vec<WindowId> {
        let mut windows = Vec::new();
        if self.sidebar_visible {
            windows.push((Focus::Sidebar, None));
        }
        if self.new_thread.is_some() {
            windows.push((Focus::NewThread, None));
            if self.panel_visible {
                windows.push((Focus::Panel, None));
            }
            return windows;
        }
        let threads = if self.is_split() && !self.split_maximized {
            self.splits.clone()
        } else {
            self.open_thread.iter().cloned().collect()
        };
        if threads.is_empty() {
            windows.push((Focus::Main, None));
        }
        for thread_id in threads {
            windows.push((Focus::Main, Some(thread_id.clone())));
            if self.interaction_panel_visible(&thread_id) {
                windows.push((Focus::Interaction, Some(thread_id)));
            } else if !self.thread_archived(&thread_id) {
                windows.push((Focus::Composer, Some(thread_id)));
            }
        }
        if self.panel_visible {
            windows.push((Focus::Panel, None));
        }
        windows
    }

    pub(crate) fn current_window(&self) -> WindowId {
        match self.focus {
            Focus::Main | Focus::Composer | Focus::Interaction => {
                (self.focus, self.open_thread.clone())
            }
            other => (other, None),
        }
    }

    pub(crate) fn focus_window(&mut self, window: WindowId) -> Vec<Effect> {
        let (focus, thread) = window;
        let mut effects = Vec::new();
        if let Some(thread_id) = thread
            && self.open_thread.as_deref() != Some(thread_id.as_str())
        {
            effects = self.open(thread_id);
        }
        self.focus = focus;
        effects
    }

    pub(crate) fn cycle_focus(&mut self, forward: bool, count: usize) -> Vec<Effect> {
        let windows = self.windows();
        if windows.is_empty() {
            return Vec::new();
        }
        let current = self.current_window();
        let index = windows
            .iter()
            .position(|window| *window == current)
            .unwrap_or(0);
        let len = windows.len();
        let step = count % len;
        let next = if forward {
            (index + step) % len
        } else {
            (index + len - step) % len
        };
        self.focus_window(windows[next].clone())
    }

    fn column_of(window: &WindowId) -> (Focus, Option<String>) {
        match window.0 {
            Focus::Composer | Focus::Interaction => (Focus::Main, window.1.clone()),
            other => (other, window.1.clone()),
        }
    }

    pub fn focus_direction(&mut self, direction: Direction) -> Vec<Effect> {
        if let Some(view) = self.settings.as_mut() {
            match direction {
                Direction::Left => view.focus = SettingsFocus::Nav,
                Direction::Right => view.focus = SettingsFocus::Content,
                _ => {}
            }
            return Vec::new();
        }
        if self.automations.is_some() || self.result_pane.is_some() {
            return Vec::new();
        }
        if self.focus == Focus::NewThread
            && let Some(form) = self.new_thread.as_mut()
        {
            match direction {
                Direction::Up if form.prompt_active => {
                    form.prompt_active = false;
                    return Vec::new();
                }
                Direction::Down if !form.prompt_active => {
                    form.field = crate::new_thread::FormField::Prompt;
                    form.prompt_active = true;
                    return Vec::new();
                }
                _ => {}
            }
        }
        let windows = self.windows();
        let current = self.current_window();
        match direction {
            Direction::Down => {
                if current.0 == Focus::Main
                    && let Some(below) = windows.iter().find(|window| {
                        matches!(window.0, Focus::Composer | Focus::Interaction)
                            && window.1 == current.1
                    })
                {
                    return self.focus_window(below.clone());
                }
                Vec::new()
            }
            Direction::Up => {
                if matches!(current.0, Focus::Composer | Focus::Interaction) {
                    return self.focus_window((Focus::Main, current.1));
                }
                Vec::new()
            }
            Direction::Left | Direction::Right => {
                let mut columns: Vec<(Focus, Option<String>)> = Vec::new();
                for window in &windows {
                    let column = Self::column_of(window);
                    if !columns.contains(&column) {
                        columns.push(column);
                    }
                }
                let here = Self::column_of(&current);
                let Some(index) = columns.iter().position(|column| *column == here) else {
                    return windows
                        .first()
                        .cloned()
                        .map(|window| self.focus_window(window))
                        .unwrap_or_default();
                };
                let target = match direction {
                    Direction::Left if index > 0 => index - 1,
                    Direction::Right if index + 1 < columns.len() => index + 1,
                    _ => return Vec::new(),
                };
                let column = columns[target].clone();
                let keep_part = matches!(current.0, Focus::Composer | Focus::Interaction);
                let window = if column.0 == Focus::Main && keep_part {
                    windows
                        .iter()
                        .find(|window| {
                            matches!(window.0, Focus::Composer | Focus::Interaction)
                                && window.1 == column.1
                        })
                        .cloned()
                        .unwrap_or(column)
                } else {
                    column
                };
                self.focus_window(window)
            }
        }
    }

    pub(crate) fn focus_last_window(&mut self) -> Vec<Effect> {
        let Some(previous) = self.prev_window.clone() else {
            return Vec::new();
        };
        if !self.windows().contains(&previous) {
            self.keys.error("The previous window is gone");
            return Vec::new();
        }
        self.focus_window(previous)
    }

    pub(crate) fn focus_edge_window(&mut self, first: bool) -> Vec<Effect> {
        let windows = self.windows();
        let window = if first {
            windows.first()
        } else {
            windows.last()
        };
        match window.cloned() {
            Some(window) => self.focus_window(window),
            None => Vec::new(),
        }
    }

    pub(crate) fn close_window(&mut self) -> Vec<Effect> {
        if self.settings.is_some() {
            self.close_settings();
            return Vec::new();
        }
        if self.automations.is_some() {
            self.close_automations();
            return Vec::new();
        }
        if self.result_pane.is_some() {
            self.result_pane = None;
            return Vec::new();
        }
        match self.focus {
            Focus::Panel if self.panel_visible => {
                self.panel_visible = false;
                self.panel.maximized = false;
                self.focus = Focus::Main;
                Vec::new()
            }
            Focus::Sidebar if self.sidebar_visible => {
                self.sidebar_visible = false;
                self.focus = Focus::Main;
                Vec::new()
            }
            Focus::NewThread => {
                self.close_new_thread();
                Vec::new()
            }
            _ if self.is_split() => self.close_split_pane(),
            _ => {
                self.keys
                    .error("E444: Cannot close last window · quit with SPC q q or :q");
                Vec::new()
            }
        }
    }

    pub(crate) fn can_close_window(&self) -> bool {
        self.settings.is_some()
            || self.automations.is_some()
            || self.result_pane.is_some()
            || (self.focus == Focus::Panel && self.panel_visible)
            || (self.focus == Focus::Sidebar && self.sidebar_visible)
            || self.focus == Focus::NewThread
            || self.is_split()
    }

    pub(crate) fn window_only(&mut self) -> Vec<Effect> {
        if self.focus == Focus::Panel && self.panel_visible {
            self.toggle_panel_maximized();
            return self.sync_panel();
        }
        if self.zoom.is_some() {
            return self.restore_zoom();
        }
        self.zoom = Some(Zoom {
            sidebar: self.sidebar_visible,
            panel: self.panel_visible,
            split_maximized: self.split_maximized,
        });
        self.sidebar_visible = false;
        self.panel_visible = false;
        if self.is_split() {
            self.split_maximized = true;
        }
        if matches!(self.focus, Focus::Sidebar | Focus::Panel) {
            self.focus = Focus::Main;
        }
        Vec::new()
    }

    fn restore_zoom(&mut self) -> Vec<Effect> {
        let Some(zoom) = self.zoom.take() else {
            return Vec::new();
        };
        self.sidebar_visible = zoom.sidebar;
        self.panel_visible = zoom.panel;
        self.split_maximized = zoom.split_maximized;
        self.sync_panel()
    }

    pub(crate) fn restore_windows(&mut self) -> Vec<Effect> {
        let effects = self.restore_zoom();
        self.split_maximized = false;
        self.panel.maximized = false;
        self.sidebar_width_delta = 0;
        self.panel_width_delta = 0;
        effects
    }

    pub(crate) fn swap_pane(&mut self) -> Vec<Effect> {
        if !self.is_split() {
            self.keys.error("Only one pane is open");
            return Vec::new();
        }
        let index = self.focused_pane();
        let next = (index + 1) % self.splits.len();
        self.splits.swap(index, next);
        vec![self.split_watch()]
    }

    pub(crate) fn rotate_panes(&mut self, count: usize) -> Vec<Effect> {
        if !self.is_split() {
            self.keys.error("Only one pane is open");
            return Vec::new();
        }
        let len = self.splits.len();
        self.splits.rotate_right(count % len);
        vec![self.split_watch()]
    }

    pub(crate) fn move_pane_far(&mut self, left: bool) -> Vec<Effect> {
        if !self.is_split() {
            self.keys.error("Only one pane is open");
            return Vec::new();
        }
        let index = self.focused_pane();
        let thread = self.splits.remove(index);
        if left {
            self.splits.insert(0, thread);
        } else {
            self.splits.push(thread);
        }
        vec![self.split_watch()]
    }

    pub(crate) fn resize_side(&mut self, sign: i8, count: Option<usize>) {
        let amount = count.map_or(RESIZE_STEP, |count| count.min(200) as i16) * i16::from(sign);
        if self.focus == Focus::Panel && self.panel_visible {
            self.panel_width_delta = (self.panel_width_delta + amount).clamp(-60, 120);
        } else if self.sidebar_visible {
            self.sidebar_width_delta = (self.sidebar_width_delta + amount).clamp(-30, 80);
        } else {
            self.toast("No side window to resize", ToastLevel::Info);
        }
    }
}
