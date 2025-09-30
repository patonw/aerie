use std::sync::{
    Arc, RwLock,
    atomic::{AtomicU16, Ordering},
};

use eframe::egui;
use egui_commonmark::*;
use rig::{
    agent::Agent,
    client::{CompletionClient as _, ProviderClient as _},
    completion::Chat as _,
    message::{AssistantContent, Message, UserContent},
    providers::ollama::{self, CompletionModel},
};
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug, Default, PartialEq, Clone)]
pub struct Settings {
    #[serde(default)]
    pub preamble: String,
    pub asetting: bool,

    #[serde(default)]
    pub temperature: f32,
}

fn get_agent(settings: &Settings) -> Agent<CompletionModel> {
    let llm_client = ollama::Client::from_env();
    llm_client
        .agent("devstral:latest")
        .preamble(&settings.preamble)
        .temperature(settings.temperature as f64)
        .build()
}

fn main() -> anyhow::Result<()> {
    env_logger::init(); // Log to stderr (if you run with `RUST_LOG=debug`).
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .unwrap();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([320.0, 240.0]),
        ..Default::default()
    };

    // TODO: CLI arg
    // TODO: ensure dir
    let settings_path = dirs::config_dir()
        .map(|p| p.join("emberlain"))
        .unwrap_or_default()
        .join("workbench.toml");

    // Runtime settings:
    let settings = if settings_path.is_file() {
        let text = std::fs::read_to_string(&settings_path)?;
        serde_yml::from_str(&text)?
    } else {
        Settings::default()
    };

    let mut stored_settings = Arc::new(settings.clone());
    let settings = Arc::new(RwLock::new(settings));

    // Our application state:
    let task_count = Arc::new(AtomicU16::new(0));
    let chat = Arc::new(RwLock::new(Vec::<Message>::new()));
    let mut cache = CommonMarkCache::default();
    let prompt = Arc::new(RwLock::new(String::new()));

    let mut llm_agent = {
        let settings_r = settings.read().unwrap();
        Arc::new(get_agent(&settings_r.clone()))
    };

    eframe::run_simple_native("My egui App", options, move |ctx, _frame| {
        egui::SidePanel::right("right_panel")
            .resizable(true)
            .default_width(100.0)
            .width_range(80.0..=500.0)
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.heading("Options");
                });
                egui::ScrollArea::vertical().show(ui, |ui| {
                    let mut settings_rw = settings.write().unwrap();

                    ui.text_edit_multiline(&mut settings_rw.preamble);
                    ui.add(egui::Slider::new(&mut settings_rw.temperature, 0.0..=1.0).text("T"))
                        .on_hover_text("temperature");

                    egui::CollapsingHeader::new("Flags")
                        .default_open(true)
                        .show(ui, |ui| {
                            ui.horizontal_wrapped(|ui| {
                                // ui.spacing_mut().item_spacing.x = 0.0;
                                ui.toggle_value(&mut settings_rw.asetting, "Na-na");
                                ui.toggle_value(&mut settings_rw.asetting, "Na");
                                ui.toggle_value(&mut settings_rw.asetting, "Na");
                                ui.toggle_value(&mut settings_rw.asetting, "Hey");
                                ui.toggle_value(&mut settings_rw.asetting, "Hey");
                                ui.toggle_value(&mut settings_rw.asetting, "Hey");
                                ui.toggle_value(&mut settings_rw.asetting, "Goodbye");
                            });
                        });
                });
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            let chat_r = chat.read().unwrap();
            for msg in chat_r.iter() {
                match msg {
                    Message::User { content } => {
                        let UserContent::Text(text) = content.first() else {
                            todo!();
                        };
                        user_bubble(ui, |ui| {
                            ui.set_width(ui.available_width() * 0.75);
                            CommonMarkViewer::new().show(ui, &mut cache, text.text());
                        });
                    }
                    Message::Assistant { content, .. } => {
                        let AssistantContent::Text(text) = content.first() else {
                            todo!();
                        };

                        agent_bubble(ui, |ui| {
                            ui.set_width(ui.available_width() * 0.75);
                            CommonMarkViewer::new().show(ui, &mut cache, text.text());
                        });
                    }
                }
            }

            if task_count.load(Ordering::Relaxed) > 0 {
                ui.spinner();
            }
        });

        let task_count_ = task_count.clone();
        // let settings_ = settings.clone();
        let llm_agent_ = llm_agent.clone();

        egui::TopBottomPanel::bottom("Down").show(ctx, |ui| {
            {
                let mut prompt_w = prompt.write().unwrap();
                // ui.text_edit_multiline(&mut *prompt_w);
                egui::TextEdit::multiline(&mut *prompt_w)
                    .desired_width(f32::INFINITY)
                    .hint_text("Type your message here \u{1F64B}")
                    .show(ui);
            }

            let submitted = ui.input(|i| {
                (i.modifiers.ctrl || i.modifiers.alt) && i.key_pressed(egui::Key::Enter)
            });

            if ui.button("Chat").clicked() || submitted {
                let ui_ctx = ui.ctx().clone();
                let chat_ = chat.clone();
                let prompt_ = prompt.clone();

                rt.handle().spawn(async move {
                    task_count_.fetch_add(1, Ordering::Relaxed);
                    let prompt = std::mem::take(&mut *prompt_.write().unwrap());

                    let history = {
                        let mut chat = chat_.write().unwrap();
                        let history = chat.clone();
                        chat.push(Message::user(&prompt));
                        ui_ctx.request_repaint();
                        history
                    };

                    let response = llm_agent_.chat(&prompt, history).await.unwrap();
                    println!("chat response: {response}");

                    {
                        let mut chat = chat_.write().unwrap();
                        chat.push(Message::assistant(response));
                        ui_ctx.request_repaint();
                    }

                    task_count_.fetch_sub(1, Ordering::Relaxed);
                    // *prompt_.write().unwrap() = String::default();
                });
            }
        });

        let dirty = {
            let settings_r = settings.read().unwrap();
            let dirty = *settings_r != *stored_settings;
            if dirty {
                stored_settings = Arc::new(settings_r.clone());
            }

            llm_agent = Arc::new(get_agent(&settings_r.clone()));

            dirty
        };

        if dirty {
            let settings_ = settings.clone();
            let settings_path_ = settings_path.clone();

            rt.spawn(async move {
                use tokio::io::AsyncWriteExt as _;
                let text = {
                    let settings_r = settings_.read().unwrap();
                    serde_yml::to_string(&*settings_r).unwrap()
                };

                let mut file = tokio::fs::File::create(settings_path_).await.unwrap();
                file.write_all(text.as_bytes()).await.unwrap();
            });
        }
    })
    .map_err(|e| anyhow::anyhow!("I can't {e:?}"))?;

    Ok(())
}

fn user_bubble<R>(ui: &mut egui::Ui, cb: impl FnMut(&mut egui::Ui) -> R) -> egui::InnerResponse<R> {
    ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
        egui::Frame::new()
            .stroke(egui::Stroke::new(1.0, egui::Color32::GRAY))
            .corner_radius(16)
            .outer_margin(4)
            .inner_margin(8)
            .show(ui, cb)
            .inner
    })
}

fn agent_bubble<R>(
    ui: &mut egui::Ui,
    cb: impl FnMut(&mut egui::Ui) -> R,
) -> egui::InnerResponse<R> {
    ui.with_layout(egui::Layout::left_to_right(egui::Align::TOP), |ui| {
        egui::Frame::new()
            .stroke(egui::Stroke::new(1.0, egui::Color32::GRAY))
            .corner_radius(16)
            .outer_margin(4)
            .inner_margin(8)
            .show(ui, cb)
            .inner
    })
}
