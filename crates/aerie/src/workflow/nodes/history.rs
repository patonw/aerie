use std::{borrow::Cow, sync::Arc};

use itertools::Itertools as _;
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

use crate::{
    ChatContent,
    rig::{
        self, OneOrMany,
        message::{AssistantContent, Message, ToolCall, ToolFunction},
    },
    utils::{EVec2, ImageResolver},
    workflow::{DynNode, FlexNode, RunContext, Value, ValueKind, WorkflowError},
};

#[cfg(feature = "ui")]
use crate::{
    ui::{resizable_frame, shortcuts::squelch},
    workflow::{EditContext, UiNode},
};

#[derive(Debug, Clone, Default, Hash, PartialEq, Eq, Deserialize, Serialize)]
pub struct GraftHistory {}

#[typetag::serde]
impl FlexNode for GraftHistory {}

impl DynNode for GraftHistory {
    fn title(&self) -> &str {
        "Side Conversation"
    }

    fn inputs(&self) -> usize {
        2
    }

    fn in_kinds(&'_ self, _in_pin: usize) -> Cow<'_, [ValueKind]> {
        Cow::Borrowed(&[ValueKind::Chat])
    }

    fn out_kind(&self, _out_pin: usize) -> ValueKind {
        ValueKind::Chat
    }

    fn execute(
        &mut self,
        _ctx: &RunContext,
        _node_id: egui_snarl::NodeId,
        inputs: Vec<Option<Value>>,
    ) -> Result<Vec<Value>, WorkflowError> {
        self.validate(&inputs)?;

        let chat = match &inputs[0] {
            Some(Value::Chat(history)) => history,
            None => Err(WorkflowError::Required(vec![
                "Chat history required".into(),
            ]))?,
            _ => unreachable!(),
        };

        let aside = match &inputs[1] {
            Some(Value::Chat(history)) => history,
            None => Err(WorkflowError::Required(vec![
                "Chat history required".into(),
            ]))?,
            _ => unreachable!(),
        };

        let common = chat
            .with_base(None)
            .find_common(aside.with_base(None).as_ref());

        let result = chat.aside(aside.with_base(common).iter().map(|it| it.content.clone()))?;
        let chat = Arc::new(result.into_owned());

        Ok(vec![Value::Chat(chat)])
    }
}

#[cfg(feature = "ui")]
impl UiNode for GraftHistory {
    fn show_input(
        &mut self,
        ui: &mut egui::Ui,
        _ctx: &EditContext,
        pin_id: usize,
        _remote: Option<Value>,
    ) -> egui_snarl::ui::PinInfo {
        match pin_id {
            0 => ui.label("main"),
            1 => ui.label("aside"),
            _ => unreachable!(),
        };

        self.in_kinds(pin_id).first().unwrap().default_pin()
    }
}

#[derive(Debug, Clone, Default, Hash, PartialEq, Eq, Deserialize, Serialize)]
pub struct MaskHistory {
    pub limit: usize,
}

#[typetag::serde]
impl FlexNode for MaskHistory {}

impl DynNode for MaskHistory {
    fn title(&self) -> &str {
        "Mask Chat"
    }

    fn inputs(&self) -> usize {
        2
    }

    fn in_kinds(&'_ self, in_pin: usize) -> Cow<'_, [ValueKind]> {
        Cow::Borrowed(match in_pin {
            0 => &[ValueKind::Chat],
            1 => &[ValueKind::Integer],
            _ => unreachable!(),
        })
    }

    fn out_kind(&self, _out_pin: usize) -> ValueKind {
        ValueKind::Chat
    }

    fn execute(
        &mut self,
        _ctx: &RunContext,
        _node_id: egui_snarl::NodeId,
        inputs: Vec<Option<Value>>,
    ) -> Result<Vec<Value>, WorkflowError> {
        self.validate(&inputs)?;

        let chat = match &inputs[0] {
            Some(Value::Chat(history)) => history,
            None => Err(WorkflowError::Required(vec![
                "Chat history required".into(),
            ]))?,
            _ => unreachable!(),
        };

        let limit = match &inputs[1] {
            Some(Value::Integer(limit)) => *limit as usize,
            None => self.limit,
            _ => unreachable!(),
        };

        let masked = if (0..100).contains(&limit)
            && let Some((_i, entry)) = chat
                .with_base(None)
                .rev_iter()
                .enumerate()
                .find(|(i, _)| *i == limit)
        {
            tracing::trace!("Setting base to {entry:?}");
            chat.with_base(Some(entry.id))
        } else {
            chat.with_base(None)
        };

        let chat = Arc::new(masked.into_owned());

        Ok(vec![Value::Chat(chat)])
    }
}

#[cfg(feature = "ui")]
impl UiNode for MaskHistory {
    fn tooltip(&self) -> &str {
        "Non-destructively limit the number of history entries visible.\nCan also remove an existing mask."
    }

    fn show_input(
        &mut self,
        ui: &mut egui::Ui,
        _ctx: &EditContext,
        pin_id: usize,
        remote: Option<Value>, // TODO: rename to "wired" this should be ValueKind!
    ) -> egui_snarl::ui::PinInfo {
        match pin_id {
            0 => {
                ui.label("conversation");
            }
            1 => {
                if remote.is_none() {
                    let widget = egui::Slider::new(&mut self.limit, 0..=100);
                    ui.add(widget);

                    ui.label("#")
                } else {
                    ui.label("limit")
                }
                .on_hover_text(
                    "Number of entries to expose. If 100 or more, shows, the entire history.",
                );
            }
            _ => unreachable!(),
        }
        self.in_kinds(pin_id).first().unwrap().default_pin()
    }
}

#[derive(Debug, Clone, Default, Hash, PartialEq, Eq, Deserialize, Serialize)]
pub enum MessageKind {
    #[default]
    User,
    Assistant,
    ToolCall,
    ToolResult,
    Error,
}

impl MessageKind {
    pub fn iter() -> impl Iterator<Item = Self> {
        [
            Self::Error,
            Self::User,
            Self::Assistant,
            Self::ToolCall,
            Self::ToolResult,
        ]
        .into_iter()
    }
}

#[skip_serializing_none]
#[derive(Debug, Clone, Default, Hash, PartialEq, Eq, Deserialize, Serialize)]
pub struct CreateMessage {
    kind: MessageKind,
    content: String,
    size: Option<EVec2>,
}

#[typetag::serde]
impl FlexNode for CreateMessage {}

impl DynNode for CreateMessage {
    fn title(&self) -> &str {
        "Create Message"
    }

    fn inputs(&self) -> usize {
        2
    }

    fn in_kinds(&'_ self, in_pin: usize) -> Cow<'_, [ValueKind]> {
        match in_pin {
            0 => Cow::Borrowed(&[ValueKind::Text, ValueKind::Json]),
            1 => Cow::Borrowed(&[
                ValueKind::Images,
                ValueKind::TextList,
                ValueKind::Text,
                ValueKind::Json,
            ]),
            _ => unreachable!(),
        }
    }

    fn out_kind(&self, _out_pin: usize) -> ValueKind {
        ValueKind::Message
    }

    fn execute(
        &mut self,
        _ctx: &RunContext,
        _node_id: egui_snarl::NodeId,
        inputs: Vec<Option<Value>>,
    ) -> Result<Vec<Value>, WorkflowError> {
        self.validate(&inputs)?;

        let value = match self.kind {
            MessageKind::ToolCall => {
                let data =
                    match &inputs[0] {
                        Some(Value::Text(text)) => {
                            Arc::new(serde_json::from_str(text).map_err(|e| {
                                WorkflowError::Conversion(format!("Invalid JSON: {e:?}"))
                            })?)
                        }
                        Some(Value::Json(data)) => data.clone(),
                        None if self.content.is_empty() => {
                            Err(WorkflowError::Required(vec!["content required".into()]))?
                        }
                        None => Arc::new(serde_json::from_str(&self.content).map_err(|e| {
                            WorkflowError::Conversion(format!("Invalid JSON: {e:?}"))
                        })?),
                        _ => unreachable!(),
                    };

                let content = if let Ok(tool_call) =
                    serde_json::from_value::<ToolCall>(data.as_ref().clone())
                {
                    AssistantContent::ToolCall(tool_call)
                } else if let Ok(tool_func) =
                    serde_json::from_value::<ToolFunction>(data.as_ref().clone())
                {
                    AssistantContent::ToolCall(ToolCall::new(String::default(), tool_func))
                } else {
                    AssistantContent::tool_call("", "", data.as_ref().clone())
                };

                Some(ChatContent::Message(Arc::new(Message::Assistant {
                    id: None,
                    content: OneOrMany::one(content),
                })))
            }
            _ => {
                let text = match &inputs[0] {
                    Some(Value::Text(text)) => text.clone(),
                    Some(Value::Json(data)) => {
                        Arc::new(serde_json::to_string_pretty(data).unwrap())
                    }
                    None if self.content.is_empty() => {
                        Err(WorkflowError::Required(vec!["content required".into()]))?
                    }
                    None => Arc::new(self.content.clone()),
                    _ => unreachable!(),
                };

                let images: im::Vector<_> = match &inputs[1] {
                    Some(Value::TextList(texts)) => texts
                        .iter()
                        .map(|url| ImageResolver::default().to_rig_image(url))
                        .collect::<anyhow::Result<_>>()?,
                    Some(Value::Text(text)) => {
                        im::vector![ImageResolver::default().to_rig_image(text)?]
                    }
                    Some(Value::Json(data)) if data.is_array() => data
                        .as_array()
                        .unwrap()
                        .iter()
                        .filter_map(|it| it.as_str())
                        .map(|s| ImageResolver::default().to_rig_image(s))
                        .collect::<anyhow::Result<_>>()?,
                    Some(Value::Json(data)) if data.is_string() => {
                        im::vector![ImageResolver::default().to_rig_image(data.as_str().unwrap())?]
                    }
                    Some(Value::Images(data)) => data.clone(),
                    None => Default::default(),
                    _ => unreachable!(),
                };

                Some(match self.kind {
                    MessageKind::Error => ChatContent::Error {
                        err: (*text).clone(),
                    },
                    MessageKind::User => {
                        let mut content =
                            rig::OneOrMany::one(rig::message::UserContent::text(&*text));

                        for image in images {
                            content.push(rig::message::UserContent::Image(image));
                        }

                        let msg = Message::User { content };
                        ChatContent::Message(Arc::new(msg))
                    }
                    MessageKind::Assistant => {
                        let mut content =
                            rig::OneOrMany::one(rig::message::AssistantContent::text(&*text));

                        for image in images {
                            content.push(rig::message::AssistantContent::Image(image));
                        }

                        let msg = Message::Assistant { id: None, content };
                        ChatContent::Message(Arc::new(msg))
                    }
                    MessageKind::ToolResult => {
                        let message = Message::tool_result("", &*text);
                        ChatContent::Message(Arc::new(message))
                    }
                    _ => unreachable!(),
                })
            }
        };

        let Some(value) = &value else {
            return Ok(vec![Value::Placeholder(ValueKind::Message)]);
        };

        let msg = match value {
            ChatContent::Message(message) => message.clone(),
            ChatContent::Error { err } => Arc::new(Message::user(format!("Error:\n{err:?}"))),
            _ => unreachable!(),
        };

        Ok(vec![Value::Message(msg)])
    }
}

#[cfg(feature = "ui")]
impl UiNode for CreateMessage {
    fn weak_eq(&self, other: &dyn std::any::Any) -> bool {
        other
            .downcast_ref::<Self>()
            .is_some_and(|other| self.kind == other.kind && self.content == other.content)
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
                            let widget = egui::TextEdit::multiline(&mut self.content)
                                .id_salt("message content")
                                .desired_width(f32::INFINITY);

                            squelch(
                                ui.add_sized(ui.available_size(), widget)
                                    .on_hover_text("message content"),
                            );
                        });
                    });
                } else {
                    ui.label("content");
                }
            }
            1 => {
                ui.label("images").on_hover_text("image URLs");
            }
            _ => unreachable!(),
        };

        self.in_kinds(pin_id).first().unwrap().default_pin()
    }

    fn has_body(&self) -> bool {
        true
    }

    fn show_body(&mut self, ui: &mut egui::Ui, _ctx: &EditContext) {
        egui::ComboBox::from_label("kind")
            .selected_text(format!("{:?}", self.kind))
            .show_ui(ui, |ui| {
                for kind in MessageKind::iter() {
                    let name = format!("{:?}", &kind);
                    ui.selectable_value(&mut self.kind, kind, name);
                }
            });
    }
}

#[derive(Debug, Clone, Default, Hash, PartialEq, Eq, Deserialize, Serialize)]
pub struct ExtendHistory {
    pub count: usize,
}

#[typetag::serde]
impl FlexNode for ExtendHistory {}

impl DynNode for ExtendHistory {
    fn title(&self) -> &str {
        "Extend History"
    }

    fn inputs(&self) -> usize {
        self.count + 2 // Slot for history plus empty to add new msg
    }

    fn in_kinds(&'_ self, in_pin: usize) -> Cow<'_, [ValueKind]> {
        Cow::Borrowed(match in_pin {
            0 => &[ValueKind::Chat],
            _ => &[ValueKind::Message],
        })
    }

    fn out_kind(&self, _out_pin: usize) -> ValueKind {
        ValueKind::Chat
    }

    fn execute(
        &mut self,
        _ctx: &RunContext,
        _node_id: egui_snarl::NodeId,
        inputs: Vec<Option<Value>>,
    ) -> Result<Vec<Value>, WorkflowError> {
        self.validate(&inputs)?;

        let history = match &inputs[0] {
            Some(Value::Chat(history)) => history.clone(),
            None => Default::default(),
            _ => unreachable!(),
        };

        let messages = inputs
            .into_iter()
            .skip(1)
            .take(self.count)
            .filter_map(|it| match it {
                Some(Value::Message(value)) => Some(value),
                None => None,
                _ => unreachable!(),
            })
            .collect_vec();

        let extended = history.extend(messages.into_iter().map(|msg| Ok(msg.clone()).into()))?;
        let value = Arc::new(extended.into_owned());

        Ok(vec![Value::Chat(value)])
    }
}

#[cfg(feature = "ui")]
impl UiNode for ExtendHistory {
    fn show_input(
        &mut self,
        ui: &mut egui::Ui,
        _ctx: &EditContext,
        pin_id: usize,
        remote: Option<Value>,
    ) -> egui_snarl::ui::PinInfo {
        if pin_id == self.count + 1 && remote.is_some() {
            self.count += 1;
        } else if pin_id != 0 && pin_id == self.count && remote.is_none() {
            self.count -= 1;
        }

        if pin_id == 0 {
            ui.label("history");
        } else if pin_id < self.count + 1 {
            ui.label(format!("{pin_id}"));
        }

        self.in_kinds(pin_id).first().unwrap().default_pin()
    }
}

#[cfg(feature = "ui")]
fn history_node_menu(
    ui: &mut egui::Ui,
    snarl: &mut egui_snarl::Snarl<super::WorkNode>,
    pos: egui::Pos2,
) {
    ui.menu_button("History", |ui| {
        if ui.button("Create Message").clicked() {
            snarl.insert_node(pos.into(), CreateMessage::default().into());
            ui.close();
        }

        if ui.button("Mask History").clicked() {
            snarl.insert_node(pos.into(), MaskHistory::default().into());
            ui.close();
        }

        if ui.button("Extend History").clicked() {
            snarl.insert_node(pos.into(), ExtendHistory::default().into());
            ui.close();
        }

        if ui.button("Side Chat").clicked() {
            snarl.insert_node(pos.into(), GraftHistory::default().into());
            ui.close();
        }
    });
}

#[cfg(feature = "ui")]
inventory::submit! {
    super::GraphSubmenu("history", history_node_menu)
}
