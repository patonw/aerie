use egui::{RichText, TextEdit};
use egui_phosphor::regular::CLOCK_COUNTER_CLOCKWISE;
use itertools::Itertools;

use crate::config::ConfigExt as _;

impl super::AppState {
    pub fn settings_ui(&mut self, ui: &mut egui::Ui) {
        let settings = self.settings.clone();
        let workflows = self.workflows.names().cloned().collect_vec();

        egui::CentralPanel::default().show_inside(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                settings.update(|settings| {
                    ui.horizontal(|ui| {
                        if !settings.prev_models.is_empty() {
                            // Attempts to force UI to recompute max-width when longer model name.
                            // Would like some kind of generation ID instead of len.
                            ui.push_id(settings.prev_models.len(), |ui| {
                                ui.menu_button(CLOCK_COUNTER_CLOCKWISE, |ui| {
                                    let mut selected = None;
                                    for (i, name) in settings.prev_models.iter().enumerate() {
                                        if ui.button(name).clicked() {
                                            selected = Some(i);
                                        }
                                    }

                                    if let Some(i) = selected {
                                        settings.llm_model = settings.prev_models.remove(i);
                                        settings.prev_models.push_front(settings.llm_model.clone());
                                    }

                                    if ui
                                        .add_sized(
                                            egui::vec2(ui.min_size().x, 0.0),
                                            egui::Button::new(RichText::new("clear").weak())
                                                .small(),
                                        )
                                        .clicked()
                                    {
                                        settings.prev_models.clear();
                                    }
                                });
                            });
                        }

                        if ui
                            .add(
                                TextEdit::singleline(&mut settings.llm_model)
                                    .hint_text("provider/model:tag"),
                            )
                            .lost_focus()
                            && !settings.llm_model.is_empty()
                        {
                            settings.prev_models.retain(|m| m != &settings.llm_model);
                            settings.prev_models.push_front(settings.llm_model.clone());
                            settings.prev_models = settings
                                .prev_models
                                .take(16.min(settings.prev_models.len()));
                        }
                    });
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
                                ui.toggle_value(&mut settings_rw.autosave, "autosave");
                                ui.toggle_value(&mut settings_rw.autoscroll, "autoscroll");
                                ui.toggle_value(&mut settings_rw.streaming, "streaming");
                            });
                        });
                });

                egui::collapsing_header::CollapsingState::load_with_default_open(
                    ui.ctx(),
                    ui.make_persistent_id("workflow_info"),
                    true,
                )
                .show_header(ui, |ui| {
                    settings.update(|settings_rw| {
                        egui::ComboBox::from_label("Workflow")
                            .selected_text(
                                settings_rw.automation.as_ref().unwrap_or(&String::new()),
                            )
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut settings_rw.automation, None, "");

                                for flow in &workflows {
                                    ui.selectable_value(
                                        &mut settings_rw.automation,
                                        Some(flow.clone()),
                                        flow,
                                    );
                                }
                            });
                    });
                })
                .body(|ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        if let Some(shadow) = settings.view(|st| {
                            st.automation
                                .as_ref()
                                .and_then(|a| self.workflows.store.get(a))
                        }) {
                            ui.label(shadow.description.as_str());
                        }
                    });
                });
            });
        });
    }
}
