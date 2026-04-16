mod runtime;

use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;
use clap::Parser;
use clap::Subcommand;
use clap::ValueEnum;
use codex_arg0::Arg0DispatchPaths;
use codex_arg0::arg0_dispatch_or_else;
use codex_git_utils::get_git_repo_root;
use codex_tt_core::ProjectPaths;
use codex_tt_core::Role;
use codex_tt_core::TtState;
use codex_tt_core::activate_tt_env;
use codex_tt_core::append_log;
use codex_tt_core::default_log_event;
use codex_tt_core::ensure_project_artifacts;
use codex_tt_core::load_state;
use codex_tt_core::save_state;
use codex_tui::AppExitInfo;
use codex_tui::Cli as TuiCli;
use codex_tui::ExitReason;
use codex_utils_cli::CliConfigOverrides;

use crate::runtime::reconcile_runtime_state;
use crate::runtime::run_daemon;
use crate::runtime::start_daemon;
use crate::runtime::stop_daemon;

#[derive(Debug, Parser)]
#[command(name = "tt", version, about = "TT orchestration CLI")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Init,
    Start,
    Stop,
    Open,
    Status,
    Attach {
        role: RoleArg,
    },
    Auto {
        state: ToggleArg,
    },
    Pause,
    #[command(hide = true)]
    Daemon {
        #[arg(long)]
        repo_root: PathBuf,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum RoleArg {
    Director,
    Developer,
}

impl From<RoleArg> for Role {
    fn from(value: RoleArg) -> Self {
        match value {
            RoleArg::Director => Role::Director,
            RoleArg::Developer => Role::Developer,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum ToggleArg {
    On,
    Off,
}

fn project_paths_for_current_dir() -> Result<ProjectPaths> {
    let cwd = std::env::current_dir().context("resolve current directory")?;
    let repo_root = get_git_repo_root(&cwd).unwrap_or(cwd);
    Ok(ProjectPaths::new(repo_root))
}

fn role_thread_id(state: &TtState, role: Role) -> Option<&str> {
    match role {
        Role::Director => state.director_thread_id.as_deref(),
        Role::Developer => state.developer_thread_id.as_deref(),
    }
}

fn status_output(paths: &ProjectPaths, state: &TtState) -> String {
    format!(
        "repo: {}\nview: {}\nruntime_running: {}\nruntime_pid: {}\nruntime_websocket_url: {}\nauto_loop: {}\noperator_pause: {}\ndirector_thread_id: {}\ndeveloper_thread_id: {}\nactive_dispatch_id: {}\npending_director_evaluation: {}\npending_developer_dispatch: {}\n",
        paths.repo_root().display(),
        state.default_view,
        state.runtime_running,
        state
            .runtime_pid
            .map(|pid| pid.to_string())
            .unwrap_or_else(|| "<none>".to_string()),
        state.runtime_websocket_url.as_deref().unwrap_or("<none>"),
        state.auto_loop,
        state.operator_pause,
        state.director_thread_id.as_deref().unwrap_or("<missing>"),
        state.developer_thread_id.as_deref().unwrap_or("<missing>"),
        state.active_dispatch_id.as_deref().unwrap_or("<none>"),
        state.pending_director_evaluation,
        state.pending_developer_dispatch
    )
}

async fn run_tt_tui(
    arg0_paths: Arg0DispatchPaths,
    project_paths: &ProjectPaths,
    role: Role,
    state: &mut TtState,
) -> Result<()> {
    let thread_id = role_thread_id(state, role)
        .with_context(|| format!("missing {} thread id in TT state", role.as_str()))?
        .to_string();
    let websocket_url = state
        .runtime_websocket_url
        .clone()
        .context("missing TT runtime websocket url")?;

    let cli = TuiCli {
        prompt: None,
        images: Vec::new(),
        resume_picker: false,
        resume_last: false,
        resume_session_id: Some(thread_id.clone()),
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
        cwd: Some(project_paths.repo_root().to_path_buf()),
        web_search: false,
        add_dir: Vec::new(),
        no_alt_screen: false,
        config_overrides: CliConfigOverrides::default(),
    };

    state.default_view = role;
    save_state(project_paths, state)?;
    append_log(
        project_paths,
        default_log_event(
            "attached",
            Some(role),
            Some(thread_id),
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

async fn require_running_state(project_paths: &ProjectPaths) -> Result<TtState> {
    let state = reconcile_runtime_state(project_paths).await?;
    if state.runtime_running {
        Ok(state)
    } else {
        anyhow::bail!("TT runtime is not running; use `tt start` first")
    }
}

async fn run(arg0_paths: Arg0DispatchPaths) -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Daemon { repo_root } => {
            run_daemon(repo_root, arg0_paths).await?;
        }
        command => {
            let project_paths = project_paths_for_current_dir()?;
            activate_tt_env(&project_paths);
            match command {
                Command::Init => {
                    ensure_project_artifacts(&project_paths)?;
                    append_log(
                        &project_paths,
                        default_log_event(
                            "init",
                            None,
                            None,
                            Some("TT project initialized".to_string()),
                        ),
                    )?;
                    println!("initialized TT in {}", project_paths.tt_dir().display());
                }
                Command::Start => {
                    let state = start_daemon(&project_paths, &arg0_paths).await?;
                    println!("{}", status_output(&project_paths, &state));
                }
                Command::Stop => {
                    let state = stop_daemon(&project_paths).await?;
                    println!("{}", status_output(&project_paths, &state));
                }
                Command::Open => {
                    let mut state = require_running_state(&project_paths).await?;
                    run_tt_tui(arg0_paths, &project_paths, Role::Director, &mut state).await?;
                }
                Command::Status => {
                    if !project_paths.state_path().exists() {
                        println!(
                            "repo: {}\nstate: uninitialized",
                            project_paths.repo_root().display()
                        );
                    } else {
                        let state = reconcile_runtime_state(&project_paths).await?;
                        print!("{}", status_output(&project_paths, &state));
                    }
                }
                Command::Attach { role } => {
                    let mut state = require_running_state(&project_paths).await?;
                    run_tt_tui(arg0_paths, &project_paths, role.into(), &mut state).await?;
                }
                Command::Auto { state: toggle } => {
                    ensure_project_artifacts(&project_paths)?;
                    let mut state = load_state(&project_paths)?;
                    state.auto_loop = matches!(toggle, ToggleArg::On);
                    save_state(&project_paths, &state)?;
                    append_log(
                        &project_paths,
                        default_log_event(
                            "auto-loop-changed",
                            None,
                            None,
                            Some(format!("auto_loop={}", state.auto_loop)),
                        ),
                    )?;
                    println!("auto_loop={}", state.auto_loop);
                }
                Command::Pause => {
                    ensure_project_artifacts(&project_paths)?;
                    let mut state = load_state(&project_paths)?;
                    state.operator_pause = !state.operator_pause;
                    save_state(&project_paths, &state)?;
                    append_log(
                        &project_paths,
                        default_log_event(
                            "operator-pause-changed",
                            None,
                            None,
                            Some(format!("operator_pause={}", state.operator_pause)),
                        ),
                    )?;
                    println!("operator_pause={}", state.operator_pause);
                }
                Command::Daemon { .. } => unreachable!(),
            }
        }
    }

    Ok(())
}

fn main() -> anyhow::Result<()> {
    arg0_dispatch_or_else(run)
}
