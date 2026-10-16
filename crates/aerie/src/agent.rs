use crate::rig::{
    self,
    agent::{Agent as RigAgent, AgentBuilder, PromptRequest},
    completion::ToolDefinition,
    message::Message,
    tool::{
        ToolContext,
        server::{ToolServer, ToolServerHandle},
    },
};
use anyhow::anyhow;
use arc_swap::{ArcSwap, ArcSwapOption};
use futures_util::{Stream, StreamExt as _};
use itertools::Itertools;
use rig_dynclient::{
    builder::DynClientBuilder, completion::CompletionModelHandle,
    rig::agent::StreamingPromptRequest,
};
use scopeguard::defer;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicU16, Ordering},
    },
};
use tokio::task::JoinSet;
use typed_builder::TypedBuilder;

use crate::{
    config::{AgentSpec, ConfigExt as _, Preferences},
    storage::CachedDirStore as _,
    toolbox::{ToolProvider, ToolStore, Toolbox},
    utils::ErrorList,
    workflow::store::WorkflowStoreDir,
};

#[allow(deprecated)]
pub type AgentBuilderT = AgentBuilder<CompletionModelHandle<'static>>;

#[allow(deprecated)]
pub type AgentT = RigAgent<CompletionModelHandle<'static>>;

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

    async fn call(
        &self,
        _ctx: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        Ok(args)
    }

    fn description(&self) -> String {
        "Submits a JSON value confirming to this schema.\n\
                Be sure to use this tool to submit your response."
            .to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        self.schema.clone()
    }
}

// Agent wrapper with accessors for tool defs & settings
#[derive(Clone)]
pub struct AgentW {
    pub spec: AgentSpec,

    tool_handle: ToolServerHandle,

    inner: AgentT,
}

pub struct CompletionOnly;

impl rig::agent::AgentHook for CompletionOnly {
    async fn on_tool_call(
        &self,
        _ctx: &rig::agent::HookContext,
        _event: rig::agent::ToolCall<'_>,
    ) -> rig::agent::ToolCallAction {
        rig::agent::ToolCallAction::Stop(Default::default())
    }
}

// TODO: wrap rig types in return values
impl AgentW {
    pub async fn tool_defs(&self) -> Vec<ToolDefinition> {
        let tool_definitions = self.tool_handle.get_tool_defs(None).await;
        tool_definitions.unwrap_or_default()
    }

    pub async fn exec_tool(&self, tool_name: &str, args: &str) -> rig::tool::ToolResult {
        let mut tool_ctx = ToolContext::new();
        self.tool_handle
            .execute(tool_name, args, &mut tool_ctx)
            .await
    }

    pub fn exec_completion<H, Item>(
        &self,
        prompt: impl Into<rig::message::Message>,
        history: H,
    ) -> impl IntoFuture<Output = Result<rig::agent::PromptResponse, rig::prelude::PromptError>>
    where
        H: IntoIterator<Item = Item>,
        Item: Into<Message>,
    {
        PromptRequest::from_agent(&self.inner, prompt)
            .history(history)
            .add_hook(CompletionOnly)
            .extended_details()
    }

    pub fn exec_chat<H, Item>(
        &self,
        prompt: impl Into<rig::message::Message>,
        history: H,
        max_turns: Option<usize>,
    ) -> impl IntoFuture<Output = Result<rig::agent::PromptResponse, rig::prelude::PromptError>>
    where
        H: IntoIterator<Item = Item>,
        Item: Into<Message>,
    {
        let mut request = PromptRequest::from_agent(&self.inner, prompt).history(history);
        if let Some(max_turns) = max_turns {
            request = request.max_turns(max_turns);
        }

        request.extended_details()
    }

    pub fn stream_completion<H, Item>(
        &self,
        prompt: impl Into<rig::message::Message>,
        history: H,
    ) -> impl Stream<
        Item = Result<
            rig::prelude::MultiTurnStreamItem<rig_dynclient::completion::FinalCompletionResponse>,
            rig::agent::StreamingError,
        >,
    >
    where
        H: IntoIterator<Item = Item>,
        Item: Into<Message>,
    {
        let request = StreamingPromptRequest::from_agent(&self.inner, prompt)
            .history(history)
            .add_hook(CompletionOnly);

        async_stream::stream! {
            let mut stream= request.await;
            while let Some(item) = stream.next().await {
                yield item;
            }
        }
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
    pub cache: Arc<ArcSwap<im::HashMap<AgentSpec, AgentW>>>,

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
        let temperature = self.prefs.view(|s| s.temperature.into_inner());

        let (provider, model) = self.parse_model(provider_model)?;

        tracing::info!("Building agent with provider {provider} model {model}");

        let completion =
            DynClientBuilder::with_env(self.env.clone()).completion(provider.leak(), &model)?;

        let handle = CompletionModelHandle::new(Arc::from(completion));
        Ok(AgentBuilder::new(handle).temperature(temperature))
    }

    pub fn spec_to_agent(&self, spec: &AgentSpec) -> anyhow::Result<AgentW> {
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

        let tool_server = if let Some(schema) = &spec.schema {
            let tool = StructuredSubmit::from(schema.as_ref());
            ToolServer::new().tool(tool)
        } else {
            self.toolbox.get_tools(&spec.tool_selection())
        };

        let tool_handle = tool_server.run();

        let inner = agent.tool_server_handle(tool_handle.clone());

        let agent = AgentW {
            spec: spec.clone(),
            tool_handle,
            inner: inner.build(),
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
            task_count.fetch_add(1, Ordering::AcqRel);

            defer! {
                task_count.fetch_sub(1, Ordering::AcqRel);
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
