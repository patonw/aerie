use delegate::delegate;
use egui::Ui;
use egui_snarl::{NodeId, Snarl};
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

use crate::workflow::{FlexNode, WorkflowError};

pub use super::{DynNode, EditContext, RunContext, UiNode, Value, ValueKind};

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
        // Pointless now
        // TODO: deprecate and replace with trait method calls
        to self.0 {
                #[call(deref)]
                pub fn as_dyn(&self) -> &dyn DynNode;

                #[call(deref_mut)]
                pub fn as_dyn_mut(&mut self) -> &mut dyn DynNode;

                #[call(deref)]
                pub fn as_ui(&self) -> &dyn UiNode;

                #[call(deref_mut)]
                pub fn as_ui_mut(&mut self) -> &mut dyn UiNode;

                pub fn execute(&mut self, ctx: &RunContext, node_id: NodeId, inputs: Vec<Option<Value>>,) -> Result<Vec<Value>, WorkflowError>;
        }
    }

    pub fn kind(&self) -> &str {
        // type_name_of_val(self)
        self.as_ui().title()
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

pub struct GraphSubmenu(
    pub &'static str,
    pub fn(&mut Ui, &mut Snarl<WorkNode>, egui::Pos2),
);

inventory::collect!(GraphSubmenu);
