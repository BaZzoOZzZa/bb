mod support;

use std::path::PathBuf;

use bb_tui::api::Mutation;
use bb_tui::app::{Action, AppState, Effect, Focus, Mode, update};
use bb_tui::attach::UploadRequest;
use bb_tui::chips::{CharRange, insert_mention};
use bb_tui::composer::Composer;
use bb_tui::palette::FileSource;
use bb_tui::prompt::PromptDraft;
use bb_tui::runtime::{run_typeahead_request, upload_attachment};
use bb_tui::typeahead::{ComposerTarget, MenuState, TypeaheadLoaded, TypeaheadRequest};
use bb_wire::plugin::{
    PluginMentionProviderContribution, PluginMentionSearchGroup, PluginMentionSearchItem,
};
use bb_wire::project::{
    ProviderCommand, ProviderCommandOrigin, ProviderCommandSource, UploadedAttachmentType,
    UploadedPromptAttachment, WorkspacePathEntry, WorkspacePathListResponse,
};
use bb_wire::prompt::{
    PathEntryKind, PermissionMode, PromptInput, PromptMentionCommandTrigger,
    PromptMentionPathSource, PromptMentionResource, PromptTextMention, ReasoningLevel,
    ResolvedThreadExecutionOptions, ThreadExecutionSource,
};
use bb_wire::system::{
    ProviderComposerAction, ProviderComposerCommand, ProviderInfo, SystemExecutionOptionsResponse,
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::style::Color;
use serde_json::json;
use support::*;

const THREAD_ID: &str = "thr_other";

fn key(state: &mut AppState, code: KeyCode) -> Vec<Effect> {
    update(state, Action::Key(KeyEvent::from(code)))
}

fn chord(state: &mut AppState, code: KeyCode, modifiers: KeyModifiers) -> Vec<Effect> {
    update(state, Action::Key(KeyEvent::new(code, modifiers)))
}

fn type_text(state: &mut AppState, text: &str) -> Vec<Effect> {
    let mut effects = Vec::new();
    for ch in text.chars() {
        effects.extend(key(state, KeyCode::Char(ch)));
    }
    effects
}

fn options(actions: Vec<ProviderComposerAction>) -> SystemExecutionOptionsResponse {
    SystemExecutionOptionsResponse {
        providers: vec![ProviderInfo {
            id: "codex".into(),
            composer_actions: actions,
            ..ProviderInfo::default()
        }],
        ..SystemExecutionOptionsResponse::default()
    }
}

fn composer_actions() -> Vec<ProviderComposerAction> {
    vec![
        ProviderComposerAction::Skills {
            trigger: PromptMentionCommandTrigger::Slash,
        },
        ProviderComposerAction::Skills {
            trigger: PromptMentionCommandTrigger::Dollar,
        },
        ProviderComposerAction::Plan {
            command: ProviderComposerCommand {
                trigger: PromptMentionCommandTrigger::Slash,
                name: "plan".into(),
                trailing_text: " ".into(),
            },
        },
    ]
}

fn setup_with(read_only: bool) -> AppState {
    let mut state = loaded_state(sample_sidebar());
    state.read_only = read_only;
    let index = state
        .rows
        .iter()
        .position(|row| row.thread().is_some_and(|thread| thread.id == THREAD_ID))
        .expect("thread row");
    state.selected = index;
    keys(&mut state, "g o");
    update(
        &mut state,
        Action::ExecutionOptionsLoaded {
            thread_id: THREAD_ID.into(),
            result: Ok(options(composer_actions())),
        },
    );
    update(
        &mut state,
        Action::DefaultsLoaded {
            thread_id: THREAD_ID.into(),
            result: Ok(Some(ResolvedThreadExecutionOptions {
                model: "gpt-5".into(),
                service_tier: String::new(),
                reasoning_level: ReasoningLevel::High,
                permission_mode: PermissionMode::Auto,
                source: ThreadExecutionSource::TurnRequested,
                seq: None,
            })),
        },
    );
    let effects = key(&mut state, KeyCode::Char('i'));
    assert_eq!(state.focus, Focus::Composer);
    assert_eq!(
        typeahead_requests(&effects),
        vec![TypeaheadRequest::MentionProviders]
    );
    state
}

fn setup() -> AppState {
    setup_with(false)
}

fn loaded(state: &mut AppState, loaded: TypeaheadLoaded) {
    update(state, Action::TypeaheadLoaded(loaded));
}

fn providers(state: &mut AppState, providers: Vec<PluginMentionProviderContribution>) {
    loaded(state, TypeaheadLoaded::MentionProviders(Ok(providers)));
}

fn entry(path: &str, kind: PathEntryKind, score: f64) -> WorkspacePathEntry {
    WorkspacePathEntry {
        kind,
        path: path.into(),
        name: path.rsplit('/').next().unwrap_or(path).into(),
        score,
        positions: Vec::new(),
    }
}

fn paths(entries: Vec<WorkspacePathEntry>) -> Result<WorkspacePathListResponse, String> {
    Ok(WorkspacePathListResponse {
        paths: entries,
        ..WorkspacePathListResponse::default()
    })
}

fn typeahead_requests(effects: &[Effect]) -> Vec<TypeaheadRequest> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Typeahead(request) => Some(request.clone()),
            _ => None,
        })
        .collect()
}

fn composer(state: &AppState) -> &Composer {
    state.composer().expect("composer")
}

fn path_resource(path: &str) -> PromptMentionResource {
    PromptMentionResource::Path {
        source: PromptMentionPathSource::Workspace,
        entry_kind: PathEntryKind::File,
        path: path.into(),
        label: path.rsplit('/').next().unwrap_or(path).into(),
    }
}

fn sent_input(effects: &[Effect]) -> Vec<PromptInput> {
    effects
        .iter()
        .find_map(|effect| match effect {
            Effect::Mutate(Mutation::SendMessage { request, .. }) => Some(request.input.clone()),
            _ => None,
        })
        .expect("send message")
}

fn screen(state: &AppState) -> String {
    render_lines(state, 120, 30).join("\n")
}

fn open_readme_mention(state: &mut AppState, prefix: &str) {
    type_text(state, prefix);
    let effects = type_text(state, "@rea");
    let requests = typeahead_requests(&effects);
    assert!(requests.contains(&TypeaheadRequest::WorkspacePaths {
        source: FileSource::Project {
            project_id: "proj_b".into(),
            root: None,
        },
        query: "rea".into(),
    }));
    assert!(requests.contains(&TypeaheadRequest::StoragePaths {
        thread_id: THREAD_ID.into(),
        query: "rea".into(),
    }));
    providers(state, Vec::new());
    loaded(
        state,
        TypeaheadLoaded::WorkspacePaths {
            query: "rea".into(),
            result: paths(vec![
                entry("README.md", PathEntryKind::File, 9.0),
                entry("docs/reading", PathEntryKind::Directory, 4.0),
            ]),
        },
    );
    loaded(
        state,
        TypeaheadLoaded::StoragePaths {
            query: "rea".into(),
            result: paths(Vec::new()),
        },
    );
}

#[test]
fn mention_inserts_a_utf16_chip_and_sends_its_range() {
    let mut state = setup();
    open_readme_mention(&mut state, "Привет 😀 ");
    assert!(state.typeahead_visible());
    let text = screen(&state);
    assert!(text.contains("Workspace"), "{text}");
    assert!(text.contains("README.md"), "{text}");
    assert!(text.contains("docs/reading/"), "{text}");
    key(&mut state, KeyCode::Enter);
    assert!(!state.typeahead_visible());
    assert_eq!(composer(&state).text(), "Привет 😀 @README.md ");
    assert_eq!(
        composer(&state).draft().mentions,
        vec![PromptTextMention {
            start: 10,
            end: 20,
            resource: path_resource("README.md"),
        }]
    );
    type_text(&mut state, "now");
    let input = sent_input(&chord(&mut state, KeyCode::Enter, KeyModifiers::CONTROL));
    assert_eq!(
        serde_json::to_value(&input).unwrap(),
        json!([{
            "type": "text",
            "text": "Привет 😀 @README.md now",
            "mentions": [{
                "start": 10,
                "end": 20,
                "resource": {
                    "kind": "path",
                    "source": "workspace",
                    "entryKind": "file",
                    "path": "README.md",
                    "label": "README.md"
                }
            }]
        }])
    );
}

#[test]
fn arrows_choose_a_directory_and_backspace_removes_the_whole_chip() {
    let mut state = setup();
    open_readme_mention(&mut state, "");
    key(&mut state, KeyCode::Down);
    key(&mut state, KeyCode::Tab);
    assert_eq!(composer(&state).text(), "@docs/reading/ ");
    key(&mut state, KeyCode::Backspace);
    assert_eq!(composer(&state).text(), "@docs/reading/");
    assert_eq!(composer(&state).draft().mentions.len(), 1);
    key(&mut state, KeyCode::Backspace);
    assert_eq!(composer(&state).text(), "");
    assert!(composer(&state).draft().mentions.is_empty());
}

#[test]
fn cursor_skips_over_chips_and_typing_never_lands_inside() {
    let mut state = setup();
    open_readme_mention(&mut state, "a ");
    key(&mut state, KeyCode::Enter);
    type_text(&mut state, "b");
    assert_eq!(composer(&state).text(), "a @README.md b");
    key(&mut state, KeyCode::Left);
    key(&mut state, KeyCode::Left);
    key(&mut state, KeyCode::Left);
    assert_eq!(composer(&state).cursor_char(), 2);
    type_text(&mut state, "x");
    assert_eq!(composer(&state).text(), "a x@README.md b");
    assert_eq!(composer(&state).draft().mentions[0].start, 3);
    key(&mut state, KeyCode::Right);
    assert_eq!(composer(&state).cursor_char(), 13);
    key(&mut state, KeyCode::Delete);
    assert_eq!(composer(&state).text(), "a x@README.mdb");
    key(&mut state, KeyCode::Left);
    key(&mut state, KeyCode::Delete);
    assert_eq!(composer(&state).text(), "a xb");
}

#[test]
fn escape_dismisses_until_the_cursor_leaves_the_trigger() {
    let mut state = setup();
    open_readme_mention(&mut state, "");
    key(&mut state, KeyCode::Esc);
    assert!(!state.typeahead_visible());
    assert_eq!(state.focus, Focus::Composer);
    type_text(&mut state, "d");
    assert!(!state.typeahead_visible());
    key(&mut state, KeyCode::Char(' '));
    type_text(&mut state, "@x");
    assert!(state.typeahead_visible());
}

#[test]
fn empty_queries_show_a_hint_and_enter_still_sends() {
    let mut state = setup();
    type_text(&mut state, "hi @");
    providers(&mut state, Vec::new());
    assert!(matches!(
        state.typeahead.as_ref().map(|typeahead| &typeahead.menu),
        Some(MenuState::Hint(_))
    ));
    let input = sent_input(&chord(&mut state, KeyCode::Enter, KeyModifiers::CONTROL));
    assert_eq!(input, vec![PromptInput::text("hi @")]);
}

#[test]
fn threads_projects_and_sections_come_from_the_sidebar() {
    let mut state = setup();
    let effects = type_text(&mut state, "@alp");
    assert!(!typeahead_requests(&effects).is_empty());
    providers(&mut state, Vec::new());
    let Some(MenuState::Mentions(ordered)) = state.typeahead.as_ref().map(|t| t.menu.clone())
    else {
        panic!("expected mention results");
    };
    assert_eq!(ordered.groups[0].label, "Projects");
    key(&mut state, KeyCode::Enter);
    assert_eq!(composer(&state).text(), "@project:proj_a ");
    let mention = &composer(&state).draft().mentions[0];
    assert_eq!(
        mention.resource,
        PromptMentionResource::Project {
            project_id: "proj_a".into(),
            label: "alpha".into(),
        }
    );

    type_text(&mut state, "@flaky");
    let Some(MenuState::Mentions(ordered)) = state.typeahead.as_ref().map(|t| t.menu.clone())
    else {
        panic!("expected thread results");
    };
    assert_eq!(ordered.groups[0].label, "Threads");
    key(&mut state, KeyCode::Enter);
    assert_eq!(
        composer(&state).text(),
        "@project:proj_a @thread:thr_unread "
    );
    assert_eq!(composer(&state).draft().mentions[1].start, 16);
}

#[test]
fn slash_commands_skills_and_prompt_actions_insert_command_chips() {
    let mut state = setup();
    let effects = type_text(&mut state, "/re");
    let requests = typeahead_requests(&effects);
    let Some(TypeaheadRequest::Commands(catalog)) = requests
        .iter()
        .find(|request| matches!(request, TypeaheadRequest::Commands(_)))
        .cloned()
    else {
        panic!("commands request: {requests:?}");
    };
    assert_eq!(catalog.project_id, "proj_b");
    assert_eq!(catalog.provider_id, "codex");
    assert!(matches!(
        state.typeahead.as_ref().map(|t| &t.menu),
        Some(MenuState::Loading(_))
    ));
    let command = |name: &str, source, origin| ProviderCommand {
        name: name.into(),
        source,
        origin,
        description: Some(format!("{name} things")),
        argument_hint: None,
        plugin_id: None,
    };
    loaded(
        &mut state,
        TypeaheadLoaded::Commands {
            key: catalog,
            result: Ok(vec![
                command(
                    "review",
                    ProviderCommandSource::Command,
                    ProviderCommandOrigin::Builtin,
                ),
                command(
                    "pdf:read",
                    ProviderCommandSource::Skill,
                    ProviderCommandOrigin::User,
                ),
            ]),
        },
    );
    let text = screen(&state);
    assert!(text.contains("/review"), "{text}");
    assert!(text.contains("Skills"), "{text}");
    key(&mut state, KeyCode::Enter);
    assert_eq!(composer(&state).text(), "/review ");
    assert_eq!(
        serde_json::to_value(&composer(&state).draft().mentions[0].resource).unwrap(),
        json!({
            "kind": "command",
            "trigger": "/",
            "name": "review",
            "source": "command",
            "origin": "builtin",
            "label": "review",
            "argumentHint": null
        })
    );

    type_text(&mut state, "$re");
    let Some(MenuState::Commands(ordered)) = state.typeahead.as_ref().map(|t| t.menu.clone())
    else {
        panic!("expected skills");
    };
    assert_eq!(ordered.len(), 1);
    key(&mut state, KeyCode::Enter);
    assert_eq!(composer(&state).text(), "/review $pdf:read ");

    type_text(&mut state, "/pla");
    key(&mut state, KeyCode::Enter);
    assert_eq!(composer(&state).text(), "/review $pdf:read /plan ");
    assert_eq!(composer(&state).draft().mentions.len(), 3);
}

#[test]
fn unknown_slash_text_stays_literal() {
    let mut state = setup();
    let effects = type_text(&mut state, "/zzz");
    let Some(TypeaheadRequest::Commands(catalog)) = typeahead_requests(&effects)
        .into_iter()
        .find(|request| matches!(request, TypeaheadRequest::Commands(_)))
    else {
        panic!("commands request");
    };
    loaded(
        &mut state,
        TypeaheadLoaded::Commands {
            key: catalog,
            result: Ok(Vec::new()),
        },
    );
    assert!(!state.typeahead_visible());
    let input = sent_input(&chord(&mut state, KeyCode::Enter, KeyModifiers::CONTROL));
    assert_eq!(input, vec![PromptInput::text("/zzz")]);
}

#[test]
fn plugin_mentions_use_their_own_trigger() {
    let mut state = setup();
    type_text(&mut state, "x");
    providers(
        &mut state,
        vec![PluginMentionProviderContribution {
            plugin_id: "linear".into(),
            id: "issues".into(),
            label: "Linear".into(),
            triggers: vec!["#".into()],
        }],
    );
    let effects = type_text(&mut state, " #bug");
    let requests = typeahead_requests(&effects);
    assert!(
        requests
            .iter()
            .all(|request| !matches!(request, TypeaheadRequest::WorkspacePaths { .. }))
    );
    let Some(TypeaheadRequest::PluginMentions(query)) = requests.last().cloned() else {
        panic!("plugin search: {requests:?}");
    };
    assert_eq!(query.trigger, "#");
    assert_eq!(query.q, "bug");
    assert_eq!(query.thread_id.as_deref(), Some(THREAD_ID));
    loaded(
        &mut state,
        TypeaheadLoaded::PluginMentions {
            trigger: "#".into(),
            query: "bug".into(),
            result: Ok(vec![PluginMentionSearchGroup {
                plugin_id: "linear".into(),
                provider_id: "issues".into(),
                label: "Linear".into(),
                items: vec![PluginMentionSearchItem {
                    item_id: "ENG-1".into(),
                    title: "Bug in 日本 parser".into(),
                    subtitle: Some("ENG-1".into()),
                    icon: None,
                }],
            }]),
        },
    );
    key(&mut state, KeyCode::Enter);
    assert_eq!(composer(&state).text(), "x #Bug in 日本 parser ");
    let mention = &composer(&state).draft().mentions[0];
    assert_eq!((mention.start, mention.end), (2, 19));
    assert_eq!(
        mention.resource,
        PromptMentionResource::Plugin {
            plugin_id: "linear".into(),
            icon: None,
            item_id: "ENG-1".into(),
            label: "Bug in 日本 parser".into(),
        }
    );
}

#[test]
fn drafts_persist_and_restore_chips_and_attachments() {
    let mut state = setup();
    open_readme_mention(&mut state, "👨‍👩‍👧 ");
    key(&mut state, KeyCode::Enter);
    update(
        &mut state,
        Action::AttachmentUploaded {
            target: ComposerTarget::Thread(THREAD_ID.into()),
            path: PathBuf::from("/tmp/shot.png"),
            result: Ok(UploadedPromptAttachment {
                source_project_id: Some("proj_b".into()),
                attachment_type: UploadedAttachmentType::LocalImage,
                path: "/data/attachments/shot.png".into(),
                name: "shot.png".into(),
                mime_type: Some("image/png".into()),
                size_bytes: 42,
            }),
        },
    );
    let raw = composer(&state)
        .persisted_draft()
        .serialize()
        .expect("draft");
    let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(value["mentions"][0]["start"], json!(9));
    assert_eq!(value["mentions"][0]["end"], json!(19));
    assert_eq!(
        value["attachments"],
        json!([{
            "type": "localImage",
            "path": "/data/attachments/shot.png",
            "name": "shot.png",
            "mimeType": "image/png",
            "sizeBytes": 42,
            "sourceProjectId": "proj_b"
        }])
    );
    let mut restored = Composer::new("thr_x");
    restored.load_stored(Some(&raw));
    assert_eq!(restored.text(), "👨‍👩‍👧 @README.md ");
    assert_eq!(
        restored.mention_ranges(),
        vec![CharRange { start: 6, end: 16 }]
    );
    assert_eq!(restored.draft().attachments.len(), 1);
    let mut broken = Composer::new("thr_y");
    broken.load_stored(Some(
        r#"{"text":"😀ab","mentions":[{"start":1,"end":3,"resource":{"kind":"project","projectId":"p","label":"p"}}]}"#,
    ));
    assert!(broken.mention_ranges().is_empty());
}

#[test]
fn chips_are_rendered_with_the_chip_style() {
    let mut state = setup();
    open_readme_mention(&mut state, "see ");
    key(&mut state, KeyCode::Enter);
    let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
    terminal
        .draw(|frame| bb_tui::ui::render(&state, frame))
        .unwrap();
    let buffer = terminal.backend().buffer();
    let lines = buffer_lines(&terminal);
    let row = lines
        .iter()
        .position(|line| line.contains("see @README.md"))
        .expect("composer row");
    let byte = lines[row].find("@README").unwrap();
    let column = unicode_width::UnicodeWidthStr::width(&lines[row][..byte]);
    let chip = &buffer[(column as u16, row as u16)];
    assert_eq!(chip.bg, Color::Indexed(236));
    let before = &buffer[((column - 2) as u16, row as u16)];
    assert_ne!(before.bg, Color::Indexed(236));
}

#[test]
fn attach_prompt_uploads_and_attachments_can_be_removed() {
    let mut state = setup();
    keys(&mut state, "C-c C-a");
    assert!(matches!(state.mode, Mode::AttachPrompt { .. }));
    for _ in 0..200 {
        key(&mut state, KeyCode::Backspace);
    }
    type_text(&mut state, "'/tmp/a' /tmp/file.txt");
    let effects = key(&mut state, KeyCode::Enter);
    assert_eq!(
        effects,
        vec![Effect::Upload(UploadRequest {
            target: ComposerTarget::Thread(THREAD_ID.into()),
            project_id: "proj_b".into(),
            path: PathBuf::from("/tmp/a"),
        })]
        .into_iter()
        .chain(std::iter::once(Effect::Upload(UploadRequest {
            target: ComposerTarget::Thread(THREAD_ID.into()),
            project_id: "proj_b".into(),
            path: PathBuf::from("/tmp/file.txt"),
        })))
        .collect::<Vec<_>>()
    );
    assert_eq!(composer(&state).uploads.len(), 2);
    assert!(screen(&state).contains("… a"));
    update(
        &mut state,
        Action::AttachmentUploaded {
            target: ComposerTarget::Thread(THREAD_ID.into()),
            path: PathBuf::from("/tmp/a"),
            result: Err("too big".into()),
        },
    );
    assert!(screen(&state).contains("Failed to attach a: too big"));
    update(
        &mut state,
        Action::AttachmentUploaded {
            target: ComposerTarget::Thread(THREAD_ID.into()),
            path: PathBuf::from("/tmp/file.txt"),
            result: Ok(UploadedPromptAttachment {
                source_project_id: Some("proj_b".into()),
                attachment_type: UploadedAttachmentType::LocalFile,
                path: "/data/file.txt".into(),
                name: "file.txt".into(),
                mime_type: Some("text/plain".into()),
                size_bytes: 3,
            }),
        },
    );
    assert!(composer(&state).uploads.is_empty());
    assert!(screen(&state).contains("▤ file.txt"));
    let input = composer(&state).draft().to_input();
    assert_eq!(
        serde_json::to_value(&input).unwrap(),
        json!([{
            "type": "localFile",
            "path": "/data/file.txt",
            "sourceProjectId": "proj_b",
            "name": "file.txt",
            "sizeBytes": 3,
            "mimeType": "text/plain"
        }])
    );
    chord(&mut state, KeyCode::Char('a'), KeyModifiers::ALT);
    assert!(matches!(state.mode, Mode::Attachments { .. }));
    assert!(screen(&state).contains("Attachments"));
    key(&mut state, KeyCode::Char('d'));
    assert_eq!(state.mode, Mode::Normal);
    assert!(composer(&state).draft().attachments.is_empty());
}

#[test]
fn attach_command_and_pasted_paths_upload_files() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("отчёт 1.pdf");
    std::fs::write(&file, b"%PDF").unwrap();
    let mut state = setup();
    type_text(&mut state, &format!(":attach '{}'", file.display()));
    let effects = chord(&mut state, KeyCode::Enter, KeyModifiers::CONTROL);
    assert!(matches!(
        effects.as_slice(),
        [Effect::Upload(UploadRequest { path, .. })] if *path == file
    ));
    assert_eq!(composer(&state).text(), "");

    let effects = update(&mut state, Action::Paste(file.display().to_string()));
    assert!(effects.is_empty(), "already uploading: {effects:?}");
    update(
        &mut state,
        Action::AttachmentUploaded {
            target: ComposerTarget::Thread(THREAD_ID.into()),
            path: file.clone(),
            result: Err("x".into()),
        },
    );
    let uri = format!("file://{}", file.display().to_string().replace(' ', "%20"));
    let effects = update(&mut state, Action::Paste(uri));
    assert!(matches!(
        effects.as_slice(),
        [Effect::Upload(UploadRequest { path, .. })] if *path == file
    ));
    update(&mut state, Action::Paste("/definitely/not/here.txt".into()));
    assert_eq!(composer(&state).text(), "/definitely/not/here.txt");
}

#[test]
fn read_only_refuses_uploads() {
    let mut state = setup_with(true);
    keys(&mut state, "C-c C-a");
    assert_eq!(state.mode, Mode::Normal);
    assert!(screen(&state).contains("Read-only mode: cannot upload attachments"));
    type_text(&mut state, ":attach /tmp/x");
    assert!(chord(&mut state, KeyCode::Enter, KeyModifiers::CONTROL).is_empty());
}

#[test]
fn utf16_ranges_match_the_web_serialization() {
    let prefixes = [
        "",
        "Привет ",
        "😀 ",
        "👨‍👩‍👧‍👦 ",
        "日本語のテキスト ",
        "e\u{301} ",
        "🇺🇦 ",
        "Ж😀日 ",
    ];
    let serialized = [
        "@src/файл.rs",
        "@thread:thr_1",
        "@docs/😀/日本.md",
        "/review",
        "$skill:пдф",
    ];
    let expected: [[(u32, u32, usize); 5]; 8] = [
        [
            (0, 12, 19),
            (0, 13, 20),
            (0, 14, 21),
            (0, 7, 14),
            (0, 10, 17),
        ],
        [
            (7, 19, 26),
            (7, 20, 27),
            (7, 21, 28),
            (7, 14, 21),
            (7, 17, 24),
        ],
        [
            (3, 15, 22),
            (3, 16, 23),
            (3, 17, 24),
            (3, 10, 17),
            (3, 13, 20),
        ],
        [
            (12, 24, 31),
            (12, 25, 32),
            (12, 26, 33),
            (12, 19, 26),
            (12, 22, 29),
        ],
        [
            (9, 21, 28),
            (9, 22, 29),
            (9, 23, 30),
            (9, 16, 23),
            (9, 19, 26),
        ],
        [
            (3, 15, 22),
            (3, 16, 23),
            (3, 17, 24),
            (3, 10, 17),
            (3, 13, 20),
        ],
        [
            (5, 17, 24),
            (5, 18, 25),
            (5, 19, 26),
            (5, 12, 19),
            (5, 15, 22),
        ],
        [
            (5, 17, 24),
            (5, 18, 25),
            (5, 19, 26),
            (5, 12, 19),
            (5, 15, 22),
        ],
    ];
    for (row, prefix) in prefixes.iter().enumerate() {
        for (column, text) in serialized.iter().enumerate() {
            let (start, end, total) = expected[row][column];
            let typed = format!("{prefix}{}", text.chars().next().unwrap());
            let from = typed.chars().count() - 1;
            let inserted = insert_mention(
                &typed,
                &[],
                CharRange {
                    start: from,
                    end: from + 1,
                },
                text,
                path_resource("x"),
                " ",
            );
            let draft = PromptDraft {
                text: format!("  {}tail ✓\n", inserted.text),
                mentions: inserted
                    .mentions
                    .iter()
                    .map(|mention| PromptTextMention {
                        start: mention.start + 2,
                        end: mention.end + 2,
                        resource: mention.resource.clone(),
                    })
                    .collect(),
                attachments: Vec::new(),
            };
            let input = draft.to_input();
            let PromptInput::Text(text_input) = &input[0] else {
                panic!("text input");
            };
            assert_eq!(
                text_input.text.encode_utf16().count(),
                total,
                "{prefix:?} {text:?}"
            );
            assert_eq!(
                (text_input.mentions[0].start, text_input.mentions[0].end),
                (start, end),
                "{prefix:?} {text:?}"
            );
            let round = PromptDraft::from_input(&input);
            assert_eq!(round.mentions, text_input.mentions);
            let restored = PromptDraft::parse(round.serialize().as_deref());
            assert_eq!(restored.mentions, text_input.mentions);
        }
    }
}

#[test]
fn new_thread_prompt_supports_mentions_and_attachments() {
    let mut state = loaded_state(sample_sidebar());
    keys(&mut state, "SPC b N");
    assert_eq!(state.focus, Focus::NewThread);
    key(&mut state, KeyCode::Char('i'));
    let effects = type_text(&mut state, "@rea");
    assert!(
        typeahead_requests(&effects)
            .iter()
            .any(|request| matches!(request, TypeaheadRequest::WorkspacePaths { .. }))
    );
    providers(&mut state, Vec::new());
    loaded(
        &mut state,
        TypeaheadLoaded::WorkspacePaths {
            query: "rea".into(),
            result: paths(vec![entry("README.md", PathEntryKind::File, 9.0)]),
        },
    );
    key(&mut state, KeyCode::Enter);
    let form = state.new_thread_form().expect("form");
    assert_eq!(form.composer.text(), "@README.md ");
    assert_eq!(form.composer.draft().mentions.len(), 1);
    keys(&mut state, "C-c C-a");
    assert!(matches!(
        state.mode,
        Mode::AttachPrompt {
            target: ComposerTarget::NewThread,
            ..
        }
    ));
}

#[tokio::test]
async fn runtime_reads_files_guesses_mime_and_runs_requests() {
    let api = MockApi::new(FakeBackend::new(sample_sidebar()));
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("снимок.PNG");
    std::fs::write(&file, b"\x89PNG").unwrap();
    let uploaded = upload_attachment(
        api.as_ref(),
        &UploadRequest {
            target: ComposerTarget::NewThread,
            project_id: "proj_a".into(),
            path: file.clone(),
        },
    )
    .await
    .unwrap();
    assert_eq!(uploaded.attachment_type, UploadedAttachmentType::LocalImage);
    let upload = api.uploads.lock().unwrap()[0].clone();
    assert_eq!(upload.file_name, "снимок.PNG");
    assert_eq!(upload.mime_type.as_deref(), Some("image/png"));
    assert_eq!(upload.bytes, b"\x89PNG");
    let missing = upload_attachment(
        api.as_ref(),
        &UploadRequest {
            target: ComposerTarget::NewThread,
            project_id: "proj_a".into(),
            path: dir.path().join("missing"),
        },
    )
    .await;
    assert!(missing.is_err());
    let directory = upload_attachment(
        api.as_ref(),
        &UploadRequest {
            target: ComposerTarget::NewThread,
            project_id: "proj_a".into(),
            path: dir.path().to_path_buf(),
        },
    )
    .await;
    assert_eq!(directory, Err("not a regular file".into()));

    let loaded = run_typeahead_request(
        api.as_ref(),
        TypeaheadRequest::StoragePaths {
            thread_id: "thr_1".into(),
            query: "notes".into(),
        },
    )
    .await;
    assert!(matches!(loaded, TypeaheadLoaded::StoragePaths { .. }));
    let (thread_id, query) = api.storage_queries.lock().unwrap()[0].clone();
    assert_eq!(thread_id, "thr_1");
    assert!(query.include_directories);
    assert_eq!(query.limit, Some(16));
}
