use std::{sync::Arc, time::Duration};

use aerie::{
    client::{ActionData, ClientAction, ClientUpdate, UpdateData},
    config::ConfigStateProxy,
    ui::{
        Pane,
        state::{AppData, AppState, WorkflowState},
    },
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

// TODO: figure out how to manage different users
// TODO: Save settings when changed
// TODO: Ensure latest message is shown when autoscrolling

pub struct WebApp {
    sock: Option<(WsSender, WsReceiver)>,
    connected: bool,
    state: AppState,
    node_states: NodeStateMap,
    tile_tree: Tree<Pane>,
}

impl WebApp {
    pub fn new(ctx: &egui::Context, node_states: NodeStateMap) -> Self {
        egui_extras::install_image_loaders(&ctx);
        let mut fonts = egui::FontDefinitions::default();
        egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
        ctx.set_fonts(fonts);

        let data = AppData::builder()
            .prefs(Default::default())
            .session(Default::default())
            .build();

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
            state,
            node_states,
            tile_tree: make_tile_tree(),
        }
    }
}

impl eframe::App for WebApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.style_mut(|style| style.visuals = Visuals::dark());
        let errors = self.state.errors.clone();
        let mut task_update = None;
        if let Some((_tx, rx)) = &mut self.sock {
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
                    }
                    WsEvent::Closed => {
                        self.connected = false;
                    }
                    WsEvent::Message(ewebsock::WsMessage::Text(txt))
                        if let Ok(payload) = serde_json::from_str::<ClientUpdate>(&txt) =>
                    {
                        task_update = payload.task_count;
                        let update_data = payload.update;
                        match update_data {
                            UpdateData::Dump(dump) => {
                                self.state.tools = dump.tools;
                                self.state.data = dump.app_data;
                                self.state.prompt = dump.prompt;
                                self.state
                                    .task_count
                                    .store(dump.task_count, std::sync::atomic::Ordering::Relaxed);
                                self.state.workflows.store.metadata = dump.workflows;
                            }
                            UpdateData::Patch(patch) => {
                                if let Some(tools) = patch.tools {
                                    self.state.tools = tools;
                                }

                                if let Some(app_data) = patch.app_data {
                                    self.state.data = app_data;
                                }

                                if let Some(prompt) = patch.prompt {
                                    if prompt.is_empty() {
                                        self.state.prompt.clear();
                                    }
                                }

                                if let Some(images) = patch.images {
                                    self.state.images.store(Arc::new(images));
                                }

                                if let Some(config_state) = patch.config_state {
                                    errors.distil(
                                        self.state.state.update(|state| *state = config_state),
                                    );
                                }

                                // log::info!("Editor patch? {:?}", &patch.editor);
                                if let Some(editor) = patch.editor {
                                    self.state.workflows.patch(editor);
                                }

                                if let Some(previews) = patch.previews {
                                    self.state.workflows.previews.replace(previews);
                                }

                                if let Some(workflows) = patch.workflows {
                                    self.state.workflows.store.metadata = workflows;
                                }
                            }
                            UpdateData::Switch(data) => {
                                self.state
                                    .workflows
                                    .switch_workflow(&data.name, data.baseline);
                            }
                            UpdateData::NodeState(events) => {
                                for event in events {
                                    errors.distil(self.node_states.handle_event(event));
                                }
                            }
                            UpdateData::Event(ev) => {
                                use aerie::events::ExecEvent::*;
                                use aerie::events::WorkflowEvent::*;

                                let runs = &mut self.state.workflows.outputs;

                                match ev {
                                    Exec(Start(_, name, started)) => {
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
                                        runs.back_mut()
                                            .unwrap()
                                            .duration
                                            .store(Arc::new(Duration::from_millis(millis)));
                                    }
                                    Exec(Output(_, label, value)) => {
                                        runs.back_mut()
                                            .unwrap()
                                            .outputs
                                            .rcu(|it| it.update(label.clone(), value.clone()));
                                    }
                                }
                            }
                        }
                        // ctx.request_repaint();
                    }
                    WsEvent::Message(ewebsock::WsMessage::Text(txt)) => {
                        let jd = &mut serde_json::Deserializer::from_str(&txt);
                        let result: Result<ClientUpdate, _> = serde_path_to_error::deserialize(jd);
                        if let Err(err) = result {
                            log::warn!("This did not parse: {err:?}",);
                        }
                    }
                    WsEvent::Message(ewebsock::WsMessage::Binary(bytes)) => {
                        let data = postcard::from_bytes::<ClientUpdate>(&bytes);
                        tracing::info!("Got binary update: {:?}", data);
                    }
                    unk => {
                        log::warn!("Got unknonwn message: {unk:?}");
                    }
                }
            }
        } else {
            log::info!("Trying to connect");
            self.sock = Some(
                ewebsock::connect("ws://localhost:3000/ws", Default::default())
                    .expect("Could not connect"),
            );

            log::info!("Connected");
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            self.tile_tree.ui(&mut self.state, ui);

            let shadow = self.state.workflows.view_stack.root();
            self.state.workflows.cast_shadow(shadow);
        });

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
            // Hmm, still doesn't help with autoscroll
            self.state
                .task_count
                .store(task_count, std::sync::atomic::Ordering::Relaxed);
        }
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
    use wasm_bindgen_futures::spawn_local;

    // Redirect `log` message to `console.log` and friends:
    eframe::WebLogger::init(log::LevelFilter::Debug).ok();

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
        // tiles.insert_pane(Pane::Tools),
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
