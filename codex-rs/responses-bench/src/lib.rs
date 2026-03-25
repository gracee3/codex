use anyhow::Context;
use clap::Parser;
use eventsource_stream::Eventsource;
use futures::StreamExt;
use reqwest::Client;
use serde::Serialize;
use serde_json::Value;
use std::fmt;
use std::time::Duration;
use std::time::Instant;

mod compat;

#[derive(Debug, Clone, Parser)]
pub struct Args {
    /// Base URL for the OpenAI-compatible server, without the endpoint path.
    #[arg(long, default_value = "http://127.0.0.1:8000")]
    pub base_url: String,

    /// Endpoint path for the Responses API.
    #[arg(long, default_value = "/v1/responses")]
    pub endpoint: String,

    /// Bearer token for the API.
    #[arg(long, env = "OPENAI_API_KEY", default_value = "local")]
    pub api_key: String,

    /// Model name to send in each request.
    #[arg(long)]
    pub model: String,

    /// Number of requests to issue.
    #[arg(long, default_value_t = 200)]
    pub num_prompts: usize,

    /// Maximum number of in-flight requests.
    #[arg(long, default_value_t = 6)]
    pub max_concurrency: usize,

    /// Number of synthetic input tokens to generate.
    #[arg(long, default_value_t = 4096)]
    pub input_tokens: usize,

    /// Max output tokens requested from the server.
    #[arg(long, default_value_t = 512)]
    pub max_output_tokens: u32,

    /// Emit progress every N completed requests.
    #[arg(long, default_value_t = 10)]
    pub progress_every: usize,

    /// Use SSE streaming and report time-to-first-delta metrics.
    #[arg(long, default_value_t = false)]
    pub stream: bool,

    /// Run live Responses/tool compatibility smoke checks instead of throughput benchmarking.
    #[arg(long, default_value_t = false)]
    pub compat_smoke: bool,
}

#[derive(Debug, Clone)]
struct BenchmarkRequest {
    url: String,
    api_key: String,
    model: String,
    prompt: String,
    max_output_tokens: u32,
    stream: bool,
}

#[derive(Debug, Clone)]
struct RequestMetrics {
    latency: Duration,
    first_event_latency: Option<Duration>,
    first_delta_latency: Option<Duration>,
    input_tokens: u64,
    output_tokens: u64,
}

#[derive(Debug, Clone)]
struct RequestFailure {
    latency: Duration,
    message: String,
}

#[derive(Debug, Clone)]
enum RequestOutcome {
    Success(RequestMetrics),
    Failure(RequestFailure),
}

#[derive(Debug, Clone, Serialize)]
pub struct BenchmarkSummary {
    successful_requests: usize,
    failed_requests: usize,
    max_concurrency: usize,
    benchmark_duration_s: f64,
    request_throughput_req_s: f64,
    total_input_tokens: u64,
    total_output_tokens: u64,
    output_token_throughput_tok_s: f64,
    total_token_throughput_tok_s: f64,
    mean_latency_ms: f64,
    median_latency_ms: f64,
    p99_latency_ms: f64,
    mean_first_event_latency_ms: Option<f64>,
    median_first_event_latency_ms: Option<f64>,
    p99_first_event_latency_ms: Option<f64>,
    mean_first_delta_latency_ms: Option<f64>,
    median_first_delta_latency_ms: Option<f64>,
    p99_first_delta_latency_ms: Option<f64>,
}

pub async fn run(args: Args) -> anyhow::Result<()> {
    if args.compat_smoke {
        return compat::run_compat_smoke(&args).await;
    }

    validate_args(&args)?;
    let client = Client::builder()
        .build()
        .context("failed to build reqwest client")?;
    let request = BenchmarkRequest {
        url: format!("{}{}", args.base_url.trim_end_matches('/'), args.endpoint),
        api_key: args.api_key,
        model: args.model,
        prompt: synthetic_prompt(args.input_tokens),
        max_output_tokens: args.max_output_tokens,
        stream: args.stream,
    };

    let started_at = Instant::now();
    let mut stream = futures::stream::iter(0..args.num_prompts)
        .map(|_| issue_request(client.clone(), request.clone()))
        .buffer_unordered(args.max_concurrency);

    let mut outcomes = Vec::with_capacity(args.num_prompts);
    let mut completed = 0usize;
    while let Some(outcome) = stream.next().await {
        let outcome = outcome?;
        completed += 1;
        if completed.is_multiple_of(args.progress_every) || completed == args.num_prompts {
            eprintln!("{completed}/{} completed", args.num_prompts);
        }
        outcomes.push(outcome);
    }

    let summary = summarize(outcomes, started_at.elapsed(), args.max_concurrency);
    println!("{summary}");
    println!("{}", serde_json::to_string_pretty(&summary)?);
    Ok(())
}

fn validate_args(args: &Args) -> anyhow::Result<()> {
    anyhow::ensure!(args.num_prompts > 0, "--num-prompts must be greater than 0");
    anyhow::ensure!(
        args.max_concurrency > 0,
        "--max-concurrency must be greater than 0"
    );
    anyhow::ensure!(
        args.input_tokens > 0,
        "--input-tokens must be greater than 0"
    );
    anyhow::ensure!(
        args.max_output_tokens > 0,
        "--max-output-tokens must be greater than 0"
    );
    anyhow::ensure!(
        args.progress_every > 0,
        "--progress-every must be greater than 0"
    );
    Ok(())
}

async fn issue_request(
    client: Client,
    request: BenchmarkRequest,
) -> anyhow::Result<RequestOutcome> {
    if request.stream {
        benchmark_streaming_request(client, request).await
    } else {
        benchmark_non_streaming_request(client, request).await
    }
}

async fn benchmark_non_streaming_request(
    client: Client,
    request: BenchmarkRequest,
) -> anyhow::Result<RequestOutcome> {
    let started_at = Instant::now();
    let body = serde_json::json!({
        "model": request.model,
        "input": request.prompt,
        "max_output_tokens": request.max_output_tokens,
    });
    let response = client
        .post(&request.url)
        .bearer_auth(&request.api_key)
        .json(&body)
        .send()
        .await;

    let response = match response {
        Ok(response) => response,
        Err(err) => {
            return Ok(RequestOutcome::Failure(RequestFailure::new(
                started_at.elapsed(),
                err,
            )));
        }
    };

    let response = match response.error_for_status() {
        Ok(response) => response,
        Err(err) => {
            return Ok(RequestOutcome::Failure(RequestFailure::new(
                started_at.elapsed(),
                err,
            )));
        }
    };

    let payload: Value = match response.json().await {
        Ok(payload) => payload,
        Err(err) => {
            return Ok(RequestOutcome::Failure(RequestFailure::new(
                started_at.elapsed(),
                err,
            )));
        }
    };

    let usage = match usage_from_response_payload(&payload) {
        Ok(usage) => usage,
        Err(err) => {
            return Ok(RequestOutcome::Failure(RequestFailure::new(
                started_at.elapsed(),
                err,
            )));
        }
    };

    Ok(RequestOutcome::Success(RequestMetrics {
        latency: started_at.elapsed(),
        first_event_latency: None,
        first_delta_latency: None,
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
    }))
}

async fn benchmark_streaming_request(
    client: Client,
    request: BenchmarkRequest,
) -> anyhow::Result<RequestOutcome> {
    let started_at = Instant::now();
    let body = serde_json::json!({
        "model": request.model,
        "input": request.prompt,
        "max_output_tokens": request.max_output_tokens,
        "stream": true,
    });
    let response = client
        .post(&request.url)
        .bearer_auth(&request.api_key)
        .json(&body)
        .send()
        .await;

    let response = match response {
        Ok(response) => response,
        Err(err) => {
            return Ok(RequestOutcome::Failure(RequestFailure::new(
                started_at.elapsed(),
                err,
            )));
        }
    };

    let response = match response.error_for_status() {
        Ok(response) => response,
        Err(err) => {
            return Ok(RequestOutcome::Failure(RequestFailure::new(
                started_at.elapsed(),
                err,
            )));
        }
    };

    let mut stream = response.bytes_stream().eventsource();
    let mut first_event_latency = None;
    let mut first_delta_latency = None;
    let mut usage = None;
    while let Some(event) = stream.next().await {
        let event = match event {
            Ok(event) => event,
            Err(err) => {
                return Ok(RequestOutcome::Failure(RequestFailure::new(
                    started_at.elapsed(),
                    err,
                )));
            }
        };
        let elapsed = started_at.elapsed();
        if first_event_latency.is_none() {
            first_event_latency = Some(elapsed);
        }

        if event.data == "[DONE]" {
            continue;
        }

        let payload: Value = match serde_json::from_str(&event.data) {
            Ok(payload) => payload,
            Err(err) => {
                return Ok(RequestOutcome::Failure(RequestFailure::new(
                    started_at.elapsed(),
                    err,
                )));
            }
        };

        if first_delta_latency.is_none()
            && event_type(&payload).is_some_and(|event_type| event_type.ends_with(".delta"))
        {
            first_delta_latency = Some(elapsed);
        }

        if event_type(&payload) == Some("response.completed") {
            usage = Some(usage_from_stream_event(&payload)?);
        }
    }

    let usage = match usage {
        Some(usage) => usage,
        None => {
            return Ok(RequestOutcome::Failure(RequestFailure {
                latency: started_at.elapsed(),
                message: "stream ended without response.completed usage".to_string(),
            }));
        }
    };

    Ok(RequestOutcome::Success(RequestMetrics {
        latency: started_at.elapsed(),
        first_event_latency,
        first_delta_latency,
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
    }))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Usage {
    input_tokens: u64,
    output_tokens: u64,
}

impl RequestFailure {
    fn new(latency: Duration, err: impl fmt::Display) -> Self {
        Self {
            latency,
            message: err.to_string(),
        }
    }
}

impl fmt::Display for BenchmarkSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "=== RESPONSES BENCH SUMMARY ===")?;
        writeln!(f, "Successful requests: {}", self.successful_requests)?;
        writeln!(f, "Failed requests: {}", self.failed_requests)?;
        writeln!(f, "Maximum request concurrency: {}", self.max_concurrency)?;
        writeln!(
            f,
            "Benchmark duration (s): {:.2}",
            self.benchmark_duration_s
        )?;
        writeln!(
            f,
            "Request throughput (req/s): {:.3}",
            self.request_throughput_req_s
        )?;
        writeln!(f, "Total input tokens: {}", self.total_input_tokens)?;
        writeln!(f, "Total output tokens: {}", self.total_output_tokens)?;
        writeln!(
            f,
            "Output token throughput (tok/s): {:.2}",
            self.output_token_throughput_tok_s
        )?;
        writeln!(
            f,
            "Total token throughput (tok/s): {:.2}",
            self.total_token_throughput_tok_s
        )?;
        writeln!(f, "Mean latency (ms): {:.2}", self.mean_latency_ms)?;
        writeln!(f, "Median latency (ms): {:.2}", self.median_latency_ms)?;
        writeln!(f, "P99 latency (ms): {:.2}", self.p99_latency_ms)?;

        if let Some(mean) = self.mean_first_event_latency_ms {
            writeln!(f, "Mean first event latency (ms): {mean:.2}")?;
        }
        if let Some(median) = self.median_first_event_latency_ms {
            writeln!(f, "Median first event latency (ms): {median:.2}")?;
        }
        if let Some(p99) = self.p99_first_event_latency_ms {
            writeln!(f, "P99 first event latency (ms): {p99:.2}")?;
        }
        if let Some(mean) = self.mean_first_delta_latency_ms {
            writeln!(f, "Mean first delta latency (ms): {mean:.2}")?;
        }
        if let Some(median) = self.median_first_delta_latency_ms {
            writeln!(f, "Median first delta latency (ms): {median:.2}")?;
        }
        if let Some(p99) = self.p99_first_delta_latency_ms {
            writeln!(f, "P99 first delta latency (ms): {p99:.2}")?;
        }
        Ok(())
    }
}

fn summarize(
    outcomes: Vec<RequestOutcome>,
    duration: Duration,
    max_concurrency: usize,
) -> BenchmarkSummary {
    let mut successes = Vec::new();
    let mut failures = Vec::new();
    for outcome in outcomes {
        match outcome {
            RequestOutcome::Success(metrics) => successes.push(metrics),
            RequestOutcome::Failure(failure) => failures.push(failure),
        }
    }

    if !failures.is_empty() {
        eprintln!("Sample failures:");
        for failure in failures.iter().take(5) {
            eprintln!(
                "  {:.2} ms: {}",
                duration_ms(failure.latency),
                failure.message
            );
        }
    }

    let latency_ms = successes
        .iter()
        .map(|metrics| duration_ms(metrics.latency))
        .collect::<Vec<_>>();
    let first_event_latency_ms = successes
        .iter()
        .filter_map(|metrics| metrics.first_event_latency.map(duration_ms))
        .collect::<Vec<_>>();
    let first_delta_latency_ms = successes
        .iter()
        .filter_map(|metrics| metrics.first_delta_latency.map(duration_ms))
        .collect::<Vec<_>>();
    let total_input_tokens = successes
        .iter()
        .map(|metrics| metrics.input_tokens)
        .sum::<u64>();
    let total_output_tokens = successes
        .iter()
        .map(|metrics| metrics.output_tokens)
        .sum::<u64>();
    let benchmark_duration_s = duration.as_secs_f64();
    let successful_requests = successes.len();
    let failed_requests = failures.len();

    BenchmarkSummary {
        successful_requests,
        failed_requests,
        max_concurrency,
        benchmark_duration_s,
        request_throughput_req_s: successful_requests as f64 / benchmark_duration_s,
        total_input_tokens,
        total_output_tokens,
        output_token_throughput_tok_s: total_output_tokens as f64 / benchmark_duration_s,
        total_token_throughput_tok_s: (total_input_tokens + total_output_tokens) as f64
            / benchmark_duration_s,
        mean_latency_ms: mean_or_zero(&latency_ms),
        median_latency_ms: percentile_or_zero(&latency_ms, 50.0),
        p99_latency_ms: percentile_or_zero(&latency_ms, 99.0),
        mean_first_event_latency_ms: optional_mean(&first_event_latency_ms),
        median_first_event_latency_ms: optional_percentile(&first_event_latency_ms, 50.0),
        p99_first_event_latency_ms: optional_percentile(&first_event_latency_ms, 99.0),
        mean_first_delta_latency_ms: optional_mean(&first_delta_latency_ms),
        median_first_delta_latency_ms: optional_percentile(&first_delta_latency_ms, 50.0),
        p99_first_delta_latency_ms: optional_percentile(&first_delta_latency_ms, 99.0),
    }
}

fn synthetic_prompt(token_count: usize) -> String {
    (0..token_count)
        .map(|index| format!("tok{}", index % 2048))
        .collect::<Vec<_>>()
        .join(" ")
}

fn event_type(payload: &Value) -> Option<&str> {
    payload.get("type").and_then(Value::as_str)
}

fn usage_from_response_payload(payload: &Value) -> anyhow::Result<Usage> {
    usage_from_value(payload.get("usage"))
}

fn usage_from_stream_event(payload: &Value) -> anyhow::Result<Usage> {
    usage_from_value(
        payload
            .get("response")
            .and_then(|response| response.get("usage")),
    )
}

fn usage_from_value(value: Option<&Value>) -> anyhow::Result<Usage> {
    let value = value.context("missing usage")?;
    let input_tokens = value
        .get("input_tokens")
        .and_then(Value::as_u64)
        .context("missing usage.input_tokens")?;
    let output_tokens = value
        .get("output_tokens")
        .and_then(Value::as_u64)
        .context("missing usage.output_tokens")?;
    Ok(Usage {
        input_tokens,
        output_tokens,
    })
}

fn duration_ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

fn mean(values: &[f64]) -> f64 {
    values.iter().sum::<f64>() / values.len() as f64
}

fn mean_or_zero(values: &[f64]) -> f64 {
    optional_mean(values).unwrap_or(0.0)
}

fn optional_mean(values: &[f64]) -> Option<f64> {
    (!values.is_empty()).then(|| mean(values))
}

fn percentile(values: &[f64], percentile: f64) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let index = ((percentile / 100.0) * (sorted.len() - 1) as f64).round() as usize;
    sorted[index]
}

fn percentile_or_zero(values: &[f64], pct: f64) -> f64 {
    optional_percentile(values, pct).unwrap_or(0.0)
}

fn optional_percentile(values: &[f64], pct: f64) -> Option<f64> {
    (!values.is_empty()).then(|| percentile(values, pct))
}

#[cfg(test)]
mod tests {
    use super::Usage;
    use super::event_type;
    use super::percentile;
    use super::synthetic_prompt;
    use super::usage_from_response_payload;
    use super::usage_from_stream_event;
    use pretty_assertions::assert_eq;
    use serde_json::json;

    #[test]
    fn synthetic_prompt_generates_requested_token_count() {
        let prompt = synthetic_prompt(5);
        let tokens = prompt.split(' ').collect::<Vec<_>>();
        assert_eq!(tokens, vec!["tok0", "tok1", "tok2", "tok3", "tok4"]);
    }

    #[test]
    fn percentile_uses_rounded_rank() {
        let values = vec![10.0, 20.0, 30.0, 40.0, 50.0];
        assert_eq!(percentile(&values, 50.0), 30.0);
        assert_eq!(percentile(&values, 99.0), 50.0);
    }

    #[test]
    fn extracts_usage_from_non_streaming_payload() {
        let payload = json!({
            "usage": {
                "input_tokens": 73,
                "output_tokens": 83
            }
        });
        assert_eq!(
            usage_from_response_payload(&payload).unwrap(),
            Usage {
                input_tokens: 73,
                output_tokens: 83,
            }
        );
    }

    #[test]
    fn extracts_usage_from_stream_completion_event() {
        let payload = json!({
            "type": "response.completed",
            "response": {
                "usage": {
                    "input_tokens": 73,
                    "output_tokens": 32
                }
            }
        });
        assert_eq!(event_type(&payload), Some("response.completed"));
        assert_eq!(
            usage_from_stream_event(&payload).unwrap(),
            Usage {
                input_tokens: 73,
                output_tokens: 32,
            }
        );
    }
}
