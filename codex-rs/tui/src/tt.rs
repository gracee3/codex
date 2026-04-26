use std::path::Path;

use anyhow::Context;
use anyhow::Result;
use codex_app_server_protocol::DynamicToolSpec;
use codex_tt_core::TtProject;
use codex_tt_core::TtThreadActivation;
use codex_tt_core::TtThreadRecord;
use codex_tt_core::TtThreadRole;
use codex_tt_core::WorktreeInfo;
use codex_tt_core::discover_repositories;
use codex_tt_core::discover_worktrees_for_repositories;
use codex_tt_core::load_thread_registry;
use codex_tt_core::thread_label_for_cwd;
use codex_tt_core::thread_name_for_cwd;
use codex_tt_core::thread_record_is_alive;
use codex_tt_core::update_thread_record;
use codex_tt_core::upsert_thread_record_pruning_dead;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TtRosterScope {
    All,
    Workers,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TtRosterThreadView {
    pub(crate) role: TtThreadRole,
    pub(crate) activation: TtThreadActivation,
    pub(crate) location: String,
    pub(crate) name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TtRosterView {
    pub(crate) project_name: String,
    pub(crate) threads: Vec<TtRosterThreadView>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TtSupervisorTarget {
    pub(crate) thread_id: String,
    pub(crate) name: String,
    pub(crate) location: String,
    pub(crate) cwd: std::path::PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TtWorkerTarget {
    pub(crate) thread_id: String,
    pub(crate) name: String,
    pub(crate) location: String,
    pub(crate) cwd: std::path::PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TtAssignment {
    pub(crate) worker: TtWorkerTarget,
    pub(crate) body: String,
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
                .filter(|thread| thread_record_is_alive(thread))
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

pub(crate) fn supervisor_target_for_cwd(
    cwd: &Path,
    current_thread_id: &str,
) -> Result<Option<TtSupervisorTarget>> {
    let Some(project) = TtProject::discover_from(cwd)? else {
        return Ok(None);
    };
    let repos = discover_repositories(&project)?;
    let worktrees = discover_worktrees_for_repositories(&repos)?;
    let registry = load_thread_registry(&project)?;
    let mut supervisors = registry
        .threads
        .into_iter()
        .filter(|thread| {
            thread.role == TtThreadRole::Supervisor && thread.thread_id != current_thread_id
        })
        .filter(thread_record_is_alive)
        .map(|thread| TtSupervisorTarget {
            thread_id: thread.thread_id,
            name: thread.name.unwrap_or_else(|| "TT Supervisor".to_string()),
            location: thread_label_for_cwd(&project, &thread.cwd, &worktrees),
            cwd: thread.cwd,
        })
        .collect::<Vec<_>>();
    supervisors.sort_by(|left, right| {
        left.location
            .cmp(&right.location)
            .then_with(|| left.name.cmp(&right.name))
            .then_with(|| left.thread_id.cmp(&right.thread_id))
    });
    Ok(supervisors.into_iter().next())
}

fn worker_relay_body(cwd: &Path, note: &str) -> Result<String> {
    let Some(project) = TtProject::discover_from(cwd)? else {
        anyhow::bail!("no TT project discovered from {}", cwd.display());
    };
    let repos = discover_repositories(&project)?;
    let worktrees = discover_worktrees_for_repositories(&repos)?;
    let location = thread_label_for_cwd(&project, cwd, &worktrees);
    Ok(format!("TT from {location}:\n{}", note.trim()))
}

pub(crate) fn reporting_worker_relay(
    cwd: &Path,
    current_thread_id: &str,
    message: &str,
) -> Result<Option<(TtSupervisorTarget, String)>> {
    let message = message.trim();
    if message.is_empty() {
        return Ok(None);
    }
    let Some(project) = TtProject::discover_from(cwd)? else {
        return Ok(None);
    };
    let registry = load_thread_registry(&project)?;
    let Some(worker) = registry
        .threads
        .iter()
        .find(|thread| thread.thread_id == current_thread_id)
    else {
        return Ok(None);
    };
    if worker.role != TtThreadRole::Worker || worker.activation != TtThreadActivation::Reporting {
        return Ok(None);
    }
    let target = supervisor_target_for_cwd(cwd, current_thread_id)?;
    let Some(target) = target else {
        return Ok(None);
    };
    Ok(Some((target, worker_relay_body(cwd, message)?)))
}

pub(crate) fn supervisor_assignment(
    cwd: &Path,
    supervisor_thread_id: &str,
    worker_ref: &str,
    prompt: &str,
) -> Result<TtAssignment> {
    let Some(project) = TtProject::discover_from(cwd)? else {
        anyhow::bail!("no TT project discovered from {}", cwd.display());
    };
    let repos = discover_repositories(&project)?;
    let worktrees = discover_worktrees_for_repositories(&repos)?;
    let registry = load_thread_registry(&project)?;
    let supervisor = registry
        .threads
        .iter()
        .find(|thread| thread.thread_id == supervisor_thread_id)
        .with_context(|| format!("unknown TT supervisor thread {supervisor_thread_id}"))?;
    if supervisor.role != TtThreadRole::Supervisor {
        anyhow::bail!(
            "TT thread {} is {}, expected supervisor",
            supervisor.thread_id,
            supervisor.role.as_str()
        );
    }
    let worker = worker_target_for_ref(&project, &worktrees, &registry.threads, worker_ref)?;
    let location = thread_label_for_cwd(&project, &supervisor.cwd, &worktrees);
    let body = format!("TT assignment from {location}:\n{}", prompt.trim());
    Ok(TtAssignment { worker, body })
}

pub(crate) fn supervisor_dynamic_tools_for_cwd(cwd: &Path) -> Option<Vec<DynamicToolSpec>> {
    let project = TtProject::discover_from(cwd).ok().flatten()?;
    if normalize_existing_path(cwd) != normalize_existing_path(project.root()) {
        return None;
    }
    Some(vec![DynamicToolSpec {
        namespace: Some("tt".to_string()),
        name: "dispatch".to_string(),
        description:
            "Send a normal turn to a registered TT worker by thread id, name, or repo:branch location."
                .to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "worker": {
                    "type": "string",
                    "description": "TT worker thread id, thread name, or location such as repo:branch."
                },
                "message": {
                    "type": "string",
                    "description": "The task or turn text to send to the worker."
                }
            },
            "required": ["worker", "message"],
            "additionalProperties": false
        }),
        defer_loading: false,
    }])
}

pub(crate) fn roster_for_cwd(cwd: &Path, scope: TtRosterScope) -> Result<Option<TtRosterView>> {
    let Some(project) = TtProject::discover_from(cwd)? else {
        return Ok(None);
    };
    let repos = discover_repositories(&project)?;
    let worktrees = discover_worktrees_for_repositories(&repos)?;
    let registry = load_thread_registry(&project)?;
    let mut threads = registry
        .threads
        .into_iter()
        .filter(thread_record_is_alive)
        .filter(|thread| match scope {
            TtRosterScope::All => true,
            TtRosterScope::Workers => thread.role == TtThreadRole::Worker,
        })
        .map(|thread| TtRosterThreadView {
            role: thread.role,
            activation: thread.activation,
            location: thread_label_for_cwd(&project, &thread.cwd, &worktrees),
            name: thread.name.unwrap_or(thread.thread_id),
        })
        .collect::<Vec<_>>();
    threads.sort_by(|left, right| {
        roster_sort_key(left)
            .cmp(&roster_sort_key(right))
            .then_with(|| left.location.cmp(&right.location))
            .then_with(|| left.name.cmp(&right.name))
    });
    Ok(Some(TtRosterView {
        project_name: project_name(&project),
        threads,
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
    upsert_thread_record_pruning_dead(&project, record)?;
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
    upsert_thread_record_pruning_dead(&project, record)?;
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
    upsert_thread_record_pruning_dead(&project, record)?;
    Ok(message)
}

fn worker_target_for_ref(
    project: &TtProject,
    worktrees: &[WorktreeInfo],
    threads: &[TtThreadRecord],
    worker_ref: &str,
) -> Result<TtWorkerTarget> {
    let mut matches = threads
        .iter()
        .filter(|thread| thread.role == TtThreadRole::Worker)
        .filter(|thread| thread_record_is_alive(thread))
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
        [worker] => Ok(TtWorkerTarget {
            thread_id: worker.thread_id.clone(),
            name: worker
                .name
                .clone()
                .unwrap_or_else(|| worker.thread_id.clone()),
            location: thread_label_for_cwd(project, &worker.cwd, worktrees),
            cwd: worker.cwd.clone(),
        }),
        _ => anyhow::bail!("multiple TT workers match `{worker_ref}`; use a longer thread id"),
    }
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

fn roster_sort_key(thread: &TtRosterThreadView) -> (u8, u8) {
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

    #[test]
    fn auto_register_thread_prunes_dead_duplicate_slot() {
        let temp = tt_project();
        let project = TtProject::discover_from(temp.path())
            .expect("discover")
            .expect("project");
        codex_tt_core::upsert_thread_record(
            &project,
            TtThreadRecord::new(
                TtThreadRole::Supervisor,
                "dead-supervisor".to_string(),
                Some("dead".to_string()),
                temp.path().to_path_buf(),
                Some(u32::MAX),
            ),
        )
        .expect("dead record");

        auto_register_thread(temp.path(), "live-supervisor").expect("auto-register");
        let registry = load_thread_registry(&project).expect("registry");

        assert_eq!(
            registry
                .threads
                .iter()
                .map(|thread| thread.thread_id.as_str())
                .collect::<Vec<_>>(),
            vec!["live-supervisor"]
        );
    }

    #[test]
    fn roster_for_cwd_lists_reporting_workers_first() {
        let temp = tt_project();
        let worker_idle = temp.path().join("worktrees/repo/idle");
        let worker_reporting = temp.path().join("worktrees/repo/reporting");
        std::fs::create_dir_all(&worker_idle).expect("idle worker dir");
        std::fs::create_dir_all(&worker_reporting).expect("reporting worker dir");
        auto_register_thread(temp.path(), "supervisor").expect("supervisor");
        set_thread_reporting(&worker_idle, "worker-idle", /*reporting*/ false)
            .expect("idle worker");
        set_thread_reporting(
            &worker_reporting,
            "worker-reporting",
            /*reporting*/ true,
        )
        .expect("reporting worker");

        let roster = roster_for_cwd(temp.path(), TtRosterScope::Workers)
            .expect("roster")
            .expect("tt roster");

        assert_eq!(
            roster.threads,
            vec![
                TtRosterThreadView {
                    role: TtThreadRole::Worker,
                    activation: TtThreadActivation::Reporting,
                    location: "reporting".to_string(),
                    name: "TT Worker - reporting".to_string(),
                },
                TtRosterThreadView {
                    role: TtThreadRole::Worker,
                    activation: TtThreadActivation::Idle,
                    location: "idle".to_string(),
                    name: "TT Worker - idle".to_string(),
                },
            ]
        );
    }

    #[test]
    fn supervisor_target_skips_current_thread() {
        let temp = tt_project();
        auto_register_thread(temp.path(), "supervisor").expect("supervisor");
        set_thread_reporting(temp.path(), "worker", /*reporting*/ true).expect("worker");

        let target = supervisor_target_for_cwd(temp.path(), "worker")
            .expect("target")
            .expect("supervisor target");
        let project_name = temp
            .path()
            .file_name()
            .and_then(|name| name.to_str())
            .expect("project name");

        assert_eq!(
            target,
            TtSupervisorTarget {
                thread_id: "supervisor".to_string(),
                name: format!("TT Supervisor - {project_name}"),
                location: project_name.to_string(),
                cwd: temp.path().to_path_buf(),
            }
        );
    }

    #[test]
    fn reporting_worker_relay_targets_supervisor() {
        let temp = tt_project();
        auto_register_thread(temp.path(), "supervisor").expect("supervisor");
        set_thread_reporting(temp.path(), "worker", /*reporting*/ true).expect("worker");

        let relay = reporting_worker_relay(temp.path(), "worker", "done")
            .expect("relay")
            .expect("reporting relay");
        let project_name = temp
            .path()
            .file_name()
            .and_then(|name| name.to_str())
            .expect("project name");

        assert_eq!(relay.0.thread_id, "supervisor");
        assert_eq!(relay.1, format!("TT from {project_name}:\ndone"));
    }

    #[test]
    fn supervisor_assignment_resolves_worker_location() {
        let temp = tt_project();
        let worker_cwd = temp.path().join("worktrees/repo/worker-a");
        std::fs::create_dir_all(&worker_cwd).expect("worker dir");
        auto_register_thread(temp.path(), "supervisor").expect("supervisor");
        auto_register_thread(&worker_cwd, "worker").expect("worker");

        let assignment = supervisor_assignment(temp.path(), "supervisor", "worker-a", "build")
            .expect("assignment");
        let project_name = temp
            .path()
            .file_name()
            .and_then(|name| name.to_str())
            .expect("project name");

        assert_eq!(
            assignment.worker,
            TtWorkerTarget {
                thread_id: "worker".to_string(),
                name: "TT Worker - worker-a".to_string(),
                location: "worker-a".to_string(),
                cwd: worker_cwd,
            }
        );
        assert_eq!(
            assignment.body,
            format!("TT assignment from {project_name}:\nbuild")
        );
    }

    #[test]
    fn supervisor_dynamic_tools_only_load_at_project_root() {
        let temp = tt_project();
        let worker_cwd = temp.path().join("worktrees/repo/worker-a");
        std::fs::create_dir_all(&worker_cwd).expect("worker dir");

        let tools = supervisor_dynamic_tools_for_cwd(temp.path()).expect("tools");

        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].namespace.as_deref(), Some("tt"));
        assert_eq!(tools[0].name, "dispatch");
        assert!(supervisor_dynamic_tools_for_cwd(&worker_cwd).is_none());
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
