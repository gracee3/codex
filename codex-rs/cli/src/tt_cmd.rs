use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

use anyhow::Context;
use anyhow::Result;
use clap::Args;
use clap::Parser;
use clap::Subcommand;
use codex_tt_core::DEFAULT_WORKTREES_DIR;
use codex_tt_core::InitOptions;
use codex_tt_core::TtProject;
use codex_tt_core::TtThreadRecord;
use codex_tt_core::TtThreadRole;
use codex_tt_core::WorktreeInfo;
use codex_tt_core::discover_repositories;
use codex_tt_core::discover_worktrees_for_repositories;
use codex_tt_core::init_project;
use codex_tt_core::load_thread_registry;
use codex_tt_core::read_runtime_registration;
use codex_tt_core::thread_label_for_cwd;
use codex_tt_core::thread_name_for_cwd;
use codex_tt_core::upsert_thread_record;

#[derive(Debug, Parser)]
#[command(bin_name = "codex tt")]
pub(crate) struct TtCli {
    #[command(subcommand)]
    command: TtSubcommand,
}

#[derive(Debug, Subcommand)]
enum TtSubcommand {
    /// Initialize a TT project layout.
    Init(TtInitArgs),

    /// Show the discovered TT project state.
    Status,

    /// List or register TT thread records.
    Thread {
        #[command(subcommand)]
        command: TtThreadCommand,
    },
}

#[derive(Debug, Args)]
struct TtInitArgs {
    /// TT project root. Defaults to the parent of the current git repo.
    #[arg(long)]
    project_root: Option<PathBuf>,

    /// Worktrees directory relative to the TT project root.
    #[arg(long, default_value = DEFAULT_WORKTREES_DIR)]
    worktrees_dir: PathBuf,
}

#[derive(Debug, Subcommand)]
enum TtThreadCommand {
    /// List registered TT threads.
    List,

    /// Register or update a TT thread record.
    Register(TtThreadRegisterArgs),
}

#[derive(Debug, Args)]
struct TtThreadRegisterArgs {
    /// Thread role in the TT project.
    #[arg(long)]
    role: TtThreadRole,

    /// App-server thread id.
    #[arg(long)]
    thread_id: String,

    /// Optional display name.
    #[arg(long)]
    name: Option<String>,

    /// Thread working directory. Defaults to the current directory.
    #[arg(long)]
    cwd: Option<PathBuf>,

    /// Process id for the Codex process that owns this registration.
    #[arg(long)]
    pid: Option<u32>,
}

pub(crate) fn run_tt_command(cli: TtCli) -> Result<()> {
    match cli.command {
        TtSubcommand::Init(args) => run_init(args),
        TtSubcommand::Status => run_status(),
        TtSubcommand::Thread { command } => run_thread_command(command),
    }
}

fn run_init(args: TtInitArgs) -> Result<()> {
    let defaults = infer_init_defaults()?;
    let project_root = args.project_root.unwrap_or(defaults.project_root);

    let summary = init_project(InitOptions {
        project_root,
        worktrees_dir: args.worktrees_dir,
    })?;

    println!("TT project initialized");
    println!("root: {}", summary.project.root().display());
    println!("worktrees: {}", summary.project.worktrees_dir().display());
    if summary.created_paths.is_empty() {
        println!("created: <none>");
    } else {
        println!("created:");
        for path in summary.created_paths {
            println!("  {}", path.display());
        }
    }
    Ok(())
}

fn run_thread_command(command: TtThreadCommand) -> Result<()> {
    match command {
        TtThreadCommand::List => run_thread_list(),
        TtThreadCommand::Register(args) => run_thread_register(args),
    }
}

fn run_thread_list() -> Result<()> {
    let project = discover_project_from_current_dir()?;
    let registry = load_thread_registry(&project)?;
    if registry.threads.is_empty() {
        println!("threads: <none>");
        return Ok(());
    }
    let repos = discover_repositories(&project).unwrap_or_default();
    let worktrees = discover_worktrees_for_repositories(&repos).unwrap_or_default();

    println!("threads:");
    for thread in registry.threads {
        let name = thread.name.as_deref().unwrap_or("<unnamed>");
        let location = thread_label_for_cwd(&project, &thread.cwd, &worktrees);
        let pid = thread
            .pid
            .map(|pid| pid.to_string())
            .unwrap_or_else(|| "<none>".to_string());
        println!(
            "  - {} {} activation={} location={} name={} pid={} cwd={}",
            thread.role.as_str(),
            thread.thread_id,
            thread.activation.as_str(),
            location,
            name,
            pid,
            thread.cwd.display()
        );
    }
    Ok(())
}

fn run_thread_register(args: TtThreadRegisterArgs) -> Result<()> {
    let project = discover_project_from_current_dir()?;
    let cwd = match args.cwd {
        Some(cwd) => cwd,
        None => std::env::current_dir().context("resolve current directory")?,
    };
    let repos = discover_repositories(&project).unwrap_or_default();
    let worktrees = discover_worktrees_for_repositories(&repos).unwrap_or_default();
    let name = args
        .name
        .or_else(|| Some(thread_name_for_cwd(&project, args.role, &cwd, &worktrees)));
    let record = TtThreadRecord::new(
        args.role,
        args.thread_id,
        name,
        cwd,
        args.pid.or_else(|| Some(std::process::id())),
    );
    upsert_thread_record(&project, record)?;
    println!("thread registered");
    Ok(())
}

fn run_status() -> Result<()> {
    let project = discover_project_from_current_dir()?;

    println!("root: {}", project.root().display());
    println!("worktrees: {}", project.worktrees_dir().display());
    println!("config: {}", project.config_path().display());
    let repos = discover_repositories(&project)?;
    if repos.is_empty() {
        println!("repos: <none>");
    } else {
        println!("repos:");
        for repo in &repos {
            let branch = repo.branch.as_deref().unwrap_or("<detached>");
            let head = repo.head.as_deref().unwrap_or("<unknown>");
            println!("  - {} {branch} {head} {}", repo.name, repo.path.display());
        }
    }
    let discovered_worktrees = match discover_worktrees_for_repositories(&repos) {
        Ok(worktrees) if worktrees.is_empty() => {
            println!("git_worktrees: <none>");
            Vec::new()
        }
        Ok(worktrees) => {
            println!("git_worktrees:");
            for worktree in &worktrees {
                let role = if worktree.is_repo_root {
                    "repo"
                } else {
                    "worktree"
                };
                let branch = worktree.branch.as_deref().unwrap_or("<detached>");
                let head = worktree.head.as_deref().unwrap_or("<unknown>");
                println!(
                    "  - {} {role} {branch} {head} {}",
                    worktree.repo_name,
                    worktree.path.display()
                );
            }
            worktrees
        }
        Err(err) => {
            println!("git_worktrees: unavailable: {err:#}");
            Vec::new()
        }
    };
    match read_runtime_registration(&project)? {
        Some(registration) => {
            println!("runtime: {}", registration.endpoint);
            println!("runtime_pid: {}", registration.pid);
        }
        None => {
            println!("runtime: <not registered>");
        }
    }
    print_thread_registry_summary(&project, &discovered_worktrees)?;
    Ok(())
}

fn discover_project_from_current_dir() -> Result<TtProject> {
    let cwd = std::env::current_dir().context("resolve current directory")?;
    let Some(project) = TtProject::discover_from(&cwd)? else {
        anyhow::bail!("no TT project discovered from {}", cwd.display());
    };
    Ok(project)
}

fn print_thread_registry_summary(project: &TtProject, worktrees: &[WorktreeInfo]) -> Result<()> {
    let registry = load_thread_registry(project)?;
    if registry.threads.is_empty() {
        println!("threads: <none>");
    } else {
        println!("threads:");
        for thread in registry.threads {
            let name = thread.name.as_deref().unwrap_or("<unnamed>");
            let location = thread_label_for_cwd(project, &thread.cwd, worktrees);
            let pid = thread
                .pid
                .map(|pid| pid.to_string())
                .unwrap_or_else(|| "<none>".to_string());
            println!(
                "  - {} {} activation={} location={} name={} pid={} cwd={}",
                thread.role.as_str(),
                thread.thread_id,
                thread.activation.as_str(),
                location,
                name,
                pid,
                thread.cwd.display()
            );
        }
    }
    Ok(())
}

struct InitDefaults {
    project_root: PathBuf,
}

fn infer_init_defaults() -> Result<InitDefaults> {
    let cwd = std::env::current_dir().context("resolve current directory")?;
    if let Some(git_root) = git_root_for(&cwd)? {
        let project_root = git_root
            .parent()
            .map(Path::to_path_buf)
            .with_context(|| format!("git root has no parent: {}", git_root.display()))?;
        return Ok(InitDefaults { project_root });
    }

    Ok(InitDefaults { project_root: cwd })
}

fn git_root_for(cwd: &Path) -> Result<Option<PathBuf>> {
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(cwd)
        .output()
        .context("run git rev-parse --show-toplevel")?;
    if !output.status.success() {
        return Ok(None);
    }
    let stdout = String::from_utf8(output.stdout).context("parse git root as utf-8")?;
    let root = stdout.trim();
    if root.is_empty() {
        return Ok(None);
    }
    Ok(Some(PathBuf::from(root)))
}
