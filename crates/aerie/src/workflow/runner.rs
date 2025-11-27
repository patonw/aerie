use arc_swap::ArcSwap;
use egui_snarl::{NodeId, OutPinId, Snarl};
use im::OrdSet;
use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Deref,
    sync::Arc,
};

use crate::workflow::{ShadowGraph, Wire, WorkflowError};

use super::{RunContext, Value, WorkNode};

#[derive(Debug, Clone, PartialEq)]
pub enum ExecState {
    Waiting(im::OrdSet<NodeId>),
    Ready,
    Running,
    Done,
    Disabled,
    Failed,
}

#[derive(Default)]
pub struct WorkflowRunner {
    pub graph: ShadowGraph<WorkNode>,
    pub run_ctx: RunContext,
    pub successors: BTreeMap<NodeId, BTreeSet<NodeId>>,
    pub dependencies: BTreeMap<NodeId, BTreeMap<usize, OutPinId>>,
    pub node_state: Arc<ArcSwap<im::OrdMap<NodeId, ExecState>>>,
    pub ready_nodes: BTreeSet<NodeId>,
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
            node_state.insert(ready_node, ExecState::Ready);
            self.ready_nodes.insert(ready_node);
        }

        self.node_state.store(Arc::new(node_state));
        self.node_state.clone()
    }

    pub async fn step(&mut self, snarl: &mut Snarl<WorkNode>) -> Result<bool, WorkflowError> {
        let node_state = self.node_state.load_full();
        let Some(node_id) = self.ready_nodes.pop_first() else {
            // Nothing ready to run, halt
            tracing::info!(
                "No more nodes ready. Final execution state: {:?}",
                self.node_state
            );
            return Ok(false);
        };

        let mut node_state = node_state.deref().clone();
        node_state.insert(node_id, ExecState::Running);
        self.node_state.store(Arc::new(node_state.clone()));

        println!("Preparing to execute node {node_id:?}");
        let mut inputs = (0..(snarl[node_id]).as_dyn().inputs())
            .map(|_| None::<Value>)
            .collect::<Vec<_>>();

        for (in_pin, remote) in self
            .dependencies
            .get(&node_id)
            .unwrap_or(&Default::default())
        {
            let other = snarl[remote.node].as_dyn();
            let value = other.value(remote.output);

            inputs[*in_pin] = Some(value);
        }

        tracing::debug!("Collected inputs {inputs:?}");

        if let Err(err) = snarl[node_id].forward(&self.run_ctx, inputs).await {
            node_state.insert(node_id, ExecState::Failed);
            self.node_state.store(Arc::new(node_state.clone()));
            return Err(err);
        };

        println!("** Executed {node_id:?}");
        node_state.insert(node_id, ExecState::Done);
        self.node_state.store(Arc::new(node_state.clone()));

        if let Some(successors) = self.successors.get(&node_id) {
            for successor in successors {
                println!("Updating successor {successor:?}");
                if let Some(state) = node_state.get(successor).cloned() {
                    match state {
                        ExecState::Waiting(deps) => {
                            let deps = deps
                                .into_iter()
                                .filter(|v| *v != node_id)
                                .collect::<OrdSet<NodeId>>();

                            let next_state = if deps.is_empty() {
                                if self.graph.is_disabled(*successor) {
                                    // TODO: mark downstream disabled too
                                    tracing::info!("Node {successor:?} is disabled. Skipping.");
                                    ExecState::Disabled
                                } else {
                                    self.ready_nodes.insert(*successor);
                                    tracing::info!("Node {successor:?} is now ready");
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
        }

        self.node_state.store(Arc::new(node_state));

        Ok(true)
    }
}
