use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

use crate::{
    ui::resizable_frame,
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
                    ui.add_sized(ui.available_size(), widget);
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

    #[serde(skip)]
    vars: Arc<serde_json::Value>,

    #[serde(skip)]
    value: String,
}

impl DynNode for TemplateNode {
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

    fn out_kind(&self, out_pin: usize) -> ValueKind {
        match out_pin {
            0 => ValueKind::Text,
            _ => unreachable!(),
        }
    }

    fn value(&self, out_pin: usize) -> Value {
        match out_pin {
            0 => Value::Text(self.value.clone()),
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
            Some(Value::Json(value)) => value.as_ref().to_owned(),
            None => Err(WorkflowError::Required(vec!["JSON input required".into()]))?,
            _ => unreachable!(),
        };

        self.vars = Arc::new(vars);
        self.value = ctx.transmuter.render_template(&template, &self.vars)?;

        Ok(self.collect_outputs())
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

                                ui.add_sized(ui.available_size(), widget);
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
