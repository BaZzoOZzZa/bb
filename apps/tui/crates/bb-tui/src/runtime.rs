use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};
use bb_store::{Interest, ResourceKey, Store};
use crossterm::event::{Event, KeyEventKind};
use futures::{Stream, StreamExt};
use ratatui::Terminal;
use ratatui::backend::Backend as TerminalBackend;
use tokio::sync::mpsc;

use crate::api::Api;
use crate::app::{Action, AppState, Effect, update};
use crate::time::now_ms;
use crate::ui;

pub const TICK: Duration = Duration::from_millis(200);

pub struct EffectRunner<A: Api> {
    store: Store,
    api: Arc<A>,
    tx: mpsc::UnboundedSender<Action>,
    _sidebar: Interest,
    thread: Option<(String, Interest)>,
}

impl<A: Api> EffectRunner<A> {
    pub fn new(store: Store, api: Arc<A>, tx: mpsc::UnboundedSender<Action>) -> Self {
        let sidebar = store.watch_sidebar();
        Self {
            store,
            api,
            tx,
            _sidebar: sidebar,
            thread: None,
        }
    }

    pub fn load_keymap(&self) {
        let api = self.api.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = api.keybinding_overrides().await;
            let _ = tx.send(Action::KeymapLoaded(result));
        });
    }

    pub fn run(&mut self, effect: Effect) {
        match effect {
            Effect::Mutate(mutation) => {
                let api = self.api.clone();
                let tx = self.tx.clone();
                tokio::spawn(async move {
                    let result = api.mutate(mutation.clone()).await;
                    let _ = tx.send(Action::MutationFinished { mutation, result });
                });
            }
            Effect::WatchThread(None) => self.thread = None,
            Effect::WatchThread(Some(thread_id)) => {
                if self.thread.as_ref().map(|(id, _)| id) != Some(&thread_id) {
                    let interest = self.store.watch_thread(&thread_id);
                    self.thread = Some((thread_id, interest));
                }
            }
            Effect::RefreshSidebar => self.store.refresh(ResourceKey::Sidebar),
            Effect::RefreshAll => self.store.refresh_all(),
            Effect::Quit => {}
        }
    }

    pub fn watched_thread(&self) -> Option<&str> {
        self.thread.as_ref().map(|(id, _)| id.as_str())
    }
}

pub fn terminal_action(event: std::io::Result<Event>) -> Option<Action> {
    match event {
        Ok(Event::Key(key)) if key.kind != KeyEventKind::Release => Some(Action::Key(key)),
        Ok(Event::Resize(_, _)) => Some(Action::Resize),
        _ => None,
    }
}

pub fn terminal_events() -> impl Stream<Item = Action> + Unpin {
    crossterm::event::EventStream::new()
        .filter_map(|event| std::future::ready(terminal_action(event)))
}

pub async fn run<B, A, S>(
    terminal: &mut Terminal<B>,
    state: &mut AppState,
    store: Store,
    api: Arc<A>,
    mut events: S,
) -> Result<()>
where
    B: TerminalBackend,
    B::Error: Send + Sync + 'static,
    A: Api,
    S: Stream<Item = Action> + Unpin,
{
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mut snapshots = store.subscribe();
    let mut runner = EffectRunner::new(store, api, tx);
    runner.load_keymap();
    let initial = snapshots.borrow_and_update().clone();
    update(state, Action::Snapshot(initial));
    let mut tick = tokio::time::interval(TICK);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut events_open = true;

    while !state.quit {
        terminal
            .draw(|frame| ui::render(state, frame))
            .map_err(|error| anyhow!(error))?;
        let action = tokio::select! {
            biased;
            Some(action) = rx.recv() => action,
            changed = snapshots.changed() => match changed {
                Ok(()) => Action::Snapshot(snapshots.borrow_and_update().clone()),
                Err(_) => return Err(anyhow!("store stopped")),
            },
            event = events.next(), if events_open => match event {
                Some(action) => action,
                None => {
                    events_open = false;
                    continue;
                }
            },
            _ = tick.tick() => Action::Tick { now: Instant::now(), now_ms: now_ms() },
        };
        dispatch(state, &mut runner, action);
        while let Ok(action) = rx.try_recv() {
            dispatch(state, &mut runner, action);
        }
    }
    Ok(())
}

fn dispatch<A: Api>(state: &mut AppState, runner: &mut EffectRunner<A>, action: Action) {
    for effect in update(state, action) {
        runner.run(effect);
    }
}
