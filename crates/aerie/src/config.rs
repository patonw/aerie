use cached::proc_macro::cached;
use glob::{Pattern, PatternError};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::{Arc, RwLock},
};

use rmcp::model::Tool;
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

use clap::{Parser, Subcommand};

use crate::Workflow;

#[derive(Parser, Clone, Debug)]
#[command(version, about, long_about = None)]
pub struct Args {
    #[arg(long, short)]
    pub session: Option<String>,

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
    pub provider: BTreeMap<String, ToolSpec>,
    pub toolset: BTreeMap<String, Toolset>,
}

/// Configuration to access a tool provider
#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
#[serde(tag = "type")]
pub enum ToolSpec {
    MCP {
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        enabled: bool,

        #[serde(default)]
        preface: Option<String>,

        #[serde(default)]
        dir: Option<PathBuf>,

        command: String,

        #[serde(default)]
        args: Vec<String>,
    },
}

impl Default for ToolSpec {
    fn default() -> Self {
        ToolSpec::MCP {
            enabled: false,
            preface: None,
            dir: None,
            command: String::new(),
            args: Vec::new(),
        }
    }
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
