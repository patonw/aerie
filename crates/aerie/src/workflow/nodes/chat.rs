use std::{borrow::Cow, sync::Arc};

use itertools::Itertools;
use rig::{
    agent::PromptRequest,
    completion::Completion,
    message::{AssistantContent, Message, ToolChoice},
};
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

use crate::{
    ChatContent, ChatHistory, ui::resizable_frame, utils::CowExt as _, workflow::WorkflowError,
};

use super::{DynNode, EditContext, RunContext, UiNode, Value, ValueKind};

// TODO: Hash & eq by hand to ignore chat
#[skip_serializing_none]
#[derive(Debug, Clone, Default, Hash, PartialEq, Eq, Deserialize, Serialize)]
pub struct ChatNode {
    pub prompt: String,

    pub size: Option<crate::utils::EVec2>,
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
        "Invoke an LLM completion model in conversation mode.\n\
            Automatically invokes tools and sends the results\n\
            back to the model for follow-up."
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
                            resizable_frame(&mut self.size, ui, |ui| {
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
            Some(Value::Text(text)) if !text.is_empty() => Message::user(text.clone()),
            Some(Value::Message(msg @ Message::User { .. })) => msg.clone(),
            Some(Value::Message(msg @ Message::Assistant { .. })) => {
                // Coerce into user message if we want to use another agent's output for cross talk
                Message::user(message_text(msg))
            }
            None if !self.prompt.is_empty() => Message::user(self.prompt.clone()),
            _ => Err(WorkflowError::Required(vec!["A prompt is required".into()]))?,
        };

        let agent = agent_spec.agent(&run_ctx.agent_factory)?;

        let mut history = chat.iter_msgs().map(|it| it.into_owned()).collect_vec();
        let last_idx = history.len();
        let request = PromptRequest::new(&agent, prompt)
            .multi_turn(5)
            .with_history(&mut history);

        match request.await {
            Ok(_) => {
                let mut chat = Cow::Borrowed(chat.as_ref());
                for msg in history.into_iter().skip(last_idx) {
                    chat = chat.try_moo(|c| c.push(Ok(msg).into()))?;
                }

                self.history = Arc::new(chat.into_owned());
            }
            Err(err) => Err(WorkflowError::Provider(err.into()))?,
        }

        Ok(())
    }
}

#[skip_serializing_none]
#[derive(Default, Debug, Clone, Deserialize, Serialize, Hash, PartialEq, Eq)]
pub struct StructuredChat {
    pub prompt: String,

    pub size: Option<crate::utils::EVec2>,

    /// If response does not conform to schema try again
    pub retries: usize,

    #[serde(skip)]
    pub history: Arc<ChatHistory>,

    #[serde(skip)]
    pub tool_name: String,

    #[serde(skip)]
    pub data: Arc<serde_json::Value>,
}

// outputs: chat, message, structured data
impl DynNode for StructuredChat {
    fn inputs(&self) -> usize {
        4
    }

    fn in_kinds(&self, in_pin: usize) -> &'static [ValueKind] {
        match in_pin {
            0 => &[ValueKind::Agent],
            1 => &[ValueKind::Chat],
            2 => &[ValueKind::Json],
            3 => &[ValueKind::Text, ValueKind::Message],
            _ => ValueKind::all(),
        }
    }

    fn outputs(&self) -> usize {
        4
    }

    fn out_kind(&self, out_pin: usize) -> ValueKind {
        match out_pin {
            0 => ValueKind::Chat,
            1 => ValueKind::Message,
            2 => ValueKind::Text,
            3 => ValueKind::Json,
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
            2 => Value::Text(self.tool_name.clone()),
            3 => super::Value::Json(self.data.clone()),
            _ => unreachable!(),
        }
    }
}

impl UiNode for StructuredChat {
    fn title(&self) -> &str {
        "Structured Output"
    }

    fn tooltip(&self) -> &str {
        "Use an LLM to produce structured data.\n\
            If a schema is provided, it will output data conforming to it.\n\
            Otherwise, the output will be arguments for the provided tool.\n\
            Tools are not invoked automatically in either case."
    }

    fn show_output(
        &mut self,
        ui: &mut egui::Ui,
        _ctx: &EditContext,
        pin_id: usize,
    ) -> egui_snarl::ui::PinInfo {
        match pin_id {
            0 => {
                ui.label("history");
            }
            1 => {
                ui.label("response");
            }
            2 => {
                ui.label("tool name");
            }
            3 => {
                ui.label("data");
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
                ui.label("history");
            }
            2 => {
                ui.label("schema");
            }
            3 => {
                return ui
                    .vertical(|ui| {
                        if remote.is_none() {
                            resizable_frame(&mut self.size, ui, |ui| {
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

    fn has_body(&self) -> bool {
        true
    }

    fn show_body(&mut self, ui: &mut egui::Ui, _ctx: &EditContext) {
        ui.add(egui::Slider::new(&mut self.retries, 0..=10).text("R"))
            .on_hover_text("retries");
    }
}

// TODO: investigate why errors in the MCP server cause a panic here
impl StructuredChat {
    pub async fn forward(
        &mut self,
        run_ctx: &RunContext,
        inputs: Vec<Option<Value>>,
    ) -> Result<(), WorkflowError> {
        self.validate(&inputs)?;

        let mut agent_spec = match &inputs[0] {
            Some(Value::Agent(spec)) => spec.clone(),
            None => Err(WorkflowError::Input(vec!["Agent is required".into()]))?,
            _ => unreachable!(),
        };

        let in_chat = match &inputs[1] {
            Some(Value::Chat(history)) => history,
            None => Err(WorkflowError::Input(vec!["Chat history required".into()]))?,
            _ => unreachable!(),
        };

        let schema = match &inputs[2] {
            Some(Value::Json(schema)) => Some(schema.clone()),
            None => None,
            _ => unreachable!(),
        };

        let prompt = match &inputs[3] {
            Some(Value::Text(text)) if !text.is_empty() => Message::user(text.clone()),
            Some(Value::Message(msg)) => msg.clone(),
            None if !self.prompt.is_empty() => Message::user(self.prompt.clone()),
            _ => Err(WorkflowError::Required(vec!["A prompt is required".into()]))?,
        };

        if let Some(schema) = &schema {
            Arc::make_mut(&mut agent_spec).schema(schema.clone());
        }

        let agent = agent_spec.agent(&run_ctx.agent_factory)?;

        let mut chat = Cow::Borrowed(in_chat.as_ref());

        let max_attempts = self.retries + 1;
        let mut attempts = 0;

        let result: Result<_, WorkflowError> = loop {
            let history = chat.iter_msgs().map(|it| it.into_owned()).collect_vec();
            let request = agent
                .completion(prompt.clone(), history)
                .await
                .unwrap()
                .tool_choice(ToolChoice::Required)
                .send();

            attempts += 1;
            match request.await {
                Ok(resp) => {
                    let (tool_calls, texts): (Vec<_>, Vec<_>) = resp
                        .choice
                        .iter()
                        .partition(|choice| matches!(choice, AssistantContent::ToolCall(_)));

                    tracing::debug!("Got tool calls {tool_calls:?} and texts {texts:?}");

                    let user_msg = prompt.clone();
                    let agent_msg = Message::Assistant {
                        id: None,
                        content: resp.choice.clone(),
                    };

                    chat = chat
                        .try_moo(|c| c.extend(vec![Ok(user_msg).into(), Ok(agent_msg).into()]))?;

                    if tool_calls.is_empty() {
                        if attempts > max_attempts {
                            break Err(WorkflowError::MissingToolCall);
                        } else {
                            tracing::warn!("No tool calls from LLM response. Retrying...");
                            chat =
                                chat.try_moo(|c| c.push_error(WorkflowError::MissingToolCall))?;
                            continue;
                        }
                    }

                    // // Monkey wrench
                    // if self.retries > 0 && attempts < 2 {
                    //     chat = chat.try_moo(|c| {
                    //         c.push_error(WorkflowError::Unknown("Just do it again".to_string()))
                    //     })?;
                    //     continue;
                    // }

                    if let Some(AssistantContent::ToolCall(tool_call)) = tool_calls.first() {
                        if let Some(schema) = schema.as_ref()
                            && let Err(err) =
                                jsonschema::validate(schema, &tool_call.function.arguments)
                        {
                            if attempts > max_attempts {
                                break Err(WorkflowError::Validation(err.to_owned()));
                            } else {
                                chat = chat.try_moo(|c| {
                                    c.push_error(WorkflowError::Validation(err.to_owned()))
                                })?;
                                continue;
                            }
                        }

                        // TODO: if a agentic tool call, fetch the schema from the toolbox
                        self.tool_name = tool_call.function.name.clone();
                        self.data = Arc::new(tool_call.function.arguments.clone());
                        break Ok(());
                    }
                }
                Err(err) if attempts > max_attempts => {
                    break Err(WorkflowError::Provider(err.into()));
                }
                Err(err) => {
                    tracing::warn!("LLM call failed on {err:?}. Retrying");
                    chat = chat.try_moo(|c| c.push_error(WorkflowError::Provider(err.into())))?;
                }
            }
        };

        if let Cow::Owned(history) = chat {
            self.history = Arc::new(history);
        } else {
            self.history = in_chat.clone();
        }

        tracing::info!("Final result {result:?} after {attempts} attempts");
        result
    }
}
