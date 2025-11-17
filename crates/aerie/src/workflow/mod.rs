use std::sync::Arc;

use egui_snarl::ui::{PinInfo, WireStyle};
use kinded::Kinded;
use serde::{Deserialize, Serialize};

use crate::{ChatHistory, Toolbox, Toolset};

pub mod nodes;
pub mod runner;
pub mod store;

pub use nodes::WorkNode;

// type DynFuture<T> = Pin<Box<dyn Future<Output = T>>>;

#[derive(Kinded, Debug, Clone, Hash, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Value {
    #[default]
    Placeholder,
    Text(String),
    Toolset(Toolset),
    Chat(Arc<ChatHistory>),
}

pub struct EditContext {
    pub toolbox: Toolbox,
}

#[derive(Debug, Default)]
pub struct RunContext {}

pub trait DynNode {
    // Moved to impl of each struct to avoid dealing with boxing
    // /// Update computed values with inputs from remotes.
    // fn forward(&mut self, inputs: Vec<Option<Value>>) -> DynFuture<Result<(), Vec<String>>> {
    //     Box::pin(async { Ok(()) })
    // }

    #[expect(unused_variables)]
    fn value(&self, out_pin: usize) -> Value {
        Default::default()
    }

    fn inputs(&self) -> usize {
        1
    }

    fn outputs(&self) -> usize {
        1
    }

    #[expect(unused_variables)]
    // We're more concerned about type validation here than updating UI visuals
    fn in_kinds(&self, in_pin: usize) -> &'static [ValueKind] {
        ValueKind::all()
    }

    #[expect(unused_variables)]
    // We're more concerned about type validation here than updating UI visuals
    fn out_kind(&self, out_pin: usize) -> ValueKind {
        ValueKind::Placeholder
    }

    fn connect(&self, in_pin: usize, kind: ValueKind) -> Result<(), String> {
        if !self.in_kinds(in_pin).contains(&kind) {
            Err("Not allowed!".into())
        } else {
            Ok(())
        }
    }
}

pub trait UiNode: DynNode {
    /// Supply placeholder values to display in UI outside of executions
    fn preview(&self, out_pin: usize) -> Value {
        self.value(out_pin)
    }

    fn title(&self) -> String {
        String::new()
    }

    fn has_body(&self) -> bool {
        false
    }

    #[expect(unused_variables)]
    fn show_body(&mut self, ui: &mut egui::Ui, ctx: &EditContext) {}

    fn default_pin(&self) -> PinInfo {
        PinInfo::circle()
            .with_fill(egui::Color32::GRAY)
            .with_wire_style(WireStyle::Bezier5)
    }

    #[expect(unused_variables)]
    fn show_input(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &EditContext,
        pin_id: usize,
        remote: Option<Value>,
    ) -> PinInfo {
        self.default_pin()
    }

    #[expect(unused_variables)]
    fn show_output(&mut self, ui: &mut egui::Ui, ctx: &EditContext, pin_id: usize) -> PinInfo {
        self.default_pin()
    }
}
