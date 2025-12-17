use std::{
    borrow::Cow,
    sync::{Arc, atomic::Ordering},
};

use arc_swap::ArcSwap;
use egui::{Align2, Color32, ComboBox, Hyperlink, RichText, Ui};
use egui_extras::{Size, StripBuilder};
use egui_phosphor::regular::{
    ARROW_CLOCKWISE, ARROW_COUNTER_CLOCKWISE, CHECK_CIRCLE, DOWNLOAD_SIMPLE, HAND_PALM,
    HOURGLASS_MEDIUM, INFO, PLAY, PLAY_CIRCLE, STOP, UPLOAD_SIMPLE, WARNING,
};
use egui_snarl::{
    NodeId, Snarl,
    ui::{SnarlViewer, SnarlWidget},
};
use itertools::Itertools;
use uuid::Uuid;

use crate::{
    config::ConfigExt as _,
    utils::ErrorDistiller as _,
    workflow::{
        EditContext, RunContext, ShadowGraph, WorkNode,
        nodes::CommentNode,
        runner::{ExecState, WorkflowRunner},
    },
};

struct WorkflowViewer {
    edit_ctx: EditContext,

    // TODO: store this in the app state so it isn't clobbered every frame
    shadow: ShadowGraph<WorkNode>,

    pub node_state: Arc<ArcSwap<im::OrdMap<NodeId, ExecState>>>,

    running: bool,

    frozen: bool,
}

impl WorkflowViewer {
    fn frozen(&self) -> bool {
        self.running || self.frozen
    }

    fn can_edit(&self) -> bool {
        !self.frozen()
    }
}

// TODO maintain a shadow graph that uses immutables
// TODO: button to reset input pins
impl SnarlViewer<WorkNode> for WorkflowViewer {
    fn title(&mut self, node: &WorkNode) -> String {
        node.as_ui().title().to_string()
    }

    fn node_frame(
        &mut self,
        default: egui::Frame,
        node: NodeId,
        _inputs: &[egui_snarl::InPin],
        _outputs: &[egui_snarl::OutPin],
        snarl: &Snarl<WorkNode>,
    ) -> egui::Frame {
        if matches!(snarl[node], WorkNode::Comment(_)) {
            default.fill(CommentNode::bg_color())
        } else {
            default
        }
    }

    fn header_frame(
        &mut self,
        default: egui::Frame,
        node: NodeId,
        _inputs: &[egui_snarl::InPin],
        _outputs: &[egui_snarl::OutPin],
        snarl: &Snarl<WorkNode>,
    ) -> egui::Frame {
        if matches!(snarl[node], WorkNode::Comment(_)) {
            let node_info = snarl.get_node_info(node).unwrap();
            if node_info.open {
                default.fill(CommentNode::bg_color())
            } else {
                default.fill(Color32::from_rgb(0x88, 0x88, 0))
            }
        } else {
            default
        }
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

        if matches!(snarl[node], WorkNode::Comment(_)) {
            return;
        }

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
                Some(ExecState::Done(_)) => {
                    ui.label(RichText::new(CHECK_CIRCLE).color(Color32::GREEN))
                        .on_hover_text("Done");
                }
                Some(ExecState::Disabled) => {
                    ui.label(HAND_PALM).on_hover_text("Disabled");
                }
                Some(ExecState::Failed(err)) => {
                    if ui
                        .label(RichText::new(WARNING).color(Color32::RED))
                        .on_hover_text(format!("{err:?}"))
                        .interact(egui::Sense::click())
                        .clicked()
                    {
                        let error = err.clone();
                        self.edit_ctx.errors.push(error.into());
                    }
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
        snarl: &mut Snarl<WorkNode>,
    ) {
        if self.shadow.is_disabled(node) {
            let painter = ui.painter();
            painter.rect_filled(
                rect,
                16.0,
                egui::Color32::from_rgb(0x42, 0, 0).gamma_multiply(0.5),
            );
        }

        // A bit hacky
        let output_drop = self
            .edit_ctx
            .output_reset
            .swap(Arc::new(Default::default()));
        for out_pin_id in output_drop.iter() {
            let out_pin = snarl.out_pin(*out_pin_id);
            self.drop_outputs(&out_pin, snarl);
        }

        let output_reset = self
            .edit_ctx
            .output_reset
            .swap(Arc::new(Default::default()));
        for out_pin_id in output_reset.iter() {
            let out_pin = snarl.out_pin(*out_pin_id);
            for in_pin_id in &out_pin.remotes {
                let in_pin = snarl.in_pin(*in_pin_id);
                self.disconnect(&out_pin, &in_pin, snarl);
                self.connect(&out_pin, &in_pin, snarl);
            }
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
        ui.add_enabled_ui(self.can_edit(), |ui| {
            let value = match &*pin.remotes {
                [] => None,
                [remote, ..] => {
                    let other = snarl[remote.node].as_ui();
                    Some(other.preview(remote.output))
                }
            };

            let node_id = pin.id.node;
            self.edit_ctx.current_node = node_id;
            let node = &mut snarl[node_id];
            let pin = node
                .as_ui_mut()
                .show_input(ui, &self.edit_ctx, pin.id.input, value);

            self.shadow = self
                .shadow
                .with_node(&node_id, snarl.get_node_info(node_id));

            pin
        })
        .inner
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
        ui.add_enabled_ui(self.can_edit(), |ui| {
            let node_id = pin.id.node;
            self.edit_ctx.current_node = node_id;
            let node = &mut snarl[node_id];
            let pin = node
                .as_ui_mut()
                .show_output(ui, &self.edit_ctx, pin.id.output);

            self.shadow = self
                .shadow
                .with_node(&node_id, snarl.get_node_info(node_id));
            pin
        })
        .inner
    }

    fn has_graph_menu(&mut self, _pos: egui::Pos2, _snarl: &mut Snarl<WorkNode>) -> bool {
        self.can_edit()
    }

    fn show_graph_menu(&mut self, pos: egui::Pos2, ui: &mut Ui, snarl: &mut Snarl<WorkNode>) {
        ui.menu_button("Control", |ui| {
            if ui.button("Fallback").clicked() {
                snarl.insert_node(pos, WorkNode::Fallback(Default::default()));
                ui.close();
            }

            if ui.button("Select").clicked() {
                snarl.insert_node(pos, WorkNode::Select(Default::default()));
                ui.close();
            }

            if ui.button("Demote").clicked() {
                snarl.insert_node(pos, WorkNode::Demote(Default::default()));
                ui.close();
            }

            if ui.button("Panic").clicked() {
                snarl.insert_node(pos, WorkNode::Panic(Default::default()));
                ui.close();
            }
        });

        ui.menu_button("Text", |ui| {
            if ui.button("Plain Text").clicked() {
                snarl.insert_node(pos, WorkNode::Text(Default::default()));
                ui.close();
            }

            if ui.button("Template").clicked() {
                snarl.insert_node(pos, WorkNode::TemplateNode(Default::default()));
                ui.close();
            }
        });

        ui.menu_button("LLM", |ui| {
            if ui.button("Agent").clicked() {
                snarl.insert_node(pos, WorkNode::Agent(Default::default()));
                ui.close();
            }

            if ui.button("Context").clicked() {
                snarl.insert_node(pos, WorkNode::Context(Default::default()));
                ui.close();
            }

            if ui.button("Chat").clicked() {
                snarl.insert_node(pos, WorkNode::Chat(Default::default()));
                ui.close();
            }

            if ui.button("Structured").clicked() {
                snarl.insert_node(pos, WorkNode::Structured(Default::default()));
                ui.close();
            }
        });

        ui.menu_button("Tools", |ui| {
            if ui.button("Select Tools").clicked() {
                snarl.insert_node(pos, WorkNode::Tools(Default::default()));
                ui.close();
            }
            if ui.button("Invoke Tools").clicked() {
                snarl.insert_node(pos, WorkNode::InvokeTool(Default::default()));
                ui.close();
            }
        });

        ui.menu_button("Conversation", |ui| {
            if ui.button("Create Message").clicked() {
                snarl.insert_node(pos, WorkNode::CreateMessage(Default::default()));
                ui.close();
            }

            if ui.button("Mask History").clicked() {
                snarl.insert_node(pos, WorkNode::MaskChat(Default::default()));
                ui.close();
            }

            if ui.button("Extend History").clicked() {
                snarl.insert_node(pos, WorkNode::ExtendHistory(Default::default()));
                ui.close();
            }

            if ui.button("Side Chat").clicked() {
                snarl.insert_node(pos, WorkNode::GraftChat(Default::default()));
                ui.close();
            }
        });

        ui.menu_button("JSON", |ui| {
            if ui.button("Parse JSON").clicked() {
                snarl.insert_node(pos, WorkNode::ParseJson(Default::default()));
                ui.close();
            }

            if ui.button("Gather JSON").clicked() {
                snarl.insert_node(pos, WorkNode::GatherJson(Default::default()));
                ui.close();
            }

            if ui.button("Validate JSON").clicked() {
                snarl.insert_node(pos, WorkNode::ValidateJson(Default::default()));
                ui.close();
            }

            if ui.button("Transform JSON").clicked() {
                snarl.insert_node(pos, WorkNode::TransformJson(Default::default()));
                ui.close();
            }
        });

        if ui.button("Preview").clicked() {
            snarl.insert_node(pos, WorkNode::Preview(Default::default()));
            ui.close();
        }

        if ui.button("Output").clicked() {
            snarl.insert_node(pos, WorkNode::Output(Default::default()));
            ui.close();
        }

        if ui.button("Comment").clicked() {
            snarl.insert_node(pos, WorkNode::Comment(Default::default()));
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
        self.edit_ctx.current_node = node;
        ui.add_enabled_ui(self.can_edit(), |ui| {
            snarl[node].as_ui_mut().show_body(ui, &self.edit_ctx);
            self.shadow = self.shadow.with_node(&node, snarl.get_node_info(node));
        })
        .inner
    }

    fn connect(
        &mut self,
        from: &egui_snarl::OutPin,
        to: &egui_snarl::InPin,
        snarl: &mut Snarl<WorkNode>,
    ) {
        // TODO: cycle check
        if self.can_edit() {
            let remote = &snarl[from.id.node];
            let wire_kind = remote.as_dyn().out_kind(from.id.output);
            let recipient = &snarl[to.id.node];
            if recipient.as_dyn().connect(to.id.input, wire_kind).is_ok() {
                self.drop_inputs(to, snarl);
                snarl.connect(from.id, to.id);
                self.shadow = self.shadow.with_wire(from.id, to.id);
            }
        }
    }

    fn disconnect(
        &mut self,
        from: &egui_snarl::OutPin,
        to: &egui_snarl::InPin,
        snarl: &mut Snarl<WorkNode>,
    ) {
        if self.can_edit() {
            snarl.disconnect(from.id, to.id);
            self.shadow = self.shadow.without_wire(from.id, to.id);
        }
    }

    fn drop_inputs(&mut self, pin: &egui_snarl::InPin, snarl: &mut Snarl<WorkNode>) {
        if self.can_edit() {
            self.shadow = self.shadow.drop_inputs(pin);
            snarl.drop_inputs(pin.id);
        }
    }

    fn drop_outputs(&mut self, pin: &egui_snarl::OutPin, snarl: &mut Snarl<WorkNode>) {
        if self.can_edit() {
            self.shadow = self.shadow.drop_outputs(pin);
            snarl.drop_outputs(pin.id);
        }
    }

    fn has_node_menu(&mut self, node: &WorkNode) -> bool {
        self.can_edit() && !matches!(node, WorkNode::Start(_) | WorkNode::Finish(_))
    }

    fn show_node_menu(
        &mut self,
        node: NodeId,
        _inputs: &[egui_snarl::InPin],
        _outputs: &[egui_snarl::OutPin],
        ui: &mut Ui,
        snarl: &mut Snarl<WorkNode>,
    ) {
        let help_link = snarl[node].as_ui().help_link();
        if !help_link.is_empty() {
            ui.add(Hyperlink::from_label_and_url("Help", help_link).open_in_new_tab(true));
        }

        if !matches!(snarl[node], WorkNode::Comment(_)) {
            if self.shadow.is_disabled(node) {
                if ui.button("Enable").clicked() {
                    self.shadow = self.shadow.enable_node(node);
                    ui.close();
                }
            } else if ui.button("Disable").clicked() {
                self.shadow = self.shadow.disable_node(node);
                ui.close();
            }
        }

        // Alleviate accidental clicks
        ui.menu_button("Remove", |ui| {
            if ui.button("OK").clicked() {
                snarl.remove_node(node);
                self.shadow = self.shadow.enable_node(node).without_node(&node);

                ui.close();
            }
        });
    }
}

impl super::AppState {
    pub fn workflow_ui(&mut self, ui: &mut egui::Ui) {
        egui::CentralPanel::default().show_inside(ui, |ui| {
            let edit_ctx = EditContext::builder()
                .toolbox(self.agent_factory.toolbox.clone())
                .errors(self.errors.clone())
                .build();

            let running = self
                .workflows
                .running
                .load(std::sync::atomic::Ordering::Relaxed);

            let shadow = self.workflows.shadow.clone();
            let mut viewer = WorkflowViewer {
                edit_ctx,
                shadow,
                running,
                frozen: self.workflows.frozen,
                node_state: self.workflows.node_state.clone(),
            };

            // Forces new widget state in children after switching or undos so that
            // Snarl will draw our persisted positions and sizes.
            ui.push_id(self.workflows.switch_count, |ui| {
                let mut snarl = self.workflows.snarl.blocking_write();
                SnarlWidget::new()
                    .id_salt(&self.workflows.editing)
                    .style(self.workflows.style)
                    .show(&mut snarl, &mut viewer, ui);
            });

            self.workflows.cast_shadow(viewer.shadow);

            egui::Area::new(egui::Id::new("workflow controls"))
                .default_pos(egui::pos2(16.0, 32.0))
                .default_size(egui::vec2(100.0, 100.0))
                .constrain_to(ui.max_rect())
                .movable(true)
                .show(ui.ctx(), |ui| {
                    egui::Frame::dark_canvas(&Default::default())
                        .inner_margin(8.0)
                        .outer_margin(4.0)
                        .corner_radius(8)
                        .show(ui, |ui| {
                            self.workflow_controls(ui);
                        });
                });

            egui::Window::new(INFO)
                .title_bar(false)
                .constrain_to(ui.max_rect())
                .pivot(Align2::LEFT_BOTTOM)
                .default_pos(egui::pos2(16.0, ui.min_rect().max.y))
                .default_size(egui::vec2(200.0, 100.0))
                .movable(true)
                .show(ui.ctx(), |ui| {
                    let mut description = Cow::Borrowed(self.workflows.shadow.description.as_str());

                    // ui.take_available_space();
                    let size = ui.available_size();

                    egui::ScrollArea::vertical().show(ui, |ui| {
                        ui.add_sized(
                            size,
                            egui::TextEdit::multiline(&mut description)
                                .hint_text("Add a description for this workflow"),
                        );
                    });

                    if let Cow::Owned(desc) = description {
                        self.workflows
                            .cast_shadow(self.workflows.shadow.with_description(&desc));
                    }
                })
        });
    }

    pub fn workflow_controls(&mut self, ui: &mut egui::Ui) {
        let settings = self.settings.clone();
        let errors = self.errors.clone();
        let running = self
            .workflows
            .running
            .load(std::sync::atomic::Ordering::Relaxed);

        ui.set_max_width(150.0);
        ui.vertical_centered_justified(|ui| {
            ui.label("Workflow:");
            ComboBox::from_id_salt("workflow")
                .wrap()
                .width(ui.available_width())
                .selected_text(&self.workflows.editing)
                .show_ui(ui, |ui| {
                    let names = self.workflows.store.workflows.keys().cloned().collect_vec();
                    for name in &names {
                        let original = self.workflows.editing.clone();
                        let mut current = &original;
                        ui.selectable_value(&mut current, name, name);
                        if current != &original {
                            self.workflows.switch(current);
                        }
                    }
                });

            if let Some(renaming) = self.workflows.renaming.as_mut() {
                if ui.text_edit_singleline(renaming).lost_focus() {
                    self.workflows.rename();
                }
            } else if ui.button("Rename").clicked() {
                self.workflows.renaming = Some(self.workflows.editing.clone());
            }

            if ui.button("New").clicked() {
                self.workflows.switch(&Uuid::new_v4().to_string());
            }

            StripBuilder::new(ui)
                .size(Size::exact(16.0))
                .vertical(|mut strip| {
                    strip.cell(|ui| {
                        StripBuilder::new(ui)
                            .sizes(Size::remainder(), 2)
                            .horizontal(|mut strip| {
                                strip.cell(|ui| {
                                    ui.add_enabled_ui(!running, |ui| {
                                        if ui
                                            .button(DOWNLOAD_SIMPLE)
                                            .on_hover_text("Import")
                                            .clicked()
                                            && let Some(path) = rfd::FileDialog::new()
                                                .set_directory(
                                                    settings.view(|s| s.last_workflow_dir.clone()),
                                                )
                                                .pick_file()
                                        {
                                            settings.update(|s| {
                                                s.last_workflow_dir = path
                                                    .parent()
                                                    .map(|p| p.to_path_buf())
                                                    .unwrap_or_default()
                                            });
                                            errors.distil(self.workflows.import(&path));
                                        }
                                    });
                                });
                                strip.cell(|ui| {
                                    if ui.button(UPLOAD_SIMPLE).on_hover_text("Export").clicked()
                                        && let Some(path) = rfd::FileDialog::new()
                                            .set_directory(
                                                settings.view(|s| s.last_workflow_dir.clone()),
                                            )
                                            .set_file_name(format!(
                                                "{}.yml",
                                                self.workflows.editing
                                            ))
                                            .save_file()
                                    {
                                        settings.update(|s| {
                                            s.last_workflow_dir = path
                                                .parent()
                                                .map(|p| p.to_path_buf())
                                                .unwrap_or_default()
                                        });
                                        errors.distil(self.workflows.export(&path));
                                    }
                                });
                            });
                    });
                });

            ui.menu_button("Delete", |ui| {
                if ui.button("OK").clicked() {
                    self.workflows.remove();
                    self.workflows.store.save().unwrap();
                    let next_name = self
                        .workflows
                        .names()
                        .next()
                        .cloned()
                        .unwrap_or("default".to_string());
                    self.workflows.switch(&next_name);
                    ui.close();
                }
            });

            ui.separator();

            // not loving the boilerplate but this gets the right results
            // Maybe should put the whole panel into the vertical strip
            StripBuilder::new(ui)
                .size(Size::exact(16.0))
                .vertical(|mut strip| {
                    strip.cell(|ui| {
                        StripBuilder::new(ui)
                            .sizes(Size::remainder(), 2)
                            .horizontal(|mut strip| {
                                strip.cell(|ui| {
                                    let stack = self.workflows.get_undo_stack().len();
                                    ui.add_enabled_ui(!running && stack > 0, |ui| {
                                        if ui
                                            .button(ARROW_COUNTER_CLOCKWISE)
                                            .on_hover_text(format!("{stack}"))
                                            .clicked()
                                        {
                                            self.workflows.undo();
                                        }
                                    });
                                });
                                strip.cell(|ui| {
                                    let stack = self.workflows.get_redo_stack().len();
                                    ui.add_enabled_ui(!running && stack > 0, |ui| {
                                        if ui
                                            .button(ARROW_CLOCKWISE)
                                            .on_hover_text(format!("{stack}"))
                                            .clicked()
                                        {
                                            self.workflows.redo();
                                        }
                                    });
                                });
                            });
                    });
                });

            ui.separator();

            ui.add_enabled_ui(self.workflows.has_changes(), |ui| {
                if ui.button("Save").clicked() {
                    self.save_workflows();
                }
            });

            ui.add_space(32.0);

            ui.scope(|ui| {
                ui.style_mut().spacing.button_padding.y = 8.0;

                let (frozen_label, frozen_hint) = if running {
                    ("« running »", "Please wait...")
                } else if self.workflows.frozen {
                    ("« frozen »", "Click to re-enable editing.")
                } else {
                    ("« editing »", "Click to prevent new changes.")
                };

                ui.toggle_value(&mut self.workflows.frozen, frozen_label)
                    .on_hover_text(frozen_hint);
            });

            ui.separator();
            ui.scope(|ui| {
                // Bigger button
                ui.style_mut().spacing.button_padding.y = 16.0;
                if running {
                    let interrupting = self.workflows.interrupt.load(Ordering::Relaxed);
                    ui.add_enabled_ui(!interrupting, |ui| {
                        if ui.button(stop_layout(interrupting)).clicked() {
                            self.workflows.interrupt.store(true, Ordering::Relaxed);
                        }
                    });
                } else if ui.button(play_layout()).clicked() {
                    self.exec_workflow();
                }
            });

            // TODO: pause and cancel buttons
        });
    }

    fn save_workflows(&mut self) {
        tracing::info!(
            "Saving {} to workflows...changed? {}",
            &self.workflows.editing,
            !self.workflows.shadow.fast_eq(&self.workflows.baseline)
        );

        self.workflows
            .store
            .put(&self.workflows.editing, self.workflows.shadow.clone());
        self.workflows.store.save().unwrap();

        let mut snarl = self.workflows.snarl.blocking_write();
        *snarl = Snarl::try_from(self.workflows.shadow.clone()).unwrap();
        self.workflows.baseline = self.workflows.shadow.clone();
    }

    // TODO: decouple editing and execution workflows
    /// Runs the workflow currently being edited and updates nodes in the viewer with results.
    pub fn exec_workflow(&mut self) {
        let snarl_ = self.workflows.snarl.clone();
        let mut target = { self.workflows.snarl.blocking_read().clone() };
        let task_count_ = self.task_count.clone();

        let mut exec = {
            let run_ctx = RunContext::builder()
                .agent_factory(self.agent_factory.clone())
                .transmuter(self.transmuter.clone())
                .interrupt(self.workflows.interrupt.clone())
                .history(self.session.history.clone())
                .user_prompt(self.prompt.read().unwrap().clone())
                .model(self.settings.view(|s| s.llm_model.clone()))
                .temperature(self.settings.view(|s| s.temperature))
                .errors(self.errors.clone())
                .build();

            self.workflows.interrupt.store(false, Ordering::Relaxed);

            let mut exec = WorkflowRunner::builder().run_ctx(run_ctx).build();

            self.workflows.node_state = exec.init(&self.workflows.shadow);

            exec
        };

        let session = self.session.clone();
        let running = self.workflows.running.clone();
        let errors = self.errors.clone();
        let interrupt = self.workflows.interrupt.clone();
        let outputs: Arc<ArcSwap<im::OrdMap<String, crate::workflow::Value>>> = Default::default();
        let started = chrono::offset::Local::now();

        self.workflows.outputs.push_back((started, outputs.clone()));

        self.rt.spawn(async move {
            task_count_.fetch_add(1, Ordering::Relaxed);
            running.store(true, std::sync::atomic::Ordering::Relaxed);

            loop {
                if interrupt.load(Ordering::Relaxed) {
                    break;
                }

                match exec.step(&mut target).await {
                    Ok(false) => break,
                    Ok(true) => {
                        let mut snarl = snarl_.write().await;
                        *snarl = target.clone();
                    }
                    Err(err) => {
                        errors.push(err.into());
                        break;
                    }
                }

                let rx = exec.run_ctx.outputs.receiver();
                while !rx.is_empty() {
                    let Ok((label, value)) = rx.recv_async().await else {
                        break;
                    };
                    tracing::info!("Received output {label}: {value:?}");

                    outputs.rcu(|it| it.update(label.clone(), value.clone()));
                }
            }

            errors.distil(session.save());
            running.store(false, std::sync::atomic::Ordering::Relaxed);
            task_count_.fetch_sub(1, Ordering::Relaxed);
        });
    }
}

fn play_layout() -> egui::text::LayoutJob {
    use egui::{Align, FontSelection, Style, text::LayoutJob};

    let style = Style::default();
    let mut layout_job = LayoutJob::default();
    RichText::new(PLAY)
        .color(egui::Color32::GREEN)
        .strong()
        .heading()
        .append_to(
            &mut layout_job,
            &style,
            FontSelection::Default,
            Align::Center,
        );

    RichText::new(" Run")
        .color(style.visuals.text_color())
        .append_to(
            &mut layout_job,
            &style,
            FontSelection::Default,
            Align::Center,
        );

    layout_job
}

fn stop_layout(stopping: bool) -> egui::text::LayoutJob {
    use egui::{Align, FontSelection, Style, text::LayoutJob};

    let style = Style::default();
    let mut layout_job = LayoutJob::default();
    RichText::new(STOP)
        .color(egui::Color32::RED)
        .strong()
        .heading()
        .append_to(
            &mut layout_job,
            &style,
            FontSelection::Default,
            Align::Center,
        );

    RichText::new(if stopping { " Stopping" } else { " Stop" })
        .color(style.visuals.text_color())
        .append_to(
            &mut layout_job,
            &style,
            FontSelection::Default,
            Align::Center,
        );

    layout_job
}
