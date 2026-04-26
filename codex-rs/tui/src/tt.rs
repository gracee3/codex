use std::path::Path;

use anyhow::Context;
use anyhow::Result;
use codex_tt_core::TtProject;
use codex_tt_core::TtThreadActivation;
use codex_tt_core::TtThreadRecord;
use codex_tt_core::TtThreadRole;
use codex_tt_core::discover_repositories;
use codex_tt_core::discover_worktrees_for_repositories;
use codex_tt_core::load_thread_registry;
use codex_tt_core::thread_label_for_cwd;
use codex_tt_core::thread_name_for_cwd;
use codex_tt_core::update_thread_record;
use codex_tt_core::upsert_thread_record;

#[derive(Debug, Clone)]
pub(crate) struct TtThreadView {
    pub(crate) role: TtThreadRole,
    pub(crate) activation: TtThreadActivation,
    pub(crate) name: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct TtStatusView {
    pub(crate) project_name: String,
    pub(crate) location: String,
    pub(crate) thread: Option<TtThreadView>,
    pub(crate) repos_count: usize,
    pub(crate) worktrees_count: usize,
    pub(crate) threads_count: usize,
}

pub(crate) fn status_for_cwd(cwd: &Path, thread_id: Option<&str>) -> Result<Option<TtStatusView>> {
    let Some(project) = TtProject::discover_from(cwd)? else {
        return Ok(None);
    };
    let repos = discover_repositories(&project)?;
    let worktrees = discover_worktrees_for_repositories(&repos)?;
    let registry = load_thread_registry(&project)?;
    let thread = thread_id
        .and_then(|thread_id| {
            registry
                .threads
                .iter()
                .find(|thread| thread.thread_id == thread_id)
        })
        .map(|thread| TtThreadView {
            role: thread.role,
            activation: thread.activation,
            name: thread.name.clone(),
        });
    Ok(Some(TtStatusView {
        project_name: project_name(&project),
        location: thread_label_for_cwd(&project, cwd, &worktrees),
        thread,
        repos_count: repos.len(),
        worktrees_count: worktrees.len(),
        threads_count: registry.threads.len(),
    }))
}

pub(crate) fn auto_register_thread(cwd: &Path, thread_id: &str) -> Result<Option<String>> {
    let Some(project) = TtProject::discover_from(cwd)? else {
        return Ok(None);
    };
    let registry = load_thread_registry(&project)?;
    if let Some(existing) = registry
        .threads
        .iter()
        .find(|thread| thread.thread_id == thread_id)
    {
        let (name, _location) = thread_name_and_location(&project, cwd, existing.role)?;
        let Some(record) = update_thread_record(&project, thread_id, |record| {
            record.cwd = cwd.to_path_buf();
            record.pid = Some(std::process::id());
            record.name = Some(name);
        })?
        else {
            return Ok(None);
        };
        return Ok(Some(format!(
            "TT {} {} is {}.",
            record.role.as_str(),
            record.thread_id,
            record.activation.as_str()
        )));
    }

    let role = default_role_for_cwd(&project, cwd);
    let (name, location) = thread_name_and_location(&project, cwd, role)?;
    let record = build_thread_record(cwd, thread_id, role, TtThreadActivation::Idle, name);
    let message = format!(
        "TT {} {} registered at {}.",
        record.role.as_str(),
        record.thread_id,
        location
    );
    upsert_thread_record(&project, record)?;
    Ok(Some(message))
}

pub(crate) fn set_thread_role(cwd: &Path, thread_id: &str, role: TtThreadRole) -> Result<String> {
    let Some(project) = TtProject::discover_from(cwd)? else {
        anyhow::bail!("no TT project discovered from {}", cwd.display());
    };
    if let Some(record) = update_thread_record(&project, thread_id, |record| {
        record.role = role;
    })? {
        return Ok(format!(
            "TT {} {} is {}.",
            record.role.as_str(),
            record.thread_id,
            record.activation.as_str()
        ));
    }

    let (name, _location) = thread_name_and_location(&project, cwd, role)?;
    let record = build_thread_record(cwd, thread_id, role, TtThreadActivation::Idle, name);
    let message = format!(
        "TT {} {} is {}.",
        record.role.as_str(),
        record.thread_id,
        record.activation.as_str()
    );
    upsert_thread_record(&project, record)?;
    Ok(message)
}

pub(crate) fn set_thread_reporting(cwd: &Path, thread_id: &str, reporting: bool) -> Result<String> {
    let Some(project) = TtProject::discover_from(cwd)? else {
        anyhow::bail!("no TT project discovered from {}", cwd.display());
    };
    let activation = if reporting {
        TtThreadActivation::Reporting
    } else {
        TtThreadActivation::Idle
    };
    if let Some(record) = update_thread_record(&project, thread_id, |record| {
        record.activation = activation;
    })? {
        return Ok(format!(
            "TT {} {} is {}.",
            record.role.as_str(),
            record.thread_id,
            record.activation.as_str()
        ));
    }

    let (name, _location) = thread_name_and_location(&project, cwd, TtThreadRole::Worker)?;
    let record = build_thread_record(cwd, thread_id, TtThreadRole::Worker, activation, name);
    let message = format!(
        "TT worker {} is {}.",
        record.thread_id,
        record.activation.as_str()
    );
    upsert_thread_record(&project, record)?;
    Ok(message)
}

fn build_thread_record(
    cwd: &Path,
    thread_id: &str,
    role: TtThreadRole,
    activation: TtThreadActivation,
    name: String,
) -> TtThreadRecord {
    let mut record = TtThreadRecord::new(
        role,
        thread_id.to_string(),
        Some(name),
        cwd.to_path_buf(),
        Some(std::process::id()),
    );
    record.activation = activation;
    record
}

fn thread_name_and_location(
    project: &TtProject,
    cwd: &Path,
    role: TtThreadRole,
) -> Result<(String, String)> {
    let repos = discover_repositories(project)?;
    let worktrees = discover_worktrees_for_repositories(&repos)?;
    Ok((
        thread_name_for_cwd(project, role, cwd, &worktrees),
        thread_label_for_cwd(project, cwd, &worktrees),
    ))
}

fn default_role_for_cwd(project: &TtProject, cwd: &Path) -> TtThreadRole {
    if normalize_existing_path(cwd) == normalize_existing_path(project.root()) {
        TtThreadRole::Supervisor
    } else {
        TtThreadRole::Worker
    }
}

fn project_name(project: &TtProject) -> String {
    project
        .root()
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .with_context(|| format!("resolve TT project name for {}", project.root().display()))
        .unwrap_or_else(|_| "ttproj".to_string())
}

fn normalize_existing_path(path: &Path) -> std::path::PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_tt_core::InitOptions;
    use codex_tt_core::init_project;
    use pretty_assertions::assert_eq;
    use tempfile::TempDir;

    #[test]
    fn auto_register_thread_registers_project_root_as_supervisor() {
        let temp = tt_project();

        auto_register_thread(temp.path(), "thread-1").expect("auto-register");
        let registry = load_thread_registry(
            &TtProject::discover_from(temp.path())
                .expect("discover")
                .expect("project"),
        )
        .expect("registry");

        assert_eq!(registry.threads.len(), 1);
        let record = &registry.threads[0];
        assert_eq!(record.thread_id, "thread-1");
        assert_eq!(record.role, TtThreadRole::Supervisor);
        assert_eq!(record.activation, TtThreadActivation::Idle);
        assert_eq!(record.cwd, temp.path());
        assert_eq!(record.pid, Some(std::process::id()));
    }

    #[test]
    fn auto_register_thread_preserves_manual_role_and_activation() {
        let temp = tt_project();
        set_thread_role(temp.path(), "thread-1", TtThreadRole::Worker).expect("set role");
        set_thread_reporting(temp.path(), "thread-1", /*reporting*/ true).expect("set reporting");

        auto_register_thread(temp.path(), "thread-1").expect("auto-register");
        let status = status_for_cwd(temp.path(), Some("thread-1"))
            .expect("status")
            .expect("tt status");
        let thread = status.thread.expect("registered thread");

        assert_eq!(thread.role, TtThreadRole::Worker);
        assert_eq!(thread.activation, TtThreadActivation::Reporting);
    }

    fn tt_project() -> TempDir {
        let temp = TempDir::new().expect("tempdir");
        init_project(InitOptions {
            project_root: temp.path().to_path_buf(),
            worktrees_dir: "worktrees".into(),
        })
        .expect("init project");
        temp
    }
}
