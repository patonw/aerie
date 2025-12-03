use std::sync::Arc;

use decorum::E64;
use itertools::Itertools;
use rig::{
    agent::{AgentBuilderSimple, PromptRequest},
    client::{CompletionClient as _, ProviderClient as _},
    message::Message,
    providers::ollama,
};
use serde::{Deserialize, Serialize};

use crate::{
    ChatContent, ChatHistory, Toolset,
    utils::CowExt as _,
    workflow::{
        WorkflowError,
        nodes::{MIN_HEIGHT, MIN_WIDTH},
    },
};

use super::{DynNode, EditContext, RunContext, UiNode, Value, ValueKind};

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
        6
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
            5 => &[ValueKind::Text],
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
        "Completion"
    }

    fn tooltip(&self) -> &str {
        "Invoke an LLM completion model"
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
                    // TODO: componentize and dynamic discovery
                    egui::ComboBox::from_label("model")
                        .selected_text(self.model.to_string())
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.model,
                                "devstral:latest".to_string(),
                                "Devstral",
                            );
                            ui.selectable_value(
                                &mut self.model,
                                "magistral:latest".to_string(),
                                "Magistral",
                            );
                            ui.selectable_value(
                                &mut self.model,
                                "my-qwen3-coder:30b".to_string(),
                                "Qwen3 Coder",
                            );
                        });
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
        ctx: &RunContext,
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
            Some(Value::Text(text)) => text.as_str(),
            None => self.prompt.as_str(),
            _ => unreachable!(),
        };

        let llm_client = ollama::Client::from_env();

        let mut agent = AgentBuilderSimple::new(llm_client.completion_model(model))
            .temperature(temperature.into())
            .preamble(preamble);

        if let Some(tools) = toolset {
            agent = ctx.toolbox.apply(agent, &tools);
        }

        let agent = agent.build();

        let mut history = chat.iter_msgs().cloned().collect_vec();
        let request = PromptRequest::new(&agent, prompt)
            .multi_turn(5)
            .with_history(&mut history);

        match request.await {
            Ok(resp) => {
                let conversation = chat
                    .push(Ok(Message::user(prompt)).into(), None::<String>)?
                    .try_moo(|c| c.push(Ok(Message::assistant(resp)).into(), None::<String>))?
                    .into_owned();

                self.chat = Arc::new(conversation);
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

        let common = chat.find_common(aside);

        let result = chat.aside(aside.with_base(common).iter().map(|it| it.content.clone()))?;
        self.chat = Arc::new(result.into_owned());

        Ok(())
    }
}
