use arc_swap::{ArcSwap, ArcSwapOption};
use clap::Parser as _;
use eframe::egui;
use egui::KeyboardShortcut;
use egui_commonmark::*;
use egui_tiles::{LinearDir, TileId};
use std::{
    sync::{
        Arc,
        atomic::{AtomicU16, Ordering},
    },
    time::{Duration, Instant},
};
use tracing_subscriber::{
    Layer as _, filter, layer::SubscriberExt as _, util::SubscriberInitExt as _,
};

use aerie::{
    AgentFactory, LogChannelLayer, LogEntry, Settings,
    chat::ChatSession,
    config::{Args, Command, ConfigExt, SessionCommand},
    ui::{AppState, Pane, state::WorkflowState},
    utils::ErrorDistiller as _,
    workflow::store::WorkflowStoreDir,
};

const SHORTCUT_QUIT: KeyboardShortcut = KeyboardShortcut {
    modifiers: egui::Modifiers::CTRL,
    logical_key: egui::Key::Q,
};

fn main() -> anyhow::Result<()> {
    let (log_tx, log_rx) = flume::unbounded::<LogEntry>();
    let args = Args::parse();

    let settings_path = args.config.unwrap_or(
        dirs::config_dir()
            .map(|p| p.join("aerie"))
            .unwrap_or_default()
            .join("workbench.yml"),
    );

    // Shhh...
    let _ = dotenvy::from_path(settings_path.with_file_name(".env"));

    let data_dir = dirs::data_dir()
        .unwrap_or(".data/share".into())
        .join("aerie");

    let session_dir = args.session_dir.unwrap_or(data_dir.join("sessions"));
    let workflow_dir = args.workflow_dir.unwrap_or(data_dir.join("workflows"));

    std::fs::create_dir_all(settings_path.parent().unwrap())?;
    std::fs::create_dir_all(&session_dir)?;
    std::fs::create_dir_all(&workflow_dir)?;

    if let Some(Command::Session {
        subcmd: SessionCommand::List,
    }) = args.command
    {
        if let Ok(read_dir) = std::fs::read_dir(&session_dir) {
            for path in read_dir {
                let Ok(dirent) = path else { continue };
                let pathbuf = dirent.path();
                let Some(stem) = pathbuf.file_stem() else {
                    continue;
                };

                println!("{}", stem.display());
            }
        }

        return Ok(());
    }

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

    let _guard = rt.enter();

    let options = eframe::NativeOptions {
        // viewport: egui::ViewportBuilder::default().with_inner_size([320.0, 240.0]),
        ..Default::default()
    };

    // Runtime settings:
    let settings = if settings_path.is_file() {
        let text = std::fs::read_to_string(&settings_path)?;
        serde_yml::from_str(&text)?
    } else {
        Settings::default()
    };

    let session_name = args.session.as_deref().or(settings.session.as_deref());
    let session = ChatSession::from_dir_name(session_dir, session_name).build()?;

    let mut stored_settings = Arc::new(settings.clone());
    let settings = Arc::new(ArcSwap::from_pointee(settings));

    // Our application state:
    let task_count = Arc::new(AtomicU16::new(0));
    let log_history: Arc<arc_swap::ArcSwapAny<Arc<im::Vector<LogEntry>>>> =
        Arc::new(ArcSwap::from_pointee(im::Vector::<LogEntry>::new()));
    let cache = CommonMarkCache::default();
    let mut debounce = Instant::now() + Duration::from_secs(1);

    let next_workflow: Arc<ArcSwapOption<String>> = Default::default();
    let next_prompt: Arc<ArcSwapOption<String>> = Default::default();

    let log_history_ = log_history.clone();

    // TODO: clean shutdown
    rt.handle().spawn(async move {
        while let Ok(entry) = log_rx.recv_async().await {
            log_history_.rcu(|logs| {
                let mut logs = logs.as_ref().clone();
                logs.push_back(entry.clone());

                if logs.len() > 1000 {
                    logs.skip(logs.len() - 1000)
                } else {
                    logs
                }
            });
        }
    });

    let mut tiles = egui_tiles::Tiles::default();
    let tabs: Vec<TileId> = vec![
        tiles.insert_pane(Pane::Chat),
        tiles.insert_pane(Pane::Logs),
        tiles.insert_pane(Pane::Messages),
        tiles.insert_pane(Pane::Workflow),
    ];
    let content_tabs: TileId = tiles.insert_tab_tile(tabs);

    let tabs = vec![
        tiles.insert_pane(Pane::Navigator),
        tiles.insert_pane(Pane::Tools),
        tiles.insert_pane(Pane::Settings),
    ];
    let setter_tabs = tiles.insert_tab_tile(tabs);

    let tabs = vec![tiles.insert_pane(Pane::Outputs)];
    let inspector_tabs = tiles.insert_tab_tile(tabs);

    let vsplit =
        egui_tiles::Linear::new_binary(LinearDir::Vertical, [setter_tabs, inspector_tabs], 0.5);

    let sidebar = tiles.insert_container(vsplit);

    let hsplit =
        egui_tiles::Linear::new_binary(LinearDir::Horizontal, [content_tabs, sidebar], 0.75);

    let root = tiles.insert_container(hsplit);

    let mut tree = egui_tiles::Tree::new("my_tree", root, tiles);

    let flow_name = settings.view(|s| s.automation.clone());
    let flow_store = WorkflowStoreDir::load_all(workflow_dir, true)?;
    let flow_state = WorkflowState::new(flow_store.clone(), flow_name);

    let mut agent_factory = AgentFactory::builder()
        .rt(rt.handle().to_owned())
        .settings(settings.clone())
        .store(Some(flow_store.clone()))
        .next_workflow(next_workflow.clone())
        .next_prompt(next_prompt.clone())
        .build();

    agent_factory.reload_tools()?;

    let mut behavior = AppState::builder()
        .settings(settings.clone())
        .log_history(log_history.clone())
        .task_count(task_count.clone())
        .session(session)
        .cache(cache)
        .rt(rt.handle().clone())
        .agent_factory(agent_factory)
        .workflows(flow_state)
        .build();

    let rt_ = rt.handle().clone();
    let settings_ = settings.clone();
    let settings_path_ = settings_path.clone();

    eframe::run_simple_native("My egui App", options, move |ctx, _frame| {
        egui_extras::install_image_loaders(ctx);
        let mut fonts = egui::FontDefinitions::default();
        egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
        ctx.set_fonts(fonts);

        if ctx.input_mut(|i| i.consume_shortcut(&SHORTCUT_QUIT)) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            tree.ui(&mut behavior, ui);
        });

        behavior.handle_events();

        let errors = behavior.errors.load();
        if !errors.is_empty() {
            let modal = egui::Modal::new(egui::Id::new("Errors")).show(ctx, |ui| {
                // TODO: calculate from window size
                ui.set_width(800.0);
                ui.set_height(400.0);

                ui.heading("Errors");
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for err in errors.iter() {
                        ui.collapsing(err.to_string(), |ui| {
                            ui.label(format!("{err:?}"));
                        });
                    }
                });
            });

            if modal.should_close() {
                behavior.errors.discard();
            }
        }

        let dirty = settings_.view(|s| *s != *stored_settings);

        if dirty && debounce < Instant::now() {
            // This will apply/save settings every n seconds *while* or *after* they have been changed
            let settings__ = settings_.clone();
            let settings_path_ = settings_path_.clone();
            debounce = Instant::now() + Duration::from_secs(5);

            log::info!("Settings changed, reloading agent");

            rt_.spawn(async move {
                save_settings(settings__, settings_path_).await;
            });

            stored_settings = settings_.view(|s| Arc::new(s.clone()));
        }

        let running = task_count.load(Ordering::Relaxed) > 0;

        if !running && next_prompt.load().is_some() {
            let prompt = next_prompt
                .swap(Default::default())
                .unwrap()
                .as_ref()
                .to_owned();

            behavior.prompt = prompt;
        }

        let next_workflow = if !running && next_workflow.load().is_some() {
            next_workflow.swap(Default::default())
        } else {
            None
        };

        if let Some(next_workflow) = next_workflow {
            behavior.workflows.switch(&next_workflow);

            let autorun = settings_.view(|s| s.autoruns);
            if behavior.run_count < autorun {
                behavior.run_count += 1;
                behavior.exec_workflow();
            }
        }
    })
    .map_err(|e| anyhow::anyhow!("I can't {e:?}"))?;

    rt.handle().block_on(async move {
        save_settings(settings, settings_path).await;
    });

    Ok(())
}

async fn save_settings(settings: Arc<ArcSwap<Settings>>, settings_path: std::path::PathBuf) {
    use tokio::io::AsyncWriteExt as _;
    let text = settings.view(|s| serde_yml::to_string(s).unwrap());

    let mut file = tokio::fs::File::create(settings_path).await.unwrap();
    file.write_all(text.as_bytes()).await.unwrap();
}
