pub mod agent;
pub mod app;
pub mod chat;
pub mod config;
pub mod logging;
pub mod pipeline;
pub mod storage;
pub mod toolbox;
pub mod transmute;
pub mod utils;
pub mod workflow;

pub use agent::AgentFactory;
pub use chat::{ChatContent, ChatEntry, ChatHistory, ChatSession};
pub use config::{Settings, ToolSelector, ToolSpec};
pub use logging::{LogChannelLayer, LogEntry};
pub use pipeline::{Pipeline, Workstep};
pub use toolbox::{ToolProvider, Toolbox};

// Re-exports for custom projects
pub use anyhow;
pub use arc_swap;
pub use decorum;
pub use egui_snarl as snarl;
pub use im;
pub use inventory;
pub use rig_dynclient::{self, rig, rmcp};
pub use serde;
pub use serde_json;
pub use tracing;
pub use typetag;

#[cfg(feature = "ui")]
pub mod ui;

#[cfg(feature = "ui")]
pub use egui;
