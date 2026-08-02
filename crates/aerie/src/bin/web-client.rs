use eframe::egui;
use ewebsock::{WsEvent, WsReceiver, WsSender};

#[derive(Default)]
pub struct ExampleApp {
    name: String,
    age: u32,
    sock: Option<(WsSender, WsReceiver)>,
    greeting: String,
    dirty: bool,
}

impl ExampleApp {
    pub fn new() -> Self {
        Self {
            name: "Arthur".into(),
            age: 42,
            sock: None,
            greeting: "".into(),
            dirty: false,
        }
    }
}

impl eframe::App for ExampleApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if let Some((tx, rx)) = &mut self.sock {
            while let Some(event) = rx.try_recv() {
                log::info!("New event: {event:?}");
                match event {
                    WsEvent::Opened => {
                        tx.send(ewebsock::WsMessage::Binary(vec![4, 5, 6]));
                    }
                    WsEvent::Message(ewebsock::WsMessage::Text(txt)) => {
                        self.greeting = txt;
                    }
                    _ => {}
                }
            }
        } else {
            log::info!("Trying to connect");
            // self.sock = Some(
            //     ewebsock::connect("ws://localhost:3000/ws", Default::default())
            //         .expect("Could not connect"),
            // );

            log::info!("Connected");
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("My egui Application");
            ui.horizontal(|ui| {
                let name_label = ui.label("You name: ");
                self.dirty |= ui
                    .text_edit_singleline(&mut self.name)
                    .labelled_by(name_label.id)
                    .changed();

                if self.dirty
                    && let Some((tx, _)) = &mut self.sock
                {
                    let payload = serde_json::json!({"name": &self.name});
                    let text = serde_json::to_string(&payload).unwrap();
                    tx.send(ewebsock::WsMessage::Text(text));
                    self.dirty = false;
                }
            });
            self.dirty |= ui
                .add(egui::Slider::new(&mut self.age, 0..=120).text("age"))
                .changed();
            if ui.button("Increment").clicked() {
                self.age += 1;
                self.dirty = true;
            }

            if self.dirty
                && let Some((tx, _)) = &mut self.sock
            {
                let payload = serde_json::json!({"age": &self.age});
                let text = serde_json::to_string(&payload).unwrap();
                tx.send(ewebsock::WsMessage::Text(text));
                self.dirty = false;
            }
            // ui.label(format!("Hello '{}', age {}", self.name, self.age));
            ui.label(&self.greeting);
        });
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
        Box::new(|_cc| Ok(Box::new(ExampleApp::new()))),
    )
    .map_err(|e| anyhow::anyhow!("Problem running app: {e:?}"))?;

    Ok(())
}

// When compiling to web using trunk:
#[cfg(target_arch = "wasm32")]
fn main() {
    use eframe::wasm_bindgen::JsCast as _;

    // Redirect `log` message to `console.log` and friends:
    eframe::WebLogger::init(log::LevelFilter::Debug).ok();

    let web_options = eframe::WebOptions::default();

    wasm_bindgen_futures::spawn_local(async {
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
                Box::new(|_cc| Ok(Box::new(ExampleApp::new()))),
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
