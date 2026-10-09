mod support;

use std::sync::Arc;

use bb_store::Resource;
use bb_tui::api::{Mutation, MutationReply};
use bb_tui::app::{Action, AppState, Effect, Focus, update};
use bb_tui::git_diff::{DiffRow, DiffScope, PatchState};
use bb_tui::panel::{PanelLoaded, PanelRequest, PanelTab};
use bb_tui::runtime::run_panel_request;
use bb_wire::environment::{
    ContentEncoding, DiffFileEntry, DiffFileLoadMode, DiffFileOrigin, DiffFileSide, DiffPatchEntry,
    Environment, EnvironmentActionRequest, EnvironmentActionResponse, EnvironmentDiffFileQuery,
    EnvironmentDiffFileResponse, EnvironmentDiffFilesResponse, EnvironmentDiffPatchResponse,
    EnvironmentDiffQuery, EnvironmentHostLifecycle, EnvironmentPullRequestResponse,
    EnvironmentStatusResponse, GitCheckoutRef, GitDiffFileChangeKind, PullRequestMergeMethod,
    PullRequestMergeOptions, PullRequestState, ThreadPullRequest, WorkspaceBranch,
    WorkspaceCommitSummary, WorkspaceDiffTarget, WorkspaceFileStatus, WorkspaceFileStatusKind,
    WorkspaceMergeBase, WorkspaceState, WorkspaceStatus, WorkspaceWorkingTree,
};
use bb_wire::thread::ThreadResponse;
use crossterm::event::{KeyCode, KeyModifiers};
use support::*;

const ENV: &str = "env_1";
const ROOT: &str = "/work/alpha";
const OPEN: &str = "thr_unread";

fn environment(merge_base: Option<&str>) -> Environment {
    Environment {
        id: ENV.into(),
        project_id: "proj_a".into(),
        host_id: "host_1".into(),
        path: Some(ROOT.into()),
        is_git_repo: true,
        merge_base_branch: merge_base.map(str::to_owned),
        host_lifecycle: EnvironmentHostLifecycle::Active,
        ..Environment::default()
    }
}

fn open_with_environment(merge_base: Option<&str>) -> AppState {
    let mut state = loaded_state(sample_sidebar());
    press(&mut state, KeyCode::Char('j'));
    press(&mut state, KeyCode::Enter);
    assert_eq!(state.open_thread.as_deref(), Some(OPEN));
    set_environment(&mut state, environment(merge_base), 0);
    state
}

fn set_environment(state: &mut AppState, environment: Environment, generation: u64) {
    let mut snapshot = (*state.snapshot).clone();
    let entry = snapshot.sidebar_thread(OPEN).cloned().expect("thread");
    let mut thread = entry.thread.clone();
    thread.environment_id = Some(environment.id.clone());
    snapshot.threads.insert(
        OPEN.into(),
        Resource {
            data: Some(Arc::new(ThreadResponse {
                thread,
                environment: Some(environment),
                ..ThreadResponse::default()
            })),
            ..Resource::default()
        },
    );
    let mut generations = (*snapshot.generations).clone();
    generations.environments.insert(ENV.into(), generation);
    snapshot.generations = Arc::new(generations);
    snapshot.version += 1;
    update(state, Action::Snapshot(Arc::new(snapshot)));
}

fn panel_requests(effects: &[Effect]) -> Vec<PanelRequest> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Panel(request) => Some(request.clone()),
            _ => None,
        })
        .collect()
}

fn commit(sha: &str, subject: &str) -> WorkspaceCommitSummary {
    WorkspaceCommitSummary {
        sha: sha.into(),
        short_sha: sha[..7].into(),
        subject: subject.into(),
        author_name: "Ada".into(),
        authored_at: NOW_MS - 3 * MINUTE,
    }
}

fn workspace(uncommitted: bool) -> WorkspaceStatus {
    let mut files = Vec::new();
    if uncommitted {
        files.push(WorkspaceFileStatus {
            path: "src/lib.rs".into(),
            status: WorkspaceFileStatusKind::Modified,
            insertions: Some(4),
            deletions: Some(1),
        });
    }
    WorkspaceStatus {
        working_tree: WorkspaceWorkingTree {
            insertions: 4,
            deletions: 1,
            line_stats_complete: true,
            has_uncommitted_changes: uncommitted,
            state: WorkspaceState::DirtyAndCommittedUnmerged,
            files,
        },
        checkout: GitCheckoutRef::Branch {
            branch_name: "feature/panel".into(),
            head_sha: Some("1234567890".into()),
        },
        branch: WorkspaceBranch {
            current_branch: Some("feature/panel".into()),
            default_branch: "main".into(),
        },
        merge_base: Some(WorkspaceMergeBase {
            insertions: 12,
            deletions: 3,
            line_stats_complete: true,
            merge_base_branch: "main".into(),
            base_ref: Some("abcdef0".into()),
            ahead_count: 2,
            behind_count: 0,
            has_committed_unmerged_changes: true,
            files: vec![WorkspaceFileStatus {
                path: "docs/guide.md".into(),
                status: WorkspaceFileStatusKind::Added,
                insertions: Some(12),
                deletions: Some(0),
            }],
            commits: vec![
                commit("aaaaaaa111", "Add the panel"),
                commit("bbbbbbb222", "Wire the diff"),
            ],
        }),
    }
}

fn status_loaded(state: &mut AppState, base: Option<&str>, status: WorkspaceStatus) -> Vec<Effect> {
    update(
        state,
        Action::PanelLoaded(PanelLoaded::Status {
            environment_id: ENV.into(),
            merge_base_branch: base.map(str::to_owned),
            result: Ok(EnvironmentStatusResponse::Available {
                workspace: Box::new(status),
            }),
        }),
    )
}

fn pull_request(state: PullRequestState) -> ThreadPullRequest {
    ThreadPullRequest {
        number: 7,
        title: "Panel".into(),
        state,
        url: "https://github.com/acme/alpha/pull/7".into(),
        base_ref_name: "main".into(),
        head_ref_name: "feature/panel".into(),
        ..ThreadPullRequest::default()
    }
}

fn pr_loaded(state: &mut AppState, pr: ThreadPullRequest) {
    update(
        state,
        Action::PanelLoaded(PanelLoaded::PullRequest {
            environment_id: ENV.into(),
            result: Ok(EnvironmentPullRequestResponse::Available { pull_request: pr }),
        }),
    );
}

fn file_entry(path: &str, additions: u64, mode: DiffFileLoadMode) -> DiffFileEntry {
    DiffFileEntry {
        path: path.into(),
        previous_path: None,
        change_kind: GitDiffFileChangeKind::Modified,
        additions,
        deletions: 1,
        binary: false,
        origin: DiffFileOrigin::Tracked,
        load_mode: mode,
    }
}

fn patch(path: &str) -> DiffPatchEntry {
    DiffPatchEntry {
        path: path.into(),
        patch: format!(
            "diff --git a/{path} b/{path}\n--- a/{path}\n+++ b/{path}\n@@ -10,3 +10,3 @@ fn main\n let a = 1;\n-let b = 2;\n+let b = 3;\n let c = 4;\n@@ -40,1 +40,2 @@\n x\n+y\n"
        ),
        truncated: false,
    }
}

fn diff_loaded(
    state: &mut AppState,
    target: WorkspaceDiffTarget,
    files: Vec<DiffFileEntry>,
    initial: Vec<DiffPatchEntry>,
) -> Vec<Effect> {
    update(
        state,
        Action::PanelLoaded(PanelLoaded::DiffFiles {
            environment_id: ENV.into(),
            target,
            result: Ok(EnvironmentDiffFilesResponse::Available {
                files,
                truncated: false,
                shortstat: String::new(),
                merge_base_ref: Some("abcdef0".into()),
                initial_patches: initial,
            }),
        }),
    )
}

fn all_main() -> WorkspaceDiffTarget {
    WorkspaceDiffTarget::All {
        merge_base_branch: "main".into(),
    }
}

fn open_diff(state: &mut AppState) {
    let effects = keys(state, "SPC g g");
    assert_eq!(state.focus, Focus::Panel);
    assert_eq!(state.panel.tab, PanelTab::Diff);
    assert!(panel_requests(&effects).contains(&PanelRequest::DiffFiles {
        environment_id: ENV.into(),
        target: all_main(),
    }));
    status_loaded(state, Some("main"), workspace(true));
}

#[test]
fn info_tab_shows_thread_git_and_pull_request_metadata() {
    let mut state = open_with_environment(Some("main"));
    let effects = keys(&mut state, "SPC o i");
    assert!(state.panel_visible);
    assert_eq!(
        panel_requests(&effects),
        [
            PanelRequest::Status {
                environment_id: ENV.into(),
                merge_base_branch: Some("main".into()),
            },
            PanelRequest::PullRequest {
                environment_id: ENV.into(),
            },
        ]
    );
    status_loaded(&mut state, Some("main"), workspace(true));
    pr_loaded(&mut state, pull_request(PullRequestState::Draft));
    let screen = render_lines(&state, 160, 44).join("\n");
    for expected in [
        "1 Info",
        "Project",
        "alpha",
        "Directory",
        "/work/alpha",
        "Branch",
        "feature/panel",
        "Merge base",
        "2 ahead of main",
        "#7 Draft",
        "Provider",
        "codex",
        "Commits (2)",
        "aaaaaaa Add the panel",
        "Files changed (1)",
        "A docs/guide.md +12",
        "Uncommitted files (1)",
        "M src/lib.rs +4 -1",
        "SPC g c c commit",
        "SPC g p r ready PR",
    ] {
        assert!(screen.contains(expected), "missing {expected:?}\n{screen}");
    }
}

#[test]
fn status_without_a_merge_base_is_refetched_against_the_default_branch() {
    let mut state = open_with_environment(None);
    let effects = keys(&mut state, "SPC o i");
    assert!(panel_requests(&effects).contains(&PanelRequest::Status {
        environment_id: ENV.into(),
        merge_base_branch: None,
    }));
    let mut status = workspace(false);
    status.merge_base = None;
    let effects = status_loaded(&mut state, None, status);
    assert_eq!(
        panel_requests(&effects),
        [PanelRequest::Status {
            environment_id: ENV.into(),
            merge_base_branch: Some("main".into()),
        }]
    );
}

#[test]
fn large_diff_loads_the_file_list_first_and_patches_only_near_the_viewport() {
    let mut state = open_with_environment(Some("main"));
    open_diff(&mut state);
    let files: Vec<DiffFileEntry> = (0..300)
        .map(|index| file_entry(&format!("src/f{index:03}.rs"), 2, DiffFileLoadMode::Auto))
        .collect();
    let effects = diff_loaded(&mut state, all_main(), files, Vec::new());
    assert!(
        panel_requests(&effects).is_empty(),
        "collapsed files load nothing"
    );
    let document = state.panel.document().expect("document");
    assert_eq!(document.row_count(), 300);

    let effects = keys(&mut state, "z R");
    let requests = panel_requests(&effects);
    let [PanelRequest::DiffPatches { paths, target, .. }] = &requests[..] else {
        panic!("{requests:?}");
    };
    assert_eq!(target, &all_main());
    assert!(paths.len() <= 50 && paths.len() > 10, "{}", paths.len());
    assert_eq!(paths[0], "src/f000.rs");
    assert!(!paths.contains(&"src/f299.rs".to_owned()));

    let effects = update(
        &mut state,
        Action::PanelLoaded(PanelLoaded::DiffPatches {
            environment_id: ENV.into(),
            target: all_main(),
            paths: paths.clone(),
            result: Ok(EnvironmentDiffPatchResponse::Available {
                patches: paths.iter().map(|path| patch(path)).collect(),
            }),
        }),
    );
    assert!(panel_requests(&effects).is_empty());
    let screen = render_lines(&state, 160, 30).join("\n");
    assert!(screen.contains("▾ M src/f000.rs +2 -1"), "{screen}");
    assert!(screen.contains("@@ -10,3 +10,3 @@ fn main"), "{screen}");
    assert!(screen.contains("11    - let b = 2;"), "{screen}");
    assert!(screen.contains("   11 + let b = 3;"), "{screen}");

    keys(&mut state, "2 ] c");
    let document = state.panel.document().unwrap();
    assert_eq!(document.row(document.cursor), Some(DiffRow::Line(0, 5)));
    keys(&mut state, "] f");
    let document = state.panel.document().unwrap();
    assert_eq!(document.row(document.cursor), Some(DiffRow::Header(1)));

    let effects = press(&mut state, KeyCode::Char('G'));
    let requests = panel_requests(&effects);
    let [PanelRequest::DiffPatches { paths, .. }] = &requests[..] else {
        panic!("{requests:?}");
    };
    assert!(paths.contains(&"src/f299.rs".to_owned()));
    assert!(!paths.contains(&"src/f100.rs".to_owned()));
}

#[test]
fn on_demand_files_load_on_enter_and_too_large_files_never_load() {
    let mut state = open_with_environment(Some("main"));
    open_diff(&mut state);
    let effects = diff_loaded(
        &mut state,
        all_main(),
        vec![
            file_entry("big.rs", 900, DiffFileLoadMode::OnDemand),
            file_entry("huge.json", 40_000, DiffFileLoadMode::TooLarge),
        ],
        Vec::new(),
    );
    assert!(panel_requests(&effects).is_empty());
    let screen = render_lines(&state, 160, 30).join("\n");
    assert!(
        screen.contains("Large diff (901 changed lines) · Enter to load"),
        "{screen}"
    );
    assert!(screen.contains("Diff too large to show"), "{screen}");
    press(&mut state, KeyCode::Char('j'));
    let effects = press(&mut state, KeyCode::Enter);
    assert_eq!(
        panel_requests(&effects),
        [PanelRequest::DiffPatches {
            environment_id: ENV.into(),
            target: all_main(),
            paths: vec!["big.rs".into()],
        }]
    );
    keys(&mut state, "] f");
    press(&mut state, KeyCode::Char('j'));
    let effects = press(&mut state, KeyCode::Enter);
    assert!(panel_requests(&effects).is_empty());
    let document = state.panel.document().unwrap();
    assert_eq!(document.files[1].patch, PatchState::Idle);
}

#[test]
fn scopes_cycle_and_commits_from_info_open_their_diff() {
    let mut state = open_with_environment(Some("main"));
    open_diff(&mut state);
    let effects = press(&mut state, KeyCode::Char('s'));
    assert_eq!(state.panel.scope, DiffScope::Committed);
    assert!(panel_requests(&effects).contains(&PanelRequest::DiffFiles {
        environment_id: ENV.into(),
        target: WorkspaceDiffTarget::BranchCommitted {
            merge_base_branch: "main".into()
        },
    }));
    let effects = press(&mut state, KeyCode::Char('s'));
    assert!(panel_requests(&effects).contains(&PanelRequest::DiffFiles {
        environment_id: ENV.into(),
        target: WorkspaceDiffTarget::Uncommitted,
    }));

    keys(&mut state, "1 g t");
    assert_eq!(state.panel.tab, PanelTab::Info);
    let entries = state.info_entries();
    let commit_row = entries
        .iter()
        .position(|entry| {
            entry
                .spans
                .iter()
                .any(|span| span.content.contains("Wire the diff"))
        })
        .expect("commit row");
    for _ in 0..commit_row {
        press(&mut state, KeyCode::Char('j'));
    }
    let effects = press(&mut state, KeyCode::Enter);
    assert_eq!(state.panel.tab, PanelTab::Diff);
    assert!(panel_requests(&effects).contains(&PanelRequest::DiffFiles {
        environment_id: ENV.into(),
        target: WorkspaceDiffTarget::Commit {
            sha: "bbbbbbb222".into()
        },
    }));
    let screen = render_lines(&state, 160, 30).join("\n");
    assert!(screen.contains("bbbbbbb Wire the diff"), "{screen}");
}

#[test]
fn changed_file_in_info_reveals_it_in_the_uncommitted_diff() {
    let mut state = open_with_environment(Some("main"));
    keys(&mut state, "SPC o i");
    state.focus = Focus::Panel;
    status_loaded(&mut state, Some("main"), workspace(true));
    let entries = state.info_entries();
    let row = entries
        .iter()
        .position(|entry| {
            entry
                .spans
                .iter()
                .any(|span| span.content.contains("src/lib.rs"))
        })
        .expect("file row");
    for _ in 0..row {
        press(&mut state, KeyCode::Char('j'));
    }
    let effects = press(&mut state, KeyCode::Enter);
    assert_eq!(state.panel.scope, DiffScope::Uncommitted);
    assert!(panel_requests(&effects).contains(&PanelRequest::DiffFiles {
        environment_id: ENV.into(),
        target: WorkspaceDiffTarget::Uncommitted,
    }));
    let files: Vec<DiffFileEntry> = (0..20)
        .map(|index| file_entry(&format!("a{index:02}.rs"), 1, DiffFileLoadMode::Auto))
        .chain(std::iter::once(file_entry(
            "src/lib.rs",
            4,
            DiffFileLoadMode::OnDemand,
        )))
        .collect();
    let effects = diff_loaded(
        &mut state,
        WorkspaceDiffTarget::Uncommitted,
        files,
        Vec::new(),
    );
    assert!(
        panel_requests(&effects).contains(&PanelRequest::DiffPatches {
            environment_id: ENV.into(),
            target: WorkspaceDiffTarget::Uncommitted,
            paths: vec!["src/lib.rs".into()],
        }),
        "{effects:?}"
    );
    let document = state.panel.document().unwrap();
    assert_eq!(document.row(document.cursor), Some(DiffRow::Header(20)));
    assert!(!document.files[20].collapsed);
    assert!(document.files[0].collapsed);
}

#[test]
fn preview_and_editor_follow_the_selected_diff_line() {
    let mut state = open_with_environment(Some("main"));
    open_diff(&mut state);
    diff_loaded(
        &mut state,
        all_main(),
        vec![file_entry("src/main.rs", 2, DiffFileLoadMode::Auto)],
        vec![patch("src/main.rs")],
    );
    keys(&mut state, "] c 3 j");
    let effects = press(&mut state, KeyCode::Char('e'));
    assert_eq!(
        effects,
        [Effect::EditFile {
            path: "/work/alpha/src/main.rs".into(),
            line: Some(11),
        }]
    );
    let effects = keys(&mut state, "g f");
    assert_eq!(state.panel.tab, PanelTab::File);
    assert_eq!(
        panel_requests(&effects),
        [PanelRequest::File {
            environment_id: ENV.into(),
            path: "src/main.rs".into(),
            query: EnvironmentDiffFileQuery::All {
                merge_base_ref: "abcdef0".into(),
                path: "src/main.rs".into(),
                side: DiffFileSide::New,
            },
        }]
    );
    let content: String = (1..=60)
        .map(|line| format!("let v{line} = {line};\n"))
        .collect();
    update(
        &mut state,
        Action::PanelLoaded(PanelLoaded::File {
            environment_id: ENV.into(),
            path: "src/main.rs".into(),
            result: Ok(EnvironmentDiffFileResponse {
                path: "/work/alpha/src/main.rs".into(),
                content,
                content_encoding: ContentEncoding::Utf8,
                mime_type: None,
                size_bytes: 900,
            }),
        }),
    );
    let screen = render_lines(&state, 160, 30).join("\n");
    assert!(screen.contains("src/main.rs · line 11"), "{screen}");
    assert!(screen.contains("11 let v11 = 11;"), "{screen}");
    press(&mut state, KeyCode::Esc);
    assert_eq!(state.panel.tab, PanelTab::Diff);
}

#[test]
fn commit_and_merge_require_confirmation_and_refresh_status() {
    let mut state = open_with_environment(Some("main"));
    open_diff(&mut state);
    pr_loaded(&mut state, pull_request(PullRequestState::Open));
    let effects = keys(&mut state, "SPC g c c");
    assert!(effects.is_empty());
    let screen = render_lines(&state, 160, 30).join("\n");
    assert!(screen.contains("Commit changes"), "{screen}");
    assert!(
        screen.contains("Commit 1 changed file on feature/panel."),
        "{screen}"
    );
    assert!(press(&mut state, KeyCode::Esc).is_empty());
    assert!(state.panel.confirm.is_none());

    keys(&mut state, ": c o m m i t RET");
    let effects = press(&mut state, KeyCode::Char('y'));
    let mutation = Mutation::EnvironmentAction {
        thread_id: OPEN.into(),
        environment_id: ENV.into(),
        request: EnvironmentActionRequest::Commit,
    };
    assert_eq!(effects, [Effect::Mutate(mutation.clone())]);
    let effects = update(
        &mut state,
        Action::MutationFinished {
            mutation,
            result: Ok(MutationReply::EnvironmentAction(Box::new(
                EnvironmentActionResponse::Commit {
                    message: "ok".into(),
                    commit_sha: "feedface1234".into(),
                    commit_subject: "Add panel".into(),
                },
            ))),
        },
    );
    assert!(
        state
            .toasts
            .iter()
            .any(|toast| toast.message == "Committed feedface1: Add panel")
    );
    let requests = panel_requests(&effects);
    assert!(requests.contains(&PanelRequest::Status {
        environment_id: ENV.into(),
        merge_base_branch: Some("main".into()),
    }));
    assert!(requests.contains(&PanelRequest::PullRequest {
        environment_id: ENV.into(),
    }));

    pr_loaded(&mut state, pull_request(PullRequestState::Open));
    keys(&mut state, "SPC g p m");
    let screen = render_lines(&state, 160, 30).join("\n");
    assert!(screen.contains("#7 Panel"), "{screen}");
    assert!(screen.contains("1 squash"), "{screen}");
    press(&mut state, KeyCode::Right);
    let effects = press(&mut state, KeyCode::Enter);
    assert_eq!(
        effects,
        [Effect::Mutate(Mutation::EnvironmentAction {
            thread_id: OPEN.into(),
            environment_id: ENV.into(),
            request: EnvironmentActionRequest::PullRequestMerge {
                options: PullRequestMergeOptions {
                    method: PullRequestMergeMethod::Merge
                }
            },
        })]
    );

    let effects = keys(&mut state, "SPC g p r");
    assert!(effects.is_empty());
    assert!(state.panel.confirm.is_none());
    assert!(
        state
            .toasts
            .iter()
            .any(|toast| toast.message == "No draft pull request")
    );
}

#[test]
fn read_only_mode_refuses_git_actions() {
    let mut state = open_with_environment(Some("main"));
    state.read_only = true;
    open_diff(&mut state);
    keys(&mut state, "SPC g c c");
    let effects = press(&mut state, KeyCode::Enter);
    assert!(effects.is_empty());
    assert!(
        state
            .toasts
            .iter()
            .any(|toast| toast.message.starts_with("Read-only mode: cannot commit"))
    );
    let effects = update(
        &mut state,
        Action::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('k'),
            KeyModifiers::CONTROL,
        )),
    );
    assert!(
        effects
            .iter()
            .all(|effect| !matches!(effect, Effect::Mutate(_)))
    );
}

#[test]
fn environment_changes_refetch_and_stale_responses_are_ignored() {
    let mut state = open_with_environment(Some("main"));
    open_diff(&mut state);
    diff_loaded(
        &mut state,
        all_main(),
        vec![file_entry("src/main.rs", 2, DiffFileLoadMode::Auto)],
        vec![patch("src/main.rs")],
    );
    let effects = update(
        &mut state,
        Action::PanelLoaded(PanelLoaded::DiffFiles {
            environment_id: "env_other".into(),
            target: all_main(),
            result: Ok(EnvironmentDiffFilesResponse::Available {
                files: Vec::new(),
                truncated: false,
                shortstat: String::new(),
                merge_base_ref: None,
                initial_patches: Vec::new(),
            }),
        }),
    );
    assert!(effects.is_empty());
    assert_eq!(state.panel.document().unwrap().files.len(), 1);

    let mut snapshot = (*state.snapshot).clone();
    let mut generations = (*snapshot.generations).clone();
    generations.environments.insert(ENV.into(), 5);
    snapshot.generations = Arc::new(generations);
    snapshot.version += 1;
    let effects = update(&mut state, Action::Snapshot(Arc::new(snapshot)));
    let requests = panel_requests(&effects);
    assert!(
        requests
            .iter()
            .any(|request| matches!(request, PanelRequest::Status { .. }))
    );
    assert!(requests.contains(&PanelRequest::DiffFiles {
        environment_id: ENV.into(),
        target: all_main(),
    }));
    assert_eq!(state.panel.document().unwrap().files.len(), 1);
}

#[test]
fn panel_tabs_maximize_and_command_ids_rebind() {
    let mut state = open_with_environment(Some("main"));
    keys(&mut state, "g t");
    assert!(state.panel_visible);
    assert_eq!(state.panel.tab, PanelTab::Diff);
    keys(&mut state, "g T");
    assert_eq!(state.panel.tab, PanelTab::Info);
    keys(&mut state, "SPC w m m");
    assert!(!state.panel.maximized);
    keys(&mut state, "SPC w m m");
    keys(&mut state, "C-l C-l");
    assert_eq!(state.focus, Focus::Panel);
    keys(&mut state, "C-w o");
    assert!(state.panel.maximized);
    assert_eq!(state.focus, Focus::Panel);
    let screen = render_lines(&state, 120, 20).join("\n");
    assert!(screen.contains("· max"), "{screen}");
    press(&mut state, KeyCode::Esc);
    assert!(!state.panel.maximized);
    press(&mut state, KeyCode::Char('q'));
    assert_eq!(state.focus, Focus::Main);
    assert_eq!(
        bb_tui::keymap::Command::from_app_command_id("diff.toggle"),
        Some(bb_tui::keymap::Command::DiffToggle)
    );
}

#[tokio::test]
async fn panel_requests_reach_the_api_with_their_targets() {
    let backend = FakeBackend::new(sample_sidebar());
    let api = MockApi::with_diff_files(
        backend,
        EnvironmentDiffFilesResponse::Available {
            files: vec![file_entry("a.rs", 1, DiffFileLoadMode::Auto)],
            truncated: false,
            shortstat: "1 file changed".into(),
            merge_base_ref: Some("abc".into()),
            initial_patches: Vec::new(),
        },
    );
    let loaded = run_panel_request(
        api.as_ref(),
        PanelRequest::DiffFiles {
            environment_id: ENV.into(),
            target: WorkspaceDiffTarget::BranchCommitted {
                merge_base_branch: "main".into(),
            },
        },
    )
    .await;
    assert!(matches!(
        loaded,
        PanelLoaded::DiffFiles {
            result: Ok(EnvironmentDiffFilesResponse::Available { .. }),
            ..
        }
    ));
    assert_eq!(
        api.diff_queries.lock().unwrap()[..],
        [(
            ENV.to_owned(),
            EnvironmentDiffQuery::BranchCommitted {
                merge_base_branch: "main".into()
            }
        )]
    );
    let loaded = run_panel_request(
        api.as_ref(),
        PanelRequest::DiffPatches {
            environment_id: ENV.into(),
            target: WorkspaceDiffTarget::Commit { sha: "abcd".into() },
            paths: vec!["a.rs".into()],
        },
    )
    .await;
    let PanelLoaded::DiffPatches {
        result: Ok(EnvironmentDiffPatchResponse::Available { patches }),
        ..
    } = loaded
    else {
        panic!("{loaded:?}");
    };
    assert_eq!(patches[0].path, "a.rs");
    assert_eq!(
        api.diff_patch_requests.lock().unwrap()[0].1.target,
        WorkspaceDiffTarget::Commit { sha: "abcd".into() }
    );
    let loaded = run_panel_request(
        api.as_ref(),
        PanelRequest::PullRequest {
            environment_id: ENV.into(),
        },
    )
    .await;
    assert!(matches!(
        loaded,
        PanelLoaded::PullRequest { result: Err(_), .. }
    ));
}

#[test]
fn diff_keys_do_not_fall_through_to_thread_commands_while_loading() {
    let mut state = open_with_environment(Some("main"));
    keys(&mut state, "SPC g g");
    assert!(state.panel.document().is_none());
    for key in ["z R", "g f", "] c", "y y", "x", "d d", "r"] {
        let effects = keys(&mut state, key);
        assert!(
            effects
                .iter()
                .all(|effect| !matches!(effect, Effect::Mutate(_))),
            "{key}: {effects:?}"
        );
    }
    assert!(state.palette.is_none());
    assert_eq!(state.focus, Focus::Panel);
}

#[test]
fn a_wide_panel_hides_the_sidebar_instead_of_crushing_the_timeline() {
    let mut state = open_with_environment(Some("main"));
    keys(&mut state, "g t");
    assert_eq!(state.panel.tab, PanelTab::Diff);
    state.focus = Focus::Main;
    let narrow = render_lines(&state, 100, 30);
    assert!(!narrow[0].contains("Threads"), "{}", narrow.join("\n"));
    let timeline_width = narrow[0]
        .split('┐')
        .next()
        .map_or(0, |segment| segment.chars().count());
    assert!(timeline_width >= 40, "{}", narrow.join("\n"));
    let wide = render_lines(&state, 200, 30);
    assert!(wide[0].contains("Threads"), "{}", wide.join("\n"));
    state.focus = Focus::Sidebar;
    let focused = render_lines(&state, 100, 30);
    assert!(focused[0].contains("Threads"), "{}", focused.join("\n"));
}
