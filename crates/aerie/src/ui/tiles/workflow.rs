use std::sync::Arc;

use arc_swap::ArcSwap;
use egui::{Color32, RichText, Ui};
use egui_phosphor::regular::{CHECK_CIRCLE, HAND_PALM, HOURGLASS_MEDIUM, PLAY_CIRCLE, WARNING};
use egui_snarl::{
    NodeId, Snarl,
    ui::{SnarlViewer, SnarlWidget},
};

use crate::{
    config::ConfigExt as _,
    utils::ErrorDistiller as _,
    workflow::{
        EditContext, ShadowGraph, WorkNode,
        runner::{ExecState, WorkflowRunner},
    },
};

struct WorkflowViewer {
    edit_ctx: EditContext,

    // TODO: store this in the app state so it isn't clobbered every frame
    shadow: ShadowGraph<WorkNode>,

    pub node_state: Arc<ArcSwap<im::OrdMap<NodeId, ExecState>>>,
}

// TODO maintain a shadow graph that uses immutables
// TODO: button to reset input pins
impl SnarlViewer<WorkNode> for WorkflowViewer {
    fn title(&mut self, node: &WorkNode) -> String {
        node.as_ui().title().to_string()
    }

    fn show_header(
        &mut self,
        node: NodeId,
        _inputs: &[egui_snarl::InPin],
        _outputs: &[egui_snarl::OutPin],
        ui: &mut Ui,
        snarl: &mut Snarl<WorkNode>,
    ) {
        let title = self.title(&snarl[node]);
        let node_state = self.node_state.load();

        egui::Sides::new().show(
            ui,
            |ui| {
                ui.label(title);
            },
            |ui| match node_state.get(&node) {
                Some(ExecState::Waiting(_)) => {
                    ui.label(RichText::new(HOURGLASS_MEDIUM).color(Color32::ORANGE))
                        .on_hover_text("Waiting");
                }
                Some(ExecState::Ready) => {
                    ui.label(RichText::new(PLAY_CIRCLE).color(Color32::BLUE))
                        .on_hover_text("Ready");
                }
                Some(ExecState::Running) => {
                    ui.add(egui::Spinner::new().color(Color32::LIGHT_GREEN))
                        .on_hover_text("Running");
                }
                Some(ExecState::Done) => {
                    ui.label(RichText::new(CHECK_CIRCLE).color(Color32::GREEN))
                        .on_hover_text("Done");
                }
                Some(ExecState::Disabled) => {
                    ui.label(HAND_PALM).on_hover_text("Disabled");
                }
                Some(ExecState::Failed) => {
                    ui.label(RichText::new(WARNING).color(Color32::RED))
                        .on_hover_text("Failed");
                }
                None => {}
            },
        );
    }

    fn final_node_rect(
        &mut self,
        node: NodeId,
        rect: egui::Rect,
        ui: &mut Ui,
        _snarl: &mut Snarl<WorkNode>,
    ) {
        if self.shadow.is_disabled(node) {
            let painter = ui.painter();
            painter.rect_filled(
                rect,
                16.0,
                egui::Color32::from_rgb(0x42, 0, 0).gamma_multiply(0.5),
            );
        }
    }

    fn has_on_hover_popup(&mut self, node: &WorkNode) -> bool {
        !node.as_ui().tooltip().is_empty()
    }

    fn show_on_hover_popup(
        &mut self,
        node: NodeId,
        _inputs: &[egui_snarl::InPin],
        _outputs: &[egui_snarl::OutPin],
        ui: &mut Ui,
        snarl: &mut Snarl<WorkNode>,
    ) {
        if self.shadow.is_disabled(node) {
            ui.label("Node has been disabled.\n\nThis and downstream nodes will not be executed.");
        } else {
            let tooltip = snarl[node].as_ui().tooltip();
            ui.label(tooltip);
        }
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

        if ui.button("LLM").clicked() {
            snarl.insert_node(pos, WorkNode::LLM(Default::default()));
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
            self.drop_inputs(to, snarl);
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

    fn drop_inputs(&mut self, pin: &egui_snarl::InPin, snarl: &mut Snarl<WorkNode>) {
        self.shadow = self.shadow.drop_inputs(pin);
        snarl.drop_inputs(pin.id);
    }

    fn drop_outputs(&mut self, pin: &egui_snarl::OutPin, snarl: &mut Snarl<WorkNode>) {
        self.shadow = self.shadow.drop_outputs(pin);
        snarl.drop_outputs(pin.id);
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
        if self.shadow.is_disabled(node) {
            if ui.button("Enable").clicked() {
                self.shadow = self.shadow.enable_node(node);
                ui.close();
            }
        } else if ui.button("Disable").clicked() {
            self.shadow = self.shadow.disable_node(node);
            ui.close();
        }

        if ui.button("Remove").clicked() {
            snarl.remove_node(node);
            self.shadow = self.shadow.enable_node(node).without_node(&node);

            ui.close();
        }
    }
}

impl super::AppState {
    pub fn workflow_ui(&mut self, ui: &mut egui::Ui) {
        egui::TopBottomPanel::top("Controls").show_inside(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                if ui.button("Run").clicked() {
                    let snarl_ = self.workflows.snarl.clone();
                    let mut target = { self.workflows.snarl.blocking_read().clone() };

                    let mut exec = {
                        let mut exec = WorkflowRunner::default();
                        self.workflows.node_state = exec.init(&self.workflows.shadow);

                        // TODO: clean this up
                        exec.run_ctx.history = self.session.history.clone();
                        exec.run_ctx.user_prompt = self.prompt.read().unwrap().clone();
                        exec.run_ctx.model = self.settings.view(|s| s.llm_model.clone());
                        exec.run_ctx.temperature = self.settings.view(|s| s.temperature);
                        exec.run_ctx.errors = self.errors.clone();
                        exec.run_ctx.toolbox = self.agent_factory.toolbox.clone();

                        exec
                    };

                    let running = self.workflows.running.clone();
                    let errors = self.errors.clone();
                    self.rt.spawn(async move {
                        running.store(true, std::sync::atomic::Ordering::Relaxed);

                        loop {
                            match exec.step(&mut target).await {
                                Ok(false) => break,
                                Ok(true) => {
                                    let mut snarl = snarl_.write().await;
                                    *snarl = target.clone();
                                }
                                Err(err) => errors.push(err.into()),
                            }
                        }

                        running.store(false, std::sync::atomic::Ordering::Relaxed);
                    });
                }

                // TODO: autosave based on change detection
                if ui.button("Save").clicked()
                    && let Some(name) = self.workflows.editing.as_deref()
                {
                    tracing::info!(
                        "Saving {name} to workflows...changed? {}",
                        !self.workflows.shadow.fast_eq(&self.workflows.baseline)
                    );

                    self.workflows
                        .store
                        .put(name, self.workflows.shadow.clone());
                    self.workflows.store.save().unwrap();

                    let mut snarl = self.workflows.snarl.blocking_write();
                    *snarl = Snarl::try_from(self.workflows.shadow.clone()).unwrap();
                    self.workflows.baseline = self.workflows.shadow.clone();
                }
            });
        });

        egui::CentralPanel::default().show_inside(ui, |ui| {
            let edit_ctx = EditContext {
                toolbox: self.agent_factory.toolbox.clone(),
            };

            let shadow = self.workflows.shadow.clone();
            let mut viewer = WorkflowViewer {
                edit_ctx,
                shadow,
                node_state: self.workflows.node_state.clone(),
            };
            let mut snarl = self.workflows.snarl.blocking_write();

            // TODO: Allow pan/zoom while running
            ui.add_enabled_ui(
                !self
                    .workflows
                    .running
                    .load(std::sync::atomic::Ordering::Relaxed),
                |ui| {
                    SnarlWidget::new().style(self.workflows.style).show(
                        &mut snarl,
                        &mut viewer,
                        ui,
                    );
                },
            );

            self.workflows.shadow = viewer.shadow;
        });
    }
}
