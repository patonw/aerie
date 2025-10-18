use eframe::egui;
use egui_commonmark::*;
use rig::{
    agent::PromptRequest,
    message::{AssistantContent, Message, UserContent},
};
use std::{f32, sync::atomic::Ordering, time::Instant};
use tracing::Level;

// use super::{Pane, agent_bubble, error_bubble, tiles, user_bubble};
use crate::LogEntry;

use crate::ui::{agent_bubble, error_bubble, user_bubble};

// Too many refs to self for a free function. Need to clean this up
impl super::AppBehavior {
    pub fn chat_ui(&mut self, ui: &mut egui::Ui) {
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

                            if ui.button("Clear").clicked() {
                                let mut chat_rw = self.chat.write().unwrap();
                                chat_rw.clear();
                            }
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

                let chat_r = self.chat.read().unwrap();
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
    }

    fn on_submit(&mut self, ui: &mut egui::Ui) {
        let ui_ctx = ui.ctx().clone();
        let chat_ = self.chat.clone();
        let prompt_ = self.prompt.clone();
        let task_count_ = self.task_count.clone();
        let llm_agent_ = self.llm_agent.clone();
        let log_history_ = self.log_history.clone();

        self.rt.spawn(async move {
            task_count_.fetch_add(1, Ordering::Relaxed);
            let prompt = std::mem::take(&mut *prompt_.write().unwrap());
            let start = Instant::now();

            let mut history = {
                let mut chat = chat_.write().unwrap();
                let history = chat.iter().cloned().filter_map(|m| m.ok()).collect();

                chat.push(Ok(Message::user(&prompt)));
                ui_ctx.request_repaint();
                history
            };

            let request = PromptRequest::new(&llm_agent_, prompt)
                .multi_turn(5)
                .with_history(&mut history);

            match request.await {
                Ok(response) => {
                    let mut chat = chat_.write().unwrap();
                    chat.push(Ok(Message::assistant(response)));
                    ui_ctx.request_repaint();
                }

                Err(err) => {
                    tracing::warn!("Failed chat: {err:?}");

                    let mut chat = chat_.write().unwrap();
                    let err_str = format!("{err}");
                    chat.push(Err(err_str.clone()));
                    ui_ctx.request_repaint();

                    let mut log_rw = log_history_.write().unwrap();
                    log_rw.push(LogEntry(Level::ERROR, format!("Error: {err:?}")));
                }
            }

            task_count_.fetch_sub(1, Ordering::Relaxed);
            tracing::info!(
                "Request completed in {} seconds.",
                Instant::now().duration_since(start).as_secs_f32()
            );
        });
    }
}
