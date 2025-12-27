use itertools::Itertools;
use rig::{
    agent::{Agent, AgentBuilderSimple},
    client::{builder::DynClientBuilder, completion::CompletionModelHandle},
};
use std::sync::Arc;

use crate::config::ConfigExt as _;

pub use super::chat::{ChatContent, ChatEntry, ChatHistory, ChatSession};
pub use super::config::{Settings, ToolSpec, Toolset};
pub use super::logging::{LogChannelLayer, LogEntry};
pub use super::pipeline::{Pipeline, Workstep};
pub use super::toolbox::{ToolProvider, Toolbox};

pub type AgentBuilderT = AgentBuilderSimple<CompletionModelHandle<'static>>;
pub type AgentT = Agent<CompletionModelHandle<'static>>;

#[derive(Clone)]
pub struct AgentFactory {
    pub rt: tokio::runtime::Handle,
    pub settings: Arc<std::sync::RwLock<Settings>>,
    pub toolbox: Arc<Toolbox>,
}

impl AgentFactory {
    pub fn builder(&self, provider_model: &str) -> anyhow::Result<AgentBuilderT> {
        let settings = self.settings.read().unwrap();

        let (provider, model) = self.parse_model(provider_model)?;

        tracing::info!("Building agent with provider {provider} model {model}");

        let completion = DynClientBuilder::new().completion(&provider, &model)?;

        let handle = CompletionModelHandle {
            inner: Arc::from(completion),
        };
        Ok(AgentBuilderSimple::new(handle)
            .preamble(&settings.preamble)
            .temperature(settings.temperature))
    }

    fn parse_model(&self, provider_model: &str) -> anyhow::Result<(String, String)> {
        let (provider, model) = provider_model
            .split_once("/")
            .map(|(p, m)| (p.to_string(), m.to_string()))
            .or_else(|| {
                self.settings.view(|s| {
                    s.llm_model
                        .split_once("/")
                        .map(|(p, m)| (p.to_string(), m.to_string()))
                })
            })
            .ok_or(anyhow::anyhow!(
                "Could not determine LLM provider and model"
            ))?;
        Ok((provider, model))
    }

    pub fn agent(&self, step: &Workstep) -> AgentT {
        let mut builder = self.builder("").unwrap();

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

    // TODO: Let's save errors to display in tool tab instead of aborting
    pub fn reload_tools(&mut self) -> anyhow::Result<()> {
        let mut toolbox = Toolbox::default();

        let providers = self.settings.view(|settings| {
            settings
                .tools
                .provider
                .iter()
                .filter(|(_, spec)| matches!(spec, ToolSpec::MCP { enabled, .. } if *enabled))
                .map(|(name, spec)| (name.clone(), spec.clone()))
                .collect_vec()
        });

        for (tool_name, tool_spec) in providers {
            let toolkit = self
                .rt
                .block_on(async move { ToolProvider::from_spec(&tool_spec).await })?;

            toolbox.with_provider(&tool_name, toolkit);
        }

        self.toolbox = Arc::new(toolbox);

        Ok(())
    }
}
