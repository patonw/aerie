use eframe::egui;
use itertools::Itertools;

use crate::{ToolProvider, Toolset};

impl super::AppBehavior {
    pub fn toolset_ui(&mut self, ui: &mut egui::Ui) {
        egui::CentralPanel::default().show_inside(ui, |ui| {
            let mut settings_rw = self.settings.write().unwrap();
            let toolsets = settings_rw.tools.toolset.keys().cloned().collect_vec();

            ui.horizontal(|ui| {
                egui::ComboBox::from_label("Toolset")
                    .selected_text(&self.edit_toolset)
                    .show_ui(ui, |ui| {
                        for name in &toolsets {
                            ui.selectable_value(&mut self.edit_toolset, name.clone(), name);
                        }
                    });

                ui.add_space(32.0);
                if ui.button("+ New").clicked() {
                    self.create_toolset = Some(String::new());
                }
            });

            ui.separator();

            egui::ScrollArea::vertical().show(ui, |ui| {
                let mut editee = if self.edit_toolset == "all" {
                    None
                } else {
                    settings_rw.tools.toolset.get_mut(&self.edit_toolset)
                };

                let providers = &self.agent_factory.toolbox.providers;
                for (name, provider) in providers.iter() {
                    egui::CollapsingHeader::new(name)
                        .default_open(true)
                        .show(ui, |ui| {
                            let ToolProvider::MCP { tools, .. } = provider;
                            let language = "json";
                            let theme = egui_extras::syntax_highlighting::CodeTheme::from_memory(
                                ui.ctx(),
                                ui.style(),
                            );

                            for tool in tools {
                                let id = ui.make_persistent_id(format!("{name}/{}", tool.name));
                                let selected = editee
                                    .as_ref()
                                    .map(|it| it.apply(name, tool))
                                    .unwrap_or_else(|| self.edit_toolset == "all");

                                egui::collapsing_header::CollapsingState::load_with_default_open(
                                    ui.ctx(),
                                    id,
                                    false,
                                )
                                .show_header(ui, |ui| {
                                    if ui.selectable_label(selected, &*tool.name).clicked()
                                        && let Some(toolset) = editee.as_mut()
                                    {
                                        toolset.toggle(name, tool);
                                    }
                                })
                                .body(|ui| {
                                    let content = serde_json::to_string_pretty(tool)
                                        .unwrap_or("???".to_string());
                                    egui_extras::syntax_highlighting::code_view_ui(
                                        ui, &theme, &content, language,
                                    );
                                });
                            }
                        });
                }

                if let Some(new_name) = self.create_toolset.as_mut() {
                    let mut submit = false;
                    let unique_name = !new_name.is_empty() && {
                        !settings_rw.tools.toolset.contains_key(new_name)
                    };

                    // TODO: refactor to dedup branch dialog
                    let modal = egui::Modal::new(egui::Id::new("New toolset dialog")).show(
                        ui.ctx(),
                        |ui| {
                            ui.set_width(250.0);

                            ui.heading("Create Toolset");

                            ui.label("Name:");
                            ui.text_edit_singleline(new_name).request_focus();

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
                                        // You can call `ui.close()` to close the modal.
                                        // (This causes the current modals `should_close` to return true)
                                        ui.close();
                                    }
                                },
                            );

                            submit |= unique_name && ui.input(|i| i.key_pressed(egui::Key::Enter));

                            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                                ui.close();
                            }

                            if submit {
                                self.edit_toolset = new_name.clone();
                                settings_rw
                                    .tools
                                    .toolset
                                    .insert(new_name.to_owned(), Toolset::empty());
                                ui.close();
                            }
                        },
                    );
                    if modal.should_close() {
                        self.create_toolset = None;
                    }
                }
            });
        });
    }
}
