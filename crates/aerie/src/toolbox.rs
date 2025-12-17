use itertools::Itertools;
use rig::{agent::AgentBuilderSimple, completion::CompletionModel, tool::ToolSet as RigToolSet};
use std::{borrow::Cow, collections::BTreeMap, sync::Arc};
use tokio::process::Command;

use rmcp::{
    Peer, RoleClient, ServiceExt as _,
    model::{ClientCapabilities, ClientInfo, Implementation, Tool},
    service::RunningService,
    transport::{
        ConfigureCommandExt, StreamableHttpClientTransport, TokioChildProcess,
        streamable_http_client::StreamableHttpClientTransportConfig,
    },
};

use super::config::{ToolSelector, ToolSpec};

#[derive(Clone)]
pub enum ToolProvider {
    MCP { client: McpClient, tools: Vec<Tool> },
}

#[derive(Clone)]
pub enum McpClient {
    Stdio(Arc<RunningService<RoleClient, ()>>),
    HTTP(Arc<RunningService<RoleClient, rmcp::model::InitializeRequestParam>>),
}

impl McpClient {
    pub fn peer(&self) -> &Peer<RoleClient> {
        match self {
            McpClient::Stdio(client) => client.peer(),
            McpClient::HTTP(client) => client.peer(),
        }
    }
}

impl ToolProvider {
    pub fn get_tools(&self, _provider: &str, selector: impl Fn(&Tool) -> bool) -> RigToolSet {
        let mut result = RigToolSet::default();
        match self {
            ToolProvider::MCP { client, tools } => {
                for tool in tools {
                    if selector(tool) {
                        // namespace tools with provider name
                        let tool = tool.clone();
                        // tool.name = Cow::Owned(format!("{provider}::{}", tool.name.as_ref()));

                        result.add_tool(rig::tool::rmcp::McpTool::from_mcp_server(
                            tool,
                            client.peer().clone(),
                        ));
                    }
                }
            }
        }

        result
    }

    pub fn select_tools<M: CompletionModel>(
        &self,
        agent: AgentBuilderSimple<M>,
        _provider: &str,
        selector: impl Fn(&Tool) -> bool,
    ) -> AgentBuilderSimple<M> {
        match self {
            ToolProvider::MCP { client, tools } => {
                let selection = tools
                    .iter()
                    .filter(|it| selector(it))
                    .cloned()
                    // .map(|mut tool| {
                    //     tool.name = Cow::Owned(format!("{provider}::{}", tool.name.as_ref()));
                    //     tool
                    // })
                    .collect_vec();
                agent.rmcp_tools(selection, client.peer().clone())
            }
        }
    }

    pub async fn from_spec(spec: &ToolSpec) -> anyhow::Result<Self> {
        let client = match spec {
            ToolSpec::Stdio {
                dir,
                command,
                args,
                env,
                ..
            } => {
                let client = ()
                    .serve(TokioChildProcess::new(Command::new(command).configure(
                        |cmd| {
                            let cmd = args.iter().fold(cmd, |cmd, arg| cmd.arg(arg));
                            // cmd.stderr(Stdio::null());
                            if let Some(cwd) = dir {
                                cmd.current_dir(cwd);
                            }

                            for (k, v) in env.split("\n").filter_map(|s| s.split_once('=')) {
                                cmd.env(k, v);
                            }
                        },
                    ))?)
                    .await
                    .inspect_err(|e| {
                        tracing::error!("client error: {:?}", e);
                    })?;

                McpClient::Stdio(Arc::new(client))
            }
            ToolSpec::HTTP { uri, auth_var, .. } => {
                const API_KEY: &str = "{{api_key}}";
                let auth_var = auth_var.as_ref().filter(|s| !s.is_empty());
                let config = if uri.contains(API_KEY) {
                    let Some(var_name) = auth_var else {
                        anyhow::bail!("No enviroment for API KEY deifned")
                    };

                    let token = std::env::var(var_name)?;
                    let uri = uri.replace(API_KEY, &token);

                    StreamableHttpClientTransportConfig::with_uri(uri.as_str())
                } else {
                    let mut config = StreamableHttpClientTransportConfig::with_uri(uri.as_str());

                    // optional auth
                    if let Some(var_name) = auth_var {
                        let token = std::env::var(var_name)?;
                        config = config.auth_header(token);
                    }

                    config
                };

                let transport = StreamableHttpClientTransport::from_config(config);

                let client_info = ClientInfo {
                    protocol_version: Default::default(),
                    capabilities: ClientCapabilities::default(),
                    client_info: Implementation {
                        name: "aerie".to_string(),
                        version: "0.1.0".to_string(),
                        ..Default::default()
                    },
                };

                let client = client_info.serve(transport).await.inspect_err(|e| {
                    tracing::error!("client error: {:?}", e);
                })?;

                McpClient::HTTP(Arc::new(client))
            }
        };

        let tools: Vec<Tool> = client.peer().list_tools(Default::default()).await?.tools;

        // prepend preface into each description
        let tools = tools
            .into_iter()
            .map(|mut tool| {
                if let Some(preface) = spec.preface()
                    && let Some(desc) = tool.description.clone()
                {
                    tool.description = Some(Cow::Owned(format!("{preface} {desc}")));
                }
                tool
            })
            .collect_vec();

        Ok(ToolProvider::MCP { client, tools })
    }
}

/// Runtime container managing all configured tool providers
#[derive(Default, Clone)]
pub struct Toolbox {
    pub providers: BTreeMap<String, ToolProvider>,
}

impl Toolbox {
    pub fn with_provider(&mut self, name: &str, provider: ToolProvider) -> &mut Self {
        self.providers.insert(name.into(), provider);
        self
    }

    pub fn get_tools(&self, toolset: &ToolSelector) -> RigToolSet {
        let mut result = RigToolSet::default();
        for (name, provider) in &self.providers {
            result.add_tools(provider.get_tools(name, |tool| toolset.apply(name, tool)))
        }

        result
    }

    pub fn select_tools<M: CompletionModel>(
        &self,
        agent: AgentBuilderSimple<M>,
        pred: impl Fn(&str, &Tool) -> bool + Copy,
    ) -> AgentBuilderSimple<M> {
        self.providers.iter().fold(agent, |agent, (name, chain)| {
            chain.select_tools(agent, name, |tool| pred(name, tool))
        })
    }

    // TODO: build the agent then manually create a ToolServer
    pub fn apply<M: CompletionModel>(
        &self,
        agent: AgentBuilderSimple<M>,
        toolset: &ToolSelector,
    ) -> AgentBuilderSimple<M> {
        self.select_tools(agent, |name, tool| toolset.apply(name, tool))
    }
}
