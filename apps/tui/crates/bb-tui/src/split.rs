use crate::app::{AppState, Effect, Focus, ToastLevel};

pub const MAX_SPLIT_PANES: usize = 4;

impl AppState {
    pub fn pane_threads(&self) -> Vec<String> {
        if self.splits.is_empty() {
            self.open_thread.iter().cloned().collect()
        } else {
            self.splits.clone()
        }
    }

    pub fn is_split(&self) -> bool {
        self.splits.len() > 1
    }

    pub fn focused_pane(&self) -> usize {
        self.open_thread
            .as_ref()
            .and_then(|open| self.splits.iter().position(|id| id == open))
            .unwrap_or(0)
    }

    pub(crate) fn split_watch(&self) -> Effect {
        Effect::WatchSplits(self.splits.clone())
    }

    pub(crate) fn place_in_split(&mut self, thread_id: &str) {
        if self.splits.is_empty() || self.splits.iter().any(|id| id == thread_id) {
            return;
        }
        let index = self.focused_pane();
        self.splits[index] = thread_id.to_owned();
    }

    pub fn open_in_split(&mut self) -> Vec<Effect> {
        let Some(open) = self.open_thread.clone() else {
            return match self.selected_thread_id().map(str::to_owned) {
                Some(thread_id) => {
                    let effects = self.open(thread_id);
                    self.focus = Focus::Main;
                    effects
                }
                None => {
                    self.toast("Select a thread to open", ToastLevel::Info);
                    Vec::new()
                }
            };
        };
        let candidate = self
            .selected_thread_id()
            .map(str::to_owned)
            .filter(|id| !self.pane_threads().contains(id));
        let Some(candidate) = candidate else {
            if let Some(selected) = self.selected_thread_id().map(str::to_owned)
                && selected != open
                && self.splits.contains(&selected)
            {
                let effects = self.open(selected);
                self.focus = Focus::Main;
                return effects;
            }
            self.toast(
                "Select another thread in the sidebar to open it beside this one",
                ToastLevel::Info,
            );
            return Vec::new();
        };
        if self.splits.len() >= MAX_SPLIT_PANES {
            self.toast(
                format!("At most {MAX_SPLIT_PANES} panes · close one with X"),
                ToastLevel::Info,
            );
            return Vec::new();
        }
        if self.splits.is_empty() {
            self.splits.push(open);
        }
        let index = self.focused_pane();
        self.splits.insert(index + 1, candidate.clone());
        self.split_maximized = false;
        let mut effects = self.open(candidate);
        self.focus = Focus::Main;
        if !effects
            .iter()
            .any(|effect| matches!(effect, Effect::WatchSplits(_)))
        {
            effects.push(self.split_watch());
        }
        effects
    }

    pub fn focus_pane(&mut self, index: usize) -> Vec<Effect> {
        let Some(thread_id) = self.splits.get(index).cloned() else {
            return Vec::new();
        };
        let focus = match self.focus {
            Focus::Composer | Focus::Interaction | Focus::Main => self.focus,
            _ => Focus::Main,
        };
        let effects = self.open(thread_id);
        self.focus = focus;
        if self.focus == Focus::Composer && !self.composer_available() {
            self.focus = Focus::Main;
        }
        if self.focus == Focus::Interaction && !self.open_interaction_panel() {
            self.focus = Focus::Main;
        }
        effects
    }

    pub fn step_pane(&mut self, delta: isize) -> Vec<Effect> {
        if !self.is_split() {
            self.toast("Only one pane is open · V opens a split", ToastLevel::Info);
            return Vec::new();
        }
        let count = self.splits.len() as isize;
        let next = (self.focused_pane() as isize + delta).rem_euclid(count) as usize;
        self.focus_pane(next)
    }

    pub fn close_split_pane(&mut self) -> Vec<Effect> {
        if !self.is_split() {
            self.toast("Only one pane is open", ToastLevel::Info);
            return Vec::new();
        }
        let index = self.focused_pane();
        self.splits.remove(index);
        let next = self.splits[index.min(self.splits.len() - 1)].clone();
        if self.splits.len() == 1 {
            self.splits.clear();
            self.split_maximized = false;
        }
        let mut effects = self.open(next);
        if matches!(self.focus, Focus::Composer | Focus::Interaction) {
            self.focus = Focus::Main;
        }
        effects.push(self.split_watch());
        effects
    }

    pub fn toggle_pane_maximized(&mut self) {
        if !self.is_split() {
            self.toast("Only one pane is open", ToastLevel::Info);
            return;
        }
        self.split_maximized = !self.split_maximized;
    }

    pub(crate) fn sync_split_timelines(&mut self) -> Vec<Effect> {
        let snapshot = self.snapshot.clone();
        let open = self.open_thread.clone();
        let mut effects = Vec::new();
        for thread_id in self.splits.clone() {
            if Some(&thread_id) == open.as_ref() {
                continue;
            }
            let loads = match self.timelines.get_mut(&thread_id) {
                Some(timeline) => timeline.sync(snapshot.clone()),
                None => continue,
            };
            effects.extend(self.load_effects_for(&thread_id, loads));
        }
        effects
    }
}
