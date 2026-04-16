use std::sync::Arc;

use crate::codex::make_session_and_context;
use crate::config::test_config;
use crate::test_support::construct_model_info_offline;
use crate::tools::context::ToolPayload;
use crate::tools::router::ToolRouterParams;
use codex_features::Features;
use codex_protocol::config_types::WebSearchMode;
use codex_protocol::config_types::WindowsSandboxLevel;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::SandboxPolicy;
use codex_protocol::protocol::SessionSource;
use codex_tools::ToolName;
use codex_tools::ToolsConfig;
use codex_tools::ToolsConfigParams;

use super::ToolRouter;

#[tokio::test]
async fn build_tool_call_uses_namespace_for_registry_name() -> anyhow::Result<()> {
    let (session, _) = make_session_and_context().await;
    let session = Arc::new(session);
    let tool_name = "create_event".to_string();

    let call = ToolRouter::build_tool_call(
        &session,
        ResponseItem::FunctionCall {
            id: None,
            name: tool_name.clone(),
            namespace: Some("mcp__codex_apps__calendar".to_string()),
            arguments: "{}".to_string(),
            call_id: "call-namespace".to_string(),
        },
    )
    .await?
    .expect("function_call should produce a tool call");

    assert_eq!(
        call.tool_name,
        ToolName::namespaced("mcp__codex_apps__calendar", tool_name)
    );
    assert_eq!(call.call_id, "call-namespace");
    match call.payload {
        ToolPayload::Function { arguments } => {
            assert_eq!(arguments, "{}");
        }
        other => panic!("expected function payload, got {other:?}"),
    }

    Ok(())
}

#[test]
fn tool_supports_parallel_uses_namespaced_display_name() {
    let config = test_config();
    let model_info = construct_model_info_offline("gpt-5-codex", &config);
    let available_models = Vec::new();
    let features = Features::with_defaults();
    let tools_config = ToolsConfig::new(&ToolsConfigParams {
        model_info: &model_info,
        available_models: &available_models,
        features: &features,
        image_generation_tool_auth_allowed: true,
        web_search_mode: Some(WebSearchMode::Cached),
        session_source: SessionSource::Cli,
        sandbox_policy: &SandboxPolicy::DangerFullAccess,
        windows_sandbox_level: WindowsSandboxLevel::Disabled,
    });
    let router = ToolRouter::from_config(
        &tools_config,
        ToolRouterParams {
            mcp_tools: Some(std::collections::HashMap::from([(
                "mcp__rmcp__sync".to_string(),
                rmcp::model::Tool {
                    name: "sync".to_string().into(),
                    title: None,
                    description: Some("Synchronize concurrent test calls.".into()),
                    input_schema: std::sync::Arc::new(rmcp::model::object(serde_json::json!({
                        "type": "object",
                        "properties": {},
                        "additionalProperties": false
                    }))),
                    output_schema: None,
                    annotations: None,
                    execution: None,
                    icons: None,
                    meta: None,
                },
            )])),
            parallel_mcp_tools: Some(std::collections::HashSet::from([
                "mcp__rmcp__sync".to_string()
            ])),
            tool_namespaces: Some(std::collections::HashMap::from([(
                "mcp__rmcp__sync".to_string(),
                codex_tools::ToolNamespace {
                    name: "mcp__rmcp__".to_string(),
                    description: None,
                },
            )])),
            app_tools: None,
            discoverable_tools: None,
            dynamic_tools: &[],
        },
    );

    assert!(router.tool_supports_parallel(&ToolName::namespaced("mcp__rmcp__", "sync")));
}
