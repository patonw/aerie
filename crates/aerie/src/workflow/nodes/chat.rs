use std::{borrow::Cow, sync::Arc};

use decorum::E64;
use egui::TextEdit;
use itertools::Itertools;
use rig::agent::PromptRequest;
use serde::{Deserialize, Serialize};

use crate::{
    ChatContent, ChatHistory, Toolset,
    utils::{CowExt as _, message_text},
    workflow::{
        WorkflowError,
        nodes::{MIN_HEIGHT, MIN_WIDTH},
    },
};

use super::{DynNode, EditContext, RunContext, UiNode, Value, ValueKind};
// TODO: Hash & eq by hand to ignore chat
#[derive(Debug, Clone, Default, Hash, PartialEq, Eq, Deserialize, Serialize)]
pub struct ChatNode {
    pub prompt: String,

    #[serde(skip)]
    pub history: Arc<ChatHistory>,
}

impl DynNode for ChatNode {
    fn inputs(&self) -> usize {
        3
    }

    fn outputs(&self) -> usize {
        2
    }

    fn in_kinds(&self, in_pin: usize) -> &'static [ValueKind] {
        match in_pin {
            0 => &[ValueKind::Agent],
            1 => &[ValueKind::Chat],
            2 => &[ValueKind::Text, ValueKind::Message],
            _ => ValueKind::all(),
        }
    }

    fn out_kind(&self, out_pin: usize) -> ValueKind {
        match out_pin {
            0 => ValueKind::Chat,
            1 => ValueKind::Message,
            _ => unreachable!(),
        }
    }

    fn value(&self, out_pin: usize) -> Value {
        match out_pin {
            0 => Value::Chat(self.history.clone()),
            1 => {
                if let Some(entry) = self.history.last()
                    && let ChatContent::Message(message) = &entry.content
                {
                    Value::Message(message.clone())
                } else {
                    Value::Placeholder(ValueKind::Message)
                }
            }
            _ => unreachable!(),
        }
    }
}

impl UiNode for ChatNode {
    fn title(&self) -> &str {
        "Chat"
    }

    fn tooltip(&self) -> &str {
        "Invoke an LLM completion model in conversation mode"
    }

    fn show_output(
        &mut self,
        ui: &mut egui::Ui,
        _ctx: &EditContext,
        pin_id: usize,
    ) -> egui_snarl::ui::PinInfo {
        match pin_id {
            0 => {
                ui.label("conversation");
            }
            1 => {
                ui.label("response");
            }
            _ => unreachable!(),
        }
        self.out_kind(pin_id).default_pin()
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
                ui.label("agent");
            }
            1 => {
                ui.label("conversation");
            }
            2 => {
                return ui
                    .vertical(|ui| {
                        if remote.is_none() {
                            egui::Resize::default()
                                .id_salt("prompt_resize")
                                .min_width(MIN_WIDTH)
                                .min_height(MIN_HEIGHT)
                                .with_stroke(false)
                                .show(ui, |ui| {
                                    let widget = egui::TextEdit::multiline(&mut self.prompt)
                                        .id_salt("prompt")
                                        .desired_width(f32::INFINITY)
                                        .hint_text("Prompt");

                                    ui.add_sized(ui.available_size(), widget)
                                        .on_hover_text("Prompt");
                                });
                            self.ghost_pin(ValueKind::Text.color())
                        } else {
                            ui.label("prompt");
                            ValueKind::Text.default_pin()
                        }
                    })
                    .inner;
            }
            _ => unreachable!(),
        };

        self.in_kinds(pin_id).first().unwrap().default_pin()
    }
}

impl ChatNode {
    pub async fn forward(
        &mut self,
        run_ctx: &RunContext,
        inputs: Vec<Option<Value>>,
    ) -> Result<(), WorkflowError> {
        self.validate(&inputs)?;

        let agent_spec = match &inputs[0] {
            Some(Value::Agent(spec)) => spec,
            None => Err(WorkflowError::Input(vec!["Agent spec required".into()]))?,
            _ => unreachable!(),
        };
        let chat = match &inputs[1] {
            Some(Value::Chat(history)) => history,
            None => Err(WorkflowError::Input(vec!["Chat history required".into()]))?,
            _ => unreachable!(),
        };

        let prompt = match &inputs[2] {
            Some(Value::Text(text)) => text.clone(),
            Some(Value::Message(msg)) => message_text(msg),
            None => self.prompt.clone(),
            _ => unreachable!(),
        };

        let agent = agent_spec.agent(&run_ctx.agent_factory)?;

        let mut history = chat.iter_msgs().cloned().collect_vec();
        let last_idx = history.len();
        let request = PromptRequest::new(&agent, &prompt)
            .multi_turn(5)
            .with_history(&mut history);

        match request.await {
            Ok(_) => {
                let mut chat = Cow::Borrowed(chat.as_ref());
                for msg in history.into_iter().skip(last_idx) {
                    chat = chat.try_moo(|c| c.push(Ok(msg).into(), None::<String>))?;
                }

                self.history = Arc::new(chat.into_owned());
            }
            Err(err) => Err(WorkflowError::Provider(err.into()))?,
        }

        Ok(())
    }
}

// TODO: Hash & eq by hand to ignore chat
#[derive(Debug, Clone, Default, Hash, PartialEq, Eq, Deserialize, Serialize)]
pub struct LLM {
    pub preamble: String,

    pub prompt: String,

    pub model: String,

    pub temperature: E64,

    #[serde(skip)]
    pub tools: Arc<Toolset>,

    #[serde(skip)]
    pub chat: Arc<ChatHistory>,
}

impl DynNode for LLM {
    fn inputs(&self) -> usize {
        7
    }

    fn outputs(&self) -> usize {
        2
    }

    fn in_kinds(&self, in_pin: usize) -> &'static [ValueKind] {
        match in_pin {
            0 => &[ValueKind::Chat],
            1 => &[ValueKind::Model],
            2 => &[ValueKind::Number],
            3 => &[ValueKind::Toolset],
            4 => &[ValueKind::Text],
            5 => &[ValueKind::Text, ValueKind::Message],
            6 => &[ValueKind::Text, ValueKind::Message],
            _ => ValueKind::all(),
        }
    }

    fn out_kind(&self, out_pin: usize) -> ValueKind {
        match out_pin {
            0 => ValueKind::Chat,
            1 => ValueKind::Message,
            _ => unreachable!(),
        }
    }

    fn value(&self, out_pin: usize) -> Value {
        match out_pin {
            0 => Value::Chat(self.chat.clone()),
            1 => {
                if let Some(entry) = self.chat.last()
                    && let ChatContent::Message(message) = &entry.content
                {
                    Value::Message(message.clone())
                } else {
                    Value::Placeholder(ValueKind::Message)
                }
            }
            _ => unreachable!(),
        }
    }
}

impl UiNode for LLM {
    fn title(&self) -> &str {
        "LLM (Deprecated)"
    }

    fn tooltip(&self) -> &str {
        "Invoke an LLM completion model in conversation mode"
    }

    fn show_output(
        &mut self,
        ui: &mut egui::Ui,
        _ctx: &EditContext,
        pin_id: usize,
    ) -> egui_snarl::ui::PinInfo {
        match pin_id {
            0 => {
                ui.label("conversation");
            }
            1 => {
                ui.label("response");
            }
            _ => unreachable!(),
        }
        self.out_kind(pin_id).default_pin()
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
                ui.label("conversation");
            }
            1 => {
                if remote.is_none() {
                    ui.add(TextEdit::singleline(&mut self.model).hint_text("provider/model:tag"));
                } else {
                    ui.label("model");
                }
            }
            2 => {
                if remote.is_none() {
                    let mut temp = self.temperature.into_inner();

                    let widget = egui::Slider::new(&mut temp, 0.0..=1.0);
                    ui.add(widget);

                    self.temperature = E64::assert(temp);
                    ui.label("T").on_hover_text("temperature");
                } else {
                    ui.label("temperature");
                }
            }
            3 => {
                ui.label("tools");
            }
            4 => {
                return ui
                    .vertical(|ui| {
                        if remote.is_none() {
                            egui::Resize::default()
                                .id_salt("preamble_resize")
                                .min_width(MIN_WIDTH)
                                .min_height(MIN_HEIGHT)
                                .with_stroke(false)
                                .show(ui, |ui| {
                                    let widget = egui::TextEdit::multiline(&mut self.preamble)
                                        .id_salt("preamble")
                                        .desired_width(f32::INFINITY)
                                        .hint_text("Preamble");

                                    ui.add_sized(ui.available_size(), widget)
                                        .on_hover_text("Preamble");
                                });
                            self.ghost_pin(ValueKind::Text.color())
                        } else {
                            ui.label("preamble");
                            ValueKind::Text.default_pin()
                        }
                    })
                    .inner;
            }
            5 => {
                return ui
                    .vertical(|ui| {
                        if remote.is_none() {
                            egui::Resize::default()
                                .id_salt("prompt_resize")
                                .min_width(MIN_WIDTH)
                                .min_height(MIN_HEIGHT)
                                .with_stroke(false)
                                .show(ui, |ui| {
                                    let widget = egui::TextEdit::multiline(&mut self.prompt)
                                        .id_salt("prompt")
                                        .desired_width(f32::INFINITY)
                                        .hint_text("Prompt");

                                    ui.add_sized(ui.available_size(), widget)
                                        .on_hover_text("Prompt");
                                });
                            self.ghost_pin(ValueKind::Text.color())
                        } else {
                            ui.label("prompt");
                            ValueKind::Text.default_pin()
                        }
                    })
                    .inner;
            }
            6 => {
                ui.label("context");
            }
            _ => unreachable!(),
        };

        self.in_kinds(pin_id).first().unwrap().default_pin()
    }

    fn show_body(&mut self, ui: &mut egui::Ui, _ctx: &EditContext) {
        // TODO: special case for chat history: show current branch
        egui::Frame::new().inner_margin(4).show(ui, |ui| {
            egui::Resize::default()
                .min_width(MIN_WIDTH)
                .min_height(MIN_WIDTH)
                .with_stroke(false)
                .show(ui, |ui| {
                    ui.label("");
                });
        });
    }
}

impl LLM {
    pub async fn forward(
        &mut self,
        run_ctx: &RunContext,
        inputs: Vec<Option<Value>>,
    ) -> Result<(), WorkflowError> {
        self.validate(&inputs)?;

        let chat = match &inputs[0] {
            Some(Value::Chat(history)) => history,
            None => Err(WorkflowError::Input(vec!["Chat history required".into()]))?,
            _ => unreachable!(),
        };

        let model = match &inputs[1] {
            Some(Value::Model(name)) => name.as_str(),
            None => self.model.as_str(),
            _ => unreachable!(),
        };

        if model.is_empty() {
            Err(WorkflowError::Input(vec![
                "Completion model not set".into(),
            ]))?;
        }

        let temperature = match &inputs[2] {
            Some(Value::Number(temp)) => *temp,
            None => self.temperature,
            _ => unreachable!(),
        };

        let toolset = match &inputs[3] {
            Some(Value::Toolset(tools)) => Some(tools.clone()),
            None => None,
            _ => unreachable!(),
        };

        let preamble = match &inputs[4] {
            Some(Value::Text(text)) => text.as_str(),
            None => self.preamble.as_str(),
            _ => unreachable!(),
        };

        let prompt = match &inputs[5] {
            Some(Value::Text(text)) => text.clone(),
            Some(Value::Message(msg)) => message_text(msg),
            None => self.prompt.clone(),
            _ => unreachable!(),
        };

        let context = match &inputs[5] {
            Some(Value::Text(text)) => text.clone(),
            Some(Value::Message(msg)) => message_text(msg),
            None => "".to_string(),
            _ => unreachable!(),
        };

        let mut agent = run_ctx
            .agent_factory
            .agent_builder(model)?
            .temperature(temperature.into())
            .context(&context)
            .preamble(preamble);

        if let Some(tools) = toolset {
            agent = run_ctx.agent_factory.toolbox.apply(agent, &tools);
        }

        let agent = agent.build();

        let mut history = chat.iter_msgs().cloned().collect_vec();
        let last_idx = history.len();
        let request = PromptRequest::new(&agent, &prompt)
            .multi_turn(5)
            .with_history(&mut history);

        match request.await {
            Ok(_) => {
                let mut chat = Cow::Borrowed(chat.as_ref());
                for msg in history.into_iter().skip(last_idx) {
                    chat = chat.try_moo(|c| c.push(Ok(msg).into(), None::<String>))?;
                }

                // let conversation = chat
                //     .push(Ok(Message::user(&prompt)).into(), None::<String>)?
                //     .try_moo(|c| c.push(Ok(Message::assistant(resp)).into(), None::<String>))?
                //     .into_owned();

                self.chat = Arc::new(chat.into_owned());
            }
            Err(err) => Err(WorkflowError::Provider(err.into()))?,
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Default, Hash, PartialEq, Eq, Deserialize, Serialize)]
pub struct GraftChat {
    #[serde(skip)]
    pub chat: Arc<ChatHistory>,
}

impl DynNode for GraftChat {
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

impl UiNode for GraftChat {
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

impl GraftChat {
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
pub struct MaskChat {
    pub limit: usize,

    #[serde(skip)]
    pub chat: Arc<ChatHistory>,
}

impl DynNode for MaskChat {
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

impl UiNode for MaskChat {
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

impl MaskChat {
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
