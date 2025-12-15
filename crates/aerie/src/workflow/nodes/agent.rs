use decorum::E64;
use egui::TextEdit;
use rig::message::Message;
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;
use std::sync::Arc;

use super::{DynNode, EditContext, RunContext, UiNode, Value, ValueKind};
use crate::{
    ChatContent, ChatHistory, ToolProvider, ToolSelector, agent::AgentSpec, ui::resizable_frame,
    workflow::WorkflowError,
};

#[derive(Debug, Clone, Default, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tools {
    toolset: Arc<ToolSelector>,
}

impl DynNode for Tools {
    fn inputs(&self) -> usize {
        0
    }

    fn out_kind(&self, out_pin: usize) -> ValueKind {
        assert_eq!(out_pin, 0);
        ValueKind::Tools
    }
    fn value(&self, out_pin: usize) -> Value {
        assert_eq!(out_pin, 0);
        Value::Tools(self.toolset.clone())
    }
}

impl UiNode for Tools {
    fn title(&self) -> &str {
        "Tools"
    }

    fn has_body(&self) -> bool {
        true
    }

    fn show_body(&mut self, ui: &mut egui::Ui, ctx: &EditContext) {
        ui.vertical_centered_justified(|ui| {
            ui.horizontal_wrapped(|ui| {
                if ui.selectable_label(self.toolset.is_all(), "all").clicked() {
                    self.toolset = Arc::new(ToolSelector::all());
                }

                if ui
                    .selectable_label(self.toolset.is_empty(), "none")
                    .clicked()
                {
                    self.toolset = Arc::new(ToolSelector::empty());
                }
            });

            ui.separator();

            for (name, provider) in &ctx.toolbox.providers {
                ui.collapsing(name, |ui| {
                    let ToolProvider::MCP { tools, .. } = provider;
                    for tool in tools {
                        let mut active = self.toolset.apply(name, tool);

                        if ui.checkbox(&mut active, tool.name.as_ref()).clicked() {
                            // Cow-like cloning if other refs exist
                            Arc::make_mut(&mut self.toolset).toggle(name, tool);
                        }
                    }
                });
            }
        });
    }
}

impl Tools {
    pub async fn forward(
        &mut self,
        _ctx: &RunContext,
        _inputs: Vec<Option<Value>>,
    ) -> Result<(), WorkflowError> {
        Ok(())
    }
}

#[skip_serializing_none]
#[derive(Default, Debug, Clone, Deserialize, Serialize, Hash, PartialEq, Eq)]
pub struct AgentNode {
    pub model: Option<String>,

    pub preamble: Option<String>,

    pub temperature: Option<E64>,

    pub size: Option<crate::utils::EVec2>,

    #[serde(skip)]
    pub tools: Option<Arc<ToolSelector>>,

    #[serde(skip)]
    pub agent_spec: Arc<AgentSpec>,
}

impl DynNode for AgentNode {
    fn inputs(&self) -> usize {
        5
    }

    fn outputs(&self) -> usize {
        1
    }

    fn in_kinds(&self, in_pin: usize) -> &'static [ValueKind] {
        match in_pin {
            0 => &[ValueKind::Agent],
            1 => &[ValueKind::Model],
            2 => &[ValueKind::Number],
            3 => &[ValueKind::Tools],
            4 => &[ValueKind::Text],
            _ => ValueKind::all(),
        }
    }

    fn out_kind(&self, out_pin: usize) -> ValueKind {
        match out_pin {
            0 => ValueKind::Agent,
            _ => unreachable!(),
        }
    }

    fn value(&self, _out_pin: usize) -> Value {
        Value::Agent(self.agent_spec.clone())
    }
}

impl UiNode for AgentNode {
    fn title(&self) -> &str {
        "Agent"
    }

    fn tooltip(&self) -> &str {
        "Create or modify an LLM Agent."
    }

    fn preview(&self, _out_pin: usize) -> Value {
        Value::Placeholder(ValueKind::Agent)
    }

    fn show_output(
        &mut self,
        ui: &mut egui::Ui,
        _ctx: &EditContext,
        pin_id: usize,
    ) -> egui_snarl::ui::PinInfo {
        match pin_id {
            0 => {
                ui.label("agent");
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
            // TODO: Toggle enabling each field to override existing. If no agent input connected,
            // do not allow toggling
            0 => {
                ui.label("Agent")
                    .on_hover_text("An existing agent to modify.");
            }
            1 => {
                if remote.is_none() {
                    crate::ui::toggled_field(
                        ui,
                        "M",
                        Some("model"),
                        &mut self.model,
                        |ui, value| {
                            ui.add(TextEdit::singleline(value).hint_text("provider/model:tag"));
                        },
                    );
                } else {
                    ui.label("model");
                }
            }
            2 => {
                if remote.is_none() {
                    crate::ui::toggled_field(
                        ui,
                        "T",
                        Some("temperature"),
                        &mut self.temperature,
                        |ui, value| {
                            let mut temp = value.into_inner();

                            let widget = egui::Slider::new(&mut temp, 0.0..=1.0);
                            ui.add(widget);
                            *value = E64::assert(temp);
                        },
                    );
                } else {
                    ui.label("temperature");
                }
            }
            3 => {
                ui.label("Tools");
            }
            4 => {
                if remote.is_none() {
                    let help = "system message\n\
                        \n\
                        Instructions to the agent outside the flow of conversation.\n\
                        Can include hints about its role, personality or formatting requirements.";
                    crate::ui::toggled_field(
                        ui,
                        "S",
                        Some(help),
                        &mut self.preamble,
                        |ui, value| {
                            resizable_frame(&mut self.size, ui, |ui| {
                                let widget = egui::TextEdit::multiline(value)
                                    .id_salt("sysmesg")
                                    .desired_width(f32::INFINITY)
                                    .hint_text("system message");

                                ui.add_sized(ui.available_size(), widget)
                                    .on_hover_text(help);
                            });
                        },
                    );
                } else {
                    ui.label("preamble");
                }
            }
            _ => unreachable!(),
        };

        self.in_kinds(pin_id).first().unwrap().default_pin()
    }
}

impl AgentNode {
    pub async fn forward(
        &mut self,
        _run_ctx: &RunContext,
        inputs: Vec<Option<Value>>,
    ) -> Result<(), WorkflowError> {
        self.validate(&inputs)?;

        let agent = match &inputs[0] {
            Some(Value::Agent(spec)) => Some(spec.clone()),
            None => None,
            _ => unreachable!(),
        };

        let model = match &inputs[1] {
            Some(Value::Model(name)) => Some(name.clone()),
            None => self.model.clone(),
            _ => unreachable!(),
        };

        if agent.is_none() && model.is_none() {
            return Err(WorkflowError::Required(vec![
                "Either model name or an existing agent is required.".into(),
            ]));
        }

        let temperature = match &inputs[2] {
            Some(Value::Number(temp)) => Some(*temp),
            None => self.temperature,
            _ => unreachable!(),
        };

        let toolset = match &inputs[3] {
            Some(Value::Tools(tools)) => Some(tools.clone()),
            None => None,
            _ => unreachable!(),
        };

        let preamble = match &inputs[4] {
            Some(Value::Text(text)) => Some(text.clone()),
            None => self.preamble.clone(),
            _ => unreachable!(),
        };

        let mut agent = agent.unwrap_or_default();
        let builder = Arc::make_mut(&mut agent);

        if let Some(model) = model {
            builder.model(model);
        }

        if let Some(temp) = temperature {
            tracing::debug!("Setting agent temperature {temp}");
            builder.temperature(temp);
        }

        if let Some(preamble) = &preamble {
            tracing::debug!("Setting agent preamble {preamble}");
            builder.preamble(preamble.clone());
        }

        if let Some(tools) = toolset {
            builder.tools(tools);
        }

        self.agent_spec = agent;

        Ok(())
    }
}

#[skip_serializing_none]
#[derive(Default, Debug, Clone, Deserialize, Serialize, Hash, PartialEq, Eq)]
pub struct ChatContext {
    pub context_doc: String,

    pub size: Option<crate::utils::EVec2>,

    #[serde(skip)]
    pub agent_spec: Arc<AgentSpec>,
}

impl DynNode for ChatContext {
    fn inputs(&self) -> usize {
        2
    }

    fn outputs(&self) -> usize {
        1
    }

    fn in_kinds(&self, in_pin: usize) -> &'static [ValueKind] {
        match in_pin {
            0 => &[ValueKind::Agent],
            1 => &[ValueKind::Text],
            _ => ValueKind::all(),
        }
    }

    fn out_kind(&self, out_pin: usize) -> ValueKind {
        match out_pin {
            0 => ValueKind::Agent,
            _ => unreachable!(),
        }
    }

    fn value(&self, _out_pin: usize) -> Value {
        Value::Agent(self.agent_spec.clone())
    }
}

impl UiNode for ChatContext {
    fn title(&self) -> &str {
        "Context"
    }

    fn tooltip(&self) -> &str {
        "Provide background context in the chat"
    }

    fn preview(&self, _out_pin: usize) -> Value {
        Value::Placeholder(ValueKind::Agent)
    }

    fn show_output(
        &mut self,
        ui: &mut egui::Ui,
        _ctx: &EditContext,
        pin_id: usize,
    ) -> egui_snarl::ui::PinInfo {
        match pin_id {
            0 => {
                ui.label("agent");
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
            // TODO: Toggle enabling each field to override existing. If no agent input connected,
            // do not allow toggling
            0 => {
                ui.label("agent").on_hover_text(
                    "An existing agent to modify.\nNOTE: tools cannot be carried over.",
                );
            }
            1 => {
                return ui
                    .vertical(|ui| {
                        if remote.is_none() {
                            resizable_frame(&mut self.size, ui, |ui| {
                                let widget = egui::TextEdit::multiline(&mut self.context_doc)
                                    .id_salt("context")
                                    .desired_width(f32::INFINITY)
                                    .hint_text("context");

                                ui.add_sized(ui.available_size(), widget)
                                    .on_hover_text("Background context");
                            });
                            self.ghost_pin(ValueKind::Text.color())
                        } else {
                            ui.label("context");
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

impl ChatContext {
    pub async fn forward(
        &mut self,
        _run_ctx: &RunContext,
        inputs: Vec<Option<Value>>,
    ) -> Result<(), WorkflowError> {
        self.validate(&inputs)?;

        let mut agent_spec = match &inputs[0] {
            Some(Value::Agent(spec)) => spec.clone(),
            None => Err(WorkflowError::Required(vec!["Agent is required".into()]))?,
            _ => unreachable!(),
        };

        let context_doc = match &inputs[1] {
            Some(Value::Text(text)) => text.clone(),
            None => self.context_doc.clone(),
            _ => unreachable!(),
        };

        Arc::make_mut(&mut agent_spec).context_doc(context_doc);

        self.agent_spec = agent_spec;

        Ok(())
    }
}

#[skip_serializing_none]
#[derive(Default, Debug, Clone, Deserialize, Serialize, Hash, PartialEq, Eq)]
pub struct InvokeTool {
    pub tool_name: String,

    #[serde(skip)]
    pub history: Arc<ChatHistory>,

    #[serde(skip)]
    pub tool_output: String,
}

impl DynNode for InvokeTool {
    fn inputs(&self) -> usize {
        4
    }

    fn in_kinds(&self, in_pin: usize) -> &'static [ValueKind] {
        match in_pin {
            0 => &[ValueKind::Chat],
            1 => &[ValueKind::Tools],
            2 => &[ValueKind::Text],
            3 => &[ValueKind::Json],
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
            3 => ValueKind::Failure,
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
            2 => Value::Text(self.tool_output.clone()),
            3 => Value::Placeholder(ValueKind::Failure),
            _ => unreachable!(),
        }
    }
}

impl UiNode for InvokeTool {
    fn title(&self) -> &str {
        "Invoke Tool"
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
                ui.label("output");
            }
            3 => {
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
                ui.label("history");
            }
            1 => {
                ui.label("tools");
            }
            2 => {
                if remote.is_none() {
                    ui.add(TextEdit::singleline(&mut self.tool_name));
                } else {
                    ui.label("tool name");
                }
            }
            3 => {
                ui.label("arguments");
            }
            _ => unreachable!(),
        };

        self.in_kinds(pin_id).first().unwrap().default_pin()
    }
}

impl InvokeTool {
    pub async fn forward(
        &mut self,
        run_ctx: &RunContext,
        inputs: Vec<Option<Value>>,
    ) -> Result<(), WorkflowError> {
        self.validate(&inputs)?;

        let chat = match &inputs[0] {
            Some(Value::Chat(history)) => history,
            None => Err(WorkflowError::Required(vec![
                "Chat history required".into(),
            ]))?,
            _ => unreachable!(),
        };

        let toolset = match &inputs[1] {
            Some(Value::Tools(spec)) => spec.clone(),
            None => Err(WorkflowError::Required(vec!["Toolset is required".into()]))?,
            _ => unreachable!(),
        };

        let rig_tools = run_ctx.agent_factory.toolbox.get_tools(&toolset);
        let single_tool = if let [tool] = rig_tools.get_tool_definitions().await.unwrap().as_slice()
        {
            Some(tool.name.clone())
        } else {
            None
        };

        // TODO: infer if only one tool on the agent
        let tool_name = match &inputs[2] {
            Some(Value::Text(text)) => text.as_str(),
            None if !self.tool_name.is_empty() => self.tool_name.as_str(),
            None if single_tool.is_some() => single_tool.as_ref().unwrap(),
            None => Err(WorkflowError::Required(vec!["Tool name required".into()]))?,
            _ => unreachable!(),
        };

        let args = match &inputs[3] {
            Some(Value::Json(value)) => value.clone(),
            None => Err(WorkflowError::Required(vec![
                "Tool arguments are required".into(),
            ]))?,
            _ => unreachable!(),
        };

        let output = rig_tools.call(tool_name, args.to_string()).await?;

        let msg = Message::tool_result(tool_name, &output);
        let chat = chat.extend(vec![Ok(msg).into()])?;
        self.history = Arc::new(chat.into_owned());

        // Should we even attempt to deseruakuze here? Can we get an output schema?
        self.tool_output = output;

        Ok(())
    }
}
