use anyhow::anyhow;
use arc_swap::ArcSwap;
use decorum::E64;
use derive_builder::Builder;
use itertools::Itertools;
use rig::{
    agent::{Agent, AgentBuilderSimple},
    client::{builder::DynClientBuilder, completion::CompletionModelHandle},
};
use std::{hash::Hash, sync::Arc};
use typed_builder::TypedBuilder;

use crate::config::ConfigExt as _;

pub use super::chat::{ChatContent, ChatEntry, ChatHistory, ChatSession};
pub use super::config::{Settings, ToolSpec, Toolset};
pub use super::logging::{LogChannelLayer, LogEntry};
pub use super::pipeline::{Pipeline, Workstep};
pub use super::toolbox::{ToolProvider, Toolbox};

pub type AgentBuilderT = AgentBuilderSimple<CompletionModelHandle<'static>>;
pub type AgentT = Agent<CompletionModelHandle<'static>>;

#[derive(TypedBuilder, Clone)]
pub struct AgentFactory {
    pub rt: tokio::runtime::Handle,
    pub settings: Arc<std::sync::RwLock<Settings>>,

    #[builder(default)]
    pub toolbox: Arc<Toolbox>,

    #[builder(default)]
    pub cache: Arc<ArcSwap<im::HashMap<AgentSpec, AgentT>>>,
}

impl AgentFactory {
    pub fn agent_builder(&self, provider_model: &str) -> anyhow::Result<AgentBuilderT> {
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

    pub fn spec_to_agent(&self, spec: &AgentSpec) -> anyhow::Result<AgentT> {
        let cache = self.cache.load();
        if let Some(cached) = cache.get(spec) {
            return Ok(cached.clone());
        }

        let Some(model) = &spec.model else {
            anyhow::bail!("A model is required")
        };

        let mut agent = self.agent_builder(model)?;

        if let Some(temperature) = spec.temperature {
            agent = agent.temperature(temperature.into_inner());
        }

        if let Some(preamble) = &spec.preamble {
            agent = agent.preamble(preamble);
        }

        if let Some(context_doc) = &spec.context_doc {
            agent = agent.context(context_doc);
        }

        if let Some(toolset) = &spec.tools {
            agent = self.toolbox.apply(agent, toolset);
        }

        let agent = agent.build();
        self.cache
            .store(Arc::new(cache.update(spec.clone(), agent.clone())));

        Ok(agent)
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
            .ok_or(anyhow!("Could not determine LLM provider and model"))?;
        Ok((provider, model))
    }

    pub fn agent(&self, step: &Workstep) -> AgentT {
        let mut builder = self.agent_builder("").unwrap();

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

#[derive(Builder)]
#[builder(name = "AgentSpec", derive(Debug, Hash, PartialEq, Eq))]
// For use via the derived builder, not directly
pub struct _AgentSpec_ {
    pub model: String,

    pub temperature: E64,

    pub preamble: String,

    pub context_doc: String,

    pub tools: Arc<Toolset>,
}

impl AgentSpec {
    pub fn agent(&self, factory: &AgentFactory) -> anyhow::Result<AgentT> {
        factory.spec_to_agent(self)
    }
}
