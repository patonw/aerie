use std::borrow::Cow;

use serde::{Deserialize, Serialize};
use serde_json::json;
use serde_with::skip_serializing_none;

use crate::{
    ui::{resizable_frame, shortcuts::squelch},
    utils::message_text,
    workflow::{DynNode, EditContext, RunContext, UiNode, Value, WorkflowError},
};

use super::ValueKind;

#[skip_serializing_none]
#[derive(Debug, Clone, Default, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommentNode {
    comment: String,

    size: Option<crate::utils::EVec2>,
}

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
                ValueKind::Message,
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
        self.validate(&inputs)?;

        let template = match &inputs[0] {
            Some(Value::Text(text)) => text.clone(),
            None => self.template.clone(),
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
            Some(Value::Message(value)) => json!({"value": message_text(value)}),
            None => Err(WorkflowError::Required(vec!["JSON input required".into()]))?,
            _ => unreachable!(),
        };

        let value = ctx.transmuter.render_template(&template, &vars)?;

        Ok(vec![Value::Text(value)])
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
