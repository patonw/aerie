use std::path::PathBuf;

use eframe::egui;
use itertools::Itertools;

use crate::{
    ToolProvider, ToolSpec,
    config::ConfigExt as _,
    ui::{state::ToolEditorState, toggled_field},
};

impl super::AppState {
    pub fn toolset_ui(&mut self, ui: &mut egui::Ui) {
        let language = "json";
        let theme = egui_extras::syntax_highlighting::CodeTheme::from_memory(ui.ctx(), ui.style());

        if self.tool_editor.is_some() {
            egui::TopBottomPanel::bottom("editor")
                .resizable(true)
                .default_height(ui.available_height() / 2.0)
                .height_range((ui.available_height() / 4.0)..=(3.0 * ui.available_height() / 4.0))
                .show_inside(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .auto_shrink(false)
                        .show(ui, |ui| {
                            match &self.tool_editor {
                                Some(ToolEditorState::ViewTool { tool }) => {
                                    let content = serde_json::to_string_pretty(tool)
                                        .unwrap_or("???".to_string());

                                    egui_extras::syntax_highlighting::code_view_ui(
                                        ui, &theme, &content, language,
                                    );
                                }
                                Some(ToolEditorState::EditProvider { .. }) => {
                                    self.tool_provider_form(ui);
                                    self.tool_provider_actions(ui);
                                }
                                None => {}
                            }
                            // ui.allocate_space(ui.available_size());
                        });
                });
        }

        egui::CentralPanel::default().show_inside(ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Providers");

                if ui
                    .button("+stdio")
                    .on_hover_text("Create a MCP/stdio provider")
                    .clicked()
                {
                    self.tool_editor = Some(ToolEditorState::EditProvider {
                        original: None,
                        modified: (
                            String::new(),
                            ToolSpec::Stdio {
                                enabled: false,
                                preface: None,
                                dir: None,
                                env: Default::default(),
                                command: String::new(),
                                args: Vec::new(),
                            },
                        ),
                    });
                }
                if ui
                    .button("+http")
                    .on_hover_text("Create a MCP/HTTP provider")
                    .clicked()
                {
                    self.tool_editor = Some(ToolEditorState::EditProvider {
                        original: None,
                        modified: (
                            String::new(),
                            ToolSpec::HTTP {
                                enabled: false,
                                preface: None,
                                uri: String::from("http://localhost:8080"),
                                auth_var: None,
                            },
                        ),
                    });
                }
            });
            self.tool_tree(ui);
        });
    }

    fn tool_tree(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
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

    fn tool_provider_form(&mut self, ui: &mut egui::Ui) {
        {
            let Some(ToolEditorState::EditProvider { modified, .. }) = &mut self.tool_editor else {
                unreachable!()
            };

            ui.heading(&modified.0);
            egui::Grid::new("ToolProvider Editor")
                .num_columns(2)
                .striped(true)
                .show(ui, |ui| {
                    match &mut modified.1 {
                        ToolSpec::Stdio {
                            enabled,
                            preface,
                            dir,
                            env,
                            command,
                            args,
                        } => {
                            ui.label("Enabled");
                            ui.checkbox(enabled, "");
                            ui.end_row();

                            ui.label("Name");
                            ui.text_edit_singleline(&mut modified.0);
                            ui.end_row();

                            ui.label("Preface");
                            toggled_field(ui, "p", None::<String>, preface, |ui, value| {
                                ui.text_edit_singleline(value);
                            });
                            ui.end_row();

                            ui.label("Working dir");
                            toggled_field(ui, "d", None::<String>, dir, |ui, value| {
                                let mut strval = value.to_str().unwrap_or_default().to_string();
                                ui.text_edit_singleline(&mut strval);
                                *value = PathBuf::from(strval);
                            });
                            ui.end_row();

                            ui.label("Environment");
                            ui.text_edit_multiline(env)
                                .on_hover_text("Additional environment variables for this command.\n\
                                    Do not use this to set API keys and auth tokens.");
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
                        }
                        ToolSpec::HTTP {
                            enabled,
                            preface,
                            uri,
                            auth_var,
                        } => {
                            ui.label("Enabled");
                            ui.checkbox(enabled, "");
                            ui.end_row();

                            ui.label("Name");
                            ui.text_edit_singleline(&mut modified.0);
                            ui.end_row();

                            ui.label("Preface");
                            toggled_field(ui, "p", None::<String>, preface, |ui, value| {
                                ui.text_edit_singleline(value);
                            });
                            ui.end_row();

                            ui.label("URI");
                            ui.text_edit_singleline(uri);
                            ui.end_row();

                            ui.label("Auth Var");
                            toggled_field(
                                ui,
                                "a",
                                "Environment variable containing your bearer token (NOT the token itself)".into(),
                                auth_var,
                                |ui, value| {
                                    ui.text_edit_singleline(value);
                                },
                            );

                            ui.end_row();
                        }
                    }
                });
        }
    }

    fn tool_provider_actions(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui.button("Save").clicked() {
                let state = std::mem::take(&mut self.tool_editor);

                let Some(ToolEditorState::EditProvider {
                    original, modified, ..
                }) = state
                else {
                    unreachable!();
                };

                let (name, value) = modified;

                // Is there any way we can keep existing tool filters from breaking on renames?
                let old_name = if let Some((old_name, _)) = original {
                    Some(old_name)
                } else {
                    None
                };

                self.settings.update(|settings| {
                    if let Some(old_name) = old_name {
                        settings.tools.provider.remove(&old_name);
                    }

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
