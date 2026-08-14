use typed_builder::TypedBuilder;

use arc_swap::{ArcSwap, ArcSwapOption};
use eframe::{NativeOptions, egui};
use egui_commonmark::*;
use egui_tiles::{LinearDir, TileId, Tree};
use std::{
    convert::identity,
    path::{Path, PathBuf},
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicU16, Ordering},
    },
    time::{Duration, Instant},
};
use tracing_subscriber::{
    Layer as _, filter, layer::SubscriberExt as _, util::SubscriberInitExt as _,
};

use crate::{
    agent::AgentFactory,
    chat::ChatSession,
    config::{Args, ConfigExt, ConfigStateStore, Preferences},
    events::{AppEvent, AppEvents},
    logging::{LogChannelLayer, LogEntry},
    storage::CachedDirStore as _,
    toolbox::ToolStore,
    ui::{
        Pane,
        shortcuts::SHORTCUT_QUIT,
        state::{AppData, AppState, WorkflowState},
    },
    utils::{ErrorDistiller as _, ErrorList, MAX_IMAGE_DIM, prune_image_cache},
    workflow::store::WorkflowStoreDir,
};

// TODO: emit an App that impl eframe::App
// also omit the customizers

// A bunch of hooks instead of sensible refactoring.
// Not sure how customizable this will be in practice yet.
#[derive(TypedBuilder)]
pub struct AppFactory {
    name: &'static str,

    args: Args,

    #[cfg(feature = "native")]
    pub rt: tokio::runtime::Handle,

    #[builder(default, setter(strip_option))]
    settings_path: Option<std::path::PathBuf>,

    #[builder(default, setter(strip_option))]
    state_path: Option<std::path::PathBuf>,

    #[builder(default, setter(strip_option))]
    min_size: Option<egui::Vec2>,

    #[builder(default, setter(strip_option))]
    max_size: Option<egui::Vec2>,

    /// Initialize tracing_subscriber to capture log messages.
    /// Values sent to the flume channel will display in the Logs tab.
    #[builder(default=Rc::new(init_logging))]
    logging_init: Rc<dyn Fn(flume::Sender<LogEntry>)>,

    /// Transform the application settings
    #[builder(default=Rc::new(identity))]
    settings_fn: Rc<dyn Fn(Preferences) -> Preferences>,

    /// Transform the data directory where sessions, workflows and backups are stored
    #[builder(default=Rc::new(identity))]
    data_dir_fn: Rc<dyn Fn(PathBuf) -> PathBuf>,

    /// Transform the agent factory which augments LLM models with tools
    #[builder(default=Rc::new(identity))]
    agent_factory_fn: Rc<dyn Fn(AgentFactory) -> AgentFactory>,

    /// Transform the initial application state
    #[builder(default=Rc::new(identity))]
    appstate_fn: Rc<dyn Fn(AppState) -> AppState>,

    /// Transform the intial workflow store
    #[builder(default=Rc::new(identity))]
    workstore_fn: Rc<dyn Fn(WorkflowStoreDir) -> WorkflowStoreDir>,

    /// Transform the initial session
    #[builder(default=Rc::new(identity))]
    session_fn: Rc<dyn Fn(ChatSession) -> ChatSession>,

    /// Override the eframe window options
    #[builder(default=Rc::new(identity))]
    eframe_opts_fn: Rc<dyn Fn(NativeOptions) -> NativeOptions>,

    /// Customize the initial UI tab tree structure
    #[builder(default=Rc::new(identity))]
    ui_tree_fn: Rc<dyn Fn(Tree<Pane>) -> Tree<Pane>>,
}

#[derive(TypedBuilder)]
pub struct App {
    pub settings_path: std::path::PathBuf,

    pub app_state: AppState,

    pub tile_tree: Tree<Pane>,

    pub stored_settings: Arc<Preferences>,

    #[builder(default=Instant::now() + Duration::from_secs(1))]
    debounce: Instant,

    #[builder(default)]
    min_size: Option<egui::Vec2>,

    #[builder(default)]
    max_size: Option<egui::Vec2>,

    #[builder(default=egui::Rect::from_pos(egui::pos2(200.0, 200.0)))]
    max_rect: egui::Rect,

    #[builder(default)]
    pub next_workflow: Arc<ArcSwapOption<String>>,

    #[builder(default)]
    pub next_prompt: Arc<ArcSwapOption<String>>,

    #[builder(default)]
    pub next_images: Arc<ArcSwap<Vec<String>>>,

    #[builder(default)]
    pub task_count: Arc<AtomicU16>,
}

impl App {
    async fn save_settings(settings: Arc<ArcSwap<Preferences>>, settings_path: impl AsRef<Path>) {
        use tokio::io::AsyncWriteExt as _;

        let text = settings.view(|s| toml::to_string(s).unwrap());

        let mut file = tokio::fs::File::create(settings_path).await.unwrap();
        file.write_all(text.as_bytes()).await.unwrap();
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let next_workflow: Arc<ArcSwapOption<String>> = self.next_workflow.clone();
        let next_prompt: Arc<ArcSwapOption<String>> = self.next_prompt.clone();
        let next_images: Arc<ArcSwap<Vec<String>>> = self.next_images.clone();
        let task_count = self.task_count.clone();

        let rt_ = self.app_state.rt.clone();

        let settings_ = self.app_state.data.prefs.clone();

        let content_rect = ctx.content_rect();

        egui::CentralPanel::default().show(ctx, |ui| {
            if let Some(size) = self.min_size {
                ui.set_min_size(size);
            }

            if let Some(size) = self.max_size {
                ui.set_max_size(size);
            }

            self.tile_tree.ui(&mut self.app_state, ui);

            self.max_rect = ui.min_rect();
        });

        self.app_state.handle_events();

        if ctx.input_mut(|i| i.consume_shortcut(&SHORTCUT_QUIT)) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }

        let errors = self.app_state.errors.load();
        if !errors.is_empty() {
            let modal = egui::Modal::new(egui::Id::new("Errors")).show(ctx, |ui| {
                ui.set_max_width(content_rect.width() * 0.8);
                ui.set_min_height(content_rect.height() * 0.5);
                ui.heading("Errors");
                egui::ScrollArea::both()
                    .auto_shrink(egui::Vec2b::new(true, false))
                    .show(ui, |ui| {
                        for (i, err) in errors.iter().enumerate() {
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
                self.app_state.errors.discard();
            }
        }

        let dirty = settings_.view(|s| *s != *self.stored_settings);

        if dirty && self.debounce < Instant::now() {
            // This will apply/save settings every n seconds *while* or *after* they have been changed
            let settings__ = settings_.clone();
            let settings_path_ = self.settings_path.clone();
            self.debounce = Instant::now() + Duration::from_secs(5);

            log::info!("Settings changed, reloading agent");

            rt_.spawn(async move {
                Self::save_settings(settings__, settings_path_).await;
            });

            self.stored_settings = settings_.view(|s| Arc::new(s.clone()));
        }

        let running = task_count.load(Ordering::Relaxed) > 0;

        if !running && next_prompt.load().is_some() {
            let prompt = next_prompt
                .swap(Default::default())
                .unwrap()
                .as_ref()
                .to_owned();

            self.app_state.prompt = prompt;
            self.app_state.images = next_images.swap(Default::default()).as_ref().clone();
        }

        let next_workflow = if !running && next_workflow.load().is_some() {
            next_workflow.swap(Default::default())
        } else {
            None
        };

        if let Some(next_workflow) = next_workflow {
            self.app_state.workflows.switch(&next_workflow);

            let autorun = settings_.view(|s| s.autoruns);
            if self.app_state.run_count < autorun {
                self.app_state.run_count += 1;

                let exec_id = self.app_state.workflows.shadow.graph.uuid.into();
                self.app_state.workflows.node_state.reset(exec_id);

                self.app_state.exec_workflow();
            }
        }

        prune_image_cache(ctx);
    }
}

impl AppFactory {
    pub fn make_app(&self) -> anyhow::Result<App> {
        let app_state = self.build_state()?;
        let tile_tree = self.make_tile_tree();
        let settings_path = self.resolve_settings();
        let settings_ = app_state.data.prefs.clone();
        let stored_settings = settings_.load_full();

        Ok(App::builder()
            .app_state(app_state)
            .tile_tree(tile_tree)
            .min_size(self.min_size)
            .max_size(self.max_size)
            .settings_path(settings_path)
            .stored_settings(stored_settings)
            .build())
    }

    pub fn run_ui(&self) -> Result<(), anyhow::Error> {
        let app = self.make_app().unwrap();
        eframe::run_native(
            "aerie",
            self.ui_options(),
            Box::new(move |cc| {
                let ctx = &cc.egui_ctx;
                egui_extras::install_image_loaders(ctx);
                let mut fonts = egui::FontDefinitions::default();
                egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
                ctx.set_fonts(fonts);

                Ok(Box::new(app))
            }),
        )
        .unwrap();

        Ok(())
    }

    fn ui_options(&self) -> NativeOptions {
        (self.eframe_opts_fn)(eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_maximized(true) // Otherwise, panics on windows
                .with_resizable(true)
                .with_drag_and_drop(true),
            ..Default::default()
        })
    }

    fn make_tile_tree(&self) -> Tree<Pane> {
        let mut tiles = egui_tiles::Tiles::default();
        let tabs: Vec<TileId> = vec![
            tiles.insert_pane(Pane::Workflow),
            tiles.insert_pane(Pane::Chat),
            // tiles.insert_pane(Pane::Logs),
            tiles.insert_pane(Pane::Messages),
        ];

        let content_tabs: TileId = tiles.insert_tab_tile(tabs);
        let tabs = vec![
            tiles.insert_pane(Pane::Tools),
            tiles.insert_pane(Pane::Navigator),
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

        (self.ui_tree_fn)(egui_tiles::Tree::new("my_tree", root, tiles))
    }

    fn resolve_settings(&self) -> PathBuf {
        if let Some(path) = &self.settings_path {
            path.clone()
        } else {
            self.args.config.clone().unwrap_or(
                dirs::config_dir()
                    .map(|p| p.join("aerie"))
                    .unwrap_or_default()
                    .join("settings.toml"),
            )
        }
    }

    #[tracing::instrument(skip_all)]
    pub fn build_state(&self) -> Result<AppState, anyhow::Error> {
        let next_workflow: Arc<ArcSwapOption<String>> = Default::default();
        let next_prompt: Arc<ArcSwapOption<String>> = Default::default();
        let next_images: Arc<ArcSwap<Vec<String>>> = Default::default();
        let task_count: Arc<AtomicU16> = Default::default();
        let settings_path = self.resolve_settings();
        let state_path = if let Some(path) = &self.state_path {
            path.clone()
        } else {
            self.args.config.clone().unwrap_or(
                dirs::state_dir()
                    .map(|p| p.join("aerie"))
                    .unwrap_or_default()
                    .join("config.sled"),
            )
        };
        let data_dir = (self.data_dir_fn)(
            dirs::data_dir()
                .unwrap_or(".data/share".into())
                .join(self.name),
        );
        let session_dir = self
            .args
            .session_dir
            .clone()
            .unwrap_or(data_dir.join("sessions"));
        let workflow_dir = self
            .args
            .workflow_dir
            .clone()
            .unwrap_or(data_dir.join("workflows"));
        let tool_dir = self.args.tool_dir.clone().unwrap_or(data_dir.join("tools"));
        std::fs::create_dir_all(settings_path.parent().unwrap())?;
        std::fs::create_dir_all(state_path.parent().unwrap())?;
        std::fs::create_dir_all(&session_dir)?;
        std::fs::create_dir_all(&workflow_dir)?;
        std::fs::create_dir_all(&tool_dir)?;

        let (log_tx, log_rx) = flume::unbounded::<LogEntry>();
        (self.logging_init)(log_tx);

        let rt = self.rt.clone();
        let preferences = if settings_path.is_file() {
            let text = std::fs::read_to_string(settings_path)?;
            toml::from_str(&text)?
        } else {
            Preferences::default()
        };
        let preferences = (self.settings_fn)(preferences);
        let config_state = ConfigStateStore::open(&state_path)?;
        if let Some(image_size) = preferences.image_size {
            MAX_IMAGE_DIM.store(image_size, Ordering::Relaxed);
        }
        let session_name = self
            .args
            .session
            .as_deref()
            .unwrap_or(config_state.session.as_str());
        let session_name = Some(session_name).filter(|s| !s.is_empty());
        let session =
            (self.session_fn)(ChatSession::from_dir_name(session_dir, session_name).build()?);
        let preferences = Arc::new(ArcSwap::from_pointee(preferences));
        let log_history: Arc<arc_swap::ArcSwapAny<Arc<im::Vector<LogEntry>>>> =
            Arc::new(ArcSwap::from_pointee(im::Vector::<LogEntry>::new()));
        let cache = CommonMarkCache::default();
        let log_history_ = log_history.clone();
        rt.spawn(async move {
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

        let flow_store = (self.workstore_fn)(WorkflowStoreDir::init(workflow_dir, true)?);
        let flow_state =
            WorkflowState::new(flow_store.clone(), Some(config_state.workflow.clone()));
        let tool_store = ToolStore::new(tool_dir);
        tool_store.preload_all();
        let events: Arc<AppEvents> = Default::default();
        let errors: ErrorList<anyhow::Error> = Default::default();
        let agent_factory = (self.agent_factory_fn)(
            AgentFactory::builder()
                .rt(rt.clone())
                .prefs(preferences.clone())
                .tools(Some(tool_store.clone()))
                .errors(errors.clone())
                .task_count(task_count.clone())
                .store(Some(flow_store.clone()))
                .next_workflow(next_workflow.clone())
                .next_prompt(next_prompt.clone())
                .next_images(next_images.clone())
                .build(),
        );
        rt.spawn({
            let that = agent_factory.clone();
            let errors = errors.clone();
            let events = events.clone();

            async move {
                let results = that.load_tools().await;
                for result in results {
                    errors.distil(result);
                }

                events.insert(AppEvent::ToolsChanged);
            }
        });
        let session_names = session.list();
        let data = AppData::builder()
            .prefs(preferences.clone())
            .session(session)
            .session_names(session_names)
            .build();
        let app_state = (self.appstate_fn)(
            AppState::builder()
                .data(data)
                .state(config_state)
                .tools(tool_store)
                .log_history(log_history.clone())
                .task_count(task_count.clone())
                .events(events.clone())
                .errors(errors.clone())
                .cache(cache)
                .rt(rt.clone())
                .agent_factory(agent_factory)
                .workflows(flow_state)
                .build(),
        );
        Ok(app_state)
    }
}

fn init_logging(log_tx: flume::Sender<LogEntry>) {
    // Can't figure out how to write the signature for a hook, here
    // Making the entire function overridable instead
    tracing_subscriber::registry()
        .with(
            LogChannelLayer(log_tx)
                // .with_filter(filter::LevelFilter::DEBUG)
                .with_filter(filter::filter_fn(|metadata| {
                    metadata.target().starts_with("rig")
                })),
        )
        .with(
            tracing_subscriber::fmt::layer()
                .with_thread_names(true)
                .with_filter(
                    tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                        format!("info,{}=warn", env!("CARGO_CRATE_NAME")).into()
                    }),
                ),
        )
        .init();
}
