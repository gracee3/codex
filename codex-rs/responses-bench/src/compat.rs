use crate::Args;
use anyhow::Context;
use anyhow::Result;
use reqwest::Client;
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, Serialize)]
struct CompatReport {
    model: String,
    endpoint: String,
    function_tool_non_stream: ProbeResult,
    function_tool_stream_non_empty_args: ProbeResult,
    function_tool_stream_zero_arg_shape: ProbeResult,
    stateless_function_call_continuation: ProbeResult,
    previous_response_id_continuation: ProbeResult,
    developer_role_message: ProbeResult,
    system_role_message: ProbeResult,
    unsupported_tools: Vec<ToolProbeResult>,
    unsupported_output_items: Vec<OutputItemProbeResult>,
}

#[derive(Debug, Clone, Serialize)]
struct ProbeResult {
    ok: bool,
    detail: String,
}

#[derive(Debug, Clone, Serialize)]
struct ToolProbeResult {
    tool_type: String,
    rejected: bool,
    status: u16,
    detail: String,
}

#[derive(Debug, Clone, Serialize)]
struct OutputItemProbeResult {
    item_type: String,
    accepted: bool,
    status: u16,
    detail: String,
}

#[derive(Debug)]
struct ResponseEnvelope {
    status: u16,
    body: Value,
}

pub(crate) async fn run_compat_smoke(args: &Args) -> Result<()> {
    let client = Client::builder()
        .build()
        .context("failed to build reqwest client")?;
    let url = format!("{}{}", args.base_url.trim_end_matches('/'), args.endpoint);

    let function_tool_non_stream =
        probe_function_tool_non_stream(&client, &url, &args.api_key, &args.model).await?;
    let function_tool_stream_non_empty_args =
        probe_stream_function_call_args(&client, &url, &args.api_key, &args.model, false).await?;
    let function_tool_stream_zero_arg_shape =
        probe_stream_function_call_args(&client, &url, &args.api_key, &args.model, true).await?;
    let stateless_function_call_continuation =
        probe_stateless_continuation(&client, &url, &args.api_key, &args.model).await?;
    let previous_response_id_continuation =
        probe_previous_response_id_continuation(&client, &url, &args.api_key, &args.model).await?;
    let developer_role_message =
        probe_message_role(&client, &url, &args.api_key, &args.model, "developer").await?;
    let system_role_message =
        probe_message_role(&client, &url, &args.api_key, &args.model, "system").await?;
    let unsupported_tools =
        probe_unsupported_tools(&client, &url, &args.api_key, &args.model).await?;
    let unsupported_output_items =
        probe_unsupported_output_items(&client, &url, &args.api_key, &args.model).await?;

    let report = CompatReport {
        model: args.model.clone(),
        endpoint: url,
        function_tool_non_stream,
        function_tool_stream_non_empty_args,
        function_tool_stream_zero_arg_shape,
        stateless_function_call_continuation,
        previous_response_id_continuation,
        developer_role_message,
        system_role_message,
        unsupported_tools,
        unsupported_output_items,
    };

    print_report(&report)?;

    let baseline_ok = report.function_tool_non_stream.ok
        && report.function_tool_stream_non_empty_args.ok
        && report.stateless_function_call_continuation.ok
        && report.developer_role_message.ok
        && report.system_role_message.ok;

    anyhow::ensure!(
        baseline_ok,
        "responses compatibility smoke failed baseline checks"
    );
    Ok(())
}

async fn probe_function_tool_non_stream(
    client: &Client,
    url: &str,
    api_key: &str,
    model: &str,
) -> Result<ProbeResult> {
    let payload = serde_json::json!({
        "model": model,
        "stream": false,
        "input": "Call echo with {\"text\":\"hello\"}. Do not answer in plain text.",
        "tools": [{
            "type": "function",
            "name": "echo",
            "description": "Echo text",
            "parameters": {
                "type": "object",
                "properties": { "text": { "type": "string" } },
                "required": ["text"],
                "additionalProperties": false
            }
        }]
    });
    let response = post_json(client, url, api_key, payload).await?;
    let arguments = first_function_call_arguments(&response.body);
    Ok(ProbeResult {
        ok: response.status == 200 && arguments.as_deref() == Some("{\"text\":\"hello\"}"),
        detail: format!("status={}, arguments={arguments:?}", response.status),
    })
}

async fn probe_stream_function_call_args(
    client: &Client,
    url: &str,
    api_key: &str,
    model: &str,
    zero_args: bool,
) -> Result<ProbeResult> {
    let (input, tools, expected_arguments) = if zero_args {
        (
            "Call noop with an empty object. Do not answer in plain text.",
            serde_json::json!([{
                "type": "function",
                "name": "noop",
                "description": "No-op",
                "parameters": {
                    "type": "object",
                    "properties": {},
                    "additionalProperties": false
                }
            }]),
            "{}",
        )
    } else {
        (
            "Call echo with {\"text\":\"hello\"}. Do not answer in plain text.",
            serde_json::json!([{
                "type": "function",
                "name": "echo",
                "description": "Echo text",
                "parameters": {
                    "type": "object",
                    "properties": { "text": { "type": "string" } },
                    "required": ["text"],
                    "additionalProperties": false
                }
            }]),
            "{\"text\":\"hello\"}",
        )
    };

    let payload = serde_json::json!({
        "model": model,
        "stream": true,
        "input": input,
        "tools": tools,
    });
    let events = stream_events(client, url, api_key, payload).await?;
    let observed = function_call_arguments_done_from_events(&events);
    Ok(ProbeResult {
        ok: observed.as_deref() == Some(expected_arguments),
        detail: format!("observed_arguments={observed:?}"),
    })
}

async fn probe_stateless_continuation(
    client: &Client,
    url: &str,
    api_key: &str,
    model: &str,
) -> Result<ProbeResult> {
    let payload = serde_json::json!({
        "model": model,
        "stream": false,
        "input": [
            {
                "type": "message",
                "role": "user",
                "content": [{ "type": "input_text", "text": "You already called noop and got ok. Reply with DONE." }]
            },
            {
                "type": "function_call",
                "call_id": "call_1",
                "name": "noop",
                "arguments": "{}"
            },
            {
                "type": "function_call_output",
                "call_id": "call_1",
                "output": "ok"
            }
        ]
    });
    let response = post_json(client, url, api_key, payload).await?;
    let text = first_output_text(&response.body);
    Ok(ProbeResult {
        ok: response.status == 200 && text.as_deref().is_some_and(|value| value.contains("DONE")),
        detail: format!("status={}, text={text:?}", response.status),
    })
}

async fn probe_previous_response_id_continuation(
    client: &Client,
    url: &str,
    api_key: &str,
    model: &str,
) -> Result<ProbeResult> {
    let initial_payload = serde_json::json!({
        "model": model,
        "stream": false,
        "input": "Call echo with {\"text\":\"hello\"}. Do not answer in plain text.",
        "tools": [{
            "type": "function",
            "name": "echo",
            "description": "Echo text",
            "parameters": {
                "type": "object",
                "properties": { "text": { "type": "string" } },
                "required": ["text"],
                "additionalProperties": false
            }
        }]
    });
    let initial_response = post_json(client, url, api_key, initial_payload).await?;
    let response_id = initial_response
        .body
        .get("id")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let call_id = first_function_call_call_id(&initial_response.body);
    let Some(response_id) = response_id else {
        return Ok(ProbeResult {
            ok: false,
            detail: "missing initial response id".to_string(),
        });
    };
    let Some(call_id) = call_id else {
        return Ok(ProbeResult {
            ok: false,
            detail: "missing initial function call id".to_string(),
        });
    };
    let continuation_payload = serde_json::json!({
        "model": model,
        "previous_response_id": response_id,
        "stream": false,
        "input": [{
            "type": "function_call_output",
            "call_id": call_id,
            "output": "ok"
        }]
    });
    let continuation_response = post_json(client, url, api_key, continuation_payload).await?;
    Ok(ProbeResult {
        ok: continuation_response.status == 200,
        detail: format!(
            "status={}, error={:?}",
            continuation_response.status,
            error_message(&continuation_response.body)
        ),
    })
}

async fn probe_message_role(
    client: &Client,
    url: &str,
    api_key: &str,
    model: &str,
    role: &str,
) -> Result<ProbeResult> {
    let payload = serde_json::json!({
        "model": model,
        "stream": false,
        "input": [
            {
                "type": "message",
                "role": role,
                "content": [{ "type": "input_text", "text": "You are helpful." }]
            },
            {
                "type": "message",
                "role": "user",
                "content": [{ "type": "input_text", "text": "Reply READY" }]
            }
        ]
    });
    let response = post_json(client, url, api_key, payload).await?;
    let text = first_output_text(&response.body);
    Ok(ProbeResult {
        ok: response.status == 200 && text.as_deref().is_some_and(|value| value.contains("READY")),
        detail: format!("status={}, text={text:?}", response.status),
    })
}

async fn probe_unsupported_tools(
    client: &Client,
    url: &str,
    api_key: &str,
    model: &str,
) -> Result<Vec<ToolProbeResult>> {
    let probes = [
        ("web_search", serde_json::json!([{ "type": "web_search" }])),
        (
            "image_generation",
            serde_json::json!([{ "type": "image_generation", "output_format": "png" }]),
        ),
        (
            "custom",
            serde_json::json!([{
                "type": "custom",
                "name": "apply_patch",
                "description": "Patch files",
                "format": {
                    "type": "grammar",
                    "syntax": "lark",
                    "definition": "start: /(.|\\n)*/"
                }
            }]),
        ),
        (
            "local_shell",
            serde_json::json!([{ "type": "local_shell" }]),
        ),
        (
            "tool_search",
            serde_json::json!([{
                "type": "tool_search",
                "execution": "deferred",
                "description": "Search tools",
                "parameters": {
                    "type": "object",
                    "properties": {},
                    "additionalProperties": false
                }
            }]),
        ),
        (
            "apply_patch",
            serde_json::json!([{ "type": "apply_patch" }]),
        ),
        ("shell", serde_json::json!([{ "type": "shell" }])),
    ];

    let mut results = Vec::with_capacity(probes.len());
    for (tool_type, tools) in probes {
        let payload = serde_json::json!({
            "model": model,
            "stream": false,
            "input": "Reply with READY",
            "tools": tools,
        });
        let response = post_json(client, url, api_key, payload).await?;
        results.push(ToolProbeResult {
            tool_type: tool_type.to_string(),
            rejected: response.status >= 400,
            status: response.status,
            detail: error_message(&response.body).unwrap_or_else(|| response.body.to_string()),
        });
    }
    Ok(results)
}

async fn probe_unsupported_output_items(
    client: &Client,
    url: &str,
    api_key: &str,
    model: &str,
) -> Result<Vec<OutputItemProbeResult>> {
    let probes = [
        (
            "custom_tool_call_output",
            serde_json::json!([{
                "type": "custom_tool_call_output",
                "call_id": "call_1",
                "output": "ok"
            }]),
        ),
        (
            "tool_search_output",
            serde_json::json!([{
                "type": "tool_search_output",
                "call_id": "call_1",
                "status": "completed",
                "execution": "deferred",
                "tools": []
            }]),
        ),
    ];

    let mut results = Vec::with_capacity(probes.len());
    for (item_type, input) in probes {
        let payload = serde_json::json!({
            "model": model,
            "stream": false,
            "input": input,
        });
        let response = post_json(client, url, api_key, payload).await?;
        results.push(OutputItemProbeResult {
            item_type: item_type.to_string(),
            accepted: response.status == 200,
            status: response.status,
            detail: error_message(&response.body).unwrap_or_else(|| response.body.to_string()),
        });
    }
    Ok(results)
}

async fn post_json(
    client: &Client,
    url: &str,
    api_key: &str,
    payload: Value,
) -> Result<ResponseEnvelope> {
    let response = client
        .post(url)
        .bearer_auth(api_key)
        .json(&payload)
        .send()
        .await
        .context("request failed")?;
    let status = response.status().as_u16();
    let body = response
        .json::<Value>()
        .await
        .context("failed to decode JSON response")?;
    Ok(ResponseEnvelope { status, body })
}

async fn stream_events(
    client: &Client,
    url: &str,
    api_key: &str,
    payload: Value,
) -> Result<Vec<Value>> {
    use eventsource_stream::Eventsource;
    use futures::StreamExt;

    let response = client
        .post(url)
        .bearer_auth(api_key)
        .json(&payload)
        .send()
        .await
        .context("stream request failed")?
        .error_for_status()
        .context("stream request returned error status")?;

    let mut stream = response.bytes_stream().eventsource();
    let mut events = Vec::new();
    while let Some(event) = stream.next().await {
        let event = event.context("failed to decode SSE event")?;
        if event.data == "[DONE]" {
            continue;
        }
        events.push(serde_json::from_str(&event.data).context("failed to parse SSE JSON payload")?);
    }
    Ok(events)
}

fn first_function_call_arguments(payload: &Value) -> Option<String> {
    payload.get("output")?.as_array()?.iter().find_map(|item| {
        (item.get("type").and_then(Value::as_str) == Some("function_call"))
            .then(|| {
                item.get("arguments")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned)
            })
            .flatten()
    })
}

fn first_function_call_call_id(payload: &Value) -> Option<String> {
    payload.get("output")?.as_array()?.iter().find_map(|item| {
        (item.get("type").and_then(Value::as_str) == Some("function_call"))
            .then(|| {
                item.get("call_id")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned)
            })
            .flatten()
    })
}

fn first_output_text(payload: &Value) -> Option<String> {
    payload.get("output")?.as_array()?.iter().find_map(|item| {
        (item.get("type").and_then(Value::as_str) == Some("message")).then(|| {
            item.get("content")
                .and_then(Value::as_array)
                .and_then(|content| {
                    content.iter().find_map(|part| {
                        (part.get("type").and_then(Value::as_str) == Some("output_text"))
                            .then(|| {
                                part.get("text")
                                    .and_then(Value::as_str)
                                    .map(ToOwned::to_owned)
                            })
                            .flatten()
                    })
                })
        })?
    })
}

fn function_call_arguments_done_from_events(events: &[Value]) -> Option<String> {
    events.iter().find_map(|payload| {
        (payload.get("type").and_then(Value::as_str)
            == Some("response.function_call_arguments.done"))
        .then(|| {
            payload
                .get("arguments")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
        .flatten()
    })
}

fn error_message(payload: &Value) -> Option<String> {
    payload
        .get("error")
        .and_then(|error| error.get("message"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

fn print_report(report: &CompatReport) -> Result<()> {
    println!("=== RESPONSES COMPAT SMOKE ===");
    println!("Model: {}", report.model);
    println!("Endpoint: {}", report.endpoint);
    print_probe("function tool non-stream", &report.function_tool_non_stream);
    print_probe(
        "function tool stream non-empty args",
        &report.function_tool_stream_non_empty_args,
    );
    print_probe(
        "function tool stream zero-arg shape",
        &report.function_tool_stream_zero_arg_shape,
    );
    print_probe(
        "stateless function call continuation",
        &report.stateless_function_call_continuation,
    );
    print_probe(
        "previous_response_id continuation",
        &report.previous_response_id_continuation,
    );
    print_probe("developer role message", &report.developer_role_message);
    print_probe("system role message", &report.system_role_message);

    println!("Unsupported tools:");
    for probe in &report.unsupported_tools {
        println!(
            "  - {}: rejected={} status={} detail={}",
            probe.tool_type, probe.rejected, probe.status, probe.detail
        );
    }

    println!("Unsupported output items:");
    for probe in &report.unsupported_output_items {
        println!(
            "  - {}: accepted={} status={} detail={}",
            probe.item_type, probe.accepted, probe.status, probe.detail
        );
    }

    println!("{}", serde_json::to_string_pretty(report)?);
    Ok(())
}

fn print_probe(label: &str, probe: &ProbeResult) {
    println!("{}: ok={} detail={}", label, probe.ok, probe.detail);
}

#[cfg(test)]
mod tests {
    use super::function_call_arguments_done_from_events;
    use pretty_assertions::assert_eq;
    use serde_json::json;

    #[test]
    fn extracts_function_call_arguments_done_from_events() {
        let events = vec![
            json!({
                "type": "response.created",
            }),
            json!({
                "type": "response.function_call_arguments.done",
                "arguments": "{\"text\":\"hello\"}",
            }),
        ];

        assert_eq!(
            function_call_arguments_done_from_events(&events),
            Some("{\"text\":\"hello\"}".to_string())
        );
    }
}
