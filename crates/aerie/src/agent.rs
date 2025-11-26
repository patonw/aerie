use itertools::Itertools;
use std::sync::Arc;

use rig::{
    agent::{Agent, AgentBuilder},
    client::{CompletionClient as _, ProviderClient as _},
    providers::ollama::{self, CompletionModel},
};

use crate::config::ConfigExt as _;

pub use super::chat::{ChatContent, ChatEntry, ChatHistory, ChatSession};
pub use super::config::{Settings, ToolSpec, Toolset};
pub use super::logging::{LogChannelLayer, LogEntry};
pub use super::pipeline::{Pipeline, Workstep};
pub use super::toolbox::{ToolProvider, Toolbox};

type AgentT = AgentBuilder<CompletionModel>;

#[derive(Clone)]
pub struct AgentFactory {
    pub rt: tokio::runtime::Handle,
    pub settings: Arc<std::sync::RwLock<Settings>>,
    pub toolbox: Arc<Toolbox>,
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
