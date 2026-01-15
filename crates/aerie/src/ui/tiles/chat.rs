use eframe::egui;
use egui_commonmark::*;
use egui_phosphor::regular::GIT_BRANCH;
use itertools::Itertools;
use rig::message::{Message, UserContent};
use std::{borrow::Cow, sync::atomic::Ordering};

use crate::{
    ChatContent,
    config::ConfigExt,
    ui::{AppEvent, agent_bubble, error_bubble, shortcuts::squelch, user_bubble},
    utils::{ErrorDistiller as _, FormatOpts},
};

// Too many refs to self for a free function. Need to clean this up
impl super::AppState {
    pub fn chat_ui(&mut self, ui: &mut egui::Ui) {
        let settings = self.settings.clone();
        let errors = self.errors.clone();
        let workflows = self.workflows.names().map(|s| s.to_string()).collect_vec();

        // TODO: top panel with helper actions
        egui::TopBottomPanel::bottom("prompt")
            .resizable(true)
            .min_height(ui.available_height() / 4.0)
            .show_inside(ui, |ui| {
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

                            settings.update(|settings_rw| {
                                egui::ComboBox::from_label("Workflow")
                                    .selected_text(
                                        settings_rw.automation.as_ref().unwrap_or(&String::new()),
                                    )
                                    .show_ui(ui, |ui| {
                                        ui.selectable_value(&mut settings_rw.automation, None, "");

                                        for flow in &workflows {
                                            if !flow.starts_with("__") {
                                                ui.selectable_value(
                                                    &mut settings_rw.automation,
                                                    Some(flow.clone()),
                                                    flow,
                                                )
                                                .on_hover_text(self.workflows.description(flow));
                                            }
                                        }
                                    });
                            });
                        });
                    });

                egui::CentralPanel::default().show_inside(ui, |ui| {
                    let mut prompt_w = Cow::Borrowed(self.prompt.as_str());
                    let widget = egui::TextEdit::multiline(&mut prompt_w)
                        .desired_width(f32::INFINITY)
                        .hint_text("Type your message here \u{1F64B}");

                    squelch(ui.add_sized(ui.available_size(), widget));

                    if let Cow::Owned(prompt) = prompt_w {
                        self.prompt = prompt;
                    }
                });

                if submitted {
                    let automation = self
                        .settings
                        .view(|s| s.automation.clone())
                        .unwrap_or_default();

                    if automation.is_empty() || workflows.contains(&automation) {
                        // TODO: deal with this nuking any edits in progress
                        self.workflows.switch(&automation);
                        self.events.insert(AppEvent::UserRunWorkflow);
                        self.events.insert(AppEvent::SetPrompt(String::new()));
                    } else {
                        errors.push(anyhow::anyhow!("Workflow {automation} does not exist."));
                    }
                }
            });

        egui::CentralPanel::default().show_inside(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.set_width(ui.available_width());

                let scroll_bottom = self.task_count.load(Ordering::Relaxed) > 0
                    && (self.settings.view(|s| s.autoscroll)
                        || ui.button("Scroll to bottom.").clicked());

                let md_cache = &mut self.cache;
                self.session.view(|history| {
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
                                    if let Message::User { .. } = message
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

                let chat_r = self.session.scratch.load();
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
                self.session
                    .view(|session_r| !session_r.has_branch(&self.new_branch))
            };

            // Copy prompt from branch point into chat input
            self.session.view(|hist| {
                if self.prompt.is_empty()
                    && let Some(entry) = hist.store.get(&branch_point)
                    && let ChatContent::Message(msg) = &entry.content
                    && let Message::User { content } = msg
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
                    errors.distil(self.session.transform(|history| {
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
}

pub fn render_message(ui: &mut egui::Ui, cache: &mut CommonMarkCache, message: &Message) {
    render_message_width(ui, cache, message, None);
}

pub fn render_message_width(
    ui: &mut egui::Ui,
    cache: &mut CommonMarkCache,
    message: &Message,
    width: Option<f32>,
) {
    use crate::utils::MessageExt as _;
    use base64::prelude::*;

    match message {
        Message::User { .. } => {
            user_bubble(ui, |ui| {
                ui.set_width(width.unwrap_or(ui.available_width() * 0.75));

                ui.vertical(|ui| {
                    for (idx, (text, fmt)) in Itertools::intersperse(
                        message.text_fmt_opts().into_iter(),
                        (String::default(), FormatOpts::Separator),
                    )
                    .enumerate()
                    {
                        match fmt {
                            FormatOpts::Plain => {
                                ui.label(&text);
                            }
                            FormatOpts::Pre => {
                                egui::ScrollArea::horizontal().id_salt(idx).show(ui, |ui| {
                                    ui.with_layout(
                                        egui::Layout::left_to_right(egui::Align::TOP),
                                        |ui| {
                                            let language = "json";
                                            let theme =
                                        egui_extras::syntax_highlighting::CodeTheme::from_memory(
                                            ui.ctx(),
                                            ui.style(),
                                        );

                                            egui_extras::syntax_highlighting::code_view_ui(
                                                ui, &theme, &text, language,
                                            );
                                        },
                                    );
                                });
                            }
                            FormatOpts::Markdown => {
                                CommonMarkViewer::new().show(ui, cache, &text);
                            }
                            FormatOpts::Unknown => {
                                ui.label(&text);
                            }
                            FormatOpts::Separator => {
                                ui.separator();
                            }
                        }
                    }
                });
            });
        }
        Message::Assistant { .. } => {
            use regex::Regex;

            let re = Regex::new(r"(?ms)```mermaid(.*)```").unwrap();

            let mut all_text = String::new();

            agent_bubble(ui, |ui| {
                ui.set_width(width.unwrap_or(ui.available_width() * 0.75));

                ui.vertical(|ui| {
                    for (idx, (text, fmt)) in Itertools::intersperse(
                        message.text_fmt_opts().into_iter(),
                        (String::default(), FormatOpts::Separator),
                    )
                    .enumerate()
                    {
                        match fmt {
                            FormatOpts::Plain => {
                                ui.label(&text);
                                all_text.push_str(&text);
                            }
                            FormatOpts::Pre => {
                                all_text.push_str(&text);

                                egui::ScrollArea::horizontal().id_salt(idx).show(ui, |ui| {
                                    let language = "json";
                                    let theme =
                                        egui_extras::syntax_highlighting::CodeTheme::from_memory(
                                            ui.ctx(),
                                            ui.style(),
                                        );

                                    egui_extras::syntax_highlighting::code_view_ui(
                                        ui, &theme, &text, language,
                                    );
                                });
                            }
                            FormatOpts::Markdown => {
                                all_text.push_str(&text);
                                CommonMarkViewer::new().show(ui, cache, &text);
                            }
                            FormatOpts::Unknown => {
                                all_text.push_str(&text);
                                ui.label(&text);
                            }
                            FormatOpts::Separator => {
                                ui.separator();
                            }
                        }
                    }
                });
            });

            for (_, [diagram]) in re.captures_iter(&all_text).map(|m| m.extract()) {
                ui.scope_builder(egui::UiBuilder::new().id_salt(diagram), |ui| {
                    let enc = BASE64_URL_SAFE_NO_PAD.encode(diagram);

                    // Would prefer to use SVGs but egui implementation is a bit buggy
                    let url = format!(
                        "https://mermaid.ink/img/{enc}?type=png&theme=forest&bgColor=888888&width={}",
                        ui.available_width() as i32
                    );
                    let img = egui::Image::new(&url)
                        .corner_radius(10)
                        .fit_to_original_size(1.0)
                        .bg_fill(egui::Color32::GRAY);
                    let resp = ui
                        .add(img)
                        .on_hover_text_at_pointer(&url)
                        .interact(egui::Sense::click());

                    let payload = serde_json::to_string(&serde_json::json!({
                        "code": diagram.to_string(),
                        "mermaid": {"theme": "default"},
                        "autoSync": true,
                        "updateDiagram": false,
                        "editorMode": "code",
                    }))
                    .unwrap_or_default();
                    // ui.label(&payload);

                    // Can't figure out the right compression settings for pako
                    let enc = BASE64_URL_SAFE_NO_PAD.encode(payload);
                    let edit_url = format!("https://mermaid.live/edit#base64:{enc}");
                    // ui.label(&enc);

                    resp.context_menu(|ui| {
                        if ui.button("Open").clicked() {
                            ui.ctx().open_url(egui::OpenUrl {
                                url: url.clone(),
                                new_tab: true,
                            });
                        }

                        if ui.button("Edit").clicked() {
                            ui.ctx().open_url(egui::OpenUrl {
                                url: edit_url,
                                new_tab: true,
                            });
                        }
                    });
                });
            }
        }
    }
}
