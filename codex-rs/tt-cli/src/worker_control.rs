use std::path::Path;

use anyhow::Context;
use anyhow::Result;
use codex_app_server_client::TypedRequestError;
use codex_app_server_protocol::CodexErrorInfo;
use codex_app_server_protocol::ThreadItem;
use codex_app_server_protocol::ThreadStatus;
use codex_app_server_protocol::Turn;
use codex_app_server_protocol::TurnError;
use codex_app_server_protocol::TurnStatus;
use codex_app_server_protocol::UserInput;
use codex_tt_core::DefaultView;
use codex_tt_core::TtState;
use codex_tt_core::WorkerBinding;
use codex_tt_core::WorkerKind;
use codex_tt_core::WorkerRecord;
use codex_tt_core::WorkspacePaths;
use codex_tt_core::append_log;
use codex_tt_core::default_log_event;
use codex_tt_core::save_state;
use serde_json::Value;

use crate::runtime::TtRuntime;
use crate::runtime::WorkerSessionStatus;

const DEFAULT_READ_TURNS: usize = 10;
const ACTIVE_TURN_MISMATCH_PREFIX: &str = "expected active turn id `";
const ACTIVE_TURN_MISMATCH_SEPARATOR: &str = "` but found `";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorkerReadWindow {
    Recent(usize),
    All,
}

impl Default for WorkerReadWindow {
    fn default() -> Self {
        Self::Recent(DEFAULT_READ_TURNS)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorkerSendMode {
    Start,
    Steer,
}

impl WorkerSendMode {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Steer => "steer",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorkerSendResult {
    pub(crate) mode: WorkerSendMode,
    pub(crate) turn_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorkerSummary {
    pub(crate) name: String,
    pub(crate) kind: WorkerKind,
    pub(crate) cwd: String,
    pub(crate) thread_id: Option<String>,
    pub(crate) thread_status: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorkerTranscriptLine {
    pub(crate) role: String,
    pub(crate) text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorkerReadResult {
    pub(crate) summary: WorkerSummary,
    pub(crate) selected_turn_count: usize,
    pub(crate) total_turn_count: usize,
    pub(crate) transcript: Vec<WorkerTranscriptLine>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ActiveTurnSteerRace {
    Missing,
    ExpectedTurnMismatch { actual_turn_id: String },
}

pub(crate) fn validate_worker_name(name: &str) -> Result<()> {
    if name.is_empty() {
        anyhow::bail!("worker name must not be empty");
    }
    if !name
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        anyhow::bail!("worker name must use only ASCII letters, numbers, `-`, or `_`");
    }
    Ok(())
}

pub(crate) async fn list_worker_summaries(
    runtime: Option<&mut TtRuntime>,
    state: &TtState,
) -> Result<Vec<WorkerSummary>> {
    let mut summaries = Vec::with_capacity(state.workers.len());
    if let Some(runtime) = runtime {
        for worker in &state.workers {
            let status = match worker.thread_id.as_deref() {
                Some(thread_id) => match runtime.read_thread(thread_id, false).await {
                    Ok(response) => Some(format_thread_status(&response.thread.status)),
                    Err(_) => Some("unavailable".to_string()),
                },
                None => None,
            };
            summaries.push(worker_summary(worker, status));
        }
    } else {
        for worker in &state.workers {
            summaries.push(worker_summary(worker, None));
        }
    }
    Ok(summaries)
}

pub(crate) async fn read_worker_history(
    runtime: &mut TtRuntime,
    state: &mut TtState,
    name: &str,
    window: WorkerReadWindow,
) -> Result<WorkerReadResult> {
    let worker = ensure_worker_thread(runtime, state, name).await?;
    let thread_id = worker
        .thread_id
        .as_deref()
        .with_context(|| format!("missing worker `{name}` thread id"))?;
    let response = read_thread_with_history_fallback(runtime, thread_id)
        .await
        .with_context(|| format!("read worker `{name}` history"))?;
    let selected_turns = selected_turns(&response.thread.turns, window);
    let mut transcript = Vec::new();
    for turn in selected_turns {
        transcript.push(WorkerTranscriptLine {
            role: "turn".to_string(),
            text: format!("{} [{}]", turn.id, format_turn_status(&turn.status)),
        });
        for item in &turn.items {
            if let Some(line) = render_item(item) {
                transcript.push(line)
            }
        }
    }

    let summary = WorkerSummary {
        thread_status: Some(format_thread_status(&response.thread.status)),
        ..worker_summary(&worker, None)
    };

    append_log(
        &runtime.workspace_paths,
        default_log_event(
            "worker-read",
            None,
            Some(name.to_string()),
            Some(thread_id.to_string()),
            Some(format!(
                "selected_turns={} total_turns={}",
                selected_turns.len(),
                response.thread.turns.len()
            )),
        ),
    )?;

    Ok(WorkerReadResult {
        summary,
        selected_turn_count: selected_turns.len(),
        total_turn_count: response.thread.turns.len(),
        transcript,
    })
}

pub(crate) async fn send_worker_prompt(
    runtime: &mut TtRuntime,
    state: &mut TtState,
    name: &str,
    message: String,
) -> Result<WorkerSendResult> {
    let worker = ensure_worker_thread(runtime, state, name).await?;
    let thread_id = worker
        .thread_id
        .as_deref()
        .with_context(|| format!("missing worker `{name}` thread id"))?;
    let response = read_thread_with_history_fallback(runtime, thread_id)
        .await
        .with_context(|| format!("read worker `{name}` state"))?;
    let mut active_turn_id = latest_in_progress_turn_id(&response.thread.turns);
    let mut refreshed_after_steer_error = false;
    let result = loop {
        let Some(steer_turn_id) = active_turn_id.clone() else {
            let response = runtime
                .start_turn(thread_id, message.clone())
                .await
                .with_context(|| format!("start worker `{name}` turn"))?;
            break WorkerSendResult {
                mode: WorkerSendMode::Start,
                turn_id: response.turn.id,
            };
        };

        match runtime
            .steer_turn(thread_id, &steer_turn_id, message.clone())
            .await
        {
            Ok(response) => {
                break WorkerSendResult {
                    mode: WorkerSendMode::Steer,
                    turn_id: response.turn_id,
                };
            }
            Err(err) if !refreshed_after_steer_error && steer_retry_requires_refresh(&err) => {
                let refreshed = read_thread_with_history_fallback(runtime, thread_id)
                    .await
                    .with_context(|| format!("refresh worker `{name}` state"))?;
                active_turn_id = latest_in_progress_turn_id(&refreshed.thread.turns);
                refreshed_after_steer_error = true;
                continue;
            }
            Err(err) => {
                if let Some(message) = non_steerable_message(&err) {
                    anyhow::bail!("worker `{name}` active turn is not steerable: {message}");
                }
                match active_turn_steer_race(&err) {
                    Some(ActiveTurnSteerRace::Missing) => {
                        active_turn_id = None;
                    }
                    Some(ActiveTurnSteerRace::ExpectedTurnMismatch { actual_turn_id })
                        if actual_turn_id != steer_turn_id =>
                    {
                        active_turn_id = Some(actual_turn_id);
                    }
                    Some(ActiveTurnSteerRace::ExpectedTurnMismatch { .. }) | None => {
                        return Err(err).with_context(|| format!("steer worker `{name}`"));
                    }
                }
            }
        }
    };

    append_log(
        &runtime.workspace_paths,
        default_log_event(
            "worker-send",
            None,
            Some(name.to_string()),
            Some(thread_id.to_string()),
            Some(format!(
                "mode={} turn_id={}",
                result.mode.as_str(),
                result.turn_id
            )),
        ),
    )?;

    Ok(result)
}

pub(crate) async fn adopt_worker(
    workspace_paths: &WorkspacePaths,
    runtime: &mut TtRuntime,
    state: &mut TtState,
    name: &str,
    thread_id: &str,
) -> Result<WorkerSummary> {
    validate_worker_name(name)?;
    if state.workers.iter().any(|worker| worker.name == name) {
        anyhow::bail!("worker `{name}` already exists");
    }

    let response = runtime
        .read_thread(thread_id, false)
        .await
        .with_context(|| format!("validate thread `{thread_id}`"))?;
    let thread_cwd = response.thread.cwd.as_path();
    if !path_is_within_workspace(workspace_paths.workspace_root(), thread_cwd) {
        anyhow::bail!(
            "thread `{thread_id}` cwd is outside this workspace: {}",
            thread_cwd.display()
        );
    }

    let worker = WorkerRecord {
        name: name.to_string(),
        kind: WorkerKind::Worker,
        cwd: thread_cwd.to_path_buf(),
        binding: WorkerBinding::Adopted,
        thread_id: Some(thread_id.to_string()),
        instruction_path: None,
    };
    state.workers.push(worker.clone());
    save_state(workspace_paths, state)?;
    append_log(
        workspace_paths,
        default_log_event(
            "worker-adopted",
            None,
            Some(name.to_string()),
            Some(thread_id.to_string()),
            Some(format!("cwd={}", worker.cwd.display())),
        ),
    )?;

    Ok(worker_summary(
        &worker,
        Some(format_thread_status(&response.thread.status)),
    ))
}

pub(crate) fn remove_worker(
    workspace_paths: &WorkspacePaths,
    state: &mut TtState,
    name: &str,
) -> Result<()> {
    let worker_index = state
        .workers
        .iter()
        .position(|worker| worker.name == name)
        .with_context(|| format!("unknown worker `{name}`"))?;
    let worker = &state.workers[worker_index];
    if worker.kind != WorkerKind::Worker {
        anyhow::bail!("cannot remove preset worker `{name}`");
    }

    let thread_id = worker.thread_id.clone();
    state.workers.remove(worker_index);
    if matches!(&state.default_view, DefaultView::Worker { name: current } if current == name) {
        state.default_view = DefaultView::Supervisor;
    }
    save_state(workspace_paths, state)?;
    append_log(
        workspace_paths,
        default_log_event(
            "worker-removed",
            None,
            Some(name.to_string()),
            thread_id,
            Some("worker unregistered".to_string()),
        ),
    )?;
    Ok(())
}

pub(crate) fn format_worker_list_for_cli(summaries: &[WorkerSummary]) -> String {
    if summaries.is_empty() {
        return "no workers registered".to_string();
    }

    let mut output = String::from("NAME\tKIND\tSTATUS\tCWD\tTHREAD_ID\n");
    for summary in summaries {
        output.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\n",
            summary.name,
            summary.kind.as_str(),
            summary.thread_status.as_deref().unwrap_or("<unknown>"),
            summary.cwd,
            summary.thread_id.as_deref().unwrap_or("<missing>")
        ));
    }
    output
}

pub(crate) fn format_worker_read_for_cli(result: &WorkerReadResult) -> String {
    let mut output = format!(
        "worker: {}\nkind: {}\ncwd: {}\nthread_id: {}\nthread_status: {}\nselected_turns: {}\ntotal_turns: {}\n\n",
        result.summary.name,
        result.summary.kind.as_str(),
        result.summary.cwd,
        result.summary.thread_id.as_deref().unwrap_or("<missing>"),
        result
            .summary
            .thread_status
            .as_deref()
            .unwrap_or("<unknown>"),
        result.selected_turn_count,
        result.total_turn_count
    );
    if result.transcript.is_empty() {
        output.push_str("transcript: <empty>\n");
        return output;
    }

    output.push_str("transcript:\n");
    for line in &result.transcript {
        output.push_str(&format_transcript_line(line));
        output.push('\n');
    }
    output
}

pub(crate) fn worker_summaries_to_json(summaries: &[WorkerSummary]) -> Value {
    Value::Array(
        summaries
            .iter()
            .map(|summary| {
                serde_json::json!({
                    "name": summary.name,
                    "kind": summary.kind.as_str(),
                    "cwd": summary.cwd,
                    "threadId": summary.thread_id,
                    "threadStatus": summary.thread_status,
                })
            })
            .collect(),
    )
}

pub(crate) fn worker_read_to_json(result: &WorkerReadResult) -> Value {
    serde_json::json!({
        "worker": {
            "name": result.summary.name,
            "kind": result.summary.kind.as_str(),
            "cwd": result.summary.cwd,
            "threadId": result.summary.thread_id,
            "threadStatus": result.summary.thread_status,
        },
        "selectedTurnCount": result.selected_turn_count,
        "totalTurnCount": result.total_turn_count,
        "transcript": result
            .transcript
            .iter()
            .map(|line| serde_json::json!({
                "role": line.role,
                "text": line.text,
            }))
            .collect::<Vec<_>>(),
    })
}

fn format_transcript_line(line: &WorkerTranscriptLine) -> String {
    match line.role.as_str() {
        "turn" => format!("  turn {}", line.text),
        "user" | "agent" => prefix_multiline(&format!("  {}: ", line.role), &line.text),
        _ => prefix_multiline("  note: ", &line.text),
    }
}

fn prefix_multiline(prefix: &str, text: &str) -> String {
    let mut lines = text.lines();
    let Some(first) = lines.next() else {
        return prefix.to_string();
    };
    let indent = " ".repeat(prefix.len());
    let mut output = format!("{prefix}{first}");
    for line in lines {
        output.push('\n');
        output.push_str(&indent);
        output.push_str(line);
    }
    output
}

async fn ensure_worker_thread(
    runtime: &mut TtRuntime,
    state: &mut TtState,
    name: &str,
) -> Result<WorkerRecord> {
    match runtime
        .ensure_named_worker_session(state, name.to_string())
        .await
        .with_context(|| format!("ensure worker `{name}` thread"))?
    {
        WorkerSessionStatus::Ready => {
            save_state(&runtime.workspace_paths, state)?;
        }
        WorkerSessionStatus::Unavailable { detail, .. } => anyhow::bail!(detail),
    }
    state
        .workers
        .iter()
        .find(|worker| worker.name == name)
        .cloned()
        .with_context(|| format!("unknown worker `{name}`"))
}

fn worker_summary(worker: &WorkerRecord, thread_status: Option<String>) -> WorkerSummary {
    WorkerSummary {
        name: worker.name.clone(),
        kind: worker.kind,
        cwd: worker.cwd.display().to_string(),
        thread_id: worker.thread_id.clone(),
        thread_status,
    }
}

async fn read_thread_with_history_fallback(
    runtime: &mut TtRuntime,
    thread_id: &str,
) -> Result<codex_app_server_protocol::ThreadReadResponse> {
    match runtime.read_thread(thread_id, true).await {
        Ok(response) => Ok(response),
        Err(err) if read_turns_not_available_yet(&err) => {
            runtime.read_thread(thread_id, false).await
        }
        Err(err) => Err(err),
    }
}

fn selected_turns(turns: &[Turn], window: WorkerReadWindow) -> &[Turn] {
    match window {
        WorkerReadWindow::All => turns,
        WorkerReadWindow::Recent(limit) => {
            let start = turns.len().saturating_sub(limit);
            &turns[start..]
        }
    }
}

fn render_item(item: &ThreadItem) -> Option<WorkerTranscriptLine> {
    match item {
        ThreadItem::UserMessage { content, .. } => Some(WorkerTranscriptLine {
            role: "user".to_string(),
            text: render_user_input(content),
        }),
        ThreadItem::AgentMessage { text, .. } => Some(WorkerTranscriptLine {
            role: "agent".to_string(),
            text: text.clone(),
        }),
        ThreadItem::HookPrompt { fragments, .. } => Some(note_line(format!(
            "[hook_prompt] fragments={}",
            fragments.len()
        ))),
        ThreadItem::Plan { text, .. } => Some(note_line(format!("[plan] {}", first_line(text)))),
        ThreadItem::Reasoning {
            summary, content, ..
        } => Some(note_line(format!(
            "[reasoning] summary_lines={} content_lines={}",
            summary.len(),
            content.len()
        ))),
        ThreadItem::CommandExecution {
            command,
            status,
            exit_code,
            ..
        } => Some(note_line(format!(
            "[command_execution] status={} exit_code={} command={}",
            format_command_status(status),
            exit_code
                .map(|code| code.to_string())
                .unwrap_or_else(|| "<none>".to_string()),
            command
        ))),
        ThreadItem::FileChange {
            changes, status, ..
        } => Some(note_line(format!(
            "[file_change] status={status:?} changes={}",
            changes.len()
        ))),
        ThreadItem::McpToolCall {
            server,
            tool,
            status,
            ..
        } => Some(note_line(format!(
            "[mcp_tool_call] status={status:?} server={server} tool={tool}"
        ))),
        ThreadItem::DynamicToolCall { tool, status, .. } => Some(note_line(format!(
            "[dynamic_tool_call] status={status:?} tool={tool}"
        ))),
        ThreadItem::CollabAgentToolCall { tool, status, .. } => Some(note_line(format!(
            "[collab_agent_tool_call] status={status:?} tool={tool:?}"
        ))),
        ThreadItem::WebSearch { query, .. } => Some(note_line(format!("[web_search] {query}"))),
        ThreadItem::ImageView { path, .. } => {
            Some(note_line(format!("[image_view] {}", path.display())))
        }
        ThreadItem::ImageGeneration { .. } => Some(note_line("[image_generation]".to_string())),
        ThreadItem::EnteredReviewMode { .. } => {
            Some(note_line("[entered_review_mode]".to_string()))
        }
        ThreadItem::ExitedReviewMode { .. } => Some(note_line("[exited_review_mode]".to_string())),
        ThreadItem::ContextCompaction { .. } => Some(note_line("[context_compaction]".to_string())),
    }
}

fn note_line(text: String) -> WorkerTranscriptLine {
    WorkerTranscriptLine {
        role: "note".to_string(),
        text,
    }
}

fn render_user_input(content: &[UserInput]) -> String {
    let rendered = content
        .iter()
        .map(|input| match input {
            UserInput::Text { text, .. } => text.clone(),
            UserInput::Image { url } => format!("[image] {url}"),
            UserInput::LocalImage { path } => format!("[local_image] {}", path.display()),
            UserInput::Skill { name, path } => format!("[skill] {name} ({})", path.display()),
            UserInput::Mention { name, path } => format!("[mention] {name} ({path})"),
        })
        .collect::<Vec<_>>();
    rendered.join("\n")
}

fn first_line(text: &str) -> &str {
    text.lines().next().unwrap_or(text)
}

fn format_thread_status(status: &ThreadStatus) -> String {
    match status {
        ThreadStatus::NotLoaded => "not_loaded".to_string(),
        ThreadStatus::Idle => "idle".to_string(),
        ThreadStatus::SystemError => "system_error".to_string(),
        ThreadStatus::Active { active_flags } => {
            if active_flags.is_empty() {
                "active".to_string()
            } else {
                let flags = active_flags
                    .iter()
                    .map(|flag| format!("{flag:?}").to_lowercase())
                    .collect::<Vec<_>>()
                    .join(",");
                format!("active({flags})")
            }
        }
    }
}

fn format_turn_status(status: &TurnStatus) -> &'static str {
    match status {
        TurnStatus::Completed => "completed",
        TurnStatus::Interrupted => "interrupted",
        TurnStatus::Failed => "failed",
        TurnStatus::InProgress => "in_progress",
    }
}

fn format_command_status(status: &codex_app_server_protocol::CommandExecutionStatus) -> String {
    format!("{status:?}").to_lowercase()
}

fn non_steerable_message(err: &TypedRequestError) -> Option<String> {
    let TypedRequestError::Server { source, .. } = err else {
        return None;
    };
    let data = source.data.clone()?;
    let turn_error: TurnError = serde_json::from_value(data).ok()?;
    match turn_error.codex_error_info {
        Some(CodexErrorInfo::ActiveTurnNotSteerable { .. }) => Some(turn_error.message),
        _ => None,
    }
}

fn active_turn_steer_race(err: &TypedRequestError) -> Option<ActiveTurnSteerRace> {
    let TypedRequestError::Server { method, source } = err else {
        return None;
    };
    if method != "turn/steer" {
        return None;
    }
    if source.message == "no active turn to steer" {
        return Some(ActiveTurnSteerRace::Missing);
    }
    let actual_turn_id = source
        .message
        .strip_prefix(ACTIVE_TURN_MISMATCH_PREFIX)?
        .split_once(ACTIVE_TURN_MISMATCH_SEPARATOR)?
        .1
        .strip_suffix('`')?
        .to_string();
    Some(ActiveTurnSteerRace::ExpectedTurnMismatch { actual_turn_id })
}

fn latest_in_progress_turn_id(turns: &[Turn]) -> Option<String> {
    turns
        .iter()
        .rev()
        .find(|turn| turn.status == TurnStatus::InProgress)
        .map(|turn| turn.id.clone())
}

fn steer_retry_requires_refresh(err: &TypedRequestError) -> bool {
    active_turn_steer_race(err).is_some() || non_steerable_message(err).is_some()
}

fn read_turns_not_available_yet(err: &anyhow::Error) -> bool {
    err.chain().any(|cause| {
        cause
            .to_string()
            .contains("includeTurns is unavailable before first user message")
    })
}

fn path_is_within_workspace(workspace_root: &Path, candidate: &Path) -> bool {
    let workspace_root = normalize_existing_path(workspace_root);
    let candidate = normalize_existing_path(candidate);
    candidate.starts_with(&workspace_root)
}

fn normalize_existing_path(path: &Path) -> std::path::PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use codex_app_server_client::TypedRequestError;
    use codex_app_server_protocol::CodexErrorInfo;
    use codex_app_server_protocol::JSONRPCErrorError;
    use codex_app_server_protocol::ThreadItem;
    use codex_app_server_protocol::Turn;
    use codex_app_server_protocol::TurnError;
    use codex_app_server_protocol::TurnStatus;
    use codex_app_server_protocol::UserInput;
    use pretty_assertions::assert_eq;

    use super::WorkerReadWindow;
    use super::non_steerable_message;
    use super::render_item;
    use super::selected_turns;

    #[test]
    fn selected_turns_defaults_to_last_window() {
        let turns = (0..12)
            .map(|index| Turn {
                id: format!("turn-{index}"),
                items: Vec::new(),
                status: TurnStatus::Completed,
                error: None,
                started_at: None,
                completed_at: None,
                duration_ms: None,
            })
            .collect::<Vec<_>>();

        let selected = selected_turns(&turns, WorkerReadWindow::default());
        assert_eq!(selected.len(), 10);
        assert_eq!(
            selected.first().map(|turn| turn.id.as_str()),
            Some("turn-2")
        );
        assert_eq!(
            selected.last().map(|turn| turn.id.as_str()),
            Some("turn-11")
        );
    }

    #[test]
    fn render_item_renders_user_text() {
        let line = render_item(&ThreadItem::UserMessage {
            id: "user-1".to_string(),
            content: vec![UserInput::Text {
                text: "hello".to_string(),
                text_elements: Vec::new(),
            }],
        })
        .unwrap_or_else(|| panic!("line"));

        assert_eq!(line.role, "user");
        assert_eq!(line.text, "hello");
    }

    #[test]
    fn non_steerable_message_extracts_structured_turn_error() {
        let err = TypedRequestError::Server {
            method: "turn/steer".to_string(),
            source: JSONRPCErrorError {
                code: -32600,
                message: "cannot steer a review turn".to_string(),
                data: Some(
                    serde_json::to_value(TurnError {
                        message: "cannot steer a review turn".to_string(),
                        codex_error_info: Some(CodexErrorInfo::ActiveTurnNotSteerable {
                            turn_kind: codex_app_server_protocol::NonSteerableTurnKind::Review,
                        }),
                        additional_details: None,
                    })
                    .unwrap_or_else(|err| panic!("serialize turn error: {err}")),
                ),
            },
        };

        assert_eq!(
            non_steerable_message(&err),
            Some("cannot steer a review turn".to_string())
        );
    }
}
