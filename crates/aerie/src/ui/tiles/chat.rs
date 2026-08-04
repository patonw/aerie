use eframe::egui;
use egui_extras::{Size, StripBuilder};
use egui_phosphor::regular::{GIT_BRANCH, GLOBE, IMAGES, TRASH, X_CIRCLE};
use itertools::Itertools;
use std::{borrow::Cow, sync::atomic::Ordering};

use crate::{
    chat::ChatContent,
    config::ConfigExt,
    events::AppEvent,
    rig::message::{Message, UserContent},
    ui::{error_bubble, shortcuts::squelch},
    utils::ErrorDistiller as _,
};

pub use crate::ui::render::render_message;
pub use crate::ui::render::render_message_width;

// Too many refs to self for a free function. Need to clean this up
impl super::AppState {
    pub fn chat_ui(&mut self, ui: &mut egui::Ui) {
        let errors = self.errors.clone();
        let workflows = self.workflows.names().map(|s| s.to_string()).collect_vec();

        // TODO: top panel with helper actions
        egui::TopBottomPanel::bottom("prompt")
            .resizable(true)
            .min_height(ui.available_height() / 4.0)
            .show_inside(ui, |ui| {
                let height = ui.available_height();

                egui::SidePanel::left("media")
                    .default_width(32.0)
                    .show_inside(ui, |ui| {
                        egui::ScrollArea::vertical()
                            .max_height(height - 32.0)
                            .auto_shrink(false)
                            .show(ui, |ui| {
                                ui.set_max_height(
                                    3.0 * ui.available_width() * self.images.len() as f32,
                                );

                                let mut remove_idx = None;

                                for (i, path) in self.images.iter().enumerate() {
                                    let path = if let Ok(exists) = std::fs::exists(path)
                                        && exists
                                    {
                                        Cow::Owned(format!("file://{path}"))
                                    } else {
                                        Cow::Borrowed(path)
                                    };

                                    let resp = ui.image(&*path).on_hover_ui(|ui| {
                                        ui.add(
                                            egui::Image::new(&*path)
                                                .max_size(ui.ctx().content_rect().size() * 0.75),
                                        );
                                    });

                                    let button = egui::Button::new(X_CIRCLE)
                                        .small()
                                        .fill(egui::Color32::from_black_alpha(0));
                                    let rect = egui::Rect::from_center_size(
                                        resp.rect.left_top() + egui::vec2(8.0, 4.0),
                                        egui::vec2(8.0, 8.0),
                                    );
                                    let resp = ui.place(rect, button);

                                    if resp.interact(egui::Sense::click()).clicked() {
                                        remove_idx = Some(i);
                                    }
                                }

                                if let Some(idx) = remove_idx {
                                    self.images.remove(idx);
                                }
                            });
                        ui.vertical_centered_justified(|ui| {
                            StripBuilder::new(ui)
                                .size(Size::exact(16.0))
                                .vertical(|mut strip| {
                                    strip.cell(|ui| {
                                        StripBuilder::new(ui)
                                            .sizes(
                                                Size::remainder(),
                                                if self.images.is_empty() { 2 } else { 3 },
                                            )
                                            .horizontal(|mut strip| {
                                                if !self.images.is_empty() {
                                                    strip.cell(|ui| {
                                                        if ui
                                                            .button(TRASH)
                                                            .on_hover_text("Clear")
                                                            .clicked()
                                                        {
                                                            self.images.clear();
                                                        }
                                                    });
                                                }
                                                strip.cell(|ui| {
                                                    if ui
                                                        .button(GLOBE)
                                                        .on_hover_text("Link Images (URL)")
                                                        .clicked()
                                                    {
                                                        self.image_input = Some(Default::default());
                                                    }
                                                });
                                                #[cfg(feature = "wip")]
                                                strip.cell(|ui| {
                                                    if ui
                                                        .button(IMAGES)
                                                        .on_hover_text("Load Images (filesystem)")
                                                        .clicked()
                                                        && let Some(paths) = rfd::FileDialog::new()
                                                            .set_directory(&self.state.export_dir)
                                                            .add_filter(
                                                                "images",
                                                                &["png", "jpg", "jpeg", "webp"],
                                                            )
                                                            .add_filter("all", &[""])
                                                            .pick_files()
                                                    {
                                                        let paths =
                                                            paths.into_iter().filter_map(|p| {
                                                                p.to_str().map(|s| s.to_string())
                                                            });

                                                        self.images.extend(paths);
                                                    }
                                                });
                                            });
                                    });
                                });

                            self.image_input_dialog(ui);
                        });
                    });

                let mut submitted = false;
                egui::TopBottomPanel::bottom("actions")
                    .resizable(false)
                    .show_separator_line(false)
                    .show_inside(ui, |ui| {
                        submitted |= ui.input(|i| {
                            (i.modifiers.ctrl || i.modifiers.alt) && i.key_pressed(egui::Key::Enter)
                        });

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            submitted |= ui.button("Chat").clicked();

                            ui.add_space(16.0);

                            self.errors.distil(self.state.update(|state| {
                                egui::ComboBox::from_label("Workflow")
                                    .selected_text(&state.workflow)
                                    .show_ui(ui, |ui| {
                                        ui.selectable_value(&mut state.workflow, String::new(), "");

                                        for flow in &workflows {
                                            if !flow.starts_with("__") {
                                                ui.selectable_value(
                                                    &mut state.workflow,
                                                    flow.clone(),
                                                    flow,
                                                )
                                                .on_hover_text(self.workflows.description(flow));
                                            }
                                        }
                                    });
                            }));
                        });
                    });

                egui::CentralPanel::default().show_inside(ui, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        let mut prompt_w = Cow::Borrowed(self.prompt.as_str());
                        let widget = egui::TextEdit::multiline(&mut prompt_w)
                            .desired_width(f32::INFINITY)
                            .hint_text("Type your message here \u{1F64B}");

                        squelch(ui.add_sized(ui.available_size(), widget));

                        if let Cow::Owned(prompt) = prompt_w {
                            // TODO: more sensible delineation of responsibility
                            #[cfg(feature = "web-ui")]
                            self.events.insert(AppEvent::SetPrompt(prompt.clone()));

                            self.prompt = prompt;
                        }
                    });
                });

                if submitted {
                    let workflow = self.state.workflow.as_str();

                    if workflow.is_empty() || workflows.contains(&workflow.to_string()) {
                        // TODO: deal with this nuking any edits in progress
                        // TODO: implement as action dispatch
                        self.events
                            .insert(AppEvent::SwitchWorkflow(workflow.to_string()));
                        self.events.insert(AppEvent::UserRunWorkflow);
                        self.events.insert(AppEvent::SetPrompt(String::new()));
                    } else {
                        errors.push(anyhow::anyhow!("Workflow {workflow} does not exist."));
                    }
                }

                ui.input(|input| {
                    if !input.raw.dropped_files.is_empty() {
                        tracing::info!("Dropped files: {:?}", input.raw.dropped_files);
                        for drop in &input.raw.dropped_files {
                            if let Some(path) = &drop.path
                                && let Some(name) = path.to_str()
                            {
                                self.images.push(name.to_string());
                            }
                        }
                    }

                    if !input.raw.hovered_files.is_empty() {
                        tracing::info!("Hovered files: {:?}", input.raw.hovered_files);
                        // Using a painter here deadlocks the app.
                    }
                });
            });

        egui::CentralPanel::default().show_inside(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.set_width(ui.available_width());

                let scroll_bottom = self.task_count.load(Ordering::Relaxed) > 0
                    && (self.data.prefs.view(|s| s.autoscroll)
                        || ui.button("Scroll to bottom.").clicked());

                let md_cache = &mut self.cache;
                self.data.session.view(|history| {
                    for msg in history.iter() {
                        ui.push_id(msg.id, |ui| {
                            let aside = history.iter_aside(msg).collect_vec();
                            if !aside.is_empty() {
                                egui::CollapsingHeader::new("details").id_salt(msg.id).show(
                                    ui,
                                    |ui| {
                                        for entry in aside {
                                            if let ChatContent::Message(message) = &entry.content {
                                                ui.push_id(entry.id, |ui| {
                                                    render_message(ui, md_cache, message)
                                                });
                                            }
                                        }
                                    },
                                );
                            }

                            match &msg.content {
                                ChatContent::Message(message) => {
                                    // TODO: only on user prompt
                                    if let Message::User { .. } = message.as_ref()
                                        && ui.button(GIT_BRANCH).clicked()
                                    {
                                        self.branch_point = Some(msg.id);
                                    }
                                    ui.push_id(msg.id, |ui| {
                                        render_message(ui, md_cache, message);
                                    });
                                }
                                ChatContent::Aside {
                                    automation: workflow,
                                    prompt: _,
                                    collapsed,
                                    content,
                                } => {
                                    let resp = egui::CollapsingHeader::new(format!(
                                        "Workflow: {workflow}"
                                    ))
                                    .id_salt(msg.id)
                                    .default_open(!collapsed)
                                    .show(ui, |ui| {
                                        for (idx, message) in content.iter().enumerate() {
                                            ui.push_id(idx, |ui| {
                                                render_message(ui, md_cache, message)
                                            });
                                        }
                                    });
                                    if resp.fully_closed()
                                        && let Some(message) = content.last()
                                    {
                                        render_message(ui, md_cache, message);
                                    }
                                }
                                ChatContent::Error { err } => {
                                    error_bubble(ui, |ui| {
                                        ui.set_width(ui.available_width());
                                        ui.label(
                                            egui::RichText::new(err).color(egui::Color32::RED),
                                        );
                                    });
                                }
                            }
                        });
                    }
                });

                let chat_r = self.data.session.scratch.load();
                if !chat_r.is_empty() {
                    ui.separator();
                }

                for entry in chat_r.iter() {
                    let msg = entry.load();
                    match msg.as_ref() {
                        Ok(message) => render_message(ui, md_cache, message),
                        Err(err) => {
                            error_bubble(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.label(egui::RichText::new(err).color(egui::Color32::RED));
                            });
                        }
                    }
                }

                if self.task_count.load(Ordering::Relaxed) > 0 {
                    ui.spinner();
                }

                // Add an extra line to prevent clipping on long text
                // let font_id = egui::TextStyle::Body.resolve(ui.style());
                // ui.add_space(128.0);

                if scroll_bottom {
                    ui.scroll_to_cursor(Some(egui::Align::BOTTOM));
                }
            });
        });

        if let Some(branch_point) = self.branch_point {
            let mut submit = false;
            let unique_name = !self.new_branch.is_empty() && {
                self.data
                    .session
                    .view(|session_r| !session_r.has_branch(&self.new_branch))
            };

            // Copy prompt from branch point into chat input
            self.data.session.view(|hist| {
                if self.prompt.is_empty()
                    && let Some(entry) = hist.store.get(&branch_point)
                    && let ChatContent::Message(msg) = &entry.content
                    && let Message::User { content } = msg.as_ref()
                    && let UserContent::Text(text) = content.first()
                {
                    self.prompt = text.text().to_string();
                }
            });

            let modal = egui::Modal::new(egui::Id::new("Branch dialog")).show(ui.ctx(), |ui| {
                ui.set_width(250.0);

                ui.heading("Create Branch");

                ui.label("Name:");
                ui.text_edit_singleline(&mut self.new_branch)
                    .request_focus();

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
                    errors.distil(self.data.session.transform(|history| {
                        let parent = history.find_parent(branch_point);

                        let name = std::mem::take(&mut self.new_branch);
                        ui.close();
                        history.create_branch(&name, parent)
                    }));
                }
            });

            if modal.should_close() {
                self.branch_point = None;
            }
        }
    }

    // A whole lotta boilerplate for something so simple
    fn image_input_dialog(&mut self, ui: &mut egui::Ui) {
        if self.image_input.is_none() {
            return;
        }

        let mut submit = false;
        let modal = egui::Modal::new(egui::Id::new("Add image")).show(ui.ctx(), |ui| {
            let Some(input) = &mut self.image_input else {
                unreachable!()
            };

            ui.label("URL(s):");
            ui.text_edit_multiline(input).request_focus();
            ui.separator();

            egui::Sides::new().show(
                ui,
                |_ui| {},
                |ui| {
                    if ui.button("Ok").clicked() {
                        submit = true;
                    }
                    if ui.button("Cancel").clicked() {
                        ui.close();
                    }
                },
            );

            // submit |= ui.input(|i| {
            //     (i.modifiers.ctrl || i.modifiers.alt) && i.key_pressed(egui::Key::Enter)
            // });

            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                ui.close();
            }

            if submit {
                if let Some(new_items) = self.image_input.take() {
                    self.images.extend(new_items.lines().map(|s| s.to_string()));
                }
                ui.close();
            }
        });

        if modal.should_close() {
            self.image_input = None;
        }
    }
}
