use anyhow::Context as _;
use eframe::egui;
use egui_phosphor::regular::DOWNLOAD_SIMPLE;
use itertools::Itertools;
use serde_yaml_ng as serde_yml;
use std::path::PathBuf;

use crate::{
    config::ToolSpec,
    ui::{AppEvent, state::ToolEditorState, toggled_field},
    utils::{ErrorDistiller as _, spawn_wherever},
};

// #[cfg(feature = "native")]
// use crate::storage::CachedDirStore as _;

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
                                enabled: true,
                                preface: None,
                                dir: None,
                                env: Default::default(),
                                command: String::new(),
                                args: Vec::new(),
                                timeout: Some(30),
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
                                timeout: None,
                            },
                        ),
                    });
                }

                if ui.button(DOWNLOAD_SIMPLE).on_hover_text("Import").clicked() {
                    let export_dir = self.state.export_dir.clone();
                    let events = self.events.clone();
                    spawn_wherever(async move {
                        if let Some(fh) = rfd::AsyncFileDialog::new()
                            .set_directory(&export_dir)
                            .add_filter("mcp", &["mcp"])
                            .add_filter("all", &[""])
                            .pick_file()
                            .await
                        {
                            let name = fh.file_name();
                            let bytes = fh.read().await;
                            events.insert(AppEvent::ImportTools(name.to_string(), bytes));
                        }
                    });
                }
            });
            self.tool_tree(ui);
        });
    }

    fn tool_tree(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {

            for (name, tools) in &self.tools.tool_info {
                let Some(spec) = self.tools.get(name) else { continue };
                let enabled = spec.enabled();

                egui::collapsing_header::CollapsingState::load_with_default_open(
                    ui.ctx(),
                    ui.id().with(name),
                    true,
                )
                .show_header(ui, |ui| {
                        let selected = matches!(
                            &self.tool_editor,
                            Some(ToolEditorState::EditProvider { original: Some(original), .. }) if original.0 == *name
                        );

                        let name_text = egui::RichText::new(name);
                        let name_text = if enabled {name_text} else {name_text.weak()};
                        let resp = ui.selectable_label(selected, name_text);
                        resp.context_menu(|ui| {
                            if ui.button(if enabled { "Disable" } else { "Enable"}).clicked() {
                                let mut tool_spec = self.tools.get(name).unwrap().into_owned();
                                tool_spec.set_enabled(!enabled);

                                self.events.insert(AppEvent::SaveToolProvider(None, name.to_string(), tool_spec));
                            }

                            ui.menu_button("Delete", |ui| {
                                if ui.button("OK").clicked() {
                                self.events.insert(AppEvent::RemoveToolProvider(name.to_string()));
                                }
                            });

                            if ui.button("Export").clicked() {
                                let mut tool_spec = self.tools.get(name).unwrap().into_owned();
                                tool_spec.set_enabled(false);
                                let export_dir = self.state.export_dir.clone();
                                let name = name.to_string();
                                let errors = self.errors.clone();

                                spawn_wherever(async move {
                                    if let Some(fh) = rfd::AsyncFileDialog::new()
                                        .add_filter("mcp", &["mcp"])
                                        .add_filter("all", &[""])
                                        .set_directory( &export_dir)
                                        .set_file_name(format!(
                                            "{name}.mcp",
                                        ))
                                        .save_file().await {
                                        let mut bytes = Vec::new();

                                        errors.distil(serde_yml::to_writer(&mut bytes, &tool_spec).context(format!("While serializing tool provider {name}")));
                                        let _ = fh.write(&bytes).await;
                                    }
                                });

                            }
                        });
                        if resp.clicked() {
                            let tool_spec = self.tools.get(name).unwrap().into_owned();

                            self.tool_editor = Some(ToolEditorState::EditProvider { original: Some((name.to_string(), tool_spec.clone())), modified: (name.to_string(), tool_spec.clone()) })
                        }
                    })
                .body(|ui| {
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
                            timeout,
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
                            // lines.push_str("\n");
                            ui.label("Arguments");
                            ui.text_edit_multiline(&mut lines)
                                .on_hover_text("Arguments to the command separated by new lines.");
                            *args = lines.split("\n").map(|s| s.to_string()).collect_vec();
                            ui.end_row();

                            ui.label("timeout");
                            toggled_field(ui, "t",
                                "Abort run if the MCP server does not respond within this number of seconds.\n\
                                    Note: this will only work via the Chat node if streaming is enabled.\n\
                                    Invoke Tools node will honor this, regardless.".into(),
                                timeout,
                                |ui, value| {

                                ui.add(egui::Slider::new(value, 0..=1000).text("T"))
                                    .on_hover_text("timeout seconds");
                            });
                            ui.end_row();
                        }
                        ToolSpec::HTTP {
                            enabled,
                            preface,
                            uri,
                            auth_var,
                            timeout,
                            ..
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

                            ui.label("timeout");
                            toggled_field(ui, "t",
                                "Abort run if the MCP server does not respond within this number of seconds".into(), timeout, |ui, value| {

                                ui.add(egui::Slider::new(value, 0..=1000).text("T"))
                                    .on_hover_text("timeout seconds");
                            });
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

                self.events.insert(AppEvent::SaveToolProvider(
                    original.map(|it| it.0),
                    name.to_string(),
                    value,
                ));
            }
            if ui.button("Cancel").clicked() {
                self.tool_editor = None;
            }
        });
    }
}
