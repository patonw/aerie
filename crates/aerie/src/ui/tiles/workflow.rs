use std::{borrow::Cow, convert::identity, sync::atomic::Ordering, time::Duration};

use egui::{Align2, ComboBox};
use egui_extras::{Size, StripBuilder};
use egui_phosphor::regular::{
    ARROW_CLOCKWISE, ARROW_COUNTER_CLOCKWISE, DOWNLOAD_SIMPLE, INFO, MAGIC_WAND, PENCIL, TRASH,
    UPLOAD_SIMPLE,
};
use egui_snarl::ui::SnarlWidget;
use itertools::Itertools;

use crate::{
    config::ConfigExt as _,
    ui::{
        AppEvent, ShowHelp,
        runner::{play_button, stop_button},
        shortcuts::{SHORTCUT_HELP, SHORTCUT_RUN, ShortcutHandler, show_shortcuts, squelch},
        workflow::get_snarl_style,
    },
    utils::ErrorDistiller as _,
    workflow::store::WorkflowStore as _,
};

impl super::AppState {
    pub fn workflow_ui(&mut self, ui: &mut egui::Ui) {
        let running = self
            .workflows
            .running
            .load(std::sync::atomic::Ordering::Relaxed);

        // Shortcuts at the start of the fn will run even if other widget focused
        if !running && ui.ctx().input_mut(|i| i.consume_shortcut(&SHORTCUT_RUN)) {
            self.events.insert(AppEvent::UserRunWorkflow);
        }

        egui::CentralPanel::default().show_inside(ui, |ui| {
            // Forces new widget state in children after switching or undos so that
            // Snarl will draw our persisted positions and sizes.
            let mut snarl = self.workflows.view_stack.root_snarl().unwrap();

            let (mut shadow, widget) = {
                let shadow = self.workflows.view_stack.leaf();
                let viewer = self.workflow_viewer();
                // Needed for preserving changes by events, but is there a better way?
                viewer.shadow = shadow;

                let widget = SnarlWidget::new()
                    .id(viewer.view_id)
                    .style(get_snarl_style());
                widget.show(&mut snarl, viewer, ui);

                // Unfortunately, there's no event for node movement so we have to
                // iterate through the whole collection to find moved nodes.
                viewer.cast_positions(&snarl);

                (viewer.shadow.clone(), widget)
            };

            egui::Window::new(INFO)
                .title_bar(false)
                .constrain_to(ui.max_rect())
                .pivot(Align2::LEFT_BOTTOM)
                .default_pos(egui::pos2(16.0, ui.min_rect().max.y))
                .default_size(egui::vec2(200.0, 100.0))
                .movable(true)
                .show(ui.ctx(), |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        ui.selectable_value(&mut self.workflows.meta_edit, 0, "Description");
                        ui.selectable_value(&mut self.workflows.meta_edit, 1, "Schema");
                    });

                    let size = ui.available_size();

                    egui::ScrollArea::vertical().show(ui, |ui| {
                        if self.workflows.meta_edit == 0 {
                            let mut description =
                                Cow::Borrowed(self.workflows.shadow.description.as_str());

                            squelch(
                                ui.add_sized(
                                    size,
                                    egui::TextEdit::multiline(&mut description)
                                        .hint_text("Add a description for this workflow"),
                                ),
                            );

                            if let Cow::Owned(desc) = description {
                                shadow = shadow.with_description(&desc);
                            }
                        } else {
                            let mut schema = Cow::Borrowed(self.workflows.shadow.schema.as_str());

                            squelch(
                                ui.add_sized(
                                    size,
                                    egui::TextEdit::multiline(&mut schema)
                                        .hint_text("Add a schema for this workflow"),
                                ),
                            );

                            if let Cow::Owned(schema) = schema {
                                shadow = shadow.with_schema(&schema);
                            }
                        }
                    });
                });

            let shadow = {
                // Must trigger other shortcuts after editor otherwise spurious activations
                let viewer = self.workflow_viewer();
                viewer.shadow = shadow.clone();
                let mut shortcuts = ShortcutHandler::builder()
                    .snarl(&mut snarl)
                    .viewer(viewer)
                    .build();

                shortcuts.viewer_shortcuts(ui, widget);
                viewer.shadow.clone()
            };

            self.workflows
                .view_stack
                .propagate(shadow, identity)
                .unwrap();

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
        });

        if ui.ctx().input_mut(|i| i.consume_shortcut(&SHORTCUT_HELP)) {
            tracing::info!("showing help");
            self.show_help = Some(ShowHelp::Workflow);
        }

        if let Some(ShowHelp::Workflow) = self.show_help {
            let modal = egui::Modal::new(egui::Id::new("Shortcuts")).show(ui.ctx(), |ui| {
                show_shortcuts(ui);
            });
            if modal.should_close() {
                self.show_help = None;
            }
        }
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
                    let original = self.workflows.editing.clone();
                    let mut current = &original;

                    let blank = String::new();
                    ui.selectable_value(&mut current, &blank, "");

                    let names = self.workflows.names().map(|s| s.to_string()).collect_vec();
                    for name in &names {
                        ui.selectable_value(&mut current, name, name);
                    }
                    if current != &original {
                        if settings.view(|s| s.autosave) {
                            self.workflows.save();
                        }

                        self.workflows.switch(current);
                    }
                });

            if let Some(renaming) = self.workflows.renaming.as_mut() {
                let editor = ui.text_edit_singleline(renaming);
                if editor.lost_focus() {
                    errors.distil(self.workflows.rename());
                }

                editor.request_focus();
            }

            StripBuilder::new(ui)
                .sizes(Size::exact(16.0), 2)
                .vertical(|mut strip| {
                    strip.cell(|ui| {
                        StripBuilder::new(ui)
                            .sizes(Size::remainder(), 3)
                            .horizontal(|mut strip| {
                                strip.cell(|ui| {
                                    if ui.button(MAGIC_WAND).on_hover_text("Create").clicked() {
                                        let datetime = chrono::offset::Local::now();
                                        let timestamp = datetime
                                            .format("unnamed-%Y-%m-%dT%H:%M:%S")
                                            .to_string();
                                        self.workflows.store.put(&timestamp, Default::default());
                                        self.workflows.switch(&timestamp);
                                    }
                                });
                                strip.cell(|ui| {
                                    if ui.button(PENCIL).on_hover_text("Rename").clicked() {
                                        self.workflows.renaming =
                                            Some(self.workflows.editing.clone());
                                    }
                                });
                                strip.cell(|ui| {
                                    ui.menu_button(TRASH, |ui| {
                                        if ui.button("OK").clicked() {
                                            errors.distil(self.workflows.remove());
                                            let next_name = self
                                                .workflows
                                                .names()
                                                .next()
                                                .map(|s| s.to_string())
                                                .unwrap_or("default".to_string());
                                            self.workflows.switch(&next_name);
                                            ui.close();
                                        }
                                    })
                                    .response
                                    .on_hover_text("Delete");
                                });
                            });
                    });
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
                                                    settings.view(|s| s.last_export_dir.clone()),
                                                )
                                                .pick_file()
                                        {
                                            settings.update(|s| {
                                                s.last_export_dir = path
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
                                                settings.view(|s| s.last_export_dir.clone()),
                                            )
                                            .set_file_name(format!(
                                                "{}.yml",
                                                self.workflows.editing
                                            ))
                                            .save_file()
                                    {
                                        settings.update(|s| {
                                            s.last_export_dir = path
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
                                    let stack = self.workflows.get_undo_count();
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
