use anyhow::Context;
use anyhow::Result;
use codex_app_server_protocol::DynamicToolCallOutputContentItem;
use codex_app_server_protocol::DynamicToolCallResponse;
use codex_app_server_protocol::DynamicToolSpec;
use codex_app_server_protocol::ServerRequest;
use codex_tt_core::TtState;

use crate::runtime::TtRuntime;
use crate::worker_control::WorkerReadWindow;
use crate::worker_control::WorkerSendMode;
use crate::worker_control::list_worker_summaries;
use crate::worker_control::read_worker_history;
use crate::worker_control::remove_worker;
use crate::worker_control::send_worker_prompt;
use crate::worker_control::worker_read_to_json;
use crate::worker_control::worker_summaries_to_json;

pub(crate) fn supervisor_dynamic_tools() -> Vec<DynamicToolSpec> {
    vec![
        DynamicToolSpec {
            name: "tt_worker_list".to_string(),
            description: "List TT workers with kind, cwd, thread id, and live thread status."
                .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
            defer_loading: false,
        },
        DynamicToolSpec {
            name: "tt_worker_read".to_string(),
            description:
                "Read one TT worker transcript. Provide `name`, optional `turns`, or `all=true`."
                    .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "turns": { "type": "integer", "minimum": 1 },
                    "all": { "type": "boolean" }
                },
                "required": ["name"],
                "additionalProperties": false
            }),
            defer_loading: false,
        },
        DynamicToolSpec {
            name: "tt_worker_send".to_string(),
            description:
                "Send a prompt into a TT worker thread. Starts a new turn or steers the active turn."
                    .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "message": { "type": "string" }
                },
                "required": ["name", "message"],
                "additionalProperties": false
            }),
            defer_loading: false,
        },
        DynamicToolSpec {
            name: "tt_worker_remove".to_string(),
            description:
                "Unregister a generic TT worker from TT state without deleting its thread or worktree."
                    .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string" }
                },
                "required": ["name"],
                "additionalProperties": false
            }),
            defer_loading: false,
        },
    ]
}

pub(crate) async fn handle_server_request(
    runtime: &mut TtRuntime,
    state: &mut TtState,
    request: ServerRequest,
) -> Result<bool> {
    let ServerRequest::DynamicToolCall { request_id, params } = request else {
        return Ok(false);
    };

    if state.supervisor_thread_id.as_deref() != Some(params.thread_id.as_str()) {
        return Ok(false);
    }

    let result = match params.tool.as_str() {
        "tt_worker_list" => {
            let summaries = list_worker_summaries(Some(runtime), state).await?;
            tool_success(worker_summaries_to_json(&summaries))
        }
        "tt_worker_read" => {
            let name = required_string_arg(&params.arguments, "name")?;
            let turns = optional_usize_arg(&params.arguments, "turns")?;
            let all = optional_bool_arg(&params.arguments, "all").unwrap_or(false);
            if all && turns.is_some() {
                tool_failure("`all` and `turns` cannot both be set")
            } else {
                let window = match (all, turns) {
                    (true, _) => WorkerReadWindow::All,
                    (false, Some(turns)) => WorkerReadWindow::Recent(turns),
                    (false, None) => WorkerReadWindow::default(),
                };
                match read_worker_history(runtime, state, &name, window).await {
                    Ok(result) => tool_success(worker_read_to_json(&result)),
                    Err(err) => tool_failure(err.to_string()),
                }
            }
        }
        "tt_worker_send" => {
            let name = required_string_arg(&params.arguments, "name")?;
            let message = required_string_arg(&params.arguments, "message")?;
            match send_worker_prompt(runtime, state, &name, message).await {
                Ok(result) => tool_success(serde_json::json!({
                    "mode": match result.mode {
                        WorkerSendMode::Start => "start",
                        WorkerSendMode::Steer => "steer",
                    },
                    "turnId": result.turn_id,
                })),
                Err(err) => tool_failure(err.to_string()),
            }
        }
        "tt_worker_remove" => {
            let name = required_string_arg(&params.arguments, "name")?;
            match remove_worker(&runtime.workspace_paths, state, &name) {
                Ok(()) => tool_success(serde_json::json!({
                    "removed": true,
                    "name": name,
                })),
                Err(err) => tool_failure(err.to_string()),
            }
        }
        _ => {
            runtime
                .reject_server_request_with_message(
                    request_id,
                    format!("unsupported TT supervisor tool `{}`", params.tool),
                )
                .await?;
            return Ok(true);
        }
    };

    runtime.resolve_server_request(request_id, result).await?;
    Ok(true)
}

fn tool_success(value: serde_json::Value) -> serde_json::Value {
    serde_json::to_value(DynamicToolCallResponse {
        content_items: vec![DynamicToolCallOutputContentItem::InputText {
            text: serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string()),
        }],
        success: true,
    })
    .unwrap_or_else(|err| panic!("serialize dynamic tool response: {err}"))
}

fn tool_failure(message: impl Into<String>) -> serde_json::Value {
    serde_json::to_value(DynamicToolCallResponse {
        content_items: vec![DynamicToolCallOutputContentItem::InputText {
            text: message.into(),
        }],
        success: false,
    })
    .unwrap_or_else(|err| panic!("serialize dynamic tool response: {err}"))
}

fn required_string_arg(arguments: &serde_json::Value, key: &str) -> Result<String> {
    arguments
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .with_context(|| format!("missing string argument `{key}`"))
}

fn optional_bool_arg(arguments: &serde_json::Value, key: &str) -> Option<bool> {
    arguments.get(key).and_then(serde_json::Value::as_bool)
}

fn optional_usize_arg(arguments: &serde_json::Value, key: &str) -> Result<Option<usize>> {
    let Some(value) = arguments.get(key) else {
        return Ok(None);
    };
    let turns = value
        .as_u64()
        .with_context(|| format!("argument `{key}` must be a positive integer"))?;
    usize::try_from(turns)
        .map(Some)
        .with_context(|| format!("argument `{key}` is too large"))
}
