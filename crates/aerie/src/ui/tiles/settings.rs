use std::collections::BTreeSet;

use egui::{Button, RichText, TextEdit};
use egui_phosphor::regular::{CLOCK_COUNTER_CLOCKWISE, PENCIL, STACK_PLUS, TRASH};
use itertools::Itertools;
use strum::{EnumMessage, IntoEnumIterator as _};

use crate::{
    config::{ConfigExt as _, ModelRole, RoleEntry},
    utils::ErrorDistiller as _,
    workflow::store::WorkflowStore as _,
};

impl super::AppState {
    pub fn settings_ui(&mut self, ui: &mut egui::Ui) {
        let settings = self.prefs.clone();

        egui::CentralPanel::default().show_inside(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                settings.update(|settings| {
                    ui.vertical_centered(|ui| {
                        ui.label("models");
                    });

                    ui.horizontal(|ui| {
                        if ui
                            .button(STACK_PLUS)
                            .on_hover_text("Create Profile")
                            .clicked()
                        {
                            let datetime = chrono::offset::Local::now();
                            let timestamp =
                                datetime.format("profile-%Y-%m-%dT%H:%M:%S").to_string();
                            settings
                                .models
                                .insert(timestamp.clone(), Default::default());
                            settings.profile = timestamp;
                        }

                        ui.menu_button(TRASH, |ui| {
                            if ui.button("OK").clicked() {
                                settings.models.remove(&settings.profile);
                                settings.profile = settings.models.first_key().unwrap_or_default();
                            }
                        })
                        .response
                        .on_hover_text("Delete Profile");

                        if ui.button(PENCIL).on_hover_text("Rename Profile").clicked() {
                            self.rename_profile = Some(settings.profile.clone());
                        }

                        if settings.profile.is_empty() {
                            settings.profile = "default".into();
                        }

                        if let Some(renaming) = self.rename_profile.as_mut() {
                            let editor = ui.text_edit_singleline(renaming);

                            if editor.lost_focus() {
                                if !renaming.is_empty() {
                                    let entry = settings
                                        .models
                                        .remove(&settings.profile)
                                        .unwrap_or_default();

                                    settings.models.insert(renaming.clone(), entry);
                                    settings.profile = renaming.to_owned();
                                }
                                self.rename_profile = None;
                            }
                            editor.request_focus();
                        } else {
                            egui::ComboBox::from_label("Profile")
                                .wrap()
                                .selected_text(settings.profile.as_str())
                                .show_ui(ui, |ui| {
                                    for key in settings.models.keys() {
                                        ui.selectable_value(
                                            &mut settings.profile,
                                            key.clone(),
                                            key,
                                        );
                                    }
                                })
                                .response
                                .on_hover_text("Profile");
                        }
                    });

                    let entries = settings.models.get_or_create(settings.profile.clone());

                    let filled_roles: BTreeSet<ModelRole> =
                        entries.iter().flat_map(|e| e.roles.clone()).collect();

                    let mut remove_index = None;

                    for (i, RoleEntry { name, roles }) in entries.iter_mut().enumerate() {
                        egui::collapsing_header::CollapsingState::load_with_default_open(
                            ui.ctx(),
                            ui.id().with(&name),
                            true,
                        )
                        .show_header(ui, |ui| {
                            if ui.button(TRASH).clicked() {
                                remove_index = Some(i);
                            }
                            self.model_picker(ui, name);
                        })
                        .body(|ui| {
                            ui.horizontal_wrapped(|ui| {
                                for role in ModelRole::iter() {
                                    if matches!(role, ModelRole::Custom(_)) {
                                        continue;
                                    }

                                    let tooltip = role.get_message();
                                    let selected = roles.contains(&role);
                                    if !selected && filled_roles.contains(&role) {
                                        let response = ui.add_enabled(
                                            false,
                                            Button::selectable(false, role.to_string()),
                                        );
                                        if let Some(text) = tooltip {
                                            response.on_hover_text(text);
                                        }
                                    } else {
                                        let response =
                                            ui.selectable_label(selected, role.to_string());
                                        if response.clicked() {
                                            if selected {
                                                roles.remove(&role);
                                            } else {
                                                roles.insert(role);
                                            }
                                        }
                                        if let Some(text) = tooltip {
                                            response.on_hover_text(text);
                                        }
                                    }
                                }
                            });
                        });
                    }

                    if let Some(i) = remove_index {
                        entries.remove(i);
                    }

                    if ui.button("add model").clicked() {
                        entries.push_back(Default::default());
                    }
                });

                ui.separator();
                ui.vertical_centered(|ui| {
                    ui.label("settings");
                });

                egui::Grid::new("Settings Editor")
                    .num_columns(2)
                    .striped(true)
                    .show(ui, |ui| {
                        ui.label("temperature").on_hover_text(
                            "controls the amount of variation/creativity in LLM outputs",
                        );
                        settings.update(|settings_rw| {
                            ui.add(egui::Slider::new(&mut settings_rw.temperature, 0.0..=1.0));
                        });

                        ui.end_row();

                        ui.label("autorun").on_hover_text(
                            "Number of additional turns to execute chained workflows automatically",
                        );
                        settings.update(|settings_rw| {
                            let widget = egui::DragValue::new(&mut settings_rw.autoruns)
                                .update_while_editing(false);
                            ui.add(widget);
                        });
                        ui.end_row();
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
                                ui.toggle_value(&mut settings_rw.cascade, "cascade");
                            });
                        });
                });

                let workflows = self.workflows.names().map(|s| s.to_string()).collect_vec();
                egui::collapsing_header::CollapsingState::load_with_default_open(
                    ui.ctx(),
                    ui.make_persistent_id("workflow_info"),
                    true,
                )
                .show_header(ui, |ui| {
                    self.errors.distil(self.state.update(|state| {
                        egui::ComboBox::from_label("Workflow")
                            .selected_text(&state.workflow)
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut state.workflow, String::new(), "");

                                for flow in &workflows {
                                    ui.selectable_value(&mut state.workflow, flow.clone(), flow);
                                }
                            });
                    }));
                })
                .body(|ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        let name = self.state.workflow.as_str();
                        let desc = self.workflows.store.description(name);
                        ui.label(desc);
                    });
                });
            });
        });
    }

    fn model_picker(&mut self, ui: &mut egui::Ui, llm_model: &mut String) {
        ui.horizontal(|ui| {
            // Attempts to force UI to recompute max-width when longer model name.
            // Would like some kind of generation ID instead of len.
            ui.push_id(self.state.modtime(), |ui| {
                ui.menu_button(CLOCK_COUNTER_CLOCKWISE, |ui| {
                    let mut selected = None;
                    for (i, name) in self.state.prev_models.iter().enumerate() {
                        if ui.button(name).clicked() {
                            selected = Some(i);
                        }
                    }

                    if let Some(i) = selected {
                        let _ = self.state.update(|state| {
                            *llm_model = state.prev_models.remove(i);
                            state.prev_models.push_front(llm_model.clone());
                        });
                    }

                    if self.state.prev_models.is_empty() {
                        ui.label(RichText::new("(empty)").weak());
                    } else if ui
                        .add_sized(
                            egui::vec2(ui.min_size().x.max(128f32), 0.0),
                            egui::Button::new(RichText::new("clear").weak()).small(),
                        )
                        .clicked()
                    {
                        let _ = self.state.update(|state| {
                            state.prev_models.clear();
                        });
                    }
                });
            });

            if crate::ui::shortcuts::squelch(
                ui.add(TextEdit::singleline(llm_model).hint_text("provider/model:tag")),
            )
            .lost_focus()
                && !llm_model.is_empty()
            {
                let _ = self.state.update(|state| {
                    state.prev_models.retain(|m| m != llm_model);
                    state.prev_models.push_front(llm_model.clone());
                    state.prev_models = state.prev_models.take(16.min(state.prev_models.len()));
                });
            }
        });
    }
}
