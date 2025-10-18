use tracing::Subscriber;
use tracing_subscriber::{Layer, layer::Context, registry::LookupSpan};

use rig::{
    agent::Agent,
    client::{CompletionClient as _, ProviderClient as _},
    providers::ollama::{self, CompletionModel},
};
use rmcp::model::Tool;
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

pub mod ui;

#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug, Default, PartialEq, Clone)]
pub struct Settings {
    #[serde(default)]
    pub llm_model: String,

    #[serde(default)]
    pub preamble: String,

    #[serde(default)]
    pub temperature: f32,

    #[serde(default)]
    pub show_logs: bool,

    #[serde(default)]
    pub autoscroll: bool,
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

pub fn get_agent(
    settings: &Settings,
    mcp_client: &rmcp::service::ServerSink,
    tools: Vec<Tool>,
) -> Agent<CompletionModel> {
    let llm_client = ollama::Client::from_env();
    let model = if settings.llm_model.is_empty() {
        "devstral:latest"
    } else {
        settings.llm_model.as_str()
    };

    let llm_agent = llm_client
        .agent(model)
        .preamble(&settings.preamble)
        .temperature(settings.temperature as f64);

    let llm_agent = tools.into_iter().fold(llm_agent, |agent, tool| {
        agent.rmcp_tool(tool, mcp_client.clone())
    });

    llm_agent.build()
}
