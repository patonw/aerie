use egui::Label;
use egui::RichText;
use egui_phosphor::regular::ARROW_CLOCKWISE;
use egui_phosphor::regular::ARROW_COUNTER_CLOCKWISE;
use egui_snarl::ui::SnarlWidget;
use std::convert::identity;
use std::sync::atomic::Ordering;
use std::time::Duration;

use egui_extras::{Size, StripBuilder};

use crate::config::ConfigExt;
use crate::ui::AppEvent;
use crate::ui::ShowHelp;
use crate::ui::runner::play_button;
use crate::ui::runner::stop_button;
use crate::ui::shortcuts::SHORTCUT_HELP;
use crate::ui::shortcuts::SHORTCUT_RUN;
use crate::ui::shortcuts::Shortcut;
use crate::ui::shortcuts::ShortcutHandler;
use crate::ui::shortcuts::show_shortcuts;
use crate::ui::workflow::get_subgraph_style;
use crate::workflow::nodes::Flavor;

// TODO: mirror inputs from Start to Control node on Loop subgraphs
impl super::AppState {
    pub fn subgraph_ui(&mut self, ui: &mut egui::Ui) {
        let running = self
            .workflows
            .running
            .load(std::sync::atomic::Ordering::Relaxed);
        let busy = self.task_count.load(Ordering::Relaxed) > 0;

        if !busy && !running && ui.ctx().input_mut(|i| i.consume_shortcut(&SHORTCUT_RUN)) {
            self.events.insert(AppEvent::UserRunWorkflow);
        }

        self.workflows.check_graph();

        egui::CentralPanel::default().show_inside(ui, |ui| {
            // Forces new widget state in children after switching or undos so that
            // Snarl will draw our persisted positions and sizes.
            let mut snarl = self.workflows.view_stack.leaf_snarl().unwrap();

            let shadow = self.workflows.view_stack.leaf();
            let viewer = self.workflow_viewer();

            // Needed for preserving changes by events, but is there a better way?
            // Maybe we can make changes directly to the stack?
            // But then we'd need shared ownership of the stack.
            viewer.shadow = shadow;

            let style = egui_snarl::ui::SnarlStyle {
                wire_width: Some(1f32.max(viewer.transform.inverse().scaling * 0.5)),
                ..get_subgraph_style()
            };

            let widget = SnarlWidget::new().id(viewer.view_id).style(style);

            let pointee = widget.show(&mut snarl, viewer, ui).contains_pointer();

            // Unfortunately, there's no event for node movement so we have to
            // iterate through the whole collection to find moved nodes.
            viewer.cast_positions(&snarl);

            if cfg!(feature = "debug-ui") {
                let egui::Pos2 { x, y } = ui.max_rect().right_top();
                let over_rect =
                    egui::Rect::from_two_pos(egui::pos2(x - 64.0, y), egui::pos2(x, y + 20.0));
                let scale_debug =
                    egui::Label::new(format!("Scale: {:.02}", viewer.transform.scaling));
                ui.place(over_rect, scale_debug);
            }

            if pointee {
                let mut shortcuts = ShortcutHandler::builder()
                    .snarl(&mut snarl)
                    .viewer(viewer)
                    .build();

                shortcuts.viewer_shortcuts(ui, widget);
            }

            let shadow = viewer.shadow.clone();
            self.workflows
                .view_stack
                .propagate(shadow, identity)
                .unwrap();

            egui::Area::new(egui::Id::new("subgraph controls"))
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
                            self.subgraph_controls(ui);
                        });
                });

            if ui.ui_contains_pointer()
                && let Some(exec_id) = self.workflows.view_stack.exec_id()
            {
                ui.ctx().input_mut(|i| {
                    let pass = &mut self.workflows.view_stack.passes[0];
                    let limit = self.workflows.node_state.max_pass(exec_id);

                    if i.consume_shortcut(&Shortcut::PrevPass.key()) {
                        *pass = pass.checked_sub(1).unwrap_or(0);
                    }

                    if i.consume_shortcut(&Shortcut::NextPass.key()) {
                        *pass = (*pass + 1).clamp(0, limit);
                    }
                });
            }
        });

        if ui.ctx().input_mut(|i| i.consume_shortcut(&SHORTCUT_HELP)) {
            tracing::info!("showing help");
            self.show_help = Some(ShowHelp::Subgraph);
        }

        if let Some(ShowHelp::Subgraph) = self.show_help {
            let modal = egui::Modal::new(egui::Id::new("Shortcuts")).show(ui.ctx(), |ui| {
                show_shortcuts(ui, ShowHelp::Subgraph);
            });
            if modal.should_close() {
                self.show_help = None;
            }
        }
    }

    pub fn subgraph_controls(&mut self, ui: &mut egui::Ui) {
        let settings = self.prefs.clone();
        let running = self
            .workflows
            .running
            .load(std::sync::atomic::Ordering::Relaxed);

        let busy = self.task_count.load(Ordering::Relaxed) > 0;

        ui.set_max_width(150.0);
        ui.vertical_centered_justified(|ui| {
            ui.add(Label::new(RichText::new("Subgraph").heading()).selectable(false));

            for info in self.workflows.view_stack.iter_levels() {
                let limit = self.workflows.node_state.max_pass(info.exec_id);

                let resp = if info.flavor == Flavor::Simple {
                    ui.add(egui::Button::new(RichText::new(info.name)))
                } else {
                    let mut resp = None;
                    StripBuilder::new(ui)
                        .size(Size::exact(18.0))
                        .vertical(|mut strip| {
                            strip.cell(|ui| {
                                ui.style_mut().spacing.item_spacing.x = 1.0;
                                StripBuilder::new(ui)
                                    .size(Size::exact(20.0))
                                    .size(Size::remainder())
                                    .size(Size::exact(20.0))
                                    .horizontal(|mut strip| {
                                        strip.cell(|ui| {
                                            if ui.button("-").clicked() {
                                                *info.pass = info.pass.checked_sub(1).unwrap_or(0);
                                            }
                                        });
                                        strip.cell(|ui| {
                                            // No corner radius setter for this yet
                                            let widget = ui.add(
                                                egui::DragValue::new(info.pass)
                                                    .range(0..=limit)
                                                    .clamp_existing_to_range(false)
                                                    .prefix(format!("{}[", info.name))
                                                    .suffix("]"),
                                            );

                                            let painter = ui.painter();
                                            let rect = widget.rect;
                                            let (ratio, color) = if limit > 0 {
                                                (
                                                    *info.pass as f32 / limit as f32,
                                                    egui::Color32::from_rgb(0x0, 0xff, 0xbb),
                                                )
                                            } else {
                                                (1.0, egui::Color32::GOLD)
                                            };

                                            let (left, _right) =
                                                rect.split_left_right_at_fraction(ratio);

                                            painter.rect_filled(
                                                left,
                                                ui.style().visuals.widgets.active.corner_radius,
                                                color.gamma_multiply(0.2),
                                            );

                                            if widget.contains_pointer() {
                                                widget.ctx.input_mut(|i| {
                                                    if i.consume_shortcut(&Shortcut::PrevPass.key())
                                                    {
                                                        *info.pass =
                                                            info.pass.checked_sub(1).unwrap_or(0);
                                                    }

                                                    if i.consume_shortcut(&Shortcut::NextPass.key())
                                                    {
                                                        *info.pass =
                                                            (*info.pass + 1).clamp(0, limit);
                                                    }
                                                });
                                            }
                                            resp = Some(widget);
                                        });
                                        strip.cell(|ui| {
                                            if ui.button("+").clicked() {
                                                *info.pass = (*info.pass + 1).clamp(0, limit);
                                            }
                                        });
                                    });
                            });
                        });
                    resp.unwrap()
                };

                if info.depth > 0 && resp.double_clicked() {
                    self.events
                        .insert(crate::ui::AppEvent::LeaveSubgraph(info.depth));
                    resp.surrender_focus();
                }
            }

            ui.separator();

            StripBuilder::new(ui)
                .size(Size::exact(16.0))
                .vertical(|mut strip| {
                    strip.cell(|ui| {
                        StripBuilder::new(ui)
                            .sizes(Size::remainder(), 2)
                            .horizontal(|mut strip| {
                                strip.cell(|ui| {
                                    let stack = self.workflows.get_undo_count();
                                    ui.add_enabled_ui(!running && stack > 0, |ui| {
                                        if ui
                                            .button(ARROW_COUNTER_CLOCKWISE)
                                            .on_hover_text(format!("{stack}"))
                                            .clicked()
                                        {
                                            // TODO: stay in this view when undoing
                                            self.workflows.undo();
                                        }
                                    });
                                });
                                strip.cell(|ui| {
                                    let stack = self.workflows.get_redo_count();
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

            if !settings.view(|s| s.autosave) {
                ui.add_enabled_ui(self.workflows.has_changes(), |ui| {
                    if ui.button("Save").clicked() {
                        self.workflows.save();
                    }
                });
            } else if !self.workflows.frozen
                && self.workflows.has_changes()
                && self.workflows.modtime.elapsed().unwrap_or(Duration::ZERO)
                    > Duration::from_secs(2)
            {
                self.workflows.save();
            }

            ui.separator();

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
                        if ui.add(stop_button(interrupting)).clicked() {
                            self.workflows.interrupt.store(true, Ordering::Relaxed);
                        }
                    });
                } else if ui.add_enabled(!busy, play_button()).clicked() {
                    self.events.insert(AppEvent::UserRunWorkflow);
                }
            });
        });
    }
}
