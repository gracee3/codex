use std::path::Path;
use std::path::PathBuf;

use crate::tt_cmd::TtAckArgs;
use crate::tt_cmd::TtAssignArgs;
use crate::tt_runtime::maybe_resolve_tt_app_server;
use anyhow::Context;
use anyhow::Result;
use codex_app_server_client::RemoteAppServerClient;
use codex_app_server_client::RemoteAppServerConnectArgs;
use codex_app_server_protocol::ClientRequest;
use codex_app_server_protocol::RequestId;
use codex_app_server_protocol::TurnStartParams;
use codex_app_server_protocol::TurnStartResponse;
use codex_app_server_protocol::UserInput;
use codex_arg0::Arg0DispatchPaths;
use codex_tt_core::TtProject;
use codex_tt_core::TtThreadActivation;
use codex_tt_core::TtThreadRecord;
use codex_tt_core::TtThreadRole;
use codex_tt_core::WorktreeInfo;
use codex_tt_core::discover_repositories;
use codex_tt_core::discover_worktrees_for_repositories;
use codex_tt_core::load_thread_registry;
use codex_tt_core::thread_label_for_cwd;
use codex_tt_core::thread_record_is_alive;

pub(crate) async fn run_ack(args: TtAckArgs, arg0_paths: &Arg0DispatchPaths) -> Result<()> {
    let cwd = std::env::current_dir().context("resolve current directory")?;
    let project = discover_project_from(&cwd)?;
    let registry = load_thread_registry(&project)?;
    let worker = resolve_current_thread(
        &registry.threads,
        &cwd,
        args.from_thread_id.as_deref(),
        Some(TtThreadRole::Worker),
    )?;
    let supervisor = resolve_supervisor_thread(&registry.threads, Some(worker.thread_id.as_str()))?;
    let note = args.note.join(" ");
    let note = (!note.trim().is_empty()).then_some(note.trim());
    let repos = discover_repositories(&project).unwrap_or_default();
    let worktrees = discover_worktrees_for_repositories(&repos).unwrap_or_default();
    let body = worker_ack_body(&project, &worktrees, worker, note);

    submit_turn(
        &cwd,
        arg0_paths,
        &supervisor.thread_id,
        supervisor.cwd.clone(),
        body,
    )
    .await?;
    println!(
        "ack sent to {} ({})",
        thread_display_name(supervisor),
        thread_label_for_cwd(&project, &supervisor.cwd, &worktrees)
    );
    Ok(())
}

pub(crate) async fn run_assign(args: TtAssignArgs, arg0_paths: &Arg0DispatchPaths) -> Result<()> {
    let cwd = std::env::current_dir().context("resolve current directory")?;
    let project = discover_project_from(&cwd)?;
    let registry = load_thread_registry(&project)?;
    let supervisor = resolve_current_thread(
        &registry.threads,
        &cwd,
        args.from_thread_id.as_deref(),
        Some(TtThreadRole::Supervisor),
    )?;
    let repos = discover_repositories(&project).unwrap_or_default();
    let worktrees = discover_worktrees_for_repositories(&repos).unwrap_or_default();
    let worker = resolve_worker_ref(&project, &worktrees, &registry.threads, &args.worker)?;
    let prompt = args.prompt.join(" ");
    let body = assignment_body(&project, &worktrees, supervisor, worker, &prompt);

    submit_turn(
        &cwd,
        arg0_paths,
        &worker.thread_id,
        worker.cwd.clone(),
        body,
    )
    .await?;
    println!(
        "assignment sent to {} ({})",
        thread_display_name(worker),
        thread_label_for_cwd(&project, &worker.cwd, &worktrees)
    );
    Ok(())
}

pub(crate) fn discover_project_from(cwd: &Path) -> Result<TtProject> {
    let Some(project) = TtProject::discover_from(cwd)? else {
        anyhow::bail!("no TT project discovered from {}", cwd.display());
    };
    Ok(project)
}

pub(crate) fn sort_threads_for_roster(threads: &mut [TtThreadRecord]) {
    threads.sort_by(|left, right| {
        thread_roster_sort_key(left)
            .cmp(&thread_roster_sort_key(right))
            .then_with(|| left.name.cmp(&right.name))
            .then_with(|| left.thread_id.cmp(&right.thread_id))
    });
}

fn resolve_current_thread<'a>(
    threads: &'a [TtThreadRecord],
    cwd: &Path,
    explicit_thread_id: Option<&str>,
    required_role: Option<TtThreadRole>,
) -> Result<&'a TtThreadRecord> {
    let thread = if let Some(thread_id) = explicit_thread_id {
        threads
            .iter()
            .filter(|thread| thread_record_is_alive(thread))
            .find(|thread| thread.thread_id == thread_id)
            .with_context(|| format!("unknown TT thread {thread_id}"))?
    } else {
        best_thread_for_cwd(threads, cwd)
            .with_context(|| format!("no registered TT thread found for cwd {}", cwd.display()))?
    };
    if let Some(role) = required_role
        && thread.role != role
    {
        anyhow::bail!(
            "TT thread {} is {}, expected {}",
            thread.thread_id,
            thread.role.as_str(),
            role.as_str()
        );
    }
    Ok(thread)
}

fn resolve_supervisor_thread<'a>(
    threads: &'a [TtThreadRecord],
    exclude_thread_id: Option<&str>,
) -> Result<&'a TtThreadRecord> {
    let mut supervisors = threads
        .iter()
        .filter(|thread| thread_record_is_alive(thread))
        .filter(|thread| {
            thread.role == TtThreadRole::Supervisor
                && Some(thread.thread_id.as_str()) != exclude_thread_id
        })
        .collect::<Vec<_>>();
    supervisors.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then_with(|| left.thread_id.cmp(&right.thread_id))
    });
    match supervisors.as_slice() {
        [] => anyhow::bail!("no TT supervisor thread is registered"),
        [supervisor] => Ok(supervisor),
        _ => anyhow::bail!("multiple TT supervisor threads are registered"),
    }
}

fn resolve_worker_ref<'a>(
    project: &TtProject,
    worktrees: &[WorktreeInfo],
    threads: &'a [TtThreadRecord],
    worker_ref: &str,
) -> Result<&'a TtThreadRecord> {
    let mut matches = threads
        .iter()
        .filter(|thread| thread_record_is_alive(thread))
        .filter(|thread| thread.role == TtThreadRole::Worker)
        .filter(|thread| {
            thread.thread_id == worker_ref
                || thread.thread_id.starts_with(worker_ref)
                || thread.name.as_deref() == Some(worker_ref)
                || thread_label_for_cwd(project, &thread.cwd, worktrees) == worker_ref
        })
        .collect::<Vec<_>>();
    matches.sort_by(|left, right| left.thread_id.cmp(&right.thread_id));
    match matches.as_slice() {
        [] => anyhow::bail!("no TT worker matches `{worker_ref}`"),
        [worker] => Ok(worker),
        _ => anyhow::bail!("multiple TT workers match `{worker_ref}`; use a longer thread id"),
    }
}

fn best_thread_for_cwd<'a>(
    threads: &'a [TtThreadRecord],
    cwd: &Path,
) -> Option<&'a TtThreadRecord> {
    let cwd = normalize_existing_path(cwd);
    threads
        .iter()
        .filter(|thread| thread_record_is_alive(thread))
        .filter_map(|thread| {
            let thread_cwd = normalize_existing_path(&thread.cwd);
            cwd.starts_with(&thread_cwd)
                .then_some((thread_cwd.components().count(), thread))
        })
        .max_by_key(|(components, _thread)| *components)
        .map(|(_components, thread)| thread)
}

async fn submit_turn(
    cwd: &Path,
    arg0_paths: &Arg0DispatchPaths,
    thread_id: &str,
    turn_cwd: PathBuf,
    body: String,
) -> Result<()> {
    let connection = maybe_resolve_tt_app_server(cwd, arg0_paths)
        .await
        .context("resolve TT app-server")?
        .with_context(|| format!("no TT app-server discovered from {}", cwd.display()))?;
    let client = RemoteAppServerClient::connect(RemoteAppServerConnectArgs {
        websocket_url: connection.websocket_url,
        auth_token: None,
        client_name: "codex-tt-cli".to_string(),
        client_version: env!("CARGO_PKG_VERSION").to_string(),
        experimental_api: true,
        opt_out_notification_methods: Vec::new(),
        channel_capacity: 8,
    })
    .await
    .context("connect to TT app-server")?;
    let request_id = RequestId::Integer(1);
    let _: TurnStartResponse = client
        .request_typed(ClientRequest::TurnStart {
            request_id,
            params: TurnStartParams {
                thread_id: thread_id.to_string(),
                input: vec![UserInput::Text {
                    text: body,
                    text_elements: Vec::new(),
                }],
                cwd: Some(turn_cwd),
                ..TurnStartParams::default()
            },
        })
        .await
        .context("submit TT turn")?;
    client
        .shutdown()
        .await
        .context("close TT app-server client")?;
    Ok(())
}

fn worker_ack_body(
    project: &TtProject,
    worktrees: &[WorktreeInfo],
    worker: &TtThreadRecord,
    note: Option<&str>,
) -> String {
    let location = thread_label_for_cwd(project, &worker.cwd, worktrees);
    match note.map(str::trim).filter(|note| !note.is_empty()) {
        Some(note) => format!("TT from {location}:\n{note}"),
        None => format!("TT from {location}: available"),
    }
}

fn assignment_body(
    project: &TtProject,
    worktrees: &[WorktreeInfo],
    supervisor: &TtThreadRecord,
    _worker: &TtThreadRecord,
    prompt: &str,
) -> String {
    let location = thread_label_for_cwd(project, &supervisor.cwd, worktrees);
    format!("TT assignment from {location}:\n{}", prompt.trim())
}

fn thread_display_name(thread: &TtThreadRecord) -> String {
    thread
        .name
        .clone()
        .unwrap_or_else(|| thread.thread_id.clone())
}

fn thread_roster_sort_key(thread: &TtThreadRecord) -> (u8, u8) {
    let role = match thread.role {
        TtThreadRole::Supervisor => 0,
        TtThreadRole::Worker => 1,
    };
    let activation = match thread.activation {
        TtThreadActivation::Reporting => 0,
        TtThreadActivation::Idle => 1,
    };
    (role, activation)
}

fn normalize_existing_path(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}
