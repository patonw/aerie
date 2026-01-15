use egui::{Event, Key, KeyboardShortcut, Modifiers, Sense};
use egui_snarl::{NodeId, Snarl, ui::SnarlWidget};
use enum_assoc::Assoc;
use enum_iterator::Sequence;
use itertools::Itertools as _;
use typed_builder::TypedBuilder;

use crate::{
    ui::{AppEvent, workflow::WorkflowViewer},
    workflow::WorkNode,
};

pub const NONE: Modifiers = Modifiers::NONE;
pub const CTRL: Modifiers = Modifiers::CTRL;
pub const SHIFT: Modifiers = Modifiers::SHIFT;
pub const CTRL_SHIFT: Modifiers = Modifiers::CTRL.plus(Modifiers::SHIFT);

const fn shortcut(modifiers: Modifiers, logical_key: Key) -> KeyboardShortcut {
    KeyboardShortcut {
        modifiers,
        logical_key,
    }
}
#[derive(Debug, Clone, Assoc, Sequence)]
#[func(pub const fn key(&self) -> KeyboardShortcut)]
pub enum Shortcut {
    #[assoc(key=shortcut(CTRL, Key::Q))]
    Quit,

    #[assoc(key=shortcut(CTRL, Key::X))]
    Cut,

    #[assoc(key=shortcut(CTRL, Key::C))]
    Copy,

    #[assoc(key=shortcut(CTRL, Key::V))]
    Paste,

    #[assoc(key=shortcut(NONE, Key::Backspace))]
    LeaveSubgraph,

    #[assoc(key=shortcut(CTRL, Key::R))]
    RunWorkflow,

    #[assoc(key=shortcut(SHIFT, Key::Questionmark))]
    Help,

    #[assoc(key=shortcut(NONE, Key::Space))]
    FreezeWorkflow,

    #[assoc(key=shortcut(CTRL, Key::Z))]
    Undo,

    #[assoc(key=shortcut(CTRL_SHIFT, Key::Z))]
    Redo,

    #[assoc(key=shortcut(CTRL, Key::A))]
    SelectAll,

    #[assoc(key=shortcut(CTRL, Key::D))]
    SelectNone,

    #[assoc(key=shortcut(NONE, Key::D))]
    DisableNode,

    #[assoc(key=shortcut(NONE, Key::Delete))]
    RemoveNode,
}

pub const SHORTCUT_QUIT: KeyboardShortcut = Shortcut::Quit.key();

pub const SHORTCUT_EXIT_SUBGRAPH: KeyboardShortcut = Shortcut::LeaveSubgraph.key();

pub const SHORTCUT_RUN: KeyboardShortcut = Shortcut::RunWorkflow.key();

pub const SHORTCUT_HELP: KeyboardShortcut = Shortcut::Help.key();

pub const SHORTCUT_COPY: KeyboardShortcut = KeyboardShortcut {
    modifiers: Modifiers::CTRL,
    logical_key: Key::C,
};

pub const SHORTCUT_PASTE: KeyboardShortcut = KeyboardShortcut {
    modifiers: Modifiers::CTRL,
    logical_key: Key::V,
};

pub const SHORTCUT_CUT: KeyboardShortcut = KeyboardShortcut {
    modifiers: Modifiers::CTRL,
    logical_key: Key::X,
};

pub const SHORTCUT_FREEZE: KeyboardShortcut = Shortcut::FreezeWorkflow.key();

pub const SHORTCUT_UNDO: KeyboardShortcut = Shortcut::Undo.key();

pub const SHORTCUT_REDO: KeyboardShortcut = Shortcut::Redo.key();

pub const SHORTCUT_SELECT_ALL: KeyboardShortcut = Shortcut::SelectAll.key();

pub const SHORTCUT_SELECT_NONE: KeyboardShortcut = Shortcut::SelectNone.key();

pub const SHORTCUT_DISABLE_NODE: KeyboardShortcut = Shortcut::DisableNode.key();

pub const SHORTCUT_REMOVE_NODE: KeyboardShortcut = Shortcut::RemoveNode.key();

#[derive(TypedBuilder)]
pub struct ShortcutHandler<'a> {
    pub snarl: &'a mut Snarl<WorkNode>,
    pub viewer: &'a mut WorkflowViewer,
}

impl<'a> ShortcutHandler<'a> {
    pub fn viewer_shortcuts(&mut self, ui: &mut egui::Ui, widget: SnarlWidget) {
        let snarl = &mut *self.snarl;
        let viewer = &mut *self.viewer;

        if ui.ctx().input_mut(|input| {
            input
                .events
                .iter()
                .any(|ev| matches!(ev, egui::Event::Copy))
        }) {
            viewer.handle_copy(ui, widget);
        }

        if !viewer.running {
            if ui.ctx().input_mut(|i| i.consume_shortcut(&SHORTCUT_RUN)) {
                viewer.events.insert(AppEvent::UserRunWorkflow);
            }

            if ui.ctx().input_mut(|i| i.consume_shortcut(&SHORTCUT_FREEZE)) {
                viewer.events.insert(AppEvent::Freeze(None));
            }

            if ui.ctx().input_mut(|i| i.consume_shortcut(&SHORTCUT_REDO)) {
                viewer.events.insert(AppEvent::Redo);
            }

            if ui.ctx().input_mut(|i| i.consume_shortcut(&SHORTCUT_UNDO)) {
                viewer.events.insert(AppEvent::Undo);
            }
        }

        if ui
            .ctx()
            .input_mut(|i| i.consume_shortcut(&SHORTCUT_SELECT_ALL))
        {
            let nodes = snarl.node_ids().map(|(id, _)| id).collect_vec();
            widget.update_selected_nodes(ui, |selection| *selection = nodes);
        }

        if ui
            .ctx()
            .input_mut(|i| i.consume_shortcut(&SHORTCUT_SELECT_NONE))
        {
            widget.update_selected_nodes(ui, |selection| selection.clear());
        }

        if !viewer.frozen && !viewer.running {
            viewer.handle_paste(snarl, ui, widget);

            if ui
                .ctx()
                .input_mut(|input| input.events.iter().any(|ev| matches!(ev, egui::Event::Cut)))
            {
                tracing::debug!("Cutting selected nodes");
                viewer.handle_copy(ui, widget);
                viewer.remove_nodes(ui, snarl, None);
            }

            if ui
                .ctx()
                .input_mut(|i| i.consume_shortcut(&SHORTCUT_DISABLE_NODE))
            {
                viewer.disable_nodes(ui, snarl, None);
            }
            if ui
                .ctx()
                .input_mut(|i| i.consume_shortcut(&SHORTCUT_REMOVE_NODE))
            {
                viewer.remove_nodes(ui, snarl, None);
            }
        }

        if ui.ctx().input_mut(|input| {
            input
                .events
                .iter()
                .any(|ev| matches!(ev, egui::Event::Copy))
        }) {
            viewer.handle_copy(ui, widget);
        }

        if ui.ctx().input_mut(|i| {
            i.consume_shortcut(&SHORTCUT_EXIT_SUBGRAPH)
                | i.pointer.button_released(egui::PointerButton::Extra1)
        }) {
            viewer.events.insert(AppEvent::LeaveSubgraph(1));
        }
    }

    pub fn node_shortcuts(&mut self, ui: &mut egui::Ui, node: NodeId) {
        let snarl = &mut *self.snarl;
        let viewer = &mut *self.viewer;

        if viewer.running || viewer.frozen {
            return;
        }

        if ui
            .ctx()
            .input_mut(|i| i.consume_shortcut(&SHORTCUT_DISABLE_NODE))
        {
            viewer.disable_nodes(ui, snarl, Some(node));
        }

        if ui
            .ctx()
            .input_mut(|i| i.consume_shortcut(&SHORTCUT_REMOVE_NODE))
        {
            viewer.remove_nodes(ui, snarl, Some(node));
        }
    }
}

fn render_shortcut(ui: &mut egui::Ui, key: egui::KeyboardShortcut) {
    let text = if key == SHORTCUT_HELP {
        "?".to_string()
    } else {
        ui.ctx().format_shortcut(&key)
    };

    dumpling(ui, &text);
}

fn dumpling(ui: &mut egui::Ui, text: &str) {
    ui.add(egui::Button::new(egui::RichText::new(text).monospace()).sense(Sense::empty()));
}

pub fn show_shortcuts(ui: &mut egui::Ui) {
    ui.set_min_size(egui::vec2(300.0, 400.0));
    ui.vertical_centered(|ui| ui.heading("Help"));
    ui.columns_const(|[col0, col1]| {
        col0.vertical(|ui| {
            egui::Grid::new("shortcuts 0")
                .num_columns(2)
                .show(ui, |ui| {
                    render_shortcut(ui, SHORTCUT_SELECT_ALL);
                    ui.label("Select all nodes in the graph");
                    ui.end_row();

                    render_shortcut(ui, SHORTCUT_SELECT_NONE);
                    ui.label("Clear the selection");
                    ui.end_row();

                    render_shortcut(ui, SHORTCUT_COPY);
                    ui.label("Copy the selected nodes");
                    ui.end_row();

                    render_shortcut(ui, SHORTCUT_CUT);
                    ui.label("Cut the selected nodes");
                    ui.end_row();

                    render_shortcut(ui, SHORTCUT_PASTE);
                    ui.label("Paste nodes from the clipboard");
                    ui.end_row();

                    ui.end_row();

                    render_shortcut(ui, SHORTCUT_DISABLE_NODE);
                    ui.label("Disable the node(s) under the cursor");
                    ui.end_row();

                    render_shortcut(ui, SHORTCUT_REMOVE_NODE);
                    ui.label("Remove the node(s) under the cursor");
                    ui.end_row();

                    ui.end_row();

                    render_shortcut(ui, SHORTCUT_RUN);
                    ui.label("Run the current workflow");
                    ui.end_row();

                    render_shortcut(ui, SHORTCUT_FREEZE);
                    ui.label("Freeze/thaw the workflow editor");
                    ui.end_row();

                    render_shortcut(ui, SHORTCUT_UNDO);
                    ui.label("Undo the last edit");
                    ui.end_row();

                    render_shortcut(ui, SHORTCUT_REDO);
                    ui.label("Redo undone edits");
                    ui.end_row();

                    render_shortcut(ui, SHORTCUT_EXIT_SUBGRAPH);
                    ui.label("Leave the subgraph");
                    ui.end_row();
                });
        });

        col1.vertical(|ui| {
            egui::Grid::new("shortcuts 2")
                .num_columns(1)
                .show(ui, |ui| {
                    dumpling(ui, "Double+Click");
                    ui.label("on canvas to fit view to workflow.");
                    ui.end_row();

                    ui.end_row();

                    dumpling(ui, "Shift+Click");
                    ui.label("on a node to select it");
                    ui.end_row();

                    dumpling(ui, "Ctrl+Click");
                    ui.label("on a node to deselect it");
                    ui.end_row();

                    ui.end_row();

                    dumpling(ui, "Ctrl+Click");
                    ui.label("on the canvas to deselect all nodes.");
                    ui.end_row();

                    dumpling(ui, "Shift+Drag");
                    ui.label("on the canvas to box select nodes.");
                    ui.end_row();

                    dumpling(ui, "Shift+Drag");
                    ui.label("on the canvas to box select nodes.");
                    ui.end_row();

                    dumpling(ui, "Ctrl+Shift+Drag");
                    ui.label("on the canvas to box deselect.");
                    ui.end_row();

                    ui.end_row();

                    render_shortcut(ui, SHORTCUT_QUIT);
                    ui.label("Quit the application");
                    ui.end_row();

                    render_shortcut(ui, SHORTCUT_HELP);
                    ui.label("Show this help");
                    ui.end_row();
                });
        });
    });
}

pub fn squelch(resp: egui::Response) -> egui::Response {
    if resp.has_focus() {
        resp.ctx
            .input_mut(|i| i.events.retain(|ev| !is_shortcut_event(ev)));
    }
    resp
}

#[inline]
fn is_shortcut_event(ev: &Event) -> bool {
    if matches!(
        ev,
        egui::Event::Copy | egui::Event::Cut | egui::Event::Paste(_)
    ) {
        return true;
    }

    enum_iterator::all::<Shortcut>().any(|shortcut| {
        matches!(
            ev,
            Event::Key {
                key,
                modifiers,
                ..
            } if *key == shortcut.key().logical_key && modifiers.matches_logically(shortcut.key().modifiers)
        )
    })
}
