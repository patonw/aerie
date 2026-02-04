use eframe::egui;
use egui::WidgetText;

pub mod runner;
pub mod shortcuts;
pub mod state;
pub mod tiles;
pub mod workflow;

use egui_snarl::{InPinId, NodeId, OutPinId};
pub use state::AppState;
use uuid::Uuid;

use crate::{
    utils::PriorityQueue,
    workflow::{AnyPin, GraphId},
};

pub enum Pane {
    Settings,
    Navigator,
    Chat,
    Logs,
    Tools,
    Workflow,
    Messages,
    Outputs,
}

#[derive(Debug, Clone)]
pub enum ShowHelp {
    All,
    Workflow,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum AppEvent {
    EnterSubgraph(NodeId),
    LeaveSubgraph(usize),

    DisableNode(GraphId, NodeId),

    /// Removes a pin from a node of a graph. Graph must be in the current ViewStack.
    PinRemoved(GraphId, AnyPin),

    /// Swaps the wires of two pins in a graph. Graph must be in the current ViewStack.
    /// Pins must both be inputs or outputs.
    SwapInputs(GraphId, InPinId, InPinId),
    SwapOutputs(GraphId, OutPinId, OutPinId),

    // User requested to run the current workflow
    UserRunWorkflow,

    SetPrompt(String),

    Freeze(Option<bool>),
    Undo,
    Redo,

    ProgressBegin(Uuid, usize),
    ProgressAdd(Uuid, usize),
    ProgressEnd(Uuid),
}

impl AppEvent {
    pub fn priority(&self) -> i64 {
        use AppEvent::*;
        match self {
            EnterSubgraph(_) | LeaveSubgraph(_) => -100,
            UserRunWorkflow | SetPrompt(_) => -200,
            _ => 0,
        }
    }
}

impl PartialOrd for AppEvent {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for AppEvent {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.priority().cmp(&other.priority())
    }
}

pub type AppEvents = PriorityQueue<AppEvent>;

fn user_bubble<R>(ui: &mut egui::Ui, cb_r: impl FnMut(&mut egui::Ui) -> R) -> R {
    egui::Sides::new()
        .show(
            ui,
            |_| {},
            |ui| {
                egui::Frame::new()
                    .stroke(egui::Stroke::new(1.0, egui::Color32::GRAY))
                    .corner_radius(16)
                    .outer_margin(4)
                    .inner_margin(8)
                    .show(ui, cb_r)
                    .inner
            },
        )
        .1
}

fn agent_bubble<R>(
    ui: &mut egui::Ui,
    cb: impl FnMut(&mut egui::Ui) -> R,
) -> egui::InnerResponse<R> {
    ui.with_layout(egui::Layout::left_to_right(egui::Align::TOP), |ui| {
        egui::Frame::new()
            .stroke(egui::Stroke::new(1.0, egui::Color32::GRAY))
            .corner_radius(16)
            .outer_margin(4)
            .inner_margin(8)
            .show(ui, cb)
            .inner
    })
}

fn error_bubble<R>(
    ui: &mut egui::Ui,
    cb: impl FnMut(&mut egui::Ui) -> R,
) -> egui::InnerResponse<R> {
    ui.with_layout(egui::Layout::left_to_right(egui::Align::TOP), |ui| {
        egui::Frame::new()
            .inner_margin(12)
            .outer_margin(24)
            .corner_radius(14)
            .shadow(egui::Shadow {
                offset: [8, 12],
                blur: 16,
                spread: 0,
                color: egui::Color32::from_black_alpha(180),
            })
            // .fill(egui::Color32::from_rgba_unmultiplied(97, 0, 255, 128))
            .stroke(egui::Stroke::new(1.0, egui::Color32::RED))
            .show(ui, cb)
            .inner
    })
}

pub fn toggled_field<'a, T: Default>(
    ui: &mut egui::Ui,
    label: impl egui::IntoAtoms<'a>,
    tooltip: Option<impl Into<WidgetText>>,
    value: &mut Option<T>,
    mut cb: impl FnMut(&mut egui::Ui, &mut T),
) {
    ui.horizontal_centered(|ui| {
        let widget = ui.selectable_label(value.is_some(), label);
        let widget = if let Some(text) = tooltip {
            widget.on_hover_text(text)
        } else {
            widget
        };

        if widget.clicked() {
            *value = match value {
                Some(_) => None,
                None => Some(Default::default()),
            };
        }

        if let Some(current) = value {
            cb(ui, current);
        } else {
            ui.weak("Toggle label to edit");
        }
    });
}

pub fn resizable_frame(
    size: &mut Option<crate::utils::EVec2>,
    ui: &mut egui::Ui,
    add_contents: impl FnOnce(&mut egui::Ui),
) {
    resizable_frame_opt(None, size, ui, add_contents);
}

pub fn resizable_frame_opt(
    default_size: Option<egui::Vec2>,
    size: &mut Option<crate::utils::EVec2>,
    ui: &mut egui::Ui,
    add_contents: impl FnOnce(&mut egui::Ui),
) {
    let default_size = default_size.unwrap_or(egui::vec2(300.0, 150.0));
    egui::Resize::default()
        .default_size(size.map(egui::Vec2::from).unwrap_or(default_size))
        .with_stroke(false)
        .show(ui, |ui| {
            *size = Some(ui.available_size().into());
            add_contents(ui);
        });
}
