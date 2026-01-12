use arc_swap::ArcSwap;
use decorum::{E32, E64};
use egui::{Color32, Stroke};
use egui_snarl::{
    InPinId, Node as SnarlNode, NodeId, OutPinId, Snarl,
    ui::{PinInfo, WireStyle},
};
use either::Either;
use itertools::Itertools as _;
use jsonschema::ValidationError;
use kinded::Kinded;
use rig::{
    message::Message,
    tool::{ToolSetError, server::ToolServerError},
};
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;
use std::{
    borrow::Cow,
    fmt::Debug,
    hash::Hash,
    sync::{Arc, atomic::AtomicBool},
};
use thiserror::Error;
use typed_builder::TypedBuilder;
use uuid::Uuid;

use crate::{
    AgentFactory, ChatHistory, ToolSelector, Toolbox,
    agent::AgentSpec,
    config::SeedConfig,
    transmute::Transmuter,
    ui::AppEvents,
    utils::{AtomicBuffer, ErrorList, ImmutableMapExt as _, ImmutableSetExt as _, message_text},
    workflow::{
        nodes::{Finish, Start},
        runner::{ExecState, NodeStateMap},
    },
};

pub mod nodes;
pub mod runner;
pub mod store;

pub use nodes::WorkNode;
// Note: Need to use decourm wrappers for floats in the graph to allow for hashing and equivalence,
// Since they need to satisfy Hash and Eq constraints for use in collections.
// Rust's primative floats don't allow this for fairly pedantic reasons.

// type DynFuture<T> = Pin<Box<dyn Future<Output = T>>>;

#[derive(Kinded, Debug, Clone, Serialize)]
#[kinded(derive(Hash, Serialize, Deserialize))]
pub enum Value {
    Placeholder(ValueKind),
    Failure(Arc<WorkflowError>),
    Text(String),
    Number(E64),
    Integer(i64),
    Json(Arc<serde_json::Value>), // I think this is immutable?
    Model(String),
    Agent(Arc<AgentSpec>),
    Tools(Arc<ToolSelector>),
    Chat(Arc<ChatHistory>),
    Message(Message),
}

impl Default for Value {
    fn default() -> Self {
        Value::Placeholder(ValueKind::Placeholder)
    }
}

#[allow(clippy::derivable_impls)]
impl Default for ValueKind {
    fn default() -> Self {
        ValueKind::Placeholder
    }
}

impl ValueKind {
    pub fn color(&self) -> Color32 {
        match self {
            ValueKind::Placeholder => Color32::LIGHT_GRAY,
            ValueKind::Failure => Color32::RED,
            ValueKind::Text => Color32::CYAN,
            ValueKind::Number => Color32::from_rgb(0xbb, 0x44, 0x88),
            ValueKind::Integer => Color32::from_rgb(0xbb, 0x77, 0x00),
            ValueKind::Json => Color32::from_rgb(0x42, 0xbb, 0x00),
            ValueKind::Model => Color32::LIGHT_BLUE,
            ValueKind::Agent => Color32::from_rgb(0x56, 0x78, 0xff),
            ValueKind::Tools => Color32::PURPLE,
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

#[derive(Clone, Default)]
pub struct PreviewData(pub Arc<ArcSwap<im::OrdMap<Uuid, crate::workflow::Value>>>);

impl PreviewData {
    pub fn update(&self, uuid: Uuid, value: Value) {
        self.0.rcu(|data| data.update(uuid, value.clone()));
    }

    pub fn value(&self, uuid: Uuid) -> Option<Value> {
        self.0.load().get(&uuid).cloned()
    }
}

// Copy-paste from egui_snarl::ui::pin
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AnyPin {
    Out(OutPinId),
    In(InPinId),
}

impl AnyPin {
    pub fn output(node: NodeId, output: usize) -> Self {
        AnyPin::Out(OutPinId { node, output })
    }

    pub fn input(node: NodeId, input: usize) -> Self {
        AnyPin::In(InPinId { node, input })
    }
}

#[derive(Clone, TypedBuilder)]
pub struct EditContext {
    pub toolbox: Arc<Toolbox>,

    pub events: Arc<AppEvents>,

    pub current_graph: GraphId,

    /// Ids of the parent graph and subgraph container node
    #[builder(default)]
    pub parent_id: Option<(GraphId, NodeId)>,

    #[builder(default)]
    pub previews: PreviewData,

    #[builder(default)]
    pub errors: ErrorList<anyhow::Error>,

    #[builder(default)]
    pub output_reset: Arc<ArcSwap<im::OrdSet<OutPinId>>>,

    #[builder(default=NodeId(0))]
    pub current_node: NodeId, // whoops

    #[builder(default)]
    pub edit_pin: Arc<ArcSwap<Option<AnyPin>>>,
}

impl EditContext {
    pub fn reset_out_pin(&self, pin_id: OutPinId) {
        let old_set = self.output_reset.load();
        let new_set = old_set.update(pin_id);

        self.output_reset.store(Arc::new(new_set));
    }
}

#[derive(Clone)]
pub struct OutputChannel(
    pub flume::Sender<(String, Value)>,
    pub flume::Receiver<(String, Value)>,
);

impl Default for OutputChannel {
    fn default() -> Self {
        let (sender, receiver) = flume::unbounded();
        Self(sender, receiver)
    }
}

impl OutputChannel {
    pub fn sender(&self) -> flume::Sender<(String, Value)> {
        self.0.clone()
    }

    pub fn receiver(&self) -> flume::Receiver<(String, Value)> {
        self.1.clone()
    }
}

#[derive(Clone, TypedBuilder)]
pub struct RunContext {
    pub runtime: tokio::runtime::Handle,

    pub agent_factory: AgentFactory,

    #[builder(default)]
    pub streaming: bool,

    #[builder(default)]
    pub node_state: NodeStateMap,

    #[builder(default)]
    pub previews: PreviewData,

    #[builder(default)]
    pub outputs: OutputChannel,

    #[builder(default)]
    pub transmuter: Transmuter,

    #[builder(default)]
    pub interrupt: Arc<AtomicBool>,

    /// Snapshot of the chat before the workflow is run
    #[builder(default)]
    pub history: Arc<ArcSwap<ChatHistory>>,

    #[builder(default)]
    pub seed: Option<SeedConfig>,

    #[builder(default)]
    pub scratch: Option<AtomicBuffer<Result<Message, String>>>,

    /// Final chat snapshot at the end of the workflow run that we want to keep
    #[builder(default)]
    pub response: Option<Arc<ChatHistory>>,

    #[builder(default)]
    pub errors: ErrorList<anyhow::Error>,
}

#[derive(TypedBuilder)]
pub struct RootContext {
    /// A full copy of the current graph
    #[builder(default)]
    pub graph: ShadowGraph<WorkNode>,

    #[builder(default)]
    pub model: String,

    #[builder(default)]
    pub temperature: f64,

    /// Snapshot of the chat before the workflow is run
    #[builder(default)]
    pub history: Arc<ArcSwap<ChatHistory>>,

    /// The user's prompt that initiated the workflow run
    #[builder(default)]
    pub user_prompt: String,
}

impl RootContext {
    pub fn inputs(&self) -> Result<Vec<Option<Value>>, WorkflowError> {
        let schema: serde_json::Value = if !self.graph.schema.is_empty() {
            serde_json::from_str(&self.graph.schema)
                .map_err(|_| WorkflowError::Conversion("Invalid input schema".into()))?
        } else {
            serde_json::json!({})
        };

        // TODO: Probably don't need most of these in the object
        let values = vec![
            Some(Value::Model(self.model.clone())),
            Some(Value::Number(E64::assert(self.temperature))),
            Some(Value::Chat(self.history.load().clone())),
            Some(Value::Json(Arc::new(schema))),
            Some(Value::Text(self.user_prompt.clone())),
        ];
        Ok(values)
    }
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

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct GraphId(pub Uuid);

impl Default for GraphId {
    fn default() -> Self {
        Self(Uuid::new_v4())
    }
}

impl GraphId {
    fn new() -> Self {
        Default::default()
    }
}

impl AsRef<Uuid> for GraphId {
    fn as_ref(&self) -> &Uuid {
        &self.0
    }
}

/// The shadow graph is incrementally updated when edits are made through the viewer.
/// Each change creates a new generation. The underlying collections use structure sharing
/// to make cloning-on-write cheap. This allows shadow graphs to be quickly compared using
/// top-level pointer comparison. We could also use this to support undo/redo operations,
/// though the shadow doesn't currently track positions.
#[skip_serializing_none]
#[derive(Debug, Clone, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShadowGraph<T>
where
    T: Clone + PartialEq + Debug,
{
    #[serde(default)]
    pub uuid: GraphId,

    pub nodes: im::OrdMap<NodeId, MetaNode<T>>,
    pub wires: im::OrdSet<Wire>,

    #[serde(default, skip_serializing_if = "im::OrdSet::is_empty")]
    pub disabled: im::OrdSet<NodeId>,

    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: Arc<String>,

    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub schema: Arc<String>,

    pub start: Option<NodeId>,

    pub finish: Option<NodeId>,
}

impl<T: Clone + PartialEq + Debug> ShadowGraph<T> {
    pub fn from_snarl(snarl: &Snarl<T>) -> Self {
        let mut baseline = Self::empty();

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

impl Default for ShadowGraph<WorkNode> {
    fn default() -> Self {
        let bytes = include_bytes!("../../tutorial/workflows/__default__.yml");
        serde_yml::from_slice::<Self>(bytes).expect("Cannot load the default graph")
    }
}

// TODO: get a handle on generics here
impl TryFrom<ShadowGraph<WorkNode>> for Snarl<WorkNode> {
    type Error = anyhow::Error;

    fn try_from(that: ShadowGraph<WorkNode>) -> Result<Self, Self::Error> {
        // Well, this is less than ideal. Can't construct snarl nodes, let alone programmatically
        // recreate the graph with same ids. API only allows us to insert/remove with inner data.
        let value = serde_json::to_value(&that)?;

        let mut snarl: Snarl<WorkNode> = serde_json::from_value(value)?;

        // Transfer transient state back to nodes
        for (node_id, meta) in &that.nodes {
            snarl[*node_id] = meta.value.clone();
        }

        Ok(snarl)
    }
}

impl<T> ShadowGraph<T>
where
    T: PartialEq + Clone + std::fmt::Debug,
{
    pub fn empty() -> Self {
        Self {
            uuid: Default::default(),
            nodes: Default::default(),
            wires: Default::default(),
            disabled: Default::default(),
            description: Default::default(),
            schema: Default::default(),
            start: Default::default(),
            finish: Default::default(),
        }
    }

    /// Quickly see if the collections have the same memory address.
    /// Does not account for identical copies in different addresses.
    /// Use the standard comparator to do a deep check instead.
    pub fn fast_eq(&self, other: &Self) -> bool {
        self.nodes.ptr_eq(&other.nodes)
            && self.wires.ptr_eq(&other.wires)
            && self.disabled.ptr_eq(&other.disabled)
            && Arc::ptr_eq(&self.description, &other.description)
            && Arc::ptr_eq(&self.schema, &other.schema)
    }

    #[must_use]
    pub fn with_node(&self, id: &NodeId, t: Option<&SnarlNode<T>>) -> Self {
        let t = t.unwrap();
        let nodes = self.nodes.with(id, &MetaNode::from(t.clone()));
        if nodes.ptr_eq(&self.nodes) {
            self.clone()
        } else {
            tracing::trace!("Nodes changed. Before {:?} after {nodes:?}", self.nodes);

            Self {
                nodes,
                ..self.clone()
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
                wires,
                ..self.clone()
            }
        }
    }

    #[must_use]
    pub fn without_wire(&self, out_pin: OutPinId, in_pin: InPinId) -> Self {
        let wire = (out_pin, in_pin).into();
        if self.wires.contains(&wire) {
            Self {
                wires: self.wires.without(&wire),
                ..self.clone()
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
    pub fn drop_inputs(&self, pin: egui_snarl::InPinId) -> Self {
        let wires = self
            .wires
            .iter()
            .filter(|wire| wire.in_pin != pin)
            .cloned()
            .collect::<im::OrdSet<Wire>>();

        Self {
            wires,
            ..self.clone()
        }
    }

    #[must_use]
    pub fn drop_outputs(&self, pin: OutPinId) -> Self {
        let wires = self
            .wires
            .iter()
            .filter(|wire| wire.out_pin != pin)
            .cloned()
            .collect::<im::OrdSet<Wire>>();

        Self {
            wires,
            ..self.clone()
        }
    }

    /// Decrements successive outputs to fill the gap left by removal
    #[must_use]
    pub fn shift_inputs(&self, pin: egui_snarl::InPinId) -> Self {
        let wires = self
            .wires
            .iter()
            .filter(|wire| wire.in_pin != pin)
            .map(|wire| {
                if wire.in_pin.node != pin.node || wire.in_pin.input < pin.input {
                    *wire
                } else {
                    Wire {
                        in_pin: InPinId {
                            node: pin.node,
                            input: wire.in_pin.input - 1,
                        },
                        out_pin: wire.out_pin,
                    }
                }
            })
            .collect::<im::OrdSet<Wire>>();

        Self {
            wires,
            ..self.clone()
        }
    }

    /// Decrements successive outputs to fill the gap left by removal
    #[must_use]
    pub fn shift_outputs(&self, pin: OutPinId) -> Self {
        let wires = self
            .wires
            .iter()
            .filter(|wire| wire.out_pin != pin)
            .map(|wire| {
                if wire.out_pin.node != pin.node || wire.out_pin.output < pin.output {
                    *wire
                } else {
                    Wire {
                        out_pin: OutPinId {
                            node: pin.node,
                            output: wire.out_pin.output - 1,
                        },
                        in_pin: wire.in_pin,
                    }
                }
            })
            .collect::<im::OrdSet<Wire>>();

        Self {
            wires,
            ..self.clone()
        }
    }

    #[must_use]
    pub fn swap_inputs(&self, a: InPinId, b: InPinId) -> Self {
        let wires = self
            .wires
            .iter()
            .map(|wire| {
                if wire.in_pin == a {
                    Wire {
                        out_pin: wire.out_pin,
                        in_pin: b,
                    }
                } else if wire.in_pin == b {
                    Wire {
                        out_pin: wire.out_pin,
                        in_pin: a,
                    }
                } else {
                    *wire
                }
            })
            .collect::<im::OrdSet<Wire>>();

        Self {
            wires,
            ..self.clone()
        }
    }

    #[must_use]
    pub fn swap_outputs(&self, a: OutPinId, b: OutPinId) -> Self {
        let wires = self
            .wires
            .iter()
            .map(|wire| {
                if wire.out_pin == a {
                    Wire {
                        out_pin: b,
                        in_pin: wire.in_pin,
                    }
                } else if wire.out_pin == b {
                    Wire {
                        out_pin: a,
                        in_pin: wire.in_pin,
                    }
                } else {
                    *wire
                }
            })
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
            disabled: self.disabled.without(&id),
            ..self.clone()
        }
    }

    pub fn disable_node(&self, id: NodeId) -> Self {
        Self {
            disabled: self.disabled.with(&id),
            ..self.clone()
        }
    }

    pub fn with_description(&self, desc: &str) -> Self {
        Self {
            description: Arc::new(desc.to_string()),
            ..self.clone()
        }
    }

    pub fn with_schema(&self, schema: &str) -> Self {
        Self {
            schema: Arc::new(schema.to_string()),
            ..self.clone()
        }
    }
}

impl ShadowGraph<WorkNode> {
    pub fn repair(&self) -> Self {
        let mut target = self.clone();
        if let Some(id) = self.nodes.iter().find_map(|(id, n)| match &n.value {
            WorkNode::Start(_) => Some(id),
            _ => None,
        }) {
            target.start = Some(*id);
        }

        if let Some(id) = self.nodes.iter().find_map(|(id, n)| match &n.value {
            WorkNode::Finish(_) => Some(id),
            _ => None,
        }) {
            target.finish = Some(*id);
        }

        let keep = target.nodes.keys().cloned().collect_vec();
        target.wires = target
            .wires
            .into_iter()
            .filter(|w| keep.contains(&w.out_pin.node) && keep.contains(&w.in_pin.node))
            .collect();

        target
    }

    pub fn start_node(&self) -> Option<&Start> {
        if let Some(node_id) = &self.start
            && let Some(WorkNode::Start(node)) = self.nodes.get(node_id).map(|n| &n.value)
        {
            Some(node)
        } else if let Some(node) = self.nodes.values().find_map(|n| match &n.value {
            WorkNode::Start(node) => Some(node),
            _ => None,
        }) {
            Some(node)
        } else {
            None
        }
    }

    pub fn finish_node(&self) -> Option<&Finish> {
        if let Some(node_id) = &self.finish
            && let Some(WorkNode::Finish(node)) = self.nodes.get(node_id).map(|n| &n.value)
        {
            Some(node)
        } else if let Some(node) = self.nodes.values().find_map(|n| match &n.value {
            WorkNode::Finish(node) => Some(node),
            _ => None,
        }) {
            Some(node)
        } else {
            None
        }
    }

    pub fn start_kinds(&self) -> impl Iterator<Item = ValueKind> {
        if let Some(start) = self.start_node() {
            Either::Right((0..start.outputs()).map(|i| start.out_kind(i)))
        } else {
            Either::Left([].into_iter())
        }
    }

    pub fn finish_kinds(&self) -> impl Iterator<Item = ValueKind> {
        if let Some(finish) = self.finish_node() {
            Either::Right((0..finish.inputs()).map(|i| finish.in_kinds(i)[0]))
        } else {
            Either::Left([].into_iter())
        }
    }
}

pub trait DynNode {
    fn priority(&self) -> usize {
        5000
    }

    fn value(&self, out_pin: usize) -> Value {
        Value::Placeholder(self.out_kind(out_pin))
    }

    fn inputs(&self) -> usize {
        1
    }

    fn outputs(&self) -> usize {
        1
    }

    // TODO: stop using this in execute. Nodes shouldn't store results internally
    fn collect_outputs(&self) -> Vec<Value> {
        (0..self.outputs()).map(|i| self.value(i)).collect()
    }

    #[expect(unused_variables)]
    // We're more concerned about type validation here than updating UI visuals
    fn in_kinds(&'_ self, in_pin: usize) -> Cow<'_, [ValueKind]> {
        Cow::Borrowed(ValueKind::all())
    }

    fn validate(&self, inputs: &[Option<Value>]) -> Result<(), WorkflowError> {
        tracing::debug!("Validating inputs for {}", std::any::type_name_of_val(self));
        tracing::trace!("Input values: {inputs:?}");

        if inputs.len() != self.inputs() {
            Err(WorkflowError::Required(vec![
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
            Err(WorkflowError::Required(errors))
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
            tracing::warn!(
                "Refusing to connect {kind:?} to {in_pin:?} accepting {:?}",
                self.in_kinds(in_pin)
            );
            Err("Not allowed!".into())
        } else {
            Ok(())
        }
    }

    fn execute(
        &mut self,
        ctx: &RunContext,
        node_id: NodeId,
        inputs: Vec<Option<Value>>,
    ) -> Result<Vec<Value>, WorkflowError> {
        let _ = (ctx, node_id, inputs);

        Ok(self.collect_outputs())
    }
}

pub trait UiNode: DynNode {
    /// Callback to enforce uniqueness after a node is duplicated using copy/paste
    fn on_paste(&mut self) {}

    /// Supply placeholder values to display in UI outside of executions
    fn preview(&self, out_pin: usize) -> Value {
        self.value(out_pin)
    }

    fn title_mut(&mut self) -> Option<&mut String> {
        None
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

    fn has_footer(&self) -> bool {
        false
    }

    #[expect(unused_variables)]
    fn show_footer(&mut self, ui: &mut egui::Ui, ctx: &EditContext) {}

    fn ghost_pin(&self, base_color: egui::Color32) -> PinInfo {
        PinInfo::circle()
            .with_stroke(Stroke::NONE)
            .with_fill(base_color.gamma_multiply(0.25))
            .with_wire_style(WireStyle::Bezier5)
    }

    #[expect(unused_variables)]
    fn show_input(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &EditContext,
        pin_id: usize,
        remote: Option<Value>,
    ) -> PinInfo {
        self.in_kinds(pin_id).first().unwrap().default_pin()
    }

    #[expect(unused_variables)]
    fn show_output(&mut self, ui: &mut egui::Ui, ctx: &EditContext, pin_id: usize) -> PinInfo {
        self.out_kind(pin_id).default_pin()
    }
}

#[derive(Debug, Error, Kinded)]
#[kinded(derive(Hash, Serialize, Deserialize))]
pub enum WorkflowError {
    #[error("Input required {0:?}")]
    Required(Vec<String>),

    #[error("Cannot convert data: {0:?}")]
    Conversion(String),

    #[error("Error while invoking provider")]
    Provider(#[source] anyhow::Error),

    #[error(
        "Expected a function call, but received none.\nPlease use one of the provided tools to submit your response."
    )]
    MissingToolCall,

    #[error("Error while invoking tool")]
    ToolCall(#[source] ToolSetError),

    #[error("Error while invoking tool")]
    ToolServerCall(#[source] ToolServerError),

    #[error(
        "Value does not conform to schema.\nPlease double check that your response has the correct structure and contains the required fields."
    )]
    Validation(#[source] ValidationError<'static>),

    #[error("Interrupted")]
    Interrupted,

    #[error("timed out")]
    Timeout,

    #[error("Graph execution halted before finishing: {0:?}")]
    Unfinished(ExecState),

    #[error("Error while executing subgraph")]
    Subgraph(#[source] Arc<WorkflowError>),

    #[error("{0}")]
    Unknown(String),
}

impl Serialize for WorkflowError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.kind().serialize(serializer)
    }
}

impl From<ToolSetError> for WorkflowError {
    fn from(value: ToolSetError) -> Self {
        WorkflowError::ToolCall(value)
    }
}

impl From<ToolServerError> for WorkflowError {
    fn from(value: ToolServerError) -> Self {
        WorkflowError::ToolServerCall(value)
    }
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

pub fn write_value(mut fh: impl std::io::Write, value: &Value) -> Result<(), anyhow::Error> {
    match value {
        Value::Text(value) => {
            writeln!(fh, "{value}")?;
        }
        Value::Number(value) => {
            writeln!(fh, "{value}")?;
        }
        Value::Integer(value) => {
            writeln!(fh, "{value}")?;
        }
        Value::Json(value) => {
            serde_json::to_writer(fh, value.as_ref())?;
        }
        Value::Chat(value) => {
            let value = value.iter_msgs().map(|it| it.into_owned()).collect_vec();
            serde_yml::to_writer(fh, &value)?;
        }
        Value::Message(value) => {
            let text = message_text(value);

            writeln!(fh, "{text}")?;
        }
        _ => {
            serde_yml::to_writer(fh, &value)?;
        }
    }

    Ok(())
}
