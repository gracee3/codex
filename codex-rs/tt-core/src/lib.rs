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
const DIRECTOR_ROLE_FILE: &str = "director.md";
const DEVELOPER_ROLE_FILE: &str = "developer.md";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Director,
    Developer,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
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
pub struct TtState {
    pub director_thread_id: Option<String>,
    pub developer_thread_id: Option<String>,
    pub runtime_running: bool,
    pub runtime_pid: Option<u32>,
    pub runtime_websocket_url: Option<String>,
    pub runtime_auth_token: Option<String>,
    pub last_runtime_started_at: Option<DateTime<Utc>>,
    pub auto_loop: bool,
    pub operator_pause: bool,
    pub default_view: Role,
    pub active_dispatch_id: Option<String>,
    pub pending_director_evaluation: bool,
    pub pending_developer_dispatch: bool,
}

impl Default for TtState {
    fn default() -> Self {
        Self {
            director_thread_id: None,
            developer_thread_id: None,
            runtime_running: false,
            runtime_pid: None,
            runtime_websocket_url: None,
            runtime_auth_token: None,
            last_runtime_started_at: None,
            auto_loop: false,
            operator_pause: false,
            default_view: Role::Director,
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
    pub thread_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ProjectPaths {
    repo_root: PathBuf,
}

impl ProjectPaths {
    pub fn new(repo_root: PathBuf) -> Self {
        Self { repo_root }
    }

    pub fn repo_root(&self) -> &Path {
        &self.repo_root
    }

    pub fn tt_dir(&self) -> PathBuf {
        self.repo_root.join(TT_DIR)
    }

    pub fn codex_home(&self) -> PathBuf {
        self.repo_root.join(CODEX_DIR)
    }

    pub fn runtime_dir(&self) -> PathBuf {
        self.codex_home().join(TT_RUNTIME_DIR)
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
            Role::Director => DIRECTOR_ROLE_FILE,
            Role::Developer => DEVELOPER_ROLE_FILE,
        })
    }
}

pub fn activate_tt_env(paths: &ProjectPaths) {
    let codex_home = paths.codex_home();
    let tt_home = paths.tt_dir();
    let repo_root = paths.repo_root().to_path_buf();

    // SAFETY: TT sets these process-scoped variables before spawning worker threads so all child
    // Codex processes resolve the same repo-local home and checked-in defaults layer.
    unsafe {
        std::env::set_var("CODEX_HOME", &codex_home);
        std::env::set_var("TT_HOME", &tt_home);
        std::env::set_var("TT_REPO_ROOT", &repo_root);
    }
}

pub fn ensure_project_artifacts(paths: &ProjectPaths) -> Result<()> {
    fs::create_dir_all(paths.roles_dir())
        .with_context(|| format!("create TT roles dir {}", paths.roles_dir().display()))?;
    fs::create_dir_all(paths.runtime_dir())
        .with_context(|| format!("create TT runtime dir {}", paths.runtime_dir().display()))?;

    migrate_legacy_runtime_artifacts(paths)?;

    write_if_missing(&paths.plan_path(), default_plan_markdown())?;
    write_if_missing(&paths.roster_path(), default_roster_markdown())?;
    write_if_missing(&paths.activate_path(), default_activate_script())?;
    write_if_missing(
        &paths.role_path(Role::Director),
        default_director_role_markdown(),
    )?;
    write_if_missing(
        &paths.role_path(Role::Developer),
        default_developer_role_markdown(),
    )?;

    if !paths.state_path().exists() {
        save_state(paths, &TtState::default())?;
    }
    if !paths.log_path().exists() {
        fs::write(paths.log_path(), b"")
            .with_context(|| format!("create TT log at {}", paths.log_path().display()))?;
    }

    Ok(())
}

pub fn load_state(paths: &ProjectPaths) -> Result<TtState> {
    let bytes = fs::read(paths.state_path())
        .with_context(|| format!("read TT state {}", paths.state_path().display()))?;
    serde_json::from_slice(&bytes)
        .with_context(|| format!("parse TT state {}", paths.state_path().display()))
}

pub fn save_state(paths: &ProjectPaths, state: &TtState) -> Result<()> {
    let json = serde_json::to_vec_pretty(state).context("serialize TT state")?;
    fs::write(paths.state_path(), json)
        .with_context(|| format!("write TT state {}", paths.state_path().display()))
}

pub fn append_log(paths: &ProjectPaths, event: LogEvent) -> Result<()> {
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
    thread_id: Option<String>,
    detail: Option<String>,
) -> LogEvent {
    LogEvent {
        ts: Utc::now(),
        event: event.into(),
        role,
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

fn migrate_legacy_runtime_artifacts(paths: &ProjectPaths) -> Result<()> {
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
    "# TT Plan\n\n## Current Objective\n- Establish the current objective.\n\n## Milestone\n- Record the active milestone.\n\n## Active Todos\n- Add current todos here.\n\n## Constraints\n- Capture repo, product, or runtime constraints here.\n\n## Open Questions\n- Record decisions that still need operator input.\n\n## Recent Decisions\n- Append resolved decisions as they happen.\n\n## Next Likely Dispatches\n- Note the next likely Director to Developer handoffs.\n"
}

fn default_roster_markdown() -> &'static str {
    "# TT Roster\n\n## Director\n- Own orchestration, prioritization, evaluation, and operator interaction.\n- Emit structured dispatches only.\n- Do not simulate implementation work.\n\n## Developer\n- Own scoped execution only.\n- Emit structured results only.\n- Do not self-dispatch or silently expand scope.\n"
}

fn default_activate_script() -> &'static str {
    r#"#!/usr/bin/env bash
if [ -n "${BASH_SOURCE[0]:-}" ]; then
  _tt_activate_source="${BASH_SOURCE[0]}"
else
  _tt_activate_source="$0"
fi
_tt_activate_dir="$(cd "$(dirname "${_tt_activate_source}")" && pwd -P)"
_tt_repo_root="$(cd "${_tt_activate_dir}/.." && pwd -P)"

export TT_REPO_ROOT="${_tt_repo_root}"
export TT_HOME="${_tt_repo_root}/.tt"
export CODEX_HOME="${_tt_repo_root}/.codex"

unset _tt_activate_source
unset _tt_activate_dir
unset _tt_repo_root
"#
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
        let paths = ProjectPaths::new(tempdir.path().to_path_buf());

        ensure_project_artifacts(&paths).expect("create artifacts");

        assert!(paths.plan_path().exists());
        assert!(paths.roster_path().exists());
        assert!(paths.activate_path().exists());
        assert!(paths.state_path().exists());
        assert!(paths.log_path().exists());
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
        assert_eq!(state, TtState::default());
    }

    #[test]
    fn appends_jsonl_log_events() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let paths = ProjectPaths::new(tempdir.path().to_path_buf());
        ensure_project_artifacts(&paths).expect("create artifacts");

        append_log(
            &paths,
            default_log_event(
                "thread-created",
                Some(Role::Director),
                Some("thr_123".to_string()),
                None,
            ),
        )
        .expect("append log");

        let contents = fs::read_to_string(paths.log_path()).expect("read log");
        assert!(contents.contains("\"event\":\"thread-created\""));
        assert!(contents.contains("\"role\":\"director\""));
    }

    #[test]
    fn migrates_legacy_runtime_artifacts_into_codex_home() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let paths = ProjectPaths::new(tempdir.path().to_path_buf());
        fs::create_dir_all(paths.tt_dir()).expect("create tt dir");
        fs::write(paths.tt_dir().join(STATE_FILE), b"{\"runtime_running\":false,\"runtime_pid\":null,\"runtime_websocket_url\":null,\"runtime_auth_token\":null,\"last_runtime_started_at\":null,\"auto_loop\":false,\"operator_pause\":false,\"default_view\":\"director\",\"active_dispatch_id\":null,\"pending_director_evaluation\":false,\"pending_developer_dispatch\":false,\"director_thread_id\":null,\"developer_thread_id\":null}").expect("write legacy state");
        fs::write(paths.tt_dir().join(LOG_FILE), b"legacy-log\n").expect("write legacy log");

        ensure_project_artifacts(&paths).expect("create artifacts");

        assert!(!paths.tt_dir().join(STATE_FILE).exists());
        assert!(!paths.tt_dir().join(LOG_FILE).exists());
        assert!(paths.state_path().exists());
        assert_eq!(
            fs::read_to_string(paths.log_path()).expect("read migrated log"),
            "legacy-log\n"
        );
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
                work_completed: "added daemon".to_string(),
                artifacts: "tt-cli".to_string(),
                open_questions: "none".to_string(),
                recommended_next_step: "send to director".to_string(),
            }
        );
    }
}
