use std::{
    f32,
    sync::{
        Arc, RwLock,
        atomic::{AtomicU16, Ordering},
    },
    time::{Duration, Instant},
};

use aerie::{LogChannelLayer, LogEntry};
use eframe::egui;
use egui_commonmark::*;
use rig::{
    agent::Agent,
    client::{CompletionClient as _, ProviderClient as _},
    completion::Chat as _,
    message::{AssistantContent, Message, UserContent},
    providers::ollama::{self, CompletionModel},
};
use rmcp::{
    ServiceExt as _,
    model::Tool,
    transport::{ConfigureCommandExt, TokioChildProcess},
};
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;
use tokio::process::Command;
use tracing_subscriber::{
    Layer as _, filter, layer::SubscriberExt as _, util::SubscriberInitExt as _,
};

#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug, Default, PartialEq, Clone)]
pub struct Settings {
    #[serde(default)]
    pub llm_model: String,

    #[serde(default)]
    pub preamble: String,

    #[serde(default)]
    pub temperature: f32,

    #[serde(default)]
    pub show_logs: bool,

    #[serde(default)]
    pub autoscroll: bool,
}

fn get_agent(
    settings: &Settings,
    mcp_client: &rmcp::service::ServerSink,
    tools: Vec<Tool>,
) -> Agent<CompletionModel> {
    let llm_client = ollama::Client::from_env();
    let model = if settings.llm_model.is_empty() {
        "devstral:latest"
    } else {
        settings.llm_model.as_str()
    };

    let llm_agent = llm_client
        .agent(model)
        .preamble(&settings.preamble)
        .temperature(settings.temperature as f64);

    let llm_agent = tools.into_iter().fold(llm_agent, |agent, tool| {
        agent.rmcp_tool(tool, mcp_client.clone())
    });

    llm_agent.build()
}

fn main() -> anyhow::Result<()> {
    let (log_tx, log_rx) = flume::unbounded::<LogEntry>();

    tracing_subscriber::registry()
        .with(
            LogChannelLayer(log_tx)
                // .with_filter(filter::LevelFilter::DEBUG)
                .with_filter(filter::filter_fn(|metadata| {
                    metadata.target().starts_with("rig")
                })),
        )
        .with(
            tracing_subscriber::fmt::layer().with_filter(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| format!("info,{}=warn", env!("CARGO_CRATE_NAME")).into()),
            ),
        )
        .init();

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
        .join("workbench.yml");

    // Runtime settings:
    let settings = if settings_path.is_file() {
        let text = std::fs::read_to_string(&settings_path)?;
        serde_yml::from_str(&text)?
    } else {
        Settings::default()
    };

    let mut stored_settings = Arc::new(settings.clone());
    let settings = Arc::new(RwLock::new(settings));

    let (mcp_client, mcp_tools) = rt.handle().block_on(async move {
        let mcp_client = ()
            .serve(TokioChildProcess::new(Command::new("cargo").configure(
                |cmd| {
                    cmd.arg("run")
                        .arg("--")
                        .arg("--embed-model")
                        .arg("MxbaiEmbedLargeV1Q")
                        .arg("--collection")
                        .arg("goose")
                        .current_dir("/code/rust/refoliate/embcp-server");
                },
            ))?)
            .await
            .inspect_err(|e| {
                tracing::error!("client error: {:?}", e);
            })?;

        let mcp_tools: Vec<Tool> = mcp_client.list_tools(Default::default()).await?.tools;
        Ok::<_, anyhow::Error>((mcp_client, mcp_tools))
    })?;

    // Our application state:
    let task_count = Arc::new(AtomicU16::new(0));
    let log_history = Arc::new(RwLock::new(Vec::<LogEntry>::new()));
    let chat = Arc::new(RwLock::new(Vec::<Message>::new()));
    let mut cache = CommonMarkCache::default();
    let prompt = Arc::new(RwLock::new(String::new()));
    let mut debounce = Instant::now() + Duration::from_secs(1);

    let log_history_ = log_history.clone();

    // TODO: clean shutdown
    rt.handle().spawn(async move {
        use std::io::Write;
        use tracing::Level;
        while let Ok(entry) = log_rx.recv_async().await {
            let colored_level = match entry.level() {
                Level::TRACE => "\x1b[35mTRACE\x1b[0m", // Purple
                Level::DEBUG => "\x1b[34mDEBUG\x1b[0m", // Blue
                Level::INFO => "\x1b[32m INFO\x1b[0m",  // Green
                Level::WARN => "\x1b[33m WARN\x1b[0m",  // Yellow
                Level::ERROR => "\x1b[31mERROR\x1b[0m", // Red
            };

            let _ = writeln!(
                std::io::stdout(),
                ">>>{colored_level} {}<<<",
                entry.message()
            );

            let mut log_rw = log_history_.write().unwrap();
            log_rw.push(entry);
        }
    });

    let mut llm_agent = {
        let settings_r = settings.read().unwrap();
        Arc::new(get_agent(
            &settings_r.clone(),
            &mcp_client,
            mcp_tools.clone(),
        ))
    };

    eframe::run_simple_native("My egui App", options, move |ctx, _frame| {
        egui::SidePanel::right("right_panel")
            .resizable(true)
            .default_width(120.0)
            .width_range(80.0..=500.0)
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.heading("Options");
                });
                egui::ScrollArea::vertical().show(ui, |ui| {
                    let mut settings_rw = settings.write().unwrap();

                    egui::ComboBox::from_label("Model")
                        .selected_text(settings_rw.llm_model.to_string())
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut settings_rw.llm_model,
                                "devstral:latest".to_string(),
                                "Devstral",
                            );
                            ui.selectable_value(
                                &mut settings_rw.llm_model,
                                "magistral:latest".to_string(),
                                "Magistral",
                            );
                            ui.selectable_value(
                                &mut settings_rw.llm_model,
                                "my-qwen3-coder:30b".to_string(),
                                "Qwen3 Coder",
                            );
                        });

                    ui.add(
                        egui::TextEdit::multiline(&mut settings_rw.preamble)
                            .hint_text("Preamble")
                            .desired_width(f32::INFINITY),
                    );

                    ui.add(egui::Slider::new(&mut settings_rw.temperature, 0.0..=1.0).text("T"))
                        .on_hover_text("temperature");

                    egui::CollapsingHeader::new("Flags")
                        .default_open(true)
                        .show(ui, |ui| {
                            ui.horizontal_wrapped(|ui| {
                                // ui.spacing_mut().item_spacing.x = 0.0;
                                ui.toggle_value(&mut settings_rw.autoscroll, "autoscroll");
                                ui.toggle_value(&mut settings_rw.show_logs, "logs");
                            });
                        });
                    ui.allocate_space(egui::vec2(ui.available_width(), 0.0))
                });
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

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
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

                        match llm_agent_.chat(&prompt, history).await {
                            Ok(response) => {
                                let mut chat = chat_.write().unwrap();
                                chat.push(Message::assistant(response));
                                ui_ctx.request_repaint();
                            }

                            Err(err) => tracing::warn!("Failed to chat: {err:?}"),
                        }

                        task_count_.fetch_sub(1, Ordering::Relaxed);
                        // *prompt_.write().unwrap() = String::default();
                    });
                }

                ui.add_space(16.0);

                if ui.button("Clear").clicked() {
                    let mut chat_rw = chat.write().unwrap();
                    chat_rw.clear();
                }
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.set_width(ui.available_width());

                let scroll_bottom = {
                    let settings_r = settings.read().unwrap();
                    settings_r.autoscroll || ui.button("Scroll to bottom.").clicked()
                };

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

                // Add an extra line to prevent clipping on long text
                // let font_id = egui::TextStyle::Body.resolve(ui.style());
                // ui.add_space(128.0);

                if scroll_bottom {
                    ui.scroll_to_cursor(Some(egui::Align::BOTTOM));
                }
            });
        });

        let dirty = {
            // Hmmm, should we change to only fire after input has stopped for a duration?
            let settings_r = settings.read().unwrap();
            *settings_r != *stored_settings
        };

        if dirty && debounce < Instant::now() {
            // This will apply/save settings every n seconds *while* or *after* they have been changed
            debounce = Instant::now() + Duration::from_secs(5);
            let settings_ = settings.clone();
            let settings_path_ = settings_path.clone();

            log::info!("Settings changed, reloading agent");

            rt.spawn(async move {
                use tokio::io::AsyncWriteExt as _;
                let text = {
                    let settings_r = settings_.read().unwrap();
                    serde_yml::to_string(&*settings_r).unwrap()
                };

                let mut file = tokio::fs::File::create(settings_path_).await.unwrap();
                file.write_all(text.as_bytes()).await.unwrap();
            });

            let settings_r = settings.read().unwrap();
            stored_settings = Arc::new(settings_r.clone());
            llm_agent = Arc::new(get_agent(
                &settings_r.clone(),
                &mcp_client,
                mcp_tools.clone(),
            ));
        }

        let mut settings_rw = settings.write().unwrap();
        egui::Window::new("Agent logs")
            .open(&mut settings_rw.show_logs)
            .default_size(egui::vec2(544.0, 512.0))
            .resizable(true)
            .show(ctx, |ui| {
                egui::ScrollArea::both().show(ui, |ui| {
                    let logs_r = log_history.read().unwrap();
                    let language = "json";
                    let theme = egui_extras::syntax_highlighting::CodeTheme::from_memory(
                        ui.ctx(),
                        ui.style(),
                    );

                    for entry in logs_r.iter() {
                        // ui.label(entry.message());
                        egui_extras::syntax_highlighting::code_view_ui(
                            ui,
                            &theme,
                            entry.message(),
                            language,
                        );
                    }
                });
            });
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
