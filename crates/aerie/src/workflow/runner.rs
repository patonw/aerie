use arc_swap::ArcSwap;
use egui_snarl::{InPinId, NodeId, OutPinId, Snarl};
use im::OrdSet;
use itertools::Itertools;
use std::{
    collections::{BTreeMap, BTreeSet, BinaryHeap},
    ops::Deref,
    sync::Arc,
};
use typed_builder::TypedBuilder;

use crate::workflow::{ShadowGraph, ValueKind, Wire, WorkflowError, nodes::WorkNodeKind};

use super::{RunContext, Value, WorkNode};

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
    pub node_state: Arc<ArcSwap<im::OrdMap<NodeId, ExecState>>>,

    #[builder(default)]
    pub ready_nodes: BinaryHeap<Prioritized<NodeId>>,
}

// TODO methods to alter status when node controls or connections changed
impl WorkflowRunner {
    pub fn init(
        &mut self,
        graph: &ShadowGraph<WorkNode>,
    ) -> Arc<ArcSwap<im::OrdMap<NodeId, ExecState>>> {
        self.graph = graph.clone();
        let mut node_state: im::OrdMap<NodeId, ExecState> = Default::default();

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
            node_state.insert(*key, ExecState::Waiting(deps));
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
            node_state.insert(ready_node, ExecState::Ready);
            self.ready_nodes
                .push(Prioritized::new(ready_node).priority(priority));
        }

        self.node_state.store(Arc::new(node_state));
        self.node_state.clone()
    }

    // TODO: fully convert this to shadow graph
    pub fn step(&mut self, snarl: &mut Snarl<WorkNode>) -> Result<bool, Arc<WorkflowError>> {
        let node_state = self.node_state.load_full();
        tracing::trace!("Priority queue: {:?}", &self.ready_nodes);

        let Some(ready_node) = self.ready_nodes.pop() else {
            // Nothing ready to run, halt
            tracing::info!(
                "No more nodes ready. Final execution state: {:?}",
                self.node_state
            );
            return Ok(false);
        };

        let node_id = ready_node.payload;

        let mut node_state = node_state.deref().clone();
        node_state.insert(node_id, ExecState::Running);
        self.node_state.store(Arc::new(node_state.clone()));

        tracing::debug!(
            "Preparing to execute node {node_id:?}: {}",
            &snarl[node_id].kind()
        );

        let dyn_node = (snarl[node_id]).as_dyn();
        let single_out = dyn_node.outputs() == 1;

        // Gather inputs
        let mut inputs = (0..dyn_node.inputs())
            .map(|_| None::<Value>)
            .collect::<Vec<_>>();

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

        for (in_pin, remote) in self
            .dependencies
            .get(&node_id)
            .unwrap_or(&Default::default())
        {
            if let Some(ExecState::Done(outputs)) = node_state.get(&remote.node)
                && remote.output < outputs.len()
            {
                inputs[*in_pin] = Some(outputs[remote.output].clone())
            } else {
                // Unnecessary sanity checking
                match snarl[node_id].kind() {
                    WorkNodeKind::Fallback if *in_pin == 0 => {
                        tracing::debug!("Fallback node fail pin {in_pin:?}");
                    }
                    WorkNodeKind::Select => {
                        tracing::debug!("Select node empty pin {in_pin:?}");
                    }
                    _ => {
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
        }

        if let Some((in_pin, remote)) = in_fail
            && let Some(ExecState::Failed(err)) = node_state.get(&remote.node)
        {
            tracing::info!(
                "Setting failure node {:?} input on {}: {:?}",
                snarl[node_id].kind(),
                in_pin,
                err
            );

            inputs[in_pin] = Some(Value::Failure(err.clone()));
        }

        // Find this node's connected failure output pin
        let out_fail = (0..dyn_node.outputs()).find_map(|pin| {
            if !matches!(dyn_node.out_kind(pin), ValueKind::Failure) {
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

        // Update run state of current node
        let succeeded = match snarl[node_id].execute(&self.run_ctx, node_id, inputs) {
            Ok(values) => {
                node_state.insert(node_id, ExecState::Done(values));
                self.node_state.store(Arc::new(node_state.clone()));
                true
            }
            Err(err) => {
                tracing::debug!("Got a failure {err:?} handlers: {fail_handlers:?}");
                let err = Arc::new(err);
                node_state.insert(node_id, ExecState::Failed(err.clone()));
                self.node_state.store(Arc::new(node_state.clone()));

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
                .filter(|n| single_out || succeeded ^ fail_handlers.contains(n))
                .collect_vec()
        } else {
            // Leaf node
            Vec::new()
        };

        tracing::debug!("Filtered successors: {:?}", successors);

        // Update state of successors
        for successor in successors {
            tracing::debug!("Updating successor {successor:?}");
            if let Some(state) = node_state.get(successor).cloned() {
                match state {
                    ExecState::Waiting(deps) => {
                        let deps = deps
                            .into_iter()
                            .filter(|v| *v != node_id)
                            .collect::<OrdSet<NodeId>>();

                        let is_select = matches!(snarl[*successor], WorkNode::Select(_));
                        let next_state = if deps.is_empty() || is_select {
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

                        node_state.insert(*successor, next_state);
                    }
                    ExecState::Disabled => {}
                    _ => todo!(),
                }
            }
        }

        self.node_state.store(Arc::new(node_state));

        Ok(true)
    }
}
