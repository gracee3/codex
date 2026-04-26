use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use anyhow::Context;
use anyhow::Result;
use serde::Deserialize;
use serde::Serialize;

pub const TT_DIR: &str = ".tt";
pub const CODEX_DIR: &str = ".codex";
pub const CONFIG_FILE: &str = "config.toml";
pub const RUNTIME_DIR: &str = "runtime";
pub const RUNTIME_FILE: &str = "app-server.json";
pub const DEFAULT_WORKTREES_DIR: &str = "worktrees";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TtProject {
    root: PathBuf,
    config: TtConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TtConfig {
    pub primary_repo: PathBuf,
    pub worktrees_dir: PathBuf,
}

impl TtConfig {
    pub fn new(primary_repo: PathBuf, worktrees_dir: PathBuf) -> Self {
        Self {
            primary_repo,
            worktrees_dir,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitOptions {
    pub project_root: PathBuf,
    pub primary_repo: PathBuf,
    pub worktrees_dir: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitSummary {
    pub project: TtProject,
    pub created_paths: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeRegistration {
    pub version: u32,
    pub pid: u32,
    pub endpoint: String,
    pub started_at_unix_secs: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeInfo {
    pub path: PathBuf,
    pub branch: Option<String>,
    pub head: Option<String>,
    pub is_primary: bool,
}

impl RuntimeRegistration {
    pub fn new(pid: u32, endpoint: String) -> Self {
        Self {
            version: 1,
            pid,
            endpoint,
            started_at_unix_secs: unix_timestamp_secs(),
        }
    }
}

impl TtProject {
    pub fn new(root: PathBuf, config: TtConfig) -> Self {
        Self { root, config }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn config(&self) -> &TtConfig {
        &self.config
    }

    pub fn tt_dir(&self) -> PathBuf {
        self.root.join(TT_DIR)
    }

    pub fn codex_home(&self) -> PathBuf {
        self.root.join(CODEX_DIR)
    }

    pub fn config_path(&self) -> PathBuf {
        self.tt_dir().join(CONFIG_FILE)
    }

    pub fn primary_repo(&self) -> PathBuf {
        self.root.join(&self.config.primary_repo)
    }

    pub fn worktrees_dir(&self) -> PathBuf {
        self.root.join(&self.config.worktrees_dir)
    }

    pub fn runtime_dir(&self) -> PathBuf {
        self.tt_dir().join(RUNTIME_DIR)
    }

    pub fn runtime_registration_path(&self) -> PathBuf {
        self.runtime_dir().join(RUNTIME_FILE)
    }

    pub fn discover_from(start: &Path) -> Result<Option<Self>> {
        for ancestor in start.ancestors() {
            let tt_dir = ancestor.join(TT_DIR);
            if !tt_dir.is_dir() {
                continue;
            }
            return read_project_at(ancestor).map(Some);
        }
        Ok(None)
    }
}

pub fn init_project(options: InitOptions) -> Result<InitSummary> {
    let project_root = absolute_logical_path(&options.project_root)
        .with_context(|| format!("resolve project root {}", options.project_root.display()))?;
    let primary_repo = normalize_relative_path(&options.primary_repo)
        .context("primary repo path must be relative to the TT project root")?;
    let worktrees_dir = normalize_relative_path(&options.worktrees_dir)
        .context("worktrees dir must be relative to the TT project root")?;
    let config = TtConfig::new(primary_repo, worktrees_dir);
    let project = TtProject::new(project_root, config);

    let mut created_paths = Vec::new();
    create_dir_if_missing(&project.tt_dir(), &mut created_paths)?;
    create_dir_if_missing(&project.codex_home(), &mut created_paths)?;
    create_dir_if_missing(&project.primary_repo(), &mut created_paths)?;
    create_dir_if_missing(&project.worktrees_dir(), &mut created_paths)?;
    create_dir_if_missing(&project.runtime_dir(), &mut created_paths)?;

    let config_path = project.config_path();
    if !config_path.exists() {
        write_config(&config_path, project.config())?;
        created_paths.push(config_path);
    }

    Ok(InitSummary {
        project,
        created_paths,
    })
}

pub fn read_runtime_registration(project: &TtProject) -> Result<Option<RuntimeRegistration>> {
    let path = project.runtime_registration_path();
    if !path.exists() {
        return Ok(None);
    }
    let contents = fs::read_to_string(&path)
        .with_context(|| format!("read TT runtime registration {}", path.display()))?;
    let registration = serde_json::from_str(&contents)
        .with_context(|| format!("parse TT runtime registration {}", path.display()))?;
    Ok(Some(registration))
}

pub fn write_runtime_registration(
    project: &TtProject,
    registration: &RuntimeRegistration,
) -> Result<()> {
    fs::create_dir_all(project.runtime_dir())
        .with_context(|| format!("create TT runtime dir {}", project.runtime_dir().display()))?;
    let contents =
        serde_json::to_string_pretty(registration).context("serialize TT runtime registration")?;
    fs::write(project.runtime_registration_path(), format!("{contents}\n")).with_context(|| {
        format!(
            "write TT runtime registration {}",
            project.runtime_registration_path().display()
        )
    })
}

pub fn discover_worktrees(project: &TtProject) -> Result<Vec<WorktreeInfo>> {
    let output = Command::new("git")
        .args(["worktree", "list", "--porcelain"])
        .current_dir(project.primary_repo())
        .output()
        .with_context(|| {
            format!(
                "list git worktrees from primary repo {}",
                project.primary_repo().display()
            )
        })?;
    if !output.status.success() {
        anyhow::bail!(
            "git worktree list failed for {}",
            project.primary_repo().display()
        );
    }
    let stdout = String::from_utf8(output.stdout).context("parse git worktree list as utf-8")?;
    parse_git_worktree_porcelain(&stdout, &project.primary_repo())
}

fn read_project_at(root: &Path) -> Result<TtProject> {
    let root = absolute_logical_path(root)?;
    let config_path = root.join(TT_DIR).join(CONFIG_FILE);
    let config = if config_path.exists() {
        let contents = fs::read_to_string(&config_path)
            .with_context(|| format!("read TT config {}", config_path.display()))?;
        toml::from_str(&contents)
            .with_context(|| format!("parse TT config {}", config_path.display()))?
    } else {
        TtConfig::new(
            PathBuf::from("primary"),
            PathBuf::from(DEFAULT_WORKTREES_DIR),
        )
    };
    Ok(TtProject::new(root, config))
}

fn parse_git_worktree_porcelain(contents: &str, primary_repo: &Path) -> Result<Vec<WorktreeInfo>> {
    let primary_repo = normalize_existing_path(primary_repo);
    let mut worktrees = Vec::new();
    let mut current_path: Option<PathBuf> = None;
    let mut current_branch: Option<String> = None;
    let mut current_head: Option<String> = None;

    for line in contents.lines().chain(std::iter::once("")) {
        if line.is_empty() {
            if let Some(path) = current_path.take() {
                let normalized_path = normalize_existing_path(&path);
                worktrees.push(WorktreeInfo {
                    is_primary: normalized_path == primary_repo,
                    path,
                    branch: current_branch.take(),
                    head: current_head.take(),
                });
            }
            current_branch = None;
            current_head = None;
            continue;
        }

        if let Some(path) = line.strip_prefix("worktree ") {
            current_path = Some(PathBuf::from(path));
        } else if let Some(head) = line.strip_prefix("HEAD ") {
            current_head = Some(head.to_string());
        } else if let Some(branch) = line.strip_prefix("branch ") {
            current_branch = Some(short_branch_name(branch));
        }
    }

    Ok(worktrees)
}

fn short_branch_name(branch: &str) -> String {
    branch
        .strip_prefix("refs/heads/")
        .unwrap_or(branch)
        .to_string()
}

fn normalize_existing_path(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn write_config(path: &Path, config: &TtConfig) -> Result<()> {
    let contents = toml::to_string_pretty(config).context("serialize TT config")?;
    fs::write(path, contents).with_context(|| format!("write TT config {}", path.display()))
}

fn create_dir_if_missing(path: &Path, created_paths: &mut Vec<PathBuf>) -> Result<()> {
    if path.exists() {
        return Ok(());
    }
    fs::create_dir_all(path).with_context(|| format!("create {}", path.display()))?;
    created_paths.push(path.to_path_buf());
    Ok(())
}

fn normalize_relative_path(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        anyhow::bail!("path is absolute: {}", path.display());
    }
    if path.components().any(|component| {
        matches!(
            component,
            std::path::Component::ParentDir | std::path::Component::RootDir
        )
    }) {
        anyhow::bail!("path escapes the TT project root: {}", path.display());
    }
    Ok(path.to_path_buf())
}

fn absolute_logical_path(path: &Path) -> Result<PathBuf> {
    if path.exists() {
        return fs::canonicalize(path)
            .with_context(|| format!("canonicalize path {}", path.display()));
    }
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    let cwd = std::env::current_dir().context("resolve current directory")?;
    Ok(cwd.join(path))
}

fn unix_timestamp_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use tempfile::TempDir;

    #[test]
    fn init_project_creates_expected_layout_and_config() {
        let temp = TempDir::new().expect("tempdir");
        let summary = init_project(InitOptions {
            project_root: temp.path().to_path_buf(),
            primary_repo: PathBuf::from("repo-name"),
            worktrees_dir: PathBuf::from("worktrees"),
        })
        .expect("init project");

        assert_eq!(
            summary.project.primary_repo(),
            temp.path().join("repo-name")
        );
        assert_eq!(
            summary.project.worktrees_dir(),
            temp.path().join("worktrees")
        );
        assert!(temp.path().join(".tt/config.toml").exists());
        assert!(temp.path().join(".codex").is_dir());
        assert!(temp.path().join(".tt/runtime").is_dir());
    }

    #[test]
    fn discover_from_finds_project_ancestor() {
        let temp = TempDir::new().expect("tempdir");
        init_project(InitOptions {
            project_root: temp.path().to_path_buf(),
            primary_repo: PathBuf::from("repo-name"),
            worktrees_dir: PathBuf::from("worktrees"),
        })
        .expect("init project");
        let nested = temp.path().join("repo-name/src");
        fs::create_dir_all(&nested).expect("nested dir");

        let project = TtProject::discover_from(&nested)
            .expect("discover result")
            .expect("project discovered");

        assert_eq!(project.root(), temp.path());
        assert_eq!(project.config().primary_repo, PathBuf::from("repo-name"));
    }

    #[test]
    fn runtime_registration_round_trips() {
        let temp = TempDir::new().expect("tempdir");
        let summary = init_project(InitOptions {
            project_root: temp.path().to_path_buf(),
            primary_repo: PathBuf::from("repo-name"),
            worktrees_dir: PathBuf::from("worktrees"),
        })
        .expect("init project");
        let registration = RuntimeRegistration::new(123, "unix:///tmp/codex.sock".to_string());

        write_runtime_registration(&summary.project, &registration).expect("write registration");

        assert_eq!(
            read_runtime_registration(&summary.project).expect("read registration"),
            Some(registration)
        );
    }

    #[test]
    fn parses_git_worktree_porcelain() {
        let primary = PathBuf::from("/tmp/ttproj/repo-name");
        let worktrees = parse_git_worktree_porcelain(
            "\
worktree /tmp/ttproj/repo-name
HEAD abc123
branch refs/heads/main

worktree /tmp/ttproj/worktrees/feature-a
HEAD def456
branch refs/heads/tt/feature/a

worktree /tmp/ttproj/worktrees/detached
HEAD fedcba
detached
",
            &primary,
        )
        .expect("parse worktree porcelain");

        assert_eq!(
            worktrees,
            vec![
                WorktreeInfo {
                    path: PathBuf::from("/tmp/ttproj/repo-name"),
                    branch: Some("main".to_string()),
                    head: Some("abc123".to_string()),
                    is_primary: true,
                },
                WorktreeInfo {
                    path: PathBuf::from("/tmp/ttproj/worktrees/feature-a"),
                    branch: Some("tt/feature/a".to_string()),
                    head: Some("def456".to_string()),
                    is_primary: false,
                },
                WorktreeInfo {
                    path: PathBuf::from("/tmp/ttproj/worktrees/detached"),
                    branch: None,
                    head: Some("fedcba".to_string()),
                    is_primary: false,
                },
            ]
        );
    }
}
