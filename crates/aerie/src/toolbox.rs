use arc_swap::ArcSwapOption;
use cached::proc_macro::cached;
use itertools::Itertools;
use rig::{
    agent::AgentBuilderSimple,
    completion::CompletionModel,
    tool::{Tool as _, ToolSet as RigToolSet},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{borrow::Cow, collections::BTreeMap, iter, sync::Arc};
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

use crate::{
    config::Ternary,
    workflow::{
        WorkflowError,
        store::{WorkflowStore as _, WorkflowStoreDir},
    },
};

use super::config::{ToolSelector, ToolSpec};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainBreaker;

impl rig::tool::Tool for ChainBreaker {
    const NAME: &'static str = "__break__";

    type Error = WorkflowError;

    type Args = serde_json::Value;

    type Output = String;

    async fn definition(&self, _prompt: String) -> rig::completion::ToolDefinition {
        rig::completion::ToolDefinition {
            name: Self::NAME.to_string(),
            description: Self::description().to_string(),
            parameters: json!({
                "type": "object",
                "properties": { }
            }),
        }
    }

    async fn call(&self, _args: Self::Args) -> Result<Self::Output, Self::Error> {
        Ok("Stopping workflow chain".to_string())
    }
}

impl ChainBreaker {
    pub fn description() -> Cow<'static, str> {
        Cow::Borrowed(
            "Do not queue any more workflows. Stop running after the current one completes.",
        )
    }
}

#[cached(result = true)]

fn parse_schema(text: String) -> anyhow::Result<serde_json::Value> {
    Ok(serde_json::from_str(&text)?)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainTool {
    next_workflow: Arc<ArcSwapOption<String>>,
    next_prompt: Arc<ArcSwapOption<String>>,

    name: String,
    description: String,
    schema: String,
}

impl rig::tool::Tool for ChainTool {
    const NAME: &'static str = "chainer";

    type Error = WorkflowError;

    type Args = serde_json::Value;

    type Output = String;

    fn name(&self) -> String {
        self.name.clone()
    }

    async fn definition(&self, _prompt: String) -> rig::completion::ToolDefinition {
        let description = format!(
            "Queues workflow '{}' to run next with a prompt.\n---\nWorkflow description:\n{}",
            &self.name, &self.description
        );

        let schema = Some(&self.schema)
            .filter(|s| !s.is_empty())
            .and_then(|s| parse_schema(s.clone()).ok())
            // .map(|s| json!({"input": s})) // To wrap or not to wrap?
            .unwrap_or_else(|| json!({
                "type": "object",
                "properties": {
                    "prompt": { "type": "string", "description": "A prompt to pass to the next agent" },
                }
            }));

        rig::completion::ToolDefinition {
            name: self.name.clone(),
            description,
            parameters: schema,
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        use serde_json::Value;
        tracing::info!(
            "Queuing next workflow: {} with prompt: {:?}",
            &self.name,
            &args
        );

        self.next_workflow.store(Some(Arc::new(self.name.clone())));
        match args {
            Value::String(text) => {
                self.next_prompt.store(Some(Arc::new(text)));
            }
            Value::Object(data) if data.keys().all(|k| k == "prompt") => {
                let input = data
                    .get("prompt")
                    .and_then(|s| s.as_str())
                    .map(|s| Arc::new(s.to_string()));
                self.next_prompt.store(input);
            }
            value => {
                self.next_prompt.store(Some(Arc::new(
                    serde_json::to_string(&value).unwrap_or_default(),
                )));
            }
        }

        Ok(format!("Queued workflow {} to run next", &self.name))
    }
}

#[derive(Clone)]
pub enum ToolProvider {
    Chainer {
        workflows: WorkflowStoreDir,

        next_workflow: Arc<ArcSwapOption<String>>,
        next_prompt: Arc<ArcSwapOption<String>>,
    },
    MCP {
        client: McpClient,
        tools: Vec<Tool>,
        timeout: Option<u64>,
    },
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
    pub fn description(&'_ self) -> Cow<'_, str> {
        match self {
            ToolProvider::Chainer { .. } => {
                Cow::Borrowed("Run another workflow after this one finishes")
            }
            ToolProvider::MCP { .. } => Cow::Borrowed("An MCP toolset"), // TODO: get from spec
        }
    }

    pub fn tool_description(&'_ self, tool_name: impl AsRef<str>) -> Cow<'_, str> {
        let tool_name = tool_name.as_ref();
        match self {
            ToolProvider::Chainer { workflows, .. } => {
                if tool_name == ChainBreaker::NAME {
                    ChainBreaker::description()
                } else {
                    workflows.description(tool_name)
                }
            }
            ToolProvider::MCP { tools, .. } => tools
                .iter()
                .find(|t| t.name == tool_name)
                .and_then(|t| t.description.clone())
                .unwrap_or(Cow::Owned("".to_string())),
        }
    }

    pub fn all_tool_names(&'_ self) -> Vec<Cow<'_, str>> {
        match self {
            ToolProvider::MCP { tools, .. } => tools.iter().map(|t| t.name.clone()).collect_vec(),
            ToolProvider::Chainer { workflows, .. } => {
                iter::once(Cow::Owned(ChainBreaker::NAME.to_string()))
                    .chain(workflows.names().map(|s| Cow::Owned(s.into_owned())))
                    .collect_vec()
            }
        }
    }

    pub fn contains_tool(&self, selector: impl Fn(&str) -> bool) -> bool {
        match self {
            ToolProvider::MCP { tools, .. } => {
                for tool in tools {
                    if selector(&tool.name) {
                        return true;
                    }
                }
            }
            ToolProvider::Chainer { workflows, .. } => {
                return workflows.names().any(|name| selector(&name));
            }
        }

        false
    }

    pub fn get_tools(&self, _provider: &str, selector: impl Fn(&str) -> bool) -> RigToolSet {
        let mut result = RigToolSet::default();
        match self {
            ToolProvider::MCP { client, tools, .. } => {
                for tool in tools {
                    if selector(&tool.name) {
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
            ToolProvider::Chainer {
                workflows,
                next_workflow,
                next_prompt,
            } => {
                if selector(ChainBreaker::NAME) {
                    result.add_tool(ChainBreaker);
                }

                let store = workflows;
                for name in store.names().filter(|name| selector(name)) {
                    let description = store.description(&name);
                    let schema = store.schema(&name);
                    let tool = ChainTool {
                        next_workflow: next_workflow.clone(),
                        next_prompt: next_prompt.clone(),
                        name: name.into_owned(),
                        description: description.into_owned(),
                        schema: schema.into_owned(),
                    };
                    result.add_tool(tool);
                }
            }
        }

        result
    }

    pub fn select_tools<M: CompletionModel>(
        &self,
        mut agent: AgentBuilderSimple<M>,
        _provider: &str,
        selector: impl Fn(&str) -> bool,
    ) -> AgentBuilderSimple<M> {
        match self {
            ToolProvider::MCP { client, tools, .. } => {
                let selection = tools
                    .iter()
                    .filter(|it| selector(&it.name))
                    .cloned()
                    // .map(|mut tool| {
                    //     tool.name = Cow::Owned(format!("{provider}::{}", tool.name.as_ref()));
                    //     tool
                    // })
                    .collect_vec();
                agent.rmcp_tools(selection, client.peer().clone())
            }
            ToolProvider::Chainer {
                workflows,
                next_workflow,
                next_prompt,
            } => {
                if selector(ChainBreaker::NAME) {
                    agent = agent.tool(ChainBreaker);
                }
                let store = workflows;
                for name in store.names().filter(|name| selector(name)) {
                    let description = store.description(&name);
                    let schema = store.schema(&name);
                    let tool = ChainTool {
                        next_workflow: next_workflow.clone(),
                        next_prompt: next_prompt.clone(),
                        name: name.into_owned(),
                        description: description.into_owned(),
                        schema: schema.into_owned(),
                    };
                    agent = agent.tool(tool);
                }

                agent
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

        Ok(ToolProvider::MCP {
            client,
            tools,
            timeout: spec.timeout(),
        })
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
            tracing::debug!("Adding tools for provider {name}");
            result.add_tools(provider.get_tools(name, |tool| toolset.apply(name, tool)))
        }

        result
    }

    pub fn select_tools<M: CompletionModel>(
        &self,
        agent: AgentBuilderSimple<M>,
        pred: impl Fn(&str, &str) -> bool + Copy,
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

    pub fn provider_for(&self, selector: &ToolSelector, tool_name: &str) -> Option<&ToolProvider> {
        self.providers
            .iter()
            .find(|(name, chain)| {
                chain.contains_tool(|tool| tool == tool_name && selector.apply(name, tool))
            })
            .map(|(_, p)| p)
    }

    pub fn timeout(&self, toolset: &ToolSelector, tool_name: &str) -> Option<u64> {
        self.provider_for(toolset, tool_name).and_then(|p| match p {
            ToolProvider::MCP { timeout, .. } => *timeout,
            ToolProvider::Chainer { .. } => None,
        })
    }

    pub fn toggle_provider(
        &self,
        selector: &ToolSelector,
        provider: &str,
        state: Ternary<Cow<str>>,
    ) -> ToolSelector {
        // First clear the selection for this item
        let selection = if selector.is_all() {
            self.providers.keys().map(|p| format!("{p}/*")).collect()
        } else {
            let prefix = format!("{provider}/");
            selector
                .0
                .iter()
                .filter(|it| !it.starts_with(&prefix))
                .collect()
        };

        // Add in our target selection
        let selection: im::OrdSet<_> = match state {
            Ternary::None => selection,
            Ternary::Some(items) => {
                selection.union(items.iter().map(|t| format!("{provider}/{t}")).collect())
            }
            Ternary::All => selection.update(format!("{provider}/*")),
        };

        ToolSelector(selection)
    }

    pub fn toggle_tool(
        &self,
        selector: &ToolSelector,
        provider: &str,
        tool_name: &str,
        active: bool,
    ) -> ToolSelector {
        match selector.provider_selection(provider) {
            Ternary::None if active => self.toggle_provider(
                selector,
                provider,
                Ternary::Some(im::ordset![Cow::Borrowed(tool_name)]),
            ),
            Ternary::Some(mut items) => {
                if active {
                    items.insert(Cow::Borrowed(tool_name));
                } else {
                    items.remove(&Cow::Borrowed(tool_name));
                }

                self.toggle_provider(selector, provider, Ternary::Some(items))
            }
            Ternary::All if !active => {
                let tools = self
                    .providers
                    .get(provider)
                    .iter()
                    .flat_map(|p| p.all_tool_names())
                    .filter(|n| *n != tool_name)
                    .collect();

                self.toggle_provider(selector, provider, Ternary::Some(tools))
            }
            _ => selector.clone(),
        }
    }
}
