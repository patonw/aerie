use arc_swap::ArcSwap;
use eframe::egui;
use egui::WidgetText;
use egui_commonmark::*;
use egui_snarl::{NodeId, Snarl, ui::SnarlStyle};
use egui_tiles::SimplificationOptions;
use itertools::Itertools;
use rmcp::model::Tool;
use std::{
    borrow::Cow,
    collections::VecDeque,
    fs::OpenOptions,
    path::Path,
    sync::{
        Arc, RwLock,
        atomic::{AtomicBool, AtomicU16},
    },
    time::{Duration, SystemTime},
};
use typed_builder::TypedBuilder;
use uuid::Uuid;

use super::Pane;
use crate::{
    AgentFactory, LogEntry, Settings, ToolSpec,
    chat::ChatSession,
    transmute::Transmuter,
    ui::tiles::messages::MessageGraph,
    utils::ErrorList,
    workflow::{
        ShadowGraph, WorkNode, fixup_workflow,
        runner::{ExecState, WorkflowRun},
        store::{WorkflowStore, WorkflowStoreDir},
    },
};

const SOFT_LIMIT: usize = 128;

pub enum ToolEditorState {
    EditProvider {
        original: Option<(String, ToolSpec)>,
        modified: (String, ToolSpec),
    },
    ViewTool {
        tool: Tool,
    },
}

#[derive(TypedBuilder)]
pub struct AppState {
    pub rt: tokio::runtime::Handle,

    #[builder(default)]
    pub errors: ErrorList<anyhow::Error>,

    pub settings: Arc<RwLock<Settings>>,

    #[builder(default)]
    pub task_count: Arc<AtomicU16>,

    #[builder(default)]
    pub log_history: Arc<arc_swap::ArcSwapAny<Arc<im::Vector<LogEntry>>>>,

    pub session: ChatSession,

    pub cache: CommonMarkCache,

    #[builder(default)]
    pub prompt: String,

    pub agent_factory: AgentFactory,

    #[builder(default)]
    pub rename_session: Option<String>,

    // TODO: decompose
    #[builder(default)]
    pub branch_point: Option<Uuid>,

    #[builder(default)]
    pub new_branch: String,

    #[builder(default)]
    pub rename_branch: Option<String>,

    #[builder(default)]
    pub tool_editor: Option<ToolEditorState>,

    pub workflows: WorkflowState<WorkflowStoreDir>,

    #[builder(default)]
    pub message_graph: MessageGraph,

    #[builder(default)]
    pub transmuter: Transmuter,
}

impl egui_tiles::Behavior<Pane> for AppState {
    fn tab_title_for_pane(&mut self, pane: &Pane) -> WidgetText {
        match pane {
            Pane::Settings => "Settings".into(),
            Pane::Navigator => "Session".into(),
            Pane::Chat => "Chat".into(),
            Pane::Logs => "Logs".into(),
            Pane::Tools => "Tools".into(),
            Pane::Workflow => "Workflow".into(),
            Pane::Messages => "Lineage".into(),
            Pane::Outputs => "Outputs".into(),
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
                self.logview_ui(ui);
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
            Pane::Outputs => {
                self.outputs_ui(ui);
            }
        };

        Default::default()
    }

    fn simplification_options(&self) -> SimplificationOptions {
        SimplificationOptions {
            all_panes_must_have_tabs: true,
            ..Default::default()
        }
    }
}

/// Portion of the UI state dealing with workflows.
pub struct WorkflowState<W: WorkflowStore> {
    pub frozen: bool,
    pub running: Arc<AtomicBool>,
    pub interrupt: Arc<AtomicBool>,
    pub editing: String,
    pub renaming: Option<String>,
    pub modtime: SystemTime,
    pub switch_count: usize,

    /// Load/save shadow graphs to disk
    pub store: W,

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

    pub outputs: im::Vector<WorkflowRun>,
}

impl<W: WorkflowStore> WorkflowState<W> {
    pub fn new(store: W, current: Option<String>) -> Self {
        use egui_snarl::ui::{BackgroundPattern, Grid, NodeLayout, PinPlacement, SnarlStyle};
        let edit_workflow = current
            .filter(|n| store.exists(n))
            .unwrap_or("basic".to_string());

        let baseline: ShadowGraph<WorkNode> = store.get(edit_workflow.as_ref()).unwrap_or_default();
        let snarl = Snarl::try_from(baseline.clone()).unwrap_or_default();
        let snarl = fixup_workflow(snarl);

        let snarl = Arc::new(tokio::sync::RwLock::new(snarl));

        Self {
            frozen: false,
            running: Arc::new(AtomicBool::new(false)),
            interrupt: Arc::new(AtomicBool::new(false)),
            editing: edit_workflow.clone(),
            renaming: None,
            store,
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
            node_state: Default::default(),
            undo_stack: Default::default(),
            redo_stack: Default::default(),
            outputs: Default::default(),
        }
    }

    pub fn has_changes(&self) -> bool {
        !self.shadow.fast_eq(&self.baseline)
    }

    pub fn switch(&mut self, workflow_name: &str) {
        if self.editing.as_str() == workflow_name {
            return;
        }

        // Stash current editee if changes unsaved
        if !self.shadow.fast_eq(&self.baseline) {
            self.undo_stack
                .entry(self.editing.clone())
                .or_default()
                .push_front((self.modtime, self.shadow.clone()));
        }

        self.baseline = self.store.get(workflow_name).unwrap_or_default();
        if let Some(undos) = self.undo_stack.get_mut(workflow_name)
            && let Some((mt, sg)) = undos.pop_front()
        {
            self.shadow = sg;
            self.modtime = mt;
        } else {
            self.shadow = self.baseline.clone();
            self.modtime = SystemTime::now();
        }

        let snarl = Snarl::try_from(self.shadow.clone()).unwrap_or_default();
        let snarl = fixup_workflow(snarl);
        self.frozen = false;
        self.snarl = Arc::new(tokio::sync::RwLock::new(snarl));
        self.editing = workflow_name.to_string();
        self.renaming = None;
        self.switch_count += 1;
    }

    pub fn rename(&mut self) -> anyhow::Result<()> {
        let new_name = self.renaming.take();

        if Some(&self.editing) == new_name.as_ref() {
            return Ok(());
        }

        let Some(new_name) = new_name else {
            return Ok(());
        };

        // Prevent traversal shenanigans without being too strict
        let new_name = Path::new(&new_name)
            .file_name()
            .ok_or(anyhow::anyhow!("Invalid name"))?
            .display()
            .to_string()
            .trim_matches('.')
            .trim_matches('_')
            .to_string();

        if self.store.exists(&new_name) {
            anyhow::bail!("Workflow {new_name} exists!");
        }

        if self.store.exists(&self.editing) {
            self.store.rename(&self.editing, &new_name)?;
        }

        self.editing = new_name;
        Ok(())
    }

    pub fn names(&self) -> impl Iterator<Item = Cow<'_, str>> {
        self.store.names()
    }

    pub fn description(&self, name: &str) -> Cow<'_, str> {
        self.store.description(name)
    }

    pub fn remove(&mut self) -> anyhow::Result<()> {
        self.store.remove(&self.editing)
    }

    pub fn cast_shadow(&mut self, shadow: ShadowGraph<WorkNode>) {
        if self.frozen || self.shadow.fast_eq(&shadow) {
            return;
        }

        if !self.undo_stack.contains_key(&self.editing)
            && let Err(err) = self.store.backup(&self.editing)
        {
            tracing::warn!("Error while backing up {}: {err:?}", &self.editing);
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

        if undo_stack.len() > SOFT_LIMIT {
            tracing::info!(
                "Pruning undo stack for {} ({}). {:?}",
                &self.editing,
                undo_stack.len(),
                undo_stack.iter().map(|it| it.0).collect_vec()
            );
            for i in 1..=(SOFT_LIMIT / 2) {
                undo_stack.swap(i, i * 2);
            }

            undo_stack.truncate(SOFT_LIMIT / 2 + 1);
            tracing::info!(
                "Finished pruning undo stack for {} ({}). {:?}",
                &self.editing,
                undo_stack.len(),
                undo_stack.iter().map(|it| it.0).collect_vec()
            );
        }

        self.redo_stack.remove(&self.editing);

        self.shadow = shadow;
        self.modtime = SystemTime::now();
        tracing::trace!("Updating shadow. stack {}", undo_stack.len());
    }

    pub fn get_undo_count(&mut self) -> usize {
        self.undo_stack
            .get(&self.editing)
            .map(|s| s.len())
            .unwrap_or_default()
    }

    pub fn get_redo_count(&mut self) -> usize {
        self.redo_stack
            .get(&self.editing)
            .map(|s| s.len())
            .unwrap_or_default()
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

    pub fn export(&mut self, path: &Path) -> anyhow::Result<()> {
        let writer = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)?;

        serde_yml::to_writer(writer, &self.shadow)?;
        Ok(())
    }

    pub fn import(&mut self, path: &Path) -> anyhow::Result<()> {
        if !path.is_file() {
            anyhow::bail!("Invalid file: {path:?}");
        }

        let name = path
            .file_stem()
            .and_then(|s| s.to_os_string().into_string().ok())
            .unwrap_or_default();

        let datetime = chrono::offset::Local::now();
        let timestamp = datetime.format("%Y-%m-%dT%H:%M:%S").to_string();
        let name = if name.is_empty() || self.names().contains(name.as_str()) {
            std::iter::chain([name], [timestamp]).join("-")
        } else {
            name
        };

        let reader = OpenOptions::new().read(true).open(path)?;
        let data: ShadowGraph<WorkNode> = serde_yml::from_reader(reader)?;

        self.store.save(&name, data)?;
        // self.store.save()?; // maybe don't?
        self.switch(&name);
        self.baseline = Default::default();

        Ok(())
    }
}
