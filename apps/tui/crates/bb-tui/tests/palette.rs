mod support;

use std::sync::Arc;

use bb_store::Resource;
use bb_tui::app::{Action, AppState, Effect, Focus, update};
use bb_tui::palette::{FileSource, PaletteKind, PaletteLoaded, PaletteRequest, PaneBody};
use bb_tui::sidebar::RowKey;
use bb_wire::environment::Environment;
use bb_wire::plugin::{PluginCliCommandInfo, PluginCliContribution, PluginCliResult};
use bb_wire::project::{WorkspacePathEntry, WorkspacePathListResponse};
use bb_wire::thread::{
    HighlightRange, ThreadResponse, ThreadSearchMatch, ThreadSearchResponse, ThreadSearchResult,
    ThreadSearchSourceKind,
};
use crossterm::event::{KeyCode, KeyModifiers};
use support::*;

fn ctrl(state: &mut AppState, ch: char) -> Vec<Effect> {
    press_mod(state, KeyCode::Char(ch), KeyModifiers::CONTROL)
}

fn type_text(state: &mut AppState, text: &str) -> Vec<Effect> {
    let mut effects = Vec::new();
    for ch in text.chars() {
        effects.extend(press(state, KeyCode::Char(ch)));
    }
    effects
}

fn palette_requests(effects: &[Effect]) -> Vec<(u64, PaletteRequest)> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Palette {
                generation,
                request,
            } => Some((*generation, request.clone())),
            _ => None,
        })
        .collect()
}

fn titles(state: &AppState) -> Vec<String> {
    state
        .palette
        .as_ref()
        .expect("palette")
        .items
        .iter()
        .map(|item| item.title.clone())
        .collect()
}

fn contributions() -> Vec<PluginCliContribution> {
    vec![PluginCliContribution {
        plugin_id: "bb-account".into(),
        name: "account".into(),
        summary: "Sign this bb in".into(),
        commands: vec![
            PluginCliCommandInfo {
                name: "status".into(),
                summary: "Show account status".into(),
                usage: "bb account status [--json]".into(),
            },
            PluginCliCommandInfo {
                name: "login".into(),
                summary: "Sign in".into(),
                usage: "bb account login [--code <code>]".into(),
            },
        ],
        renders_help: true,
    }]
}

fn with_environment(state: &mut AppState, thread_id: &str, environment_id: &str, root: &str) {
    let mut snapshot = (*state.snapshot).clone();
    let entry = snapshot.sidebar_thread(thread_id).cloned().expect("thread");
    let mut thread = entry.thread.clone();
    thread.environment_id = Some(environment_id.into());
    snapshot.threads.insert(
        thread_id.into(),
        Resource {
            data: Some(Arc::new(ThreadResponse {
                thread,
                environment: Some(Environment {
                    id: environment_id.into(),
                    path: Some(root.into()),
                    ..Environment::default()
                }),
                ..ThreadResponse::default()
            })),
            ..Resource::default()
        },
    );
    snapshot.version += 1;
    update(state, Action::Snapshot(Arc::new(snapshot)));
}

#[test]
fn command_palette_ranks_runs_and_remembers_commands() {
    let mut state = loaded_state(sample_sidebar());
    let effects = ctrl(&mut state, 'k');
    let palette = state.palette.as_ref().expect("palette open");
    assert_eq!(palette.kind, PaletteKind::Commands);
    assert!(matches!(
        palette_requests(&effects)[..],
        [(_, PaletteRequest::Contributions)]
    ));
    let screen = render_lines(&state, 110, 30).join("\n");
    assert!(screen.contains("Commands"), "{screen}");
    assert!(screen.contains("Search threads"), "{screen}");
    assert!(screen.contains("C-b"), "{screen}");

    type_text(&mut state, "tog side");
    assert_eq!(titles(&state)[0], "Toggle sidebar");
    let effects = press(&mut state, KeyCode::Enter);
    assert!(state.palette.is_none());
    assert!(!state.sidebar_visible);
    assert!(effects.contains(&Effect::SavePaletteRecents(vec![
        "app:sidebar.toggle".into()
    ])));

    ctrl(&mut state, 'k');
    let palette = state.palette.as_ref().unwrap();
    assert_eq!(palette.items[0].title, "Toggle sidebar");
    assert_eq!(palette.items[0].group, "Recent");
    press(&mut state, KeyCode::Esc);
    assert!(state.palette.is_none());
}

#[test]
fn palette_opens_from_composer_and_switches_modes() {
    let mut state = loaded_state(sample_sidebar());
    press(&mut state, KeyCode::Char('j'));
    press(&mut state, KeyCode::Enter);
    press(&mut state, KeyCode::Char('c'));
    assert_eq!(state.focus, Focus::Composer);
    ctrl(&mut state, 'k');
    assert_eq!(state.palette.as_ref().unwrap().kind, PaletteKind::Commands);
    type_text(&mut state, "search thr");
    press(&mut state, KeyCode::Enter);
    let palette = state.palette.as_ref().expect("threads palette");
    assert_eq!(palette.kind, PaletteKind::Threads);
    assert_eq!(palette.origin, PaletteKind::Threads);
    press(&mut state, KeyCode::Esc);
    assert_eq!(state.focus, Focus::Composer);
    press(&mut state, KeyCode::Char('x'));
    let composer = state.composers.get("thr_unread").expect("composer");
    assert_eq!(composer.text(), "x");
}

#[test]
fn thread_search_merges_local_and_server_results_and_opens_thread() {
    let mut state = loaded_state(sample_sidebar());
    press(&mut state, KeyCode::Char('f'));
    assert_eq!(
        titles(&state)[..2],
        ["Needs approval".to_owned(), "Refactor parser".to_owned()]
    );
    let effects = type_text(&mut state, "parser");
    let requests = palette_requests(&effects);
    let (generation, last) = requests.last().cloned().expect("search request");
    assert_eq!(
        last,
        PaletteRequest::SearchThreads {
            query: "parser".into()
        }
    );
    assert_eq!(titles(&state), ["Refactor parser"]);

    let mut archived = thread("thr_gone", "proj_a", "Old parser spike", 900);
    archived.thread.archived_at = Some(NOW_MS);
    let response = ThreadSearchResponse {
        active: bb_wire::thread::ThreadSearchResultGroup {
            total: 1,
            results: vec![ThreadSearchResult {
                thread: thread("thr_old", "proj_a", "Write docs", 60 * 30),
                matches: vec![ThreadSearchMatch {
                    source_kind: ThreadSearchSourceKind::AssistantMessage,
                    text: "The parser docs need a rewrite".into(),
                    highlight_ranges: vec![HighlightRange { start: 4, end: 10 }],
                    source_seq: Some(9),
                }],
            }],
        },
        archived: bb_wire::thread::ThreadSearchResultGroup {
            total: 1,
            results: vec![ThreadSearchResult {
                thread: archived,
                matches: Vec::new(),
            }],
        },
    };
    update(
        &mut state,
        Action::PaletteLoaded {
            generation,
            loaded: PaletteLoaded::Threads {
                query: "pars".into(),
                result: Ok(ThreadSearchResponse::default()),
            },
        },
    );
    assert_eq!(state.palette.as_ref().unwrap().threads.data, None);
    update(
        &mut state,
        Action::PaletteLoaded {
            generation,
            loaded: PaletteLoaded::Threads {
                query: "parser".into(),
                result: Ok(response),
            },
        },
    );
    assert_eq!(
        titles(&state),
        ["Refactor parser", "Write docs", "Old parser spike"]
    );
    let screen = render_lines(&state, 110, 30).join("\n");
    assert!(
        screen.contains("The parser docs need a rewrite"),
        "{screen}"
    );
    assert!(screen.contains("Archived"), "{screen}");

    press(&mut state, KeyCode::Down);
    let effects = press(&mut state, KeyCode::Enter);
    assert!(state.palette.is_none());
    assert_eq!(state.open_thread.as_deref(), Some("thr_old"));
    assert_eq!(state.focus, Focus::Main);
    assert!(effects.contains(&Effect::WatchThread(Some("thr_old".into()))));
}

#[test]
fn stale_generation_results_are_ignored() {
    let mut state = loaded_state(sample_sidebar());
    press(&mut state, KeyCode::Char('f'));
    let effects = type_text(&mut state, "docs");
    let (generation, _) = palette_requests(&effects).pop().unwrap();
    press(&mut state, KeyCode::Esc);
    press(&mut state, KeyCode::Char('f'));
    type_text(&mut state, "docs");
    update(
        &mut state,
        Action::PaletteLoaded {
            generation,
            loaded: PaletteLoaded::Threads {
                query: "docs".into(),
                result: Err("late".into()),
            },
        },
    );
    assert_eq!(state.palette.as_ref().unwrap().threads.error, None);
}

#[test]
fn project_switcher_reveals_project_in_sidebar() {
    let mut state = loaded_state(sample_sidebar());
    state.collapsed.insert("proj_b".into());
    press(&mut state, KeyCode::Char('P'));
    type_text(&mut state, "bet");
    assert_eq!(titles(&state), ["beta"]);
    press(&mut state, KeyCode::Enter);
    assert!(state.palette.is_none());
    assert_eq!(state.focus, Focus::Sidebar);
    assert!(!state.collapsed.contains("proj_b"));
    assert_eq!(
        state.selected_row().map(|row| row.key()),
        Some(RowKey::Group("proj_b".into()))
    );
}

#[test]
fn quick_open_searches_environment_files_and_previews_them() {
    let mut state = loaded_state(sample_sidebar());
    press(&mut state, KeyCode::Enter);
    with_environment(&mut state, "thr_wait", "env_1", "/work/repo");
    ctrl(&mut state, 'p');
    let palette = state.palette.as_ref().expect("files palette");
    assert_eq!(palette.kind, PaletteKind::Files);
    let source = FileSource::Environment {
        environment_id: "env_1".into(),
        root: Some("/work/repo".into()),
    };
    assert_eq!(palette.source, Some(source.clone()));
    let screen = render_lines(&state, 110, 30).join("\n");
    assert!(
        screen.contains("Type to search files in /work/repo"),
        "{screen}"
    );

    let effects = type_text(&mut state, "ma");
    let (generation, request) = palette_requests(&effects).pop().unwrap();
    assert_eq!(
        request,
        PaletteRequest::Paths {
            source: source.clone(),
            query: "ma".into()
        }
    );
    let entry = |path: &str, score: f64| WorkspacePathEntry {
        path: path.into(),
        name: path.rsplit('/').next().unwrap().into(),
        score,
        positions: vec![0],
        ..WorkspacePathEntry::default()
    };
    update(
        &mut state,
        Action::PaletteLoaded {
            generation,
            loaded: PaletteLoaded::Paths {
                query: "ma".into(),
                result: Ok(WorkspacePathListResponse {
                    paths: vec![
                        entry("docs/manual.md", 1.0),
                        entry("src/main.rs", 5.0),
                        entry("Makefile", 2.0),
                    ],
                    ..WorkspacePathListResponse::default()
                }),
            },
        },
    );
    assert_eq!(
        titles(&state),
        ["src/main.rs", "Makefile", "docs/manual.md"]
    );
    type_text(&mut state, "in");
    assert_eq!(titles(&state), ["src/main.rs"]);

    let effects = press(&mut state, KeyCode::Enter);
    assert!(state.palette.is_none());
    let (generation, request) = palette_requests(&effects).pop().unwrap();
    assert_eq!(
        request,
        PaletteRequest::File {
            source,
            path: "src/main.rs".into()
        }
    );
    let pane = state.result_pane.as_ref().expect("pane");
    assert_eq!(pane.body, PaneBody::Loading);
    update(
        &mut state,
        Action::PaletteLoaded {
            generation,
            loaded: PaletteLoaded::File {
                path: "src/main.rs".into(),
                result: Ok(b"fn main() {\n    println!(\"hi\");\n}\n".to_vec()),
            },
        },
    );
    let screen = render_lines(&state, 100, 20).join("\n");
    assert!(screen.contains("src/main.rs"), "{screen}");
    assert!(screen.contains("println!(\"hi\");"), "{screen}");
    assert!(screen.contains("1-3/3"), "{screen}");
    assert_eq!(
        press(&mut state, KeyCode::Char('o')),
        vec![Effect::OpenExternal("/work/repo/src/main.rs".into())]
    );
    assert_eq!(
        press(&mut state, KeyCode::Char('y')),
        vec![Effect::Copy("src/main.rs".into())]
    );
    press(&mut state, KeyCode::Esc);
    assert!(state.result_pane.is_none());
}

#[test]
fn quick_open_without_workspace_explains_why() {
    let mut personal = sample_sidebar();
    personal.projects.clear();
    let mut state = loaded_state(personal);
    state.selected = 0;
    ctrl(&mut state, 'p');
    let screen = render_lines(&state, 110, 20).join("\n");
    assert!(
        screen.contains("Open a thread or select a project to search its files"),
        "{screen}"
    );
    let effects = type_text(&mut state, "main");
    assert!(palette_requests(&effects).is_empty());
}

fn load_contributions(state: &mut AppState, effects: &[Effect]) {
    let (generation, _) = palette_requests(effects).pop().expect("contributions");
    update(
        state,
        Action::PaletteLoaded {
            generation,
            loaded: PaletteLoaded::Contributions(Ok(contributions())),
        },
    );
}

#[test]
fn plugin_cli_commands_run_with_arguments_and_show_output() {
    let mut state = loaded_state(sample_sidebar());
    press(&mut state, KeyCode::Enter);
    let effects = ctrl(&mut state, 'k');
    load_contributions(&mut state, &effects);
    type_text(&mut state, "account status");
    assert_eq!(titles(&state)[0], "bb account status");
    press(&mut state, KeyCode::Enter);
    let palette = state.palette.as_ref().expect("args palette");
    assert_eq!(palette.kind, PaletteKind::PluginArgs);
    assert_eq!(palette.query(), "status ");
    let screen = render_lines(&state, 110, 20).join("\n");
    assert!(screen.contains("bb account status [--json]"), "{screen}");
    type_text(&mut state, "--json \"a b\"");
    let effects = press(&mut state, KeyCode::Enter);
    assert!(state.palette.is_none());
    let (generation, request) = palette_requests(&effects).pop().expect("cli run");
    let PaletteRequest::RunCli { plugin_id, request } = request else {
        panic!("expected cli run");
    };
    assert_eq!(plugin_id, "bb-account");
    assert_eq!(request.argv, ["status", "--json", "a b"]);
    assert_eq!(request.thread_id.as_deref(), Some("thr_wait"));
    assert_eq!(request.project_id.as_deref(), Some("proj_a"));
    assert_eq!(request.cwd, None);
    update(
        &mut state,
        Action::PaletteLoaded {
            generation,
            loaded: PaletteLoaded::Cli {
                label: "account status --json a b".into(),
                result: Ok(PluginCliResult {
                    exit_code: 2,
                    stdout: "signed in as \u{1b}[1mada\u{1b}[0m\n".into(),
                    stderr: "warning: token expires soon\n".into(),
                    error: None,
                }),
            },
        },
    );
    let screen = render_lines(&state, 100, 20).join("\n");
    assert!(screen.contains("signed in as ada"), "{screen}");
    assert!(screen.contains("warning: token expires soon"), "{screen}");
    assert!(screen.contains("exit 2"), "{screen}");
    assert_eq!(
        press(&mut state, KeyCode::Char('y')),
        vec![Effect::Copy("signed in as \u{1b}[1mada\u{1b}[0m\n".into())]
    );
}

#[test]
fn plugin_arguments_backspace_returns_to_commands_and_bad_quotes_toast() {
    let mut state = loaded_state(sample_sidebar());
    let effects = ctrl(&mut state, 'k');
    load_contributions(&mut state, &effects);
    type_text(&mut state, "account login");
    press(&mut state, KeyCode::Enter);
    type_text(&mut state, "\"open");
    let effects = press(&mut state, KeyCode::Enter);
    assert!(palette_requests(&effects).is_empty());
    assert!(
        state
            .toasts
            .iter()
            .any(|toast| toast.message == "Unclosed quote")
    );
    for _ in 0..20 {
        press(&mut state, KeyCode::Backspace);
    }
    assert_eq!(state.palette.as_ref().unwrap().kind, PaletteKind::Commands);
}

#[test]
fn read_only_refuses_plugin_commands() {
    let mut state = loaded_state(sample_sidebar());
    state.read_only = true;
    let effects = ctrl(&mut state, 'k');
    load_contributions(&mut state, &effects);
    type_text(&mut state, "account status");
    press(&mut state, KeyCode::Enter);
    let effects = press(&mut state, KeyCode::Enter);
    assert!(palette_requests(&effects).is_empty());
    assert!(state.result_pane.is_none());
    assert!(
        state
            .toasts
            .iter()
            .any(|toast| toast.message == "Read-only mode: cannot run plugin commands")
    );
}

#[test]
fn keybinding_override_rebinds_palette_open() {
    let mut state = loaded_state(sample_sidebar());
    update(
        &mut state,
        Action::KeymapLoaded(Ok(vec![bb_wire::system::AppKeybindingOverride {
            command: "palette.open".into(),
            shortcut: Some(bb_wire::system::AppShortcut {
                key: "o".into(),
                primary_modifier: true,
                ..bb_wire::system::AppShortcut::default()
            }),
            platform: None,
        }])),
    );
    ctrl(&mut state, 'k');
    assert!(state.palette.is_none());
    ctrl(&mut state, 'o');
    assert_eq!(state.palette.as_ref().unwrap().kind, PaletteKind::Commands);
    let screen = render_lines(&state, 110, 30).join("\n");
    assert!(screen.contains("Toggle sidebar"), "{screen}");
}
