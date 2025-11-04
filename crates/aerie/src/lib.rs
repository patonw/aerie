use cached::proc_macro::cached;
use glob::{Pattern, PatternError};
use itertools::Itertools;
use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet, HashMap},
    ops::Deref,
    path::PathBuf,
    sync::Arc,
};
use tokio::process::Command;
use tracing::Subscriber;
use tracing_subscriber::{Layer, layer::Context, registry::LookupSpan};

use rig::{
    agent::{Agent, AgentBuilder},
    client::{CompletionClient as _, ProviderClient as _},
    message::Message,
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
use uuid::Uuid;

pub mod ui;

#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug, Default, PartialEq, Clone)]
pub struct Settings {
    #[serde(default)]
    pub llm_model: String,

    #[serde(default)]
    pub preamble: String,

    #[serde(default)]
    pub temperature: f64,

    #[serde(default)]
    pub show_logs: bool,

    #[serde(default)]
    pub autoscroll: bool,

    #[serde(default)]
    pub workflows: Vec<Workflow>,

    #[serde(default)]
    pub active_flow: Option<String>,

    #[serde(default)]
    pub tools: ToolSettings,
}

impl Settings {
    pub fn get_workflow(&self) -> Option<&Workflow> {
        let name = self.active_flow.as_ref()?;

        self.workflows.iter().find(|it| it.name == *name)
    }
}

#[skip_serializing_none]
#[derive(Serialize, Deserialize, Default, Debug, PartialEq, Clone)]
pub struct ToolSettings {
    pub provider: BTreeMap<String, ToolSpec>,
    pub toolset: BTreeMap<String, Toolset>,
}

/// Configuration to access a tool provider
#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
#[serde(tag = "type")]
pub enum ToolSpec {
    MCP {
        preface: Option<String>,
        dir: Option<PathBuf>,
        command: String,
        args: Vec<String>,
    },
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
pub struct Toolset(BTreeSet<String>);

impl Toolset {
    pub fn empty() -> Self {
        Self(Default::default())
    }

    pub fn with_include(mut self, provider: &str, tool: &str) -> Self {
        self.0.insert(format!("{provider}/{tool}"));
        self
    }

    pub fn add(&mut self, selector: &str) {
        self.0.insert(selector.to_string());
    }

    pub fn remove(&mut self, selector: &str) {
        self.0.remove(selector);
    }

    pub fn include(&mut self, provider: &str, tool: &Tool) {
        self.add(&format!("{provider}/{}", tool.name));
    }

    pub fn exclude(&mut self, provider: &str, tool: &Tool) {
        let path = format!("{provider}/{}", tool.name);
        self.0.retain(|it| {
            if let Ok(pattern) = tool_glob(it.clone()) {
                !pattern.matches(&path)
            } else {
                false
            }
        });
    }
    pub fn toggle(&mut self, provider: &str, tool: &Tool) {
        if self.apply(provider, tool) {
            self.exclude(provider, tool);
        } else {
            self.include(provider, tool);
        }
    }

    pub fn apply(&self, provider: &str, tool: &Tool) -> bool {
        self.0
            .iter()
            .filter_map(|it| tool_glob(it.clone()).ok())
            .any(|it| it.matches(&format!("{provider}/{}", tool.name)))
    }
}

#[cached(result = true)]
pub fn tool_glob(pattern: String) -> Result<Pattern, PatternError> {
    Pattern::new(&pattern)
}

impl Default for Toolset {
    fn default() -> Self {
        Self(["*/*".into()].into())
    }
}

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

#[derive(Debug, Clone)]
pub enum ChatContent {
    Message(Message),
    Aside {
        workflow: String,
        prompt: String,
        collapsed: bool,
        content: Vec<Message>,
    },
    Error(String),
}

impl From<Result<Message, String>> for ChatContent {
    fn from(value: Result<Message, String>) -> Self {
        match value {
            Ok(msg) => ChatContent::Message(msg),
            Err(err) => ChatContent::Error(err),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ChatEntry {
    pub id: Uuid,
    pub parent: Option<Uuid>,
    pub branch: String,
    pub content: ChatContent,
}

impl Deref for ChatEntry {
    type Target = ChatContent;

    fn deref(&self) -> &Self::Target {
        &self.content
    }
}

#[derive(Debug, Default, Clone)]
pub struct ChatHistory {
    pub counter: usize,
    pub store: HashMap<Uuid, ChatEntry>,
    pub branches: HashMap<String, Uuid>,
    pub head: Option<String>,
}

impl ChatHistory {
    pub fn push(&mut self, content: ChatContent, branch: Option<impl AsRef<str>>) {
        let (branch, parent) = if let Some(branch) = branch {
            (
                branch.as_ref().to_string(),
                self.branches.get(branch.as_ref()).cloned(),
            )
        } else {
            self.head_branch()
        };

        let id = Uuid::new_v4();
        let entry = ChatEntry {
            id,
            parent,
            content,
            branch: branch.clone(),
        };
        self.store.insert(id, entry);

        self.branches.insert(branch.clone(), id);
    }

    pub fn extend(
        &mut self,
        contents: impl std::iter::IntoIterator<Item = ChatContent>,
        branch: Option<impl AsRef<str>>,
    ) {
        for content in contents {
            self.push(content, branch.as_ref());
        }
    }

    pub fn clear(&mut self) {
        self.store.clear();
        self.branches.clear();
        self.head = None;
    }

    fn head_branch(&mut self) -> (String, Option<Uuid>) {
        if self.head.is_none() {
            self.head = Some("default".into());
        }

        let branch = self.head.to_owned().unwrap();
        let uuid = self.branches.get(&branch).cloned();
        (branch, uuid)
    }

    pub fn switch_branch(&mut self, branch: &str, parent: Option<Uuid>) {
        // Nah, a little pointless to branch from current head
        // let parent = parent.or_else(|| self.head_branch().1);

        // If no parent, then creates a new root
        if !self.branches.contains_key(branch)
            && let Some(parent) = parent
        {
            self.branches.insert(branch.to_string(), parent);
        }

        self.head = Some(branch.into());
    }

    pub fn find_parent(&self, id: Uuid) -> Option<Uuid> {
        self.store.get(&id).and_then(|it| it.parent)
    }

    pub fn last(&self) -> Option<&ChatEntry> {
        self.head
            .as_ref()
            .and_then(|it| self.branches.get(it))
            .and_then(|it| self.store.get(it))
    }

    pub fn iter(&self) -> impl Iterator<Item = &ChatEntry> {
        let mut buffer: Vec<&ChatEntry> = Vec::new();
        let mut cursor = self
            .head
            .as_ref()
            .and_then(|it| self.branches.get(it))
            .cloned();

        while let Some(id) = cursor {
            if let Some(entry) = self.store.get(&id) {
                buffer.push(entry);
                cursor = entry.parent;
            }
        }

        buffer.reverse();
        buffer.into_iter()
    }

    pub fn lineage(&self) -> BTreeMap<String, BTreeSet<String>> {
        let mut buffer: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for entry in self.store.values() {
            let parent_branch = entry
                .parent
                .as_ref()
                .and_then(|it| self.store.get(it))
                .map(|it| it.branch.as_ref());

            if let Some(pb) = parent_branch {
                if pb != entry.branch {
                    buffer
                        .entry(pb.to_string())
                        .or_default()
                        .insert(entry.branch.clone());
                }
            } else {
                // Is a root entry
                buffer
                    .entry("".into())
                    .or_default()
                    .insert(entry.branch.clone());
            }
        }

        let parent = self
            .head
            .as_ref()
            .and_then(|h| self.branches.get(h))
            .and_then(|id| self.store.get(id))
            .map(|it| it.branch.clone());

        if let Some(head) = self.head.clone()
            && parent != self.head
        {
            buffer
                .entry(parent.unwrap_or_default())
                .or_default()
                .insert(head);
        }

        buffer
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
