use std::{borrow::Cow, sync::Arc};

use itertools::Itertools;
use serde::{Deserialize, Serialize};
use serde_json::json;
use serde_with::skip_serializing_none;

use crate::{
    ui::{
        resizable_frame,
        shortcuts::{Shortcut, squelch},
    },
    utils::{extract_json, message_text},
    workflow::{
        DynNode, EditContext, FlexNode, RunContext, UiNode, Value, WorkNode, WorkflowError,
        nodes::GraphSubmenu,
    },
};

use super::ValueKind;

#[skip_serializing_none]
#[derive(Debug, Clone, Default, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParseJson {
    text: String,

    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    extract: bool,

    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    as_array: bool,

    size: Option<crate::utils::EVec2>,
}

#[typetag::serde]
impl FlexNode for ParseJson {}

impl DynNode for ParseJson {
    fn in_kinds(&'_ self, in_pin: usize) -> Cow<'_, [ValueKind]> {
        Cow::Borrowed(match in_pin {
            0 => &[ValueKind::Text],
            _ => unreachable!(),
        })
    }

    fn outputs(&self) -> usize {
        2
    }

    fn out_kind(&self, out_pin: usize) -> ValueKind {
        match out_pin {
            0 => ValueKind::Json,
            1 => ValueKind::Failure,
            _ => unreachable!(),
        }
    }

    fn execute(
        &mut self,
        _ctx: &RunContext,
        _node_id: egui_snarl::NodeId,
        inputs: Vec<Option<Value>>,
    ) -> Result<Vec<Value>, WorkflowError> {
        self.validate(&inputs)?;

        let text = match &inputs[0] {
            Some(Value::Text(text)) => text.as_str(),
            None => self.text.as_str(),
            _ => unreachable!(),
        };

        let result = serde_json::from_str::<serde_json::Value>(text);
        let value = match result {
            Ok(value) => value,
            Err(_) if self.extract => {
                extract_json(text, self.as_array).ok_or(WorkflowError::Conversion(format!(
                    "Could not find a JSON {} inside text",
                    if self.as_array { "array" } else { "object" }
                )))?
            }
            Err(_) => {
                result.map_err(|e| WorkflowError::Conversion(format!("Invalid JSON: {e:?}")))?
            }
        };

        let value = Arc::new(value);

        Ok(vec![
            Value::Json(value),
            Value::Placeholder(ValueKind::Failure),
        ])
    }
}

fn json_editor(
    ui: &mut egui::Ui,
    buffer: &mut dyn egui::TextBuffer,
    hint: Option<&str>,
) -> egui::Response {
    let theme = egui_extras::syntax_highlighting::CodeTheme::from_memory(ui.ctx(), ui.style());
    let mut layouter = |ui: &egui::Ui, buf: &dyn egui::TextBuffer, wrap_width: f32| {
        let mut layout_job = egui_extras::syntax_highlighting::highlight(
            ui.ctx(),
            ui.style(),
            &theme,
            buf.as_str(),
            "json",
        );
        layout_job.wrap.max_width = wrap_width;
        ui.fonts_mut(|f| f.layout_job(layout_job))
    };

    let mut widget = egui::TextEdit::multiline(buffer)
        .id_salt("json text")
        .font(egui::TextStyle::Monospace) // for cursor height
        .code_editor()
        .desired_rows(10)
        .lock_focus(true)
        .desired_width(f32::INFINITY)
        .layouter(&mut layouter);

    let hover = if let Some(hint) = hint {
        widget = widget.id_salt(hint).hint_text(hint);
        hint
    } else {
        "JSON"
    };

    let resp = ui
        .add_sized(ui.available_size(), widget)
        .on_hover_text(hover);

    if resp.has_focus()
        && resp
            .ctx
            .input_mut(|i| i.consume_shortcut(&Shortcut::FormatCode.key()))
        && let Ok(text) = serde_json::from_str::<serde_json::Value>(buffer.as_str())
            .and_then(|v| serde_json::to_string_pretty(&v))
    {
        buffer.replace_with(text.as_str());
    }

    squelch(resp)
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
                    resizable_frame(&mut self.size, ui, |ui| {
                        json_editor(ui, &mut self.text, None);
                    });
                } else {
                    ui.label("JSON text");
                }
            }
            _ => unreachable!(),
        };

        self.in_kinds(pin_id).first().unwrap().default_pin()
    }

    fn show_output(
        &mut self,
        ui: &mut egui::Ui,
        _ctx: &EditContext,
        pin_id: usize,
    ) -> egui_snarl::ui::PinInfo {
        match pin_id {
            0 => {
                ui.label("json");
            }
            1 => {
                ui.label("failure");
            }
            _ => unreachable!(),
        }
        self.out_kind(pin_id).default_pin()
    }

    fn has_body(&self) -> bool {
        true
    }

    fn show_body(&mut self, ui: &mut egui::Ui, _ctx: &EditContext) {
        ui.vertical(|ui| {
            ui.checkbox(&mut self.extract, "extract").on_hover_text(
                "Attempt to find a JSON object inside unstructured text.\n\
                    Use this when you expect that the input is mostly JSON.\n\
                    It can be slow on large documents with many braces,\n\
                    such as source code, log files or malformed JSON.\n\
                    Does not attempt to repair broken JSON documents.",
            );

            if self.extract {
                ui.checkbox(&mut self.as_array, "as array")
                    .on_hover_text("Find an array instead of an object.");
            }
        });
    }
}

#[skip_serializing_none]
#[derive(Debug, Clone, Default, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidateJson {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    schema: String,

    size: Option<crate::utils::EVec2>,
}

#[typetag::serde]
impl FlexNode for ValidateJson {}

impl DynNode for ValidateJson {
    fn inputs(&self) -> usize {
        2
    }

    fn in_kinds(&'_ self, in_pin: usize) -> Cow<'_, [ValueKind]> {
        Cow::Borrowed(match in_pin {
            0 => &[ValueKind::Json],
            1 => &[ValueKind::Json, ValueKind::Text, ValueKind::Message],
            _ => unreachable!(),
        })
    }

    fn outputs(&self) -> usize {
        2
    }

    fn out_kind(&self, out_pin: usize) -> ValueKind {
        match out_pin {
            0 => ValueKind::Json,
            1 => ValueKind::Failure,
            _ => unreachable!(),
        }
    }

    fn execute(
        &mut self,
        _ctx: &RunContext,
        _node_id: egui_snarl::NodeId,
        inputs: Vec<Option<Value>>,
    ) -> Result<Vec<Value>, WorkflowError> {
        self.validate(&inputs)?;

        let schema = match &inputs[0] {
            Some(Value::Json(schema)) => schema.as_ref().to_owned(),
            None if !self.schema.is_empty() => serde_json::from_str(&self.schema)
                .map_err(|err| WorkflowError::Conversion(err.to_string()))?,
            None => Err(WorkflowError::Required(vec!["Schema is required".into()]))?,
            _ => unreachable!(),
        };

        let input = match &inputs[1] {
            Some(Value::Json(input)) => input.as_ref().to_owned(),
            Some(Value::Text(text)) => serde_json::from_str(text)
                .map_err(|err| WorkflowError::Conversion(err.to_string()))?,
            Some(Value::Message(msg)) => serde_json::from_str(&message_text(msg))
                .map_err(|err| WorkflowError::Conversion(err.to_string()))?,
            None => Err(WorkflowError::Required(vec!["JSON input required".into()]))?,
            _ => unreachable!(),
        };

        jsonschema::validate(&schema, &input)
            .map_err(|err| anyhow::anyhow!("Validation error: {err:?}"))?;

        let value = Arc::new(input);

        Ok(vec![
            Value::Json(value),
            Value::Placeholder(ValueKind::Failure),
        ])
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
        remote: Option<Value>,
    ) -> egui_snarl::ui::PinInfo {
        match pin_id {
            0 => {
                if remote.is_none() {
                    resizable_frame(&mut self.size, ui, |ui| {
                        egui::ScrollArea::vertical().show(ui, |ui| {
                            let hint_text = "JSON Schema";
                            json_editor(ui, &mut self.schema, Some(hint_text));
                        });
                    });
                } else {
                    ui.label("schema");
                }
            }
            1 => {
                ui.label("json");
            }
            _ => unreachable!(),
        };

        self.in_kinds(pin_id).first().unwrap().default_pin()
    }

    fn show_output(
        &mut self,
        ui: &mut egui::Ui,
        _ctx: &EditContext,
        pin_id: usize,
    ) -> egui_snarl::ui::PinInfo {
        match pin_id {
            0 => {
                ui.label("json");
            }
            1 => {
                ui.label("failure");
            }
            _ => unreachable!(),
        }
        self.out_kind(pin_id).default_pin()
    }
}

#[skip_serializing_none]
#[derive(Debug, Clone, Default, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransformJson {
    filter: String,

    size: Option<crate::utils::EVec2>,
}

#[typetag::serde]
impl FlexNode for TransformJson {}

impl DynNode for TransformJson {
    fn inputs(&self) -> usize {
        2
    }

    fn in_kinds(&'_ self, in_pin: usize) -> Cow<'_, [ValueKind]> {
        Cow::Borrowed(match in_pin {
            0 => &[ValueKind::Text],
            1 => &[
                ValueKind::Json,
                ValueKind::Number,
                ValueKind::Integer,
                ValueKind::Text,
                ValueKind::Message,
            ],
            _ => unreachable!(),
        })
    }

    fn out_kind(&self, _out_pin: usize) -> ValueKind {
        ValueKind::Json
    }

    fn execute(
        &mut self,
        ctx: &RunContext,
        _node_id: egui_snarl::NodeId,
        inputs: Vec<Option<Value>>,
    ) -> Result<Vec<Value>, WorkflowError> {
        self.validate(&inputs)?;

        let filter = match &inputs[0] {
            Some(Value::Text(text)) => text.as_str(),
            None => self.filter.as_str(),
            _ => unreachable!(),
        };

        let filter = ctx.transmuter.init_filter(filter)?;

        let input = match &inputs[1] {
            Some(Value::Json(input)) => input.as_ref().to_owned(),
            Some(Value::Number(value)) => json!(value),
            Some(Value::Integer(value)) => json!(value),
            Some(Value::Text(value)) => json!(value),
            Some(Value::Message(value)) => json!(message_text(value)),
            None => Err(WorkflowError::Required(vec!["JSON input required".into()]))?,
            _ => unreachable!(),
        };

        let value = ctx.transmuter.run_filter(&filter, input)?;

        Ok(vec![Value::Json(Arc::new(value))])
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
                        resizable_frame(&mut self.size, ui, |ui| {
                            egui::ScrollArea::vertical().show(ui, |ui| {
                                let widget = egui::TextEdit::multiline(&mut self.filter)
                                    .desired_width(f32::INFINITY)
                                    .hint_text("Filter\u{1F64B}");

                                squelch(ui.add_sized(ui.available_size(), widget));
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

#[derive(Debug, Clone, Default, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub struct GatherJson {
    count: usize,
}

#[typetag::serde]
impl FlexNode for GatherJson {}

impl DynNode for GatherJson {
    fn inputs(&self) -> usize {
        self.count + 1 // Extra slot to add another document
    }

    fn in_kinds(&'_ self, _in_pin: usize) -> Cow<'_, [ValueKind]> {
        Cow::Borrowed(&[
            ValueKind::Json,
            ValueKind::Text,
            ValueKind::Number,
            ValueKind::Integer,
            ValueKind::Message,
        ])
    }

    fn out_kind(&self, _out_pin: usize) -> ValueKind {
        ValueKind::Json
    }

    fn execute(
        &mut self,
        _ctx: &RunContext,
        _node_id: egui_snarl::NodeId,
        inputs: Vec<Option<Value>>,
    ) -> Result<Vec<Value>, WorkflowError> {
        use crate::utils::message_text;
        use serde_json::Number;

        self.validate(&inputs)?;

        let values = inputs
            .into_iter()
            .take(self.count)
            .map(|it| match it {
                Some(Value::Json(value)) => value.as_ref().clone(),
                Some(Value::Text(value)) => serde_json::Value::String((*value).clone()),
                Some(Value::Number(value)) => {
                    serde_json::Value::Number(Number::from_f64(value.into_inner()).unwrap())
                }
                Some(Value::Integer(value)) => {
                    serde_json::Value::Number(Number::from_i128(value as i128).unwrap())
                }
                Some(Value::Message(value)) => serde_json::Value::String(message_text(&value)),
                None => serde_json::Value::Null,
                _ => unreachable!(),
            })
            .collect_vec();

        let value = Arc::new(serde_json::Value::Array(values));

        Ok(vec![Value::Json(value)])
    }
}

impl UiNode for GatherJson {
    fn title(&self) -> &str {
        "Gather JSON"
    }

    fn tooltip(&self) -> &str {
        "Combine multiple JSON documents into a single array.\n\
            The output can be transformed using shallow, deep or arbitrary merging"
    }

    fn show_input(
        &mut self,
        ui: &mut egui::Ui,
        _ctx: &EditContext,
        pin_id: usize,
        remote: Option<Value>,
    ) -> egui_snarl::ui::PinInfo {
        if pin_id == self.count && remote.is_some() {
            self.count += 1;
        } else if pin_id + 1 == self.count && remote.is_none() {
            self.count -= 1;
        }

        if pin_id < self.count {
            ui.label(format!(".[{pin_id}]"));
        }

        self.in_kinds(pin_id).first().unwrap().default_pin()
    }
}

fn json_node_menu(ui: &mut egui::Ui, snarl: &mut egui_snarl::Snarl<WorkNode>, pos: egui::Pos2) {
    ui.menu_button("JSON", |ui| {
        if ui.button("Parse JSON").clicked() {
            snarl.insert_node(pos, ParseJson::default().into());
            ui.close();
        }

        if ui.button("Gather JSON").clicked() {
            snarl.insert_node(pos, GatherJson::default().into());
            ui.close();
        }

        if ui.button("Validate JSON").clicked() {
            snarl.insert_node(pos, ValidateJson::default().into());
            ui.close();
        }

        if ui.button("Transform JSON").clicked() {
            snarl.insert_node(pos, TransformJson::default().into());
            ui.close();
        }
    });
}

inventory::submit! {
    GraphSubmenu("json", json_node_menu)
}
