use std::sync::Arc;

use decorum::E64;
use serde::{Deserialize, Serialize};

use crate::ChatHistory;

use super::{DynNode, EditContext, RunContext, UiNode, Value, ValueKind};

// These fields will always be set from the run context each execution.
// Saving them to disk is just a waste.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Start {
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
            1 => ValueKind::Text,
            2 => ValueKind::Model,
            3 => ValueKind::Number,
            _ => unreachable!(),
        }
    }

    fn value(&self, out_pin: usize) -> Value {
        match out_pin {
            0 => Value::Chat(self.history.clone()),
            1 => Value::Text(self.user_prompt.clone()),
            2 => Value::Model(self.model.clone()),
            3 => Value::Number(self.temperature),
            _ => unreachable!(),
        }
    }
}

impl UiNode for Start {
    fn title(&self) -> String {
        "Start".to_string()
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
                ui.label("prompt");
            }
            2 => {
                ui.label("model");
            }
            3 => {
                ui.label("temperature");
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
    ) -> Result<(), Vec<String>> {
        let history = ctx.history.clone();

        self.history = history;
        self.user_prompt = ctx.user_prompt.clone();
        self.model = ctx.model.clone();
        self.temperature = E64::assert(ctx.temperature);

        Ok(())
    }
}
