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
pub const THREADS_FILE: &str = "threads.json";
pub const DEFAULT_WORKTREES_DIR: &str = "worktrees";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TtProject {
    root: PathBuf,
    config: TtConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TtConfig {
    pub worktrees_dir: PathBuf,
}

impl TtConfig {
    pub fn new(worktrees_dir: PathBuf) -> Self {
        Self { worktrees_dir }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitOptions {
    pub project_root: PathBuf,
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
pub struct RepoInfo {
    pub name: String,
    pub path: PathBuf,
    pub branch: Option<String>,
    pub head: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeInfo {
    pub repo_name: String,
    pub path: PathBuf,
    pub branch: Option<String>,
    pub head: Option<String>,
    pub is_repo_root: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TtThreadRole {
    Supervisor,
    Worker,
}

impl TtThreadRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Supervisor => "supervisor",
            Self::Worker => "worker",
        }
    }
}

impl std::str::FromStr for TtThreadRole {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "supervisor" => Ok(Self::Supervisor),
            "worker" => Ok(Self::Worker),
            _ => anyhow::bail!("unknown TT thread role `{value}`"),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TtThreadActivation {
    #[default]
    Idle,
    Reporting,
}

impl TtThreadActivation {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Reporting => "reporting",
        }
    }
}

impl std::str::FromStr for TtThreadActivation {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "idle" | "off" => Ok(Self::Idle),
            "reporting" | "on" => Ok(Self::Reporting),
            _ => anyhow::bail!("unknown TT thread activation `{value}`"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TtThreadRecord {
    pub role: TtThreadRole,
    #[serde(default)]
    pub activation: TtThreadActivation,
    pub thread_id: String,
    pub name: Option<String>,
    pub cwd: PathBuf,
    pub pid: Option<u32>,
    pub registered_at_unix_secs: u64,
}

impl TtThreadRecord {
    pub fn new(
        role: TtThreadRole,
        thread_id: String,
        name: Option<String>,
        cwd: PathBuf,
        pid: Option<u32>,
    ) -> Self {
        Self {
            role,
            activation: TtThreadActivation::Idle,
            thread_id,
            name,
            cwd,
            pid,
            registered_at_unix_secs: unix_timestamp_secs(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TtThreadRegistry {
    pub threads: Vec<TtThreadRecord>,
}

impl TtThreadRegistry {
    pub fn upsert(&mut self, record: TtThreadRecord) {
        if let Some(existing) = self
            .threads
            .iter_mut()
            .find(|thread| thread.thread_id == record.thread_id)
        {
            *existing = record;
        } else {
            self.threads.push(record);
        }
        self.threads.sort_by(|left, right| {
            left.role
                .as_str()
                .cmp(right.role.as_str())
                .then_with(|| left.name.cmp(&right.name))
                .then_with(|| left.thread_id.cmp(&right.thread_id))
        });
    }

    pub fn prune_dead_records_for(&mut self, role: TtThreadRole, cwd: &Path) -> bool {
        let original_len = self.threads.len();
        let cwd = normalize_existing_path(cwd);
        self.threads.retain(|thread| {
            let same_slot = thread.role == role && normalize_existing_path(&thread.cwd) == cwd;
            !same_slot || thread_record_is_alive(thread)
        });
        self.threads.len() != original_len
    }
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

    pub fn worktrees_dir(&self) -> PathBuf {
        self.root.join(&self.config.worktrees_dir)
    }

    pub fn runtime_dir(&self) -> PathBuf {
        self.tt_dir().join(RUNTIME_DIR)
    }

    pub fn runtime_registration_path(&self) -> PathBuf {
        self.runtime_dir().join(RUNTIME_FILE)
    }

    pub fn thread_registry_path(&self) -> PathBuf {
        self.runtime_dir().join(THREADS_FILE)
    }

    pub fn discover_from(start: &Path) -> Result<Option<Self>> {
        for ancestor in start.ancestors() {
            let tt_dir = ancestor.join(TT_DIR);
            if !tt_dir.is_dir() {
                continue;
            }
            if let Some(project) = read_project_at(ancestor)? {
                return Ok(Some(project));
            }
        }
        Ok(None)
    }
}

pub fn init_project(options: InitOptions) -> Result<InitSummary> {
    let project_root = absolute_logical_path(&options.project_root)
        .with_context(|| format!("resolve project root {}", options.project_root.display()))?;
    let worktrees_dir = normalize_relative_path(&options.worktrees_dir)
        .context("worktrees dir must be relative to the TT project root")?;
    let config = TtConfig::new(worktrees_dir);
    let project = TtProject::new(project_root, config);

    let mut created_paths = Vec::new();
    create_dir_if_missing(&project.tt_dir(), &mut created_paths)?;
    create_dir_if_missing(&project.codex_home(), &mut created_paths)?;
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

pub fn load_thread_registry(project: &TtProject) -> Result<TtThreadRegistry> {
    let path = project.thread_registry_path();
    if !path.exists() {
        return Ok(TtThreadRegistry::default());
    }
    let contents = fs::read_to_string(&path)
        .with_context(|| format!("read TT thread registry {}", path.display()))?;
    serde_json::from_str(&contents)
        .with_context(|| format!("parse TT thread registry {}", path.display()))
}

pub fn save_thread_registry(project: &TtProject, registry: &TtThreadRegistry) -> Result<()> {
    fs::create_dir_all(project.runtime_dir())
        .with_context(|| format!("create TT runtime dir {}", project.runtime_dir().display()))?;
    let contents =
        serde_json::to_string_pretty(registry).context("serialize TT thread registry")?;
    fs::write(project.thread_registry_path(), format!("{contents}\n")).with_context(|| {
        format!(
            "write TT thread registry {}",
            project.thread_registry_path().display()
        )
    })
}

pub fn upsert_thread_record(project: &TtProject, record: TtThreadRecord) -> Result<()> {
    let mut registry = load_thread_registry(project)?;
    registry.upsert(record);
    save_thread_registry(project, &registry)
}

pub fn upsert_thread_record_pruning_dead(
    project: &TtProject,
    record: TtThreadRecord,
) -> Result<()> {
    let mut registry = load_thread_registry(project)?;
    registry.prune_dead_records_for(record.role, &record.cwd);
    registry.upsert(record);
    save_thread_registry(project, &registry)
}

pub fn update_thread_record(
    project: &TtProject,
    thread_id: &str,
    update: impl FnOnce(&mut TtThreadRecord),
) -> Result<Option<TtThreadRecord>> {
    let mut registry = load_thread_registry(project)?;
    let Some(record) = registry
        .threads
        .iter_mut()
        .find(|thread| thread.thread_id == thread_id)
    else {
        return Ok(None);
    };
    update(record);
    let updated = record.clone();
    save_thread_registry(project, &registry)?;
    Ok(Some(updated))
}

pub fn discover_repositories(project: &TtProject) -> Result<Vec<RepoInfo>> {
    let mut repos = Vec::new();
    for entry in fs::read_dir(project.root())
        .with_context(|| format!("read TT project root {}", project.root().display()))?
    {
        let entry =
            entry.with_context(|| format!("read entry under {}", project.root().display()))?;
        let file_type = entry
            .file_type()
            .with_context(|| format!("read file type for {}", entry.path().display()))?;
        if !file_type.is_dir() {
            continue;
        }
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if is_reserved_project_dir(project, name, &entry.path()) {
            continue;
        }
        let path = entry.path();
        if !is_git_repo_root(&path) {
            continue;
        }
        repos.push(RepoInfo {
            name: name.to_string(),
            branch: git_branch_for(&path)?,
            head: git_head_for(&path)?,
            path,
        });
    }
    repos.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(repos)
}

pub fn discover_worktrees(project: &TtProject) -> Result<Vec<WorktreeInfo>> {
    let repos = discover_repositories(project)?;
    discover_worktrees_for_repositories(&repos)
}

pub fn discover_worktrees_for_repositories(repos: &[RepoInfo]) -> Result<Vec<WorktreeInfo>> {
    let mut worktrees = Vec::new();
    for repo in repos {
        worktrees.extend(discover_worktrees_for_repository(repo)?);
    }
    worktrees.sort_by(|left, right| {
        left.repo_name
            .cmp(&right.repo_name)
            .then_with(|| left.branch.cmp(&right.branch))
            .then_with(|| left.path.cmp(&right.path))
    });
    Ok(worktrees)
}

fn discover_worktrees_for_repository(repo: &RepoInfo) -> Result<Vec<WorktreeInfo>> {
    let output = Command::new("git")
        .args(["worktree", "list", "--porcelain"])
        .current_dir(&repo.path)
        .output()
        .with_context(|| {
            format!(
                "list git worktrees from repo {} at {}",
                repo.name,
                repo.path.display()
            )
        })?;
    if !output.status.success() {
        anyhow::bail!("git worktree list failed for {}", repo.path.display());
    }
    let stdout = String::from_utf8(output.stdout).context("parse git worktree list as utf-8")?;
    parse_git_worktree_porcelain(&stdout, repo)
}

pub fn worktree_for_cwd<'a>(worktrees: &'a [WorktreeInfo], cwd: &Path) -> Option<&'a WorktreeInfo> {
    let cwd = normalize_existing_path(cwd);
    worktrees
        .iter()
        .filter_map(|worktree| {
            let path = normalize_existing_path(&worktree.path);
            cwd.starts_with(&path)
                .then_some((path.components().count(), worktree))
        })
        .max_by_key(|(component_count, _)| *component_count)
        .map(|(_, worktree)| worktree)
}

pub fn thread_name_for_cwd(
    project: &TtProject,
    role: TtThreadRole,
    cwd: &Path,
    worktrees: &[WorktreeInfo],
) -> String {
    let role_name = match role {
        TtThreadRole::Supervisor => "Supervisor",
        TtThreadRole::Worker => "Worker",
    };
    format!(
        "TT {role_name} - {}",
        thread_label_for_cwd(project, cwd, worktrees)
    )
}

pub fn thread_label_for_cwd(project: &TtProject, cwd: &Path, worktrees: &[WorktreeInfo]) -> String {
    let cwd = normalize_existing_path(cwd);
    if cwd == normalize_existing_path(project.root()) {
        return project_name(project).to_string();
    }
    worktree_label_for_cwd(&cwd, worktrees)
}

pub fn worktree_label_for_cwd(cwd: &Path, worktrees: &[WorktreeInfo]) -> String {
    if let Some(worktree) = worktree_for_cwd(worktrees, cwd) {
        let branch = worktree.branch.as_deref().map(str::to_string).or_else(|| {
            worktree
                .head
                .as_deref()
                .map(|head| format!("detached@{}", short_commit(head)))
        });
        return format!(
            "{}:{}",
            worktree.repo_name,
            branch.unwrap_or_else(|| "unknown".to_string())
        );
    }

    cwd.file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("unknown")
        .to_string()
}

fn read_project_at(root: &Path) -> Result<Option<TtProject>> {
    let root = absolute_logical_path(root)?;
    let config_path = root.join(TT_DIR).join(CONFIG_FILE);
    let config = if config_path.exists() {
        let contents = fs::read_to_string(&config_path)
            .with_context(|| format!("read TT config {}", config_path.display()))?;
        match toml::from_str(&contents) {
            Ok(config) => config,
            Err(_) => return Ok(None),
        }
    } else {
        TtConfig::new(PathBuf::from(DEFAULT_WORKTREES_DIR))
    };
    Ok(Some(TtProject::new(root, config)))
}

fn parse_git_worktree_porcelain(contents: &str, repo: &RepoInfo) -> Result<Vec<WorktreeInfo>> {
    let repo_path = normalize_existing_path(&repo.path);
    let mut worktrees = Vec::new();
    let mut current_path: Option<PathBuf> = None;
    let mut current_branch: Option<String> = None;
    let mut current_head: Option<String> = None;

    for line in contents.lines().chain(std::iter::once("")) {
        if line.is_empty() {
            if let Some(path) = current_path.take() {
                let normalized_path = normalize_existing_path(&path);
                worktrees.push(WorktreeInfo {
                    repo_name: repo.name.clone(),
                    is_repo_root: normalized_path == repo_path,
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

fn short_commit(head: &str) -> &str {
    head.get(..7).unwrap_or(head)
}

fn git_branch_for(repo: &Path) -> Result<Option<String>> {
    let output = Command::new("git")
        .args(["symbolic-ref", "--quiet", "--short", "HEAD"])
        .current_dir(repo)
        .output()
        .with_context(|| format!("read git branch for {}", repo.display()))?;
    if !output.status.success() {
        return Ok(None);
    }
    let stdout = String::from_utf8(output.stdout).context("parse git branch as utf-8")?;
    let branch = stdout.trim();
    if branch.is_empty() {
        Ok(None)
    } else {
        Ok(Some(branch.to_string()))
    }
}

fn git_head_for(repo: &Path) -> Result<Option<String>> {
    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo)
        .output()
        .with_context(|| format!("read git HEAD for {}", repo.display()))?;
    if !output.status.success() {
        return Ok(None);
    }
    let stdout = String::from_utf8(output.stdout).context("parse git HEAD as utf-8")?;
    let head = stdout.trim();
    if head.is_empty() {
        Ok(None)
    } else {
        Ok(Some(head.to_string()))
    }
}

fn is_git_repo_root(path: &Path) -> bool {
    path.join(".git").exists()
}

fn is_reserved_project_dir(project: &TtProject, name: &str, path: &Path) -> bool {
    name == TT_DIR
        || name == CODEX_DIR
        || normalize_existing_path(path) == normalize_existing_path(&project.worktrees_dir())
        || name.starts_with('.')
}

fn project_name(project: &TtProject) -> &str {
    project
        .root()
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("ttproj")
}

fn normalize_existing_path(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

pub fn thread_record_is_alive(thread: &TtThreadRecord) -> bool {
    let Some(pid) = thread.pid else {
        return true;
    };
    process_is_alive(pid)
}

#[cfg(target_os = "linux")]
pub fn process_is_alive(pid: u32) -> bool {
    PathBuf::from(format!("/proc/{pid}")).exists()
}

#[cfg(not(target_os = "linux"))]
pub fn process_is_alive(_pid: u32) -> bool {
    true
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
            worktrees_dir: PathBuf::from("worktrees"),
        })
        .expect("init project");

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
            worktrees_dir: PathBuf::from("worktrees"),
        })
        .expect("init project");
        let nested = temp.path().join("repo-name/src");
        fs::create_dir_all(&nested).expect("nested dir");

        let project = TtProject::discover_from(&nested)
            .expect("discover result")
            .expect("project discovered");

        assert_eq!(project.root(), temp.path());
        assert_eq!(project.config().worktrees_dir, PathBuf::from("worktrees"));
    }

    #[test]
    fn discover_from_skips_incompatible_tt_config() {
        let temp = TempDir::new().expect("tempdir");
        let summary = init_project(InitOptions {
            project_root: temp.path().to_path_buf(),
            worktrees_dir: PathBuf::from("worktrees"),
        })
        .expect("init project");
        let nested = temp.path().join("repo-name/src");
        fs::create_dir_all(nested.join(TT_DIR)).expect("nested tt dir");
        fs::write(
            nested.join(TT_DIR).join(CONFIG_FILE),
            "[tt]\nlegacy = true\n",
        )
        .expect("write incompatible config");

        let project = TtProject::discover_from(&nested)
            .expect("discover result")
            .expect("project discovered");

        assert_eq!(project, summary.project);
    }

    #[test]
    fn runtime_registration_round_trips() {
        let temp = TempDir::new().expect("tempdir");
        let summary = init_project(InitOptions {
            project_root: temp.path().to_path_buf(),
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
        let repo = RepoInfo {
            name: "repo-name".to_string(),
            path: PathBuf::from("/tmp/ttproj/repo-name"),
            branch: Some("main".to_string()),
            head: Some("abc123".to_string()),
        };
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
            &repo,
        )
        .expect("parse worktree porcelain");

        assert_eq!(
            worktrees,
            vec![
                WorktreeInfo {
                    repo_name: "repo-name".to_string(),
                    path: PathBuf::from("/tmp/ttproj/repo-name"),
                    branch: Some("main".to_string()),
                    head: Some("abc123".to_string()),
                    is_repo_root: true,
                },
                WorktreeInfo {
                    repo_name: "repo-name".to_string(),
                    path: PathBuf::from("/tmp/ttproj/worktrees/feature-a"),
                    branch: Some("tt/feature/a".to_string()),
                    head: Some("def456".to_string()),
                    is_repo_root: false,
                },
                WorktreeInfo {
                    repo_name: "repo-name".to_string(),
                    path: PathBuf::from("/tmp/ttproj/worktrees/detached"),
                    branch: None,
                    head: Some("fedcba".to_string()),
                    is_repo_root: false,
                },
            ]
        );
    }

    #[test]
    fn worktree_for_cwd_selects_deepest_matching_worktree() {
        let worktrees = vec![
            WorktreeInfo {
                repo_name: "repo-name".to_string(),
                path: PathBuf::from("/tmp/ttproj/repo-name"),
                branch: Some("main".to_string()),
                head: Some("abc123".to_string()),
                is_repo_root: true,
            },
            WorktreeInfo {
                repo_name: "repo-name".to_string(),
                path: PathBuf::from("/tmp/ttproj/repo-name/nested"),
                branch: Some("nested".to_string()),
                head: Some("def456".to_string()),
                is_repo_root: false,
            },
        ];

        let selected = worktree_for_cwd(&worktrees, Path::new("/tmp/ttproj/repo-name/nested/src"))
            .expect("matching worktree");

        assert_eq!(selected.branch.as_deref(), Some("nested"));
    }

    #[test]
    fn thread_name_for_cwd_prefers_branch_name() {
        let project = TtProject::new(
            PathBuf::from("/tmp/ttproj"),
            TtConfig::new(PathBuf::from("worktrees")),
        );
        let worktrees = vec![WorktreeInfo {
            repo_name: "repo-name".to_string(),
            path: PathBuf::from("/tmp/ttproj/worktrees/feature-a"),
            branch: Some("tt/feature/a".to_string()),
            head: Some("def456".to_string()),
            is_repo_root: false,
        }];

        assert_eq!(
            thread_name_for_cwd(
                &project,
                TtThreadRole::Worker,
                Path::new("/tmp/ttproj/worktrees/feature-a/codex-rs"),
                &worktrees
            ),
            "TT Worker - repo-name:tt/feature/a"
        );
    }

    #[test]
    fn thread_name_for_cwd_uses_project_root_detached_head_or_directory_fallback() {
        let project = TtProject::new(
            PathBuf::from("/tmp/ttproj"),
            TtConfig::new(PathBuf::from("worktrees")),
        );
        let worktrees = vec![WorktreeInfo {
            repo_name: "repo-name".to_string(),
            path: PathBuf::from("/tmp/ttproj/worktrees/detached"),
            branch: None,
            head: Some("fedcba9876543210".to_string()),
            is_repo_root: false,
        }];

        assert_eq!(
            thread_name_for_cwd(
                &project,
                TtThreadRole::Supervisor,
                Path::new("/tmp/ttproj"),
                &worktrees
            ),
            "TT Supervisor - ttproj"
        );
        assert_eq!(
            thread_name_for_cwd(
                &project,
                TtThreadRole::Supervisor,
                Path::new("/tmp/ttproj/worktrees/detached"),
                &worktrees
            ),
            "TT Supervisor - repo-name:detached@fedcba9"
        );
        assert_eq!(
            thread_name_for_cwd(
                &project,
                TtThreadRole::Worker,
                Path::new("/tmp/outside"),
                &worktrees
            ),
            "TT Worker - outside"
        );
    }

    #[test]
    fn thread_registry_upsert_replaces_existing_thread() {
        let mut registry = TtThreadRegistry::default();
        registry.upsert(TtThreadRecord::new(
            TtThreadRole::Worker,
            "thread-1".to_string(),
            Some("first".to_string()),
            PathBuf::from("/tmp/one"),
            Some(1),
        ));
        registry.upsert(TtThreadRecord::new(
            TtThreadRole::Supervisor,
            "thread-1".to_string(),
            Some("supervisor".to_string()),
            PathBuf::from("/tmp/two"),
            Some(2),
        ));

        assert_eq!(
            registry,
            TtThreadRegistry {
                threads: vec![TtThreadRecord {
                    role: TtThreadRole::Supervisor,
                    activation: TtThreadActivation::Idle,
                    thread_id: "thread-1".to_string(),
                    name: Some("supervisor".to_string()),
                    cwd: PathBuf::from("/tmp/two"),
                    pid: Some(2),
                    registered_at_unix_secs: registry.threads[0].registered_at_unix_secs,
                }]
            }
        );
    }

    #[test]
    fn thread_registry_prunes_dead_records_for_same_slot() {
        let mut registry = TtThreadRegistry::default();
        let cwd = PathBuf::from("/tmp/worker");
        registry.upsert(TtThreadRecord::new(
            TtThreadRole::Worker,
            "dead-worker".to_string(),
            Some("dead".to_string()),
            cwd.clone(),
            Some(u32::MAX),
        ));
        registry.upsert(TtThreadRecord::new(
            TtThreadRole::Supervisor,
            "dead-supervisor".to_string(),
            Some("dead supervisor".to_string()),
            cwd.clone(),
            Some(u32::MAX),
        ));

        assert!(registry.prune_dead_records_for(TtThreadRole::Worker, &cwd));

        assert_eq!(
            registry
                .threads
                .iter()
                .map(|thread| thread.thread_id.as_str())
                .collect::<Vec<_>>(),
            vec!["dead-supervisor"]
        );
    }

    #[test]
    fn update_thread_record_mutates_existing_thread() {
        let temp = TempDir::new().expect("tempdir");
        let summary = init_project(InitOptions {
            project_root: temp.path().to_path_buf(),
            worktrees_dir: PathBuf::from("worktrees"),
        })
        .expect("init project");
        let record = TtThreadRecord::new(
            TtThreadRole::Worker,
            "thread-1".to_string(),
            None,
            temp.path().to_path_buf(),
            None,
        );
        upsert_thread_record(&summary.project, record).expect("upsert thread");

        let updated = update_thread_record(&summary.project, "thread-1", |record| {
            record.role = TtThreadRole::Supervisor;
            record.activation = TtThreadActivation::Reporting;
        })
        .expect("update thread")
        .expect("thread exists");

        assert_eq!(
            updated,
            TtThreadRecord {
                role: TtThreadRole::Supervisor,
                activation: TtThreadActivation::Reporting,
                thread_id: "thread-1".to_string(),
                name: None,
                cwd: temp.path().to_path_buf(),
                pid: None,
                registered_at_unix_secs: updated.registered_at_unix_secs,
            }
        );
    }

    #[test]
    fn thread_registry_round_trips() {
        let temp = TempDir::new().expect("tempdir");
        let summary = init_project(InitOptions {
            project_root: temp.path().to_path_buf(),
            worktrees_dir: PathBuf::from("worktrees"),
        })
        .expect("init project");
        let record = TtThreadRecord::new(
            TtThreadRole::Supervisor,
            "thread-supervisor".to_string(),
            None,
            temp.path().join("repo-name"),
            None,
        );

        upsert_thread_record(&summary.project, record.clone()).expect("upsert thread");

        assert_eq!(
            load_thread_registry(&summary.project).expect("load registry"),
            TtThreadRegistry {
                threads: vec![record]
            }
        );
    }
}
