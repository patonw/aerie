use std::{borrow::Cow, sync::atomic::Ordering};

use itertools::Itertools;
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

use crate::{
    utils::ErrorDistiller as _,
    workflow::{DynNode, ShadowGraph, UiNode, ValueKind, WorkNode, runner::WorkflowRunner},
};

// "serializing nested enums in YAML is not supported yet"
// So we'll embed the enum into the node struct instead
/// Controls execution behavior
#[derive(Debug, Default, Clone, Hash, PartialEq, Eq, Deserialize, Serialize)]
pub enum Flavor {
    #[default]
    Simple,
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

impl DynNode for Subgraph {
    fn inputs(&self) -> usize {
        self.graph
            .start_node()
            .map(|n| n.outputs())
            .unwrap_or_default()
    }

    fn in_kinds(&'_ self, in_pin: usize) -> Cow<'_, [super::ValueKind]> {
        let Some(start) = self.graph.start_node() else {
            return Cow::Borrowed(&[]);
        };

        Cow::Owned(vec![start.out_kind(in_pin)])
    }

    fn outputs(&self) -> usize {
        self.graph
            .finish_node()
            .map(|n| n.inputs())
            .unwrap_or_default()
    }

    fn out_kind(&self, out_pin: usize) -> ValueKind {
        let Some(finish) = self.graph.finish_node() else {
            return ValueKind::Placeholder;
        };

        finish.in_kinds(out_pin)[0]
    }

    fn execute(
        &mut self,
        ctx: &super::RunContext,
        _node_id: egui_snarl::NodeId,
        inputs: Vec<Option<super::Value>>,
    ) -> Result<Vec<super::Value>, crate::workflow::WorkflowError> {
        self.validate(&inputs)?;

        // TODO: customize context for subgraph
        // What to do about outputs channel?
        let mut exec = WorkflowRunner::builder()
            .inputs(inputs)
            .run_ctx(ctx.clone())
            .build();

        // TODO: keep global node states keyed by graph uuid
        let node_state = exec.init(&self.graph);
        let interrupt = ctx.interrupt.clone();
        let errors = ctx.errors.clone();

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
                    tracing::info!("Stepped subgraph");
                }
                Err(err) => {
                    errors.push(err.into());
                    break;
                }
            }

            tracing::info!("Executed subgraph. Final state: {node_state:?}");
        }

        let results = exec
            .outputs
            .iter()
            .map(|it| it.clone().unwrap_or_default())
            .collect_vec();

        Ok(results)
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

    fn has_body(&self) -> bool {
        true
    }

    fn show_input(
        &mut self,
        ui: &mut egui::Ui,
        _ctx: &super::EditContext,
        pin_id: usize,
        _remote: Option<super::Value>,
    ) -> egui_snarl::ui::PinInfo {
        if let Some(start) = self.graph.start_node() {
            ui.label(&start.fields[pin_id].0);
        };

        self.in_kinds(pin_id).first().unwrap().default_pin()
    }

    fn show_output(
        &mut self,
        ui: &mut egui::Ui,
        _ctx: &super::EditContext,
        pin_id: usize,
    ) -> egui_snarl::ui::PinInfo {
        if let Some(finish) = self.graph.finish_node() {
            ui.label(&finish.fields[pin_id].0);
        };

        self.out_kind(pin_id).default_pin()
    }
}
