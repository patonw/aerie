use eframe::egui;
use itertools::Itertools;
use std::collections::BTreeSet;

use crate::{Pipeline, Workstep, ui::toggled_field};

impl super::AppState {
    pub fn pipeline_ui(&mut self, ui: &mut egui::Ui) {
        egui::CentralPanel::default().show_inside(ui, |ui| {
            let mut settings_rw = self.settings.write().unwrap();

            if ui.button("+ New").clicked() {
                let existing = settings_rw
                    .pipelines
                    .iter()
                    .map(|it| it.name.clone())
                    .collect::<BTreeSet<_>>();

                let mut counter = 0;
                let name = loop {
                    let name = format!("New pipeline {counter:04}");
                    if !existing.contains(&name) {
                        break name;
                    }
                    counter += 1;
                };

                settings_rw.automation = Some(name.clone());
                settings_rw.pipelines.push(Pipeline {
                    name,
                    ..Default::default()
                });
            }

            let toolsets = settings_rw.tools.toolset.keys().cloned().collect_vec();

            ui.add_enabled_ui(settings_rw.automation.is_some(), |ui| {
                let pipeline_name = settings_rw.automation.to_owned().unwrap_or_default();
                if let Some(pipeline) = settings_rw
                    .pipelines
                    .iter_mut()
                    .find(|it| it.name == pipeline_name)
                {
                    let mut name_changed = false;
                    let mut checked = pipeline.preamble.is_some();
                    let mut value = pipeline.preamble.to_owned().unwrap_or_default();

                    egui::Grid::new("pipeline settings")
                        .num_columns(2)
                        .striped(true)
                        .show(ui, |ui| {
                            ui.label("Name").on_hover_text("Name of the pipeline");
                            name_changed =
                                ui.text_edit_singleline(&mut pipeline.name).changed();
                            ui.end_row();

                            ui.label("Preamble").on_hover_text("Optionally, override the system preamble.\nIf enabled and empty, then no preamble is used.");
                            ui.checkbox(&mut checked, "Override");
                            ui.end_row();
                        });

                    if checked {
                        ui.add(
                            egui::TextEdit::multiline(&mut value)
                                .hint_text("pipeline specific preamble"),
                        );
                    }
                    // ui.add_visible(checked, egui::TextEdit::multiline(&mut value));

                    pipeline.preamble = if checked { Some(value) } else { None };

                    ui.separator();
                    ui.heading("Steps");
                    for (i, step) in pipeline.steps.iter_mut().enumerate() {
                        egui::Frame::new()
                            .stroke(egui::Stroke::new(1.0, egui::Color32::GRAY))
                            .corner_radius(4)
                            .outer_margin(4)
                            .inner_margin(8)
                            .show(ui, |ui| {
                                let id = ui.id().with(i);
                                egui::Grid::new(id).num_columns(2).striped(true).show(
                                    ui,
                                    |ui| {
                                        ui.label("Skip").on_hover_text("Disable this step and advance to the next");
                                        ui.checkbox(&mut step.disabled, "");
                                        ui.end_row();

                                        toggled_field(
                                            ui,
                                            "Temperature",
                                            None::<String>,
                                            &mut step.temperature,
                                            |ui, value| {
                                                ui.add(egui::Slider::new(value, 0.0..=1.0));
                                            },
                                        );
                                        ui.end_row();

                                        toggled_field(
                                            ui,
                                            "Depth",
                                            None::<String>,
                                            &mut step.depth,
                                            |ui, value| {
                                                ui.add(egui::Slider::new(value, 0..=100));
                                            },
                                        );
                                        ui.end_row();

                                        toggled_field(
                                            ui,
                                            "Preamble",
                                            None::<String>,
                                            &mut step.preamble,
                                            |ui, value| {
                                                ui.add(
                                                    egui::TextEdit::multiline(value)
                                                        .hint_text("Step specific preamble"),
                                                );
                                            },
                                        );
                                        ui.end_row();

                                        ui.label("Prompt");
                                        ui.text_edit_multiline(&mut step.prompt);
                                        ui.end_row();

                                        toggled_field(
                                            ui,
                                            "Tools",
                                            None::<String>,
                                            &mut step.tools,
                                            |ui, value| {

                                                egui::ComboBox::from_id_salt("Tools")
                                                    .selected_text(value.as_str()).show_ui(ui, |ui| {
                                                        for name in  &toolsets {
                                                            ui.selectable_value(value, name.clone(), name);
                                                        }
                                                    });

                                            },
                                        );
                                    },
                                );
                            });
                    }
                    if ui.button("+ New step").clicked() {
                        pipeline.steps.push(Workstep::default());
                    }

                    if name_changed {
                        settings_rw.automation = Some(pipeline.name.clone());
                    }
                }
            });
        });
    }
}
