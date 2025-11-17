use itertools::Itertools;
use std::{borrow::Cow, collections::BTreeMap, sync::Arc};
use tokio::process::Command;

use rig::{agent::AgentBuilder, providers::ollama::CompletionModel};
use rmcp::{
    RoleClient, ServiceExt as _,
    model::Tool,
    service::RunningService,
    transport::{ConfigureCommandExt, TokioChildProcess},
};

use super::config::{ToolSpec, Toolset};

type AgentT = AgentBuilder<CompletionModel>;

#[derive(Clone)]
pub enum ToolProvider {
    MCP {
        client: Arc<RunningService<RoleClient, ()>>,
        tools: Vec<Tool>,
    },
}

impl ToolProvider {
    pub fn select_tools(&self, agent: AgentT, selector: impl Fn(&Tool) -> bool) -> AgentT {
        match self {
            ToolProvider::MCP { client, tools } => tools
                .iter()
                .filter(|it| selector(it))
                .cloned()
                .fold(agent, |agent, tool| {
                    agent.rmcp_tool(tool, client.peer().clone())
                }),
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

    pub fn all_tools(&self, agent: AgentT) -> AgentT {
        self.select_tools(agent, |_, _| true)
    }

    pub fn select_chains(&self, agent: AgentT, selection: &[&str]) -> AgentT {
        self.providers
            .iter()
            .filter(|(k, _)| selection.contains(&k.as_str()))
            .map(|(_, v)| v)
            .fold(agent, |agent, chain| chain.select_tools(agent, |_| true))
    }

    pub fn select_tools(&self, agent: AgentT, pred: impl Fn(&str, &Tool) -> bool) -> AgentT {
        self.providers.iter().fold(agent, |agent, (name, chain)| {
            chain.select_tools(agent, |tool| pred(name, tool))
        })
    }

    pub fn apply(&self, agent: AgentT, toolset: &Toolset) -> AgentT {
        self.select_tools(agent, |name, tool| toolset.apply(name, tool))
    }
}
