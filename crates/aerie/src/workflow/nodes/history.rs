use std::sync::Arc;

use itertools::Itertools as _;
use rig::{
    OneOrMany,
    message::{AssistantContent, Message, ToolCall, ToolFunction},
};
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

use crate::{ChatContent, ChatHistory, ui::resizable_frame, utils::EVec2, workflow::WorkflowError};

use super::{DynNode, EditContext, RunContext, UiNode, Value, ValueKind};

#[derive(Debug, Clone, Default, Hash, PartialEq, Eq, Deserialize, Serialize)]
pub struct GraftHistory {
    #[serde(skip)]
    pub chat: Arc<ChatHistory>,
}

impl DynNode for GraftHistory {
    fn value(&self, _out_pin: usize) -> Value {
        Value::Chat(self.chat.clone())
    }

    fn inputs(&self) -> usize {
        2
    }

    fn in_kinds(&self, _in_pin: usize) -> &'static [ValueKind] {
        &[ValueKind::Chat]
    }

    fn out_kind(&self, _out_pin: usize) -> ValueKind {
        ValueKind::Chat
    }
}

impl UiNode for GraftHistory {
    fn title(&self) -> &str {
        "Side Conversation"
    }

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

impl GraftHistory {
    pub async fn forward(
        &mut self,
        _ctx: &RunContext,
        inputs: Vec<Option<Value>>,
    ) -> Result<(), WorkflowError> {
        self.validate(&inputs)?;

        let chat = match &inputs[0] {
            Some(Value::Chat(history)) => history,
            None => Err(WorkflowError::Input(vec!["Chat history required".into()]))?,
            _ => unreachable!(),
        };

        let aside = match &inputs[1] {
            Some(Value::Chat(history)) => history,
            None => Err(WorkflowError::Input(vec!["Chat history required".into()]))?,
            _ => unreachable!(),
        };

        let common = chat
            .with_base(None)
            .find_common(aside.with_base(None).as_ref());

        let result = chat.aside(aside.with_base(common).iter().map(|it| it.content.clone()))?;
        self.chat = Arc::new(result.into_owned());

        Ok(())
    }
}

#[derive(Debug, Clone, Default, Hash, PartialEq, Eq, Deserialize, Serialize)]
pub struct MaskHistory {
    pub limit: usize,

    #[serde(skip)]
    pub chat: Arc<ChatHistory>,
}

impl DynNode for MaskHistory {
    fn inputs(&self) -> usize {
        2
    }

    fn in_kinds(&self, in_pin: usize) -> &'static [ValueKind] {
        match in_pin {
            0 => &[ValueKind::Chat],
            1 => &[ValueKind::Integer],
            _ => unreachable!(),
        }
    }

    fn out_kind(&self, _out_pin: usize) -> ValueKind {
        ValueKind::Chat
    }

    fn value(&self, _out_pin: usize) -> Value {
        Value::Chat(self.chat.clone())
    }
}

impl UiNode for MaskHistory {
    fn title(&self) -> &str {
        "Mask Chat"
    }

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

impl MaskHistory {
    pub async fn forward(
        &mut self,
        _ctx: &RunContext,
        inputs: Vec<Option<Value>>,
    ) -> Result<(), WorkflowError> {
        self.validate(&inputs)?;

        let chat = match &inputs[0] {
            Some(Value::Chat(history)) => history,
            None => Err(WorkflowError::Input(vec!["Chat history required".into()]))?,
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
            tracing::debug!("Setting base to {entry:?}");
            chat.with_base(Some(entry.id))
        } else {
            chat.with_base(None)
        };

        self.chat = Arc::new(masked.into_owned());

        Ok(())
    }
}

#[derive(Debug, Clone, Default, Hash, PartialEq, Eq, Deserialize, Serialize)]
pub enum MessageKind {
    #[default]
    Error,
    User,
    Assistant,
    ToolCall,
    ToolResult,
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

    #[serde(skip)]
    value: Option<ChatContent>,
}

impl DynNode for CreateMessage {
    fn in_kinds(&self, _in_pin: usize) -> &'static [ValueKind] {
        &[ValueKind::Text, ValueKind::Json]
    }

    fn out_kind(&self, _out_pin: usize) -> ValueKind {
        ValueKind::Message
    }

    fn value(&self, _out_pin: usize) -> Value {
        let Some(value) = &self.value else {
            return Value::Placeholder(ValueKind::Message);
        };

        let msg = match value {
            ChatContent::Message(message) => message.clone(),
            ChatContent::Error { err } => Message::user(format!("Error:\n{err:?}")),
            _ => unreachable!(),
        };

        Value::Message(msg)
    }
}

impl UiNode for CreateMessage {
    fn title(&self) -> &str {
        "Create Message"
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

                            ui.add_sized(ui.available_size(), widget)
                                .on_hover_text("message content");
                        });
                    });
                } else {
                    ui.label("content");
                }
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

impl CreateMessage {
    pub async fn forward(
        &mut self,
        _ctx: &RunContext,
        inputs: Vec<Option<Value>>,
    ) -> Result<(), WorkflowError> {
        self.validate(&inputs)?;

        match self.kind {
            MessageKind::ToolCall => {
                let data =
                    match &inputs[0] {
                        Some(Value::Text(text)) => {
                            Arc::new(serde_json::from_str(text).map_err(|e| {
                                WorkflowError::Input(vec![format!("Invalid JSON: {e:?}")])
                            })?)
                        }
                        Some(Value::Json(data)) => data.clone(),
                        None if self.content.is_empty() => {
                            Err(WorkflowError::Input(vec!["content required".into()]))?
                        }
                        None => Arc::new(serde_json::from_str(&self.content).map_err(|e| {
                            WorkflowError::Input(vec![format!("Invalid JSON: {e:?}")])
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
                    AssistantContent::ToolCall(ToolCall {
                        id: String::default(),
                        call_id: None,
                        function: tool_func,
                    })
                } else {
                    AssistantContent::tool_call("", "", data.as_ref().clone())
                };

                self.value = Some(ChatContent::Message(Message::Assistant {
                    id: None,
                    content: OneOrMany::one(content),
                }));
            }
            _ => {
                let text = match &inputs[0] {
                    Some(Value::Text(text)) => text.clone(),
                    Some(Value::Json(data)) => serde_json::to_string_pretty(&data).unwrap(),
                    None if self.content.is_empty() => {
                        Err(WorkflowError::Input(vec!["content required".into()]))?
                    }
                    None => self.content.clone(),
                    _ => unreachable!(),
                };

                self.value = Some(match self.kind {
                    MessageKind::Error => ChatContent::Error { err: text },
                    MessageKind::User => ChatContent::Message(Message::user(text)),
                    MessageKind::Assistant => ChatContent::Message(Message::assistant(text)),
                    MessageKind::ToolResult => {
                        let message = Message::tool_result("", text);
                        ChatContent::Message(message)
                    }
                    _ => unreachable!(),
                });
            }
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Default, Hash, PartialEq, Eq, Deserialize, Serialize)]
pub struct ExtendHistory {
    pub count: usize,

    #[serde(skip)]
    pub history: Arc<ChatHistory>,
}

impl DynNode for ExtendHistory {
    fn inputs(&self) -> usize {
        self.count + 2 // Slot for history plus empty to add new msg
    }

    fn in_kinds(&self, in_pin: usize) -> &'static [ValueKind] {
        match in_pin {
            0 => &[ValueKind::Chat],
            _ => &[ValueKind::Message],
        }
    }

    fn out_kind(&self, _out_pin: usize) -> ValueKind {
        ValueKind::Chat
    }

    fn value(&self, _out_pin: usize) -> Value {
        Value::Chat(self.history.clone())
    }
}

impl UiNode for ExtendHistory {
    fn title(&self) -> &str {
        "Extend History"
    }

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

impl ExtendHistory {
    pub async fn forward(
        &mut self,
        _run_ctx: &RunContext,
        inputs: Vec<Option<Value>>,
    ) -> Result<(), WorkflowError> {
        self.validate(&inputs)?;

        let history = match &inputs[0] {
            Some(Value::Chat(history)) => history.clone(),
            None => Err(WorkflowError::Input(vec!["Chat history required".into()]))?,
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

        let extended = history.extend(messages.into_iter().map(|msg| Ok(msg).into()))?;
        self.history = Arc::new(extended.into_owned());

        Ok(())
    }
}
