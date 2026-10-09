use std::collections::BTreeMap;
use std::fmt::Debug;

use bb_wire::environment::*;
use bb_wire::error::{LifecycleErrorDetails, ThreadNotWritableReason};
use bb_wire::interaction::*;
use bb_wire::plugin::*;
use bb_wire::project::*;
use bb_wire::prompt::*;
use bb_wire::queue::*;
use bb_wire::realtime::*;
use bb_wire::system::*;
use bb_wire::terminal::*;
use bb_wire::thread::*;
use bb_wire::timeline::*;
use bb_wire::{ApiError, JsonValue};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::json;

fn fixture(name: &str) -> JsonValue {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("{path}: {err}"));
    serde_json::from_str(&text).unwrap_or_else(|err| panic!("{path}: {err}"))
}

fn response(key: &str) -> JsonValue {
    fixture("responses.json")
        .get(key)
        .cloned()
        .unwrap_or_else(|| panic!("missing fixture key {key}"))
}

fn parse<T: DeserializeOwned>(value: JsonValue) -> T {
    serde_json::from_value(value).unwrap_or_else(|err| panic!("parse failed: {err}"))
}

fn parse_roundtrip<T>(value: JsonValue) -> T
where
    T: DeserializeOwned + Serialize + PartialEq + Debug,
{
    let parsed: T = parse(value);
    let reserialized = serde_json::to_value(&parsed).expect("serialize");
    let reparsed: T = parse(reserialized);
    assert_eq!(parsed, reparsed, "round trip changed the value");
    parsed
}

fn rows_by_id(rows: &[TimelineRow]) -> BTreeMap<String, &TimelineRow> {
    let mut map = BTreeMap::new();
    for row in rows {
        if let Some(id) = row.id() {
            map.insert(id.to_owned(), row);
        }
        if let Some(children) = row.nested_rows() {
            map.extend(rows_by_id(children));
        }
    }
    map
}

#[test]
fn timeline_response_parses_every_row_kind() {
    let response: ThreadTimelineResponse = parse_roundtrip(fixture("timeline_response.json"));
    assert_eq!(response.max_seq, 44);
    assert_eq!(response.rows.len(), 6);
    assert_eq!(
        response.timeline_page.older_cursor,
        Some(TimelinePaginationCursor {
            anchor_seq: 1,
            anchor_id: "row_user_1".into()
        })
    );
    assert_eq!(
        response.active_thinking.as_ref().map(|t| t.text.as_str()),
        Some("Considering options")
    );
    assert_eq!(
        response.completed_turn_display,
        CompletedTurnDisplay::Collapse
    );
    let todos = response.pending_todos.as_ref().expect("todos");
    assert_eq!(todos.items[1].status, PendingTodoStatus::InProgress);
    assert_eq!(todos.items[2].status, PendingTodoStatus::Unknown);
    assert_eq!(
        response.goal.as_ref().map(|g| g.status),
        Some(GoalStatus::Active)
    );
    assert!(response.context_window_usage.is_some());

    let rows = rows_by_id(&response.rows);
    let expected_ids = [
        "row_user_1",
        "turn_row_1",
        "row_cmd_1",
        "row_tool_1",
        "row_fc_1",
        "row_ws_1",
        "row_wf_1",
        "row_ig_1",
        "row_iv_1",
        "row_fr_1",
        "row_s_1",
        "row_ps_1",
        "row_ext_1",
        "row_ap_1",
        "row_q_1",
        "row_form_1",
        "row_del_1",
        "row_child_fr",
        "row_wfl_1",
        "row_future_work",
        "row_asst_1",
        "row_sys_reason",
        "row_sys_parent",
        "row_sys_err",
        "row_future_kind",
    ];
    for id in expected_ids {
        assert!(rows.contains_key(id), "missing row {id}");
    }

    match rows["row_user_1"] {
        TimelineRow::Conversation(row) => {
            assert_eq!(row.role, ConversationRole::User);
            assert_eq!(row.initiator, Some(TurnInitiator::User));
            assert_eq!(
                row.turn_request.as_ref().map(|r| r.status),
                Some(TurnRequestStatus::Accepted)
            );
            assert!(matches!(
                row.mentions[0].resource,
                PromptMentionResource::Path {
                    entry_kind: PathEntryKind::File,
                    ..
                }
            ));
        }
        other => panic!("unexpected {other:?}"),
    }

    match rows["row_cmd_1"] {
        TimelineRow::Work(TimelineWorkRow::Command(row)) => {
            assert!(row.content_deferred);
            assert_eq!(row.exit_code, Some(0));
            assert_eq!(
                row.output_preview
                    .as_ref()
                    .map(|p| p.full_output_availability),
                Some(FullOutputAvailability::Available)
            );
            assert!(matches!(
                row.activity_intents[0],
                ActivityIntent::Read { .. }
            ));
            assert!(matches!(
                row.activity_intents[1],
                ActivityIntent::Generic { .. }
            ));
            assert!(matches!(
                row.activity_intents[2],
                ActivityIntent::Unknown(_)
            ));
        }
        other => panic!("unexpected {other:?}"),
    }

    assert!(matches!(
        rows["row_tool_1"],
        TimelineRow::Work(TimelineWorkRow::Tool(row)) if row.presentation.is_some()
    ));
    match rows["row_fc_1"] {
        TimelineRow::Work(TimelineWorkRow::FileChange(row)) => {
            assert_eq!(
                row.change.diff_stats,
                DiffStats {
                    added: 1,
                    removed: 1
                }
            );
            assert_eq!(
                row.approval_status,
                Some(TimelineApprovalStatus::WaitingForApproval)
            );
        }
        other => panic!("unexpected {other:?}"),
    }
    assert!(matches!(
        rows["row_ws_1"],
        TimelineRow::Work(TimelineWorkRow::WebSearch(_))
    ));
    assert!(matches!(
        rows["row_wf_1"],
        TimelineRow::Work(TimelineWorkRow::WebFetch(_))
    ));
    assert!(matches!(
        rows["row_ig_1"],
        TimelineRow::Work(TimelineWorkRow::ImageGeneration(_))
    ));
    assert!(matches!(
        rows["row_iv_1"],
        TimelineRow::Work(TimelineWorkRow::ImageView(_))
    ));
    assert!(matches!(
        rows["row_fr_1"],
        TimelineRow::Work(TimelineWorkRow::FileRead(_))
    ));
    assert!(matches!(
        rows["row_s_1"],
        TimelineRow::Work(TimelineWorkRow::Search(row)) if row.mode == SearchMode::Content
    ));
    match rows["row_ps_1"] {
        TimelineRow::Work(TimelineWorkRow::PlanSteps(row)) => {
            assert_eq!(row.steps[1].status, Some(PlanStepStatus::Active));
            assert_eq!(row.steps[2].status, None);
        }
        other => panic!("unexpected {other:?}"),
    }
    assert!(matches!(
        rows["row_ext_1"],
        TimelineRow::Work(TimelineWorkRow::Extension(_))
    ));
    match rows["row_ap_1"] {
        TimelineRow::Work(TimelineWorkRow::Approval(row)) => {
            assert_eq!(row.approval_kind, ApprovalKind::PermissionGrant);
            assert_eq!(row.lifecycle, ApprovalLifecycle::Pending);
        }
        other => panic!("unexpected {other:?}"),
    }
    match rows["row_q_1"] {
        TimelineRow::Work(TimelineWorkRow::Question(row)) => {
            assert_eq!(row.lifecycle, QuestionLifecycle::Answered);
            let answers = row.answers.as_ref().expect("answers");
            assert_eq!(answers["q1"].selected, vec!["sqlite".to_owned()]);
        }
        other => panic!("unexpected {other:?}"),
    }
    assert!(matches!(
        rows["row_form_1"],
        TimelineRow::Work(TimelineWorkRow::Form(row)) if row.lifecycle == FormLifecycle::Submitted
    ));
    match rows["row_del_1"] {
        TimelineRow::Work(TimelineWorkRow::Delegation(row)) => {
            assert_eq!(row.child_rows.as_ref().map(Vec::len), Some(1));
        }
        other => panic!("unexpected {other:?}"),
    }
    match rows["row_wfl_1"] {
        TimelineRow::Work(TimelineWorkRow::Workflow(row)) => {
            assert_eq!(row.task_status, BackgroundTaskStatus::Completed);
            let workflow = row.workflow.as_ref().expect("workflow");
            assert_eq!(workflow.agents[0].state, WorkflowAgentState::Done);
            assert_eq!(row.usage.as_ref().map(|u| u.duration_ms), Some(2500.5));
        }
        other => panic!("unexpected {other:?}"),
    }
    match rows["row_future_work"] {
        row @ TimelineRow::Work(TimelineWorkRow::Unknown(value)) => {
            assert_eq!(value["intensity"], 11);
            assert!(row.is_unknown());
            assert_eq!(row.source_seq_end(), Some(33));
        }
        other => panic!("unexpected {other:?}"),
    }
    match rows["row_asst_1"] {
        TimelineRow::Conversation(row) => {
            assert_eq!(row.role, ConversationRole::Assistant);
            assert!(row.turn_request.is_none());
        }
        other => panic!("unexpected {other:?}"),
    }
    match rows["row_sys_reason"] {
        TimelineRow::System(row) => {
            assert_eq!(row.system_kind, SystemRowKind::Operation);
            assert_eq!(row.operation_kind, Some(SystemOperationKind::Reasoning));
            assert!(row.content_deferred);
        }
        other => panic!("unexpected {other:?}"),
    }
    match rows["row_sys_parent"] {
        TimelineRow::System(row) => {
            assert_eq!(
                row.parent_change.as_ref().map(|p| p.action),
                Some(ParentChangeAction::Assign)
            );
        }
        other => panic!("unexpected {other:?}"),
    }
    assert!(matches!(
        rows["row_sys_err"],
        TimelineRow::System(row) if row.system_kind == SystemRowKind::Error
    ));
    assert!(matches!(rows["row_future_kind"], TimelineRow::Unknown(_)));
}

#[test]
fn unknown_rows_serialize_back_to_their_original_json() {
    let response: ThreadTimelineResponse = parse(fixture("timeline_response.json"));
    let future = response
        .rows
        .iter()
        .find(|row| row.id() == Some("row_future_kind"))
        .expect("future row");
    assert_eq!(
        serde_json::to_value(future).unwrap(),
        json!({"kind": "sticker", "id": "row_future_kind", "threadId": "thr_demo", "emoji": "🎉"})
    );
}

#[test]
fn timeline_delta_parses_upserts_and_order() {
    let delta: TimelineDelta = parse_roundtrip(json!({
        "upsertRows": [{
            "kind": "conversation", "role": "assistant", "id": "a", "threadId": "t",
            "turnId": null, "sourceSeqStart": 1, "sourceSeqEnd": 2, "startedAt": 0,
            "createdAt": 0, "messageSeq": 1, "text": "hi", "attachments": null, "turnRequest": null
        }],
        "rowOrder": ["a"]
    }));
    assert_eq!(delta.row_order, Some(vec!["a".to_owned()]));
    assert_eq!(delta.upsert_rows[0].id(), Some("a"));
}

#[test]
fn sidebar_bootstrap_tolerates_unknown_statuses_and_fields() {
    let bootstrap: SidebarBootstrapResponse = parse_roundtrip(fixture("sidebar_bootstrap.json"));
    assert_eq!(bootstrap.sections.len(), 1);
    assert_eq!(
        bootstrap.personal_project.project.kind,
        ProjectKind::Personal
    );
    let project = &bootstrap.projects[0];
    assert_eq!(project.sources[0].path, "/work/demo");
    assert_eq!(
        project
            .default_execution_options
            .as_ref()
            .map(|d| d.reasoning_level),
        Some(ReasoningLevel::High)
    );
    let active = &project.threads[0];
    assert_eq!(active.thread.status, ThreadStatus::Active);
    assert_eq!(active.queued_work, ThreadQueuedWork::Waiting);
    assert!(active.has_pending_interaction);
    assert!(active.thread.is_pinned());
    assert_eq!(
        active.environment_workspace_display_kind,
        EnvironmentWorkspaceDisplayKind::ManagedWorktree
    );
    let future = &project.threads[1];
    assert_eq!(future.thread.status, ThreadStatus::Unknown);
    assert_eq!(future.runtime.display_status, ThreadDisplayStatus::Unknown);
    assert_eq!(future.thread.origin_kind, Some(ThreadOriginKind::Unknown));
    assert_eq!(future.thread.title_or_fallback(), Some("Untitled draft"));
}

#[test]
fn thread_response_with_includes() {
    let thread: ThreadResponse = parse_roundtrip(response("thread_response"));
    assert_eq!(
        thread.runtime.display_status,
        ThreadDisplayStatus::WaitingForHost
    );
    assert_eq!(thread.queued_message_count, 2);
    let environment = thread.environment.as_ref().expect("environment");
    assert_eq!(environment.status, EnvironmentStatus::Ready);
    assert_eq!(environment.resolved_merge_base_branch(), Some("main"));
    let host = thread.host.as_ref().expect("host");
    assert_eq!(host.status, HostStatus::Connected);
    assert_eq!(host.max_permission_mode, PermissionMode::Full);
}

#[test]
fn thread_search_response() {
    let search: ThreadSearchResponse = parse_roundtrip(response("thread_search"));
    let result = &search.active.results[0];
    assert_eq!(result.thread.thread.id, "thr_demo");
    assert_eq!(
        result.matches[0].source_kind,
        ThreadSearchSourceKind::UserMessage
    );
    assert_eq!(
        result.matches[0].highlight_ranges[0],
        HighlightRange { start: 4, end: 7 }
    );
    assert_eq!(
        result.matches[1].source_kind,
        ThreadSearchSourceKind::Unknown
    );
}

#[test]
fn pending_interactions_cover_every_payload_kind() {
    let interactions: Vec<PendingInteraction> = parse_roundtrip(fixture("interactions.json"));
    assert_eq!(interactions.len(), 8);

    match &interactions[0].payload {
        InteractionPayload::Approval(payload) => {
            assert!(matches!(
                &payload.subject,
                ApprovalSubject::Command { command, actions, .. }
                    if command == "rm -rf build" && matches!(actions[0], CommandAction::ListFiles { .. })
            ));
            assert_eq!(payload.available_decisions.len(), 3);
        }
        other => panic!("unexpected {other:?}"),
    }
    assert!(interactions[0].is_open());

    match &interactions[1].payload {
        InteractionPayload::Approval(payload) => {
            assert!(matches!(
                payload.subject,
                ApprovalSubject::PermissionGrant { .. }
            ));
            assert_eq!(payload.available_decisions[2], ApprovalDecision::Unknown);
        }
        other => panic!("unexpected {other:?}"),
    }
    match &interactions[1].resolution {
        Some(InteractionResolution::Approval(resolution)) => {
            assert_eq!(resolution.decision(), ApprovalDecision::AllowForSession);
        }
        other => panic!("unexpected {other:?}"),
    }

    assert!(matches!(
        &interactions[2].payload,
        InteractionPayload::Approval(ApprovalPayload {
            subject: ApprovalSubject::Plan { .. },
            ..
        })
    ));
    assert_eq!(interactions[2].expires_at, Some(1760000700000));

    match &interactions[3].payload {
        InteractionPayload::UserQuestion(payload) => {
            assert!(payload.questions[0].multi_select);
        }
        other => panic!("unexpected {other:?}"),
    }
    match &interactions[3].resolution {
        Some(InteractionResolution::Kinded(KindedResolution::UserAnswer { answers })) => {
            assert_eq!(answers["q_color"].free_text.as_deref(), Some("and teal"));
        }
        other => panic!("unexpected {other:?}"),
    }

    assert_eq!(
        interactions[4].renderer_id(),
        Some(ASK_USER_QUESTION_RENDERER_ID)
    );
    let auq: AskUserQuestionPluginPayload = interactions[4].plugin_data().expect("auq data");
    assert_eq!(auq.questions[0].options.as_ref().map(Vec::len), Some(2));
    assert_eq!(
        auq.questions[0].options.as_ref().unwrap()[0]
            .preview
            .as_deref(),
        Some("use chrono::Utc;")
    );

    assert_eq!(
        interactions[5].renderer_id(),
        Some(SECRET_REQUEST_RENDERER_ID)
    );
    assert_eq!(interactions[5].plugin_id(), Some("secrets"));
    let secret: SecretRequestPayload = interactions[5].plugin_data().expect("secret data");
    assert_eq!(secret.fields[0].name, "API_KEY");
    assert_eq!(secret.destination.path, "/work/demo/.env");

    match &interactions[6].payload {
        InteractionPayload::Extension(payload) => assert_eq!(payload.kind, "acme/picker"),
        other => panic!("unexpected {other:?}"),
    }
    assert!(matches!(
        interactions[6].resolution,
        Some(InteractionResolution::Kinded(
            KindedResolution::RequestAnswer { .. }
        ))
    ));

    let mystery = &interactions[7];
    assert_eq!(mystery.status, PendingInteractionStatus::Unknown);
    assert!(matches!(mystery.payload, InteractionPayload::Unknown(_)));
    assert!(matches!(
        mystery.origin,
        Some(InteractionOrigin::Unknown(_))
    ));
    assert!(matches!(
        mystery.resolution,
        Some(InteractionResolution::Unknown(_))
    ));
}

#[test]
fn resolution_requests_serialize_to_the_server_shapes() {
    assert_eq!(
        serde_json::to_value(InteractionResolution::approval(ApprovalDecision::AllowOnce)).unwrap(),
        json!({"decision": "allow_once", "grantedPermissions": null})
    );
    assert_eq!(
        serde_json::to_value(InteractionResolution::approval(ApprovalDecision::Deny)).unwrap(),
        json!({"decision": "deny"})
    );
    let answers = BTreeMap::from([(
        "q1".to_owned(),
        UserAnswer {
            selected: vec!["a".into()],
            free_text: None,
        },
    )]);
    assert_eq!(
        serde_json::to_value(InteractionResolution::user_answer(answers)).unwrap(),
        json!({"kind": "user_answer", "answers": {"q1": {"selected": ["a"]}}})
    );
    let secret = SecretRequestResponse {
        values: BTreeMap::from([("API_KEY".to_owned(), "s3cr3t".to_owned())]),
    };
    assert_eq!(
        serde_json::to_value(RespondPluginInteractionRequest {
            value: serde_json::to_value(secret).unwrap()
        })
        .unwrap(),
        json!({"value": {"values": {"API_KEY": "s3cr3t"}}})
    );
}

#[test]
fn queued_messages_and_send_responses() {
    let queued: Vec<ThreadQueuedMessage> = parse_roundtrip(response("queued_messages"));
    assert_eq!(
        queued[0].waiting_on,
        Some(QueuedMessageWaitingOn::ThreadBusy)
    );
    assert_eq!(queued[0].payload, QueuedMessagePayload::Inline);
    assert!(matches!(
        &queued[1].waiting_on,
        Some(QueuedMessageWaitingOn::Plugin { reason, .. }) if reason == "Rate limited"
    ));
    assert!(matches!(
        queued[1].payload,
        QueuedMessagePayload::Retry { attempt: 2, .. }
    ));
    assert!(matches!(queued[1].content[0], PromptInput::LocalImage(_)));
    assert!(matches!(queued[1].content[1], PromptInput::Unknown(_)));
    assert_eq!(queued[1].permission_mode, PermissionMode::Unknown);

    let sent: SendMessageResponse = parse_roundtrip(response("send_sent"));
    assert_eq!(sent, SendMessageResponse::Sent { ok: true });
    match parse_roundtrip::<SendMessageResponse>(response("send_queued")) {
        SendMessageResponse::Queued { queued_message, .. } => {
            assert_eq!(
                queued_message.waiting_on,
                Some(QueuedMessageWaitingOn::HostOffline {
                    host_name: "laptop".into()
                })
            );
        }
        other => panic!("unexpected {other:?}"),
    }
    assert!(matches!(
        parse::<SendMessageResponse>(response("send_future")),
        SendMessageResponse::Unknown(_)
    ));
    assert!(matches!(
        parse_roundtrip::<RetryTurnResponse>(response("retry_queued")),
        RetryTurnResponse::Queued {
            attempt: 3,
            waiting_on: QueuedMessageWaitingOn::Time,
            ..
        }
    ));
}

#[test]
fn send_request_serializes_minimal_body() {
    let mut request = SendMessageRequest::new(
        vec![PromptInput::text("continue")],
        SendMessageMode::QueueIfActive,
    );
    request.reasoning_level = Some(ReasoningLevel::Xhigh);
    assert_eq!(
        serde_json::to_value(&request).unwrap(),
        json!({
            "input": [{"type": "text", "text": "continue", "mentions": []}],
            "reasoningLevel": "xhigh",
            "mode": "queue-if-active"
        })
    );
    assert_eq!(
        serde_json::to_value(SendQueuedMessageRequest {
            mode: SendQueuedMessageMode::Steer
        })
        .unwrap(),
        json!({"mode": "steer"})
    );
}

#[test]
fn create_thread_request_serializes_environment_args() {
    let mut request = CreateThreadRequest::new(
        "proj_demo",
        CreateThreadEnvironmentArgs::Host {
            host_id: Some("host_local".into()),
            workspace: WorkspaceArgs::ManagedWorktree {
                base_branch: GitBranchSelection::Named {
                    name: "main".into(),
                },
            },
        },
        vec![PromptInput::text("hello")],
    );
    request.permission_mode = Some(PermissionMode::AcceptEdits);
    assert_eq!(
        serde_json::to_value(&request).unwrap(),
        json!({
            "projectId": "proj_demo",
            "origin": "app",
            "input": [{"type": "text", "text": "hello", "mentions": []}],
            "permissionMode": "accept-edits",
            "environment": {
                "type": "host",
                "hostId": "host_local",
                "workspace": {"type": "managed-worktree", "baseBranch": {"kind": "named", "name": "main"}}
            }
        })
    );
    assert_eq!(
        serde_json::to_value(CreateThreadEnvironmentArgs::Provider {
            environment_provider_id: "cloud".into(),
            machine: Some(MachineSelection::Existing {
                host_id: "h1".into()
            }),
            inputs: None,
        })
        .unwrap(),
        json!({"type": "provider", "environmentProviderId": "cloud", "machine": {"type": "existing", "hostId": "h1"}, "inputs": null})
    );
    assert_eq!(
        serde_json::to_value(CreateThreadEnvironmentArgs::ProjectDefault).unwrap(),
        json!({"type": "project-default"})
    );
}

#[test]
fn update_thread_request_distinguishes_clear_from_absent() {
    let request = UpdateThreadRequest {
        section_id: Some(None),
        ..UpdateThreadRequest::rename("New title")
    };
    assert_eq!(
        serde_json::to_value(&request).unwrap(),
        json!({"title": "New title", "sectionId": null})
    );
}

#[test]
fn execution_options_and_environment_providers() {
    let options: SystemExecutionOptionsResponse = parse_roundtrip(response("execution_options"));
    let provider = &options.providers[0];
    assert_eq!(provider.capabilities.permission_modes.len(), 3);
    assert_eq!(provider.completed_turn_display, CompletedTurnDisplay::Flat);
    assert!(matches!(
        provider.composer_actions[0],
        ProviderComposerAction::Skills {
            trigger: PromptMentionCommandTrigger::Dollar
        }
    ));
    assert!(matches!(
        provider.composer_actions[2],
        ProviderComposerAction::Unknown(_)
    ));
    let model = &options.models[0];
    assert_eq!(model.default_reasoning_effort, ReasoningLevel::Medium);
    assert_eq!(
        model.supported_reasoning_efforts[2].reasoning_effort,
        ReasoningLevel::Unknown
    );
    assert_eq!(
        options.model_load_error.as_ref().map(|e| e.code),
        Some(ModelLoadErrorCode::AuthRequired)
    );

    let providers: SystemEnvironmentProvidersResponse =
        parse_roundtrip(response("environment_providers"));
    let local = &providers.providers[0];
    assert!(
        local
            .availability
            .as_ref()
            .is_some_and(ProviderAvailability::is_available)
    );
    assert_eq!(local.machine_availability["host_local"], None);
    assert!(matches!(
        local.machine_availability["host_remote"],
        Some(ProviderAvailability::SetupRequired { .. })
    ));
    assert!(matches!(
        providers.providers[1].availability,
        Some(ProviderAvailability::Unknown(_))
    ));
}

#[test]
fn git_status_diff_and_pull_request_endpoints() {
    match parse_roundtrip::<EnvironmentStatusResponse>(response("environment_status_available")) {
        EnvironmentStatusResponse::Available { workspace } => {
            assert_eq!(
                workspace.working_tree.state,
                WorkspaceState::DirtyUncommitted
            );
            assert_eq!(
                workspace.working_tree.files[1].status,
                WorkspaceFileStatusKind::Untracked
            );
            assert!(matches!(workspace.checkout, GitCheckoutRef::Branch { .. }));
            assert_eq!(
                workspace.merge_base.as_ref().map(|m| m.ahead_count),
                Some(2)
            );
        }
        other => panic!("unexpected {other:?}"),
    }
    assert!(matches!(
        parse_roundtrip::<EnvironmentStatusResponse>(response("environment_status_not_applicable")),
        EnvironmentStatusResponse::NotApplicable { .. }
    ));
    assert!(matches!(
        parse_roundtrip::<EnvironmentStatusResponse>(response("environment_status_unavailable")),
        EnvironmentStatusResponse::Unavailable { .. }
    ));
    assert!(matches!(
        parse::<EnvironmentStatusResponse>(response("environment_status_future")),
        EnvironmentStatusResponse::Unknown(_)
    ));

    match parse_roundtrip::<EnvironmentDiffFilesResponse>(response("diff_files")) {
        EnvironmentDiffFilesResponse::Available {
            files,
            initial_patches,
            ..
        } => {
            assert_eq!(files[1].change_kind, GitDiffFileChangeKind::Renamed);
            assert_eq!(files[1].load_mode, DiffFileLoadMode::TooLarge);
            assert_eq!(initial_patches.len(), 1);
        }
        other => panic!("unexpected {other:?}"),
    }
    assert!(matches!(
        parse_roundtrip::<EnvironmentDiffResponse>(response("diff")),
        EnvironmentDiffResponse::Available { .. }
    ));
    let file: EnvironmentDiffFileResponse = parse_roundtrip(response("diff_file"));
    assert_eq!(file.content_encoding, ContentEncoding::Base64);

    match parse_roundtrip::<EnvironmentPullRequestResponse>(response("pull_request")) {
        EnvironmentPullRequestResponse::Available { pull_request } => {
            assert_eq!(pull_request.number, 42);
            assert_eq!(
                pull_request.attention,
                PullRequestAttentionState::ReviewRequested
            );
            assert_eq!(pull_request.checks.state, PullRequestChecksState::Passing);
        }
        other => panic!("unexpected {other:?}"),
    }
    assert_eq!(
        parse_roundtrip::<EnvironmentPullRequestResponse>(response("pull_request_absent")),
        EnvironmentPullRequestResponse::Absent
    );
    assert!(matches!(
        parse_roundtrip::<EnvironmentActionResponse>(response("action_commit")),
        EnvironmentActionResponse::Commit { .. }
    ));
    assert_eq!(
        serde_json::to_value(EnvironmentActionRequest::PullRequestMerge {
            options: PullRequestMergeOptions {
                method: PullRequestMergeMethod::Squash
            }
        })
        .unwrap(),
        json!({"action": "pull_request_merge", "options": {"method": "squash"}})
    );

    let branches: ProjectBranchesResponse = parse_roundtrip(response("branches"));
    assert!(matches!(
        branches.checkout,
        GitCheckoutRef::Detached { head_sha: None }
    ));
    assert_eq!(branches.operation.kind, WorkspaceGitOperationKind::Rebase);
    assert_eq!(
        branches.default_branch_relation,
        Some(DefaultBranchRelation::LocalBehind)
    );
}

#[test]
fn diff_queries_encode_as_flat_query_strings() {
    let query = serde_urlencoded::to_string(EnvironmentDiffQuery::BranchCommitted {
        merge_base_branch: "main".into(),
    })
    .unwrap();
    assert_eq!(query, "target=branch_committed&mergeBaseBranch=main");
    let query = serde_urlencoded::to_string(EnvironmentDiffFileQuery::Uncommitted {
        path: "src/a b.rs".into(),
        side: DiffFileSide::New,
    })
    .unwrap();
    assert_eq!(query, "target=uncommitted&path=src%2Fa+b.rs&side=new");
}

#[test]
fn timeline_query_strings_match_the_server_contract() {
    assert_eq!(
        serde_urlencoded::to_string(ThreadTimelineQuery::after_sequence(44)).unwrap(),
        "afterSequence=44&deferContent=true"
    );
    let cursor = TimelinePaginationCursor {
        anchor_seq: 7,
        anchor_id: "row_7".into(),
    };
    assert_eq!(
        serde_urlencoded::to_string(ThreadTimelineQuery::older(&cursor)).unwrap(),
        "beforeAnchorSeq=7&beforeAnchorId=row_7&deferContent=true"
    );
    assert_eq!(
        serde_urlencoded::to_string(TurnSummaryDetailsQuery {
            turn_id: "turn_1".into(),
            source_seq_start: 2,
            source_seq_end: 40,
            defer_content: Some(true),
            ..TurnSummaryDetailsQuery::default()
        })
        .unwrap(),
        "deferContent=true&turnId=turn_1&sourceSeqStart=2&sourceSeqEnd=40"
    );
    assert_eq!(
        serde_urlencoded::to_string(ThreadListQuery {
            project_id: Some("proj_demo".into()),
            archived: Some(false),
            limit: Some(50),
            ..ThreadListQuery::default()
        })
        .unwrap(),
        "projectId=proj_demo&archived=false&limit=50"
    );
    assert_eq!(
        serde_urlencoded::to_string(TerminalWebSocketQuery {
            since_seq: 12,
            output_acks: true
        })
        .unwrap(),
        "sinceSeq=12&outputAcks=1"
    );
}

#[test]
fn realtime_server_messages_are_lenient() {
    let messages: Vec<ServerMessage> = parse(fixture("realtime_messages.json"));
    assert_eq!(messages.len(), 12);
    match &messages[0] {
        ServerMessage::Changed(ChangedMessage::Thread {
            id,
            metadata,
            changes,
        }) => {
            assert_eq!(id.as_deref(), Some("thr_demo"));
            assert_eq!(
                changes,
                &vec![
                    ThreadChangeKind::EventsAppended,
                    ThreadChangeKind::StatusChanged,
                    ThreadChangeKind::Unknown
                ]
            );
            let metadata = metadata.as_ref().expect("metadata");
            assert_eq!(metadata.timeline_sequence, Some(45));
            assert_eq!(
                metadata.status_change.as_ref().map(|s| s.status),
                Some(ThreadStatus::Idle)
            );
        }
        other => panic!("unexpected {other:?}"),
    }
    let cleaned = match &messages[0] {
        ServerMessage::Changed(changed) => changed.clone().without_unknown_kinds(),
        other => panic!("unexpected {other:?}"),
    };
    assert!(matches!(cleaned, ChangedMessage::Thread { ref changes, .. } if changes.len() == 2));

    match &messages[1] {
        ServerMessage::Changed(ChangedMessage::Thread { metadata, .. }) => {
            assert_eq!(
                metadata.as_ref().and_then(|m| m.status_change.as_ref()),
                None
            );
        }
        other => panic!("unexpected {other:?}"),
    }
    assert!(matches!(
        &messages[2],
        ServerMessage::Changed(ChangedMessage::Project { changes, .. })
            if changes == &vec![ProjectChangeKind::ThreadsChanged]
    ));
    assert!(matches!(
        &messages[3],
        ServerMessage::Changed(ChangedMessage::Environment { changes, .. })
            if changes == &vec![EnvironmentChangeKind::GitRefsChanged, EnvironmentChangeKind::StatusChanged]
    ));
    assert!(matches!(
        &messages[4],
        ServerMessage::Changed(ChangedMessage::Host { .. })
    ));
    assert!(matches!(
        &messages[5],
        ServerMessage::Changed(ChangedMessage::System { changes })
            if changes == &vec![SystemChangeKind::PluginsChanged, SystemChangeKind::ConfigChanged]
    ));
    assert!(matches!(
        &messages[6],
        ServerMessage::Changed(ChangedMessage::Unknown(_))
    ));
    assert_eq!(messages[7], ServerMessage::Pong);
    match &messages[8] {
        ServerMessage::PluginSignal {
            plugin_id,
            channel,
            payload,
        } => {
            assert_eq!(plugin_id, PUSH_NOTIFICATIONS_PLUGIN_ID);
            assert_eq!(channel, PUSH_NOTIFICATION_CHANNEL);
            let notification: ClientNotification = parse(payload.clone());
            assert_eq!(notification.channels, vec![NotificationChannel::Desktop]);
            assert_eq!(notification.thread_id.as_deref(), Some("thr_demo"));
        }
        other => panic!("unexpected {other:?}"),
    }
    assert!(matches!(
        &messages[9],
        ServerMessage::ThreadOpen {
            split: ThreadOpenSplit::Right,
            file: Some(ThreadOpenFile {
                line_number: Some(3),
                ..
            }),
            ..
        }
    ));
    assert!(matches!(
        &messages[10],
        ServerMessage::ThreadPaneAction {
            action: ThreadPaneAction::Maximize,
            ..
        }
    ));
    assert!(matches!(&messages[11], ServerMessage::Unknown(_)));
    assert!(matches!(
        ServerMessage::parse("not json"),
        ServerMessage::Unknown(_)
    ));
}

#[test]
fn realtime_client_messages_match_the_protocol() {
    let subscribe = ClientMessage::Subscribe {
        target: SubscriptionTarget::ThreadDetail {
            thread_id: "thr_demo".into(),
        },
    };
    assert_eq!(
        serde_json::to_value(&subscribe).unwrap(),
        json!({"type": "subscribe", "target": {"kind": "thread-detail", "threadId": "thr_demo"}})
    );
    assert_eq!(
        ClientMessage::Unsubscribe {
            target: SubscriptionTarget::ThreadList
        }
        .to_json(),
        r#"{"type":"unsubscribe","target":{"kind":"thread-list"}}"#
    );
    assert_eq!(ClientMessage::Ping.to_json(), r#"{"type":"ping"}"#);
    assert_eq!(
        SubscriptionTarget::EnvironmentDetail {
            environment_id: "env_1".into()
        }
        .key(),
        "environment-detail:env_1"
    );
}

#[test]
fn terminal_api_and_websocket_frames() {
    let list: TerminalListResponse = parse_roundtrip(response("terminal_list"));
    assert!(list.sessions[0].status.is_active());
    let output: TerminalOutputResponse = parse_roundtrip(response("terminal_output"));
    assert_eq!(output.chunks[0].decode().unwrap(), b"hello\r\n");
    assert_eq!(output.close_reason, Some(TerminalCloseReason::ProcessExit));

    let frames: Vec<TerminalServerMessage> = parse_roundtrip(response("terminal_frames"));
    assert!(matches!(
        &frames[0],
        TerminalServerMessage::Attached { next_seq: 3, session, .. }
            if session.status == TerminalSessionStatus::Starting
    ));
    match &frames[1] {
        TerminalServerMessage::Output { chunk } => {
            assert_eq!(chunk.decode().unwrap(), b"\x1b[31mred\x1b[0m");
        }
        other => panic!("unexpected {other:?}"),
    }
    assert!(matches!(frames[2], TerminalServerMessage::Error { .. }));
    assert_eq!(frames[3], TerminalServerMessage::Pong);
    assert!(matches!(frames[4], TerminalServerMessage::Unknown(_)));

    assert_eq!(
        serde_json::to_value(TerminalClientMessage::input(b"ls\r")).unwrap(),
        json!({"type": "input", "dataBase64": "bHMN"})
    );
    assert_eq!(
        serde_json::to_value(TerminalClientMessage::Ack { next_seq: 9 }).unwrap(),
        json!({"type": "ack", "nextSeq": 9})
    );
    assert_eq!(
        serde_json::to_value(TerminalClientMessage::Resize {
            cols: 100,
            rows: 30
        })
        .unwrap(),
        json!({"type": "resize", "cols": 100, "rows": 30})
    );
    let big = vec![b'x'; TERMINAL_DATA_MAX_BYTES + 1];
    assert_eq!(TerminalClientMessage::input_chunks(&big).len(), 2);

    let create = CreateTerminalRequest {
        cols: 80,
        rows: 24,
        start: Some(TerminalStart::Shell),
        target: TerminalCreateTarget::HostPath {
            host_id: "host_local".into(),
            cwd: None,
        },
        title: None,
    };
    assert_eq!(
        serde_json::to_value(&create).unwrap(),
        json!({"cols": 80, "rows": 24, "start": {"mode": "shell"}, "target": {"kind": "host_path", "hostId": "host_local", "cwd": null}})
    );
    assert_eq!(
        serde_json::to_value(CloseTerminalRequest::by_user(TerminalCloseMode::IfClean)).unwrap(),
        json!({"mode": "if-clean", "reason": "user"})
    );
}

#[test]
fn plugin_contributions_settings_and_rpc() {
    let contributions: PluginContributionsResponse =
        parse_roundtrip(response("plugin_contributions"));
    assert_eq!(
        contributions.cli_commands[0].commands[0].usage,
        "bb automation list"
    );
    assert_eq!(contributions.mention_providers[0].triggers, vec!["@", "#"]);

    let settings: PluginSettingsResponse = parse_roundtrip(response("plugin_settings"));
    assert!(settings.schema["token"].is_secret());
    assert!(matches!(
        settings.schema["notes"],
        PluginSettingDescriptor::String {
            multiline: Some(true),
            ..
        }
    ));
    assert!(matches!(
        settings.schema["limit"],
        PluginSettingDescriptor::Number {
            default: Some(2.5),
            ..
        }
    ));
    assert!(matches!(
        settings.schema["mode"],
        PluginSettingDescriptor::Select { ref options, .. } if options.len() == 2
    ));
    assert!(matches!(
        settings.schema["color"],
        PluginSettingDescriptor::Unknown(_)
    ));
    assert_eq!(settings.schema["color"].label(), Some("Color"));
    assert_eq!(settings.values["limit"], 3);

    let plugins: PluginListResponse = parse_roundtrip(response("plugin_list"));
    assert_eq!(plugins.plugins[0].status, PluginRuntimeStatus::Running);
    assert_eq!(
        plugins.plugins[0]
            .cli_command
            .as_ref()
            .map(|c| c.name.as_str()),
        Some("automation")
    );

    let ok: PluginRpcResponse = parse_roundtrip(response("plugin_rpc_ok"));
    assert_eq!(ok.into_result().unwrap(), json!({"items": [1, 2]}));
    let coded: PluginRpcResponse = parse_roundtrip(response("plugin_rpc_coded_error"));
    let error = coded.into_result().unwrap_err();
    assert_eq!(error.code(), Some("unknown_method"));
    assert_eq!(error.message(), "no such method");
    let plain: PluginRpcResponse = parse_roundtrip(response("plugin_rpc_string_error"));
    assert_eq!(
        plain.into_result().unwrap_err().message(),
        "unknown plugin \"x\""
    );

    let body = response("plugin_cli_stream");
    let result = PluginCliResult::parse_body(body.as_str().unwrap()).unwrap();
    assert_eq!(result.exit_code, 0);
    assert_eq!(result.stdout, "ok\n");
    assert_eq!(
        serde_json::to_value(PluginCliRequest {
            argv: vec!["list".into()],
            thread_id: Some("thr_demo".into()),
            ..PluginCliRequest::default()
        })
        .unwrap(),
        json!({"argv": ["list"], "threadId": "thr_demo"})
    );
}

#[test]
fn error_envelopes_expose_typed_details() {
    let error: ApiError = parse_roundtrip(response("error_thread_not_writable"));
    assert!(!error.is_retryable());
    match error.lifecycle_details() {
        Some(LifecycleErrorDetails::ThreadNotWritable(details)) => {
            assert_eq!(details.reason, ThreadNotWritableReason::Archived);
            assert_eq!(details.thread_status, ThreadStatus::Idle);
        }
        other => panic!("unexpected {other:?}"),
    }
    let moved: ApiError = parse_roundtrip(response("error_server_moved"));
    assert_eq!(
        moved.server_moved_details().map(|d| d.server_url),
        Some("http://10.0.0.2:38886".to_owned())
    );
    let plain: ApiError = parse_roundtrip(response("error_plain"));
    assert_eq!(plain.lifecycle_details(), None);
    assert_eq!(plain.to_string(), "not_found: Thread not found");
}

#[test]
fn system_health_version_and_config() {
    let health: HealthResponse = parse_roundtrip(response("health"));
    assert_eq!(
        health.server_move.map(|m| m.state),
        Some(ServerMoveHealthState::Ready)
    );
    let degraded: HealthResponse = parse(response("health_bad_move"));
    assert!(degraded.ok);
    assert_eq!(degraded.server_move, None);

    let version: SystemVersionResponse = parse_roundtrip(response("version"));
    assert_eq!(version.install_kind, Some(InstallKind::Npm));
    assert!(version.update_available);

    let attention: SystemAttentionResponse = parse_roundtrip(response("attention"));
    assert!(attention.has_attention);

    let config: SystemConfigResponse = parse(response("config"));
    assert_eq!(config.keybindings.len(), 1);
    assert!(config.keybindings[0].shortcut.primary_modifier);
    assert!(is_app_command_id(&config.keybindings[0].command));
    assert_eq!(config.default_keybindings[0].shortcut, None);
    assert_eq!(
        parse_plugin_command_id(&config.keybinding_overrides[0].command),
        Some(("automations", "open"))
    );
    assert_eq!(config.host_daemon_port, Some(38887));
    assert_eq!(parse_plugin_command_id("plugin:Bad/x"), None);
}
