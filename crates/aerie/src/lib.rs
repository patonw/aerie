#[cfg(feature = "native")]
pub mod agent;
#[cfg(feature = "wip")]
pub mod app;
pub mod chat;
pub mod config;
pub mod events;
pub mod logging;
pub mod pipeline;
#[cfg(feature = "native")]
pub mod storage;
pub mod toolbox;
pub mod transmute;
#[cfg(feature = "ui")]
pub mod ui;
pub mod utils;
pub mod workflow;

// Re-exports for custom projects
pub use anyhow;
pub use arc_swap;
pub use decorum;
#[cfg(feature = "ui")]
pub use egui;
pub use egui_snarl as snarl;
pub use im;
pub use inventory;
#[cfg(feature = "wip")]
pub use rig_dynclient::{self, rig_agent, rig_core, rmcp};
pub use serde;
pub use serde_json;
pub use tracing;
pub use typetag;

pub mod rig {
    pub use rig_dynclient::rig::*;
    pub use rig_dynclient::rig_agent::agent;
    pub use rig_dynclient::rig_core;
}
