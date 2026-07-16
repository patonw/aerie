use egui_snarl::{InPinId, NodeId, OutPinId};
use uuid::Uuid;

use crate::{
    utils::PriorityQueue,
    workflow::{AnyPin, GraphId, ValueKind},
};

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum AppEvent {
    EnterSubgraph(NodeId),
    LeaveSubgraph(usize),

    DisableNode(GraphId, NodeId),

    InPinCreated(GraphId, NodeId, ValueKind),
    OutPinCreated(GraphId, NodeId, ValueKind),

    PinRenamed(GraphId, AnyPin, String),

    /// Removes a pin from a node of a graph. Graph must be in the current ViewStack.
    PinRemoved(GraphId, AnyPin),

    /// Swaps the wires of two pins in a graph. Graph must be in the current ViewStack.
    /// Pins must both be inputs or outputs.
    SwapInputs(GraphId, InPinId, InPinId),
    SwapOutputs(GraphId, OutPinId, OutPinId),

    // User requested to run the current workflow
    UserRunWorkflow,

    ToolsChanged,

    NodesChanged(GraphId, im::OrdSet<NodeId>),
    RerunNodes(GraphId, Vec<NodeId>),

    SetPrompt(String),

    Freeze(Option<bool>),
    Undo,
    Redo,

    ProgressBegin(Uuid, usize),
    ProgressAdd(Uuid, usize),
    ProgressEnd(Uuid),
}

impl AppEvent {
    pub fn priority(&self) -> i64 {
        use AppEvent::*;
        match self {
            EnterSubgraph(_) | LeaveSubgraph(_) => -100,
            UserRunWorkflow | SetPrompt(_) => -200,
            _ => 0,
        }
    }
}

impl PartialOrd for AppEvent {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for AppEvent {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.priority().cmp(&other.priority())
    }
}

pub type AppEvents = PriorityQueue<AppEvent>;
