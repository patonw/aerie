use egui::RichText;
use egui_commonmark::CommonMarkCache;
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

use crate::{
    ChatContent,
    ui::{resizable_frame, tiles::chat::render_message_width},
    utils::{message_party, message_text},
    workflow::WorkflowError,
};

use super::{DynNode, EditContext, RunContext, UiNode, Value, ValueKind};

#[skip_serializing_none]
#[derive(Debug, Clone, Default, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub struct Text {
    pub value: String,

    pub size: Option<crate::utils::EVec2>,
}

impl DynNode for Text {
    fn value(&self, _out_pin: usize) -> Value {
        Value::Text(self.value.clone())
    }

    fn inputs(&self) -> usize {
        0
    }

    fn out_kind(&self, out_pin: usize) -> ValueKind {
        assert_eq!(out_pin, 0);
        ValueKind::Text
    }
}

impl UiNode for Text {
    fn title(&self) -> &str {
        "Text"
    }

    fn has_body(&self) -> bool {
        true
    }

    fn show_body(&mut self, ui: &mut egui::Ui, _ctx: &EditContext) {
        egui::Frame::new().inner_margin(4).show(ui, |ui| {
            resizable_frame(&mut self.size, ui, |ui| {
                let widget = egui::TextEdit::multiline(&mut self.value)
                    .desired_width(f32::INFINITY)
                    .hint_text("Enter text \u{1F64B}");

                ui.add_sized(ui.available_size(), widget);
            });
        });
    }

    fn show_output(
        &mut self,
        _ui: &mut egui::Ui,
        _ctx: &EditContext,
        pin_id: usize,
    ) -> egui_snarl::ui::PinInfo {
        assert_eq!(pin_id, 0);

        self.out_kind(pin_id).default_pin()
    }
}

impl Text {
    pub async fn forward(
        &mut self,
        _ctx: &RunContext,
        _inputs: Vec<Option<Value>>,
    ) -> Result<(), WorkflowError> {
        Ok(())
    }
}

#[skip_serializing_none]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Preview {
    size: Option<crate::utils::EVec2>,

    #[serde(skip)]
    pub value: Value,
}

impl std::hash::Hash for Preview {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.size.hash(state);
    }
}

impl PartialEq for Preview {
    fn eq(&self, other: &Self) -> bool {
        self.size.eq(&other.size)
    }
}

impl Eq for Preview {}

impl DynNode for Preview {
    fn priority(&self) -> usize {
        9999
    }

    fn reset(&mut self, _in_pin: usize) {
        self.value = Default::default();
    }

    fn outputs(&self) -> usize {
        0
    }

    fn value(&self, _out_pin: usize) -> Value {
        self.value.clone()
    }
}

impl UiNode for Preview {
    fn title(&self) -> &str {
        "Preview"
    }

    fn has_body(&self) -> bool {
        true
    }

    fn show_body(&mut self, ui: &mut egui::Ui, _ctx: &EditContext) {
        let mut cache = CommonMarkCache::default();

        egui::Frame::new().inner_margin(4).show(ui, |ui| {
            resizable_frame(&mut self.size, ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| match &self.value {
                    Value::Text(text) => {
                        ui.add(egui::Label::new(text).wrap());
                    }
                    Value::Chat(history) => {
                        ui.vertical(|ui| {
                            for entry in history.iter() {
                                if let ChatContent::Message(msg) = &entry.content {
                                    ui.label(RichText::new(message_party(msg)).strong());
                                    ui.add(egui::Label::new(message_text(msg)).wrap());
                                    ui.separator();
                                }
                            }
                        });
                    }
                    Value::Message(msg) => {
                        render_message_width(ui, &mut cache, msg, Some(600.0));
                    }
                    Value::Json(value) => {
                        if let Ok(text) = serde_json::to_string_pretty(value) {
                            let language = "json";
                            let theme = egui_extras::syntax_highlighting::CodeTheme::from_memory(
                                ui.ctx(),
                                ui.style(),
                            );

                            egui_extras::syntax_highlighting::code_view_ui(
                                ui, &theme, &text, language,
                            );
                        } else {
                            ui.add(egui::Label::new(format!("{:?}", value)).wrap());
                        }
                    }
                    _ => {
                        ui.add(egui::Label::new(format!("{:?}", self.value)).wrap());
                    }
                });
            });
        });
    }
}

impl Preview {
    pub async fn forward(
        &mut self,
        _ctx: &RunContext,
        inputs: Vec<Option<Value>>,
    ) -> Result<(), WorkflowError> {
        if let Some(value) = inputs.first().and_then(|it| it.as_ref()) {
            self.value = value.to_owned();
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub struct Panic {}

impl DynNode for Panic {
    fn priority(&self) -> usize {
        9000
    }
}

impl UiNode for Panic {
    fn title(&self) -> &str {
        "Panic"
    }

    fn tooltip(&self) -> &str {
        "Aborts run if the input is non-empty"
    }
}

impl Panic {
    pub async fn forward(
        &mut self,
        _ctx: &RunContext,
        inputs: Vec<Option<Value>>,
    ) -> Result<(), WorkflowError> {
        if let Some(value) = inputs.first().and_then(|it| it.as_ref()) {
            match value {
                Value::Placeholder(_) => {}
                Value::Text(txt) if txt.is_empty() => {}
                _ => Err(WorkflowError::Unknown(format!(
                    "Panic node received a non-empty input: {value:?}"
                )))?,
            }
        }

        Ok(())
    }
}
