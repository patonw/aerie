use arc_swap::ArcSwap;
use decorum::{E32, E64};
use egui::{Color32, Stroke};
use egui_snarl::{
    InPinId, Node as SnarlNode, NodeId, OutPinId, Snarl,
    ui::{PinInfo, WireStyle},
};
use itertools::Itertools as _;
use kinded::Kinded;
use rig::message::Message;
use serde::{Deserialize, Serialize};
use std::{hash::Hash, sync::Arc};
use thiserror::Error;
use typed_builder::TypedBuilder;

use crate::{
    AgentFactory, ChatHistory, Toolbox, Toolset,
    agent::AgentSpec,
    transmute::Transmuter,
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
    Agent(Arc<AgentSpec>),
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
            ValueKind::Number => Color32::from_rgb(0xff, 0x56, 0x78),
            ValueKind::Integer => Color32::from_rgb(0xff, 0x65, 0x43),
            ValueKind::Json => Color32::BROWN,
            ValueKind::Model => Color32::LIGHT_BLUE,
            ValueKind::Agent => Color32::from_rgb(0x56, 0x78, 0xff),
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
    pub toolbox: Arc<Toolbox>,
}

#[derive(TypedBuilder)]
pub struct RunContext {
    pub agent_factory: AgentFactory,

    pub transmuter: Transmuter,

    /// Snapshot of the chat before the workflow is run
    #[builder(default)]
    pub history: Arc<ArcSwap<ChatHistory>>,

    /// The user's prompt that initiated the workflow run
    #[builder(default)]
    pub user_prompt: String,

    #[builder(default)]
    pub model: String,

    #[builder(default)]
    pub temperature: f64,

    /// Final chat snapshot at the end of the workflow run that we want to keep
    #[builder(default)]
    pub response: Option<Arc<ChatHistory>>,

    #[builder(default)]
    pub errors: ErrorList<anyhow::Error>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetaNode<T> {
    /// Node generic value.
    pub value: T,

    /// Position of the top-left corner of the node.
    /// This does not include frame margin.
    pub pos: egui::Pos2,

    /// Flag indicating that the node is open - not collapsed.
    pub open: bool,
}

impl<T> Hash for MetaNode<T>
where
    T: Hash + PartialEq,
{
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.value.hash(state);
        let egui::Pos2 { x, y } = self.pos;
        let (x, y): (E32, E32) = (E32::assert(x), E32::assert(y));
        x.hash(state);
        y.hash(state);
        self.open.hash(state);
    }
}

impl<T> From<SnarlNode<T>> for MetaNode<T> {
    fn from(other: SnarlNode<T>) -> Self {
        Self {
            value: other.value,
            pos: other.pos,
            open: other.open,
        }
    }
}

// Copy of egui_snarl::Wire
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Wire {
    pub out_pin: OutPinId,
    pub in_pin: InPinId,
}

impl From<(OutPinId, InPinId)> for Wire {
    fn from((out_pin, in_pin): (OutPinId, InPinId)) -> Self {
        Self { out_pin, in_pin }
    }
}

/// The shadow graph is incrementally updated when edits are made through the viewer.
/// Each change creates a new generation. The underlying collections use structure sharing
/// to make cloning-on-write cheap. This allows shadow graphs to be quickly compared using
/// top-level pointer comparison. We could also use this to support undo/redo operations,
/// though the shadow doesn't currently track positions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShadowGraph<T>
where
    T: Clone + PartialEq,
{
    pub nodes: im::OrdMap<NodeId, MetaNode<T>>,
    pub wires: im::OrdSet<Wire>,

    #[serde(default, skip_serializing_if = "im::OrdSet::is_empty")]
    pub disabled: im::OrdSet<NodeId>,
}

impl<T: Clone + PartialEq> ShadowGraph<T> {
    pub fn from_snarl(snarl: &Snarl<T>) -> Self {
        let mut baseline = Self::default();

        // Initialize with bulk mutators since we're not tracking generations yet
        baseline.nodes.extend(
            snarl
                .nodes_ids_data()
                .map(|(id, node)| (id, MetaNode::from(node.clone()))),
        );

        baseline.wires.extend(snarl.wires().map(Wire::from));

        baseline
    }
}

impl<T> Default for ShadowGraph<T>
where
    T: Clone + PartialEq,
{
    fn default() -> Self {
        Self {
            nodes: Default::default(),
            wires: Default::default(),
            disabled: Default::default(),
        }
    }
}

impl<T> From<&Snarl<T>> for ShadowGraph<T>
where
    T: Clone + PartialEq,
{
    fn from(value: &Snarl<T>) -> Self {
        ShadowGraph::from_snarl(value)
    }
}

// TODO: get a handle on generics here
impl TryFrom<ShadowGraph<WorkNode>> for Snarl<WorkNode> {
    type Error = anyhow::Error;

    fn try_from(that: ShadowGraph<WorkNode>) -> Result<Self, Self::Error> {
        // Well, this is less than ideal. Can't construct snarl nodes, let alone programmatically
        // recreate the graph with same ids. API only allows us to insert/remove with inner data.
        let value = serde_json::to_value(that)?;

        let snarl = serde_json::from_value(value)?;
        Ok(fixup_workflow(snarl))
    }
}

impl<T> ShadowGraph<T>
where
    T: PartialEq + Clone,
{
    /// Quickly see if the collections have the same memory address.
    /// Does not account for identical copies in different addresses.
    /// Use the standard comparator to do a deep check instead.
    pub fn fast_eq(&self, other: &Self) -> bool {
        self.nodes.ptr_eq(&other.nodes)
            && self.wires.ptr_eq(&other.wires)
            && self.disabled.ptr_eq(&other.disabled)
    }

    #[must_use]
    pub fn with_node(&self, id: &NodeId, t: Option<&SnarlNode<T>>) -> Self {
        let t = t.unwrap();
        let nodes = self.nodes.with(id, &MetaNode::from(t.clone()));
        if nodes.ptr_eq(&self.nodes) {
            self.clone()
        } else {
            Self {
                nodes,
                wires: self.wires.clone(),
                disabled: self.disabled.clone(),
            }
        }
    }

    #[must_use]
    pub fn without_node(&self, id: &NodeId) -> Self {
        let Self { nodes, .. } = self;
        if nodes.contains_key(id) {
            Self {
                nodes: nodes.without(id),
                ..self.drop_io(*id)
            }
        } else {
            self.clone()
        }
    }

    #[must_use]
    pub fn with_wire(&self, out_pin: OutPinId, in_pin: InPinId) -> Self {
        let wire = (out_pin, in_pin).into();
        let wires = self.wires.with(&wire);
        if wires.ptr_eq(&self.wires) {
            self.clone()
        } else {
            Self {
                nodes: self.nodes.clone(),
                wires,
                disabled: self.disabled.clone(),
            }
        }
    }

    #[must_use]
    pub fn without_wire(&self, out_pin: OutPinId, in_pin: InPinId) -> Self {
        let wire = (out_pin, in_pin).into();
        if self.wires.contains(&wire) {
            Self {
                nodes: self.nodes.clone(),
                wires: self.wires.without(&wire),
                disabled: self.disabled.clone(),
            }
        } else {
            self.clone()
        }
    }

    #[must_use]
    pub fn drop_io(&self, node: NodeId) -> Self {
        let wires = self
            .wires
            .iter()
            .filter(|wire| wire.in_pin.node != node && wire.out_pin.node != node)
            .cloned()
            .collect::<im::OrdSet<_>>();

        Self {
            wires,
            ..self.clone()
        }
    }

    #[must_use]
    pub fn drop_inputs(&self, pin: &egui_snarl::InPin) -> Self {
        let wires = self
            .wires
            .iter()
            .filter(|wire| wire.in_pin != pin.id)
            .cloned()
            .collect::<im::OrdSet<Wire>>();

        Self {
            wires,
            ..self.clone()
        }
    }

    #[must_use]
    pub fn drop_outputs(&self, pin: &egui_snarl::OutPin) -> Self {
        let wires = self
            .wires
            .iter()
            .filter(|wire| wire.out_pin != pin.id)
            .cloned()
            .collect::<im::OrdSet<Wire>>();

        Self {
            wires,
            ..self.clone()
        }
    }

    pub fn is_disabled(&self, id: NodeId) -> bool {
        self.disabled.contains(&id)
    }

    pub fn enable_node(&self, id: NodeId) -> Self {
        Self {
            nodes: self.nodes.clone(),
            wires: self.wires.clone(),
            disabled: self.disabled.without(&id),
        }
    }

    pub fn disable_node(&self, id: NodeId) -> Self {
        Self {
            nodes: self.nodes.clone(),
            wires: self.wires.clone(),
            disabled: self.disabled.with(&id),
        }
    }
}

// TODO: help link property
pub trait DynNode {
    // Moved to impl of each struct to avoid dealing with boxing
    // /// Update computed values with inputs from remotes.
    // fn forward(&mut self, inputs: Vec<Option<Value>>) -> DynFuture<Result<(), Vec<String>>> {
    //     Box::pin(async { Ok(()) })
    // }

    /// Clear values set by the connected pin so we can leave widget connected values alone.
    #[expect(unused_variables)]
    fn reset(&mut self, in_pin: usize) {}

    fn priority(&self) -> usize {
        100
    }

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

    fn validate(&self, inputs: &[Option<Value>]) -> Result<(), WorkflowError> {
        tracing::trace!(
            "Validating inputs for {} -- {inputs:?}",
            std::any::type_name_of_val(self)
        );

        if inputs.len() != self.inputs() {
            Err(WorkflowError::Validation(vec![
                "Incorrect number of inputs".into(),
            ]))?
        }

        let errors = inputs
            .iter()
            .enumerate()
            .filter(|(_, input)| input.is_some())
            .filter_map(|(i, input)| {
                let Some(value) = input else { unreachable!() };
                let kinds = self.in_kinds(i);
                if kinds.contains(&value.kind()) {
                    None::<String>
                } else {
                    Some(format!("{value:?} is not one of {kinds:?}"))
                }
            })
            .collect_vec();

        if !errors.is_empty() {
            Err(WorkflowError::Validation(errors))
        } else {
            Ok(())
        }
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

    fn title(&self) -> &str {
        ""
    }

    fn tooltip(&self) -> &str {
        ""
    }

    fn help_link(&self) -> &str {
        ""
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

#[derive(Debug, Error)]
pub enum WorkflowError {
    #[error("Validation error")]
    Validation(Vec<String>),

    #[error("Input error {0:?}")]
    Input(Vec<String>),

    #[error("Error while invoking provider")]
    Provider(#[source] anyhow::Error),

    #[error("{0}")]
    Unknown(String),
}

impl From<anyhow::Error> for WorkflowError {
    fn from(value: anyhow::Error) -> Self {
        WorkflowError::Unknown(format!("anyhow... {value:?}"))
    }
}

pub fn fixup_workflow(mut snarl: Snarl<WorkNode>) -> Snarl<WorkNode> {
    tracing::debug!("Examining graph {snarl:?}");

    if snarl.nodes().count() < 1 || !snarl.nodes().any(|n| matches!(n, WorkNode::Start(_))) {
        tracing::info!("Inserting missing start node");
        snarl.insert_node(egui::pos2(0.0, 0.0), WorkNode::Start(Default::default()));
    }

    if !snarl.nodes().any(|n| matches!(n, WorkNode::Finish(_))) {
        tracing::info!("Inserting missing finish node");
        snarl.insert_node(
            egui::pos2(1000.0, 0.0),
            WorkNode::Finish(Default::default()),
        );
    }

    snarl
}
