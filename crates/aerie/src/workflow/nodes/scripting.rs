#![cfg(feature = "scripting")]
use std::{borrow::Cow, collections::BTreeSet, sync::Arc};

use decorum::E64;
use egui::RichText;
use egui_phosphor::regular::{ARROW_CIRCLE_DOWN, ARROW_CIRCLE_UP, TRASH};
use egui_snarl::{InPinId, OutPinId};
use im::vector;
use itertools::Itertools;
use rhai::{Dynamic, Engine, Scope};
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

use crate::{
    ui::{AppEvent, resizable_frame, shortcuts::squelch},
    workflow::{
        AnyPin, DynNode as _, EditContext, Value, ValueKind, WorkNode, WorkflowError,
        nodes::GraphSubmenu,
    },
};

#[skip_serializing_none]
#[derive(Debug, Clone, Hash, PartialEq, Eq, Deserialize, Serialize)]
pub struct RhaiNode {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    name: String,

    script: String,

    #[serde(default, skip_serializing_if = "im::Vector::is_empty")]
    inputs: im::Vector<(String, ValueKind)>,

    #[serde(default, skip_serializing_if = "im::Vector::is_empty")]
    outputs: im::Vector<(String, ValueKind)>,

    size: Option<crate::utils::EVec2>,
}

impl Default for RhaiNode {
    fn default() -> Self {
        Self {
            name: Default::default(),
            script: Default::default(),
            inputs: vector![("input".into(), ValueKind::Text)],
            outputs: vector![("output".into(), ValueKind::Text)],
            size: Default::default(),
        }
    }
}

impl super::DynNode for RhaiNode {
    fn inputs(&self) -> usize {
        self.inputs.len()
    }

    fn in_kinds(&'_ self, in_pin: usize) -> Cow<'_, [ValueKind]> {
        if in_pin < self.inputs.len() {
            Cow::Borrowed(std::slice::from_ref(&self.inputs[in_pin].1))
        } else {
            Cow::Borrowed(&[ValueKind::Placeholder])
        }
    }

    fn outputs(&self) -> usize {
        self.outputs.len() + 1
    }

    fn out_kind(&self, out_pin: usize) -> ValueKind {
        if out_pin < self.outputs.len() {
            self.outputs[out_pin].1
        } else {
            ValueKind::Failure
        }
    }

    fn execute(
        &mut self,
        _ctx: &super::RunContext,
        _node_id: egui_snarl::NodeId,
        inputs: Vec<Option<super::Value>>,
    ) -> Result<Vec<super::Value>, crate::workflow::WorkflowError> {
        self.validate(&inputs)?;

        let engine = Engine::new();
        let mut scope = Scope::new();
        scope.push("hello", 42_i64);

        for (i, item) in inputs.iter().enumerate() {
            let name = self.inputs[i].0.as_str();
            let name: String = name
                .chars()
                .map(|c| match c {
                    'A'..='Z' | 'a'..='z' | '0'..='9' => c,
                    _ => '_',
                })
                .collect();
            let Some(value) = item else {
                scope.push(name, Dynamic::UNIT);
                continue;
            };

            use Value::*;
            match value {
                Text(text) => {
                    // TODO: bind without cloning
                    scope.push(name, (**text).clone());
                }
                Integer(value) => {
                    scope.push(name, *value);
                }
                Number(value) => {
                    scope.push(name, value.into_inner());
                }
                Json(value) => {
                    // TODO: convert without cloning
                    let dynamic: Dynamic = serde_json::from_value((**value).clone()).unwrap();
                    scope.push(name, dynamic);
                }
                TextList(value) => {
                    let items = value
                        .iter()
                        .map(|s| s.as_str().to_string())
                        .map(Dynamic::from)
                        .collect_vec();
                    scope.push(name, items);
                }
                IntList(value) => {
                    let items = value.iter().cloned().map(Dynamic::from).collect_vec();
                    scope.push(name, items);
                }
                FloatList(value) => {
                    let items = value
                        .iter()
                        .map(|it| it.into_inner())
                        .map(Dynamic::from)
                        .collect_vec();
                    scope.push(name, items);
                }
                _ => {
                    scope.push(name, Dynamic::UNIT);
                }
            }
        }

        let result = engine.eval_with_scope::<Dynamic>(&mut scope, &self.script)?;
        tracing::debug!("Result is {result:?}");

        let mut output = vec![Value::Placeholder(ValueKind::Placeholder); self.outputs.len()];

        if output.len() == 1 {
            output[0] = rhai_to_value(&result, self.outputs[0].1).map_err(|e| {
                WorkflowError::Conversion(format!("Expected a string but got a {e}"))
            })?;
        } else if result.is_array() {
            let values = result.as_array_ref().unwrap();
            if output.len() != values.len() {
                Err(WorkflowError::Conversion(format!(
                    "Expected script to output {} values",
                    output.len()
                )))?;
            }

            for (i, ((_name, kind), value)) in self.outputs.iter().zip(values.iter()).enumerate() {
                output[i] = rhai_to_value(value, *kind).map_err(|e| {
                    WorkflowError::Conversion(format!("Expected a {kind:?} but got a {e}"))
                })?;
            }
        } else if result.is_map() {
            let values = result.as_map_ref().unwrap();
            let keys: BTreeSet<_> = values.keys().map(|k| k.to_string()).collect();
            let missing = self
                .outputs
                .iter()
                .filter(|(name, _)| !keys.contains(name))
                .collect_vec();
            if !missing.is_empty() {
                Err(WorkflowError::Conversion(format!(
                    "Expected script to output a map containing {:?} but got {keys:?}",
                    self.outputs.iter().map(|(n, _)| n).collect_vec()
                )))?;
            }

            for (i, (name, kind)) in self.outputs.iter().enumerate() {
                let value = values.get(name.as_str()).unwrap();
                output[i] = rhai_to_value(value, *kind).map_err(|e| {
                    WorkflowError::Conversion(format!("Expected a {kind:?} but got a {e}"))
                })?;
            }
        } else {
            Err(WorkflowError::Conversion(
                "Script must output multiple values in a map or array".to_string(),
            ))?;
        }

        Ok(output)
    }
}

fn rhai_to_value(data: &Dynamic, kind: ValueKind) -> Result<Value, &'static str> {
    use ValueKind::*;
    Ok(match kind {
        Text => Value::Text(Arc::new(data.to_string())),
        Integer => Value::Integer(data.as_int()?),
        Number => Value::Number(E64::assert(data.as_float()?)),
        Json => Value::Json(Arc::new(serde_json::to_value(data).unwrap())),
        TextList => {
            let dyn_arr = data.as_array_ref()?;
            let items: Result<im::Vector<_>, _> =
                dyn_arr.iter().map(|v| v.clone().into_string()).collect();
            let items = items?.into_iter().map(Arc::new).collect();
            Value::TextList(items)
        }
        IntList => {
            let dyn_arr = data.as_array_ref()?;
            let items: Result<im::Vector<_>, _> = dyn_arr.iter().map(|v| v.as_int()).collect();
            Value::IntList(items?)
        }
        FloatList => {
            let dyn_arr = data.as_array_ref()?;
            let items: Result<im::Vector<_>, _> = dyn_arr.iter().map(|v| v.as_float()).collect();
            let items = items?.iter().map(|x| E64::assert(*x)).collect();
            Value::FloatList(items)
        }
        _ => unreachable!(),
    })
}

impl super::UiNode for RhaiNode {
    fn title(&self) -> &str {
        if self.name.is_empty() {
            "Rhai script"
        } else {
            self.name.as_str()
        }
    }

    fn title_mut(&mut self) -> Option<&mut String> {
        Some(&mut self.name)
    }

    fn tooltip(&self) -> &str {
        "Run a rhai script on the node inputs.\n\
            Inputs are added to the scope by pin name.\n\
            Unconnected pins receive the `()` value.\n\
            Test against `()` to set default values as needed."
    }

    fn help_link(&self) -> &str {
        "https://rhai.rs/book/language/"
    }

    fn has_body(&self) -> bool {
        true
    }

    fn show_body(&mut self, ui: &mut egui::Ui, _ctx: &super::EditContext) {
        ui.vertical(|ui| {
            ui.menu_button("+in", |ui| {
                let kinds = [
                    ValueKind::Text,
                    ValueKind::TextList,
                    ValueKind::Number,
                    ValueKind::FloatList,
                    ValueKind::Integer,
                    ValueKind::IntList,
                    ValueKind::Json,
                ];
                for kind in kinds {
                    let mut label = kind.to_string().to_lowercase();
                    if kind.is_list() {
                        label = format!("[{label}]");
                    }
                    if ui.button(&label).clicked() {
                        self.inputs = self.inputs.clone();
                        self.inputs.push_back((label, kind));
                    }
                }
            });

            ui.add_space(16.0);
            resizable_frame(&mut self.size, ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    let theme = egui_extras::syntax_highlighting::CodeTheme::from_memory(
                        ui.ctx(),
                        ui.style(),
                    );
                    let mut layouter =
                        |ui: &egui::Ui, buf: &dyn egui::TextBuffer, wrap_width: f32| {
                            let mut layout_job = egui_extras::syntax_highlighting::highlight(
                                ui.ctx(),
                                ui.style(),
                                &theme,
                                buf.as_str(),
                                "rs",
                            );
                            layout_job.wrap.max_width = wrap_width;
                            ui.fonts_mut(|f| f.layout_job(layout_job))
                        };
                    let widget = egui::TextEdit::multiline(&mut self.script)
                        .id_salt("rhai script")
                        .font(egui::TextStyle::Monospace) // for cursor height
                        .code_editor()
                        .desired_rows(10)
                        .lock_focus(true)
                        .desired_width(f32::INFINITY)
                        .layouter(&mut layouter);

                    squelch(ui.add_sized(ui.available_size(), widget));
                });
            });
            ui.add_space(8.0);
        });
    }

    fn show_input(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &EditContext,
        pin_id: usize,
        _remote: Option<Value>,
    ) -> egui_snarl::ui::PinInfo {
        if ctx.edit_pin.load().as_ref() == &Some(AnyPin::input(ctx.current_node, pin_id)) {
            ui.spacing_mut().item_spacing.x = 4.0;
            let name = self.inputs.get_mut(pin_id).unwrap();
            let widget = egui::TextEdit::singleline(&mut name.0).desired_width(100.0);
            let resp = squelch(ui.add(widget));

            ui.add_enabled_ui(pin_id > 0, |ui| {
                if ui.button(ARROW_CIRCLE_UP).clicked() {
                    ctx.events.insert(AppEvent::SwapInputs(
                        ctx.current_graph,
                        InPinId {
                            node: ctx.current_node,
                            input: pin_id,
                        },
                        InPinId {
                            node: ctx.current_node,
                            input: pin_id - 1,
                        },
                    ));

                    ctx.edit_pin
                        .store(Arc::new(Some(AnyPin::input(ctx.current_node, pin_id - 1))));
                    self.inputs.swap(pin_id, pin_id - 1);
                    resp.request_focus();
                }
            });

            ui.add_enabled_ui(pin_id < self.inputs.len() - 1, |ui| {
                if ui.button(ARROW_CIRCLE_DOWN).clicked() {
                    ctx.events.insert(AppEvent::SwapInputs(
                        ctx.current_graph,
                        InPinId {
                            node: ctx.current_node,
                            input: pin_id,
                        },
                        InPinId {
                            node: ctx.current_node,
                            input: pin_id + 1,
                        },
                    ));

                    ctx.edit_pin
                        .store(Arc::new(Some(AnyPin::input(ctx.current_node, pin_id + 1))));
                    self.inputs.swap(pin_id, pin_id + 1);
                    resp.request_focus();
                }
            });
            if ui.button(TRASH).clicked() {
                let event = AppEvent::PinRemoved(
                    ctx.current_graph,
                    AnyPin::input(ctx.current_node, pin_id),
                );
                tracing::debug!("Removing pin on subgraph: {event:?}");
                ctx.events.insert(event);

                self.inputs.remove(pin_id);
            }

            if resp.lost_focus() {
                ctx.edit_pin.store(Arc::new(None));
            }

            resp.request_focus();
        } else {
            let name = self.inputs.get(pin_id).map(|x| x.0.as_str()).unwrap_or("");
            let text = if name.is_empty() {
                RichText::new("(empty)").weak()
            } else {
                RichText::new(name)
            };

            let widget = egui::Label::new(text).truncate();
            if ui
                .add(widget)
                .interact(egui::Sense::click())
                .double_clicked()
            {
                ctx.edit_pin
                    .store(Arc::new(Some(AnyPin::input(ctx.current_node, pin_id))));
            }
        }

        self.in_kinds(pin_id).first().unwrap().default_pin()
    }

    fn show_output(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &EditContext,
        pin_id: usize,
    ) -> egui_snarl::ui::PinInfo {
        if pin_id == self.outputs.len() {
            ui.label("failure");
        } else if ctx.edit_pin.load().as_ref() == &Some(AnyPin::output(ctx.current_node, pin_id)) {
            ui.spacing_mut().item_spacing.x = 4.0;
            let name = self.outputs.get_mut(pin_id).unwrap();
            let widget = egui::TextEdit::singleline(&mut name.0).desired_width(100.0);
            let resp = squelch(ui.add(widget));

            ui.add_enabled_ui(pin_id > 0, |ui| {
                if ui.button(ARROW_CIRCLE_UP).clicked() {
                    ctx.events.insert(AppEvent::SwapOutputs(
                        ctx.current_graph,
                        OutPinId {
                            node: ctx.current_node,
                            output: pin_id,
                        },
                        OutPinId {
                            node: ctx.current_node,
                            output: pin_id - 1,
                        },
                    ));

                    ctx.edit_pin
                        .store(Arc::new(Some(AnyPin::output(ctx.current_node, pin_id - 1))));
                    self.outputs.swap(pin_id, pin_id - 1);
                    resp.request_focus();
                }
            });

            ui.add_enabled_ui(pin_id < self.outputs.len() - 1, |ui| {
                if ui.button(ARROW_CIRCLE_DOWN).clicked() {
                    ctx.events.insert(AppEvent::SwapOutputs(
                        ctx.current_graph,
                        OutPinId {
                            node: ctx.current_node,
                            output: pin_id,
                        },
                        OutPinId {
                            node: ctx.current_node,
                            output: pin_id + 1,
                        },
                    ));

                    ctx.edit_pin
                        .store(Arc::new(Some(AnyPin::output(ctx.current_node, pin_id + 1))));
                    self.outputs.swap(pin_id, pin_id + 1);
                    resp.request_focus();
                }
            });
            if ui.button(TRASH).clicked() {
                let event = AppEvent::PinRemoved(
                    ctx.current_graph,
                    AnyPin::output(ctx.current_node, pin_id),
                );
                tracing::debug!("Removing pin on subgraph: {event:?}");
                ctx.events.insert(event);

                self.outputs.remove(pin_id);
            }

            if resp.lost_focus() {
                ctx.edit_pin.store(Arc::new(None));
            }

            resp.request_focus();
        } else {
            let name = self.outputs.get(pin_id).map(|x| x.0.as_str()).unwrap_or("");
            let text = if name.is_empty() {
                RichText::new("(empty)").weak()
            } else {
                RichText::new(name)
            };

            let widget = egui::Label::new(text).truncate();
            if ui
                .add(widget)
                .interact(egui::Sense::click())
                .double_clicked()
            {
                ctx.edit_pin
                    .store(Arc::new(Some(AnyPin::output(ctx.current_node, pin_id))));
            }
        }

        self.out_kind(pin_id).default_pin()
    }

    fn has_footer(&self) -> bool {
        true
    }

    fn show_footer(&mut self, ui: &mut egui::Ui, ctx: &EditContext) {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
            ui.menu_button("+out", |ui| {
                let kinds = [
                    ValueKind::Text,
                    ValueKind::TextList,
                    ValueKind::Number,
                    ValueKind::FloatList,
                    ValueKind::Integer,
                    ValueKind::IntList,
                    ValueKind::Json,
                ];
                for kind in kinds {
                    let mut label = kind.to_string().to_lowercase();
                    if kind.is_list() {
                        label = format!("[{label}]");
                    }
                    if ui.button(&label).clicked() {
                        self.outputs = self.outputs.clone();
                        self.outputs.push_back((label, kind));

                        // Shift failure pin on subgraph node
                        let pin_id = self.outputs.len() - 1;
                        ctx.events.insert(AppEvent::SwapOutputs(
                            ctx.current_graph,
                            OutPinId {
                                node: ctx.current_node,
                                output: pin_id,
                            },
                            OutPinId {
                                node: ctx.current_node,
                                output: pin_id + 1,
                            },
                        ));
                    }
                }
            });
        });
    }
}

#[typetag::serde]
impl super::FlexNode for RhaiNode {}

fn script_node_menu(ui: &mut egui::Ui, snarl: &mut egui_snarl::Snarl<WorkNode>, pos: egui::Pos2) {
    ui.menu_button("Scripting", |ui| {
        if ui.button("Rhai").clicked() {
            snarl.insert_node(pos, RhaiNode::default().into());
            ui.close();
        }
    });
}

inventory::submit! {
    GraphSubmenu("scripting", script_node_menu)
}
