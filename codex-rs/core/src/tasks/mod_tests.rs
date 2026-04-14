use super::emit_turn_network_proxy_metric;
use codex_otel::MetricsError;
use codex_otel::SessionTelemetry;
use codex_protocol::ThreadId;
use codex_protocol::protocol::SessionSource;
use pretty_assertions::assert_eq;

fn test_session_telemetry() -> SessionTelemetry {
    SessionTelemetry::new(
        ThreadId::new(),
        "gpt-5.1",
        "gpt-5.1",
        /*account_id*/ None,
        /*account_email*/ None,
        /*auth_mode*/ None,
        "test_originator".to_string(),
        /*log_user_prompts*/ false,
        "tty".to_string(),
        SessionSource::Cli,
    )
}

#[test]
fn emit_turn_network_proxy_metric_does_not_enable_runtime_snapshots() {
    let session_telemetry = test_session_telemetry();

    emit_turn_network_proxy_metric(
        &session_telemetry,
        /*network_proxy_active*/ true,
        ("tmp_mem_enabled", "true"),
    );

    assert!(matches!(
        session_telemetry.snapshot_metrics(),
        Err(MetricsError::RuntimeSnapshotUnavailable)
    ));
    assert_eq!(session_telemetry.runtime_metrics_summary(), None);
}

#[test]
fn emit_turn_network_proxy_metric_keeps_runtime_snapshots_disabled_for_inactive_turns() {
    let session_telemetry = test_session_telemetry();

    emit_turn_network_proxy_metric(
        &session_telemetry,
        /*network_proxy_active*/ false,
        ("tmp_mem_enabled", "false"),
    );

    assert!(matches!(
        session_telemetry.snapshot_metrics(),
        Err(MetricsError::RuntimeSnapshotUnavailable)
    ));
    assert_eq!(session_telemetry.runtime_metrics_summary(), None);
}
