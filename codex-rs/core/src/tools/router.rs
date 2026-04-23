use crate::function_tool::FunctionCallError;
use crate::sandboxing::SandboxPermissions;
use crate::session::session::Session;
use crate::session::turn_context::TurnContext;
use crate::tools::context::SharedTurnDiffTracker;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::registry::AnyToolResult;
use crate::tools::registry::ToolArgumentDiffConsumer;
use crate::tools::registry::ToolRegistry;
use crate::tools::spec::build_specs_with_discoverable_tools;
use codex_mcp::ToolInfo;
use codex_mcp::split_qualified_tool_name;
use codex_protocol::dynamic_tools::DynamicToolSpec;
use codex_protocol::models::LocalShellAction;
use codex_protocol::models::ResponseItem;
use codex_protocol::models::SearchToolCallParams;
use codex_protocol::models::ShellToolCallParams;
use codex_tools::ConfiguredToolSpec;
use codex_tools::DiscoverableTool;
use codex_tools::ToolName;
use codex_tools::ToolNamespace;
use codex_tools::ToolSpec;
use codex_tools::ToolsConfig;
use rmcp::model::Tool;
use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use tracing::instrument;

pub use crate::tools::context::ToolCallSource;

#[derive(Clone, Debug)]
pub struct ToolCall {
    pub tool_name: ToolName,
    pub call_id: String,
    pub payload: ToolPayload,
}

pub struct ToolRouter {
    registry: ToolRegistry,
    specs: Vec<ConfiguredToolSpec>,
    model_visible_specs: Vec<ToolSpec>,
}

pub(crate) struct ToolRouterParams<'a> {
    pub(crate) mcp_tools: Option<HashMap<String, Tool>>,
    pub(crate) parallel_mcp_tools: Option<HashSet<String>>,
    pub(crate) tool_namespaces: Option<HashMap<String, ToolNamespace>>,
    pub(crate) app_tools: Option<HashMap<String, ToolInfo>>,
    pub(crate) discoverable_tools: Option<Vec<DiscoverableTool>>,
    pub(crate) dynamic_tools: &'a [DynamicToolSpec],
}

pub(crate) struct McpToolRouterInputs {
    pub(crate) mcp_tools: HashMap<String, Tool>,
    pub(crate) parallel_mcp_tools: HashSet<String>,
    pub(crate) tool_namespaces: HashMap<String, ToolNamespace>,
}

pub(crate) fn map_mcp_tool_infos(mcp_tools: &HashMap<String, ToolInfo>) -> McpToolRouterInputs {
    McpToolRouterInputs {
        mcp_tools: mcp_tools
            .iter()
            .map(|(name, tool)| (name.clone(), tool.tool.clone()))
            .collect(),
        parallel_mcp_tools: mcp_tools
            .iter()
            .filter(|(_, tool)| tool.supports_parallel_tool_calls)
            .map(|(name, _)| name.clone())
            .collect(),
        tool_namespaces: mcp_tools
            .iter()
            .map(|(name, tool)| {
                (
                    name.clone(),
                    ToolNamespace {
                        name: tool.callable_namespace.clone(),
                        description: tool.server_instructions.clone(),
                    },
                )
            })
            .collect(),
    }
}

impl ToolRouter {
    pub fn from_config(config: &ToolsConfig, params: ToolRouterParams<'_>) -> Self {
        let ToolRouterParams {
            mcp_tools,
            parallel_mcp_tools,
            tool_namespaces,
            app_tools,
            discoverable_tools,
            dynamic_tools,
        } = params;
        let builder = build_specs_with_discoverable_tools(
            config,
            mcp_tools,
            parallel_mcp_tools,
            app_tools,
            tool_namespaces,
            discoverable_tools,
            dynamic_tools,
        );
        let (specs, registry) = builder.build();
        let model_visible_specs = specs
            .iter()
            .map(|configured_tool| configured_tool.spec.clone())
            .collect();

        Self {
            registry,
            specs,
            model_visible_specs,
        }
    }

    #[allow(dead_code)]
    pub fn specs(&self) -> Vec<ToolSpec> {
        self.specs
            .iter()
            .map(|config| config.spec.clone())
            .collect()
    }

    pub fn model_visible_specs(&self) -> Vec<ToolSpec> {
        self.model_visible_specs.clone()
    }

    #[allow(dead_code)]
    pub fn find_spec(&self, tool_name: &str) -> Option<ToolSpec> {
        self.specs
            .iter()
            .find(|config| config.name() == tool_name)
            .map(|config| config.spec.clone())
    }

    pub fn tool_supports_parallel(&self, tool_name: &str) -> bool {
        self.specs
            .iter()
            .filter(|config| config.supports_parallel_tool_calls)
            .any(|config| config.name() == tool_name)
    }

    pub(crate) fn create_diff_consumer(
        &self,
        tool_name: &ToolName,
    ) -> Option<Box<dyn ToolArgumentDiffConsumer>> {
        self.registry.create_diff_consumer(tool_name)
    }

    #[instrument(level = "trace", skip_all, err)]
    pub async fn build_tool_call(
        session: &Session,
        item: ResponseItem,
    ) -> Result<Option<ToolCall>, FunctionCallError> {
        match item {
            ResponseItem::FunctionCall {
                name,
                namespace,
                arguments,
                call_id,
                ..
            } => {
                if let Some((server, tool)) = parse_mcp_tool_name(session, &name, &namespace).await
                {
                    let tool_name = match namespace {
                        Some(namespace) if !name.starts_with(namespace.as_str()) => {
                            ToolName::plain(format!("{namespace}{name}"))
                        }
                        _ => ToolName::plain(name.clone()),
                    };
                    Ok(Some(ToolCall {
                        tool_name,
                        call_id,
                        payload: ToolPayload::Mcp {
                            server,
                            tool,
                            raw_arguments: arguments,
                        },
                    }))
                } else {
                    Ok(Some(ToolCall {
                        tool_name: ToolName::new(namespace, name),
                        call_id,
                        payload: ToolPayload::Function { arguments },
                    }))
                }
            }
            ResponseItem::ToolSearchCall {
                call_id: Some(call_id),
                execution,
                arguments,
                ..
            } if execution == "client" => {
                let arguments: SearchToolCallParams =
                    serde_json::from_value(arguments).map_err(|err| {
                        FunctionCallError::RespondToModel(format!(
                            "failed to parse tool_search arguments: {err}"
                        ))
                    })?;
                Ok(Some(ToolCall {
                    tool_name: ToolName::plain("tool_search"),
                    call_id,
                    payload: ToolPayload::ToolSearch { arguments },
                }))
            }
            ResponseItem::ToolSearchCall { .. } => Ok(None),
            ResponseItem::CustomToolCall {
                name,
                input,
                call_id,
                ..
            } => Ok(Some(ToolCall {
                tool_name: ToolName::plain(name),
                call_id,
                payload: ToolPayload::Custom { input },
            })),
            ResponseItem::LocalShellCall {
                id,
                call_id,
                action,
                ..
            } => {
                let call_id = call_id
                    .or(id)
                    .ok_or(FunctionCallError::MissingLocalShellCallId)?;

                match action {
                    LocalShellAction::Exec(exec) => {
                        let params = ShellToolCallParams {
                            command: exec.command,
                            workdir: exec.working_directory,
                            timeout_ms: exec.timeout_ms,
                            sandbox_permissions: Some(SandboxPermissions::UseDefault),
                            additional_permissions: None,
                            prefix_rule: None,
                            justification: None,
                        };
                        Ok(Some(ToolCall {
                            tool_name: ToolName::plain("local_shell"),
                            call_id,
                            payload: ToolPayload::LocalShell { params },
                        }))
                    }
                }
            }
            _ => Ok(None),
        }
    }

    #[instrument(level = "trace", skip_all, err)]
    pub async fn dispatch_tool_call(
        &self,
        session: Arc<Session>,
        turn: Arc<TurnContext>,
        cancellation_token: CancellationToken,
        tracker: SharedTurnDiffTracker,
        call: ToolCall,
        _source: ToolCallSource,
    ) -> Result<AnyToolResult, FunctionCallError> {
        let ToolCall {
            tool_name,
            call_id,
            payload,
        } = call;

        let invocation = ToolInvocation {
            session,
            turn,
            cancellation_token,
            tracker,
            call_id,
            tool_name,
            payload,
        };

        self.registry.dispatch_any(invocation).await
    }
}

async fn parse_mcp_tool_name(
    session: &Session,
    name: &str,
    namespace: &Option<String>,
) -> Option<(String, String)> {
    let tool_name = if let Some(namespace) = namespace {
        if name.starts_with(namespace.as_str()) {
            name.to_string()
        } else {
            format!("{namespace}{name}")
        }
    } else {
        name.to_string()
    };
    let all_tools = session
        .services
        .mcp_connection_manager
        .read()
        .await
        .list_all_tools()
        .await;
    if namespace.is_none()
        && let Some(tool) = all_tools
            .iter()
            .find(|(qualified_name, _)| qualified_name.as_str() == tool_name)
            .map(|(_, tool)| tool)
            .or_else(|| {
                all_tools.values().find(|tool| {
                    format!("{}{}", tool.callable_namespace, tool.callable_name) == tool_name
                })
            })
    {
        return Some((tool.server_name.clone(), tool.tool.name.to_string()));
    }
    all_tools
        .into_values()
        .find(|tool| {
            tool.canonical_tool_name()
                == match namespace {
                    Some(namespace) => {
                        let stripped = name.strip_prefix(namespace.as_str()).unwrap_or(name);
                        ToolName::new(Some(namespace.clone()), stripped.to_string())
                    }
                    None => {
                        if let Some((server_name, callable_name)) =
                            split_qualified_tool_name(&tool_name)
                        {
                            ToolName::namespaced(format!("mcp__{server_name}__"), callable_name)
                        } else {
                            ToolName::plain(tool_name.clone())
                        }
                    }
                }
        })
        .map(|tool| (tool.server_name, tool.tool.name.to_string()))
}
#[cfg(test)]
#[path = "router_tests.rs"]
mod tests;
