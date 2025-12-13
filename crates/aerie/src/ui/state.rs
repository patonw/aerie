use arc_swap::ArcSwap;
use eframe::egui;
use egui::WidgetText;
use egui_commonmark::*;
use egui_snarl::{NodeId, Snarl, ui::SnarlStyle};
use rmcp::model::Tool;
use std::{
    collections::VecDeque,
    path::Path,
    sync::{
        Arc, RwLock,
        atomic::{AtomicBool, AtomicU16},
    },
    time::{Duration, SystemTime},
};
use uuid::Uuid;

use super::{Pane, tiles};
use crate::{
    AgentFactory, LogEntry, Settings, ToolSpec,
    chat::ChatSession,
    transmute::Transmuter,
    ui::tiles::messages::MessageGraph,
    utils::ErrorList,
    workflow::{ShadowGraph, WorkNode, fixup_workflow, runner::ExecState, store::WorkflowStore},
};

pub enum ToolEditorState {
    EditProvider {
        original: Option<(String, ToolSpec)>,
        modified: (String, ToolSpec),
    },
    ViewTool {
        tool: Tool,
    },
}

pub struct AppState {
    pub rt: tokio::runtime::Handle,
    pub errors: ErrorList<anyhow::Error>,
    pub settings: Arc<RwLock<Settings>>,
    pub task_count: Arc<AtomicU16>,
    pub log_history: Arc<RwLock<Vec<LogEntry>>>,
    pub session: ChatSession,
    pub cache: CommonMarkCache,
    pub prompt: Arc<RwLock<String>>,
    pub agent_factory: AgentFactory,

    // TODO: decompose
    pub branch_point: Option<Uuid>,
    pub new_branch: String,
    pub rename_branch: Option<String>,

    pub create_toolset: Option<String>,
    pub edit_toolset: String,

    pub tool_editor: Option<ToolEditorState>,

    pub workflows: WorkflowState,

    pub message_graph: MessageGraph,

    pub transmuter: Transmuter,
}

impl egui_tiles::Behavior<Pane> for AppState {
    fn tab_title_for_pane(&mut self, pane: &Pane) -> WidgetText {
        match pane {
            Pane::Settings => "Settings".into(),
            Pane::Navigator => "Branches".into(),
            Pane::Chat => "Chat".into(),
            Pane::Logs => "Logs".into(),
            Pane::Pipeline => "Pipeline".into(),
            Pane::Tools => "Tools".into(),
            Pane::Workflow => "Workflow".into(),
            Pane::Messages => "Lineage".into(),
        }
    }

    fn pane_ui(
        &mut self,
        ui: &mut egui::Ui,
        _tile_id: egui_tiles::TileId,
        pane: &mut Pane,
    ) -> egui_tiles::UiResponse {
        match pane {
            Pane::Settings => {
                self.settings_ui(ui);
            }
            Pane::Navigator => {
                self.nav_ui(ui);
            }
            Pane::Chat => {
                self.chat_ui(ui);
            }
            Pane::Logs => {
                let logs_r = self.log_history.read().unwrap();
                tiles::logview::log_ui(ui, logs_r.as_ref());
            }
            Pane::Pipeline => {
                self.pipeline_ui(ui);
            }
            Pane::Tools => {
                self.toolset_ui(ui);
            }
            Pane::Workflow => {
                self.workflow_ui(ui);
            }
            Pane::Messages => {
                self.message_graph(ui);
            }
        };

        Default::default()
    }
}

pub struct WorkflowState {
    pub frozen: bool,
    pub running: Arc<AtomicBool>,
    pub editing: String,
    pub renaming: Option<String>,
    pub modtime: SystemTime,
    pub switch_count: usize,

    /// Load/save shadow graphs to disk
    pub store: WorkflowStore,

    /// Data for the graph editor widget
    pub snarl: Arc<tokio::sync::RwLock<Snarl<WorkNode>>>,
    pub style: SnarlStyle,

    /// The version of the current shadow graph saved to disk
    pub baseline: ShadowGraph<WorkNode>,

    /// The shadow graph actively being edited
    pub shadow: ShadowGraph<WorkNode>,

    /// Snapshot of graph runner's state
    pub node_state: Arc<ArcSwap<im::OrdMap<NodeId, ExecState>>>,

    /// Undo/redo support
    pub undo_stack: im::OrdMap<String, VecDeque<(SystemTime, ShadowGraph<WorkNode>)>>,
    pub redo_stack: im::OrdMap<String, VecDeque<(SystemTime, ShadowGraph<WorkNode>)>>,
}

impl WorkflowState {
    pub fn from_path(path: impl AsRef<Path>, flow_name: Option<String>) -> anyhow::Result<Self> {
        use egui_snarl::ui::{BackgroundPattern, Grid, NodeLayout, PinPlacement, SnarlStyle};
        let store = WorkflowStore::load(path)?;

        let edit_workflow = flow_name
            .filter(|n| store.workflows.contains_key(n))
            .unwrap_or("default".to_string());

        let snarl = store.get_snarl(edit_workflow.as_ref()).unwrap_or_default();
        let snarl = fixup_workflow(snarl);
        let baseline: ShadowGraph<WorkNode> = store.get(edit_workflow.as_ref()).unwrap_or_default();

        let snarl = Arc::new(tokio::sync::RwLock::new(snarl));

        Ok(Self {
            running: Arc::new(AtomicBool::new(false)),
            editing: edit_workflow.clone(),
            renaming: None,
            store: store.clone(),
            snarl: snarl.clone(),
            style: SnarlStyle {
                crisp_magnified_text: Some(true),
                bg_pattern: Some(BackgroundPattern::Grid(Grid::new(
                    egui::Vec2::new(100.0, 100.0),
                    0.0,
                ))),
                node_frame: SnarlStyle::default()
                    .node_frame
                    .map(|frame| frame.inner_margin(16.0)),
                node_layout: Some(NodeLayout::sandwich()),
                pin_placement: Some(PinPlacement::Edge),
                ..Default::default()
            },
            baseline: baseline.clone(),
            shadow: baseline.clone(),
            modtime: SystemTime::now(),
            switch_count: Default::default(),
            frozen: Default::default(),
            node_state: Default::default(),
            undo_stack: Default::default(),
            redo_stack: Default::default(),
        })
    }

    pub fn has_changes(&self) -> bool {
        !self.shadow.fast_eq(&self.baseline)
    }

    pub fn switch(&mut self, workflow_name: &str) {
        if self.editing.as_str() == workflow_name {
            return;
        }

        let snarl = self.store.get_snarl(workflow_name).unwrap_or_default();
        let snarl = fixup_workflow(snarl);
        self.frozen = false;
        self.snarl = Arc::new(tokio::sync::RwLock::new(snarl));
        self.editing = workflow_name.to_string();
        self.renaming = None;
        self.baseline = self.store.get(workflow_name).unwrap_or_default();
        self.shadow = self.baseline.clone();
        self.modtime = SystemTime::now();
        self.switch_count += 1;
    }

    pub fn rename(&mut self) {
        if Some(&self.editing) == self.renaming.as_ref() {
            self.renaming = None;
        }

        let Some(new_name) = self.renaming.take() else {
            return;
        };

        if self.store.workflows.contains_key(&new_name) {
            return;
        }

        self.store.rename(&self.editing, &new_name);
        self.editing = new_name;
    }

    pub fn names(&self) -> impl Iterator<Item = &String> {
        self.store.workflows.keys()
    }

    pub fn remove(&mut self) {
        self.store.remove(&self.editing);
    }

    pub fn cast_shadow(&mut self, shadow: ShadowGraph<WorkNode>) {
        if self.frozen || self.shadow.fast_eq(&shadow) {
            return;
        }

        let undo_stack = self.undo_stack.entry(self.editing.clone()).or_default();

        // Initialize with baseline
        // if undo_stack.is_empty() {
        //     undo_stack.push_front((self.modtime, self.baseline.clone()));
        // }
        //
        let last_undo = undo_stack.front();
        let dur = last_undo
            .and_then(|(t, _)| t.elapsed().ok())
            .unwrap_or(Duration::MAX);

        // Debounce if still editing a second ago
        if dur < Duration::from_secs(1) {
            undo_stack.pop_front();
        }

        undo_stack.push_front((self.modtime, self.shadow.clone()));

        undo_stack.truncate(256);
        self.redo_stack.remove(&self.editing);

        self.shadow = shadow;
        self.modtime = SystemTime::now();
        tracing::trace!("Updating shadow. stack {}", undo_stack.len());
    }

    pub fn get_undo_stack(&mut self) -> &mut VecDeque<(SystemTime, ShadowGraph<WorkNode>)> {
        self.undo_stack.entry(self.editing.clone()).or_default()
    }

    pub fn get_redo_stack(&mut self) -> &mut VecDeque<(SystemTime, ShadowGraph<WorkNode>)> {
        self.redo_stack.entry(self.editing.clone()).or_default()
    }

    pub fn undo(&mut self) {
        let undo_stack = self.undo_stack.entry(self.editing.clone()).or_default();
        let redo_stack = self.redo_stack.entry(self.editing.clone()).or_default();
        tracing::debug!(
            "Undoing. undos={} redos={}",
            undo_stack.len(),
            redo_stack.len()
        );

        if let Some((mut modtime, mut shadow)) = undo_stack.pop_front() {
            redo_stack.push_front((self.modtime, self.shadow.clone()));

            // Fast forward over duplicates
            while !undo_stack.is_empty() && self.shadow == shadow {
                if let Some((mt, sg)) = undo_stack.pop_front() {
                    modtime = mt;
                    shadow = sg;
                }
            }

            self.snarl = Arc::new(tokio::sync::RwLock::new(
                Snarl::try_from(shadow.clone()).unwrap_or_default(),
            ));
            self.shadow = shadow.clone();
            self.modtime = modtime;
            self.switch_count += 1;
            self.frozen = true;
        }
        tracing::debug!(
            "Undid. undos={} redos={}",
            undo_stack.len(),
            redo_stack.len()
        );
    }
    pub fn redo(&mut self) {
        let redo_stack = self.redo_stack.entry(self.editing.clone()).or_default();
        let undo_stack = self.undo_stack.entry(self.editing.clone()).or_default();
        tracing::debug!(
            "Redoing. undos={} redos={}",
            undo_stack.len(),
            redo_stack.len()
        );

        if let Some((ts, shadow)) = redo_stack.pop_front() {
            undo_stack.push_front((self.modtime, self.shadow.clone()));
            self.snarl = Arc::new(tokio::sync::RwLock::new(
                Snarl::try_from(shadow.clone()).unwrap_or_default(),
            ));

            self.shadow = shadow.clone();
            self.modtime = ts;
            self.switch_count += 1;
            self.frozen = true;
        }
        tracing::debug!(
            "Redid. undos={} redos={}",
            undo_stack.len(),
            redo_stack.len()
        );
    }
}
