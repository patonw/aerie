use std::{
    borrow::Cow,
    sync::{Arc, LazyLock},
};

use serde::{Deserialize, Serialize};
use serde_json::json;
use serde_with::skip_serializing_none;

use crate::{
    ui::{resizable_frame, shortcuts::squelch},
    utils::{message_party, message_text},
    workflow::{DynNode, EditContext, FlexNode, RunContext, UiNode, Value, WorkflowError},
};

use super::ValueKind;

static ENV_JSON: LazyLock<Arc<serde_json::Value>> = LazyLock::new(|| {
    let entries = std::env::vars()
        .map(|(k, v)| (k, serde_json::Value::String(v)))
        .collect();

    Arc::new(serde_json::Value::Object(entries))
});

#[skip_serializing_none]
#[derive(Debug, Clone, Default, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommentNode {
    comment: String,

    size: Option<crate::utils::EVec2>,
}

#[typetag::serde]
impl FlexNode for CommentNode {}

impl DynNode for CommentNode {
    fn inputs(&self) -> usize {
        0
    }

    fn outputs(&self) -> usize {
        0
    }
}

impl UiNode for CommentNode {
    fn title(&self) -> &str {
        "Comment"
    }

    fn has_body(&self) -> bool {
        true
    }

    fn show_body(&mut self, ui: &mut egui::Ui, _ctx: &EditContext) {
        egui::Frame::new().inner_margin(4).show(ui, |ui| {
            resizable_frame(&mut self.size, ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    let widget = egui::TextEdit::multiline(&mut self.comment)
                        .desired_width(f32::INFINITY)
                        .background_color(Self::bg_color())
                        .text_color_opt(Some(egui::Color32::BLACK))
                        .hint_text("Comment body\u{1F64B}");
                    squelch(ui.add_sized(ui.available_size(), widget));
                });
            });
        });
    }
}

impl CommentNode {
    pub fn bg_color() -> egui::Color32 {
        egui::Color32::LIGHT_YELLOW.gamma_multiply(0.75)
    }
}

#[skip_serializing_none]
#[derive(Debug, Clone, Default, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateNode {
    template: String,

    size: Option<crate::utils::EVec2>,
}

#[typetag::serde]
impl FlexNode for TemplateNode {}

impl DynNode for TemplateNode {
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
                ValueKind::FloatList,
                ValueKind::IntList,
                ValueKind::TextList,
                ValueKind::Chat,
                ValueKind::Message,
                ValueKind::MsgList,
            ],
            _ => unreachable!(),
        })
    }

    fn out_kind(&self, out_pin: usize) -> ValueKind {
        match out_pin {
            0 => ValueKind::Text,
            _ => unreachable!(),
        }
    }

    fn execute(
        &mut self,
        ctx: &RunContext,
        _node_id: egui_snarl::NodeId,
        inputs: Vec<Option<Value>>,
    ) -> Result<Vec<Value>, WorkflowError> {
        use itertools::Itertools as _;
        self.validate(&inputs)?;

        let template = match &inputs[0] {
            Some(Value::Text(text)) => text.as_str(),
            None => self.template.as_str(),
            _ => unreachable!(),
        };

        let vars = match &inputs[1] {
            Some(Value::Json(value)) => match value.as_ref() {
                serde_json::Value::Object(_) => value.as_ref().clone(),
                _ => json!({"value": value}),
            },
            Some(Value::Number(value)) => json!({"value": value}),
            Some(Value::Integer(value)) => json!({"value": value}),
            Some(Value::Text(value)) => json!({"value": value}),
            Some(Value::FloatList(value)) => json!({"value": value}),
            Some(Value::IntList(value)) => json!({"value": value}),
            Some(Value::TextList(value)) => json!({"value": value}),
            Some(Value::Chat(value)) => {
                json!({
                    "value":
                    value
                        .iter_msgs()
                        .map(|m| json!({"author": message_party(&m), "content": message_text(&m)}))
                        .collect_vec()
                })
            }
            Some(Value::Message(value)) => {
                json!({"value": {"author": message_party(value), "content": message_text(value)}})
            }
            Some(Value::MsgList(value)) => {
                json!({"value":
                   value
                       .iter()
                       .map(|m| json!({"author": message_party(m), "content": message_text(m)}))
                       .collect_vec()
                })
            }
            None => Err(WorkflowError::Required(vec!["JSON input required".into()]))?,
            _ => unreachable!(),
        };

        let value = ctx.transmuter.render_template(template, &vars)?;

        Ok(vec![Value::Text(Arc::new(value))])
    }
}

impl UiNode for TemplateNode {
    fn title(&self) -> &str {
        "Template"
    }

    fn tooltip(&self) -> &str {
        "Renders a Minijinja template with variables from a JSON value"
    }

    fn help_link(&self) -> &str {
        "https://docs.rs/minijinja/latest/minijinja/syntax/index.html"
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
                                let widget = egui::TextEdit::multiline(&mut self.template)
                                    .desired_width(f32::INFINITY)
                                    .hint_text("Template body\u{1F64B}");

                                squelch(ui.add_sized(ui.available_size(), widget));
                            });
                        });
                    });
                } else {
                    ui.label("template");
                }
            }
            1 => {
                ui.label("variables");
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
                ui.label("text");
            }
            _ => unreachable!(),
        }
        self.out_kind(pin_id).default_pin()
    }
}

/// Returns the current environment as a key-value object
#[skip_serializing_none]
#[derive(Debug, Clone, Default, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvironmentNode {}

#[typetag::serde]
impl FlexNode for EnvironmentNode {}

impl DynNode for EnvironmentNode {
    fn inputs(&self) -> usize {
        0
    }

    fn out_kind(&self, _out_pin: usize) -> ValueKind {
        ValueKind::Json
    }

    fn execute(
        &mut self,
        _ctx: &RunContext,
        _node_id: egui_snarl::NodeId,
        _inputs: Vec<Option<Value>>,
    ) -> Result<Vec<Value>, WorkflowError> {
        Ok(vec![Value::Json(ENV_JSON.clone())])
    }
}

impl UiNode for EnvironmentNode {
    fn title(&self) -> &str {
        "Environment"
    }

    fn tooltip(&self) -> &str {
        "Gets the current set of environment variables"
    }
}
