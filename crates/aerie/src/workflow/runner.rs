use std::collections::{BTreeMap, BTreeSet};

use egui_snarl::{NodeId, OutPinId, Snarl};

use crate::workflow::{ShadowGraph, Wire, WorkflowError};

use super::{RunContext, Value, WorkNode};

#[derive(Debug)]
pub enum ExecState {
    Waiting(BTreeSet<NodeId>),
    Ready,
    Done,
    Disabled,
}

#[derive(Default)]
pub struct WorkflowRunner {
    pub graph: ShadowGraph<WorkNode>,
    pub run_ctx: RunContext,
    pub successors: BTreeMap<NodeId, BTreeSet<NodeId>>,
    pub dependencies: BTreeMap<NodeId, BTreeMap<usize, OutPinId>>,
    pub node_state: BTreeMap<NodeId, ExecState>,
    pub ready_nodes: BTreeSet<NodeId>,
}

// TODO methods to alter status when node controls or connections changed
impl WorkflowRunner {
    pub fn init(&mut self, graph: &ShadowGraph<WorkNode>) {
        self.graph = graph.clone();

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
            let deps = value.values().map(|it| it.node).collect::<BTreeSet<_>>();
            self.node_state.insert(*key, ExecState::Waiting(deps));
        }

        for ready_node in graph
            .nodes
            .keys()
            .cloned()
            .filter(|id| !self.dependencies.contains_key(id))
            .filter(|id| !graph.is_disabled(*id))
        {
            self.node_state.insert(ready_node, ExecState::Ready);
            self.ready_nodes.insert(ready_node);
        }
    }

    pub async fn step(&mut self, snarl: &mut Snarl<WorkNode>) -> Result<bool, WorkflowError> {
        let Some(node_id) = self.ready_nodes.pop_first() else {
            // Nothing ready to run, halt
            tracing::info!(
                "No more nodes ready. Final execution state: {:?}",
                self.node_state
            );
            return Ok(false);
        };

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

        snarl[node_id].forward(&self.run_ctx, inputs).await?;

        println!("** Executed {node_id:?}");
        self.node_state.insert(node_id, ExecState::Done);

        if let Some(successors) = self.successors.get(&node_id) {
            for successor in successors {
                println!("Updating successor {successor:?}");
                if let Some(node_state) = self.node_state.get_mut(successor) {
                    match node_state {
                        ExecState::Waiting(deps) => {
                            deps.retain(|v| *v != node_id);
                            if deps.is_empty() {
                                if self.graph.is_disabled(*successor) {
                                    // TODO: mark downstream disabled too
                                    *node_state = ExecState::Disabled;
                                    tracing::info!("Node {successor:?} is disabled. Skipping.");
                                } else {
                                    self.ready_nodes.insert(*successor);
                                    *node_state = ExecState::Ready;
                                    tracing::info!("Node {successor:?} is now ready");
                                }
                            }
                        }
                        ExecState::Disabled => {}
                        _ => todo!(),
                    }
                }
            }
        }
        Ok(true)
    }
}
