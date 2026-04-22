mod runtime;
mod supervisor_tools;
mod worker_control;

use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

use anyhow::Context;
use anyhow::Result;
use clap::Args;
use clap::Parser;
use clap::Subcommand;
use codex_arg0::Arg0DispatchPaths;
use codex_arg0::arg0_dispatch_or_else;
use codex_tt_core::DefaultView;
use codex_tt_core::TtState;
use codex_tt_core::WorkerBinding;
use codex_tt_core::WorkerKind;
use codex_tt_core::WorkerRecord;
use codex_tt_core::WorkspacePaths;
use codex_tt_core::activate_tt_env;
use codex_tt_core::append_log;
use codex_tt_core::default_log_event;
use codex_tt_core::ensure_workspace_artifacts;
use codex_tt_core::load_state;
use codex_tt_core::save_state;
use codex_tui::AppExitInfo;
use codex_tui::Cli as TuiCli;
use codex_tui::ExitReason;
use codex_utils_cli::CliConfigOverrides;

use crate::runtime::ensure_runtime_worker_thread;
use crate::runtime::open_running_runtime;
use crate::runtime::reconcile_runtime_state;
use crate::runtime::run_daemon;
use crate::runtime::start_daemon;
use crate::runtime::stop_daemon;
use crate::worker_control::WorkerReadWindow;
use crate::worker_control::adopt_worker;
use crate::worker_control::format_worker_list_for_cli;
use crate::worker_control::format_worker_read_for_cli;
use crate::worker_control::list_worker_summaries;
use crate::worker_control::read_worker_history;
use crate::worker_control::remove_worker;
use crate::worker_control::send_worker_prompt;
use crate::worker_control::validate_worker_name;

#[derive(Debug, Parser)]
#[command(name = "tt", version, about = "TT orchestration CLI")]
struct Cli {
    #[command(subcommand)]
    command: CommandKind,
}

#[derive(Debug, Subcommand)]
enum CommandKind {
    Clone {
        repo_url: String,
        dir: Option<PathBuf>,
    },
    Start,
    Stop,
    Open,
    Status,
    Auto {
        #[arg(value_parser = ["on", "off"])]
        state: String,
    },
    Pause,
    Worker {
        #[command(subcommand)]
        command: WorkerCommand,
    },
    #[command(hide = true)]
    Daemon {
        #[arg(long)]
        workspace_root: PathBuf,
    },
}

#[derive(Debug, Subcommand)]
enum WorkerCommand {
    Add(WorkerAddArgs),
    List,
    Attach { name: String },
    Read(WorkerReadArgs),
    Send(WorkerSendArgs),
    Adopt(WorkerAdoptArgs),
    Remove { name: String },
}

#[derive(Debug, Args)]
struct WorkerAddArgs {
    name: String,
}

#[derive(Debug, Args)]
struct WorkerReadArgs {
    name: String,
    #[arg(long, conflicts_with = "all")]
    turns: Option<usize>,
    #[arg(long)]
    all: bool,
}

#[derive(Debug, Args)]
struct WorkerSendArgs {
    name: String,
    #[arg(long)]
    message: String,
}

#[derive(Debug, Args)]
struct WorkerAdoptArgs {
    name: String,
    #[arg(long = "thread-id")]
    thread_id: String,
}

fn workspace_paths_for_current_dir() -> Result<WorkspacePaths> {
    let cwd = std::env::current_dir().context("resolve current directory")?;
    WorkspacePaths::discover_from(&cwd)
        .with_context(|| format!("no TT workspace found from {}", cwd.display()))
}

fn status_output(paths: &WorkspacePaths, state: &TtState) -> String {
    let worker_lines = if state.workers.is_empty() {
        "workers: <none>\n".to_string()
    } else {
        let mut output = String::from("workers:\n");
        for worker in &state.workers {
            let thread_id = worker.thread_id.as_deref().unwrap_or("<missing>");
            output.push_str(&format!(
                "  - {} [{}] cwd={} thread_id={}\n",
                worker.name,
                worker.kind.as_str(),
                worker.cwd.display(),
                thread_id
            ));
        }
        output
    };

    format!(
        "workspace: {}\nprimary: {}\nview: {}\nruntime_running: {}\nruntime_pid: {}\nruntime_websocket_url: {}\nauto_loop: {}\noperator_pause: {}\nsupervisor_thread_id: {}\nactive_dispatch_id: {}\npending_director_evaluation: {}\npending_developer_dispatch: {}\n{}",
        paths.workspace_root().display(),
        paths.primary_checkout().display(),
        state.default_view,
        state.runtime_running,
        state
            .runtime_pid
            .map(|pid| pid.to_string())
            .unwrap_or_else(|| "<none>".to_string()),
        state.runtime_websocket_url.as_deref().unwrap_or("<none>"),
        state.auto_loop,
        state.operator_pause,
        state.supervisor_thread_id.as_deref().unwrap_or("<missing>"),
        state.active_dispatch_id.as_deref().unwrap_or("<none>"),
        state.pending_director_evaluation,
        state.pending_developer_dispatch,
        worker_lines
    )
}

fn default_view_target(state: &TtState) -> Result<(&str, &Path)> {
    match &state.default_view {
        DefaultView::Supervisor => Ok((
            state
                .supervisor_thread_id
                .as_deref()
                .context("missing supervisor thread id in TT state")?,
            Path::new("."),
        )),
        DefaultView::Worker { name } => {
            let worker = state
                .workers
                .iter()
                .find(|worker| worker.name == *name)
                .with_context(|| format!("missing TT worker `{name}` in state"))?;
            Ok((
                worker
                    .thread_id
                    .as_deref()
                    .with_context(|| format!("missing TT worker `{name}` thread id"))?,
                worker.cwd.as_path(),
            ))
        }
    }
}

async fn run_tt_tui(
    arg0_paths: Arg0DispatchPaths,
    workspace_paths: &WorkspacePaths,
    resume_thread_id: String,
    cwd: PathBuf,
) -> Result<()> {
    let state = load_state(workspace_paths)?;
    let websocket_url = state
        .runtime_websocket_url
        .clone()
        .context("missing TT runtime websocket url")?;

    let cli = TuiCli {
        prompt: None,
        images: Vec::new(),
        resume_picker: false,
        resume_last: false,
        resume_session_id: Some(resume_thread_id.clone()),
        resume_show_all: false,
        resume_include_non_interactive: false,
        fork_picker: false,
        fork_last: false,
        fork_session_id: None,
        fork_show_all: false,
        model: None,
        oss: false,
        oss_provider: None,
        config_profile: None,
        sandbox_mode: None,
        approval_policy: None,
        full_auto: false,
        dangerously_bypass_approvals_and_sandbox: false,
        cwd: Some(cwd),
        web_search: false,
        add_dir: Vec::new(),
        no_alt_screen: false,
        config_overrides: CliConfigOverrides::default(),
    };

    append_log(
        workspace_paths,
        default_log_event(
            "attached",
            None,
            None,
            Some(resume_thread_id),
            Some("operator attached via tt".to_string()),
        ),
    )?;

    let exit_info = codex_tui::run_main(
        cli,
        arg0_paths,
        codex_core::config_loader::LoaderOverrides::default(),
        Some(websocket_url),
        state.runtime_auth_token.clone(),
    )
    .await?;
    handle_tui_exit(exit_info)
}

fn handle_tui_exit(exit_info: AppExitInfo) -> Result<()> {
    match exit_info.exit_reason {
        ExitReason::UserRequested => Ok(()),
        ExitReason::Fatal(message) => anyhow::bail!(message),
    }
}

async fn require_running_state(workspace_paths: &WorkspacePaths) -> Result<TtState> {
    let state = reconcile_runtime_state(workspace_paths).await?;
    if state.runtime_running {
        Ok(state)
    } else {
        anyhow::bail!("TT runtime is not running; use `tt start` first")
    }
}

fn infer_workspace_dir(repo_url: &str) -> Result<PathBuf> {
    let last_segment = repo_url
        .rsplit(['/', ':'])
        .next()
        .filter(|segment| !segment.is_empty())
        .context("unable to infer workspace name from repo URL")?;
    let workspace_name = last_segment
        .strip_suffix(".git")
        .unwrap_or(last_segment)
        .trim();
    if workspace_name.is_empty() {
        anyhow::bail!("repo URL does not produce a usable workspace name");
    }
    Ok(PathBuf::from(workspace_name))
}

fn run_git(current_dir: &Path, args: &[&str]) -> Result<()> {
    let output = Command::new("git")
        .args(args)
        .current_dir(current_dir)
        .output()
        .with_context(|| format!("run git {}", args.join(" ")))?;
    if output.status.success() {
        Ok(())
    } else {
        anyhow::bail!(
            "git {} failed:\nstdout:\n{}\nstderr:\n{}",
            args.join(" "),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

async fn clone_workspace(repo_url: String, dir: Option<PathBuf>) -> Result<()> {
    let cwd = std::env::current_dir().context("resolve current directory")?;
    let workspace_dir = match dir {
        Some(dir) => dir,
        None => infer_workspace_dir(&repo_url)?,
    };
    let workspace_root = if workspace_dir.is_absolute() {
        workspace_dir
    } else {
        cwd.join(workspace_dir)
    };
    if workspace_root.exists() {
        anyhow::bail!(
            "workspace destination already exists: {}",
            workspace_root.display()
        );
    }

    std::fs::create_dir_all(&workspace_root)
        .with_context(|| format!("create workspace root {}", workspace_root.display()))?;
    let workspace_paths = WorkspacePaths::new(workspace_root.clone());
    std::fs::create_dir_all(workspace_paths.worktrees_dir()).with_context(|| {
        format!(
            "create workspace worktrees dir {}",
            workspace_paths.worktrees_dir().display()
        )
    })?;
    run_git(
        &workspace_root,
        &[
            "clone",
            repo_url.as_str(),
            workspace_paths
                .primary_checkout()
                .to_string_lossy()
                .as_ref(),
        ],
    )?;
    ensure_workspace_artifacts(&workspace_paths)?;
    activate_tt_env(&workspace_paths);
    append_log(
        &workspace_paths,
        default_log_event(
            "workspace-cloned",
            None,
            None,
            None,
            Some(format!(
                "repo_url={} primary={}",
                repo_url,
                workspace_paths.primary_checkout().display()
            )),
        ),
    )?;
    println!(
        "cloned TT workspace at {}",
        workspace_paths.workspace_root().display()
    );
    Ok(())
}

async fn add_worker(
    arg0_paths: &Arg0DispatchPaths,
    workspace_paths: &WorkspacePaths,
    name: String,
) -> Result<()> {
    validate_worker_name(&name)?;
    ensure_workspace_artifacts(workspace_paths)?;
    let mut state = load_state(workspace_paths)?;
    if state.workers.iter().any(|worker| worker.name == name) {
        anyhow::bail!("worker `{name}` already exists");
    }

    let worker_path = workspace_paths.worker_checkout(&name);
    if worker_path.exists() {
        anyhow::bail!(
            "worker checkout path already exists: {}",
            worker_path.display()
        );
    }

    run_git(
        &workspace_paths.primary_checkout(),
        &[
            "worktree",
            "add",
            "--detach",
            worker_path.to_string_lossy().as_ref(),
            "HEAD",
        ],
    )?;

    state.workers.push(WorkerRecord {
        name: name.clone(),
        kind: WorkerKind::Worker,
        cwd: worker_path.clone(),
        binding: WorkerBinding::Managed,
        thread_id: None,
        instruction_path: None,
    });
    save_state(workspace_paths, &state)?;

    if state.runtime_running {
        let mut state =
            ensure_runtime_worker_thread(workspace_paths, arg0_paths, name.clone()).await?;
        state.default_view = DefaultView::Worker { name: name.clone() };
        save_state(workspace_paths, &state)?;
    }

    append_log(
        workspace_paths,
        default_log_event(
            "worker-added",
            None,
            Some(name.clone()),
            None,
            Some(format!("cwd={}", worker_path.display())),
        ),
    )?;
    println!("added worker `{name}` at {}", worker_path.display());
    Ok(())
}

async fn list_workers(
    arg0_paths: &Arg0DispatchPaths,
    workspace_paths: &WorkspacePaths,
) -> Result<()> {
    let state = reconcile_runtime_state(workspace_paths).await?;
    let output = if state.runtime_running {
        let (mut runtime, state) = open_running_runtime(workspace_paths, arg0_paths).await?;
        let summaries = list_worker_summaries(Some(&mut runtime), &state).await?;
        format_worker_list_for_cli(&summaries)
    } else {
        let summaries = list_worker_summaries(None, &state).await?;
        format_worker_list_for_cli(&summaries)
    };
    print!("{output}");
    Ok(())
}

async fn attach_worker(
    arg0_paths: Arg0DispatchPaths,
    workspace_paths: &WorkspacePaths,
    name: String,
) -> Result<()> {
    let mut state = require_running_state(workspace_paths).await?;
    if state
        .workers
        .iter()
        .find(|worker| worker.name == name)
        .and_then(|worker| worker.thread_id.as_ref())
        .is_none()
    {
        state = ensure_runtime_worker_thread(workspace_paths, &arg0_paths, name.clone()).await?;
    }
    let worker = state
        .workers
        .iter()
        .find(|worker| worker.name == name)
        .with_context(|| format!("unknown worker `{name}`"))?;
    let thread_id = worker
        .thread_id
        .clone()
        .with_context(|| format!("missing worker `{name}` thread id"))?;
    state.default_view = DefaultView::Worker { name };
    save_state(workspace_paths, &state)?;
    run_tt_tui(arg0_paths, workspace_paths, thread_id, worker.cwd.clone()).await
}

async fn read_worker(
    arg0_paths: &Arg0DispatchPaths,
    workspace_paths: &WorkspacePaths,
    args: WorkerReadArgs,
) -> Result<()> {
    let window = match (args.all, args.turns) {
        (true, Some(_)) => anyhow::bail!("`--all` and `--turns` cannot both be set"),
        (true, None) => WorkerReadWindow::All,
        (false, Some(turns)) => WorkerReadWindow::Recent(turns),
        (false, None) => WorkerReadWindow::default(),
    };
    let (mut runtime, mut state) = open_running_runtime(workspace_paths, arg0_paths).await?;
    let result = read_worker_history(&mut runtime, &mut state, &args.name, window).await?;
    print!("{}", format_worker_read_for_cli(&result));
    Ok(())
}

async fn send_worker(
    arg0_paths: &Arg0DispatchPaths,
    workspace_paths: &WorkspacePaths,
    args: WorkerSendArgs,
) -> Result<()> {
    let (mut runtime, mut state) = open_running_runtime(workspace_paths, arg0_paths).await?;
    let result = send_worker_prompt(&mut runtime, &mut state, &args.name, args.message).await?;
    println!(
        "mode: {}\nturn_id: {}",
        result.mode.as_str(),
        result.turn_id
    );
    Ok(())
}

async fn adopt_worker_command(
    arg0_paths: &Arg0DispatchPaths,
    workspace_paths: &WorkspacePaths,
    args: WorkerAdoptArgs,
) -> Result<()> {
    let (mut runtime, mut state) = open_running_runtime(workspace_paths, arg0_paths).await?;
    let summary = adopt_worker(
        workspace_paths,
        &mut runtime,
        &mut state,
        &args.name,
        &args.thread_id,
    )
    .await?;
    println!(
        "adopted worker `{}` kind={} cwd={} thread_id={}",
        summary.name,
        summary.kind.as_str(),
        summary.cwd,
        summary.thread_id.as_deref().unwrap_or("<missing>")
    );
    Ok(())
}

fn remove_worker_command(workspace_paths: &WorkspacePaths, name: String) -> Result<()> {
    ensure_workspace_artifacts(workspace_paths)?;
    let mut state = load_state(workspace_paths)?;
    remove_worker(workspace_paths, &mut state, &name)?;
    println!("removed worker `{name}`");
    Ok(())
}

async fn run(arg0_paths: Arg0DispatchPaths) -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        CommandKind::Clone { repo_url, dir } => clone_workspace(repo_url, dir).await?,
        CommandKind::Daemon { workspace_root } => {
            run_daemon(workspace_root, arg0_paths).await?;
        }
        command => {
            let workspace_paths = workspace_paths_for_current_dir()?;
            activate_tt_env(&workspace_paths);
            match command {
                CommandKind::Start => {
                    let state = start_daemon(&workspace_paths, &arg0_paths).await?;
                    println!("{}", status_output(&workspace_paths, &state));
                }
                CommandKind::Stop => {
                    let state = stop_daemon(&workspace_paths).await?;
                    println!("{}", status_output(&workspace_paths, &state));
                }
                CommandKind::Open => {
                    let mut state = require_running_state(&workspace_paths).await?;
                    let (thread_id, cwd) = default_view_target(&state)?;
                    let resume_thread_id = thread_id.to_string();
                    let cwd = if cwd == Path::new(".") {
                        workspace_paths.workspace_root().to_path_buf()
                    } else {
                        cwd.to_path_buf()
                    };
                    if matches!(state.default_view, DefaultView::Supervisor) {
                        state.default_view = DefaultView::Supervisor;
                        save_state(&workspace_paths, &state)?;
                    }
                    run_tt_tui(arg0_paths, &workspace_paths, resume_thread_id, cwd).await?;
                }
                CommandKind::Status => {
                    if !workspace_paths.state_path().exists() {
                        println!(
                            "workspace: {}\nstate: uninitialized",
                            workspace_paths.workspace_root().display()
                        );
                    } else {
                        let state = reconcile_runtime_state(&workspace_paths).await?;
                        print!("{}", status_output(&workspace_paths, &state));
                    }
                }
                CommandKind::Auto { state: toggle } => {
                    ensure_workspace_artifacts(&workspace_paths)?;
                    let mut state = load_state(&workspace_paths)?;
                    state.auto_loop = toggle == "on";
                    save_state(&workspace_paths, &state)?;
                    append_log(
                        &workspace_paths,
                        default_log_event(
                            "auto-loop-changed",
                            None,
                            None,
                            None,
                            Some(format!("auto_loop={}", state.auto_loop)),
                        ),
                    )?;
                    println!("auto_loop={}", state.auto_loop);
                }
                CommandKind::Pause => {
                    ensure_workspace_artifacts(&workspace_paths)?;
                    let mut state = load_state(&workspace_paths)?;
                    state.operator_pause = !state.operator_pause;
                    save_state(&workspace_paths, &state)?;
                    append_log(
                        &workspace_paths,
                        default_log_event(
                            "operator-pause-changed",
                            None,
                            None,
                            None,
                            Some(format!("operator_pause={}", state.operator_pause)),
                        ),
                    )?;
                    println!("operator_pause={}", state.operator_pause);
                }
                CommandKind::Worker { command } => match command {
                    WorkerCommand::Add(args) => {
                        add_worker(&arg0_paths, &workspace_paths, args.name).await?;
                    }
                    WorkerCommand::List => {
                        list_workers(&arg0_paths, &workspace_paths).await?;
                    }
                    WorkerCommand::Attach { name } => {
                        attach_worker(arg0_paths, &workspace_paths, name).await?;
                    }
                    WorkerCommand::Read(args) => {
                        read_worker(&arg0_paths, &workspace_paths, args).await?;
                    }
                    WorkerCommand::Send(args) => {
                        send_worker(&arg0_paths, &workspace_paths, args).await?;
                    }
                    WorkerCommand::Adopt(args) => {
                        adopt_worker_command(&arg0_paths, &workspace_paths, args).await?;
                    }
                    WorkerCommand::Remove { name } => {
                        remove_worker_command(&workspace_paths, name)?;
                    }
                },
                CommandKind::Clone { .. } | CommandKind::Daemon { .. } => unreachable!(),
            }
        }
    }

    Ok(())
}

fn main() -> anyhow::Result<()> {
    arg0_dispatch_or_else(run)
}
