use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use anyhow::Context;
use anyhow::Result;
use chrono::Utc;
use codex_app_server_client::AppServerClient;
use codex_app_server_client::AppServerEvent;
use codex_app_server_client::DEFAULT_IN_PROCESS_CHANNEL_CAPACITY;
use codex_app_server_client::RemoteAppServerClient;
use codex_app_server_client::RemoteAppServerConnectArgs;
use codex_app_server_client::TypedRequestError;
use codex_app_server_protocol::ApprovalsReviewer;
use codex_app_server_protocol::ClientRequest;
use codex_app_server_protocol::JSONRPCErrorError;
use codex_app_server_protocol::RequestId;
use codex_app_server_protocol::ServerNotification;
use codex_app_server_protocol::ThreadReadParams;
use codex_app_server_protocol::ThreadReadResponse;
use codex_app_server_protocol::ThreadResumeParams;
use codex_app_server_protocol::ThreadResumeResponse;
use codex_app_server_protocol::ThreadSetNameParams;
use codex_app_server_protocol::ThreadSetNameResponse;
use codex_app_server_protocol::ThreadStartParams;
use codex_app_server_protocol::ThreadStartResponse;
use codex_app_server_protocol::TurnCompletedNotification;
use codex_app_server_protocol::TurnStartParams;
use codex_app_server_protocol::TurnStartResponse;
use codex_app_server_protocol::TurnStatus;
use codex_app_server_protocol::TurnSteerParams;
use codex_app_server_protocol::TurnSteerResponse;
use codex_app_server_protocol::UserInput;
use codex_arg0::Arg0DispatchPaths;
use codex_core::config::Config;
use codex_core::config::ConfigBuilder;
use codex_core::config::ConfigOverrides;
use codex_protocol::protocol::AskForApproval;
use codex_tt_core::DispatchEnvelope;
use codex_tt_core::ResultEnvelope;
use codex_tt_core::Role;
use codex_tt_core::TtState;
use codex_tt_core::WorkerKind;
use codex_tt_core::WorkerRecord;
use codex_tt_core::WorkspacePaths;
use codex_tt_core::activate_tt_env;
use codex_tt_core::append_log;
use codex_tt_core::default_log_event;
use codex_tt_core::ensure_workspace_artifacts;
use codex_tt_core::load_state;
use codex_tt_core::parse_dispatch_envelope;
use codex_tt_core::parse_result_envelope;
use codex_tt_core::save_state;
use tokio::io::AsyncBufReadExt;
use tokio::io::BufReader;
use tokio::process::Child;
use tokio::process::Command;
use tokio::time::Instant;
use tokio::time::sleep;

use crate::supervisor_tools;

const LOOP_POLL_INTERVAL: Duration = Duration::from_secs(1);
const START_TIMEOUT: Duration = Duration::from_secs(15);
const STOP_TIMEOUT: Duration = Duration::from_secs(10);
const APP_SERVER_LISTEN_URL: &str = "ws://127.0.0.1:0";

#[derive(Clone)]
struct RequestIdSequencer {
    next: i64,
}

impl RequestIdSequencer {
    fn new() -> Self {
        Self { next: 1 }
    }

    fn next(&mut self) -> RequestId {
        let value = self.next;
        self.next += 1;
        RequestId::Integer(value)
    }
}

pub(crate) struct TtRuntime {
    pub(crate) workspace_paths: WorkspacePaths,
    arg0_paths: Arg0DispatchPaths,
    client: AppServerClient,
    request_ids: RequestIdSequencer,
}

impl TtRuntime {
    pub(crate) async fn open_remote(
        workspace_root: PathBuf,
        arg0_paths: Arg0DispatchPaths,
        websocket_url: String,
        auth_token: Option<String>,
    ) -> Result<Self> {
        let workspace_paths = WorkspacePaths::new(workspace_root);
        ensure_workspace_artifacts(&workspace_paths)?;
        let client = RemoteAppServerClient::connect(RemoteAppServerConnectArgs {
            websocket_url,
            auth_token,
            client_name: "tt-cli".to_string(),
            client_version: env!("CARGO_PKG_VERSION").to_string(),
            experimental_api: true,
            opt_out_notification_methods: Vec::new(),
            channel_capacity: DEFAULT_IN_PROCESS_CHANNEL_CAPACITY,
        })
        .await
        .context("connect TT remote app server")?;

        Ok(Self {
            workspace_paths,
            arg0_paths,
            client: AppServerClient::Remote(client),
            request_ids: RequestIdSequencer::new(),
        })
    }

    pub(crate) async fn ensure_sessions(&mut self, state: &mut TtState) -> Result<()> {
        state.supervisor_thread_id = Some(
            self.ensure_supervisor_thread(state.supervisor_thread_id.clone())
                .await?,
        );

        let worker_names: Vec<String> = state
            .workers
            .iter()
            .map(|worker| worker.name.clone())
            .collect();
        for worker_name in worker_names {
            self.ensure_named_worker_session(state, worker_name).await?;
        }
        Ok(())
    }

    async fn ensure_supervisor_thread(
        &mut self,
        existing_thread_id: Option<String>,
    ) -> Result<String> {
        if let Some(thread_id) = existing_thread_id
            && self.resume_thread(&thread_id).await.is_ok()
        {
            return Ok(thread_id);
        }

        let instructions =
            std::fs::read_to_string(self.workspace_paths.role_path(Role::Supervisor))
                .context("read supervisor instructions")?;
        let supervisor_cwd = self.workspace_paths.workspace_root().to_path_buf();
        let response = self
            .start_thread(
                &supervisor_cwd,
                "tt-supervisor",
                "TT Supervisor",
                Some(instructions),
                Some(supervisor_tools::supervisor_dynamic_tools()),
            )
            .await
            .context("start supervisor thread")?;
        let thread_id = response.thread.id;

        append_log(
            &self.workspace_paths,
            default_log_event(
                "thread-created",
                Some(Role::Supervisor),
                None,
                Some(thread_id.clone()),
                Some("created supervisor session".to_string()),
            ),
        )?;

        Ok(thread_id)
    }

    pub(crate) async fn ensure_named_worker_session(
        &mut self,
        state: &mut TtState,
        worker_name: String,
    ) -> Result<String> {
        let worker_index = state
            .workers
            .iter()
            .position(|worker| worker.name == worker_name)
            .with_context(|| format!("missing worker `{worker_name}` in TT state"))?;

        if let Some(thread_id) = state.workers[worker_index].thread_id.as_deref()
            && self.resume_thread(thread_id).await.is_ok()
        {
            return Ok(thread_id.to_string());
        }

        let worker = state.workers[worker_index].clone();
        let instructions = read_optional_instructions(worker.instruction_path.as_deref())?;
        let response = self
            .start_thread(
                &worker.cwd,
                service_name_for_worker(&worker),
                &thread_name_for_worker(&worker),
                instructions,
                None,
            )
            .await
            .with_context(|| format!("start worker `{}` thread", worker.name))?;

        let thread_id = response.thread.id;
        state.workers[worker_index].thread_id = Some(thread_id.clone());

        append_log(
            &self.workspace_paths,
            default_log_event(
                "thread-created",
                worker.kind.preset_role(),
                Some(worker.name.clone()),
                Some(thread_id.clone()),
                Some(format!("created {} session", worker.kind.as_str())),
            ),
        )?;

        Ok(thread_id)
    }

    async fn start_thread(
        &mut self,
        cwd: &Path,
        service_name: &str,
        thread_name: &str,
        developer_instructions: Option<String>,
        dynamic_tools: Option<Vec<codex_app_server_protocol::DynamicToolSpec>>,
    ) -> Result<ThreadStartResponse> {
        let config = load_config(
            self.workspace_paths.workspace_root().to_path_buf(),
            cwd.to_path_buf(),
            &self.arg0_paths,
        )
        .await?;
        let response: ThreadStartResponse = self
            .client
            .request_typed(ClientRequest::ThreadStart {
                request_id: self.request_ids.next(),
                params: ThreadStartParams {
                    model: config.model.clone(),
                    model_provider: Some(config.model_provider_id.clone()),
                    service_tier: config.service_tier.map(Some),
                    cwd: Some(cwd.to_string_lossy().to_string()),
                    approval_policy: Some(AskForApproval::Never.into()),
                    approvals_reviewer: Some(ApprovalsReviewer::User),
                    sandbox: Some(codex_app_server_protocol::SandboxMode::WorkspaceWrite),
                    config: config_request_overrides_from_config(&config),
                    service_name: Some(service_name.to_string()),
                    base_instructions: None,
                    developer_instructions,
                    personality: config.personality,
                    ephemeral: Some(false),
                    session_start_source: None,
                    dynamic_tools,
                    mock_experimental_field: None,
                    experimental_raw_events: false,
                    persist_extended_history: true,
                },
            })
            .await?;

        let _: ThreadSetNameResponse = self
            .client
            .request_typed(ClientRequest::ThreadSetName {
                request_id: self.request_ids.next(),
                params: ThreadSetNameParams {
                    thread_id: response.thread.id.clone(),
                    name: thread_name.to_string(),
                },
            })
            .await
            .with_context(|| format!("name thread `{thread_name}`"))?;

        Ok(response)
    }

    async fn resume_thread(&mut self, thread_id: &str) -> Result<ThreadResumeResponse> {
        self.client
            .request_typed(ClientRequest::ThreadResume {
                request_id: self.request_ids.next(),
                params: ThreadResumeParams {
                    thread_id: thread_id.to_string(),
                    history: None,
                    path: None,
                    model: None,
                    model_provider: None,
                    service_tier: None,
                    cwd: None,
                    approval_policy: None,
                    approvals_reviewer: None,
                    sandbox: None,
                    config: None,
                    base_instructions: None,
                    developer_instructions: None,
                    personality: None,
                    persist_extended_history: true,
                },
            })
            .await
            .with_context(|| format!("resume TT thread {thread_id}"))
    }

    pub(crate) async fn read_thread(
        &mut self,
        thread_id: &str,
        include_turns: bool,
    ) -> Result<ThreadReadResponse> {
        self.client
            .request_typed(ClientRequest::ThreadRead {
                request_id: self.request_ids.next(),
                params: ThreadReadParams {
                    thread_id: thread_id.to_string(),
                    include_turns,
                },
            })
            .await
            .with_context(|| format!("read TT thread history for {thread_id}"))
    }

    pub(crate) async fn start_turn(
        &mut self,
        thread_id: &str,
        text: String,
    ) -> std::result::Result<TurnStartResponse, TypedRequestError> {
        self.client
            .request_typed(ClientRequest::TurnStart {
                request_id: self.request_ids.next(),
                params: TurnStartParams {
                    thread_id: thread_id.to_string(),
                    input: vec![UserInput::Text {
                        text,
                        text_elements: Vec::new(),
                    }],
                    ..Default::default()
                },
            })
            .await
    }

    pub(crate) async fn steer_turn(
        &mut self,
        thread_id: &str,
        expected_turn_id: &str,
        text: String,
    ) -> std::result::Result<TurnSteerResponse, TypedRequestError> {
        self.client
            .request_typed(ClientRequest::TurnSteer {
                request_id: self.request_ids.next(),
                params: TurnSteerParams {
                    thread_id: thread_id.to_string(),
                    input: vec![UserInput::Text {
                        text,
                        text_elements: Vec::new(),
                    }],
                    responsesapi_client_metadata: None,
                    expected_turn_id: expected_turn_id.to_string(),
                },
            })
            .await
    }

    async fn send_turn(&mut self, thread_id: &str, text: String) -> Result<String> {
        let response = self
            .start_turn(thread_id, text)
            .await
            .with_context(|| format!("start turn for {thread_id}"))?;
        Ok(response.turn.id)
    }

    async fn latest_agent_message(&mut self, thread_id: &str) -> Result<Option<String>> {
        let response = self.read_thread(thread_id, true).await?;
        Ok(response.thread.turns.into_iter().rev().find_map(|turn| {
            turn.items.into_iter().rev().find_map(|item| match item {
                codex_app_server_protocol::ThreadItem::AgentMessage { text, .. } => Some(text),
                _ => None,
            })
        }))
    }

    async fn next_event(&mut self) -> Option<AppServerEvent> {
        self.client.next_event().await
    }

    pub(crate) async fn resolve_server_request(
        &self,
        request_id: RequestId,
        result: serde_json::Value,
    ) -> Result<()> {
        self.client
            .resolve_server_request(request_id, result)
            .await
            .context("resolve TT server request")
    }

    async fn reject_server_request(&self, request_id: RequestId) -> Result<()> {
        self.reject_server_request_with_message(
            request_id,
            "tt daemon cannot satisfy interactive server requests".to_string(),
        )
        .await
    }

    pub(crate) async fn reject_server_request_with_message(
        &self,
        request_id: RequestId,
        message: String,
    ) -> Result<()> {
        self.client
            .reject_server_request(
                request_id,
                JSONRPCErrorError {
                    code: -32000,
                    data: None,
                    message,
                },
            )
            .await
            .context("reject TT server request")
    }
}

pub(crate) async fn open_running_runtime(
    workspace_paths: &WorkspacePaths,
    arg0_paths: &Arg0DispatchPaths,
) -> Result<(TtRuntime, TtState)> {
    let state = reconcile_runtime_state(workspace_paths).await?;
    if !state.runtime_running {
        anyhow::bail!("TT runtime is not running; use `tt start` first");
    }
    let websocket_url = state
        .runtime_websocket_url
        .clone()
        .context("missing TT runtime websocket url")?;
    let runtime = TtRuntime::open_remote(
        workspace_paths.workspace_root().to_path_buf(),
        arg0_paths.clone(),
        websocket_url,
        state.runtime_auth_token.clone(),
    )
    .await?;
    Ok((runtime, state))
}

pub(crate) async fn ensure_runtime_worker_thread(
    workspace_paths: &WorkspacePaths,
    arg0_paths: &Arg0DispatchPaths,
    worker_name: String,
) -> Result<TtState> {
    let mut state = reconcile_runtime_state(workspace_paths).await?;
    if !state.runtime_running {
        anyhow::bail!("TT runtime is not running; use `tt start` first");
    }
    let websocket_url = state
        .runtime_websocket_url
        .clone()
        .context("missing TT runtime websocket url")?;
    let mut runtime = TtRuntime::open_remote(
        workspace_paths.workspace_root().to_path_buf(),
        arg0_paths.clone(),
        websocket_url,
        state.runtime_auth_token.clone(),
    )
    .await?;
    runtime
        .ensure_named_worker_session(&mut state, worker_name)
        .await?;
    save_state(workspace_paths, &state)?;
    Ok(state)
}

pub(crate) async fn start_daemon(
    workspace_paths: &WorkspacePaths,
    arg0_paths: &Arg0DispatchPaths,
) -> Result<TtState> {
    ensure_workspace_artifacts(workspace_paths)?;
    let state = reconcile_runtime_state(workspace_paths).await?;
    if state.runtime_running {
        return Ok(state);
    }

    let current_exe = arg0_paths
        .codex_self_exe
        .clone()
        .or_else(|| std::env::current_exe().ok())
        .context("resolve tt executable path")?;
    let daemon_log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(workspace_paths.daemon_log_path())
        .context("open TT daemon log")?;
    let daemon_log_err = daemon_log
        .try_clone()
        .context("clone TT daemon log handle")?;

    let mut command = std::process::Command::new(current_exe);
    command
        .arg("daemon")
        .arg("--workspace-root")
        .arg(workspace_paths.workspace_root())
        .stdin(Stdio::null())
        .stdout(Stdio::from(daemon_log))
        .stderr(Stdio::from(daemon_log_err));
    let _child = command.spawn().context("spawn TT daemon")?;

    let deadline = Instant::now() + START_TIMEOUT;
    loop {
        let state = load_state(workspace_paths).context("load TT state during daemon startup")?;
        let all_workers_ready = state
            .workers
            .iter()
            .all(|worker| worker.thread_id.is_some());
        if state.runtime_running
            && state.runtime_websocket_url.is_some()
            && state.supervisor_thread_id.is_some()
            && all_workers_ready
        {
            return Ok(state);
        }
        if Instant::now() >= deadline {
            anyhow::bail!("timed out waiting for TT daemon startup");
        }
        sleep(Duration::from_millis(200)).await;
    }
}

pub(crate) async fn stop_daemon(workspace_paths: &WorkspacePaths) -> Result<TtState> {
    let mut state = reconcile_runtime_state(workspace_paths).await?;
    if !state.runtime_running {
        return Ok(state);
    }
    let pid = state.runtime_pid.context("missing TT runtime pid")?;
    terminate_process(pid)?;

    let deadline = Instant::now() + STOP_TIMEOUT;
    while Instant::now() < deadline {
        state = load_state(workspace_paths).context("load TT state during daemon stop")?;
        if !state.runtime_running {
            return Ok(state);
        }
        sleep(Duration::from_millis(200)).await;
    }

    clear_runtime_fields(&mut state);
    save_state(workspace_paths, &state)?;
    Ok(state)
}

pub(crate) async fn reconcile_runtime_state(workspace_paths: &WorkspacePaths) -> Result<TtState> {
    ensure_workspace_artifacts(workspace_paths)?;
    let mut state = load_state(workspace_paths)?;
    let runtime_reachable = if state.runtime_running {
        match state.runtime_websocket_url.as_deref() {
            Some(websocket_url) => {
                can_connect_remote(websocket_url, state.runtime_auth_token.clone()).await
            }
            None => false,
        }
    } else {
        false
    };
    if state.runtime_running && !runtime_reachable {
        clear_runtime_fields(&mut state);
        save_state(workspace_paths, &state)?;
    }
    Ok(state)
}

pub(crate) async fn run_daemon(
    workspace_root: PathBuf,
    arg0_paths: Arg0DispatchPaths,
) -> Result<()> {
    let workspace_paths = WorkspacePaths::new(workspace_root.clone());
    ensure_workspace_artifacts(&workspace_paths)?;
    activate_tt_env(&workspace_paths);
    let mut state = load_state(&workspace_paths)?;
    let mut app_server = spawn_app_server(&workspace_paths, &arg0_paths).await?;

    state.runtime_running = true;
    state.runtime_pid = Some(std::process::id());
    state.runtime_websocket_url = Some(app_server.websocket_url.clone());
    state.runtime_auth_token = None;
    state.last_runtime_started_at = Some(Utc::now());
    save_state(&workspace_paths, &state)?;
    append_log(
        &workspace_paths,
        default_log_event(
            "runtime-started",
            None,
            None,
            None,
            Some(format!("runtime websocket={}", app_server.websocket_url)),
        ),
    )?;

    let mut runtime = TtRuntime::open_remote(
        workspace_root,
        arg0_paths,
        app_server.websocket_url.clone(),
        None,
    )
    .await?;
    runtime.ensure_sessions(&mut state).await?;
    save_state(&runtime.workspace_paths, &state)?;

    let signal = shutdown_signal();
    tokio::pin!(signal);

    loop {
        tokio::select! {
            _ = &mut signal => {
                break;
            }
            event = runtime.next_event() => {
                let Some(event) = event else {
                    append_log(
                        &runtime.workspace_paths,
                        default_log_event(
                            "runtime-disconnected",
                            None,
                            None,
                            None,
                            Some("TT app server client disconnected".to_string()),
                        ),
                    )?;
                    break;
                };
                handle_app_server_event(&mut runtime, &mut state, event).await?;
            }
            _ = sleep(LOOP_POLL_INTERVAL) => {
                state = refresh_operator_state(&runtime.workspace_paths, state)?;
                if let Some(status) = app_server.child.try_wait().context("poll TT app server child")? {
                    append_log(
                        &runtime.workspace_paths,
                        default_log_event(
                            "runtime-app-server-exited",
                            None,
                            None,
                            None,
                            Some(format!("app server exited with {status}")),
                        ),
                    )?;
                    break;
                }
                advance_if_needed(&mut runtime, &mut state).await?;
            }
        }
    }

    let _ = app_server.child.start_kill();
    state = refresh_operator_state(&runtime.workspace_paths, state)?;
    clear_runtime_fields(&mut state);
    save_state(&runtime.workspace_paths, &state)?;
    append_log(
        &runtime.workspace_paths,
        default_log_event(
            "runtime-stopped",
            None,
            None,
            None,
            Some("TT runtime stopped".to_string()),
        ),
    )?;
    Ok(())
}

fn refresh_operator_state(workspace_paths: &WorkspacePaths, mut state: TtState) -> Result<TtState> {
    let on_disk = load_state(workspace_paths)?;
    state.auto_loop = on_disk.auto_loop;
    state.operator_pause = on_disk.operator_pause;
    state.default_view = on_disk.default_view;
    state.workers = on_disk.workers;
    state.supervisor_thread_id = on_disk.supervisor_thread_id;
    Ok(state)
}

async fn handle_app_server_event(
    runtime: &mut TtRuntime,
    state: &mut TtState,
    event: AppServerEvent,
) -> Result<()> {
    match event {
        AppServerEvent::ServerNotification(ServerNotification::TurnCompleted(notification)) => {
            handle_turn_completed(runtime, state, notification).await?;
        }
        AppServerEvent::ServerRequest(request) => {
            if !supervisor_tools::handle_server_request(runtime, state, request.clone()).await? {
                runtime.reject_server_request(request.id().clone()).await?;
            }
        }
        AppServerEvent::Disconnected { message } => {
            append_log(
                &runtime.workspace_paths,
                default_log_event("runtime-disconnected", None, None, None, Some(message)),
            )?;
        }
        AppServerEvent::Lagged { skipped } => {
            append_log(
                &runtime.workspace_paths,
                default_log_event(
                    "runtime-event-lagged",
                    None,
                    None,
                    None,
                    Some(format!("skipped={skipped}")),
                ),
            )?;
        }
        _ => {}
    }
    advance_if_needed(runtime, state).await
}

async fn handle_turn_completed(
    runtime: &mut TtRuntime,
    state: &mut TtState,
    notification: TurnCompletedNotification,
) -> Result<()> {
    if !matches!(
        notification.turn.status,
        TurnStatus::Completed | TurnStatus::Interrupted | TurnStatus::Failed
    ) {
        return Ok(());
    }
    let Some(role) = preset_role_for_thread(state, &notification.thread_id) else {
        return Ok(());
    };
    let Some(message) = runtime
        .latest_agent_message(&notification.thread_id)
        .await?
    else {
        return Ok(());
    };

    match role {
        Role::Director => {
            if !message.contains("[DISPATCH]") {
                return Ok(());
            }
            match parse_dispatch_envelope(&message) {
                Ok(dispatch) => {
                    state.active_dispatch_id = Some(dispatch.dispatch_id.clone());
                    state.pending_developer_dispatch = true;
                    state.pending_director_evaluation = false;
                    save_state(&runtime.workspace_paths, state)?;
                    append_log(
                        &runtime.workspace_paths,
                        default_log_event(
                            "dispatch-received",
                            Some(Role::Director),
                            Some("director".to_string()),
                            Some(notification.thread_id),
                            Some(format!("dispatch_id={}", dispatch.dispatch_id)),
                        ),
                    )?;
                }
                Err(err) => {
                    append_log(
                        &runtime.workspace_paths,
                        default_log_event(
                            "loop-blocked",
                            Some(Role::Director),
                            Some("director".to_string()),
                            Some(notification.thread_id),
                            Some(format!("dispatch parse failed: {err}")),
                        ),
                    )?;
                }
            }
        }
        Role::Developer => {
            if !message.contains("[RESULT]") {
                return Ok(());
            }
            match parse_result_envelope(&message) {
                Ok(result) => {
                    if let Some(active_dispatch_id) = state.active_dispatch_id.as_deref()
                        && active_dispatch_id != result.dispatch_id
                    {
                        append_log(
                            &runtime.workspace_paths,
                            default_log_event(
                                "loop-blocked",
                                Some(Role::Developer),
                                Some("developer".to_string()),
                                Some(notification.thread_id),
                                Some(format!(
                                    "result dispatch mismatch: active={active_dispatch_id} result={}",
                                    result.dispatch_id
                                )),
                            ),
                        )?;
                        return Ok(());
                    }
                    state.pending_director_evaluation = true;
                    state.pending_developer_dispatch = false;
                    save_state(&runtime.workspace_paths, state)?;
                    append_log(
                        &runtime.workspace_paths,
                        default_log_event(
                            "result-received",
                            Some(Role::Developer),
                            Some("developer".to_string()),
                            Some(notification.thread_id),
                            Some(format!("dispatch_id={}", result.dispatch_id)),
                        ),
                    )?;
                }
                Err(err) => {
                    append_log(
                        &runtime.workspace_paths,
                        default_log_event(
                            "loop-blocked",
                            Some(Role::Developer),
                            Some("developer".to_string()),
                            Some(notification.thread_id),
                            Some(format!("result parse failed: {err}")),
                        ),
                    )?;
                }
            }
        }
        Role::Supervisor => {}
    }

    Ok(())
}

async fn advance_if_needed(runtime: &mut TtRuntime, state: &mut TtState) -> Result<()> {
    if !state.auto_loop || state.operator_pause {
        return Ok(());
    }
    if state.pending_developer_dispatch {
        let dispatch = latest_dispatch(runtime, state).await?;
        let developer_thread_id = worker_thread_id(state, WorkerKind::Developer)
            .context("missing TT developer thread id")?;
        let prompt = developer_prompt(&dispatch);
        let turn_id = runtime.send_turn(&developer_thread_id, prompt).await?;
        state.pending_developer_dispatch = false;
        save_state(&runtime.workspace_paths, state)?;
        append_log(
            &runtime.workspace_paths,
            default_log_event(
                "dispatch-sent",
                Some(Role::Developer),
                Some("developer".to_string()),
                Some(developer_thread_id),
                Some(format!(
                    "dispatch_id={} turn_id={turn_id}",
                    dispatch.dispatch_id
                )),
            ),
        )?;
    } else if state.pending_director_evaluation {
        let result = latest_result(runtime, state).await?;
        let director_thread_id = worker_thread_id(state, WorkerKind::Director)
            .context("missing TT director thread id")?;
        let prompt = director_prompt(&result);
        let turn_id = runtime.send_turn(&director_thread_id, prompt).await?;
        state.pending_director_evaluation = false;
        save_state(&runtime.workspace_paths, state)?;
        append_log(
            &runtime.workspace_paths,
            default_log_event(
                "loop-advanced",
                Some(Role::Director),
                Some("director".to_string()),
                Some(director_thread_id),
                Some(format!(
                    "dispatch_id={} turn_id={turn_id}",
                    result.dispatch_id
                )),
            ),
        )?;
    }
    Ok(())
}

async fn latest_dispatch(runtime: &mut TtRuntime, state: &TtState) -> Result<DispatchEnvelope> {
    let director_thread_id =
        worker_thread_id(state, WorkerKind::Director).context("missing TT director thread id")?;
    let message = runtime
        .latest_agent_message(&director_thread_id)
        .await?
        .context("missing TT director dispatch message")?;
    parse_dispatch_envelope(&message)
}

async fn latest_result(runtime: &mut TtRuntime, state: &TtState) -> Result<ResultEnvelope> {
    let developer_thread_id =
        worker_thread_id(state, WorkerKind::Developer).context("missing TT developer thread id")?;
    let message = runtime
        .latest_agent_message(&developer_thread_id)
        .await?
        .context("missing TT developer result message")?;
    parse_result_envelope(&message)
}

fn preset_role_for_thread(state: &TtState, thread_id: &str) -> Option<Role> {
    if state.supervisor_thread_id.as_deref() == Some(thread_id) {
        return Some(Role::Supervisor);
    }

    state.workers.iter().find_map(|worker| {
        (worker.thread_id.as_deref() == Some(thread_id))
            .then_some(worker.kind.preset_role())
            .flatten()
    })
}

fn worker_thread_id(state: &TtState, kind: WorkerKind) -> Option<String> {
    state
        .workers
        .iter()
        .find(|worker| worker.kind == kind)
        .and_then(|worker| worker.thread_id.clone())
}

fn developer_prompt(dispatch: &DispatchEnvelope) -> String {
    format!(
        "TT runtime delivered the following Director dispatch. Execute within scope and respond only with a [RESULT] envelope.\n\n[DISPATCH]\nDispatch-ID: {}\nObjective: {}\nScope: {}\nConstraints: {}\nExpected-Output: {}\nCompletion-Criteria: {}\nPriority: {}\nPlan-Refs: {}\nTodo-Refs: {}\n[/DISPATCH]",
        dispatch.dispatch_id,
        dispatch.objective,
        dispatch.scope,
        dispatch.constraints,
        dispatch.expected_output,
        dispatch.completion_criteria,
        dispatch.priority,
        dispatch.plan_refs,
        dispatch.todo_refs
    )
}

fn director_prompt(result: &ResultEnvelope) -> String {
    format!(
        "TT runtime delivered the following Developer result. Evaluate it against `.tt/plan.md` and emit the next [DISPATCH] envelope if work should continue.\n\n[RESULT]\nDispatch-ID: {}\nStatus: {}\nSummary: {}\nWork-Completed: {}\nArtifacts: {}\nOpen-Questions: {}\nRecommended-Next-Step: {}\n[/RESULT]",
        result.dispatch_id,
        result.status,
        result.summary,
        result.work_completed,
        result.artifacts,
        result.open_questions,
        result.recommended_next_step
    )
}

async fn load_config(
    workspace_root: PathBuf,
    cwd: PathBuf,
    arg0_paths: &Arg0DispatchPaths,
) -> Result<Config> {
    let workspace_paths = WorkspacePaths::new(workspace_root);
    ConfigBuilder::default()
        .codex_home(workspace_paths.codex_home())
        .harness_overrides(ConfigOverrides {
            cwd: Some(cwd),
            codex_self_exe: arg0_paths.codex_self_exe.clone(),
            codex_linux_sandbox_exe: arg0_paths.codex_linux_sandbox_exe.clone(),
            main_execve_wrapper_exe: arg0_paths.main_execve_wrapper_exe.clone(),
            ..Default::default()
        })
        .build()
        .await
        .context("load TT config")
}

fn config_request_overrides_from_config(
    config: &Config,
) -> Option<HashMap<String, serde_json::Value>> {
    config.active_profile.as_ref().map(|profile| {
        HashMap::from([(
            "profile".to_string(),
            serde_json::Value::String(profile.clone()),
        )])
    })
}

fn read_optional_instructions(path: Option<&Path>) -> Result<Option<String>> {
    let Some(path) = path else {
        return Ok(None);
    };
    Ok(Some(std::fs::read_to_string(path).with_context(|| {
        format!("read worker instructions {}", path.display())
    })?))
}

fn service_name_for_worker(worker: &WorkerRecord) -> &str {
    match worker.kind {
        WorkerKind::Director => "tt-director",
        WorkerKind::Developer => "tt-developer",
        WorkerKind::Worker => "tt-worker",
    }
}

fn thread_name_for_worker(worker: &WorkerRecord) -> String {
    match worker.kind {
        WorkerKind::Director => "TT Director".to_string(),
        WorkerKind::Developer => "TT Developer".to_string(),
        WorkerKind::Worker => format!("TT Worker {}", worker.name),
    }
}

struct SpawnedAppServer {
    child: Child,
    websocket_url: String,
}

async fn spawn_app_server(
    workspace_paths: &WorkspacePaths,
    arg0_paths: &Arg0DispatchPaths,
) -> Result<SpawnedAppServer> {
    let program = resolve_codex_app_server_binary(arg0_paths);
    let mut command = Command::new(program);
    command
        .arg("--listen")
        .arg(APP_SERVER_LISTEN_URL)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .current_dir(workspace_paths.workspace_root())
        .env("CODEX_HOME", workspace_paths.codex_home())
        .env("TT_HOME", workspace_paths.tt_dir())
        .env("TT_REPO_ROOT", workspace_paths.workspace_root());
    let mut child = command.spawn().context("spawn codex-app-server for TT")?;
    let stderr = child
        .stderr
        .take()
        .context("capture TT app server stderr")?;
    let mut stderr_reader = BufReader::new(stderr).lines();
    let deadline = Instant::now() + START_TIMEOUT;
    let websocket_url = loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let line = tokio::time::timeout(remaining, stderr_reader.next_line())
            .await
            .context("timed out waiting for TT app server websocket address")?
            .context("read TT app server stderr")?
            .context("TT app server exited before announcing websocket address")?;
        let stripped_line = strip_ansi(&line);
        if let Some(websocket_url) = stripped_line
            .split_whitespace()
            .find(|token| token.starts_with("ws://"))
        {
            break websocket_url.to_string();
        }
    };

    tokio::spawn(async move {
        while let Ok(Some(line)) = stderr_reader.next_line().await {
            eprintln!("[codex-app-server] {line}");
        }
    });

    Ok(SpawnedAppServer {
        child,
        websocket_url,
    })
}

fn resolve_codex_app_server_binary(arg0_paths: &Arg0DispatchPaths) -> PathBuf {
    if let Some(current_exe) = arg0_paths.codex_self_exe.as_ref() {
        let file_name =
            if let Some(extension) = current_exe.extension().and_then(|ext| ext.to_str()) {
                format!("codex-app-server.{extension}")
            } else {
                "codex-app-server".to_string()
            };
        let sibling = current_exe.with_file_name(file_name);
        if sibling.exists() {
            return sibling;
        }
    }
    PathBuf::from("codex-app-server")
}

fn strip_ansi(line: &str) -> String {
    let mut stripped = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' && matches!(chars.peek(), Some(&'[')) {
            chars.next();
            for next in chars.by_ref() {
                if ('@'..='~').contains(&next) {
                    break;
                }
            }
            continue;
        }
        stripped.push(ch);
    }
    stripped
}

async fn can_connect_remote(websocket_url: &str, auth_token: Option<String>) -> bool {
    match RemoteAppServerClient::connect(RemoteAppServerConnectArgs {
        websocket_url: websocket_url.to_string(),
        auth_token,
        client_name: "tt-status".to_string(),
        client_version: env!("CARGO_PKG_VERSION").to_string(),
        experimental_api: true,
        opt_out_notification_methods: Vec::new(),
        channel_capacity: DEFAULT_IN_PROCESS_CHANNEL_CAPACITY,
    })
    .await
    {
        Ok(client) => {
            let _ = client.shutdown().await;
            true
        }
        Err(_) => false,
    }
}

fn clear_runtime_fields(state: &mut TtState) {
    state.runtime_running = false;
    state.runtime_pid = None;
    state.runtime_websocket_url = None;
    state.runtime_auth_token = None;
}

#[cfg(unix)]
fn terminate_process(pid: u32) -> Result<()> {
    let status = std::process::Command::new("kill")
        .args(["-TERM", &pid.to_string()])
        .status()
        .context("run kill for TT daemon")?;
    if status.success() {
        Ok(())
    } else {
        anyhow::bail!("kill failed for TT daemon pid {pid}");
    }
}

#[cfg(windows)]
fn terminate_process(pid: u32) -> Result<()> {
    let status = std::process::Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .status()
        .context("run taskkill for TT daemon")?;
    if status.success() {
        Ok(())
    } else {
        anyhow::bail!("taskkill failed for TT daemon pid {pid}");
    }
}

#[cfg(unix)]
async fn shutdown_signal() {
    if let Ok(mut terminate) =
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
    {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = terminate.recv() => {}
        }
    } else {
        let _ = tokio::signal::ctrl_c().await;
    }
}

#[cfg(not(unix))]
async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}
