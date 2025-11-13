use itertools::Itertools;
use std::{borrow::Cow, collections::BTreeMap, sync::Arc};
use tokio::process::Command;
use tracing::Subscriber;
use tracing_subscriber::{Layer, layer::Context, registry::LookupSpan};

use rig::{
    agent::{Agent, AgentBuilder},
    client::{CompletionClient as _, ProviderClient as _},
    providers::ollama::{self, CompletionModel},
};
use rmcp::{
    RoleClient, ServiceExt as _,
    model::Tool,
    service::RunningService,
    transport::{ConfigureCommandExt, TokioChildProcess},
};
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

pub mod chat;
pub mod config;
pub mod ui;

pub use chat::{ChatContent, ChatEntry, ChatHistory, ChatSession};
pub use config::{Settings, ToolSpec, Toolset};

/// A single step in a workflow
#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug, Default, PartialEq, Clone)]
pub struct Workstep {
    #[serde(default)]
    pub disabled: bool,

    #[serde(default)]
    pub temperature: Option<f64>,

    /// Override the workflow preamble for this step
    #[serde(default)]
    pub preamble: Option<String>,

    /// Include the last `N` messages as context
    #[serde(default)]
    pub depth: Option<usize>,

    // TODO: templating mechanism
    pub prompt: String,

    pub tools: Option<String>,
}

/// A sequence of steps consisting of LLM invocations
#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
pub struct Workflow {
    pub name: String,

    /// Only retain the final response in the chat history
    #[serde(default)]
    pub collapse: bool,

    /// Override the global preamble
    #[serde(default)]
    pub preamble: Option<String>,

    pub steps: Vec<Workstep>,
}

impl Default for Workflow {
    fn default() -> Self {
        Self {
            name: Default::default(),
            collapse: false,
            preamble: None,
            steps: vec![Workstep {
                disabled: false,
                temperature: None,
                preamble: None,
                depth: None,
                prompt: "{{user_prompt}}".to_string(),
                tools: None,
            }],
        }
    }
}

// TODO: preserve more data
pub struct LogEntry(pub tracing::Level, pub String);

impl LogEntry {
    pub fn level(&self) -> tracing::Level {
        self.0
    }

    pub fn message(&self) -> &str {
        &self.1
    }
}

#[derive(Clone)]
pub struct LogChannelLayer(pub flume::Sender<LogEntry>);

impl<S> Layer<S> for LogChannelLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        use tracing::field::{Field, Visit};

        struct MessageVisitor {
            message: Option<String>,
        }

        impl Visit for MessageVisitor {
            fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
                if field.name() == "message" {
                    self.message = Some(format!("{:?}", value));
                }
            }
        }

        let mut visitor = MessageVisitor { message: None };
        event.record(&mut visitor);

        if let Some(msg) = visitor.message {
            self.0
                .send(LogEntry(
                    *event.metadata().level(),
                    msg.trim_matches('"').to_string(),
                ))
                .unwrap();
        }
    }
}

type AgentT = AgentBuilder<CompletionModel>;

/// An external service or process that provides tools that LLM agents can use
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

#[derive(Clone)]
pub struct AgentFactory {
    pub settings: Arc<std::sync::RwLock<Settings>>,
    pub toolbox: Toolbox,
}

impl AgentFactory {
    pub fn builder(&self) -> AgentT {
        let settings = self.settings.read().unwrap();
        let llm_client = ollama::Client::from_env();
        let model = if settings.llm_model.is_empty() {
            "devstral:latest"
        } else {
            settings.llm_model.as_str()
        };

        llm_client
            .agent(model)
            .preamble(&settings.preamble)
            .temperature(settings.temperature)
    }

    pub fn agent(&self, step: &Workstep) -> Agent<CompletionModel> {
        let mut builder = self.builder();
        if let Some(temperature) = step.temperature {
            builder = builder.temperature(temperature);
        }
        if let Some(preamble) = &step.preamble {
            builder = builder.preamble(preamble);
        }

        if let Some(tools) = &step.tools {
            let settings = self.settings.read().unwrap();
            if let Some(toolset) = settings.tools.toolset.get(tools) {
                builder = self.toolbox.apply(builder, toolset);
            }
        }

        builder.build()
    }
}
