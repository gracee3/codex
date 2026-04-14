use std::ffi::OsString;
use std::path::Path;
use std::path::PathBuf;

use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use ts_rs::TS;

use crate::GitToolingError;
use crate::operations::ensure_git_repository;
use crate::operations::run_git_for_status;
use crate::resolve_root_git_project_for_trust;

const CODEX_MANAGED_WORKTREES_DIR: &str = ".codex/worktrees";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub enum ManagedGitWorkspaceKind {
    RepoRoot,
    EphemeralWorktree,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct ManagedGitWorkspace {
    pub repo_root: PathBuf,
    pub workspace_path: PathBuf,
    pub kind: ManagedGitWorkspaceKind,
}

pub fn managed_worktree_root(repo_root: &Path) -> PathBuf {
    repo_root.join(CODEX_MANAGED_WORKTREES_DIR)
}

pub fn managed_workspace_for_path(cwd: &Path) -> Option<ManagedGitWorkspace> {
    let repo_root = resolve_root_git_project_for_trust(cwd)?;
    let workspace_path = std::fs::canonicalize(cwd).unwrap_or_else(|_| cwd.to_path_buf());
    let managed_root = managed_worktree_root(&repo_root);
    if workspace_path == repo_root {
        return Some(ManagedGitWorkspace {
            repo_root,
            workspace_path,
            kind: ManagedGitWorkspaceKind::RepoRoot,
        });
    }
    if workspace_path.starts_with(&managed_root) {
        return Some(ManagedGitWorkspace {
            repo_root,
            workspace_path,
            kind: ManagedGitWorkspaceKind::EphemeralWorktree,
        });
    }
    None
}

pub fn create_managed_worktree(
    repo_root: &Path,
    worktree_id: &str,
) -> Result<ManagedGitWorkspace, GitToolingError> {
    ensure_git_repository(repo_root)?;
    let managed_root = managed_worktree_root(repo_root);
    std::fs::create_dir_all(&managed_root)?;
    let workspace_path = managed_root.join(worktree_id);
    run_git_for_status(
        repo_root,
        vec![
            OsString::from("worktree"),
            OsString::from("add"),
            OsString::from("--detach"),
            OsString::from("--force"),
            workspace_path.clone().into_os_string(),
            OsString::from("HEAD"),
        ],
        /*env*/ None,
    )?;
    Ok(ManagedGitWorkspace {
        repo_root: repo_root.to_path_buf(),
        workspace_path,
        kind: ManagedGitWorkspaceKind::EphemeralWorktree,
    })
}

pub fn remove_managed_worktree(workspace_path: &Path) -> Result<(), GitToolingError> {
    let Some(workspace) = managed_workspace_for_path(workspace_path) else {
        return Ok(());
    };
    if workspace.kind != ManagedGitWorkspaceKind::EphemeralWorktree {
        return Ok(());
    }
    if !workspace.workspace_path.exists() {
        return Ok(());
    }
    run_git_for_status(
        workspace.repo_root.as_path(),
        vec![
            OsString::from("worktree"),
            OsString::from("remove"),
            OsString::from("--force"),
            workspace.workspace_path.clone().into_os_string(),
        ],
        /*env*/ None,
    )
}

#[cfg(test)]
mod tests {
    use super::ManagedGitWorkspaceKind;
    use super::create_managed_worktree;
    use super::managed_workspace_for_path;
    use super::remove_managed_worktree;
    use pretty_assertions::assert_eq;
    use std::path::Path;
    use std::process::Command;
    use tempfile::tempdir;

    fn run_git_in(repo_path: &Path, args: &[&str]) {
        let status = Command::new("git")
            .current_dir(repo_path)
            .args(args)
            .status()
            .expect("git command");
        assert!(status.success(), "git command failed: {args:?}");
    }

    fn init_test_repo(repo_path: &Path) {
        run_git_in(repo_path, &["init", "--initial-branch=main"]);
        run_git_in(repo_path, &["config", "core.autocrlf", "false"]);
        std::fs::write(repo_path.join("README.md"), "repo\n").expect("write repo file");
        run_git_in(repo_path, &["add", "README.md"]);
        run_git_in(
            repo_path,
            &[
                "-c",
                "user.name=Tester",
                "-c",
                "user.email=test@example.com",
                "commit",
                "-m",
                "init",
            ],
        );
    }

    #[test]
    fn managed_workspace_for_repo_root_returns_repo_root_kind() {
        let temp = tempdir().expect("temp dir");
        let repo = temp.path();
        init_test_repo(repo);

        let workspace = managed_workspace_for_path(repo).expect("repo workspace");
        assert_eq!(workspace.repo_root, repo);
        assert_eq!(workspace.workspace_path, repo);
        assert_eq!(workspace.kind, ManagedGitWorkspaceKind::RepoRoot);
    }

    #[test]
    fn create_and_remove_managed_worktree_round_trips() {
        let temp = tempdir().expect("temp dir");
        let repo = temp.path();
        init_test_repo(repo);

        let workspace =
            create_managed_worktree(repo, "child-thread").expect("managed worktree should create");
        assert_eq!(workspace.kind, ManagedGitWorkspaceKind::EphemeralWorktree);
        assert!(workspace.workspace_path.exists());

        let resolved =
            managed_workspace_for_path(&workspace.workspace_path).expect("managed workspace");
        assert_eq!(resolved, workspace);

        remove_managed_worktree(&workspace.workspace_path).expect("managed worktree should remove");
        assert!(!workspace.workspace_path.exists());
    }
}
