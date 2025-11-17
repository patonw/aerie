use serde::{Deserialize, Serialize};

use super::{DynNode, EditContext, MIN_WIDTH, RunContext, UiNode, Value, ValueKind};

#[derive(Debug, Clone, Default, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub struct Text {
    pub value: String,
}

impl DynNode for Text {
    fn value(&self, _out_pin: usize) -> Value {
        Value::Text(self.value.clone())
    }

    fn inputs(&self) -> usize {
        0
    }

    fn out_kind(&self, out_pin: usize) -> ValueKind {
        assert_eq!(out_pin, 0);
        ValueKind::Text
    }
}
impl UiNode for Text {
    fn title(&self) -> String {
        "Text".into()
    }

    fn show_output(
        &mut self,
        ui: &mut egui::Ui,
        _ctx: &EditContext,
        pin_id: usize,
    ) -> egui_snarl::ui::PinInfo {
        assert_eq!(pin_id, 0);
        ui.set_min_width(MIN_WIDTH);

        ui.text_edit_multiline(&mut self.value);

        self.default_pin()
    }
}

impl Text {
    pub async fn forward(
        &mut self,
        _ctx: &RunContext,
        _inputs: Vec<Option<Value>>,
    ) -> Result<(), Vec<String>> {
        Ok(())
    }
}
#[derive(Debug, Clone, Hash, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Preview {
    pub value: Value,
}

impl DynNode for Preview {
    fn value(&self, _out_pin: usize) -> Value {
        self.value.clone()
    }
}

impl UiNode for Preview {
    fn title(&self) -> String {
        "Preview".into()
    }

    fn has_body(&self) -> bool {
        true
    }

    fn show_body(&mut self, ui: &mut egui::Ui, _ctx: &EditContext) {
        ui.set_min_width(MIN_WIDTH);
        ui.label(format!("{:?}", self.value));
    }
}

impl Preview {
    pub async fn forward(
        &mut self,
        _ctx: &RunContext,
        inputs: Vec<Option<Value>>,
    ) -> Result<(), Vec<String>> {
        if let Some(value) = inputs.first().and_then(|it| it.as_ref()) {
            self.value = value.to_owned();
        }
        Ok(())
    }
}
