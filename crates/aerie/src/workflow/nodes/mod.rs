use delegate::delegate;
#[cfg(feature = "ui")]
use egui::Ui;
// TODO: redefine NodeId and remove dependency on snarl
use egui_snarl::NodeId;
#[cfg(feature = "ui")]
use egui_snarl::Snarl;
use serde::{Deserialize, Serialize};
use std::{
    hash::Hash,
    ops::{Deref, DerefMut},
};

pub mod agent;
pub mod chat;
pub mod history;
pub mod json;
pub mod misc;
pub mod primatives;
pub mod scaffold;
pub mod scripting;
pub mod subgraph;

pub use agent::*;
pub use chat::*;
pub use history::*;
pub use json::*;
pub use misc::*;
pub use primatives::*;
pub use scaffold::*;
pub use subgraph::*;

pub const MIN_WIDTH: f32 = 128.0;
pub const MIN_HEIGHT: f32 = 32.0;

use crate::workflow::{DynNode, FlexNode, RunContext, Value, ValueKind, WorkflowError};

#[cfg(feature = "ui")]
use crate::workflow::{EditContext, UiNode};

#[derive(Debug, Clone, Eq, Serialize, Deserialize)]
pub struct WorkNode(pub Box<dyn FlexNode>);

impl PartialEq for WorkNode {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl Hash for WorkNode {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

impl<T: FlexNode> From<T> for WorkNode {
    fn from(value: T) -> Self {
        Self(Box::new(value))
    }
}

impl WorkNode {
    delegate! {
        to self.0 {
            #[call(deref)]
            pub fn as_dyn(&self) -> &dyn DynNode;

            #[call(deref_mut)]
            pub fn as_dyn_mut(&mut self) -> &mut dyn DynNode;

            pub fn execute(&mut self, ctx: &RunContext, node_id: NodeId, inputs: Vec<Option<Value>>,) -> Result<Vec<Value>, WorkflowError>;
        }
    }

    #[cfg(feature = "ui")]
    delegate! {
        to self.0 {
            #[call(deref)]
            pub fn as_ui(&self) -> &dyn UiNode;

            #[call(deref_mut)]
            pub fn as_ui_mut(&mut self) -> &mut dyn UiNode;
        }
    }

    pub fn kind(&self) -> &str {
        let full_name = self.0.as_ref().node_type();
        full_name.split("::").last().unwrap_or(full_name)
    }

    #[inline]
    pub fn as_node<T: FlexNode>(&self) -> Option<&T> {
        self.0.as_ref().downcast_ref::<T>()
    }

    #[inline]
    pub fn as_node_mut<T: FlexNode>(&mut self) -> Option<&mut T> {
        self.0.as_mut().downcast_mut::<T>()
    }

    #[inline]
    pub fn is_subgraph(&self) -> bool {
        self.0.as_ref().downcast_ref::<Subgraph>().is_some()
    }

    #[inline]
    pub fn is_start(&self) -> bool {
        self.0.as_ref().downcast_ref::<Start>().is_some()
    }

    #[inline]
    pub fn is_finish(&self) -> bool {
        self.0.as_ref().downcast_ref::<Finish>().is_some()
    }

    #[inline]
    pub fn is_output(&self) -> bool {
        self.0.as_ref().downcast_ref::<OutputNode>().is_some()
    }

    #[inline]
    pub fn is_preview(&self) -> bool {
        self.0.as_ref().downcast_ref::<Preview>().is_some()
    }

    #[inline]
    pub fn is_comment(&self) -> bool {
        self.0.as_ref().downcast_ref::<CommentNode>().is_some()
    }
    #[inline]
    pub fn is_protected(&self) -> bool {
        self.is_start() || self.is_finish()
    }

    #[inline]
    pub fn is_eager(&self) -> bool {
        self.0.as_ref().downcast_ref::<Select>().is_some()
    }
}

#[cfg(feature = "ui")]
pub struct GraphSubmenu(
    pub &'static str,
    pub fn(&mut Ui, &mut Snarl<WorkNode>, egui::Pos2),
);

#[cfg(feature = "ui")]
inventory::collect!(GraphSubmenu);
