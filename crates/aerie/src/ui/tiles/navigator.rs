use egui_phosphor::regular::{PENCIL, ROCKET, TRASH};
use std::collections::BTreeSet;

impl super::AppBehavior {
    pub fn nav_ui(&mut self, ui: &mut egui::Ui) {
        egui::CentralPanel::default().show_inside(ui, |ui| {
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
        if let Some(children) = lineage.get(cursor) {
            let id = ui.make_persistent_id(format!("navigator_{cursor}"));
            egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, true)
                .show_header(ui, |ui| {
                    egui::Sides::new().show(
                        ui,
                        |ui| {
                            self.session.update(|history| {
                                ui.selectable_value(
                                    &mut history.head,
                                    Some(cursor.to_string()),
                                    cursor,
                                );
                            });
                        },
                        |ui| {
                            if ui.button(PENCIL).on_hover_text("Rename").clicked() {
                                self.rename_branch = Some(cursor.to_string());
                            }
                            if ui.button(ROCKET).on_hover_text("Promote").clicked() {
                                self.session.update(|history| {
                                    history.promote_branch(cursor);
                                });
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
                    self.session.update(|history| {
                        ui.selectable_value(&mut history.head, Some(cursor.to_string()), cursor);
                    });
                },
                |ui| {
                    if ui.button(PENCIL).on_hover_text("Rename").clicked() {
                        self.rename_branch = Some(cursor.to_string());
                    }
                    if ui.button(ROCKET).on_hover_text("Promote").clicked() {
                        self.session.update(|history| {
                            history.promote_branch(cursor);
                        });
                    }
                    if ui.button(TRASH).on_hover_text("Prune").clicked() {
                        self.session.update(|history| {
                            history.prune_branch(cursor);
                        });
                    }
                },
            );
        }
    }

    fn rename_branch_dialog(&mut self, ui: &mut egui::Ui) {
        let Some(old_name) = &self.rename_branch else {
            return;
        };

        let mut submit = false;
        let unique_name = !self.new_branch.is_empty()
            && self
                .session
                .view(|history| !history.branches.contains_key(&self.new_branch));

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
                self.session
                    .update(|history| history.rename_branch(old_name, &new_name));
                ui.close();
            }
        });

        if modal.should_close() {
            self.rename_branch = None;
        }
    }
}
