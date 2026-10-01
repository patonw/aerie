use std::{ops::Add, sync::Arc, time::Duration};

use aerie::{
    client::{ActionData, ClientAction, ClientUpdate, StatePatch, UpdateData},
    config::ConfigStateProxy,
    state::{
        app::{AppData, AppState},
        workflow::WorkflowState,
    },
    ui::{Pane, render::display_errors},
    utils::ErrorDistiller as _,
    workflow::{
        runner::{NodeStateMap, WorkflowRun},
        store::WorkflowStoreStub,
    },
};
use eframe::egui;
use egui::Visuals;
use egui_tiles::{LinearDir, TileId, Tree};
use ewebsock::{WsEvent, WsReceiver, WsSender};
use footnote::Footnote;
use struct_patch::{Patch as _, Status};
use typed_builder::TypedBuilder;
use web_time::Instant;

// TODO: figure out how to manage different users
// TODO: Save settings when changed
// TODO: Ensure latest message is shown when autoscrolling

const WATCHDOG_MS: u64 = 10_000;

#[derive(TypedBuilder)]
pub struct WebApp {
    sock: Option<(WsSender, WsReceiver)>,
    connected: bool,
    disconnect: bool,
    ready: bool,
    watchdog: Instant,
    state: AppState,
    checkpoint: StatePatch,
    node_states: NodeStateMap,
    tile_tree: Tree<Pane>,
}

impl WebApp {
    pub fn new(ctx: &egui::Context, node_states: NodeStateMap) -> Self {
        egui_extras::install_image_loaders(ctx);
        let mut fonts = egui::FontDefinitions::default();
        egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
        ctx.set_fonts(fonts);

        let data = AppData::builder()
            .prefs(Default::default())
            .session(Default::default())
            .build();

        let checkpoint = Default::default();

        let workflows = WorkflowState::new(WorkflowStoreStub::default(), None)
            .with_node_state(node_states.clone());
        let state = AppState::builder()
            .data(data)
            .workflows(workflows)
            .state(ConfigStateProxy::default())
            .build();

        Self {
            sock: None,
            connected: false,
            disconnect: false,
            ready: false,
            watchdog: Instant::now().add(Duration::from_millis(WATCHDOG_MS)),
            state,
            checkpoint,
            node_states,
            tile_tree: make_tile_tree(),
        }
    }

    pub fn feed_watchdog(&mut self) {
        self.watchdog = Instant::now().add(Duration::from_millis(WATCHDOG_MS));
    }
}

impl eframe::App for WebApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.style_mut(|style| style.visuals = Visuals::dark());
        let errors = self.state.errors.clone();
        let mut task_update = None;
        let mut feed = false;

        if let Some((tx, rx)) = &mut self.sock {
            while let Some(event) = rx.try_recv() {
                // log::info!("New event: {event:?}");
                match event {
                    WsEvent::Opened => {
                        // let payload = serde_json::to_string(&ClientAction {
                        //     action: ActionData::Noop("meh".into()),
                        // })
                        // .unwrap();
                        // tx.send(ewebsock::WsMessage::Text(payload));

                        // Expect backend to send initial dump
                        self.connected = true;
                        ctx.request_repaint();
                        feed |= true;
                    }
                    WsEvent::Closed => {
                        self.disconnect = true;
                        self.connected = false;
                        self.ready = false;
                    }
                    WsEvent::Message(ewebsock::WsMessage::Text(txt))
                        if let Ok(payload) = serde_json::from_str::<ClientUpdate>(&txt) =>
                    {
                        feed |= true;
                        tracing::trace!("received {} chars", txt.len());
                        task_update = payload.task_count;
                        handle_update(
                            &mut self.state,
                            &mut self.checkpoint,
                            &mut self.node_states,
                            payload.update,
                        );
                    }
                    WsEvent::Message(ewebsock::WsMessage::Text(txt)) => {
                        feed |= true;
                        let jd = &mut serde_json::Deserializer::from_str(&txt);
                        let result: Result<ClientUpdate, _> = serde_path_to_error::deserialize(jd);
                        if let Err(err) = result {
                            log::warn!("This did not parse: {err:?}",);
                        }
                    }
                    WsEvent::Message(ewebsock::WsMessage::Binary(bytes)) => {
                        feed |= true;
                        tracing::trace!("received {} bytes", bytes.len());
                        match rmp_serde::from_slice::<ClientUpdate>(&bytes) {
                            Ok(payload) => {
                                task_update = payload.task_count;
                                let status = handle_update(
                                    &mut self.state,
                                    &mut self.checkpoint,
                                    &mut self.node_states,
                                    payload.update,
                                );

                                if status {
                                    tracing::trace!("#tasks: {:?}", payload.task_count);
                                    let action = ActionData::Checkpoint(self.checkpoint.get_note());

                                    let payload =
                                        serde_json::to_string(&ClientAction { action }).unwrap();
                                    tx.send(ewebsock::WsMessage::Text(payload));
                                }
                            }
                            Err(err) => {
                                tracing::error!(%err);
                            }
                        }
                    }
                    unk => {
                        log::warn!("Got unknown message: {unk:?}");
                    }
                }
            }
        } else if !self.disconnect {
            log::info!("Trying to connect");
            feed |= true;
            self.sock =
                Some(ewebsock::connect("/ws", Default::default()).expect("Could not connect"));
        }

        if feed {
            self.feed_watchdog();
        }

        if self.watchdog < Instant::now() {
            tracing::warn!("Watchdog activated, resetting connection");
            self.sock = None;
        }

        let content_rect = ctx.content_rect();
        egui::CentralPanel::default().show(ctx, |ui| {
            self.tile_tree.ui(&mut self.state, ui);

            let shadow = self.state.workflows.view_stack.root();
            self.state.workflows.cast_shadow(shadow);
        });

        if self.disconnect {
            egui::Modal::new("Disconnected".into()).show(ctx, |ui| {
                ui.set_width(200f32);
                // ui.set_width(250f32.max(content_rect.width() * 0.25));
                // ui.set_height(content_rect.height() * 0.25);

                ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
                    ui.heading("Disconnected");
                    ui.add_space(32.0);

                    if ui.button("reconnect").clicked() {
                        self.disconnect = false;
                    }
                });
            });
        } else if !self.connected {
            egui::Modal::new("Connecting".into()).show(ctx, |ui| {
                ui.set_width(250f32.max(content_rect.width() * 0.25));
                ui.set_height(content_rect.height() * 0.25);
                ui.centered_and_justified(|ui| {
                    ui.heading("Connecting...");
                });
            });
        } else if !self.ready {
            egui::Modal::new("Initializing".into()).show(ctx, |ui| {
                ui.set_width(250f32.max(content_rect.width() * 0.25));
                ui.set_height(content_rect.height() * 0.25);
                ui.centered_and_justified(|ui| {
                    ui.heading("Connected.\nInitializing...");
                });
            });
        }

        // TODO: debounce events
        let mut events = Vec::new();
        while let Some(event) = self.state.events.pop() {
            use aerie::events::AppEvent::*;
            match event {
                // TODO: Nope, need to patch view stack from server
                ev @ EnterSubgraph(node_id) => {
                    self.state.workflows.view_stack.enter(node_id).unwrap();
                    self.state.workflows.viewer = None;

                    events.push(ev);
                }
                ev @ LeaveSubgraph(levels) => {
                    if !self.state.workflows.view_stack.is_empty() {
                        self.state.workflows.view_stack.exit(levels).unwrap();
                        self.state.workflows.viewer = None;

                        log::info!("root id: {:?}", self.state.workflows.view_stack.exec_ids());

                        events.push(ev);
                    }
                }
                // NodesChanged(id, nodes) => {
                //     log::info!("Detected change");
                // }
                // GraphSnapshot(_) => {
                //     log::info!("Blocking graph snapshot");
                // }
                ev => {
                    events.push(ev);
                }
            }
        }

        if !events.is_empty()
            && self.connected
            && let Some((tx, _rx)) = &mut self.sock
        {
            let action = ActionData::Dispatch(events);

            let payload = serde_json::to_string(&ClientAction { action }).unwrap();
            tx.send(ewebsock::WsMessage::Text(payload));
        }

        if let Some(task_count) = task_update {
            if self.connected && task_count == 0 {
                self.ready = true;
            }

            // Hmm, still doesn't help with autoscroll
            self.state
                .task_count
                .store(task_count, std::sync::atomic::Ordering::Relaxed);
        }

        display_errors(ctx, content_rect, errors, self.state.events.clone());
    }
}

fn handle_update(
    app_state: &mut AppState,
    checkpoint: &mut StatePatch,
    node_states: &mut NodeStateMap,
    update_data: UpdateData,
) -> bool {
    let errors = app_state.errors.clone();
    match update_data {
        UpdateData::Poke => {}
        UpdateData::Dump(dump) => {
            app_state.tools = dump.tools;
            app_state.data = dump.app_data;
            app_state.prompt = dump.prompt;
            app_state
                .task_count
                .store(dump.task_count, std::sync::atomic::Ordering::Relaxed);
            app_state.workflows.store.metadata = dump.workflows;
        }
        UpdateData::Patch(patch) => {
            patch_state(app_state, patch);
            return true;
        }
        UpdateData::Delta(delta) => {
            let status = !delta.is_empty();
            checkpoint.apply(delta);
            patch_state(app_state, checkpoint.clone());
            return status;
        }
        UpdateData::Switch(data) => {
            app_state
                .workflows
                .switch_workflow(&data.name, data.baseline);
        }
        UpdateData::NodeState(events) => {
            for event in events {
                errors.distil(node_states.handle_event(event));
            }
        }
        UpdateData::Event(evs) => {
            use aerie::events::ExecEvent::*;
            use aerie::events::WorkflowEvent::*;

            for ev in evs {
                match ev {
                    Exec(Start(_, name, started)) => {
                        let runs = &mut app_state.workflows.outputs;
                        let entry = WorkflowRun::builder()
                            .started(started)
                            .duration(Default::default())
                            .workflow(name)
                            .outputs(Default::default())
                            .build();

                        runs.push_back(entry);
                        if runs.len() > 128 {
                            *runs = runs.skip(runs.len() - 128);
                        }
                    }
                    Exec(Tick(_, millis)) | Exec(Finish(_, millis)) => {
                        let runs = &mut app_state.workflows.outputs;
                        runs.back_mut()
                            .unwrap()
                            .duration
                            .store(Arc::new(Duration::from_millis(millis)));
                    }
                    Exec(Output(_, label, value)) => {
                        let runs = &mut app_state.workflows.outputs;
                        runs.back_mut()
                            .unwrap()
                            .outputs
                            .rcu(|it| it.update(label.clone(), value.clone()));
                    }
                    Progress(ev) => {
                        app_state.workflows.handle_progress(&ev);
                    }
                }
            }
        }
    }

    false
}

fn patch_state(app_state: &mut AppState, patch: StatePatch) {
    let errors = app_state.errors.clone();
    if let Some(tools) = patch.tools {
        app_state.tools = tools;
    }

    app_state.data = patch.app_data;
    app_state.data.prefs.store(Arc::new(patch.prefs));

    if let Some(prompt) = patch.prompt
        && prompt.is_empty()
    {
        app_state.prompt.clear();
    }

    if let Some(images) = patch.images {
        app_state.images.store(Arc::new(images));
    }

    if let Some(config_state) = patch.config_state {
        errors.distil(app_state.state.update(|state| *state = config_state));
    }

    app_state.workflows.patch(patch.editor);

    if let Some(previews) = patch.previews {
        app_state.workflows.previews.replace(previews);
    }

    if let Some(errors) = patch.errors {
        app_state.errors.discard();
        for err in errors.into_iter().map(|e| anyhow::anyhow!(e)) {
            app_state.errors.push(err);
        }
    }

    if let Some(workflows) = patch.workflows {
        app_state.workflows.store.metadata = workflows;
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn main() -> anyhow::Result<()> {
    env_logger::init(); // Log to stderr (if you run with `RUST_LOG=debug`).

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([320.0, 240.0]),
        ..Default::default()
    };
    eframe::run_native(
        "MyApp",
        native_options,
        Box::new(|cc| {
            let ctx = cc.egui_ctx.clone();
            std::thread::spawn(move || {
                // Periodic refresh even when not focused

                use std::time::Duration;
                loop {
                    std::thread::sleep(Duration::from_secs(1));
                    ctx.request_repaint();
                }
            });
            let node_states = NodeStateMap::from_capacity(64);

            Ok(Box::new(WebApp::new(&cc.egui_ctx, node_states)))
        }),
    )
    .map_err(|e| anyhow::anyhow!("Problem running app: {e:?}"))?;

    Ok(())
}

// When compiling to web using trunk:
#[cfg(target_arch = "wasm32")]
fn main() {
    use eframe::egui::{Style, Visuals};
    use eframe::wasm_bindgen::JsCast as _;
    use itertools::Itertools;
    use wasm_bindgen_futures::spawn_local;

    let opts = web_sys::window()
        .map(|w| w.location())
        .and_then(|loc| loc.hash().ok())
        .map(|s| {
            s.trim_start_matches('#')
                .split(",s")
                .map(|s| s.trim().to_lowercase())
                .collect_vec()
        });

    web_sys::console::log_1(&format!("{opts:?}").into());

    let level = opts
        .and_then(|v| {
            v.iter()
                .filter_map(|it| match it.as_str() {
                    "console=trace" => Some(log::LevelFilter::Trace),
                    "console=debug" => Some(log::LevelFilter::Debug),
                    _ => None,
                })
                .next()
        })
        .unwrap_or(log::LevelFilter::Info);

    // Redirect `log` message to `console.log` and friends:
    eframe::WebLogger::init(level).ok();

    let web_options = eframe::WebOptions::default();

    let node_states = NodeStateMap::from_capacity(64);

    spawn_local(async {
        let document = web_sys::window()
            .expect("No window")
            .document()
            .expect("No document");

        let canvas = document
            .get_element_by_id("the_canvas_id")
            .expect("Failed to find the_canvas_id")
            .dyn_into::<web_sys::HtmlCanvasElement>()
            .expect("the_canvas_id was not a HtmlCanvasElement");

        let start_result = eframe::WebRunner::new()
            .start(
                canvas,
                web_options,
                Box::new(|cc| {
                    use gloo_timers::callback::Interval;

                    let style = Style {
                        visuals: Visuals::dark(),
                        ..Style::default()
                    };
                    cc.egui_ctx.set_style(style);

                    let ctx = cc.egui_ctx.clone();

                    // We want to do stuff every second, indefinitely.
                    Interval::new(1_000, move || {
                        ctx.request_repaint();
                    })
                    .forget();
                    Ok(Box::new(WebApp::new(&cc.egui_ctx, node_states)))
                }),
            )
            .await;

        // Remove the loading text and spinner:
        if let Some(loading_text) = document.get_element_by_id("loading_text") {
            match start_result {
                Ok(_) => {
                    loading_text.remove();
                }
                Err(e) => {
                    loading_text.set_inner_html(
                        "<p> The app has crashed. See the developer console for details. </p>",
                    );
                    panic!("Failed to start eframe: {e:?}");
                }
            }
        }
    });
}

fn make_tile_tree() -> Tree<Pane> {
    let mut tiles = egui_tiles::Tiles::default();
    let tabs: Vec<TileId> = vec![
        tiles.insert_pane(Pane::Workflow),
        tiles.insert_pane(Pane::Chat),
        // tiles.insert_pane(Pane::Logs),
        // tiles.insert_pane(Pane::Messages),
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

    egui_tiles::Tree::new("my_tree", root, tiles)
}
