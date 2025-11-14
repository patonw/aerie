use std::path::PathBuf;

use eframe::egui;
use egui_phosphor::regular::FILE_PLUS;
use itertools::Itertools;

use crate::{
    Settings, ToolProvider, ToolSpec, Toolset,
    config::ConfigExt as _,
    ui::{behavior::ToolEditorState, toggled_field},
};

impl super::AppBehavior {
    pub fn toolset_ui(&mut self, ui: &mut egui::Ui) {
        let language = "json";
        let theme = egui_extras::syntax_highlighting::CodeTheme::from_memory(ui.ctx(), ui.style());

        if self.tool_editor.is_some() {
            egui::TopBottomPanel::bottom("editor")
                .resizable(true)
                .default_height(ui.available_height() / 3.0)
                .height_range((ui.available_height() / 4.0)..=(3.0 * ui.available_height() / 4.0))
                .show_inside(ui, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| match &self.tool_editor {
                        Some(ToolEditorState::ViewTool { tool }) => {
                            let content =
                                serde_json::to_string_pretty(tool).unwrap_or("???".to_string());

                            egui_extras::syntax_highlighting::code_view_ui(
                                ui, &theme, &content, language,
                            );
                        }
                        Some(ToolEditorState::EditProvider { .. }) => {
                            self.tool_provider_form(ui);
                            self.tool_provider_actions(ui);
                        }
                        None => {}
                    });
                });
        }

        egui::CentralPanel::default().show_inside(ui, |ui| {
            let toolsets = self
                .settings
                .view(|settings| settings.tools.toolset.keys().cloned().collect_vec());

            ui.horizontal(|ui| {
                ui.heading("Toolsets");

                if ui
                    .button(FILE_PLUS)
                    .on_hover_text("Create toolset")
                    .clicked()
                {
                    self.create_toolset = Some(String::new());
                }
            });

            ui.horizontal_wrapped(|ui| {
                for name in &toolsets {
                    ui.selectable_value(&mut self.edit_toolset, name.clone(), name);
                }
            });
            // egui::ComboBox::from_label("Toolset")
            //     .selected_text(&self.edit_toolset)
            //     .show_ui(ui, |ui| {
            //     });

            ui.separator();

            ui.horizontal(|ui| {
                ui.heading("Providers");
                if ui
                    .button(FILE_PLUS)
                    .on_hover_text("Create provider")
                    .clicked()
                {
                    self.tool_editor = Some(ToolEditorState::EditProvider {
                        original: None,
                        modified: (
                            String::new(),
                            ToolSpec::MCP {
                                enabled: false,
                                preface: None,
                                dir: None,
                                command: String::new(),
                                args: Vec::new(),
                            },
                        ),
                    });
                }
            });
            self.tool_tree(ui);
        });

        let settings = self.settings.clone();
        settings.update(|settings| self.toolset_modal(settings, ui));
    }

    fn tool_tree(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().show(ui, |ui| {
            let names_with_status = self.settings.view(|settings|
                settings.tools.provider.iter()
                    .map(|(a,b)| (a.clone(), b.enabled()))
                    .collect_vec());

            for (name, enabled) in &names_with_status {
                egui::collapsing_header::CollapsingState::load_with_default_open(
                    ui.ctx(),
                    ui.id().with(name),
                    true,
                )
                .show_header(ui, |ui| {
                        let selected = matches!(
                            &self.tool_editor,
                            Some(ToolEditorState::EditProvider { original: Some(original), .. }) if &original.0 == name
                        );

                        let name_text = egui::RichText::new(name);
                        let name_text = if *enabled {name_text} else {name_text.weak()};
                        if ui.selectable_label(selected, name_text).clicked() {
                            let tool_spec = self
                                .settings
                                .view(|settings_rw| settings_rw.tools.provider.get(name).cloned()).unwrap();

                            self.tool_editor = Some(ToolEditorState::EditProvider { original: Some((name.to_string(), tool_spec.clone())), modified: (name.to_string(), tool_spec.clone()) })
                        }
                    })
                .body(|ui| {
                        // TODO: Show any errors connecting to provider
                        if let Some(provider) = self.agent_factory.toolbox.providers.get(name) {

                            let ToolProvider::MCP { tools, .. } = provider;
                            for item in tools {
                                ui.horizontal(|ui| {
                                    self.settings.update(|settings_rw| {
                                        let mut editee = if self.edit_toolset == "all" {
                                            None
                                        } else {
                                            settings_rw.tools.toolset.get_mut(&self.edit_toolset)
                                        };
                                        let mut active = editee
                                            .as_ref()
                                            .map(|it| it.apply(name, item))
                                            .unwrap_or_else(|| self.edit_toolset == "all");

                                        if ui.checkbox(&mut active, "").clicked()
                                        && let Some(toolset) = editee.as_mut()
                                        {
                                            toolset.toggle(name, item);
                                        }
                                    });

                                    let selected = matches!(
                                        &self.tool_editor,
                                        Some(ToolEditorState::ViewTool { tool }) if tool == item
                                    );

                                    if ui.selectable_label(selected, &*item.name).clicked() {
                                        // toolset.toggle(name, tool);
                                        self.tool_editor =
                                            Some(ToolEditorState::ViewTool { tool: item.clone() })
                                    }
                                });
                            }
                        }
                });
            }
        });
    }

    fn toolset_modal(&mut self, settings_rw: &mut Settings, ui: &mut egui::Ui) {
        if let Some(new_name) = self.create_toolset.as_mut() {
            let mut submit = false;
            let unique_name =
                !new_name.is_empty() && { !settings_rw.tools.toolset.contains_key(new_name) };

            // TODO: refactor to dedup branch dialog
            let modal =
                egui::Modal::new(egui::Id::new("New toolset dialog")).show(ui.ctx(), |ui| {
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
                });
            if modal.should_close() {
                self.create_toolset = None;
            }
        }
    }

    fn tool_provider_form(&mut self, ui: &mut egui::Ui) {
        {
            let Some(ToolEditorState::EditProvider { modified, .. }) = &mut self.tool_editor else {
                unreachable!()
            };

            match &mut modified.1 {
                ToolSpec::MCP {
                    enabled,
                    preface,
                    dir,
                    command,
                    args,
                } => {
                    ui.heading(&modified.0);
                    egui::Grid::new("ToolProvider Editor")
                        .num_columns(2)
                        .striped(true)
                        .show(ui, |ui| {
                            ui.label("Enabled");
                            ui.checkbox(enabled, "");
                            ui.end_row();

                            ui.label("Name");
                            ui.label(&modified.0);
                            ui.end_row();

                            toggled_field(ui, "Preface", None::<String>, preface, |ui, value| {
                                ui.text_edit_singleline(value);
                            });
                            ui.end_row();

                            toggled_field(ui, "Working Dir", None::<String>, dir, |ui, value| {
                                let mut strval = value.to_str().unwrap_or_default().to_string();
                                ui.text_edit_singleline(&mut strval);
                                *value = PathBuf::from(strval);
                            });
                            ui.end_row();

                            ui.label("Command");
                            ui.text_edit_singleline(command);
                            ui.end_row();

                            // TODO: cleaner way to do this?
                            let mut lines = args.join("\n");
                            ui.label("Arguments");
                            ui.text_edit_multiline(&mut lines)
                                .on_hover_text("Arguments to the command separated by new lines.");
                            *args = lines.lines().map(|s| s.to_string()).collect_vec();
                            ui.end_row();
                        });
                }
            }
        }
    }

    fn tool_provider_actions(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui.button("Save").clicked() {
                // TODO: implement renaming
                let state = std::mem::take(&mut self.tool_editor);
                let Some(ToolEditorState::EditProvider {
                    modified: (name, value),
                    ..
                }) = state
                else {
                    unreachable!()
                };

                self.settings.update(|settings| {
                    settings.tools.provider.insert(name, value);
                });

                // TODO: Spawn thread
                let _ = self.agent_factory.reload_tools();
            }
            if ui.button("Cancel").clicked() {
                self.tool_editor = None;
            }
        });
    }
}
