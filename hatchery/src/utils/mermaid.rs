use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = ["window", "mermaid"])]
    pub fn contentLoaded();

    #[wasm_bindgen(js_namespace = ["window", "mermaid"])]
    pub fn run();

    #[wasm_bindgen(js_name = "run", js_namespace = ["window", "mermaid"])]
    pub fn render(runOpts: &JsValue);
}
