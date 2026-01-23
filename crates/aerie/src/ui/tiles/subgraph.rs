use egui_phosphor::regular::ARROW_CLOCKWISE;
use egui_phosphor::regular::ARROW_COUNTER_CLOCKWISE;
use egui_snarl::ui::SnarlWidget;
use itertools::Itertools as _;
use std::convert::identity;
use std::sync::atomic::Ordering;
use std::time::Duration;

use egui_extras::{Size, StripBuilder};

use crate::config::ConfigExt;
use crate::ui::AppEvent;
use crate::ui::runner::play_button;
use crate::ui::runner::stop_button;
use crate::ui::shortcuts::SHORTCUT_RUN;
use crate::ui::shortcuts::ShortcutHandler;
use crate::ui::workflow::get_snarl_style;

impl super::AppState {
    pub fn subgraph_ui(&mut self, ui: &mut egui::Ui) {
        let running = self
            .workflows
            .running
            .load(std::sync::atomic::Ordering::Relaxed);

        if !running && ui.ctx().input_mut(|i| i.consume_shortcut(&SHORTCUT_RUN)) {
            self.events.insert(AppEvent::UserRunWorkflow);
        }
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

            let widget = SnarlWidget::new()
                .id(viewer.view_id)
                .style(get_snarl_style());

            let pointee = widget.show(&mut snarl, viewer, ui).contains_pointer();

            // Unfortunately, there's no event for node movement so we have to
            // iterate through the whole collection to find moved nodes.
            viewer.cast_positions(&snarl);

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
        });
    }

    pub fn subgraph_controls(&mut self, ui: &mut egui::Ui) {
        let settings = self.settings.clone();
        let running = self
            .workflows
            .running
            .load(std::sync::atomic::Ordering::Relaxed);

        ui.set_max_width(150.0);
        ui.vertical_centered_justified(|ui| {
            ui.label("subgraph:");

            let names = self.workflows.view_stack.names().collect_vec();
            for (i, name) in names.iter().enumerate().rev() {
                if i == 0 {
                    ui.add_enabled(false, egui::Button::new(name));
                } else if ui.button(name).clicked() {
                    self.events.insert(crate::ui::AppEvent::LeaveSubgraph(i));
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
                } else if ui.add(play_button()).clicked() {
                    self.events.insert(AppEvent::UserRunWorkflow);
                }
            });
        });
    }
}
