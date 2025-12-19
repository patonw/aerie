use std::{
    borrow::Cow,
    sync::{Arc, atomic::Ordering},
};

use itertools::Itertools;
use rig::{
    OneOrMany,
    agent::PromptRequest,
    completion::{Completion, CompletionError, CompletionResponse},
    message::{AssistantContent, Message, Reasoning, ToolCall, ToolFunction, UserContent},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use serde_with::skip_serializing_none;

use crate::{
    ChatContent, ChatHistory,
    ui::resizable_frame,
    utils::{CowExt as _, extract_json, message_text},
    workflow::WorkflowError,
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
        3
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
            2 => ValueKind::Failure,
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
            2 => Value::Placeholder(ValueKind::Failure), // Runner handles the actual error values
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
            2 => {
                ui.label("failure");
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
            None => Err(WorkflowError::Required(vec!["Agent spec required".into()]))?,
            _ => unreachable!(),
        };
        let chat = match &inputs[1] {
            Some(Value::Chat(history)) => history,
            None => Err(WorkflowError::Required(vec![
                "Chat history required".into(),
            ]))?,
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

        let mut history = chat.iter_msgs().map(|it| it.into_owned()).collect_vec();
        let last_idx = history.len();

        let agent = agent_spec.agent(&run_ctx.agent_factory)?;

        let request = multi_turn_completion(run_ctx, &agent, prompt, &mut history);
        let prompt_request = request.await;
        match prompt_request {
            Ok(_) => {
                let mut chat = Cow::Borrowed(chat.as_ref());
                for msg in history.into_iter().skip(last_idx) {
                    // When we implement streaming, we can hold onto the pointer
                    // and update it incrementally.
                    if !run_ctx.streaming
                        && let Some(scratch) = &run_ctx.scratch
                    {
                        scratch.push_back(Ok(msg.clone()));
                    }

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

    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub extract: bool,

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
        5
    }

    fn out_kind(&self, out_pin: usize) -> ValueKind {
        match out_pin {
            0 => ValueKind::Chat,
            1 => ValueKind::Message,
            2 => ValueKind::Text,
            3 => ValueKind::Json,
            4 => ValueKind::Failure,
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
            3 => Value::Json(self.data.clone()),
            4 => Value::Placeholder(ValueKind::Failure),
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
            4 => {
                ui.label("failure");
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
        ui.vertical(|ui| {
            ui.add(egui::Slider::new(&mut self.retries, 0..=10).text("R"))
                .on_hover_text("retries");

            ui.checkbox(&mut self.extract, "extract").on_hover_text(
                "If the model fails to submit a proper tool call,\n\
                    Attempt to find tool arguments inside its text response.",
            );
        });
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
            None => Err(WorkflowError::Required(vec!["Agent is required".into()]))?,
            _ => unreachable!(),
        };

        let in_chat = match &inputs[1] {
            Some(Value::Chat(history)) => history,
            None => Err(WorkflowError::Required(vec![
                "Chat history required".into(),
            ]))?,
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

        if !run_ctx.streaming
            && let Some(scratch) = &run_ctx.scratch
        {
            scratch.push_back(Ok(prompt.clone()));
        }

        chat = chat.try_moo(|c| c.push(Ok(prompt).into()))?;

        let result: Result<_, WorkflowError> = loop {
            if run_ctx.interrupt.load(Ordering::Relaxed) {
                Err(WorkflowError::Interrupted)?;
            }

            // chat is the source of truth. history is just its shadow.
            let mut history = chat.iter_msgs().map(|it| it.into_owned()).collect_vec();
            // Use the last message as the prompt
            let current_prompt = history.pop().unwrap();

            let response =
                one_shot_completion(run_ctx, &agent, current_prompt, history.clone()).await;

            attempts += 1;
            match response {
                Ok(resp) => {
                    let agent_msg = Message::Assistant {
                        id: None,
                        content: resp.choice.clone(),
                    };

                    if !run_ctx.streaming
                        && let Some(scratch) = &run_ctx.scratch
                    {
                        scratch.push_back(Ok(agent_msg.clone()));
                    }

                    let (tool_calls, texts): (Vec<_>, Vec<_>) = resp
                        .choice
                        .iter()
                        .partition(|choice| matches!(choice, AssistantContent::ToolCall(_)));

                    tracing::debug!("Got tool calls {tool_calls:?} and texts {texts:?}");

                    let mut tool_func = tool_calls
                        .iter()
                        .filter_map(|call| match call {
                            AssistantContent::ToolCall(tool_call) => {
                                Some(tool_call.function.clone())
                            }
                            _ => None,
                        })
                        .next();

                    if self.extract && tool_func.is_none() {
                        tracing::info!(
                            "Did not receive a tool call. Attempting to find one in message text."
                        );

                        let text = message_text(&agent_msg);
                        tool_func = extract_json(&text, false);

                        if tool_func.is_none() {
                            // common mistakes:
                            let text = text
                                .replace("\"tool\"", "\"name\"")
                                .replace("\"function\"", "\"name\"")
                                .replace("\"parameters\"", "\"arguments\"");

                            tool_func = extract_json(&text, false);
                        }

                        // Still failed? If we asked for just an object try to find one to validate
                        if schema.is_some()
                            && tool_func.is_none()
                            && let Some(args) = extract_json::<serde_json::Value>(&text, false)
                        {
                            tool_func = Some(ToolFunction {
                                name: "???".into(),
                                arguments: args,
                            })
                        }
                    }

                    chat = chat.try_moo(|c| c.push(Ok(agent_msg).into()))?;

                    if let Some(tool_func) = tool_func {
                        if let Some(schema) = schema.as_ref()
                            && let Err(err) = jsonschema::validate(schema, &tool_func.arguments)
                        {
                            if attempts > max_attempts {
                                break Err(WorkflowError::Validation(err.to_owned()));
                            } else {
                                if let Some(scratch) = &run_ctx.scratch {
                                    scratch.push_back(Err(format!("{err:?}")));
                                }
                                chat = chat.try_moo(|c| {
                                    let validation = WorkflowError::Validation(err.to_owned());
                                    c.push_error(validation)
                                })?;
                                continue;
                            }
                        }

                        // TODO: if a agentic tool call, fetch the schema from the toolbox
                        self.tool_name = tool_func.name.clone();
                        self.data = Arc::new(tool_func.arguments.clone());
                        break Ok(());
                    } else if attempts > max_attempts {
                        break Err(WorkflowError::MissingToolCall);
                    } else {
                        tracing::warn!("No tool calls from LLM response. Retrying...");
                        if let Some(scratch) = &run_ctx.scratch {
                            scratch.push_back(Err(format!("{:?}", WorkflowError::MissingToolCall)));
                        }

                        chat = chat.try_moo(|c| c.push_error(WorkflowError::MissingToolCall))?;
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

async fn one_shot_completion(
    run_ctx: &RunContext,
    agent: &rig::agent::Agent<rig::client::completion::CompletionModelHandle<'static>>,
    prompt: Message,
    history: Vec<Message>,
) -> Result<CompletionResponse<()>, WorkflowError> {
    use futures_util::stream::StreamExt as _;
    use rig::{
        agent::Text,
        streaming::{StreamedAssistantContent, StreamingCompletion},
    };

    if !run_ctx.streaming {
        let mut request = agent
            .completion(prompt.clone(), history)
            .await
            .map_err(|e| WorkflowError::Provider(e.into()))?;

        if let Some(seed) = &run_ctx.seed {
            let value = seed.value.fetch_add(seed.increment, Ordering::Relaxed);
            request = request.additional_params(json!({"seed": value}));
        }

        return request
            .send()
            .await
            .map_err(|e| WorkflowError::Provider(e.into()));
    }

    let mut request = agent
        .stream_completion(prompt.clone(), history)
        .await
        .map_err(|e| WorkflowError::Provider(e.into()))?;

    if let Some(seed) = &run_ctx.seed {
        let value = seed.value.fetch_add(seed.increment, Ordering::Relaxed);
        request = request.additional_params(json!({"seed": value}));
    }

    let mut stream = request
        .stream()
        .await
        .map_err(|e| WorkflowError::Provider(e.into()))?;

    let mut texts = String::new();
    let mut reasonings = Vec::new();
    let mut tool_calls: Vec<ToolCall> = Vec::new();

    let agent_msg = if let Some(scratch) = &run_ctx.scratch {
        scratch.push_back(Ok(prompt.clone()));
        Some(scratch.push_back(Ok(Message::assistant(""))))
    } else {
        None
    };

    while let Some(content) = stream.next().await {
        if run_ctx.interrupt.load(Ordering::Relaxed) {
            Err(WorkflowError::Interrupted)?;
        }

        match content {
            Ok(item) => match item {
                StreamedAssistantContent::Text(text) => {
                    texts.push_str(&text.text);
                    let msg = Message::assistant(&texts);
                    if let Some(a) = &agent_msg {
                        a.store(Arc::new(Ok(msg)));
                    }
                }

                StreamedAssistantContent::ToolCall(tool_call) => {
                    tool_calls.push(tool_call.clone());
                }
                // No idea how to handle this case
                StreamedAssistantContent::ToolCallDelta { .. } => {
                    // Maybe we can just ignore. Seems like APIs that use this send
                    // a complete ToolCall at the end.
                    //
                    // Also, deltas seem to omit the actual tool name.
                    // Only sends back arguments.
                }
                StreamedAssistantContent::Reasoning(Reasoning { reasoning, .. }) => {
                    reasonings.extend(reasoning);
                }
                StreamedAssistantContent::Final(_) => {}
            },
            Err(_) => todo!(),
        }
    }

    // run_ctx.scratch.pop_back();

    let mut contents = vec![];
    if !reasonings.is_empty() {
        contents.push(AssistantContent::Reasoning(Reasoning::multi(reasonings)))
    }

    if !texts.is_empty() {
        contents.push(AssistantContent::Text(Text::from(&texts)));
    }

    if !tool_calls.is_empty() {
        contents.extend(tool_calls.into_iter().map(AssistantContent::ToolCall));
    }

    Ok(CompletionResponse {
        choice: OneOrMany::many(contents).unwrap(),
        usage: Default::default(),
        raw_response: (),
    })
}

use thiserror::Error;

#[derive(Debug, Error)]
enum StreamingError {
    #[error("WorkflowError: {0}")]
    Workflow(#[from] WorkflowError),
    #[error("CompletionError: {0}")]
    OneShot(#[from] rig::completion::PromptError),
    #[error("CompletionError: {0}")]
    Completion(#[from] CompletionError),
    #[error("PromptError: {0}")]
    Prompt(#[from] Box<rig::completion::PromptError>),
    #[error("ToolSetError: {0}")]
    Tool(#[from] rig::tool::ToolSetError),
}

// Following the example multi_turn_streaming_gemini, but I'm pretty lost.
async fn multi_turn_completion(
    run_ctx: &RunContext,
    agent: &rig::agent::Agent<rig::client::completion::CompletionModelHandle<'static>>,
    prompt: Message,
    chat_history: &mut Vec<Message>,
) -> Result<(), StreamingError> {
    use futures_util::stream::StreamExt as _;
    use rig::{
        agent::Text,
        streaming::{StreamedAssistantContent, StreamingCompletion},
    };

    if !run_ctx.streaming {
        PromptRequest::new(agent, prompt)
            .multi_turn(5)
            .with_history(chat_history)
            .await?;
        return Ok(());
    }

    // Using two buffers since chat_history is specific to this call, while scratch is
    // more for monitoring global progress across nodes.
    chat_history.push(prompt.clone());
    if let Some(scratch) = &run_ctx.scratch {
        scratch.push_back(Ok(prompt.clone()));
    }

    for _ in 0..5 {
        let current_prompt = match chat_history.pop() {
            Some(prompt) => prompt,
            None => unreachable!("Chat history should never be empty at this point"),
        };
        if let Some(scratch) = &run_ctx.scratch {
            scratch.pop_back();
        }

        let mut request = agent
            .stream_completion(current_prompt.clone(), chat_history.clone())
            .await?;

        if let Some(seed) = &run_ctx.seed {
            let value = seed.value.fetch_add(seed.increment, Ordering::Relaxed);
            request = request.additional_params(json!({"seed": value}));
        }

        let mut stream = request.stream().await?;

        chat_history.push(current_prompt.clone());
        if let Some(scratch) = &run_ctx.scratch {
            scratch.push_back(Ok(current_prompt.clone()));
        }

        let agent_msg = run_ctx
            .scratch
            .as_ref()
            .map(|s| s.push_back(Ok(Message::assistant(""))));

        let mut reasonings = Vec::new();
        let mut texts = String::new();
        let mut tool_calls = vec![];

        while let Some(content) = stream.next().await {
            if run_ctx.interrupt.load(Ordering::Relaxed) {
                Err(WorkflowError::Interrupted)?;
            }
            match content {
                Ok(StreamedAssistantContent::Text(text)) => {
                    texts.push_str(&text.text);
                    let msg = Message::assistant(&texts);
                    if let Some(a) = &agent_msg {
                        a.store(Arc::new(Ok(msg)));
                    }
                }
                Ok(StreamedAssistantContent::ToolCall(tool_call)) => {
                    tool_calls.push(tool_call);
                }
                Ok(StreamedAssistantContent::Reasoning(rig::message::Reasoning {
                    reasoning,
                    ..
                })) => {
                    reasonings.extend(reasoning);
                }
                Ok(_) => {}
                err => {
                    err?;
                }
            }
        }

        let mut contents = Vec::new();

        if !reasonings.is_empty() {
            contents.push(AssistantContent::Reasoning(Reasoning::multi(reasonings)))
        }

        if !texts.is_empty() {
            contents.push(AssistantContent::Text(Text::from(&texts)));
        }

        let tool_call_contents = tool_calls
            .iter()
            .cloned()
            .map(AssistantContent::ToolCall)
            .collect_vec();

        let done = tool_call_contents.is_empty();
        contents.extend(tool_call_contents);

        if !contents.is_empty() {
            let msg = Message::Assistant {
                id: None,
                content: OneOrMany::many(contents).unwrap(),
            };
            chat_history.push(msg.clone());

            if let Some(a) = agent_msg {
                a.store(Arc::new(Ok(msg)));
            }
        }

        let mut tool_results = vec![];
        for tool_call in &tool_calls {
            // TODO: implement tool namespacing by generating a new toolset
            let tool_result = agent
                .tool_server_handle
                .call_tool(
                    &tool_call.function.name,
                    &tool_call.function.arguments.to_string(),
                )
                .await
                .map_err(|x| {
                    StreamingError::Tool(rig::tool::ToolSetError::ToolCallError(
                        rig::tool::ToolError::ToolCallError(x.into()),
                    ))
                })?;
            tool_results.push((tool_call.id.clone(), tool_call.call_id.clone(), tool_result));
        }

        // Add tool results to chat history
        for (id, call_id, tool_result) in tool_results {
            let msg = if let Some(call_id) = call_id {
                Message::User {
                    content: OneOrMany::one(UserContent::tool_result_with_call_id(
                        id,
                        call_id,
                        OneOrMany::one(rig::message::ToolResultContent::text(tool_result)),
                    )),
                }
            } else {
                Message::User {
                    content: OneOrMany::one(UserContent::tool_result(
                        id,
                        OneOrMany::one(rig::message::ToolResultContent::text(tool_result)),
                    )),
                }
            };

            chat_history.push(msg.clone());
            if let Some(scratch) = &run_ctx.scratch {
                scratch.push_back(Ok(msg));
            }
        }

        if done {
            return Ok(());
        }
    }

    // TODO: out of turns
    Ok(())
}
