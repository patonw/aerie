use std::{borrow::Cow, convert::identity, sync::Arc};

use decorum::E64;
use egui::RichText;
use egui_phosphor::regular::{ARROW_CIRCLE_DOWN, ARROW_CIRCLE_UP, TRASH};
use egui_snarl::OutPinId;
use itertools::Itertools;
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::{utils::message_text, workflow::WorkflowError};

use super::{DynNode, EditContext, RunContext, UiNode, Value, ValueKind};

// These fields will always be set from the run context each execution.
// Saving them to disk is just a waste.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Start {}

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
        5
    }

    fn out_kind(&self, out_pin: usize) -> ValueKind {
        match out_pin {
            0 => ValueKind::Model,
            1 => ValueKind::Number,
            2 => ValueKind::Chat,
            3 => ValueKind::Json,
            4 => ValueKind::Text,
            _ => unreachable!(),
        }
    }

    fn execute(
        &mut self,
        ctx: &RunContext,
        _node_id: egui_snarl::NodeId,
        _inputs: Vec<Option<Value>>,
    ) -> Result<Vec<Value>, WorkflowError> {
        let schema: serde_json::Value = if !ctx.graph.schema.is_empty() {
            serde_json::from_str(&ctx.graph.schema)
                .map_err(|_| WorkflowError::Conversion("Invalid input schema".into()))?
        } else {
            serde_json::json!({})
        };
        Ok(vec![
            Value::Model(ctx.model.clone()),
            Value::Number(E64::assert(ctx.temperature)),
            Value::Chat(ctx.history.load().clone()),
            Value::Json(Arc::new(schema)),
            Value::Text(ctx.user_prompt.clone()),
        ])
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
                ui.label("schema");
            }
            4 => {
                ui.label("input");
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

    fn in_kinds(&'_ self, in_pin: usize) -> Cow<'_, [ValueKind]> {
        match in_pin {
            0 => Cow::Borrowed(&[ValueKind::Chat]),
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

    fn in_kinds(&'_ self, in_pin: usize) -> Cow<'_, [ValueKind]> {
        let is_var_pin = in_pin > 0 && in_pin < self.kinds.len() + 1;
        Cow::Borrowed(match in_pin {
            0 => &[ValueKind::Failure],
            _ if is_var_pin && self.kinds[in_pin - 1] != ValueKind::Placeholder => {
                std::slice::from_ref(&self.kinds[in_pin - 1])
            }
            _ => ValueKind::all(),
        })
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

#[derive(Debug, Clone, Hash, PartialEq, Eq, Deserialize, Serialize)]
pub struct Matcher {
    kind: ValueKind,

    patterns: im::Vector<String>,

    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    exact: bool,

    #[serde(skip)]
    editing: Option<usize>,
}

impl Default for Matcher {
    fn default() -> Self {
        Self {
            kind: Default::default(),
            patterns: Default::default(),
            exact: true,
            editing: Default::default(),
        }
    }
}

impl DynNode for Matcher {
    fn inputs(&self) -> usize {
        2
    }

    fn in_kinds(&'_ self, in_pin: usize) -> Cow<'_, [ValueKind]> {
        Cow::Borrowed(match in_pin {
            0 => &[
                ValueKind::Text,
                ValueKind::Message,
                ValueKind::Number,
                ValueKind::Integer,
                ValueKind::Json,
            ],
            _ if self.kind == ValueKind::Placeholder => ValueKind::all(),
            _ => std::slice::from_ref(&self.kind),
        })
    }

    fn outputs(&self) -> usize {
        self.patterns.len() + 1
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
        let mut result = vec![Value::Placeholder(self.kind); self.patterns.len() + 1];
        let default_pin = self.patterns.len();

        let data = match &inputs[1] {
            Some(value) => value.clone(),
            None => {
                return Ok(vec![]);
            }
        };

        match &inputs[0] {
            Some(Value::Number(number)) => {
                if let Some(pos) = self.match_float_range(number.into_inner())? {
                    result[pos] = data;
                } else {
                    result[default_pin] = data;
                }

                return Ok(result);
            }
            Some(Value::Integer(number)) => {
                if let Some(pos) = self.match_int_range(*number)? {
                    result[pos] = data;
                } else {
                    result[default_pin] = data;
                }

                return Ok(result);
            }
            Some(Value::Json(value)) => {
                if let serde_json::Value::Number(number) = value.as_ref() {
                    if let Some(pos) = self.match_float_range(number.as_f64().unwrap())? {
                        result[pos] = data;
                    } else {
                        result[default_pin] = data;
                    }

                    return Ok(result);
                }
            }
            _ => {}
        }

        let key = match &inputs[0] {
            Some(Value::Text(text)) => text.clone(),
            Some(Value::Message(message)) => message_text(message),
            Some(Value::Json(value)) => match value.as_ref() {
                serde_json::Value::String(text) => text.clone(),
                _ => Err(WorkflowError::Conversion(format!(
                    "Unsuppported conversion: {value:?}"
                )))?,
            },
            None => Err(WorkflowError::Required(vec!["Key is required".into()]))?,
            _ => unreachable!(),
        };

        let pos = self.match_strings(key)?;
        let out_pin = pos.unwrap_or(default_pin);
        result[out_pin] = data;

        Ok(result)
    }
}

impl Matcher {
    fn match_int_range(&mut self, key: i64) -> anyhow::Result<Option<usize>> {
        // TODO: lossless
        self.match_float_range(key as f64)
    }

    fn match_float_range(&mut self, key: f64) -> anyhow::Result<Option<usize>> {
        for (i, pattern) in self.patterns.iter().enumerate() {
            for pattern in pattern.split('|') {
                if self.exact {
                    if pattern.trim().parse::<f64>()? == key {
                        return Ok(Some(i));
                    }
                } else if let Some((min, max)) = pattern.split_once("..") {
                    let (closed, max) = if let Some(max) = max.strip_prefix('=') {
                        (true, max)
                    } else {
                        (false, max)
                    };

                    let min = min.trim().parse::<f64>()?;
                    let max = max.trim().parse::<f64>()?;

                    if closed {
                        if (min..=max).contains(&key) {
                            return Ok(Some(i));
                        }
                    } else if (min..max).contains(&key) {
                        return Ok(Some(i));
                    }
                } else if pattern.trim().parse::<f64>()? == key {
                    return Ok(Some(i));
                }
            }
        }

        Ok(None)
    }

    fn match_strings(&mut self, key: String) -> anyhow::Result<Option<usize>> {
        for (i, pattern) in self.patterns.iter().enumerate() {
            if self.exact || pattern.is_empty() {
                for pattern in pattern.split('|') {
                    if pattern.trim() == key.trim() {
                        return Ok(Some(i));
                    }
                }
            } else {
                let rx = Regex::new(pattern)?;
                if rx.is_match(&key) {
                    return Ok(Some(i));
                }
            }
        }

        Ok(None)
    }
}

impl UiNode for Matcher {
    fn title(&self) -> &str {
        "Match"
    }

    fn tooltip(&self) -> &str {
        "Routes the data input to the output that matches the key"
    }

    fn show_input(
        &mut self,
        ui: &mut egui::Ui,
        _ctx: &EditContext,
        pin_id: usize,
        remote: Option<Value>,
    ) -> egui_snarl::ui::PinInfo {
        match pin_id {
            0 => {
                ui.label("key");
            }
            1 => {
                let in_kind = match &remote {
                    Some(Value::Placeholder(kind)) => Some(*kind),
                    Some(value) => Some(value.kind()),
                    _ => None,
                };

                // TODO: reset all output pins on type change
                if self.kind == ValueKind::Placeholder
                    && let Some(in_kind) = in_kind
                {
                    self.kind = in_kind;
                }

                ui.label("data");
            }
            _ => unreachable!(),
        }

        self.in_kinds(pin_id).first().unwrap().default_pin()
    }

    fn has_body(&self) -> bool {
        true
    }

    fn show_body(&mut self, ui: &mut egui::Ui, ctx: &EditContext) {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
            if ui.button("+new").clicked() {
                ctx.swap_outputs(
                    OutPinId {
                        node: ctx.current_node,
                        output: self.patterns.len(),
                    },
                    OutPinId {
                        node: ctx.current_node,
                        output: self.patterns.len() + 1,
                    },
                );
                self.patterns.push_back(Default::default());
                self.editing = Some(self.patterns.len() - 1);
            }

            ui.toggle_value(&mut self.exact, "exact");
        });
    }

    fn show_output(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &EditContext,
        pin_id: usize,
    ) -> egui_snarl::ui::PinInfo {
        if pin_id >= self.patterns.len() {
            ui.weak("(default)")
                .on_hover_text("If none of the following match, output to this pin");
        } else if let Some(editing) = self.editing
            && editing == pin_id
        {
            ui.spacing_mut().item_spacing.x = 4.0;
            let pattern = self.patterns.get_mut(editing).unwrap();
            let widget = egui::TextEdit::singleline(pattern).desired_width(200.0);
            let resp = ui.add(widget);

            ui.add_enabled_ui(pin_id > 0, |ui| {
                if ui.button(ARROW_CIRCLE_UP).clicked() {
                    ctx.swap_outputs(
                        OutPinId {
                            node: ctx.current_node,
                            output: pin_id,
                        },
                        OutPinId {
                            node: ctx.current_node,
                            output: pin_id - 1,
                        },
                    );

                    self.editing = Some(editing - 1);
                    self.patterns.swap(editing, editing - 1);
                    resp.request_focus();
                }
            });

            ui.add_enabled_ui(pin_id < self.patterns.len() - 1, |ui| {
                if ui.button(ARROW_CIRCLE_DOWN).clicked() {
                    ctx.swap_outputs(
                        OutPinId {
                            node: ctx.current_node,
                            output: pin_id,
                        },
                        OutPinId {
                            node: ctx.current_node,
                            output: pin_id + 1,
                        },
                    );

                    self.editing = Some(editing + 1);
                    self.patterns.swap(editing, editing + 1);
                    resp.request_focus();
                }
            });

            // Can only delete last one
            if pin_id == self.patterns.len() - 1
                && ui
                    .menu_button(TRASH, |ui| {
                        if ui.button("Remove").clicked() {
                            ctx.swap_outputs(
                                OutPinId {
                                    node: ctx.current_node,
                                    output: pin_id,
                                },
                                OutPinId {
                                    node: ctx.current_node,
                                    output: pin_id + 1,
                                },
                            );
                            ctx.drop_out_pin(OutPinId {
                                node: ctx.current_node,
                                output: pin_id + 1,
                            });

                            self.patterns.pop_back();
                        }
                    })
                    .response
                    .clicked()
            {
                resp.request_focus();
            }

            if resp.lost_focus() {
                self.editing = None;
            }

            resp.request_focus();
        } else {
            let pattern = &self.patterns[pin_id];
            let text = if pattern.is_empty() {
                RichText::new("(empty)").weak()
            } else {
                RichText::new(pattern)
            };
            let widget = egui::Label::new(text).truncate();
            if ui
                .add(widget)
                .interact(egui::Sense::click())
                .double_clicked()
            {
                self.editing = Some(pin_id);
            }
        }

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
    fn in_kinds(&'_ self, _in_pin: usize) -> Cow<'_, [ValueKind]> {
        Cow::Borrowed(if self.count == 0 {
            ValueKind::all()
        } else {
            std::slice::from_ref(&self.kind)
        })
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

    fn in_kinds(&'_ self, _in_pin: usize) -> Cow<'_, [ValueKind]> {
        Cow::Borrowed(if matches!(self.kind, ValueKind::Placeholder) {
            ValueKind::all()
        } else {
            std::slice::from_ref(&self.kind)
        })
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
