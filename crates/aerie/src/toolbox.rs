use itertools::Itertools;
use rig::{agent::AgentBuilderSimple, completion::CompletionModel, tool::ToolSet as RigToolSet};
use std::{borrow::Cow, collections::BTreeMap, sync::Arc};
use tokio::process::Command;

use rmcp::{
    RoleClient, ServiceExt as _,
    model::Tool,
    service::RunningService,
    transport::{ConfigureCommandExt, TokioChildProcess},
};

use super::config::{ToolSpec, Toolset};

#[derive(Clone)]
pub enum ToolProvider {
    MCP {
        client: Arc<RunningService<RoleClient, ()>>,
        tools: Vec<Tool>,
    },
}

impl ToolProvider {
    pub fn get_tools(&self, selector: impl Fn(&Tool) -> bool) -> RigToolSet {
        let mut result = RigToolSet::default();
        match self {
            ToolProvider::MCP { client, tools } => {
                for tool in tools {
                    if selector(tool) {
                        result.add_tool(rig::tool::rmcp::McpTool::from_mcp_server(
                            tool.clone(),
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
        selector: impl Fn(&Tool) -> bool,
    ) -> AgentBuilderSimple<M> {
        match self {
            ToolProvider::MCP { client, tools } => {
                let selection = tools
                    .iter()
                    .filter(|it| selector(it))
                    .cloned()
                    .collect_vec();
                agent.rmcp_tools(selection, client.peer().clone())
            }
        }
    }

    pub async fn from_spec(spec: &ToolSpec) -> anyhow::Result<Self> {
        match spec {
            ToolSpec::MCP {
                enabled: _,
                preface,
                dir,
                command,
                args,
            } => {
                let client = ()
                    .serve(TokioChildProcess::new(Command::new(command).configure(
                        |cmd| {
                            let cmd = args.iter().fold(cmd, |cmd, arg| cmd.arg(arg));
                            if let Some(cwd) = dir {
                                cmd.current_dir(cwd);
                            }
                        },
                    ))?)
                    .await
                    .inspect_err(|e| {
                        tracing::error!("client error: {:?}", e);
                    })?;

                let client = Arc::new(client);

                let tools: Vec<Tool> = client.list_tools(Default::default()).await?.tools;

                // prepend preface into each description
                let tools = tools
                    .into_iter()
                    .map(|mut tool| {
                        if let Some(preface) = preface
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

    pub fn all_tools<M: CompletionModel>(
        &self,
        agent: AgentBuilderSimple<M>,
    ) -> AgentBuilderSimple<M> {
        self.select_tools(agent, |_, _| true)
    }

    pub fn select_chains<M: CompletionModel>(
        &self,
        agent: AgentBuilderSimple<M>,
        selection: &[&str],
    ) -> AgentBuilderSimple<M> {
        self.providers
            .iter()
            .filter(|(k, _)| selection.contains(&k.as_str()))
            .map(|(_, v)| v)
            .fold(agent, |agent, chain| chain.select_tools(agent, |_| true))
    }

    pub fn get_tools(&self, toolset: &Toolset) -> RigToolSet {
        let mut result = RigToolSet::default();
        for (name, provider) in &self.providers {
            result.add_tools(provider.get_tools(|tool| toolset.apply(name, tool)))
        }

        result
    }

    pub fn select_tools<M: CompletionModel>(
        &self,
        agent: AgentBuilderSimple<M>,
        pred: impl Fn(&str, &Tool) -> bool + Copy,
    ) -> AgentBuilderSimple<M> {
        self.providers.iter().fold(agent, |agent, (name, chain)| {
            chain.select_tools(agent, |tool| pred(name, tool))
        })
    }

    // TODO: build the agent then manually create a ToolServer
    pub fn apply<M: CompletionModel>(
        &self,
        agent: AgentBuilderSimple<M>,
        toolset: &Toolset,
    ) -> AgentBuilderSimple<M> {
        self.select_tools(agent, |name, tool| toolset.apply(name, tool))
    }
}
