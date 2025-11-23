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

    fn has_body(&self) -> bool {
        true
    }

    fn show_body(&mut self, ui: &mut egui::Ui, _ctx: &EditContext) {
        egui::Frame::new().inner_margin(4).show(ui, |ui| {
            egui::Resize::default()
                .min_width(MIN_WIDTH)
                .min_height(MIN_WIDTH)
                .show(ui, |ui| {
                    let widget = egui::TextEdit::multiline(&mut self.value)
                        .desired_width(f32::INFINITY)
                        .hint_text("Enter text \u{1F64B}");

                    ui.add_sized(ui.available_size(), widget);
                });
        });
    }

    fn show_output(
        &mut self,
        _ui: &mut egui::Ui,
        _ctx: &EditContext,
        pin_id: usize,
    ) -> egui_snarl::ui::PinInfo {
        assert_eq!(pin_id, 0);

        self.out_kind(pin_id).default_pin()
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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Preview {
    #[serde(skip)]
    pub value: Value,
}

impl std::hash::Hash for Preview {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        "preview".hash(state);
    }
}

impl PartialEq for Preview {
    fn eq(&self, _other: &Self) -> bool {
        true // Preview is entirely transient, so all copies are equal
    }
}

impl Eq for Preview {}

impl DynNode for Preview {
    fn reset(&mut self, _in_pin: usize) {
        self.value = Default::default();
    }

    fn outputs(&self) -> usize {
        0
    }

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
        // TODO: special case for chat history: show current branch
        egui::Frame::new().inner_margin(4).show(ui, |ui| {
            egui::Resize::default()
                .min_width(MIN_WIDTH)
                .min_height(MIN_WIDTH)
                .with_stroke(false)
                .show(ui, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        ui.add(egui::Label::new(format!("{:?}", self.value)).wrap());
                    });
                });
        });
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
