use eframe::egui;
use egui_commonmark::*;
use egui_phosphor::regular::GIT_BRANCH;
use minijinja::{Environment, context};
use rig::{
    agent::PromptRequest,
    message::{AssistantContent, Message, UserContent},
};
use std::{borrow::Cow, collections::VecDeque, f32, sync::atomic::Ordering, time::Instant};
use tracing::Level;

// use super::{Pane, agent_bubble, error_bubble, tiles, user_bubble};
use crate::{ChatContent, LogEntry, utils::ErrorDistiller as _};

use crate::ui::{agent_bubble, error_bubble, user_bubble};
use crate::utils::CowExt as _;

// Too many refs to self for a free function. Need to clean this up
impl super::AppState {
    pub fn chat_ui(&mut self, ui: &mut egui::Ui) {
        let errors = self.errors.clone();

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

                            // Can branch from beginning to have an empty history
                            // TODO: support branch deletion instead
                            // if ui.button("Clear").clicked() {
                            //     let mut chat_rw = self.session.as_ref().write().unwrap();
                            //     chat_rw.clear();
                            // }
                        });
                    });

                egui::CentralPanel::default().show_inside(ui, |ui| {
                    let mut prompt_w = self.prompt.write().unwrap();
                    // ui.text_edit_multiline(&mut *prompt_w);
                    let widget = egui::TextEdit::multiline(&mut *prompt_w)
                        .desired_width(f32::INFINITY)
                        .hint_text("Type your message here \u{1F64B}");

                    ui.add_sized(ui.available_size(), widget);
                });

                if submitted {
                    self.on_submit(ui);
                }
            });

        egui::CentralPanel::default().show_inside(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.set_width(ui.available_width());

                let scroll_bottom = {
                    let settings_r = self.settings.read().unwrap();
                    settings_r.autoscroll || ui.button("Scroll to bottom.").clicked()
                };

                let cache = &mut self.cache;
                self.session.view(|session_r| {
                    for msg in session_r.iter() {
                        match &msg.content {
                            ChatContent::Message(message) => {
                                // TODO: only on user prompt
                                if let Message::User { .. } = message
                                    && ui.button(GIT_BRANCH).clicked()
                                {
                                    self.branch_point = Some(msg.id);
                                }
                                render_message(ui, cache, message);
                            }
                            ChatContent::Aside {
                                automation: workflow,
                                prompt: _,
                                collapsed,
                                content,
                            } => {
                                let resp =
                                    egui::CollapsingHeader::new(format!("Workflow: {workflow}"))
                                        .id_salt(msg.id)
                                        .default_open(!collapsed)
                                        .show(ui, |ui| {
                                            for message in content {
                                                render_message(ui, cache, message);
                                            }
                                        });
                                if resp.fully_closed()
                                    && let Some(message) = content.last()
                                {
                                    render_message(ui, cache, message);
                                }
                            }
                            ChatContent::Error(err) => {
                                error_bubble(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    ui.label(egui::RichText::new(err).color(egui::Color32::RED));
                                });
                            }
                        }
                    }
                });

                let chat_r = self.scratch.read().unwrap();
                for msg in chat_r.iter() {
                    match msg {
                        Ok(Message::User { content }) => {
                            let UserContent::Text(text) = content.first() else {
                                todo!();
                            };
                            user_bubble(ui, |ui| {
                                ui.set_width(ui.available_width() * 0.75);
                                CommonMarkViewer::new().show(ui, &mut self.cache, text.text());
                            });
                        }
                        Ok(Message::Assistant { content, .. }) => {
                            let AssistantContent::Text(text) = content.first() else {
                                todo!();
                            };

                            agent_bubble(ui, |ui| {
                                ui.set_width(ui.available_width() * 0.75);
                                CommonMarkViewer::new().show(ui, &mut self.cache, text.text());
                            });
                        }
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
                let mut prompt_w = self.prompt.write().unwrap();
                if prompt_w.is_empty()
                    && let Some(entry) = hist.store.get(&branch_point)
                    && let ChatContent::Message(msg) = &entry.content
                    && let Message::User { content } = msg
                    && let UserContent::Text(text) = content.first()
                {
                    *prompt_w = text.text().to_string();
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

    fn on_submit(&mut self, ui: &mut egui::Ui) {
        let ui_ctx = ui.ctx().clone();
        let session_ = self.session.clone();
        let scratch_ = self.scratch.clone();
        let prompt_ = self.prompt.clone();
        let task_count_ = self.task_count.clone();
        let agent_factory_ = self.agent_factory.clone();
        let log_history_ = self.log_history.clone();
        let branch = self.session.view(|hist| hist.head.clone());
        let errors = self.errors.clone();

        let workflow = {
            let settings_r = self.settings.read().unwrap();
            settings_r.get_automation().cloned().unwrap_or_default()
        };

        self.rt.spawn(async move {
            task_count_.fetch_add(1, Ordering::Relaxed);
            let user_prompt = std::mem::take(&mut *prompt_.write().unwrap());
            let start = Instant::now();
            let env = Environment::new();

            for workstep in workflow.steps {
                if workstep.disabled {
                    continue;
                }

                let Ok(tmpl) = env.template_from_str(&workstep.prompt) else {
                    tracing::warn!("Cannot load template from step: {workstep:?}");
                    continue;
                };

                // TODO: implement last_message
                // let last_message = {
                //     let session = session_.read().unwrap();
                //     let last_message = session
                //         .last()
                //         .map(|msg| match msg.content {
                //             ChatContent::Message(message) => message.content,
                //             ChatContent::Aside {
                //                 workflow,
                //                 prompt,
                //                 collapsed,
                //                 content,
                //             } => todo!(),
                //             ChatContent::Error(err) => err,
                //         })
                //         .unwrap_or(String::default());
                // };

                let Ok(prompt) = tmpl.render(context! {user_prompt}) else {
                    tracing::warn!("Cannot render prompt for step: {workstep:?}");
                    continue;
                };

                let mut history = session_.view(|session| {
                    let mut buffer: VecDeque<&Message> = if let Some(depth) = workstep.depth {
                        VecDeque::with_capacity(depth)
                    } else {
                        VecDeque::new()
                    };

                    for entry in session.iter() {
                        match &entry.content {
                            ChatContent::Message(message) => buffer.push_back(message),
                            ChatContent::Aside { content, .. } => {
                                if let Some(message) = content.last() {
                                    buffer.push_back(message);
                                }
                            }
                            ChatContent::Error(_) => {}
                        }
                    }

                    let mut scratch = scratch_.write().unwrap();
                    scratch.push(Ok(Message::user(&prompt)));

                    for message in scratch[..scratch.len() - 1]
                        .iter()
                        .filter_map(|it| it.as_ref().ok())
                    {
                        buffer.push_back(message);
                    }

                    // Laziness avoids cloning older items we won't use
                    buffer.into_iter().cloned().collect::<Vec<_>>()
                });

                let agent = agent_factory_.agent(&workstep);
                let request = PromptRequest::new(&agent, &prompt)
                    .multi_turn(5)
                    .with_history(&mut history);

                match request.await {
                    Ok(response) => {
                        let mut chat = scratch_.write().unwrap();
                        chat.push(Ok(Message::assistant(response)));
                        ui_ctx.request_repaint();
                    }

                    Err(err) => {
                        tracing::warn!("Failed chat: {err:?}");

                        let mut chat = scratch_.write().unwrap();
                        let err_str = format!("{err}");
                        chat.push(Err(err_str.clone()));
                        ui_ctx.request_repaint();

                        let mut log_rw = log_history_.write().unwrap();
                        log_rw.push(LogEntry(Level::ERROR, format!("Error: {err:?}")));
                    }
                }
            }

            let scratch = std::mem::take(&mut *scratch_.write().unwrap());
            errors.distil(session_.transform(|history| {
                let history = Cow::Borrowed(history);
                if scratch.len() <= 2 {
                    history.try_moo(|h| {
                        h.extend(scratch.into_iter().map(|it| it.into()), Some(&branch))
                    })
                } else {
                    use itertools::{Either, Itertools as _};

                    // Split scratch so messages can be collapsed but errors will appear inline
                    let mut iter = scratch.into_iter();
                    let Some(prompt_msg) = iter.next() else {
                        unreachable!()
                    };

                    let (content, errors): (Vec<_>, Vec<_>) = iter.partition_map(|r| match r {
                        Ok(v) => Either::Left(v),
                        Err(v) => Either::Right(v),
                    });

                    let aside_msgs = ChatContent::Aside {
                        automation: workflow.name,
                        prompt: user_prompt,
                        collapsed: true,
                        content,
                    };

                    let error_msgs = errors.into_iter().map(ChatContent::Error);

                    // No inconsistent partial updates if intermediate ops fail
                    history
                        .try_moo(|h| h.push(prompt_msg.into(), Some(&branch)))?
                        .try_moo(|h| h.push(aside_msgs, Some(&branch)))?
                        .try_moo(|h| h.extend(error_msgs, Some(&branch)))
                }
            }));

            task_count_.fetch_sub(1, Ordering::Relaxed);
            tracing::info!(
                "Request completed in {} seconds.",
                Instant::now().duration_since(start).as_secs_f32()
            );
        });
    }
}

fn render_message(ui: &mut egui::Ui, cache: &mut CommonMarkCache, message: &Message) {
    use base64::prelude::*;

    match message {
        Message::User { content } => {
            let UserContent::Text(text) = content.first() else {
                todo!();
            };

            user_bubble(ui, |ui| {
                ui.set_width(ui.available_width() * 0.75);
                CommonMarkViewer::new().show(ui, cache, text.text());
            });
        }
        Message::Assistant { content, .. } => {
            use regex::Regex;

            let re = Regex::new(r"(?ms)```mermaid(.*)```").unwrap();

            let AssistantContent::Text(text) = content.first() else {
                todo!();
            };

            agent_bubble(ui, |ui| {
                ui.set_width(ui.available_width() * 0.75);
                CommonMarkViewer::new().show(ui, cache, text.text());
            });

            for (_, [diagram]) in re.captures_iter(text.text()).map(|m| m.extract()) {
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
