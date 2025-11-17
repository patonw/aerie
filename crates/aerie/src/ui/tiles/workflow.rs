use std::collections::{BTreeSet, HashSet};

use egui::Ui;
use egui_snarl::{
    NodeId, Snarl,
    ui::{SnarlViewer, SnarlWidget},
};

use crate::workflow::{EditContext, WorkNode, runner::WorkflowRunner};

struct WorkflowViewer {
    edit_ctx: EditContext,
}

impl SnarlViewer<WorkNode> for WorkflowViewer {
    fn title(&mut self, node: &WorkNode) -> String {
        node.as_ui().title()
    }

    fn inputs(&mut self, node: &WorkNode) -> usize {
        node.as_ui().inputs()
    }

    fn show_input(
        &mut self,
        pin: &egui_snarl::InPin,
        ui: &mut egui::Ui,
        snarl: &mut egui_snarl::Snarl<WorkNode>,
    ) -> impl egui_snarl::ui::SnarlPin + 'static {
        let value = match &*pin.remotes {
            [] => None,
            [remote] => {
                let other = snarl[remote.node].as_ui();
                Some(other.preview(remote.output))
            }
            _ => unreachable!(),
        };

        snarl[pin.id.node]
            .as_ui_mut()
            .show_input(ui, &self.edit_ctx, pin.id.input, value)
    }

    fn outputs(&mut self, node: &WorkNode) -> usize {
        node.as_ui().outputs()
    }

    fn show_output(
        &mut self,
        pin: &egui_snarl::OutPin,
        ui: &mut egui::Ui,
        snarl: &mut egui_snarl::Snarl<WorkNode>,
    ) -> impl egui_snarl::ui::SnarlPin + 'static {
        snarl[pin.id.node]
            .as_ui_mut()
            .show_output(ui, &self.edit_ctx, pin.id.output)
    }

    fn has_graph_menu(&mut self, _pos: egui::Pos2, _snarl: &mut Snarl<WorkNode>) -> bool {
        true
    }

    fn show_graph_menu(&mut self, pos: egui::Pos2, ui: &mut Ui, snarl: &mut Snarl<WorkNode>) {
        if ui.button("Preview").clicked() {
            snarl.insert_node(pos, WorkNode::Preview(Default::default()));
            ui.close();
        }

        if ui.button("Text").clicked() {
            snarl.insert_node(pos, WorkNode::Text(Default::default()));
            ui.close();
        }

        if ui.button("Tools").clicked() {
            snarl.insert_node(pos, WorkNode::Tools(Default::default()));
            ui.close();
        }
    }

    fn has_body(&mut self, node: &WorkNode) -> bool {
        node.as_ui().has_body()
    }

    fn show_body(
        &mut self,
        node: egui_snarl::NodeId,
        _inputs: &[egui_snarl::InPin],
        _outputs: &[egui_snarl::OutPin],
        ui: &mut Ui,
        snarl: &mut Snarl<WorkNode>,
    ) {
        snarl[node].as_ui_mut().show_body(ui, &self.edit_ctx);
    }

    fn connect(
        &mut self,
        from: &egui_snarl::OutPin,
        to: &egui_snarl::InPin,
        snarl: &mut Snarl<WorkNode>,
    ) {
        // TODO: cycle check
        let remote = &snarl[from.id.node];
        let wire_kind = remote.as_dyn().out_kind(from.id.output);
        let recipient = &snarl[to.id.node];
        if recipient.as_dyn().connect(to.id.input, wire_kind).is_ok() {
            snarl.connect(from.id, to.id);
        }
    }

    fn has_node_menu(&mut self, _node: &WorkNode) -> bool {
        true
    }

    fn show_node_menu(
        &mut self,
        node: NodeId,
        _inputs: &[egui_snarl::InPin],
        _outputs: &[egui_snarl::OutPin],
        ui: &mut Ui,
        snarl: &mut Snarl<WorkNode>,
    ) {
        if ui.button("Remove").clicked() {
            snarl.remove_node(node);
            ui.close();
        }
    }
}

impl super::AppState {
    pub fn workflow_ui(&mut self, ui: &mut egui::Ui) {
        egui::TopBottomPanel::top("Controls").show_inside(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                // TODO: remove button. run only during chat
                if ui.button("Run").clicked() {
                    let snarl_ = self.snarl.clone();
                    let mut exec = {
                        let snarl = snarl_.blocking_read();
                        let mut exec = WorkflowRunner::default();
                        exec.init(&snarl);
                        exec
                    };

                    self.rt.spawn(async move {
                        loop {
                            // This would block workflow rendering while waiting for LLM to respond
                            // TODO: explore how to use ArcSwap to snapshot snarl graphs
                            let mut snarl = snarl_.write().await;
                            if exec.step(&mut snarl).await.is_none() {
                                break;
                            }
                        }
                    });
                }

                // Expensive, don't want to run this every frame so leave button active
                if ui.button("Save").clicked()
                    && let Some(name) = self.edit_workflow.as_deref()
                // && self.workflow_changed()
                {
                    tracing::info!("Saving {name} to workflows");
                    // TODO: swap object?
                    let snarl = self.snarl.blocking_read().to_owned();
                    self.workflows.put(name, snarl);
                    self.workflows.save().unwrap();
                }
            });
        });

        egui::CentralPanel::default().show_inside(ui, |ui| {
            let edit_ctx = EditContext {
                toolbox: self.agent_factory.toolbox.clone(),
            };
            let mut viewer = WorkflowViewer { edit_ctx };
            let mut snarl = self.snarl.blocking_write();

            SnarlWidget::new()
                .style(self.snarl_style)
                .show(&mut snarl, &mut viewer, ui);
        });
    }

    pub fn workflow_changed(&self) -> bool {
        let Some(name) = &self.edit_workflow else {
            return true;
        };

        let Some(baseline) = self.workflows.get(name) else {
            return true;
        };

        let current = self.snarl.blocking_write();

        // How the heck do we detect when the graph has changed, efficiently?
        // No comparator, dirty flag or event bus.

        let wires_b = baseline.wires().collect::<BTreeSet<_>>();
        let wires_c = current.wires().collect::<BTreeSet<_>>();
        if wires_b != wires_c {
            return true;
        }

        let nodes_b = baseline.nodes().collect::<HashSet<_>>();
        let nodes_c = baseline.nodes().collect::<HashSet<_>>();
        nodes_b != nodes_c
    }
}
