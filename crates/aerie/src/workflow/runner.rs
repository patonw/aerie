use arc_swap::ArcSwap;
use chrono::{DateTime, Local};
use egui_snarl::{InPinId, NodeId, OutPinId, Snarl};
use im::OrdSet;
use itertools::{EitherOrBoth, Itertools};
use std::{
    collections::{BTreeMap, BTreeSet, BinaryHeap},
    ops::Deref,
    sync::Arc,
    time::Duration,
};
use typed_builder::TypedBuilder;

use crate::workflow::{
    ShadowGraph, ValueKind, Wire, WorkflowError,
    nodes::{Fallback, Select},
};

use super::{GraphId, RunContext, Value, WorkNode};

pub type RunOutput = Arc<ArcSwap<im::OrdMap<String, crate::workflow::Value>>>;

#[derive(Debug, Clone, Default, TypedBuilder)]
pub struct WorkflowRun {
    pub workflow: String,
    pub started: DateTime<Local>,
    pub duration: Arc<ArcSwap<Duration>>,
    pub outputs: RunOutput,
}

#[derive(Clone)]
pub enum ExecState {
    Waiting(im::OrdSet<NodeId>),
    Ready,
    Running,
    Done(Vec<Value>),
    Disabled,
    Failed(Arc<WorkflowError>),
}

impl std::fmt::Debug for ExecState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Waiting(arg0) => f.debug_tuple("Waiting").field(arg0).finish(),
            Self::Ready => write!(f, "Ready"),
            Self::Running => write!(f, "Running"),
            Self::Done(arg0) => {
                let args = arg0
                    .iter()
                    .map(|it| match it {
                        // Mask the session to avoid spamming the logs/console
                        Value::Chat(_) => &Value::Placeholder(ValueKind::Chat),
                        _ => it,
                    })
                    .collect_vec();
                f.debug_tuple("Done").field(&args).finish()
            }
            Self::Disabled => write!(f, "Disabled"),
            Self::Failed(arg0) => f.debug_tuple("Failed").field(arg0).finish(),
        }
    }
}

/// A global cache of node execution states across all graphs and subgraphs
#[derive(Default, Clone, Debug)]
pub struct NodeStateMap(pub Arc<ArcSwap<im::OrdMap<(GraphId, NodeId), ExecState>>>);

impl NodeStateMap {
    pub fn clear(&self) {
        self.0.store(Default::default());
    }

    pub fn view(&self, graph_id: &GraphId) -> NodeStateView {
        let data = self.clone();
        NodeStateView {
            data,
            graph_id: *graph_id,
        }
    }
}

/// A slice of the node state for a single graph
#[derive(Default, Clone, Debug)]
pub struct NodeStateView {
    pub data: NodeStateMap,
    pub graph_id: GraphId,
}

impl NodeStateView {
    pub fn clear(&self) {
        self.data.0.rcu(|states| {
            // TODO: a more effecient way to do this
            im::OrdMap::from_iter(
                states
                    .as_ref()
                    .clone()
                    .into_iter()
                    .filter(|it| it.0.0 != self.graph_id),
            )
        });
    }

    pub fn insert(&self, node: NodeId, value: ExecState) {
        self.data
            .0
            .rcu(|states| states.update((self.graph_id, node), value.clone()));
    }

    pub fn get(&self, node: &NodeId) -> Option<ExecState> {
        self.data.0.load().get(&(self.graph_id, *node)).cloned()
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Prioritized<T> {
    pub priority: usize,
    pub payload: T,
}

impl<T> Prioritized<T> {
    pub fn new(payload: T) -> Self {
        Self {
            priority: 0,
            payload,
        }
    }

    pub fn priority(mut self, priority: usize) -> Self {
        self.priority = priority;
        self
    }
}

impl<T> Deref for Prioritized<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.payload
    }
}

impl<T: Ord> PartialOrd for Prioritized<T> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl<T: Ord> Ord for Prioritized<T> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match self.priority.cmp(&other.priority) {
            std::cmp::Ordering::Equal => self.payload.cmp(&other.payload),
            cmp => cmp,
        }
    }
}

#[derive(TypedBuilder)]
pub struct WorkflowRunner {
    #[builder(default)]
    pub graph: ShadowGraph<WorkNode>,

    pub run_ctx: RunContext,

    #[builder(default)]
    pub successors: BTreeMap<NodeId, BTreeSet<NodeId>>,

    #[builder(default)]
    pub dependencies: BTreeMap<NodeId, BTreeMap<usize, OutPinId>>,

    #[builder(default)]
    pub state_view: NodeStateView,

    #[builder(default)]
    pub ready_nodes: BinaryHeap<Prioritized<NodeId>>,

    #[builder(default)]
    pub inputs: Vec<Option<Value>>,

    #[builder(default)]
    pub outputs: Vec<Option<Value>>,
}

// TODO methods to alter status when node controls or connections changed
impl WorkflowRunner {
    pub fn init(&mut self, graph: &ShadowGraph<WorkNode>) {
        self.graph = graph.repair();
        self.state_view.clear();

        for Wire { out_pin, in_pin } in &graph.wires {
            // ignore orphaned inputs/outputs... ideally remove them before saving
            let Some(out_node) = graph.nodes.get(&out_pin.node) else {
                continue;
            };

            if out_pin.output >= out_node.value.as_dyn().outputs() {
                continue;
            }

            let Some(in_node) = graph.nodes.get(&in_pin.node) else {
                continue;
            };

            if in_pin.input >= in_node.value.as_dyn().inputs() {
                continue;
            }

            self.successors
                .entry(out_pin.node)
                .or_default()
                .insert(in_pin.node);

            self.dependencies
                .entry(in_pin.node)
                .or_default()
                .insert(in_pin.input, *out_pin);
        }

        for (key, value) in &self.dependencies {
            let deps = value.values().map(|it| it.node).collect::<im::OrdSet<_>>();
            self.state_view.insert(*key, ExecState::Waiting(deps));
        }

        for ready_node in graph
            .nodes
            .keys()
            .cloned()
            .filter(|id| !self.dependencies.contains_key(id))
            .filter(|id| !graph.is_disabled(*id))
        {
            let priority = graph
                .nodes
                .get(&ready_node)
                .map(|n| n.value.as_dyn().priority())
                .unwrap_or_default();
            self.state_view.insert(ready_node, ExecState::Ready);
            self.ready_nodes
                .push(Prioritized::new(ready_node).priority(priority));
        }
    }

    // TODO: fully convert this to shadow graph
    pub fn step(&mut self, snarl: &mut Snarl<WorkNode>) -> Result<bool, Arc<WorkflowError>> {
        tracing::trace!("Priority queue: {:?}", &self.ready_nodes);

        let Some(ready_node) = self.ready_nodes.pop() else {
            // Nothing ready to run, halt
            tracing::info!("No more nodes ready.");

            if self.outputs.is_empty() {
                let finish_state = self
                    .graph
                    .finish
                    .as_ref()
                    .and_then(|f| self.state_view.get(f))
                    .unwrap_or(ExecState::Waiting(Default::default()));

                Err(WorkflowError::Unfinished(finish_state))?;
            }
            return Ok(false);
        };

        let node_id = ready_node.payload;

        self.state_view.insert(node_id, ExecState::Running);

        tracing::debug!(
            "Preparing to execute node {node_id:?}: {}",
            snarl[node_id].kind(),
        );

        let single_out = snarl[node_id].as_dyn().outputs() == 1;
        let num_outs = snarl[node_id].as_dyn().outputs();

        let inputs = self.gather_inputs(snarl, node_id)?;
        let inputs = self.inject_failure(snarl, node_id, inputs);

        // Find this node's connected failure output pin
        let out_fail = (0..num_outs).find_map(|pin| {
            if !matches!(snarl[node_id].as_dyn().out_kind(pin), ValueKind::Failure) {
                None
            } else {
                let out_pin = snarl.out_pin(OutPinId {
                    node: node_id,
                    output: pin,
                });

                let remotes = &out_pin.remotes;

                if remotes.is_empty() || remotes.iter().all(|r| self.graph.is_disabled(r.node)) {
                    None
                } else {
                    Some(out_pin)
                }
            }
        });

        // Collate into set for faster lookups
        let fail_handlers = out_fail
            .iter()
            .flat_map(|op| op.remotes.iter())
            .map(|remote| remote.node)
            .collect::<BTreeSet<_>>();

        // Create a lookup of output pin to remotes
        let out_remotes = (0..num_outs)
            .map(|pin_num| {
                let out_pin = snarl.out_pin(OutPinId {
                    node: node_id,
                    output: pin_num,
                });

                let remotes = &out_pin.remotes;
                remotes.iter().map(|p| p.node).collect::<BTreeSet<_>>()
            })
            .collect_vec();

        // When a pin outputs a placeholder, don't allow its remotes to become ready
        let mut blacklist: BTreeSet<NodeId> = Default::default();

        if Some(node_id) == self.graph.finish {
            tracing::info!("Setting graph {:?} outputs to {inputs:?}", self.graph.uuid);
            self.outputs = inputs.clone();
        }

        // Update run state of current node
        let succeeded = match snarl[node_id].execute(&self.run_ctx, node_id, inputs) {
            Ok(values) => {
                for tooth in (0..num_outs).zip_longest(values.iter()) {
                    match tooth {
                        EitherOrBoth::Both(i, value) => {
                            if matches!(value, Value::Placeholder(_)) {
                                blacklist.extend(&out_remotes[i]);
                            }
                        }
                        EitherOrBoth::Left(i) => {
                            blacklist.extend(&out_remotes[i]);
                        }
                        EitherOrBoth::Right(_) => unreachable!(),
                    }
                }

                tracing::trace!("Values: {values:?}");
                self.state_view.insert(node_id, ExecState::Done(values));
                true
            }
            Err(err) => {
                tracing::debug!("Got a failure {err:?} handlers: {fail_handlers:?}");
                let err = Arc::new(err);
                self.state_view
                    .insert(node_id, ExecState::Failed(err.clone()));

                // Input errors from mis-wired graphs -- non-recoverable
                // Nothing to catch errors, abort run
                if matches!(err.as_ref(), WorkflowError::Required(_)) || fail_handlers.is_empty() {
                    return Err(err);
                }

                tracing::debug!("Falliable node {node_id:?} failed with {err:?}");
                false
            }
        };

        tracing::info!("** Executed {node_id:?}");

        let successors = if let Some(successors) = self.successors.get(&node_id) {
            tracing::debug!(
                "Filtering successors {:?} -- {succeeded} -- {fail_handlers:?}",
                successors
            );

            // Filter down successors to just failure wires when failed,
            // otherwise only non-failure (normal data) wires.
            successors
                .iter()
                // .filter(|n| single_out || succeeded ^ fail_handlers.contains(n))
                .filter(|n| {
                    if succeeded {
                        // Disallow anything on a pin with Placehlder output to run
                        // This should include failure pins, but double check anyhow
                        // matches!(snarl[**n], WorkNode::Select(_)) ||
                        !blacklist.contains(n) && !fail_handlers.contains(n)
                    } else if single_out {
                        // Router nodes that mirror their input since it doesn't make sense for a
                        // node to have only a failure output otherwise
                        true
                    } else {
                        fail_handlers.contains(n)
                    }
                })
                .collect_vec()
        } else {
            // Leaf node
            Vec::new()
        };

        tracing::debug!("Filtered successors: {:?}", successors);

        // Update state of successors
        for successor in successors {
            tracing::debug!("Updating successor {successor:?}");
            if let Some(state) = self.state_view.get(successor) {
                match state {
                    ExecState::Waiting(deps) => {
                        let deps = deps
                            .into_iter()
                            .filter(|v| *v != node_id)
                            .collect::<OrdSet<NodeId>>();

                        let is_eager = snarl[*successor].is_eager();
                        let next_state = if deps.is_empty() || is_eager {
                            if self.graph.is_disabled(*successor) {
                                tracing::info!("Node {successor:?} is disabled. Skipping.");
                                ExecState::Disabled
                            } else {
                                let priority = self
                                    .graph
                                    .nodes
                                    .get(successor)
                                    .map(|n| n.value.as_dyn().priority())
                                    .unwrap_or_default();
                                self.ready_nodes
                                    .push(Prioritized::new(*successor).priority(priority));

                                tracing::info!(
                                    "Node {successor:?} ({}) is now ready with priority {priority}",
                                    snarl[*successor].kind()
                                );

                                ExecState::Ready
                            }
                        } else {
                            ExecState::Waiting(deps)
                        };

                        self.state_view.insert(*successor, next_state);
                    }
                    ExecState::Disabled => {}
                    ExecState::Done(_) => {}
                    _ => {
                        tracing::warn!("Not implemented for {state:?}");
                        todo!()
                    }
                }
            }
        }

        Ok(true)
    }

    fn gather_inputs(
        &self,
        snarl: &Snarl<WorkNode>,
        node_id: NodeId,
    ) -> Result<Vec<Option<Value>>, WorkflowError> {
        if Some(node_id) == self.graph.start {
            return Ok(self.inputs.clone());
        }

        let dyn_node = (snarl[node_id]).as_dyn();
        // Gather inputs
        let mut inputs = (0..dyn_node.inputs())
            .map(|_| None::<Value>)
            .collect::<Vec<_>>();

        for (in_pin, remote) in self
            .dependencies
            .get(&node_id)
            .unwrap_or(&Default::default())
        {
            if let Some(ExecState::Done(outputs)) = self.state_view.get(&remote.node)
                && remote.output < outputs.len()
            {
                let value = &outputs[remote.output];
                inputs[*in_pin] = if matches!(value, Value::Placeholder(_)) {
                    None
                } else {
                    Some(value.clone())
                };
            } else {
                // Unnecessary sanity checking
                let snode = &snarl[node_id];
                if snode.as_node::<Fallback>().is_some() {
                    tracing::debug!("Fallback node fail pin {in_pin:?}");
                } else if snode.as_node::<Select>().is_some() {
                    tracing::debug!("Select node empty pin {in_pin:?}");
                } else {
                    tracing::warn!(
                        "Falling back on legacy input value for {:?} pin #{:?}",
                        snarl[node_id].kind(),
                        in_pin
                    );

                    let other = snarl[remote.node].as_dyn();
                    let value = other.value(remote.output);

                    inputs[*in_pin] = Some(value);
                }
            }
        }
        Ok(inputs)
    }

    fn inject_failure(
        &self,
        snarl: &mut Snarl<WorkNode>,
        node_id: NodeId,
        mut inputs: Vec<Option<Value>>,
    ) -> Vec<Option<Value>> {
        let dyn_node = (snarl[node_id]).as_dyn();
        // The input pin on this node that takes the failure with its remote output
        let in_fail = (0..dyn_node.inputs()).find_map(|pin| {
            let in_pin = snarl.in_pin(InPinId {
                node: node_id,
                input: pin,
            });

            let out_pin = in_pin
                .remotes
                .iter()
                .find(|r| {
                    matches!(
                        snarl[r.node].as_dyn().out_kind(r.output),
                        ValueKind::Failure
                    )
                })
                .cloned();

            out_pin.map(|out| (pin, out))
        });

        if let Some((in_pin, remote)) = in_fail
            && let Some(ExecState::Failed(err)) = self.state_view.get(&remote.node)
        {
            tracing::info!(
                "Setting failure node {:?} input on {}: {:?}",
                snarl[node_id].kind(),
                in_pin,
                err
            );

            inputs[in_pin] = Some(Value::Failure(err.clone()));
        }
        inputs
    }

    // TODO: refactor. Then we can move history out of RunContext
    pub fn root_finish(&self) -> Result<(), WorkflowError> {
        let ctx = &self.run_ctx;
        let inputs = self.outputs.clone();
        match &inputs[0] {
            Some(Value::Chat(chat)) => {
                if ctx.history.load().is_subset(chat) {
                    ctx.history
                        .store(Arc::new(chat.with_base(None).into_owned()));
                } else {
                    Err(WorkflowError::Conversion(
                        "Final chat history is not related to the session. Refusing to overwrite."
                            .into(),
                    ))?;
                }
            }
            None => {}
            _ => unreachable!(),
        }

        Ok(())
    }
}
