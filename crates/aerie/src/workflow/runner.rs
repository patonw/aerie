use std::collections::{BTreeMap, BTreeSet};

use egui_snarl::{NodeId, OutPinId, Snarl};

use super::{RunContext, Value, WorkNode};

#[derive(Debug)]
pub enum ExecState {
    Waiting(BTreeSet<NodeId>),
    Ready,
    Done,
}

#[derive(Default)]
pub struct WorkflowRunner {
    pub run_ctx: RunContext,
    pub successors: BTreeMap<NodeId, BTreeSet<NodeId>>,
    pub dependencies: BTreeMap<NodeId, BTreeMap<usize, OutPinId>>,
    pub node_state: BTreeMap<NodeId, ExecState>,
    pub ready_nodes: BTreeSet<NodeId>,
}

// TODO methods to alter status when node controls or connections changed
impl WorkflowRunner {
    pub fn init(&mut self, snarl: &Snarl<WorkNode>) {
        for (src_pin, dest_pin) in snarl.wires() {
            self.successors
                .entry(src_pin.node)
                .or_default()
                .insert(dest_pin.node);

            self.dependencies
                .entry(dest_pin.node)
                .or_default()
                .insert(dest_pin.input, src_pin);
        }

        for (key, value) in &self.dependencies {
            let deps = value.values().map(|it| it.node).collect::<BTreeSet<_>>();
            self.node_state.insert(*key, ExecState::Waiting(deps));
        }

        for ready_node in snarl
            .node_ids()
            .map(|(id, _)| id)
            .filter(|id| !self.dependencies.contains_key(id))
        {
            self.node_state.insert(ready_node, ExecState::Ready);
            self.ready_nodes.insert(ready_node);
        }
    }

    pub async fn step(&mut self, snarl: &mut Snarl<WorkNode>) -> Option<NodeId> {
        let node_id = self.ready_nodes.pop_first()?;

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

        snarl[node_id].forward(&self.run_ctx, inputs).await.unwrap();

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
                                self.ready_nodes.insert(*successor);
                                *node_state = ExecState::Ready;
                                println!("{successor:?} is now ready");
                            }
                        }
                        _ => todo!(),
                    }
                }
            }
        }
        Some(node_id)
    }

    pub async fn exec(&mut self, snarl: &mut Snarl<WorkNode>) {
        while self.step(snarl).await.is_some() {}
    }
}
