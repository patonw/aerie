use aerie::{
    client::{ActionData, ClientAction, ClientUpdate, UpdateData},
    config::ConfigStateProxy,
    ui::{
        AppState,
        state::{AppData, WorkflowState},
    },
    workflow::store::WorkflowStoreStub,
};
use eframe::egui;
use egui::Visuals;
use ewebsock::{WsEvent, WsReceiver, WsSender};

// TODO: figure out how to manage different users
// TODO: Save settings when changed
// TODO: Ensure latest message is shown when autoscrolling

pub struct WebApp {
    sock: Option<(WsSender, WsReceiver)>,
    dirty: bool,
    state: AppState,
}

impl WebApp {
    pub fn new(ctx: &egui::Context) -> Self {
        let content_rect = ctx.content_rect();
        egui_extras::install_image_loaders(&ctx);
        let mut fonts = egui::FontDefinitions::default();
        egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
        ctx.set_fonts(fonts);

        let data = AppData::builder()
            .prefs(Default::default())
            .session(Default::default())
            .build();

        let workflows = WorkflowState::new(WorkflowStoreStub::default(), None);
        let state = AppState::builder()
            .data(data)
            .workflows(workflows)
            .state(ConfigStateProxy::default())
            .build();

        Self {
            sock: None,
            dirty: false,
            state,
        }
    }
}

impl eframe::App for WebApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.style_mut(|style| style.visuals = Visuals::dark());
        let mut task_update = None;
        if let Some((tx, rx)) = &mut self.sock {
            while let Some(event) = rx.try_recv() {
                log::info!("New event: {event:?}");
                match event {
                    WsEvent::Opened => {
                        // let payload = serde_json::to_string(&ClientAction {
                        //     action: ActionData::Noop("meh".into()),
                        // })
                        // .unwrap();
                        // tx.send(ewebsock::WsMessage::Text(payload));

                        // Expect backend to send initial dump
                        ctx.request_repaint();
                    }
                    WsEvent::Message(ewebsock::WsMessage::Text(txt))
                        if let Ok(payload) = serde_json::from_str::<ClientUpdate>(&txt) =>
                    {
                        log::info!("Got client update");
                        match payload.update {
                            UpdateData::Dump(dump) => {
                                self.state.tools = dump.tools;
                                self.state.data = dump.app_data;
                                self.state.prompt = dump.prompt;
                                self.state
                                    .task_count
                                    .store(dump.task_count, std::sync::atomic::Ordering::Relaxed);
                                self.state.workflows.store.metadata = dump.workflows;
                                log::info!("Updated state from server dump");
                            }
                            UpdateData::Patch(patch) => {
                                task_update = patch.task_count;

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
                                    // Bad UX since delay in round trip
                                    // self.state.prompt = prompt;
                                }

                                if let Some(config_state) = patch.config_state {
                                    self.state.state.update(|state| *state = config_state);
                                }

                                if let Some(workflows) = patch.workflows {
                                    self.state.workflows.store.metadata = workflows;
                                }
                            }
                        }
                        ctx.request_repaint();
                    }
                    WsEvent::Message(ewebsock::WsMessage::Binary(bytes)) => {
                        let data = postcard::from_bytes::<ClientUpdate>(&bytes);
                        log::info!("Got client update");
                    }
                    _ => {}
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
            egui::SidePanel::right("Settings Panel")
                .resizable(true)
                .show_inside(ui, |ui| {
                    self.state.settings_ui(ui);
                    ui.separator();
                });

            egui::CentralPanel::default().show_inside(ui, |ui| {
                self.state.chat_ui(ui);
            });
        });

        let mut events = Vec::new();
        while let Some(event) = self.state.events.pop() {
            events.push(event);
        }

        if !events.is_empty()
            && let Some((tx, rx)) = &mut self.sock
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
            task_update = None;
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn main() -> anyhow::Result<()> {
    println!("Hello, world!");
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

            Ok(Box::new(WebApp::new(&cc.egui_ctx)))
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
                    Ok(Box::new(WebApp::new(&cc.egui_ctx)))
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
