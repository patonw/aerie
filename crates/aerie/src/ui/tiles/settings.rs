use egui::RichText;
use itertools::Itertools;

use crate::config::ConfigExt as _;

impl super::AppState {
    pub fn settings_ui(&mut self, ui: &mut egui::Ui) {
        let settings = self.settings.clone();
        let workflows = self.workflows.names().cloned().collect_vec();

        egui::CentralPanel::default().show_inside(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                egui::ComboBox::from_label("Model")
                    .selected_text(settings.view(|s| s.llm_model.to_string()))
                    .show_ui(ui, |ui| {
                        settings.update(|settings_rw| {
                            ui.selectable_value(
                                &mut settings_rw.llm_model,
                                "devstral:latest".to_string(),
                                "Devstral",
                            );
                            ui.selectable_value(
                                &mut settings_rw.llm_model,
                                "magistral:latest".to_string(),
                                "Magistral",
                            );
                            ui.selectable_value(
                                &mut settings_rw.llm_model,
                                "my-qwen3-coder:30b".to_string(),
                                "Qwen3 Coder",
                            );
                        });
                    });

                settings.update(|settings_rw| {
                    ui.add(
                        egui::TextEdit::multiline(&mut settings_rw.preamble)
                            .hint_text("Preamble")
                            .desired_width(f32::INFINITY),
                    );
                });

                settings.update(|settings_rw| {
                    ui.add(egui::Slider::new(&mut settings_rw.temperature, 0.0..=1.0).text("T"))
                        .on_hover_text("temperature");
                });

                settings.update(|settings_rw| {
                    egui::CollapsingHeader::new("Flags")
                        .default_open(true)
                        .show(ui, |ui| {
                            ui.horizontal_wrapped(|ui| {
                                // ui.spacing_mut().item_spacing.x = 0.0;
                                ui.toggle_value(&mut settings_rw.autoscroll, "autoscroll");
                                // ui.toggle_value(&mut settings_rw.show_logs, "logs");
                            });
                        });
                });

                settings.update(|settings_rw| {
                    egui::ComboBox::from_label("Automation")
                        .selected_text(settings_rw.automation.as_ref().unwrap_or(&String::new()))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut settings_rw.automation, None, "");
                            ui.label(RichText::new("Pipelines:").strong());
                            for flow in &settings_rw.pipelines {
                                ui.selectable_value(
                                    &mut settings_rw.automation,
                                    Some(flow.name.clone()),
                                    &flow.name,
                                );
                            }

                            ui.label(RichText::new("Workflows:").strong());
                            for flow in &workflows {
                                ui.selectable_value(
                                    &mut settings_rw.automation,
                                    Some(flow.clone()),
                                    flow,
                                );
                            }
                        });
                });
            });
        });
    }
}
