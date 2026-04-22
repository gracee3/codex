use std::fmt;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;
use chrono::DateTime;
use chrono::Utc;
use serde::Deserialize;
use serde::Serialize;

const TT_DIR: &str = ".tt";
const CODEX_DIR: &str = ".codex";
const TT_RUNTIME_DIR: &str = "tt";
const PLAN_FILE: &str = "plan.md";
const ROSTER_FILE: &str = "roster.md";
const STATE_FILE: &str = "state.json";
const LOG_FILE: &str = "log.ndjson";
const ACTIVATE_FILE: &str = "activate";
const DAEMON_LOG_FILE: &str = "daemon.log";
const ROLES_DIR: &str = "roles";
const SUPERVISOR_ROLE_FILE: &str = "supervisor.md";
const DIRECTOR_ROLE_FILE: &str = "director.md";
const DEVELOPER_ROLE_FILE: &str = "developer.md";
const PRIMARY_DIR: &str = "primary";
const WORKTREES_DIR: &str = "worktrees";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Supervisor,
    Director,
    Developer,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Supervisor => "supervisor",
            Self::Director => "director",
            Self::Developer => "developer",
        }
    }
}

impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DefaultView {
    Supervisor,
    Worker { name: String },
}

impl fmt::Display for DefaultView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Supervisor => f.write_str("supervisor"),
            Self::Worker { name } => write!(f, "worker:{name}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerKind {
    Director,
    Developer,
    Worker,
}

impl WorkerKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Director => "director",
            Self::Developer => "developer",
            Self::Worker => "worker",
        }
    }

    pub fn preset_role(self) -> Option<Role> {
        match self {
            Self::Director => Some(Role::Director),
            Self::Developer => Some(Role::Developer),
            Self::Worker => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerBinding {
    Managed,
    Adopted,
}

impl WorkerBinding {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Managed => "managed",
            Self::Adopted => "adopted",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerRecord {
    pub name: String,
    pub kind: WorkerKind,
    pub cwd: PathBuf,
    pub binding: WorkerBinding,
    pub thread_id: Option<String>,
    pub instruction_path: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TtState {
    pub supervisor_thread_id: Option<String>,
    pub workers: Vec<WorkerRecord>,
    pub runtime_running: bool,
    pub runtime_pid: Option<u32>,
    pub runtime_websocket_url: Option<String>,
    pub runtime_auth_token: Option<String>,
    pub last_runtime_started_at: Option<DateTime<Utc>>,
    pub auto_loop: bool,
    pub operator_pause: bool,
    pub default_view: DefaultView,
    pub active_dispatch_id: Option<String>,
    pub pending_director_evaluation: bool,
    pub pending_developer_dispatch: bool,
}

impl Default for TtState {
    fn default() -> Self {
        Self {
            supervisor_thread_id: None,
            workers: Vec::new(),
            runtime_running: false,
            runtime_pid: None,
            runtime_websocket_url: None,
            runtime_auth_token: None,
            last_runtime_started_at: None,
            auto_loop: false,
            operator_pause: false,
            default_view: DefaultView::Supervisor,
            active_dispatch_id: None,
            pending_director_evaluation: false,
            pending_developer_dispatch: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DispatchEnvelope {
    pub dispatch_id: String,
    pub objective: String,
    pub scope: String,
    pub constraints: String,
    pub expected_output: String,
    pub completion_criteria: String,
    pub priority: String,
    pub plan_refs: String,
    pub todo_refs: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResultEnvelope {
    pub dispatch_id: String,
    pub status: String,
    pub summary: String,
    pub work_completed: String,
    pub artifacts: String,
    pub open_questions: String,
    pub recommended_next_step: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogEvent {
    pub ts: DateTime<Utc>,
    pub event: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<Role>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worker_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone)]
pub struct WorkspacePaths {
    workspace_root: PathBuf,
}

impl WorkspacePaths {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }

    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    pub fn tt_dir(&self) -> PathBuf {
        self.workspace_root.join(TT_DIR)
    }

    pub fn codex_home(&self) -> PathBuf {
        self.workspace_root.join(CODEX_DIR)
    }

    pub fn runtime_dir(&self) -> PathBuf {
        self.codex_home().join(TT_RUNTIME_DIR)
    }

    pub fn primary_checkout(&self) -> PathBuf {
        self.workspace_root.join(PRIMARY_DIR)
    }

    pub fn worktrees_dir(&self) -> PathBuf {
        self.workspace_root.join(WORKTREES_DIR)
    }

    pub fn worker_checkout(&self, name: &str) -> PathBuf {
        self.worktrees_dir().join(name)
    }

    pub fn roles_dir(&self) -> PathBuf {
        self.tt_dir().join(ROLES_DIR)
    }

    pub fn activate_path(&self) -> PathBuf {
        self.tt_dir().join(ACTIVATE_FILE)
    }

    pub fn plan_path(&self) -> PathBuf {
        self.tt_dir().join(PLAN_FILE)
    }

    pub fn roster_path(&self) -> PathBuf {
        self.tt_dir().join(ROSTER_FILE)
    }

    pub fn state_path(&self) -> PathBuf {
        self.runtime_dir().join(STATE_FILE)
    }

    pub fn log_path(&self) -> PathBuf {
        self.runtime_dir().join(LOG_FILE)
    }

    pub fn daemon_log_path(&self) -> PathBuf {
        self.runtime_dir().join(DAEMON_LOG_FILE)
    }

    pub fn role_path(&self, role: Role) -> PathBuf {
        self.roles_dir().join(match role {
            Role::Supervisor => SUPERVISOR_ROLE_FILE,
            Role::Director => DIRECTOR_ROLE_FILE,
            Role::Developer => DEVELOPER_ROLE_FILE,
        })
    }

    pub fn is_workspace_root(path: &Path) -> bool {
        path.join(TT_DIR).is_dir()
            && path.join(CODEX_DIR).is_dir()
            && path.join(PRIMARY_DIR).is_dir()
            && path.join(WORKTREES_DIR).is_dir()
    }

    pub fn discover_from(start: &Path) -> Option<Self> {
        start.ancestors().find_map(|ancestor| {
            Self::is_workspace_root(ancestor).then(|| Self::new(ancestor.to_path_buf()))
        })
    }
}

pub fn activate_tt_env(paths: &WorkspacePaths) {
    let codex_home = paths.codex_home();
    let tt_home = paths.tt_dir();
    let workspace_root = paths.workspace_root().to_path_buf();

    // SAFETY: TT sets these process-scoped variables before spawning worker threads so all child
    // Codex processes resolve the same workspace-local home and checked-in defaults layer.
    unsafe {
        std::env::set_var("CODEX_HOME", &codex_home);
        std::env::set_var("TT_HOME", &tt_home);
        std::env::set_var("TT_REPO_ROOT", &workspace_root);
    }
}

pub fn ensure_workspace_artifacts(paths: &WorkspacePaths) -> Result<()> {
    fs::create_dir_all(paths.tt_dir())
        .with_context(|| format!("create TT dir {}", paths.tt_dir().display()))?;
    fs::create_dir_all(paths.roles_dir())
        .with_context(|| format!("create TT roles dir {}", paths.roles_dir().display()))?;
    fs::create_dir_all(paths.codex_home())
        .with_context(|| format!("create CODEX_HOME {}", paths.codex_home().display()))?;
    fs::create_dir_all(paths.runtime_dir())
        .with_context(|| format!("create TT runtime dir {}", paths.runtime_dir().display()))?;
    fs::create_dir_all(paths.primary_checkout()).with_context(|| {
        format!(
            "create primary checkout dir {}",
            paths.primary_checkout().display()
        )
    })?;
    fs::create_dir_all(paths.worktrees_dir())
        .with_context(|| format!("create worktrees dir {}", paths.worktrees_dir().display()))?;

    migrate_legacy_runtime_artifacts(paths)?;

    write_if_missing(&paths.plan_path(), default_plan_markdown())?;
    write_if_missing(&paths.roster_path(), default_roster_markdown())?;
    write_if_missing(&paths.activate_path(), default_activate_script())?;
    write_if_missing(
        &paths.role_path(Role::Supervisor),
        default_supervisor_role_markdown(),
    )?;
    write_if_missing(
        &paths.role_path(Role::Director),
        default_director_role_markdown(),
    )?;
    write_if_missing(
        &paths.role_path(Role::Developer),
        default_developer_role_markdown(),
    )?;

    if !paths.state_path().exists() {
        save_state(paths, &default_workspace_state(paths))?;
    } else {
        let mut state = load_state(paths)?;
        seed_missing_default_workers(paths, &mut state);
        save_state(paths, &state)?;
    }
    if !paths.log_path().exists() {
        fs::write(paths.log_path(), b"")
            .with_context(|| format!("create TT log at {}", paths.log_path().display()))?;
    }

    Ok(())
}

pub fn load_state(paths: &WorkspacePaths) -> Result<TtState> {
    let bytes = fs::read(paths.state_path())
        .with_context(|| format!("read TT state {}", paths.state_path().display()))?;
    let raw: RawTtState = serde_json::from_slice(&bytes)
        .with_context(|| format!("parse TT state {}", paths.state_path().display()))?;
    Ok(raw.into_state(paths))
}

pub fn save_state(paths: &WorkspacePaths, state: &TtState) -> Result<()> {
    let json = serde_json::to_vec_pretty(state).context("serialize TT state")?;
    fs::write(paths.state_path(), json)
        .with_context(|| format!("write TT state {}", paths.state_path().display()))
}

pub fn append_log(paths: &WorkspacePaths, event: LogEvent) -> Result<()> {
    let line = serde_json::to_string(&event).context("serialize TT log event")?;
    let mut existing = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(paths.log_path())
        .with_context(|| format!("open TT log {}", paths.log_path().display()))?;
    use std::io::Write as _;
    writeln!(existing, "{line}").context("append TT log event")
}

pub fn default_log_event(
    event: impl Into<String>,
    role: Option<Role>,
    worker_name: Option<String>,
    thread_id: Option<String>,
    detail: Option<String>,
) -> LogEvent {
    LogEvent {
        ts: Utc::now(),
        event: event.into(),
        role,
        worker_name,
        thread_id,
        detail,
    }
}

pub fn parse_dispatch_envelope(text: &str) -> Result<DispatchEnvelope> {
    let body = extract_envelope_body(text, "[DISPATCH]", "[/DISPATCH]")?;
    Ok(DispatchEnvelope {
        dispatch_id: required_field(&body, "Dispatch-ID")?,
        objective: required_field(&body, "Objective")?,
        scope: required_field(&body, "Scope")?,
        constraints: required_field(&body, "Constraints")?,
        expected_output: required_field(&body, "Expected-Output")?,
        completion_criteria: required_field(&body, "Completion-Criteria")?,
        priority: required_field(&body, "Priority")?,
        plan_refs: required_field(&body, "Plan-Refs")?,
        todo_refs: required_field(&body, "Todo-Refs")?,
    })
}

pub fn parse_result_envelope(text: &str) -> Result<ResultEnvelope> {
    let body = extract_envelope_body(text, "[RESULT]", "[/RESULT]")?;
    Ok(ResultEnvelope {
        dispatch_id: required_field(&body, "Dispatch-ID")?,
        status: required_field(&body, "Status")?,
        summary: required_field(&body, "Summary")?,
        work_completed: required_field(&body, "Work-Completed")?,
        artifacts: required_field(&body, "Artifacts")?,
        open_questions: required_field(&body, "Open-Questions")?,
        recommended_next_step: required_field(&body, "Recommended-Next-Step")?,
    })
}

pub fn default_worker_records(paths: &WorkspacePaths) -> Vec<WorkerRecord> {
    vec![
        WorkerRecord {
            name: "director".to_string(),
            kind: WorkerKind::Director,
            cwd: paths.primary_checkout(),
            binding: WorkerBinding::Managed,
            thread_id: None,
            instruction_path: Some(paths.role_path(Role::Director)),
        },
        WorkerRecord {
            name: "developer".to_string(),
            kind: WorkerKind::Developer,
            cwd: paths.primary_checkout(),
            binding: WorkerBinding::Managed,
            thread_id: None,
            instruction_path: Some(paths.role_path(Role::Developer)),
        },
    ]
}

fn default_workspace_state(paths: &WorkspacePaths) -> TtState {
    TtState {
        workers: default_worker_records(paths),
        ..TtState::default()
    }
}

fn seed_missing_default_workers(paths: &WorkspacePaths, state: &mut TtState) {
    if state.workers.is_empty() {
        state.workers = default_worker_records(paths);
        return;
    }

    for default_worker in default_worker_records(paths) {
        if let Some(existing) = state
            .workers
            .iter_mut()
            .find(|worker| worker.name == default_worker.name)
        {
            existing.binding = WorkerBinding::Managed;
            if existing.instruction_path.is_none() {
                existing.instruction_path = default_worker.instruction_path;
            }
            continue;
        }
        state.workers.push(default_worker);
    }
}

#[derive(Debug, Clone, Deserialize)]
struct RawWorkerRecord {
    name: String,
    kind: WorkerKind,
    cwd: PathBuf,
    binding: Option<WorkerBinding>,
    thread_id: Option<String>,
    instruction_path: Option<PathBuf>,
}

impl RawWorkerRecord {
    fn into_worker_record(self, paths: &WorkspacePaths) -> WorkerRecord {
        let binding = self
            .binding
            .unwrap_or_else(|| infer_worker_binding(paths, &self));
        WorkerRecord {
            name: self.name,
            kind: self.kind,
            cwd: self.cwd,
            binding,
            thread_id: self.thread_id,
            instruction_path: self.instruction_path,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
struct RawTtState {
    supervisor_thread_id: Option<String>,
    workers: Vec<RawWorkerRecord>,
    runtime_running: bool,
    runtime_pid: Option<u32>,
    runtime_websocket_url: Option<String>,
    runtime_auth_token: Option<String>,
    last_runtime_started_at: Option<DateTime<Utc>>,
    auto_loop: bool,
    operator_pause: bool,
    default_view: DefaultView,
    active_dispatch_id: Option<String>,
    pending_director_evaluation: bool,
    pending_developer_dispatch: bool,
}

impl RawTtState {
    fn into_state(self, paths: &WorkspacePaths) -> TtState {
        TtState {
            supervisor_thread_id: self.supervisor_thread_id,
            workers: self
                .workers
                .into_iter()
                .map(|worker| worker.into_worker_record(paths))
                .collect(),
            runtime_running: self.runtime_running,
            runtime_pid: self.runtime_pid,
            runtime_websocket_url: self.runtime_websocket_url,
            runtime_auth_token: self.runtime_auth_token,
            last_runtime_started_at: self.last_runtime_started_at,
            auto_loop: self.auto_loop,
            operator_pause: self.operator_pause,
            default_view: self.default_view,
            active_dispatch_id: self.active_dispatch_id,
            pending_director_evaluation: self.pending_director_evaluation,
            pending_developer_dispatch: self.pending_developer_dispatch,
        }
    }
}

fn infer_worker_binding(paths: &WorkspacePaths, worker: &RawWorkerRecord) -> WorkerBinding {
    if worker.kind.preset_role().is_some() {
        return WorkerBinding::Managed;
    }

    let worker_root = normalize_existing_path(&paths.worker_checkout(&worker.name));
    let worker_cwd = normalize_existing_path(&worker.cwd);
    if worker_cwd == worker_root {
        WorkerBinding::Managed
    } else {
        WorkerBinding::Adopted
    }
}

fn extract_envelope_body(text: &str, start_marker: &str, end_marker: &str) -> Result<String> {
    let start = text
        .rfind(start_marker)
        .with_context(|| format!("missing envelope start marker {start_marker}"))?;
    let text = &text[start + start_marker.len()..];
    let end = text
        .find(end_marker)
        .with_context(|| format!("missing envelope end marker {end_marker}"))?;
    Ok(text[..end].trim().to_string())
}

fn required_field(body: &str, field_name: &str) -> Result<String> {
    for line in body.lines() {
        let trimmed = line.trim();
        if let Some(value) = trimmed.strip_prefix(&format!("{field_name}:")) {
            let value = value.trim();
            if value.is_empty() {
                anyhow::bail!("empty envelope field {field_name}");
            }
            return Ok(value.to_string());
        }
    }
    anyhow::bail!("missing envelope field {field_name}")
}

fn write_if_missing(path: &Path, contents: &str) -> Result<()> {
    if path.exists() {
        return Ok(());
    }
    fs::write(path, contents).with_context(|| format!("write {}", path.display()))
}

fn normalize_existing_path(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn migrate_legacy_runtime_artifacts(paths: &WorkspacePaths) -> Result<()> {
    for (legacy_path, runtime_path) in [
        (paths.tt_dir().join(STATE_FILE), paths.state_path()),
        (paths.tt_dir().join(LOG_FILE), paths.log_path()),
        (
            paths.tt_dir().join(DAEMON_LOG_FILE),
            paths.daemon_log_path(),
        ),
    ] {
        if !legacy_path.exists() || runtime_path.exists() {
            continue;
        }
        fs::rename(&legacy_path, &runtime_path).with_context(|| {
            format!(
                "migrate TT runtime artifact {} -> {}",
                legacy_path.display(),
                runtime_path.display()
            )
        })?;
    }
    Ok(())
}

fn default_plan_markdown() -> &'static str {
    "# TT Plan\n\n## Current Objective\n- Establish the current objective.\n\n## Milestone\n- Record the active milestone.\n\n## Active Todos\n- Add current todos here.\n\n## Constraints\n- Capture workspace, product, or runtime constraints here.\n\n## Open Questions\n- Record decisions that still need operator input.\n\n## Recent Decisions\n- Append resolved decisions as they happen.\n\n## Next Likely Dispatches\n- Note the next likely Director to Developer handoffs.\n"
}

fn default_roster_markdown() -> &'static str {
    "# TT Workspace Roster\n\n## Supervisor\n- Own operator interaction and workspace-level coordination.\n- Inspect worker history and decide where to steer work next.\n\n## Director\n- Own orchestration, prioritization, evaluation, and operator interaction.\n- Emit structured dispatches only.\n- Do not simulate implementation work.\n\n## Developer\n- Own scoped execution only.\n- Emit structured results only.\n- Do not self-dispatch or silently expand scope.\n\n## Additional Workers\n- Live under `worktrees/` and are managed through `tt worker` commands.\n"
}

fn default_activate_script() -> &'static str {
    r#"#!/usr/bin/env bash
if [ -n "${BASH_SOURCE[0]:-}" ]; then
  _tt_activate_source="${BASH_SOURCE[0]}"
else
  _tt_activate_source="$0"
fi
_tt_activate_dir="$(cd "$(dirname "${_tt_activate_source}")" && pwd -P)"
_tt_workspace_root="$(cd "${_tt_activate_dir}/.." && pwd -P)"

export TT_REPO_ROOT="${_tt_workspace_root}"
export TT_HOME="${_tt_workspace_root}/.tt"
export CODEX_HOME="${_tt_workspace_root}/.codex"

unset _tt_activate_source
unset _tt_activate_dir
unset _tt_workspace_root
"#
}

fn default_supervisor_role_markdown() -> &'static str {
    "You are the TT Supervisor.\n\nResponsibilities:\n- Coordinate work across the TT workspace.\n- Read worker history and operator guidance.\n- Decide which worker should receive the next prompt.\n\nRules:\n- Do not impersonate worker execution.\n- Prefer explicit worker routing over broad instructions.\n- Treat the TT workspace as the shared control plane.\n"
}

fn default_director_role_markdown() -> &'static str {
    "You are the TT Director.\n\nResponsibilities:\n- Evaluate progress against `.tt/plan.md`.\n- Interact with the operator when needed.\n- Produce only scoped Developer dispatches.\n- Stop when blocked, paused, or awaiting operator input.\n\nRules:\n- Do not simulate Developer execution.\n- Keep dispatches structured and parser-friendly.\n- Treat TT as the orchestration authority.\n"
}

fn default_developer_role_markdown() -> &'static str {
    "You are the TT Developer.\n\nResponsibilities:\n- Wait for Director dispatches.\n- Execute only the assigned scope.\n- Report structured results with blockers and next steps.\n\nRules:\n- Do not self-dispatch.\n- Do not silently expand scope.\n- Stop at the assignment boundary.\n"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_default_artifacts_and_state() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let paths = WorkspacePaths::new(tempdir.path().to_path_buf());

        ensure_workspace_artifacts(&paths).expect("create artifacts");

        assert!(paths.plan_path().exists());
        assert!(paths.roster_path().exists());
        assert!(paths.activate_path().exists());
        assert!(paths.primary_checkout().exists());
        assert!(paths.worktrees_dir().exists());
        assert!(paths.state_path().exists());
        assert!(paths.log_path().exists());
        assert!(paths.role_path(Role::Supervisor).exists());
        assert!(paths.role_path(Role::Director).exists());
        assert!(paths.role_path(Role::Developer).exists());
        assert_eq!(
            paths.state_path(),
            tempdir.path().join(".codex/tt/state.json")
        );
        assert_eq!(
            paths.log_path(),
            tempdir.path().join(".codex/tt/log.ndjson")
        );

        let state = load_state(&paths).expect("load state");
        assert_eq!(state.supervisor_thread_id, None);
        assert_eq!(state.default_view, DefaultView::Supervisor);
        assert_eq!(state.workers.len(), 2);
        assert_eq!(state.workers[0].name, "director");
        assert_eq!(state.workers[0].binding, WorkerBinding::Managed);
        assert_eq!(state.workers[0].cwd, paths.primary_checkout());
        assert_eq!(state.workers[1].name, "developer");
        assert_eq!(state.workers[1].binding, WorkerBinding::Managed);
    }

    #[test]
    fn discovers_workspace_root_from_nested_path() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let paths = WorkspacePaths::new(tempdir.path().to_path_buf());
        ensure_workspace_artifacts(&paths).expect("create artifacts");
        let nested = paths.primary_checkout().join("src/module");
        fs::create_dir_all(&nested).expect("create nested dir");

        let discovered = WorkspacePaths::discover_from(&nested).expect("discover workspace");
        assert_eq!(discovered.workspace_root(), paths.workspace_root());
    }

    #[test]
    fn appends_jsonl_log_events() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let paths = WorkspacePaths::new(tempdir.path().to_path_buf());
        ensure_workspace_artifacts(&paths).expect("create artifacts");

        append_log(
            &paths,
            default_log_event(
                "thread-created",
                Some(Role::Supervisor),
                Some("director".to_string()),
                Some("thr_123".to_string()),
                None,
            ),
        )
        .expect("append log");

        let contents = fs::read_to_string(paths.log_path()).expect("read log");
        assert!(contents.contains("\"event\":\"thread-created\""));
        assert!(contents.contains("\"role\":\"supervisor\""));
        assert!(contents.contains("\"worker_name\":\"director\""));
    }

    #[test]
    fn migrates_legacy_runtime_artifacts_into_codex_home() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let paths = WorkspacePaths::new(tempdir.path().to_path_buf());
        fs::create_dir_all(paths.tt_dir()).expect("create tt dir");
        fs::write(paths.tt_dir().join(STATE_FILE), br#"{"supervisor_thread_id":null,"workers":[],"runtime_running":false,"runtime_pid":null,"runtime_websocket_url":null,"runtime_auth_token":null,"last_runtime_started_at":null,"auto_loop":false,"operator_pause":false,"default_view":"supervisor","active_dispatch_id":null,"pending_director_evaluation":false,"pending_developer_dispatch":false}"#).expect("write legacy state");
        fs::write(paths.tt_dir().join(LOG_FILE), b"legacy-log\n").expect("write legacy log");

        ensure_workspace_artifacts(&paths).expect("create artifacts");

        assert!(!paths.tt_dir().join(STATE_FILE).exists());
        assert!(!paths.tt_dir().join(LOG_FILE).exists());
        assert!(paths.state_path().exists());
        assert_eq!(
            fs::read_to_string(paths.log_path()).expect("read migrated log"),
            "legacy-log\n"
        );
    }

    #[test]
    fn migrates_legacy_worker_bindings_from_worker_shape() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let paths = WorkspacePaths::new(tempdir.path().to_path_buf());
        fs::create_dir_all(paths.runtime_dir()).expect("create runtime dir");

        let managed_worker_path = paths.worker_checkout("feature-a");
        let adopted_worker_path = tempdir.path().join("adopted");
        fs::create_dir_all(&managed_worker_path).expect("create managed worker path");
        fs::create_dir_all(&adopted_worker_path).expect("create adopted worker path");

        fs::write(
            paths.state_path(),
            format!(
                r#"{{
  "supervisor_thread_id": null,
  "workers": [
    {{
      "name": "director",
      "kind": "director",
      "cwd": "{}",
      "thread_id": null,
      "instruction_path": "{}"
    }},
    {{
      "name": "feature-a",
      "kind": "worker",
      "cwd": "{}",
      "thread_id": "thr_managed",
      "instruction_path": null
    }},
    {{
      "name": "adopted",
      "kind": "worker",
      "cwd": "{}",
      "thread_id": "thr_adopted",
      "instruction_path": null
    }}
  ],
  "runtime_running": false,
  "runtime_pid": null,
  "runtime_websocket_url": null,
  "runtime_auth_token": null,
  "last_runtime_started_at": null,
  "auto_loop": false,
  "operator_pause": false,
  "default_view": "supervisor",
  "active_dispatch_id": null,
  "pending_director_evaluation": false,
  "pending_developer_dispatch": false
}}"#,
                paths.primary_checkout().display(),
                paths.role_path(Role::Director).display(),
                managed_worker_path.display(),
                adopted_worker_path.display()
            ),
        )
        .expect("write legacy state");

        let state = load_state(&paths).expect("load state");
        assert_eq!(state.workers.len(), 3);
        assert_eq!(state.workers[0].binding, WorkerBinding::Managed);
        assert_eq!(state.workers[1].binding, WorkerBinding::Managed);
        assert_eq!(state.workers[2].binding, WorkerBinding::Adopted);

        save_state(&paths, &state).expect("save state");
        let saved = fs::read_to_string(paths.state_path()).expect("read saved state");
        assert!(saved.contains(r#""binding": "managed""#));
        assert!(saved.contains(r#""binding": "adopted""#));
    }

    #[test]
    fn parses_dispatch_envelope() {
        let envelope = parse_dispatch_envelope(
            "[DISPATCH]
Dispatch-ID: dsp_123
Objective: Ship detached runtime
Scope: tt-cli and tt-core
Constraints: Reuse app-server websocket
Expected-Output: working routing loop
Completion-Criteria: status reflects dispatch
Priority: high
Plan-Refs: plan.md
Todo-Refs: todo-1
[/DISPATCH]",
        )
        .expect("parse dispatch");

        assert_eq!(
            envelope,
            DispatchEnvelope {
                dispatch_id: "dsp_123".to_string(),
                objective: "Ship detached runtime".to_string(),
                scope: "tt-cli and tt-core".to_string(),
                constraints: "Reuse app-server websocket".to_string(),
                expected_output: "working routing loop".to_string(),
                completion_criteria: "status reflects dispatch".to_string(),
                priority: "high".to_string(),
                plan_refs: "plan.md".to_string(),
                todo_refs: "todo-1".to_string(),
            }
        );
    }

    #[test]
    fn parses_result_envelope() {
        let envelope = parse_result_envelope(
            "[RESULT]
Dispatch-ID: dsp_123
Status: done
Summary: implemented runtime
Work-Completed: added daemon
Artifacts: tt-cli
Open-Questions: none
Recommended-Next-Step: send to director
[/RESULT]",
        )
        .expect("parse result");

        assert_eq!(
            envelope,
            ResultEnvelope {
                dispatch_id: "dsp_123".to_string(),
                status: "done".to_string(),
                summary: "implemented runtime".to_string(),
                work_completed: "implemented runtime"
                    .replace("implemented runtime", "added daemon"),
                artifacts: "tt-cli".to_string(),
                open_questions: "none".to_string(),
                recommended_next_step: "send to director".to_string(),
            }
        );
    }
}
