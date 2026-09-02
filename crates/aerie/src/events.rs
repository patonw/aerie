use std::sync::Arc;

use egui_snarl::{InPinId, NodeId, OutPinId};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    config::Preferences,
    utils::PriorityQueue,
    workflow::{
        AnyPin, GraphId, ShadowGraph, ValueKind, WorkNode, Workflow,
        runner::{ExecId, ExecState},
    },
};

#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
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

    GraphSnapshot(ShadowGraph<WorkNode>),

    CreateWorkflow,
    ImportWorkflow(String, Vec<u8>),
    DeleteWorkflow(String),
    SaveWorkflow,
    WorkflowSnapshot(Workflow),
    SwitchWorkflow(String),
    RenameWorkflow(String, String),
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

    UpdatePreferences(Arc<Preferences>),
    ClearModels,
    TouchModel(String),

    CreateSession,
    SwitchSession(String),
    RenameSession(String, String),
    ImportSession(String, Vec<u8>),
    DeleteSession(String),

    SwitchBranch(String),
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

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub enum NodeStateEvent {
    /// Drops all execution data
    Clear,

    /// Drops node states for a specific execution
    Reset(ExecId),

    /// Drop a single node from a specific execution
    Remove(ExecId, NodeId),

    /// Update the state of a node within an execution
    Update(ExecId, NodeId, ExecState),
}

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub enum ExecEvent {
    Start(ExecId, String, chrono::DateTime<chrono::Local>),
    Tick(ExecId, u64),
    Output(ExecId, String, crate::workflow::Value),
    Finish(ExecId, u64),
}

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub enum GraphEditEvent {}

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub enum WorkflowEvent {
    Exec(ExecEvent),
    // NodeState(NodeStateEvent),
    // GraphEdit(GraphEditEvent),
}
