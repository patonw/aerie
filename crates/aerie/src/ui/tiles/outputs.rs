use std::{io::Cursor, sync::atomic::Ordering, time::Duration};

use egui_phosphor::regular::FLOPPY_DISK;

use crate::{
    config::ConfigExt as _,
    utils::{ErrorDistiller as _, spawn_wherever},
    workflow::write_value,
};

impl super::AppState {
    pub fn outputs_ui(&mut self, ui: &mut egui::Ui) {
        let errors = self.errors.clone();
        let mut trash_idx = None;

        let scroll_bottom =
            self.task_count.load(Ordering::Relaxed) > 0 && self.data.prefs.view(|s| s.autoscroll);

        egui::CentralPanel::default().show_inside(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                for (run_i, entry) in self.workflows.outputs.iter().enumerate() {
                    let dt = &entry.started;
                    let outputs = &entry.outputs;
                    ui.push_id(run_i, |ui| {
                        if run_i > 0 {
                            ui.separator();
                        }

                        egui::Sides::new().show(
                            ui,
                            |ui| {
                                // Really don't care about micro and nano-seconds
                                let seconds = entry.duration.load().as_ref().as_secs();

                                if seconds > 0 {
                                    ui.label(format!(
                                        "{} ({})",
                                        &entry.workflow,
                                        humantime::Duration::from(Duration::from_secs(seconds))
                                    ));
                                } else {
                                    ui.label(&entry.workflow);
                                }
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

                        ui.small(dt.to_string());

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
                                        if ui.button(FLOPPY_DISK).on_hover_text("Save").clicked() {
                                            let name = k.clone();
                                            let value = v.clone();
                                            let errors = errors.clone();
                                            spawn_wherever(async move {
                                                if let Some(fh) = rfd::AsyncFileDialog::new()
                                                    // .set_directory(&self.state.output_dir)
                                                    .set_file_name(name)
                                                    .save_file()
                                                    .await
                                                {
                                                    let mut bytes = Vec::with_capacity(256);
                                                    errors.distil(write_value(&mut bytes, &value));
                                                    fh.write(&bytes).await.unwrap();
                                                }
                                            });
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

                if scroll_bottom {
                    ui.scroll_to_cursor(Some(egui::Align::BOTTOM));
                }
            });
        });

        if let Some(idx) = trash_idx {
            self.workflows.outputs.remove(idx);
        }
    }
}
