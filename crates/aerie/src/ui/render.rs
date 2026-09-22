use crate::events::{AppEvent, AppEvents};
use crate::rig::message::Message;
use crate::utils::{ErrorList, IMAGE_CACHE, MERMAID_MD, show_image};
use eframe::egui;
use egui_commonmark::*;
use std::collections::VecDeque;
use std::sync::Arc;

use crate::{
    ui::{agent_bubble, user_bubble},
    utils::FormatOpts,
};

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
    let mut dq = VecDeque::from_iter(message.text_fmt_opts().into_iter().enumerate());

    match message {
        Message::User { .. } => {
            user_bubble(ui, |ui| {
                ui.set_width(width.unwrap_or(ui.available_width() * 0.75));

                ui.vertical(|ui| {
                    while let Some((idx, (text, fmt))) = dq.pop_front() {
                        match fmt {
                            FormatOpts::Plain => {
                                ui.label(text.as_ref());
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

                                            let layout_job =
                                                egui_extras::syntax_highlighting::highlight(
                                                    ui.ctx(),
                                                    ui.style(),
                                                    &theme,
                                                    &text,
                                                    language,
                                                );
                                            ui.add(
                                                egui::Label::new(layout_job)
                                                    .selectable(true)
                                                    .wrap(),
                                            );
                                        },
                                    );
                                });
                            }
                            FormatOpts::Markdown => {
                                CommonMarkViewer::new().show(ui, cache, &text);
                            }
                            FormatOpts::Reasoning => {}
                            FormatOpts::Image => {
                                let mut cache = IMAGE_CACHE.lock();
                                let mut image = text;

                                egui::ScrollArea::horizontal().show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        loop {
                                            if let Some(image) = cache.get(image.as_ref()) {
                                                show_image(ui, image, 100.0);
                                            }

                                            if matches!(
                                                dq.front(),
                                                Some((_, (_, FormatOpts::Image)))
                                            ) {
                                                image = dq.pop_front().unwrap().1.0;
                                            } else {
                                                break;
                                            }
                                        }
                                    });
                                });
                            }
                            FormatOpts::Unknown => {
                                ui.label(text.as_ref());
                            }
                        }

                        if !dq.is_empty() {
                            ui.separator();
                        }
                    }
                });
            });
        }
        Message::Assistant { .. } => {
            let mut all_text = String::new();

            agent_bubble(ui, |ui| {
                ui.set_width(width.unwrap_or(ui.available_width() * 0.75));

                ui.vertical(|ui| {
                    while let Some((idx, (text, fmt))) = dq.pop_front() {
                        match fmt {
                            FormatOpts::Plain => {
                                ui.label(text.as_ref());
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
                            FormatOpts::Reasoning => {
                                ui.label(egui::RichText::new(text.as_ref()).italics());
                            }
                            FormatOpts::Image => {
                                let mut cache = IMAGE_CACHE.lock();
                                let mut image = text;

                                ui.horizontal(|ui| {
                                    loop {
                                        if let Some(image) = cache.get(image.as_ref()) {
                                            show_image(ui, image, 100.0);
                                        }

                                        if matches!(dq.front(), Some((_, (_, FormatOpts::Image)))) {
                                            image = dq.pop_front().unwrap().1.0;
                                        } else {
                                            break;
                                        }
                                    }
                                });
                            }
                            FormatOpts::Unknown => {
                                all_text.push_str(&text);
                                ui.label(text.as_ref());
                            }
                        }

                        if !dq.is_empty() {
                            ui.separator();
                        }
                    }
                });
            });

            for (_, [diagram]) in MERMAID_MD.captures_iter(&all_text).map(|m| m.extract()) {
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
        Message::System { content } => {
            // We don't actually include system messages in the chat history.
            // They're properties of individual agents and can change within a workflow
            // as different agents operate on the history.
            egui::Frame::new()
                .stroke(egui::Stroke::new(1.0, egui::Color32::GRAY))
                .corner_radius(16)
                .outer_margin(4)
                .inner_margin(8)
                .show(ui, |ui| {
                    ui.set_width(width.unwrap_or(ui.available_width()));
                    ui.label(content);
                });
        }
    }
}

pub fn display_errors(
    ctx: &egui::Context,
    content_rect: egui::Rect,
    errors: ErrorList<anyhow::Error>,
    events: Arc<AppEvents>,
) {
    let guard = errors.load();
    if !guard.is_empty() {
        let modal = egui::Modal::new(egui::Id::new("Errors")).show(ctx, |ui| {
            ui.set_max_width(content_rect.width() * 0.8);
            ui.set_min_height(content_rect.height() * 0.5);
            ui.heading("Errors");
            egui::ScrollArea::both()
                .auto_shrink(egui::Vec2b::new(true, false))
                .show(ui, |ui| {
                    for (i, err) in guard.iter().enumerate() {
                        egui::collapsing_header::CollapsingState::load_with_default_open(
                            ui.ctx(),
                            ui.make_persistent_id(format!("error #{i}")),
                            false,
                        )
                        .show_header(ui, |ui| {
                            let heading = egui::RichText::new(
                                err.to_string().lines().next().unwrap_or_default(),
                            )
                            .strong();
                            ui.add(egui::Label::new(heading).wrap());
                        })
                        .body(|ui| {
                            ui.label(format!("{err:?}"));
                            // for line in err.to_string().lines().skip(1) {
                            //     ui.label(line);
                            // }
                        });
                    }
                });
        });

        if modal.should_close() {
            events.insert(AppEvent::DiscardErrors);
        }
    }
}
