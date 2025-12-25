use egui_extras::{Size, StripBuilder};
use egui_phosphor::regular::{DOWNLOAD_SIMPLE, MAGIC_WAND, PENCIL, ROCKET, TRASH, UPLOAD_SIMPLE};
use std::{collections::BTreeSet, sync::atomic::Ordering};

use crate::{config::ConfigExt as _, utils::ErrorDistiller as _};

impl super::AppState {
    pub fn nav_ui(&mut self, ui: &mut egui::Ui) {
        let session = self.session.clone();
        let settings = self.settings.clone();
        let errors = self.errors.clone();

        let running = self.task_count.load(Ordering::Relaxed) > 0;
        egui::CentralPanel::default().show_inside(ui, |ui| {
            ui.vertical_centered_justified(|ui| {
                ui.add_enabled_ui(!running, |ui| {
                    egui::ComboBox::from_id_salt("session_list")
                        .wrap()
                        .width(ui.available_width())
                        .selected_text(session.name())
                        .show_ui(ui, |ui| {
                            let original = session.name();
                            let mut current = &original;
                            let blank = String::new();

                            ui.selectable_value(&mut current, &blank, "");

                            let names = session.list();
                            for name in &names {
                                ui.selectable_value(&mut current, name, name);
                            }

                            if current != &original {
                                errors.distil(self.session.switch(current));
                                settings.update(|sets| sets.session = self.session.name_opt());
                            }
                        });

                    if let Some(renaming) = self.rename_session.as_mut() {
                        let editor = ui.text_edit_singleline(renaming);
                        if editor.lost_focus() {
                            if !renaming.is_empty() {
                                errors.distil(self.session.rename(renaming));
                                settings.update(|sets| sets.session = self.session.name_opt());
                            }
                            self.rename_session = None;
                        }
                        editor.request_focus();
                    }

                    StripBuilder::new(ui)
                        .size(Size::exact(16.0))
                        .vertical(|mut strip| {
                            strip.cell(|ui| {
                                StripBuilder::new(ui)
                                    .sizes(Size::remainder(), 5)
                                    .horizontal(|mut strip| {
                                        strip.cell(|ui| {
                                            if ui
                                                .button(MAGIC_WAND)
                                                .on_hover_text("Create")
                                                .clicked()
                                            {
                                                let datetime = chrono::offset::Local::now();
                                                let timestamp = datetime
                                                    .format("unnamed-%Y-%m-%dT%H:%M:%S")
                                                    .to_string();
                                                errors.distil(self.session.switch(&timestamp));
                                                errors.distil(self.session.save());
                                                settings.update(|sets| {
                                                    sets.session = self.session.name_opt()
                                                });
                                            }
                                        });
                                        strip.cell(|ui| {
                                            if ui.button(PENCIL).on_hover_text("Rename").clicked() {
                                                self.rename_session = Some(self.session.name());
                                            }
                                        });
                                        strip.cell(|ui| {
                                            if ui
                                                .button(DOWNLOAD_SIMPLE)
                                                .on_hover_text("Import")
                                                .clicked()
                                                && let Some(path) = rfd::FileDialog::new()
                                                    .set_directory(
                                                        settings
                                                            .view(|s| s.last_export_dir.clone()),
                                                    )
                                                    .pick_file()
                                            {
                                                settings.update(|s| {
                                                    s.last_export_dir = path
                                                        .parent()
                                                        .map(|p| p.to_path_buf())
                                                        .unwrap_or_default()
                                                });
                                                errors.distil(self.session.import(&path));
                                            }
                                        });
                                        strip.cell(|ui| {
                                            if ui
                                                .button(UPLOAD_SIMPLE)
                                                .on_hover_text("Export")
                                                .clicked()
                                                && let Some(path) = rfd::FileDialog::new()
                                                    .set_directory(
                                                        settings
                                                            .view(|s| s.last_export_dir.clone()),
                                                    )
                                                    .set_file_name(format!(
                                                        "{}.yml",
                                                        session.name()
                                                    ))
                                                    .save_file()
                                            {
                                                settings.update(|s| {
                                                    s.last_export_dir = path
                                                        .parent()
                                                        .map(|p| p.to_path_buf())
                                                        .unwrap_or_default()
                                                });
                                                errors.distil(self.session.export(&path));
                                            }
                                        });
                                        strip.cell(|ui| {
                                            ui.menu_button(TRASH, |ui| {
                                                if ui.button("OK").clicked() {
                                                    let old_name = session.name();
                                                    errors.distil(self.session.switch(""));
                                                    errors.distil(self.session.delete(&old_name));
                                                    settings.update(|sets| {
                                                        sets.session = self.session.name_opt()
                                                    });
                                                    ui.close();
                                                }
                                            })
                                            .response
                                            .on_hover_text("Delete");
                                        });
                                    });
                            });
                        });
                });
            });

            ui.separator();
            ui.vertical_centered(|ui| ui.monospace("Branches"));
            egui::ScrollArea::vertical().show(ui, |ui| {
                let lineage = self.session.view(|history| history.lineage());

                if let Some(children) = lineage.get("") {
                    for child in children {
                        self.render_subtree(ui, &lineage, child);
                    }
                }
            });
        });

        self.rename_branch_dialog(ui);
    }

    fn render_subtree(
        &mut self,
        ui: &mut egui::Ui,
        lineage: &std::collections::BTreeMap<String, BTreeSet<String>>,
        cursor: &str,
    ) {
        let errors = self.errors.clone();
        if let Some(children) = lineage.get(cursor) {
            let id = ui.make_persistent_id(format!("navigator_{cursor}"));
            egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, true)
                .show_header(ui, |ui| {
                    egui::Sides::new().show(
                        ui,
                        |ui| {
                            errors.distil(self.session.transform(|history| {
                                let mut head = history.head.as_str();
                                ui.selectable_value(&mut head, cursor, cursor);
                                Ok(history.switch(head))
                            }));
                        },
                        |ui| {
                            if ui.button(PENCIL).on_hover_text("Rename").clicked() {
                                self.rename_branch = Some(cursor.to_string());
                            }
                            if ui.button(ROCKET).on_hover_text("Promote").clicked() {
                                errors.distil(
                                    self.session
                                        .transform(|history| history.promote_branch(cursor)),
                                );
                            }
                        },
                    );
                })
                .body(|ui| {
                    for child in children {
                        self.render_subtree(ui, lineage, child);
                    }
                });
        } else {
            egui::Sides::new().show(
                ui,
                |ui| {
                    errors.distil(self.session.transform(|history| {
                        let mut head = history.head.as_str();
                        ui.selectable_value(&mut head, cursor, cursor);
                        Ok(history.switch(head))
                    }));
                },
                |ui| {
                    if ui.button(PENCIL).on_hover_text("Rename").clicked() {
                        self.rename_branch = Some(cursor.to_string());
                    }
                    if ui.button(ROCKET).on_hover_text("Promote").clicked() {
                        errors.distil(
                            self.session
                                .transform(|history| history.promote_branch(cursor)),
                        );
                    }

                    ui.add_enabled_ui(self.session.view(|history| history.head == cursor), |ui| {
                        if ui.button(TRASH).on_hover_text("Prune").clicked() {
                            errors.distil(
                                self.session
                                    .transform(|history| history.prune_branch(cursor)),
                            );

                            self.session.scratch.clear();
                        }
                    });
                },
            );
        }
    }

    fn rename_branch_dialog(&mut self, ui: &mut egui::Ui) {
        let errors = self.errors.clone();

        let Some(old_name) = &self.rename_branch else {
            return;
        };

        let mut submit = false;
        let unique_name = !self.new_branch.is_empty()
            && self
                .session
                .view(|history| !history.has_branch(&self.new_branch));

        let title = "Rename Branch";

        let modal = egui::Modal::new(egui::Id::new(title)).show(ui.ctx(), |ui| {
            ui.set_width(250.0);

            ui.heading(title);

            ui.label("Name:");
            ui.text_edit_singleline(&mut self.new_branch)
                .request_focus();

            ui.separator();

            egui::Sides::new().show(
                ui,
                |_ui| {},
                |ui| {
                    ui.add_enabled_ui(unique_name, |ui| {
                        if ui.button("Ok").clicked() {
                            submit = true;
                        }
                    });
                    if ui.button("Cancel").clicked() {
                        ui.close();
                    }
                },
            );

            submit |= unique_name && ui.input(|i| i.key_pressed(egui::Key::Enter));

            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                ui.close();
            }

            if submit {
                let new_name = std::mem::take(&mut self.new_branch);
                errors.distil(
                    self.session
                        .transform(|history| history.rename_branch(old_name, &new_name)),
                );
                ui.close();
            }
        });

        if modal.should_close() {
            self.rename_branch = None;
        }
    }
}
