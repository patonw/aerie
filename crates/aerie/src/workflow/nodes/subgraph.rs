use std::{
    borrow::Cow,
    sync::{Arc, atomic::Ordering},
};

use egui::{Sense, UiBuilder};
use egui_phosphor::regular::{GRAPH, LINE_SEGMENTS};
use im::vector;
use itertools::Itertools;
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

use crate::workflow::{
    DynNode, FlexNode, ShadowGraph, UiNode, Value, ValueKind, WorkNode, WorkflowError,
    runner::WorkflowRunner,
};

// "serializing nested enums in YAML is not supported yet"
// So we'll embed the enum into the node struct instead
/// Controls execution behavior
#[derive(Debug, Default, Clone, Hash, PartialEq, Eq, Deserialize, Serialize)]
pub enum Flavor {
    #[default]
    Simple,

    Iterative,
}

impl Flavor {
    pub fn is_simple(&self) -> bool {
        *self == Self::Simple
    }
}

#[skip_serializing_none]
#[derive(Debug, Clone, Hash, PartialEq, Eq, Deserialize, Serialize)]
pub struct Subgraph {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,

    #[serde(default, skip_serializing_if = "Flavor::is_simple")]
    pub flavor: Flavor,

    pub graph: ShadowGraph<WorkNode>,
}

#[typetag::serde]
impl FlexNode for Subgraph {}

impl Default for Subgraph {
    fn default() -> Self {
        let bytes = include_bytes!("./default_subgraph.yml");
        let graph = serde_yml::from_slice::<ShadowGraph<WorkNode>>(bytes).unwrap();

        Self {
            title: "Subgraph".to_string(),
            graph,
            flavor: Flavor::Simple,
        }
    }
}

impl Subgraph {
    pub fn with_flavor(self, flavor: Flavor) -> Self {
        Self { flavor, ..self }
    }

    fn exec_simple(
        &mut self,
        ctx: &super::RunContext,
        inputs: Vec<Option<Value>>,
    ) -> Result<Vec<Value>, WorkflowError> {
        // TODO: customize context for subgraph
        // What to do about outputs channel?
        let mut exec = WorkflowRunner::builder()
            .inputs(inputs)
            .run_ctx(ctx.clone())
            .state_view(ctx.node_state.view(&self.graph.uuid))
            .build();

        exec.init(&self.graph);
        let interrupt = ctx.interrupt.clone();

        let mut target = egui_snarl::Snarl::try_from(self.graph.clone())?;
        tracing::info!("About to execute subgraph");

        loop {
            if interrupt.load(Ordering::Relaxed) {
                break;
            }

            match exec.step(&mut target) {
                Ok(false) => {
                    break;
                }
                Ok(true) => {
                    tracing::trace!("Stepped subgraph");
                }
                Err(err) => Err(WorkflowError::Subgraph(err))?,
            }
        }

        let mut results = exec
            .outputs
            .iter()
            .map(|it| it.clone().unwrap_or_default())
            .collect_vec();

        results.push(Value::Placeholder(ValueKind::Failure));

        tracing::info!("Executed subgraph. results {results:?}");

        Ok(results)
    }

    fn exec_foreach(
        &mut self,
        ctx: &super::RunContext,
        inputs: Vec<Option<Value>>,
    ) -> Result<Vec<Value>, WorkflowError> {
        use Value::*;
        if inputs.iter().flatten().all(|it| !it.kind().is_list()) {
            return self.exec_simple(ctx, inputs);
        }

        let lengths = inputs
            .iter()
            .filter_map(|x| x.clone())
            .filter(|v| v.kind().is_list())
            .map(|v| match v {
                TextList(items) => items.len(),
                IntList(items) => items.len(),
                FloatList(items) => items.len(),
                MsgList(items) => items.len(),
                Json(inner) if inner.is_array() => inner.as_array().unwrap().len(),
                _ => unreachable!(),
            })
            .collect_vec();

        if !lengths.iter().all(|s| *s == lengths[0]) {
            Err(WorkflowError::Conversion(format!(
                "List inputs are not the same length: {lengths:?}"
            )))?;
        }

        let mut results = vec![];

        for i in 0..lengths[0] {
            let sliced = inputs
                .iter()
                .map(|it| match it {
                    Some(TextList(items)) => Some(Text(items[i].clone())),
                    Some(FloatList(items)) => Some(Number(items[i])),
                    Some(IntList(items)) => Some(Integer(items[i])),
                    Some(MsgList(items)) => Some(Message((*items[i]).clone())),
                    Some(Json(arr)) if matches!(**arr, serde_json::Value::Array(_)) => {
                        let serde_json::Value::Array(items) = arr.as_ref() else {
                            unreachable!()
                        };
                        Some(Json(Arc::new(items[i].clone())))
                    }
                    value => value.clone(),
                })
                .collect_vec();

            let mut exec = WorkflowRunner::builder()
                .inputs(sliced)
                .run_ctx(ctx.clone())
                .state_view(ctx.node_state.view(&self.graph.uuid))
                .build();

            exec.init(&self.graph);
            let interrupt = ctx.interrupt.clone();

            let mut target = egui_snarl::Snarl::try_from(self.graph.clone())?;
            tracing::info!("About to execute subgraph");

            loop {
                if interrupt.load(Ordering::Relaxed) {
                    break;
                }

                match exec.step(&mut target) {
                    Ok(false) => {
                        break;
                    }
                    Ok(true) => {
                        tracing::trace!("Stepped subgraph");
                    }
                    Err(err) => Err(WorkflowError::Subgraph(err))?,
                }
            }

            if results.is_empty() {
                results = exec
                    .outputs
                    .iter()
                    .map(|it| match it {
                        Some(Text(_)) => TextList(vector![]),
                        Some(Integer(_)) => IntList(vector![]),
                        Some(Number(_)) => FloatList(vector![]),
                        Some(Message(_)) => MsgList(vector![]),
                        Some(Json(_)) => Json(Arc::new(serde_json::Value::Array(vec![]))),
                        Some(value) => value.clone(),
                        None => Default::default(),
                    })
                    .collect_vec();
            }

            for (res, val) in results.iter_mut().zip(exec.outputs.into_iter()) {
                match (res, val) {
                    (TextList(items), Some(Text(value))) => {
                        items.push_back(value);
                    }
                    (IntList(items), Some(Integer(value))) => {
                        items.push_back(value);
                    }
                    (FloatList(items), Some(Number(value))) => {
                        items.push_back(value);
                    }
                    (MsgList(items), Some(Message(value))) => {
                        items.push_back(Arc::new(value));
                    }
                    (Json(arr), Some(Json(value))) => {
                        let serde_json::Value::Array(items) = Arc::make_mut(arr) else {
                            unreachable!();
                        };

                        items.push((*value).clone());
                    }
                    (result, Some(value)) => {
                        *result = value;
                    }
                    (_, None) => {}
                }
            }
        }

        results.push(Value::Placeholder(ValueKind::Failure));

        Ok(results)
    }
}

impl DynNode for Subgraph {
    fn inputs(&self) -> usize {
        self.graph
            .start_node()
            .map(|n| n.outputs())
            .unwrap_or_default()
    }

    fn in_kinds(&'_ self, in_pin: usize) -> Cow<'_, [super::ValueKind]> {
        use ValueKind::*;
        let Some(start) = self.graph.start_node() else {
            return Cow::Borrowed(&[]);
        };

        match start.out_kind(in_pin) {
            Text => Cow::Owned(vec![TextList, Text]),
            Integer => Cow::Owned(vec![IntList, Integer]),
            Number => Cow::Owned(vec![FloatList, Number]),
            Message => Cow::Owned(vec![MsgList, Message]),
            kind => Cow::Owned(vec![kind]),
        }
    }

    fn outputs(&self) -> usize {
        self.graph
            .finish_node()
            .map(|n| n.inputs())
            .unwrap_or_default()
            + 1
    }

    // TODO: always include failure node last
    fn out_kind(&self, out_pin: usize) -> ValueKind {
        use ValueKind::*;
        let Some(finish) = self.graph.finish_node() else {
            return ValueKind::Placeholder;
        };

        if out_pin == finish.inputs() {
            ValueKind::Failure
        } else {
            match finish.in_kinds(out_pin)[0] {
                Text => TextList,
                Integer => IntList,
                Number => FloatList,
                Message => MsgList,
                kind => kind,
            }
        }
    }

    fn execute(
        &mut self,
        ctx: &super::RunContext,
        _node_id: egui_snarl::NodeId,
        inputs: Vec<Option<super::Value>>,
    ) -> Result<Vec<super::Value>, crate::workflow::WorkflowError> {
        self.validate(&inputs)?;

        match &self.flavor {
            Flavor::Simple => self.exec_simple(ctx, inputs),
            Flavor::Iterative => self.exec_foreach(ctx, inputs),
        }
    }
}

impl UiNode for Subgraph {
    fn on_paste(&mut self) {
        let uuid = crate::workflow::GraphId::new();
        let nodes = self
            .graph
            .nodes
            .iter()
            .map(|(k, v)| {
                let mut meta = v.clone();
                meta.value.as_ui_mut().on_paste();

                (*k, meta)
            })
            .collect();

        self.graph = ShadowGraph {
            uuid,
            nodes,
            ..self.graph.clone()
        };
    }

    fn title(&self) -> &str {
        &self.title
    }

    fn title_mut(&mut self) -> Option<&mut String> {
        Some(&mut self.title)
    }

    fn tooltip(&self) -> &str {
        match self.flavor {
            Flavor::Iterative => {
                "Runs a workflow for every item in the input list(s).\n\
                    All input lists must have the same length.\n\
                    Any scalar values will be broadcast to each run.\n\
                    Output values will be collected into output lists."
            }
            _ => {
                "Contains a workflow that executes independently when this node is run.\n\
                    Double click the icon to edit the internal graph.\n\
                    Customize the in/out pins by editing the Start/Finish nodes inside."
            }
        }
    }

    // TODO: allow adding/remove pins. Removal should drop connections inside graph.
    fn show_input(
        &mut self,
        ui: &mut egui::Ui,
        _ctx: &super::EditContext,
        pin_id: usize,
        _remote: Option<super::Value>,
    ) -> egui_snarl::ui::PinInfo {
        if let Some(start) = self.graph.start_node() {
            let text = start.fields.get(pin_id).map(|x| x.0.as_str()).unwrap_or("");
            ui.label(text);
        };

        self.in_kinds(pin_id).first().unwrap().default_pin()
    }

    fn show_output(
        &mut self,
        ui: &mut egui::Ui,
        _ctx: &super::EditContext,
        pin_id: usize,
    ) -> egui_snarl::ui::PinInfo {
        if pin_id == self.outputs() - 1 {
            ui.label("failure");
        } else if let Some(finish) = self.graph.finish_node() {
            let text = finish
                .fields
                .get(pin_id)
                .map(|x| x.0.as_str())
                .unwrap_or("");
            ui.label(text);
        };

        self.out_kind(pin_id).default_pin()
    }

    fn has_body(&self) -> bool {
        true
    }

    fn show_body(&mut self, ui: &mut egui::Ui, ctx: &super::EditContext) {
        let resp = ui.scope_builder(
            UiBuilder::new()
                .id_salt(ctx.current_node)
                .sense(Sense::click()),
            |ui| {
                ui.vertical_centered(|ui| {
                    ui.set_min_width(250.0);

                    ui.style_mut().interaction.selectable_labels = false;

                    match &self.flavor {
                        Flavor::Simple => ui
                            .label(egui::RichText::new(GRAPH).size(128.0))
                            .interact(egui::Sense::click())
                            .double_clicked(),
                        Flavor::Iterative => ui
                            .label(egui::RichText::new(LINE_SEGMENTS).size(128.0))
                            .interact(egui::Sense::click())
                            .double_clicked(),
                    }
                })
                .inner
            },
        );

        if resp.response.double_clicked() || resp.inner {
            ctx.events
                .insert(crate::ui::AppEvent::EnterSubgraph(ctx.current_node));
        }
    }
}

fn subgraph_menu(ui: &mut egui::Ui, snarl: &mut egui_snarl::Snarl<WorkNode>, pos: egui::Pos2) {
    ui.menu_button("Subgraph", |ui| {
        if ui.button("Simple").clicked() {
            snarl.insert_node(pos, Subgraph::default().into());
        }

        if ui.button("Iterative").clicked() {
            snarl.insert_node(
                pos,
                Subgraph::default().with_flavor(Flavor::Iterative).into(),
            );
        }
    });
}

inventory::submit! {
    super::GraphSubmenu("subgraph", subgraph_menu)
}
