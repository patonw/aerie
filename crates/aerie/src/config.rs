use cached::proc_macro::cached;
use glob::{Pattern, PatternError};
use itertools::Itertools as _;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::{
        Arc, RwLock,
        atomic::{AtomicU64, Ordering},
    },
};

use rmcp::model::Tool;
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

use clap::{Parser, Subcommand};

use crate::{ToolProvider, Toolbox};

#[derive(Parser, Clone, Debug)]
#[command(version, about, long_about = None)]
pub struct Args {
    #[arg(long, short)]
    pub session: Option<String>,

    #[arg(long, short)]
    pub config: Option<PathBuf>,

    #[arg(long)]
    pub session_dir: Option<PathBuf>,

    #[arg(long)]
    pub workflow_dir: Option<PathBuf>,

    #[arg(long)]
    pub backup_dir: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand, Clone, Debug)]
pub enum Command {
    Session {
        #[command(subcommand)]
        subcmd: SessionCommand,
    },
}

#[derive(Subcommand, Clone, Debug)]
pub enum SessionCommand {
    List,
}

#[inline]
fn is_zero(x: &u64) -> bool {
    *x == 0
}

#[derive(Serialize, Deserialize, Debug, Default, Clone)]
pub struct SeedConfig {
    pub value: Arc<AtomicU64>,

    #[serde(default, skip_serializing_if = "is_zero")]
    pub increment: u64,
}

impl PartialEq for SeedConfig {
    fn eq(&self, other: &Self) -> bool {
        let a = self.value.load(Ordering::Relaxed);
        let b = other.value.load(Ordering::Relaxed);
        a == b && self.increment == other.increment
    }
}

impl Eq for SeedConfig {}

#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug, Default, PartialEq, Clone)]
pub struct Settings {
    #[serde(default)]
    pub llm_model: String,

    #[serde(default, skip_serializing_if = "im::Vector::is_empty")]
    pub prev_models: im::Vector<String>,

    #[serde(default)]
    pub preamble: String,

    #[serde(default)]
    pub temperature: f64,

    pub seed: Option<SeedConfig>,

    #[serde(default)]
    pub show_logs: bool,

    #[serde(default)]
    pub autoscroll: bool,

    #[serde(default)]
    pub automation: Option<String>,

    #[serde(default)]
    pub tools: ToolSettings,

    #[serde(default)]
    pub last_workflow_dir: PathBuf,

    #[serde(default)]
    pub last_output_dir: PathBuf,

    // Making this configurable since not 100% confident in the streaming implementation
    // The runner will fall back to non-streaming.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub streaming: bool,

    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub autosave: bool,
}

pub trait ConfigExt {
    fn view<T>(&self, cb: impl FnMut(&Settings) -> T) -> T;

    fn update<T>(&self, cb: impl FnOnce(&mut Settings) -> T) -> T;
}

impl ConfigExt for Arc<RwLock<Settings>> {
    fn view<T>(&self, mut cb: impl FnMut(&Settings) -> T) -> T {
        let settings = self.read().unwrap();
        cb(&settings)
    }

    // TODO: handle auto-save
    fn update<T>(&self, cb: impl FnOnce(&mut Settings) -> T) -> T {
        let mut settings = self.write().unwrap();
        cb(&mut settings)
    }
}

#[skip_serializing_none]
#[derive(Serialize, Deserialize, Default, Debug, PartialEq, Clone)]
pub struct ToolSettings {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub provider: BTreeMap<String, ToolSpec>,

    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub toolset: BTreeMap<String, ToolSelector>,
}

impl ToolSettings {
    pub async fn load_toolbox(&self) -> anyhow::Result<Toolbox> {
        let mut toolbox = Toolbox::default();

        let providers = self
            .provider
            .iter()
            .filter(|(_, spec)| spec.enabled())
            .map(|(name, spec)| (name.clone(), spec.clone()))
            .collect_vec();

        for (tool_name, tool_spec) in providers {
            let toolkit = ToolProvider::from_spec(&tool_spec).await?;

            toolbox.with_provider(&tool_name, toolkit);
        }

        Ok(toolbox)
    }
}

/// Configuration to access a tool provider
#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
#[serde(tag = "type")]
pub enum ToolSpec {
    Stdio {
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        enabled: bool,

        #[serde(default)]
        preface: Option<String>,

        #[serde(default)]
        dir: Option<PathBuf>,

        #[serde(default, skip_serializing_if = "String::is_empty")]
        env: String,

        command: String,

        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        args: Vec<String>,
    },
    HTTP {
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        enabled: bool,

        #[serde(default)]
        preface: Option<String>,

        uri: String,

        /// : environment var for API key
        auth_var: Option<String>,
    },
}

impl Default for ToolSpec {
    fn default() -> Self {
        ToolSpec::Stdio {
            enabled: false,
            preface: None,
            dir: None,
            env: Default::default(),
            command: String::new(),
            args: Vec::new(),
        }
    }
}

impl ToolSpec {
    pub fn enabled(&self) -> bool {
        match self {
            ToolSpec::Stdio { enabled, .. } => *enabled,
            ToolSpec::HTTP { enabled, .. } => *enabled,
        }
    }

    pub fn preface(&self) -> Option<&str> {
        match self {
            ToolSpec::Stdio { preface, .. } => preface.as_deref(),
            ToolSpec::HTTP { preface, .. } => preface.as_deref(),
        }
    }
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolSelector(BTreeSet<String>);

impl ToolSelector {
    pub fn empty() -> Self {
        Self(Default::default())
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn all() -> Self {
        Self::empty().with_include("*", "*")
    }

    pub fn is_all(&self) -> bool {
        self.0.contains("*/*")
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

impl Default for ToolSelector {
    fn default() -> Self {
        Self(["*/*".into()].into())
    }
}
