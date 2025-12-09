use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::workflow::{
    DynNode, EditContext, RunContext, UiNode, Value, WorkflowError,
    nodes::{MIN_HEIGHT, MIN_WIDTH},
};

use super::ValueKind;

#[derive(Debug, Clone, Default, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommentNode {
    comment: String,
}

impl DynNode for CommentNode {
    fn inputs(&self) -> usize {
        0
    }

    fn outputs(&self) -> usize {
        0
    }
}

// TODO: render as a yellow sticky note without a header
impl UiNode for CommentNode {
    fn title(&self) -> &str {
        "Comment"
    }

    fn has_body(&self) -> bool {
        true
    }

    fn show_body(&mut self, ui: &mut egui::Ui, _ctx: &EditContext) {
        egui::Frame::new().inner_margin(4).show(ui, |ui| {
            egui::Resize::default()
                .min_width(MIN_WIDTH)
                .min_height(MIN_HEIGHT)
                .show(ui, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        let widget = egui::TextEdit::multiline(&mut self.comment)
                            .desired_width(f32::INFINITY)
                            .hint_text("Comment body\u{1F64B}");

                        ui.add_sized(ui.available_size(), widget);
                    });
                });
        });
    }
}

impl CommentNode {
    pub async fn forward(
        &mut self,
        _ctx: &RunContext,
        _inputs: Vec<Option<Value>>,
    ) -> Result<(), WorkflowError> {
        Ok(())
    }
}

#[derive(Debug, Clone, Default, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateNode {
    template: String,

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
                        egui::Resize::default()
                            .min_width(MIN_WIDTH)
                            .min_height(MIN_HEIGHT)
                            .show(ui, |ui| {
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
}

impl TemplateNode {
    pub async fn forward(
        &mut self,
        run_ctx: &RunContext,
        inputs: Vec<Option<Value>>,
    ) -> Result<(), WorkflowError> {
        self.validate(&inputs)?;

        let template = match &inputs[0] {
            Some(Value::Text(text)) => text.clone(),
            None => self.template.clone(),
            _ => unreachable!(),
        };

        let vars = match &inputs[1] {
            Some(Value::Json(value)) => value.as_ref().to_owned(),
            None => Err(WorkflowError::Input(vec!["JSON input required".into()]))?,
            _ => unreachable!(),
        };

        self.vars = Arc::new(vars);
        self.value = run_ctx.transmuter.render_template(&template, &self.vars)?;

        Ok(())
    }
}
