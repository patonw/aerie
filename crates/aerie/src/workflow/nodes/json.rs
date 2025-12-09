use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::workflow::{
    DynNode, EditContext, RunContext, UiNode, Value, WorkflowError,
    nodes::{MIN_HEIGHT, MIN_WIDTH},
};

use super::ValueKind;

#[derive(Debug, Clone, Default, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParseJson {
    text: String,

    #[serde(skip)]
    value: Arc<serde_json::Value>,
}

impl DynNode for ParseJson {
    fn in_kinds(&self, in_pin: usize) -> &'static [ValueKind] {
        match in_pin {
            0 => &[ValueKind::Text],
            _ => unreachable!(),
        }
    }

    fn out_kind(&self, out_pin: usize) -> ValueKind {
        match out_pin {
            0 => ValueKind::Json,
            _ => unreachable!(),
        }
    }

    fn value(&self, out_pin: usize) -> Value {
        match out_pin {
            0 => super::Value::Json(self.value.clone()),
            _ => unreachable!(),
        }
    }
}

impl UiNode for ParseJson {
    fn title(&self) -> &str {
        "Parse JSON"
    }

    fn show_input(
        &mut self,
        ui: &mut egui::Ui,
        _ctx: &EditContext,
        pin_id: usize,
        remote: Option<Value>,
    ) -> egui_snarl::ui::PinInfo {
        match pin_id {
            0 => {
                if remote.is_none() {
                    egui::Resize::default()
                        .id_salt("prompt_resize")
                        .min_width(MIN_WIDTH)
                        .min_height(MIN_HEIGHT)
                        .with_stroke(false)
                        .show(ui, |ui| {
                            egui::ScrollArea::vertical().show(ui, |ui| {
                                let widget = egui::TextEdit::multiline(&mut self.text)
                                    .id_salt("json text")
                                    .desired_width(f32::INFINITY);

                                ui.add_sized(ui.available_size(), widget)
                                    .on_hover_text("JSON text");
                            });
                        });
                } else {
                    ui.label("JSON text");
                }
            }
            _ => unreachable!(),
        };

        self.in_kinds(pin_id).first().unwrap().default_pin()
    }
}

impl ParseJson {
    pub async fn forward(
        &mut self,
        _run_ctx: &RunContext,
        inputs: Vec<Option<Value>>,
    ) -> Result<(), WorkflowError> {
        self.validate(&inputs)?;

        let text = match &inputs[0] {
            Some(Value::Text(text)) => text.as_str(),
            None => self.text.as_str(),
            _ => unreachable!(),
        };

        let value = serde_json::from_str::<serde_json::Value>(text)
            .map_err(|e| WorkflowError::Input(vec![format!("Invalid JSON: {e:?}")]))?;

        self.value = Arc::new(value);

        Ok(())
    }
}

#[derive(Debug, Clone, Default, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidateJson {
    // schema_text: String,
    #[serde(skip)]
    value: Arc<serde_json::Value>,
}

impl DynNode for ValidateJson {
    fn inputs(&self) -> usize {
        2
    }

    fn in_kinds(&self, in_pin: usize) -> &'static [ValueKind] {
        match in_pin {
            0 => &[ValueKind::Json],
            1 => &[ValueKind::Json],
            _ => unreachable!(),
        }
    }

    fn out_kind(&self, _out_pin: usize) -> ValueKind {
        ValueKind::Json
    }

    fn value(&self, _out_pin: usize) -> super::Value {
        super::Value::Json(self.value.clone())
    }
}

impl UiNode for ValidateJson {
    fn title(&self) -> &str {
        "Validate JSON"
    }

    fn tooltip(&self) -> &str {
        "Validates a JSON value against a JSON Schema (as a JSON object).\n\
            If the value is valid it is passed through to the output."
    }

    fn help_link(&self) -> &str {
        "https://json-schema.org/understanding-json-schema/reference"
    }

    fn show_input(
        &mut self,
        ui: &mut egui::Ui,
        _ctx: &EditContext,
        pin_id: usize,
        _remote: Option<Value>,
    ) -> egui_snarl::ui::PinInfo {
        match pin_id {
            0 => {
                ui.label("schema");
            }
            1 => {
                ui.label("json");
            }
            _ => unreachable!(),
        };

        self.in_kinds(pin_id).first().unwrap().default_pin()
    }
}

impl ValidateJson {
    pub async fn forward(
        &mut self,
        _run_ctx: &RunContext,
        inputs: Vec<Option<Value>>,
    ) -> Result<(), WorkflowError> {
        self.validate(&inputs)?;
        self.value = Arc::new(serde_json::Value::Null);

        let schema = match &inputs[0] {
            Some(Value::Json(schema)) => schema.as_ref().to_owned(),
            None => Err(WorkflowError::Input(vec!["Schema is required".into()]))?,
            _ => unreachable!(),
        };

        let input = match &inputs[1] {
            Some(Value::Json(input)) => input.as_ref().to_owned(),
            None => Err(WorkflowError::Input(vec!["JSON input required".into()]))?,
            _ => unreachable!(),
        };

        jsonschema::validate(&schema, &input)
            .map_err(|err| anyhow::anyhow!("Validation error: {err:?}"))?;

        self.value = Arc::new(input);

        Ok(())
    }
}

#[derive(Debug, Clone, Default, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransformJson {
    filter: String,

    #[serde(skip)]
    value: Arc<serde_json::Value>,
}

impl DynNode for TransformJson {
    fn inputs(&self) -> usize {
        2
    }

    fn in_kinds(&self, in_pin: usize) -> &'static [ValueKind] {
        match in_pin {
            0 => &[ValueKind::Text],
            1 => &[ValueKind::Json],
            _ => unreachable!(),
        }
    }

    fn out_kind(&self, _out_pin: usize) -> ValueKind {
        ValueKind::Json
    }

    fn value(&self, _out_pin: usize) -> super::Value {
        super::Value::Json(self.value.clone())
    }
}

impl UiNode for TransformJson {
    fn title(&self) -> &str {
        "Transform JSON"
    }

    fn tooltip(&self) -> &str {
        "Transform a JSON value using jq/jaq filters."
    }

    fn help_link(&self) -> &str {
        "https://gedenkt.at/jaq/manual/#corelang"
    }

    fn show_input(
        &mut self,
        ui: &mut egui::Ui,
        _ctx: &EditContext,
        pin_id: usize,
        remote: Option<Value>,
    ) -> egui_snarl::ui::PinInfo {
        match pin_id {
            0 => {
                if remote.is_none() {
                    egui::Frame::new().inner_margin(4).show(ui, |ui| {
                        egui::Resize::default()
                            .min_width(MIN_WIDTH)
                            .min_height(MIN_HEIGHT)
                            .show(ui, |ui| {
                                egui::ScrollArea::vertical().show(ui, |ui| {
                                    let widget = egui::TextEdit::multiline(&mut self.filter)
                                        .desired_width(f32::INFINITY)
                                        .hint_text("Fitler\u{1F64B}");

                                    ui.add_sized(ui.available_size(), widget);
                                });
                            });
                    });
                } else {
                    ui.label("filter");
                }
            }
            1 => {
                ui.label("json");
            }
            _ => unreachable!(),
        };

        self.in_kinds(pin_id).first().unwrap().default_pin()
    }
}

impl TransformJson {
    pub async fn forward(
        &mut self,
        run_ctx: &RunContext,
        inputs: Vec<Option<Value>>,
    ) -> Result<(), WorkflowError> {
        self.validate(&inputs)?;

        let filter = match &inputs[0] {
            Some(Value::Text(text)) => text.clone(),
            None => self.filter.clone(),
            _ => unreachable!(),
        };

        let filter = run_ctx.transmuter.init_filter(&filter)?;

        let input = match &inputs[1] {
            Some(Value::Json(input)) => input.as_ref().to_owned(),
            None => Err(WorkflowError::Input(vec!["JSON input required".into()]))?,
            _ => unreachable!(),
        };

        let value = run_ctx.transmuter.run_filter(&filter, input)?;

        self.value = Arc::new(value);

        Ok(())
    }
}
