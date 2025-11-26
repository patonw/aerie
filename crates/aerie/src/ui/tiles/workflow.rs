use std::{
    collections::BTreeSet,
    hash::{DefaultHasher, Hasher as _},
};

use egui::Ui;
use egui_snarl::{
    NodeId, Snarl,
    ui::{SnarlViewer, SnarlWidget},
};

use crate::{
    config::ConfigExt as _,
    workflow::{EditContext, ShadowGraph, WorkNode, runner::WorkflowRunner},
};

struct WorkflowViewer {
    edit_ctx: EditContext,

    // TODO: store this in the app state so it isn't clobbered every frame
    shadow: ShadowGraph<WorkNode>,
}

// TODO maintain a shadow graph that uses immutables
// TODO: button to reset input pins
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
            [remote, ..] => {
                let other = snarl[remote.node].as_ui();
                Some(other.preview(remote.output))
            }
        };

        let node_id = pin.id.node;
        let node = &mut snarl[node_id];
        let pin = node
            .as_ui_mut()
            .show_input(ui, &self.edit_ctx, pin.id.input, value);

        self.shadow = self
            .shadow
            .with_node(&node_id, snarl.get_node_info(node_id));

        pin
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
        let node_id = pin.id.node;
        let node = &mut snarl[node_id];
        let pin = node
            .as_ui_mut()
            .show_output(ui, &self.edit_ctx, pin.id.output);

        self.shadow = self
            .shadow
            .with_node(&node_id, snarl.get_node_info(node_id));
        pin
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

        // TODO: add start to empty graph by default. Don't allow insert or deletion
        if ui.button("Start").clicked() {
            snarl.insert_node(pos, WorkNode::Start(Default::default()));
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
        self.shadow = self.shadow.with_node(&node, snarl.get_node_info(node));
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
            snarl.drop_inputs(to.id);
            snarl.connect(from.id, to.id);
            self.shadow = self.shadow.with_wire(from.id, to.id);
        }
    }

    fn disconnect(
        &mut self,
        from: &egui_snarl::OutPin,
        to: &egui_snarl::InPin,
        snarl: &mut Snarl<WorkNode>,
    ) {
        snarl.disconnect(from.id, to.id);
        self.shadow = self.shadow.without_wire(from.id, to.id);
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
            self.shadow = self.shadow.without_node(&node);

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
                    let snarl_ = self.workflows.snarl.clone();
                    let mut exec = {
                        let snarl = snarl_.blocking_read();
                        let mut exec = WorkflowRunner::default();
                        exec.init(&snarl);

                        // TODO: clean this up
                        exec.run_ctx.history = self.session.history.load().clone();
                        exec.run_ctx.user_prompt = self.prompt.read().unwrap().clone();
                        exec.run_ctx.model = self.settings.view(|s| s.llm_model.clone());
                        exec.run_ctx.temperature = self.settings.view(|s| s.temperature);
                        exec.run_ctx.errors = self.errors.clone();

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
                    && let Some(name) = self.workflows.editing.as_deref()
                {
                    tracing::info!(
                        "Saving {name} to workflows...changed? {}",
                        !self.workflows.shadow.fast_eq(&self.workflows.baseline)
                    );

                    // TODO: swap object?
                    let snarl = self.workflows.snarl.blocking_read().to_owned();
                    self.workflows.baseline = ShadowGraph::from_snarl(&snarl);
                    self.workflows.shadow = self.workflows.baseline.clone();
                    self.workflows.store.put(name, snarl);
                    self.workflows.store.save().unwrap();
                }
            });
        });

        egui::CentralPanel::default().show_inside(ui, |ui| {
            let edit_ctx = EditContext {
                toolbox: self.agent_factory.toolbox.clone(),
            };

            let shadow = self.workflows.shadow.clone();
            let mut viewer = WorkflowViewer { edit_ctx, shadow };
            let mut snarl = self.workflows.snarl.blocking_write();

            SnarlWidget::new()
                .style(self.workflows.style)
                .show(&mut snarl, &mut viewer, ui);

            self.workflows.shadow = viewer.shadow;
        });
    }
}
