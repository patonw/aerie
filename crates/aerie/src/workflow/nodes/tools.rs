use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::{ToolProvider, Toolset};

use super::{DynNode, EditContext, RunContext, UiNode, Value, ValueKind};

#[derive(Debug, Clone, Default, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tools {
    toolset: Arc<Toolset>,
}

impl DynNode for Tools {
    fn inputs(&self) -> usize {
        0
    }

    fn out_kind(&self, out_pin: usize) -> ValueKind {
        assert_eq!(out_pin, 0);
        ValueKind::Toolset
    }
    fn value(&self, out_pin: usize) -> Value {
        assert_eq!(out_pin, 0);
        Value::Toolset(self.toolset.clone())
    }
}

impl UiNode for Tools {
    fn title(&self) -> String {
        "Tools".into()
    }

    fn has_body(&self) -> bool {
        true
    }

    fn show_body(&mut self, ui: &mut egui::Ui, ctx: &EditContext) {
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
    }
}

impl Tools {
    pub async fn forward(
        &mut self,
        _ctx: &RunContext,
        _inputs: Vec<Option<Value>>,
    ) -> Result<(), Vec<String>> {
        Ok(())
    }
}
