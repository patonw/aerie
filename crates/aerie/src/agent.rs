#[allow(deprecated)]
use crate::rig::{
    self,
    agent::{Agent, AgentBuilder},
    completion::ToolDefinition,
};
use anyhow::anyhow;
use arc_swap::{ArcSwap, ArcSwapOption};
use decorum::E64;
use derive_builder::Builder;
use itertools::Itertools;
use rig_dynclient::{builder::DynClientBuilder, completion::CompletionModelHandle};
use scopeguard::defer;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    hash::Hash,
    sync::{
        Arc,
        atomic::{AtomicU16, Ordering},
    },
};
use tokio::task::JoinSet;
use typed_builder::TypedBuilder;

use crate::{
    config::ConfigExt as _, storage::CachedDirStore as _, toolbox::ToolStore, utils::ErrorList,
    workflow::store::WorkflowStoreDir,
};

pub use super::chat::{ChatContent, ChatEntry, ChatHistory, ChatSession};
pub use super::config::{Preferences, ToolSelector, ToolSpec};
pub use super::logging::{LogChannelLayer, LogEntry};
pub use super::pipeline::{Pipeline, Workstep};
pub use super::toolbox::{ToolProvider, Toolbox};

#[allow(deprecated)]
pub type AgentBuilderT = AgentBuilder<CompletionModelHandle<'static>>;

#[allow(deprecated)]
pub type AgentT = Agent<CompletionModelHandle<'static>>;

#[derive(Serialize, Deserialize)]
pub struct StructuredSubmit {
    schema: serde_json::Value,
}

impl From<&serde_json::Value> for StructuredSubmit {
    fn from(value: &serde_json::Value) -> Self {
        Self {
            schema: value.clone(),
        }
    }
}

impl rig::tool::Tool for StructuredSubmit {
    const NAME: &'static str = "submit-structured-data";

    type Error = std::io::Error; // placeholder

    type Args = serde_json::Value;

    type Output = serde_json::Value;

    async fn definition(&self, _prompt: String) -> rig::completion::ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "\
                Submits a JSON value confirming to this schema.\n\
                Be sure to use this tool to submit your response."
                .to_string(),
            parameters: self.schema.clone(),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        Ok(args)
    }
}

#[derive(TypedBuilder, Clone)]
pub struct AgentFactory {
    pub rt: tokio::runtime::Handle,

    pub prefs: Arc<ArcSwap<Preferences>>,

    // #[builder(default, setter(strip_option))]
    pub tools: Option<ToolStore>,

    #[builder(default=std::env::vars().collect())]
    pub env: HashMap<String, String>,

    #[builder(default)]
    pub errors: ErrorList<anyhow::Error>,

    #[builder(default)]
    pub task_count: Arc<AtomicU16>,

    #[builder(default)]
    pub store: Option<WorkflowStoreDir>,

    #[builder(default)]
    pub toolbox: Toolbox,

    #[builder(default)]
    pub cache: Arc<ArcSwap<im::HashMap<AgentSpec, AgentT>>>,

    #[builder(default)]
    pub next_workflow: Arc<ArcSwapOption<String>>,

    #[builder(default)]
    pub next_prompt: Arc<ArcSwapOption<String>>,

    #[builder(default)]
    pub next_images: Arc<ArcSwap<Vec<String>>>,
}

impl AgentFactory {
    pub fn with_tools(self, tools: Option<ToolStore>) -> Self {
        Self { tools, ..self }
    }

    pub fn with_env(self, env: impl IntoIterator<Item = (String, String)>) -> Self {
        Self {
            env: env.into_iter().collect(),
            toolbox: Default::default(),
            ..self
        }
    }

    #[allow(deprecated)]
    pub fn agent_builder(&self, provider_model: &str) -> anyhow::Result<AgentBuilderT> {
        let temperature = self.prefs.view(|s| s.temperature);

        let (provider, model) = self.parse_model(provider_model)?;

        tracing::info!("Building agent with provider {provider} model {model}");

        let completion =
            DynClientBuilder::with_env(self.env.clone()).completion(provider.leak(), &model)?;

        let handle = CompletionModelHandle::new(Arc::from(completion));
        Ok(AgentBuilder::new(handle).temperature(temperature))
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

        let agent = if let Some(schema) = &spec.schema {
            let tool = StructuredSubmit::from(schema.as_ref());
            agent.tool(tool).build()
        } else if let Some(toolset) = &spec.tools {
            match self.toolbox.apply(agent, toolset) {
                either::Either::Left(builder) => builder.build(),
                either::Either::Right(builder) => builder.build(),
            }
        } else {
            agent.build()
        };

        self.cache
            .store(Arc::new(cache.update(spec.clone(), agent.clone())));

        Ok(agent)
    }

    fn parse_model(&self, provider_model: &str) -> anyhow::Result<(String, String)> {
        let (provider, model) = provider_model
            .split_once("/")
            .map(|(p, m)| (p.to_string(), m.to_string()))
            .ok_or(anyhow!("Could not determine LLM provider and model"))?;
        Ok((provider, model))
    }

    pub fn stop_provider(&mut self, name: &str) {
        self.toolbox.clone().without_provider(name);
    }

    /// Load or reload a provider from the tool store
    pub async fn reload_provider(&self, name: &str) -> anyhow::Result<()> {
        let task_count = self.task_count.clone();
        let toolbox = self.toolbox.clone();

        if let Some(tool_store) = self.tools.clone() {
            task_count.fetch_add(1, Ordering::Relaxed);

            defer! {
                task_count.fetch_sub(1, Ordering::Relaxed);
            };

            tool_store.load_provider(toolbox, name, &self.env).await?;
        }
        Ok(())
    }

    /// Initialize toolbox by loading each active provider
    pub async fn load_tools(&self) -> Vec<anyhow::Result<()>> {
        if let Some(store) = &self.store {
            self.toolbox.with_provider(
                "chainer",
                ToolProvider::Chainer {
                    workflows: store.clone(),
                    next_workflow: self.next_workflow.clone(),
                    next_prompt: self.next_prompt.clone(),
                    next_images: self.next_images.clone(),
                },
            );
        }

        let providers = self
            .tools
            .iter()
            .flat_map(|store| store.cached_names())
            .collect_vec();

        let mut joiner = JoinSet::new();

        for provider in providers {
            let that = self.clone();
            joiner.spawn(async move { that.reload_provider(&provider).await });
        }

        joiner.join_all().await
    }
}

#[derive(Builder)]
#[builder(
    name = "AgentSpec",
    derive(Debug, Hash, PartialEq, Eq, Serialize),
    field(public)
)]
// For use via the derived builder, not directly
pub struct _AgentSpec_ {
    pub model: String,

    pub temperature: E64,

    pub preamble: String,

    pub context_doc: Arc<String>,

    pub tools: Arc<ToolSelector>,

    pub schema: Arc<serde_json::Value>,
}

impl AgentSpec {
    pub fn agent(&self, factory: &AgentFactory) -> anyhow::Result<AgentT> {
        factory.spec_to_agent(self)
    }

    pub fn tool_selection(&self) -> Arc<ToolSelector> {
        self.tools.clone().unwrap_or_default()
    }

    // TODO: method to just get rig tools from selection
}
