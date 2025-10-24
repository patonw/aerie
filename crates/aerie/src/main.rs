use eframe::egui;
use egui_commonmark::*;
use egui_tiles::{LinearDir, TileId};
use rmcp::{
    ServiceExt as _,
    model::Tool,
    transport::{ConfigureCommandExt, TokioChildProcess},
};
use std::{
    sync::{Arc, RwLock, atomic::AtomicU16},
    time::{Duration, Instant},
};
use tokio::process::Command;
use tracing_subscriber::{
    Layer as _, filter, layer::SubscriberExt as _, util::SubscriberInitExt as _,
};

use aerie::{
    AgentFactory, LogChannelLayer, LogEntry, Settings,
    ui::{AppBehavior, Pane},
};

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
    let cache = CommonMarkCache::default();
    let prompt = Arc::new(RwLock::new(String::new()));
    let mut debounce = Instant::now() + Duration::from_secs(1);

    let log_history_ = log_history.clone();

    // TODO: clean shutdown
    rt.handle().spawn(async move {
        while let Ok(entry) = log_rx.recv_async().await {
            let mut log_rw = log_history_.write().unwrap();
            log_rw.push(entry);
        }
    });

    let agent_factory = AgentFactory {
        settings: settings.clone(),
        mcp_client: mcp_client.clone(),
        mcp_tools: mcp_tools.clone(),
    };

    let mut tiles = egui_tiles::Tiles::default();
    let tabs: Vec<TileId> = vec![tiles.insert_pane(Pane::Chat), tiles.insert_pane(Pane::Logs)];
    let content_tabs: TileId = tiles.insert_tab_tile(tabs);

    let tabs = vec![
        tiles.insert_pane(Pane::Settings),
        tiles.insert_pane(Pane::Navigator),
    ];
    let inspector_tabs = tiles.insert_tab_tile(tabs);

    let split =
        egui_tiles::Linear::new_binary(LinearDir::Horizontal, [content_tabs, inspector_tabs], 0.75);

    let root = tiles.insert_container(split);

    let mut tree = egui_tiles::Tree::new("my_tree", root, tiles);
    let mut behavior = AppBehavior {
        settings: settings.clone(),
        log_history: log_history.clone(),
        task_count: task_count.clone(),
        scratch: Default::default(),
        session: Default::default(),
        cache,
        prompt: prompt.clone(),
        rt: rt.handle().clone(),
        agent_factory,
        branch_point: None,
        dest_branch: String::new(),
    };

    eframe::run_simple_native("My egui App", options, move |ctx, _frame| {
        let mut fonts = egui::FontDefinitions::default();
        egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
        ctx.set_fonts(fonts);
        egui::CentralPanel::default().show(ctx, |ui| {
            tree.ui(&mut behavior, ui);
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
        }
    })
    .map_err(|e| anyhow::anyhow!("I can't {e:?}"))?;

    Ok(())
}
