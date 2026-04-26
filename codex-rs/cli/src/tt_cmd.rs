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
use codex_tt_core::discover_worktrees;
use codex_tt_core::init_project;
use codex_tt_core::read_runtime_registration;

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
}

#[derive(Debug, Args)]
struct TtInitArgs {
    /// TT project root. Defaults to the parent of the current git repo.
    #[arg(long)]
    project_root: Option<PathBuf>,

    /// Primary repo path relative to the TT project root.
    #[arg(long)]
    primary_repo: Option<PathBuf>,

    /// Worktrees directory relative to the TT project root.
    #[arg(long, default_value = DEFAULT_WORKTREES_DIR)]
    worktrees_dir: PathBuf,
}

pub(crate) fn run_tt_command(cli: TtCli) -> Result<()> {
    match cli.command {
        TtSubcommand::Init(args) => run_init(args),
        TtSubcommand::Status => run_status(),
    }
}

fn run_init(args: TtInitArgs) -> Result<()> {
    let defaults = infer_init_defaults()?;
    let project_root = args.project_root.unwrap_or(defaults.project_root);
    let primary_repo = match args.primary_repo {
        Some(primary_repo) => primary_repo,
        None => relative_path(&project_root, &defaults.primary_repo).with_context(|| {
            format!(
                "infer primary repo path relative to {}",
                project_root.display()
            )
        })?,
    };

    let summary = init_project(InitOptions {
        project_root,
        primary_repo,
        worktrees_dir: args.worktrees_dir,
    })?;

    println!("TT project initialized");
    println!("root: {}", summary.project.root().display());
    println!("primary: {}", summary.project.primary_repo().display());
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

fn run_status() -> Result<()> {
    let cwd = std::env::current_dir().context("resolve current directory")?;
    let Some(project) = TtProject::discover_from(&cwd)? else {
        anyhow::bail!("no TT project discovered from {}", cwd.display());
    };

    println!("root: {}", project.root().display());
    println!("primary: {}", project.primary_repo().display());
    println!("worktrees: {}", project.worktrees_dir().display());
    println!("config: {}", project.config_path().display());
    match discover_worktrees(&project) {
        Ok(worktrees) if worktrees.is_empty() => {
            println!("git_worktrees: <none>");
        }
        Ok(worktrees) => {
            println!("git_worktrees:");
            for worktree in worktrees {
                let role = if worktree.is_primary {
                    "primary"
                } else {
                    "worktree"
                };
                let branch = worktree.branch.as_deref().unwrap_or("<detached>");
                let head = worktree.head.as_deref().unwrap_or("<unknown>");
                println!("  - {role} {branch} {head} {}", worktree.path.display());
            }
        }
        Err(err) => {
            println!("git_worktrees: unavailable: {err:#}");
        }
    }
    match read_runtime_registration(&project)? {
        Some(registration) => {
            println!("runtime: {}", registration.endpoint);
            println!("runtime_pid: {}", registration.pid);
        }
        None => {
            println!("runtime: <not registered>");
        }
    }
    Ok(())
}

struct InitDefaults {
    project_root: PathBuf,
    primary_repo: PathBuf,
}

fn infer_init_defaults() -> Result<InitDefaults> {
    let cwd = std::env::current_dir().context("resolve current directory")?;
    if let Some(git_root) = git_root_for(&cwd)? {
        let project_root = git_root
            .parent()
            .map(Path::to_path_buf)
            .with_context(|| format!("git root has no parent: {}", git_root.display()))?;
        return Ok(InitDefaults {
            project_root,
            primary_repo: git_root,
        });
    }

    Ok(InitDefaults {
        project_root: cwd,
        primary_repo: PathBuf::from("primary"),
    })
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

fn relative_path(root: &Path, path: &Path) -> Result<PathBuf> {
    let root = canonical_or_logical(root)?;
    let path = canonical_or_logical(path)?;
    Ok(path
        .strip_prefix(&root)
        .with_context(|| format!("{} is not inside {}", path.display(), root.display()))?
        .to_path_buf())
}

fn canonical_or_logical(path: &Path) -> Result<PathBuf> {
    if path.exists() {
        return std::fs::canonicalize(path)
            .with_context(|| format!("canonicalize {}", path.display()));
    }
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    Ok(std::env::current_dir()
        .context("resolve current directory")?
        .join(path))
}
