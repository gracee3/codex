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

    let record = build_thread_record(&project, cwd, thread_id, role, TtThreadActivation::Idle)?;
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

    let record = build_thread_record(&project, cwd, thread_id, TtThreadRole::Worker, activation)?;
    let message = format!(
        "TT worker {} is {}.",
        record.thread_id,
        record.activation.as_str()
    );
    upsert_thread_record(&project, record)?;
    Ok(message)
}

fn build_thread_record(
    project: &TtProject,
    cwd: &Path,
    thread_id: &str,
    role: TtThreadRole,
    activation: TtThreadActivation,
) -> Result<TtThreadRecord> {
    let repos = discover_repositories(project)?;
    let worktrees = discover_worktrees_for_repositories(&repos)?;
    let mut record = TtThreadRecord::new(
        role,
        thread_id.to_string(),
        Some(thread_name_for_cwd(project, role, cwd, &worktrees)),
        cwd.to_path_buf(),
        Some(std::process::id()),
    );
    record.activation = activation;
    Ok(record)
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
