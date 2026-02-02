use std::{borrow::Cow, convert::identity, str::FromStr as _, sync::Arc};

use decorum::E64;
use egui::RichText;
use egui_commonmark::CommonMarkCache;
use egui_phosphor::regular::{BRACKETS_SQUARE, NUMPAD};
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

use crate::{
    ChatContent,
    ui::{AppEvent, resizable_frame, shortcuts::squelch, tiles::chat::render_message_width},
    utils::{message_party, message_text},
    workflow::{FlexNode, GraphId, WorkflowError},
};

use super::{DynNode, EditContext, RunContext, UiNode, Value, ValueKind};

#[derive(Debug, Clone, Default, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub struct Number {
    f_value: E64,
    i_value: i64,

    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    list: bool,

    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    integer: bool,
}

#[typetag::serde]
impl FlexNode for Number {}

impl DynNode for Number {
    fn in_kinds(&'_ self, _in_pin: usize) -> Cow<'_, [ValueKind]> {
        let Self { integer, .. } = self;
        if *integer {
            Cow::Borrowed(&[
                ValueKind::Integer,
                ValueKind::IntList,
                ValueKind::Number,
                ValueKind::FloatList,
                ValueKind::TextList,
                ValueKind::Text,
            ])
        } else {
            Cow::Borrowed(&[
                ValueKind::Number,
                ValueKind::FloatList,
                ValueKind::Integer,
                ValueKind::IntList,
                ValueKind::TextList,
                ValueKind::Text,
            ])
        }
    }

    fn outputs(&self) -> usize {
        2
    }

    fn out_kind(&self, out_pin: usize) -> ValueKind {
        let Self { list, integer, .. } = self;

        if out_pin == 0 {
            match (list, integer) {
                (true, true) => ValueKind::IntList,
                (false, true) => ValueKind::Integer,
                (true, false) => ValueKind::FloatList,
                (false, false) => ValueKind::Number,
            }
        } else {
            ValueKind::Failure
        }
    }

    fn execute(
        &mut self,
        _ctx: &RunContext,
        _node_id: egui_snarl::NodeId,
        inputs: Vec<Option<Value>>,
    ) -> Result<Vec<Value>, WorkflowError> {
        let Self { list, integer, .. } = self;

        match &inputs[0] {
            None => {
                if self.integer {
                    Ok(vec![Value::Integer(self.i_value)])
                } else {
                    Ok(vec![Value::Number(self.f_value)])
                }
            }
            Some(Value::Integer(value)) => Ok(vec![match (list, integer) {
                (true, true) => Value::IntList(im::vector![*value]),
                (false, true) => Value::Integer(*value),
                (true, false) => Value::FloatList(im::vector![E64::assert(*value as f64)]),
                (false, false) => Value::Number(E64::assert(*value as f64)),
            }]),
            Some(Value::IntList(value)) => Ok(vec![match (list, integer) {
                (true, true) => Value::IntList(value.clone()),
                (false, true) if value.len() == 1 => Value::Integer(value[0]),
                (true, false) => {
                    Value::FloatList(value.iter().map(|it| E64::assert(*it as f64)).collect())
                }
                (false, false) if value.len() == 1 => Value::Number(E64::assert(value[0] as f64)),
                (false, _) => Err(WorkflowError::Conversion(
                    "List must have exactly one value".into(),
                ))?,
            }]),
            Some(Value::Number(value)) => Ok(vec![match (list, integer) {
                (true, true) => Value::IntList(im::vector![value.into_inner() as i64]),
                (false, true) => Value::Integer(value.into_inner() as i64),
                (true, false) => Value::FloatList(im::vector![*value]),
                (false, false) => Value::Number(*value),
            }]),
            Some(Value::FloatList(value)) => Ok(vec![match (list, integer) {
                (true, true) => {
                    Value::IntList(value.iter().map(|it| it.into_inner() as i64).collect())
                }
                (false, true) if value.len() == 1 => Value::Integer(value[0].into_inner() as i64),
                (true, false) => Value::FloatList(value.clone()),
                (false, false) if value.len() == 1 => Value::Number(value[0]),
                (false, _) => Err(WorkflowError::Conversion(
                    "List must have exactly one value".into(),
                ))?,
            }]),
            Some(Value::Text(value)) => Ok(vec![match (list, integer) {
                (true, true) => {
                    let item = value
                        .parse::<i64>()
                        .map_err(|e| WorkflowError::Conversion(format!("{e:?}")))?;
                    Value::IntList(im::vector![item])
                }
                (false, true) => Value::Integer(
                    value
                        .parse()
                        .map_err(|e| WorkflowError::Conversion(format!("{e:?}")))?,
                ),
                (true, false) => {
                    let item = E64::from_str(value)
                        .map_err(|e| WorkflowError::Conversion(format!("{e:?}")))?;
                    Value::FloatList(im::vector![item])
                }
                (false, false) => Value::Number(
                    E64::from_str(value.as_str())
                        .map_err(|e| WorkflowError::Conversion(format!("{e:?}")))?,
                ),
            }]),
            Some(Value::TextList(value)) => Ok(vec![match (list, integer) {
                (true, true) => {
                    let items: Result<im::Vector<i64>, WorkflowError> = value
                        .iter()
                        .map(|it| {
                            it.parse::<i64>()
                                .map_err(|e| WorkflowError::Conversion(format!("{e:?}")))
                        })
                        .collect();
                    Value::IntList(items?)
                }
                (false, true) if value.len() == 1 => Value::Integer(
                    value[0]
                        .parse()
                        .map_err(|e| WorkflowError::Conversion(format!("{e:?}")))?,
                ),
                (true, false) => {
                    let items: Result<im::Vector<_>, WorkflowError> = value
                        .iter()
                        .map(|it| {
                            E64::from_str(it)
                                .map_err(|e| WorkflowError::Conversion(format!("{e:?}")))
                        })
                        .collect();
                    Value::FloatList(items?)
                }
                (false, false) if value.len() == 1 => Value::Number(
                    E64::from_str(value[0].as_str())
                        .map_err(|e| WorkflowError::Conversion(format!("{e:?}")))?,
                ),
                (false, _) => Err(WorkflowError::Conversion(
                    "List must have exactly one value".into(),
                ))?,
            }]),
            _ => unreachable!(),
        }
    }
}

impl UiNode for Number {
    fn title(&self) -> &str {
        "Number"
    }
    fn show_input(
        &mut self,
        ui: &mut egui::Ui,
        _ctx: &EditContext,
        pin_id: usize,
        remote: Option<Value>,
    ) -> egui_snarl::ui::PinInfo {
        assert_eq!(pin_id, 0);

        let Self {
            list,
            integer,
            i_value,
            f_value,
        } = self;

        if remote.is_none() {
            if *integer {
                ui.add(egui::DragValue::new(i_value).update_while_editing(false));
                *f_value = E64::assert(*i_value as f64);
            } else {
                let mut inner = f_value.into_inner();
                ui.add(
                    egui::DragValue::new(&mut inner)
                        .speed(0.1)
                        .update_while_editing(false),
                );
                *f_value = E64::assert(inner);
                *i_value = inner as i64;
            }

            self.list = false;
        } else {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.toggle_value(list, BRACKETS_SQUARE)
                .on_hover_text("list value");
        }

        ui.toggle_value(integer, NUMPAD)
            .on_hover_text("integer value");

        self.in_kinds(pin_id).first().unwrap().default_pin()
    }
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub enum TextSplit {
    Lines,
    Paragraphs,
    Words,
    Documents,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Default, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub struct Text {
    pub value: Arc<String>,

    pub delim: Option<TextSplit>,

    pub size: Option<crate::utils::EVec2>,
}

#[typetag::serde]
impl FlexNode for Text {}

impl DynNode for Text {
    fn value(&self, _out_pin: usize) -> Value {
        Value::Text(self.value.clone())
    }

    fn in_kinds(&'_ self, in_pin: usize) -> std::borrow::Cow<'_, [ValueKind]> {
        Cow::Borrowed(match in_pin {
            0 => &[ValueKind::Text, ValueKind::Message],
            _ => unreachable!(),
        })
    }

    fn out_kind(&self, out_pin: usize) -> ValueKind {
        assert_eq!(out_pin, 0);
        if self.delim.is_none() {
            ValueKind::Text
        } else {
            ValueKind::TextList
        }
    }

    fn execute(
        &mut self,
        _ctx: &RunContext,
        _node_id: egui_snarl::NodeId,
        inputs: Vec<Option<Value>>,
    ) -> Result<Vec<Value>, WorkflowError> {
        use TextSplit::*;
        let Self { value, delim, .. } = self;

        let text = match &inputs[0] {
            Some(Value::Text(text)) => text.clone(),
            Some(Value::Message(message)) => Arc::new(message_text(message)),
            None => value.clone(),
            _ => unreachable!(),
        };

        match delim {
            None => Ok(vec![Value::Text(text.clone())]),
            Some(Lines) => Ok(vec![Value::TextList(
                text.split("\n")
                    .filter(|it| !it.is_empty())
                    .map(|it| Arc::new(it.trim().to_string()))
                    .collect(),
            )]),
            Some(Paragraphs) => Ok(vec![Value::TextList(
                text.split("\n\n")
                    .filter(|it| !it.is_empty())
                    .map(|it| Arc::new(it.trim().to_string()))
                    .collect(),
            )]),
            Some(Words) => Ok(vec![Value::TextList(
                text.split_whitespace()
                    .filter(|it| !it.is_empty())
                    .map(|it| Arc::new(it.to_string()))
                    .collect(),
            )]),
            Some(Documents) => Ok(vec![Value::TextList(
                text.split("\n---\n")
                    .filter(|it| !it.is_empty())
                    .map(|it| Arc::new(it.trim().to_string()))
                    .collect(),
            )]),
        }
    }
}

impl UiNode for Text {
    fn title(&self) -> &str {
        "Text"
    }

    fn show_input(
        &mut self,
        ui: &mut egui::Ui,
        _ctx: &EditContext,
        pin_id: usize,
        remote: Option<Value>,
    ) -> egui_snarl::ui::PinInfo {
        if remote.is_none() {
            egui::Frame::new().inner_margin(4).show(ui, |ui| {
                resizable_frame(&mut self.size, ui, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        let text = Arc::make_mut(&mut self.value);
                        let widget = egui::TextEdit::multiline(text)
                            .desired_width(f32::INFINITY)
                            .hint_text("Enter text \u{1F64B}");

                        squelch(ui.add_sized(ui.available_size(), widget));
                    });
                });
            });
        } else {
            ui.label("input");
        }

        self.in_kinds(pin_id).first().unwrap().default_pin()
    }

    fn has_body(&self) -> bool {
        true
    }

    fn show_body(&mut self, ui: &mut egui::Ui, _ctx: &EditContext) {
        use TextSplit::*;
        let Self { delim, .. } = self;

        egui::ComboBox::from_label("split")
            .selected_text(if delim.is_none() {
                String::new()
            } else {
                format!("{:?}", delim.as_ref().unwrap())
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(delim, None, "");
                ui.selectable_value(delim, Some(Lines), "Lines");
                ui.selectable_value(delim, Some(Paragraphs), "Paragraphs")
                    .on_hover_text("Paragraphs separated by blank lines");
                ui.selectable_value(delim, Some(Words), "Words");
                ui.selectable_value(delim, Some(Documents), "Documents")
                    .on_hover_text("Documents separated by triple dash (---)");
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

#[typetag::serde]
impl FlexNode for Preview {}

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

    fn uuid(&self) -> Option<uuid::Uuid> {
        Some(self.uuid.0)
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
                egui::ScrollArea::vertical()
                    .auto_shrink(false)
                    .show(ui, |ui| {
                        match &ctx.previews.value(self.uuid.0).unwrap_or_default() {
                            Value::Text(text) => {
                                ui.add(egui::Label::new(text.as_str()).wrap());
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

                                    {
                                        let layout_job =
                                            egui_extras::syntax_highlighting::highlight(
                                                ui.ctx(),
                                                ui.style(),
                                                &theme,
                                                &text,
                                                language,
                                            );
                                        ui.add(egui::Label::new(layout_job).selectable(true).wrap())
                                    };
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

#[typetag::serde]
impl FlexNode for OutputNode {}

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

#[typetag::serde]
impl FlexNode for Panic {}

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
