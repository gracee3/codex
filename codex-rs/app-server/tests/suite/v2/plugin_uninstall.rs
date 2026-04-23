use std::time::Duration;

use anyhow::Result;
use app_test_support::ChatGptAuthFixture;
use app_test_support::McpProcess;
use app_test_support::start_analytics_events_server;
use app_test_support::to_response;
use app_test_support::write_chatgpt_auth;
use codex_app_server_protocol::JSONRPCResponse;
use codex_app_server_protocol::PluginUninstallParams;
use codex_app_server_protocol::PluginUninstallResponse;
use codex_app_server_protocol::RequestId;
use codex_config::types::AuthCredentialsStoreMode;
use pretty_assertions::assert_eq;
use tempfile::TempDir;
use tokio::time::sleep;
use tokio::time::timeout;

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

#[tokio::test]
async fn plugin_uninstall_removes_plugin_cache_and_config_entry() -> Result<()> {
    let codex_home = TempDir::new()?;
    write_installed_plugin(&codex_home, "debug", "sample-plugin")?;
    std::fs::write(
        codex_home.path().join("config.toml"),
        r#"[features]
plugins = true

[plugins."sample-plugin@debug"]
enabled = true
"#,
    )?;

    let mut mcp = McpProcess::new(codex_home.path()).await?;
    timeout(DEFAULT_TIMEOUT, mcp.initialize()).await??;

    let params = PluginUninstallParams {
        plugin_id: "sample-plugin@debug".to_string(),
    };

    let request_id = mcp.send_plugin_uninstall_request(params.clone()).await?;
    let response: JSONRPCResponse = timeout(
        DEFAULT_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(request_id)),
    )
    .await??;
    let response: PluginUninstallResponse = to_response(response)?;
    assert_eq!(response, PluginUninstallResponse {});

    assert!(
        !codex_home
            .path()
            .join("plugins/cache/debug/sample-plugin")
            .exists()
    );
    let config = std::fs::read_to_string(codex_home.path().join("config.toml"))?;
    assert!(!config.contains(r#"[plugins."sample-plugin@debug"]"#));

    let request_id = mcp.send_plugin_uninstall_request(params).await?;
    let response: JSONRPCResponse = timeout(
        DEFAULT_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(request_id)),
    )
    .await??;
    let response: PluginUninstallResponse = to_response(response)?;
    assert_eq!(response, PluginUninstallResponse {});

    Ok(())
}

#[tokio::test]
async fn plugin_uninstall_tracks_analytics_event() -> Result<()> {
    let analytics_server = start_analytics_events_server().await?;
    let codex_home = TempDir::new()?;
    write_installed_plugin(&codex_home, "debug", "sample-plugin")?;
    std::fs::write(
        codex_home.path().join("config.toml"),
        format!(
            "chatgpt_base_url = \"{}\"\n\n[features]\nplugins = true\n\n[plugins.\"sample-plugin@debug\"]\nenabled = true\n",
            analytics_server.uri()
        ),
    )?;
    write_chatgpt_auth(
        codex_home.path(),
        ChatGptAuthFixture::new("chatgpt-token")
            .account_id("account-123")
            .chatgpt_user_id("user-123")
            .chatgpt_account_id("account-123"),
        AuthCredentialsStoreMode::File,
    )?;

    let mut mcp = McpProcess::new(codex_home.path()).await?;
    timeout(DEFAULT_TIMEOUT, mcp.initialize()).await??;

    let request_id = mcp
        .send_plugin_uninstall_request(PluginUninstallParams {
            plugin_id: "sample-plugin@debug".to_string(),
        })
        .await?;
    let response: JSONRPCResponse = timeout(
        DEFAULT_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(request_id)),
    )
    .await??;
    let response: PluginUninstallResponse = to_response(response)?;
    assert_eq!(response, PluginUninstallResponse {});
    // The standalone `codex-app-server` binary used by these tests hard-disables analytics in
    // this fork, so uninstall succeeds without posting telemetry.
    sleep(Duration::from_millis(100)).await;
    let requests = analytics_server
        .received_requests()
        .await
        .unwrap_or_default();
    assert!(
        requests
            .iter()
            .all(|request| request.url.path() != "/codex/analytics-events/events"),
        "unexpected analytics requests: {requests:?}"
    );
    Ok(())
}

fn write_installed_plugin(
    codex_home: &TempDir,
    marketplace_name: &str,
    plugin_name: &str,
) -> Result<()> {
    let plugin_root = codex_home
        .path()
        .join("plugins/cache")
        .join(marketplace_name)
        .join(plugin_name)
        .join("local/.codex-plugin");
    std::fs::create_dir_all(&plugin_root)?;
    std::fs::write(
        plugin_root.join("plugin.json"),
        format!(r#"{{"name":"{plugin_name}"}}"#),
    )?;
    Ok(())
}
