use decorum::E64;
use egui::{Color32, Stroke};
use egui_snarl::{
    InPinId, NodeId, OutPinId, Snarl,
    ui::{PinInfo, WireStyle},
};
use kinded::Kinded;
use rig::message::Message;
use std::sync::Arc;

use crate::{
    ChatHistory, Toolbox, Toolset,
    utils::{ErrorList, ImmutableMapExt as _, ImmutableSetExt as _},
};

pub mod nodes;
pub mod runner;
pub mod store;

pub use nodes::WorkNode;
// Note: Need to use decourm wrappers for floats in the graph to allow for hashing and equivalence,
// Since they need to satisfy Hash and Eq constraints for use in collections.
// Rust's primative floats don't allow this for fairly pedantic reasons.

// type DynFuture<T> = Pin<Box<dyn Future<Output = T>>>;

#[derive(Kinded, Debug, Clone)]
pub enum Value {
    Placeholder(ValueKind),
    Text(String),
    Number(E64),
    Integer(i64),
    Json(Arc<serde_json::Value>), // I think this is immutable?
    Model(String),
    Toolset(Arc<Toolset>),
    Chat(Arc<ChatHistory>),
    Message(Message),
}

impl Default for Value {
    fn default() -> Self {
        Value::Placeholder(ValueKind::Placeholder)
    }
}

impl ValueKind {
    pub fn color(&self) -> Color32 {
        match self {
            ValueKind::Placeholder => Color32::LIGHT_GRAY,
            ValueKind::Text => Color32::CYAN,
            ValueKind::Number => Color32::LIGHT_GREEN,
            ValueKind::Integer => Color32::LIGHT_RED,
            ValueKind::Json => Color32::ORANGE,
            ValueKind::Model => Color32::LIGHT_BLUE,
            ValueKind::Toolset => Color32::PURPLE,
            ValueKind::Chat => Color32::GOLD,
            ValueKind::Message => Color32::from_rgb(0xe9, 0x74, 0x51),
        }
    }

    pub fn default_pin(&self) -> PinInfo {
        PinInfo::circle()
            .with_fill(self.color())
            .with_wire_style(WireStyle::Bezier5)
    }
}

pub struct EditContext {
    pub toolbox: Toolbox,
}

#[derive(Debug, Default)]
pub struct RunContext {
    /// Snapshot of the chat before the workflow is run
    pub history: Arc<ChatHistory>,

    /// The user's prompt that initiated the workflow run
    pub user_prompt: String,

    pub model: String,

    pub temperature: f64,

    /// Final chat snapshot at the end of the workflow run that we want to keep
    pub response: Option<Arc<ChatHistory>>,

    pub errors: ErrorList<anyhow::Error>,
}

/// The shadow graph is incrementally updated when edits are made through the viewer.
/// Each change creates a new generation. The underlying collections use structure sharing
/// to make cloning-on-write cheap. This allows shadow graphs to be quickly compared using
/// top-level pointer comparison. We could also use this to support undo/redo operations,
/// though the shadow doesn't currently track positions.
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct ShadowGraph<T> {
    pub nodes: im::HashMap<NodeId, T>,
    pub wires: im::HashSet<(OutPinId, InPinId)>,
}

impl<T: Clone + PartialEq> ShadowGraph<T> {
    pub fn from_snarl(snarl: &Snarl<T>) -> Self {
        let mut baseline = Self::default();

        // Initialize with bulk mutators since we're not tracking generations yet
        baseline
            .nodes
            .extend(snarl.node_ids().map(|(id, node)| (id, node.clone())));

        baseline.wires.extend(snarl.wires());

        baseline
    }
}

impl<T> Default for ShadowGraph<T> {
    fn default() -> Self {
        Self {
            nodes: Default::default(),
            wires: Default::default(),
        }
    }
}

impl<T: PartialEq + Clone + std::fmt::Debug> ShadowGraph<T> {
    /// Quickly see if the collections have the same memory address.
    /// Does not account for identical copies in different addresses.
    /// Use the standard comparator to do a deep check instead.
    pub fn fast_eq(&self, other: &Self) -> bool {
        self.nodes.ptr_eq(&other.nodes) && self.wires.ptr_eq(&other.wires)
    }

    #[must_use]
    pub fn with_node(&self, id: &NodeId, t: &T) -> Self {
        let nodes = self.nodes.with(id, t);
        if nodes.ptr_eq(&self.nodes) {
            self.clone()
        } else {
            // tracing::info!("Updating node {t:?}");
            Self {
                nodes,
                wires: self.wires.clone(),
            }
        }
    }

    #[must_use]
    pub fn without_node(&self, id: &NodeId) -> Self {
        let Self { nodes, wires } = self;
        if nodes.contains_key(id) {
            Self {
                nodes: nodes.without(id),
                wires: wires.clone(),
            }
        } else {
            self.clone()
        }
    }

    #[must_use]
    pub fn with_wire(&self, out_pin: OutPinId, in_pin: InPinId) -> Self {
        let wire = (out_pin, in_pin);
        let wires = self.wires.with(&wire);
        if wires.ptr_eq(&self.wires) {
            self.clone()
        } else {
            Self {
                nodes: self.nodes.clone(),
                wires,
            }
        }
    }

    #[must_use]
    pub fn without_wire(&self, out_pin: OutPinId, in_pin: InPinId) -> Self {
        let wire = (out_pin, in_pin);
        if self.wires.contains(&wire) {
            Self {
                nodes: self.nodes.clone(),
                wires: self.wires.without(&wire),
            }
        } else {
            self.clone()
        }
    }
}

pub trait DynNode {
    // Moved to impl of each struct to avoid dealing with boxing
    // /// Update computed values with inputs from remotes.
    // fn forward(&mut self, inputs: Vec<Option<Value>>) -> DynFuture<Result<(), Vec<String>>> {
    //     Box::pin(async { Ok(()) })
    // }

    /// Clear values set by the connected pin so we can leave widget connected values alone.
    #[expect(unused_variables)]
    fn reset(&mut self, in_pin: usize) {}

    #[expect(unused_variables)]
    fn value(&self, out_pin: usize) -> Value {
        Default::default()
    }

    fn inputs(&self) -> usize {
        1
    }

    fn outputs(&self) -> usize {
        1
    }

    #[expect(unused_variables)]
    // We're more concerned about type validation here than updating UI visuals
    fn in_kinds(&self, in_pin: usize) -> &'static [ValueKind] {
        ValueKind::all()
    }

    #[expect(unused_variables)]
    // We're more concerned about type validation here than updating UI visuals
    fn out_kind(&self, out_pin: usize) -> ValueKind {
        ValueKind::Placeholder
    }

    fn connect(&self, in_pin: usize, kind: ValueKind) -> Result<(), String> {
        if !self.in_kinds(in_pin).contains(&kind) {
            Err("Not allowed!".into())
        } else {
            Ok(())
        }
    }
}

pub trait UiNode: DynNode {
    /// Supply placeholder values to display in UI outside of executions
    fn preview(&self, out_pin: usize) -> Value {
        self.value(out_pin)
    }

    fn title(&self) -> String {
        String::new()
    }

    fn has_body(&self) -> bool {
        false
    }

    #[expect(unused_variables)]
    fn show_body(&mut self, ui: &mut egui::Ui, ctx: &EditContext) {}

    fn ghost_pin(&self, base_color: egui::Color32) -> PinInfo {
        PinInfo::circle()
            .with_stroke(Stroke::NONE)
            .with_fill(base_color.gamma_multiply(0.25))
            .with_wire_style(WireStyle::Bezier5)
    }

    fn default_pin(&self) -> PinInfo {
        PinInfo::circle()
            .with_fill(egui::Color32::GRAY)
            .with_wire_style(WireStyle::Bezier5)
    }

    #[expect(unused_variables)]
    fn show_input(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &EditContext,
        pin_id: usize,
        remote: Option<Value>, // TODO: rename to "wired" this should be ValueKind!
    ) -> PinInfo {
        self.in_kinds(pin_id).first().unwrap().default_pin()
    }

    #[expect(unused_variables)]
    fn show_output(&mut self, ui: &mut egui::Ui, ctx: &EditContext, pin_id: usize) -> PinInfo {
        self.out_kind(pin_id).default_pin()
    }
}
