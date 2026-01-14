use std::convert::identity;

use decorum::E64;
use egui::RichText;
use egui_commonmark::CommonMarkCache;
use egui_phosphor::regular::NUMPAD;
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

use crate::{
    ChatContent,
    ui::{AppEvent, resizable_frame, shortcuts::squelch, tiles::chat::render_message_width},
    utils::{message_party, message_text},
    workflow::{GraphId, WorkflowError},
};

use super::{DynNode, EditContext, RunContext, UiNode, Value, ValueKind};

#[derive(Debug, Clone, Default, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub struct Number {
    f_value: E64,
    i_value: i64,

    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    integer: bool,
}

impl DynNode for Number {
    fn out_kind(&self, _out_pin: usize) -> ValueKind {
        if self.integer {
            ValueKind::Integer
        } else {
            ValueKind::Number
        }
    }

    fn execute(
        &mut self,
        _ctx: &RunContext,
        _node_id: egui_snarl::NodeId,
        _inputs: Vec<Option<Value>>,
    ) -> Result<Vec<Value>, WorkflowError> {
        if self.integer {
            Ok(vec![Value::Integer(self.i_value)])
        } else {
            Ok(vec![Value::Number(self.f_value)])
        }
    }
}

impl UiNode for Number {
    fn title(&self) -> &str {
        "Number"
    }

    fn show_output(
        &mut self,
        ui: &mut egui::Ui,
        _ctx: &EditContext,
        pin_id: usize,
    ) -> egui_snarl::ui::PinInfo {
        assert_eq!(pin_id, 0);

        if self.integer {
            ui.add(egui::DragValue::new(&mut self.i_value).update_while_editing(false));
            self.f_value = E64::assert(self.i_value as f64);
        } else {
            let mut inner = self.f_value.into_inner();
            ui.add(
                egui::DragValue::new(&mut inner)
                    .speed(0.1)
                    .update_while_editing(false),
            );
            self.f_value = E64::assert(inner);
            self.i_value = inner as i64;
        }

        ui.toggle_value(&mut self.integer, NUMPAD)
            .on_hover_text("treat value as integer");
        self.out_kind(pin_id).default_pin()
    }
}

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

    fn out_kind(&self, out_pin: usize) -> ValueKind {
        assert_eq!(out_pin, 0);
        ValueKind::Text
    }

    fn execute(
        &mut self,
        _ctx: &RunContext,
        _node_id: egui_snarl::NodeId,
        _inputs: Vec<Option<Value>>,
    ) -> Result<Vec<Value>, WorkflowError> {
        Ok(vec![Value::Text(self.value.clone())])
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

                squelch(ui.add_sized(ui.available_size(), widget));
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

#[skip_serializing_none]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Preview {
    size: Option<crate::utils::EVec2>,

    // TODO: regenerate after paste
    #[serde(default)]
    pub uuid: GraphId,
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

    fn outputs(&self) -> usize {
        0
    }

    fn execute(
        &mut self,
        ctx: &RunContext,
        _node_id: egui_snarl::NodeId,
        inputs: Vec<Option<Value>>,
    ) -> Result<Vec<Value>, WorkflowError> {
        if let Some(value) = inputs.first().and_then(|it| it.as_ref()) {
            ctx.previews.update(self.uuid.0, value.clone());
        }
        Ok(vec![])
    }
}

impl UiNode for Preview {
    fn on_paste(&mut self) {
        self.uuid = GraphId::new();
    }

    fn title(&self) -> &str {
        "Preview"
    }

    fn has_body(&self) -> bool {
        true
    }

    fn show_body(&mut self, ui: &mut egui::Ui, ctx: &EditContext) {
        let mut cache = CommonMarkCache::default();

        egui::Frame::new().inner_margin(4).show(ui, |ui| {
            resizable_frame(&mut self.size, ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    match &ctx.previews.value(self.uuid.0).unwrap_or_default() {
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
                                let theme =
                                    egui_extras::syntax_highlighting::CodeTheme::from_memory(
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
                        unk => {
                            ui.add(egui::Label::new(format!("{unk:?}")).wrap());
                        }
                    }
                });
            });
        });
    }
}

#[derive(Debug, Clone, Default, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputNode {
    label: String,
}

impl DynNode for OutputNode {
    fn priority(&self) -> usize {
        9999
    }

    fn outputs(&self) -> usize {
        0
    }

    fn execute(
        &mut self,
        ctx: &RunContext,
        _node_id: egui_snarl::NodeId,
        inputs: Vec<Option<Value>>,
    ) -> Result<Vec<Value>, WorkflowError> {
        self.validate(&inputs)?;

        if self.label.is_empty() {
            Err(WorkflowError::Required(vec!["Label is required".into()]))?;
        }
        let output = inputs
            .into_iter()
            .find_map(identity)
            .ok_or(WorkflowError::Required(vec![
                "Output called with empty inputs".into(),
            ]))?;

        ctx.outputs
            .sender()
            .send((self.label.clone(), output))
            .map_err(|err| WorkflowError::Unknown(format!("Couldn't send output: {err:?}")))?;

        Ok(vec![])
    }
}

impl UiNode for OutputNode {
    fn title(&self) -> &str {
        "Output"
    }

    fn tooltip(&self) -> &str {
        "Emits an output.\n\
            It is up to the workflow runner to determine what to do with it."
    }

    fn has_body(&self) -> bool {
        true
    }

    fn show_body(&mut self, ui: &mut egui::Ui, ctx: &EditContext) {
        if ctx.parent_id.is_some() && !ctx.disabled {
            ctx.events
                .insert(AppEvent::DisableNode(ctx.current_graph, ctx.current_node));
        }

        ui.vertical(|ui| {
            ui.label("label:");
            ui.text_edit_singleline(&mut self.label);
        });
    }
}

#[derive(Debug, Clone, Default, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub struct Panic {}

impl DynNode for Panic {
    fn priority(&self) -> usize {
        9000
    }

    fn outputs(&self) -> usize {
        0
    }

    fn execute(
        &mut self,
        _ctx: &RunContext,
        _node_id: egui_snarl::NodeId,
        inputs: Vec<Option<Value>>,
    ) -> Result<Vec<Value>, WorkflowError> {
        if let Some(value) = inputs.first().and_then(|it| it.as_ref()) {
            match value {
                Value::Placeholder(_) => {}
                Value::Text(txt) if txt.is_empty() => {}
                _ => Err(WorkflowError::Unknown(format!(
                    "Panic node received a non-empty input: {value:?}"
                )))?,
            }
        }

        Ok(vec![])
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
