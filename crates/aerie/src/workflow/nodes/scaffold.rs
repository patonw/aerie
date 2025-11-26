use std::sync::Arc;

use decorum::E64;
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
            0 => ValueKind::Chat,
            1 => ValueKind::Model,
            2 => ValueKind::Number,
            3 => ValueKind::Text,
            _ => unreachable!(),
        }
    }

    fn value(&self, out_pin: usize) -> Value {
        match out_pin {
            0 => Value::Chat(self.history.clone()),
            1 => Value::Model(self.model.clone()),
            2 => Value::Number(self.temperature),
            3 => Value::Text(self.user_prompt.clone()),
            _ => unreachable!(),
        }
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
                ui.label("conversation");
            }
            1 => {
                ui.label("model");
            }
            2 => {
                ui.label("temperature");
            }
            3 => {
                ui.label("prompt");
            }
            _ => unreachable!(),
        };

        self.out_kind(pin_id).default_pin()
    }
}

impl Start {
    pub async fn forward(
        &mut self,
        ctx: &RunContext,
        _inputs: Vec<Option<Value>>,
    ) -> Result<(), WorkflowError> {
        self.history = ctx.history.load().clone();
        self.user_prompt = ctx.user_prompt.clone();
        self.model = ctx.model.clone();
        self.temperature = E64::assert(ctx.temperature);

        Ok(())
    }
}

#[derive(Debug, Clone, Default, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finish {}

impl DynNode for Finish {
    fn outputs(&self) -> usize {
        0
    }

    fn in_kinds(&self, in_pin: usize) -> &'static [ValueKind] {
        match in_pin {
            0 => &[ValueKind::Chat],
            _ => unreachable!(),
        }
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

impl Finish {
    pub async fn forward(
        &mut self,
        ctx: &RunContext,
        inputs: Vec<Option<Value>>,
    ) -> Result<(), WorkflowError> {
        self.validate(&inputs)?;
        match &inputs[0] {
            Some(Value::Chat(chat)) => {
                ctx.history.store(chat.clone());
            }
            _ => unreachable!(),
        }

        Ok(())
    }
}
