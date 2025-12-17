use std::{fs::OpenOptions, io::Cursor};

use egui_phosphor::regular::FLOPPY_DISK;

use crate::{config::ConfigExt as _, utils::ErrorDistiller as _, workflow::write_value};

impl super::AppState {
    pub fn outputs_ui(&mut self, ui: &mut egui::Ui) {
        let settings = self.settings.clone();
        let errors = self.errors.clone();
        let mut trash_idx = None;
        egui::CentralPanel::default().show_inside(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                for (run_i, (dt, outputs)) in self.workflows.outputs.iter().enumerate() {
                    ui.push_id(run_i, |ui| {
                        if run_i > 0 {
                            ui.separator();
                        }

                        egui::Sides::new().show(
                            ui,
                            |ui| {
                                ui.label(dt.to_string());
                            },
                            |ui| {
                                ui.menu_button("Delete", |ui| {
                                    if ui.button("OK").clicked() {
                                        trash_idx = Some(run_i);
                                    }
                                })
                                .response
                                .on_hover_text("Delete outputs from this run");
                            },
                        );

                        let outputs = outputs.load();

                        if outputs.is_empty() {
                            ui.vertical_centered(|ui| ui.monospace("No outputs"));
                        }

                        for (k, v) in outputs.iter() {
                            egui::collapsing_header::CollapsingState::load_with_default_open(
                                ui.ctx(),
                                ui.make_persistent_id(format!("run #{run_i} output: {k}")),
                                false,
                            )
                            .show_header(ui, |ui| {
                                egui::Sides::new().show(
                                    ui,
                                    |ui| ui.label(k),
                                    |ui| {
                                        if ui.button(FLOPPY_DISK).on_hover_text("Save").clicked()
                                            && let Some(path) = rfd::FileDialog::new()
                                                .set_directory(
                                                    settings.view(|s| s.last_output_dir.clone()),
                                                )
                                                .set_file_name(k)
                                                .save_file()
                                            && let Some(mut fh) = errors.distil(
                                                OpenOptions::new()
                                                    .write(true)
                                                    .create(true)
                                                    .truncate(true)
                                                    .open(path.as_path())
                                                    .map_err(|e| e.into()),
                                            )
                                        {
                                            settings.update(|s| {
                                                s.last_output_dir = path
                                                    .as_path()
                                                    .parent()
                                                    .map(|p| p.to_path_buf())
                                                    .unwrap_or_default()
                                            });
                                            errors.distil(write_value(&mut fh, v));
                                        }
                                    },
                                );
                            })
                            .body(|ui| {
                                let mut writer = Cursor::new(Vec::new());
                                let _ = write_value(&mut writer, v);
                                let bytes = writer.into_inner();
                                ui.label(String::from_utf8(bytes).unwrap_or_default());
                            });
                        }
                    });
                }
            });
        });

        if let Some(idx) = trash_idx {
            self.workflows.outputs.remove(idx);
        }
    }
}
