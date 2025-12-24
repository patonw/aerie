use std::{convert::identity, sync::Arc};

use decorum::E64;
use egui_snarl::OutPinId;
use itertools::Itertools;
use serde::{Deserialize, Serialize};

use crate::{ChatHistory, workflow::WorkflowError};

use super::{DynNode, EditContext, RunContext, UiNode, Value, ValueKind};

// These fields will always be set from the run context each execution.
// Saving them to disk is just a waste.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Start {
    /// A snapshot of the history at the start of run
    #[serde(skip)]
    pub history: Arc<ChatHistory>,

    #[serde(skip)]
    pub user_prompt: String,

    #[serde(skip)]
    pub model: String,

    #[serde(skip)]
    pub temperature: E64,
}

impl std::hash::Hash for Start {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        "Start".hash(state);
    }
}

impl PartialEq for Start {
    fn eq(&self, _other: &Self) -> bool {
        true // Start is entirely transient, so all copies are equal
    }
}

impl Eq for Start {}

impl DynNode for Start {
    fn inputs(&self) -> usize {
        0
    }

    fn outputs(&self) -> usize {
        4
    }

    fn out_kind(&self, out_pin: usize) -> ValueKind {
        match out_pin {
            0 => ValueKind::Model,
            1 => ValueKind::Number,
            2 => ValueKind::Chat,
            3 => ValueKind::Text,
            _ => unreachable!(),
        }
    }

    fn value(&self, out_pin: usize) -> Value {
        match out_pin {
            0 => Value::Model(self.model.clone()),
            1 => Value::Number(self.temperature),
            2 => Value::Chat(self.history.clone()),
            3 => Value::Text(self.user_prompt.clone()),
            _ => unreachable!(),
        }
    }

    fn execute(
        &mut self,
        ctx: &RunContext,
        _node_id: egui_snarl::NodeId,
        _inputs: Vec<Option<Value>>,
    ) -> Result<Vec<Value>, WorkflowError> {
        self.history = ctx.history.load().clone();
        self.user_prompt = ctx.user_prompt.clone();
        self.model = ctx.model.clone();
        self.temperature = E64::assert(ctx.temperature);

        Ok(self.collect_outputs())
    }
}

impl UiNode for Start {
    fn title(&self) -> &str {
        "Start"
    }

    fn show_output(
        &mut self,
        ui: &mut egui::Ui,
        _ctx: &EditContext,
        pin_id: usize,
    ) -> egui_snarl::ui::PinInfo {
        match pin_id {
            0 => {
                ui.label("model");
            }
            1 => {
                ui.label("temperature");
            }
            2 => {
                ui.label("conversation");
            }
            3 => {
                ui.label("prompt");
            }
            _ => unreachable!(),
        };

        self.out_kind(pin_id).default_pin()
    }
}

#[derive(Debug, Clone, Default, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finish {}

impl DynNode for Finish {
    fn priority(&self) -> usize {
        2000
    }

    fn outputs(&self) -> usize {
        0
    }

    fn in_kinds(&self, in_pin: usize) -> &'static [ValueKind] {
        match in_pin {
            0 => &[ValueKind::Chat],
            _ => unreachable!(),
        }
    }

    fn execute(
        &mut self,
        ctx: &RunContext,
        _node_id: egui_snarl::NodeId,
        inputs: Vec<Option<Value>>,
    ) -> Result<Vec<Value>, WorkflowError> {
        self.validate(&inputs)?;

        match &inputs[0] {
            Some(Value::Chat(chat)) => {
                if ctx.history.load().is_subset(chat) {
                    ctx.history
                        .store(Arc::new(chat.with_base(None).into_owned()));
                } else {
                    Err(WorkflowError::Conversion(
                        "Final chat history is not related to the session. Refusing to overwrite."
                            .into(),
                    ))?;
                }
            }
            None => {}
            _ => unreachable!(),
        }

        Ok(vec![])
    }
}

impl UiNode for Finish {
    fn title(&self) -> &str {
        "Finish"
    }

    fn tooltip(&self) -> &str {
        "Finish the run by injecting the input conversation into the session"
    }

    fn show_input(
        &mut self,
        ui: &mut egui::Ui,
        _ctx: &EditContext,
        pin_id: usize,
        _remote: Option<Value>, // TODO: rename to "wired" this should be ValueKind!
    ) -> egui_snarl::ui::PinInfo {
        match pin_id {
            0 => ui.label("conversation"),
            _ => unreachable!(),
        };

        self.in_kinds(pin_id).first().unwrap().default_pin()
    }
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, Deserialize, Serialize)]
pub struct Fallback {
    pub kinds: Vec<ValueKind>,
}

impl Default for Fallback {
    fn default() -> Self {
        Self {
            kinds: vec![ValueKind::Placeholder],
        }
    }
}

impl DynNode for Fallback {
    fn inputs(&self) -> usize {
        self.kinds.len() + 1 // Slot for history plus empty to add new msg
    }

    fn in_kinds(&self, in_pin: usize) -> &[ValueKind] {
        let is_var_pin = in_pin > 0 && in_pin < self.kinds.len() + 1;
        match in_pin {
            0 => &[ValueKind::Failure],
            _ if is_var_pin && self.kinds[in_pin - 1] != ValueKind::Placeholder => {
                std::slice::from_ref(&self.kinds[in_pin - 1])
            }
            _ => ValueKind::all(),
        }
    }

    fn outputs(&self) -> usize {
        self.kinds.len()
    }

    fn out_kind(&self, out_pin: usize) -> ValueKind {
        if out_pin < self.kinds.len() {
            self.kinds[out_pin]
        } else {
            ValueKind::Placeholder
        }
    }

    fn execute(
        &mut self,
        _ctx: &RunContext,
        _node_id: egui_snarl::NodeId,
        inputs: Vec<Option<Value>>,
    ) -> Result<Vec<Value>, WorkflowError> {
        self.validate(&inputs)?;

        Ok(inputs
            .into_iter()
            .skip(1)
            .map(|it| it.unwrap_or_default())
            .collect_vec())
    }
}

impl UiNode for Fallback {
    fn title(&self) -> &str {
        "Fallback"
    }

    fn preview(&self, out_pin: usize) -> Value {
        Value::Placeholder(self.out_kind(out_pin))
    }

    fn show_input(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &EditContext,
        pin_id: usize,
        remote: Option<Value>,
    ) -> egui_snarl::ui::PinInfo {
        let kind = match &remote {
            Some(Value::Placeholder(kind)) => Some(*kind),
            Some(value) => Some(value.kind()),
            _ => None,
        };

        // // Dynamic sizing makes this needlessly complex
        // // Extend inputs to allow additional collection
        // if pin_id == self.kinds.len() + 1 && remote.is_some() {
        //     self.kinds.push(ValueKind::Placeholder);
        // }
        // // // GC unused pins... leads to strange behavior with stale output wires
        // // // Better to avoid for now.
        // else if pin_id != 0 && pin_id == self.kinds.len() && remote.is_none() {
        //     tracing::debug!("Resetting garbage collected pin {:?}", pin_id);
        //     ctx.drop_out_pin(OutPinId {
        //         node: ctx.current_node,
        //         output: pin_id - 1,
        //     });
        //     self.kinds.pop();
        // }

        if pin_id == 0 {
            ui.label("failure");
        } else if pin_id < self.kinds.len() + 1 {
            // when kind changes
            if kind.is_some() ^ (self.kinds[pin_id - 1] != ValueKind::Placeholder) {
                if let Some(kind) = kind {
                    self.kinds[pin_id - 1] = kind;
                } else {
                    self.kinds[pin_id - 1] = ValueKind::Placeholder;
                }

                tracing::debug!("Resetting kind changed pin {:?}", pin_id);
                ctx.reset_out_pin(OutPinId {
                    node: ctx.current_node,
                    output: pin_id - 1,
                });
            }

            ui.label(format!("{pin_id}"));
        }

        self.in_kinds(pin_id).first().unwrap().default_pin()
    }

    fn show_output(
        &mut self,
        ui: &mut egui::Ui,
        _ctx: &EditContext,
        pin_id: usize,
    ) -> egui_snarl::ui::PinInfo {
        ui.label(format!("{}", pin_id + 1));

        self.out_kind(pin_id).default_pin()
    }
}

// a la I/O select
#[derive(Debug, Clone, Default, Hash, PartialEq, Eq, Deserialize, Serialize)]
pub struct Select {
    count: usize,

    kind: ValueKind,
}

impl DynNode for Select {
    fn inputs(&self) -> usize {
        self.count + 1 // Extra slot to add another document
    }

    // Allows anything for the first value, but all other inputs
    // must be of the same kind.
    fn in_kinds(&self, _in_pin: usize) -> &[ValueKind] {
        if self.count == 0 {
            ValueKind::all()
        } else {
            std::slice::from_ref(&self.kind)
        }
    }

    fn out_kind(&self, _out_pin: usize) -> ValueKind {
        self.kind
    }

    fn value(&self, _out_pin: usize) -> Value {
        Value::Placeholder(self.kind)
    }

    fn priority(&self) -> usize {
        8000
    }

    fn execute(
        &mut self,
        _ctx: &RunContext,
        _node_id: egui_snarl::NodeId,
        inputs: Vec<Option<Value>>,
    ) -> Result<Vec<Value>, WorkflowError> {
        self.validate(&inputs)?;

        let output = inputs
            .into_iter()
            .find_map(identity)
            .ok_or(WorkflowError::Unknown(
                "Select called with empty inputs".into(),
            ))?;

        Ok(vec![output])
    }
}

impl UiNode for Select {
    fn title(&self) -> &str {
        "Select"
    }

    fn tooltip(&self) -> &str {
        "Emits the first input value that becomes ready.\n\
            Used for joining fallback branches to main control flow."
    }

    fn show_input(
        &mut self,
        _ui: &mut egui::Ui,
        ctx: &EditContext,
        pin_id: usize,
        remote: Option<Value>,
    ) -> egui_snarl::ui::PinInfo {
        let kind = match &remote {
            Some(Value::Placeholder(kind)) => Some(*kind),
            Some(value) => Some(value.kind()),
            _ => None,
        };

        if self.count == 0 {
            if self.kind == ValueKind::Placeholder
                && let Some(kind) = kind
            {
                self.kind = kind;

                ctx.reset_out_pin(OutPinId {
                    node: ctx.current_node,
                    output: 0,
                });
            } else if kind.is_none() {
                self.kind = ValueKind::Placeholder;
            }
        }

        if pin_id == self.count && remote.is_some() {
            self.count += 1;
        } else if pin_id + 1 == self.count && remote.is_none() {
            self.count -= 1;
        }

        self.in_kinds(pin_id).first().unwrap().default_pin()
    }

    fn show_output(
        &mut self,
        ui: &mut egui::Ui,
        _ctx: &EditContext,
        pin_id: usize,
    ) -> egui_snarl::ui::PinInfo {
        if self.count > 0 {
            ui.label(format!("{}", self.kind).to_lowercase());
        }

        self.out_kind(pin_id).default_pin()
    }
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, Deserialize, Serialize)]
pub struct Demote {
    priority: usize,

    kind: ValueKind,
}

impl Default for Demote {
    fn default() -> Self {
        Self {
            priority: 5000,
            kind: Default::default(),
        }
    }
}

impl DynNode for Demote {
    fn priority(&self) -> usize {
        self.priority
    }

    fn in_kinds(&self, _in_pin: usize) -> &[ValueKind] {
        if matches!(self.kind, ValueKind::Placeholder) {
            ValueKind::all()
        } else {
            std::slice::from_ref(&self.kind)
        }
    }

    fn out_kind(&self, _out_pin: usize) -> ValueKind {
        self.kind
    }

    fn execute(
        &mut self,
        _ctx: &RunContext,
        _node_id: egui_snarl::NodeId,
        inputs: Vec<Option<Value>>,
    ) -> Result<Vec<Value>, WorkflowError> {
        self.validate(&inputs)?;

        let output = inputs
            .into_iter()
            .find_map(identity)
            .ok_or(WorkflowError::Unknown(
                "Demote called with empty inputs".into(),
            ))?;

        Ok(vec![output])
    }
}

impl UiNode for Demote {
    fn title(&self) -> &str {
        "Demote"
    }

    fn tooltip(&self) -> &str {
        "Blocks a path in the graph until there are no more\n\
            nodes with higher priority that are ready to run."
    }

    fn show_input(
        &mut self,
        _ui: &mut egui::Ui,
        ctx: &EditContext,
        pin_id: usize,
        remote: Option<Value>,
    ) -> egui_snarl::ui::PinInfo {
        let kind = match remote {
            Some(Value::Placeholder(kind)) => Some(kind),
            Some(value) => Some(value.kind()),
            _ => None,
        };

        if self.kind == ValueKind::Placeholder
            && let Some(kind) = kind
        {
            self.kind = kind;

            ctx.reset_out_pin(OutPinId {
                node: ctx.current_node,
                output: pin_id,
            });
        } else if kind.is_none() {
            self.kind = ValueKind::Placeholder;
        }

        self.in_kinds(pin_id).first().unwrap().default_pin()
    }

    fn has_body(&self) -> bool {
        true
    }

    fn show_body(&mut self, ui: &mut egui::Ui, _ctx: &EditContext) {
        ui.add(egui::Slider::new(&mut self.priority, 0..=10_000).text("P"))
            .on_hover_text("priority");
    }
}
