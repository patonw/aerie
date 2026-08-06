use arc_swap::ArcSwap;
use eframe::egui;
use egui::WidgetText;
use egui_commonmark::*;
use egui_snarl::{InPinId, NodeId};
use egui_tiles::SimplificationOptions;
use itertools::Itertools;
use rmcp::model::Tool;
use serde::{Deserialize, Serialize};
use serde_yaml_ng as serde_yml;
use std::{
    borrow::Cow,
    collections::VecDeque,
    fs::OpenOptions,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU16},
    },
};
use typed_builder::TypedBuilder;
use uuid::Uuid;
use web_time::{Duration, SystemTime};

#[cfg(not(feature = "native"))]
use crate::config::ConfigStateProxy;
#[cfg(feature = "native")]
use crate::config::ConfigStateStore;
use crate::workflow::CheckContext;
use crate::workflow::ShadowGraph;
use crate::workflow::WorkNode;
#[cfg(feature = "native")]
use crate::workflow::store::{WorkflowStore, WorkflowStoreDir};
#[cfg(not(feature = "native"))]
use crate::workflow::store::{WorkflowStore, WorkflowStoreStub};
#[cfg(feature = "wip")]
use crate::{agent::AgentFactory, ui::tiles::messages::MessageGraph};
use crate::{
    toolbox::Toolbox,
    ui::{
        Pane,
        workflow::{ProgressEntry, ViewStack, WorkflowViewer},
    },
};

use crate::workflow::{EditContext, nodes::LoopControl};
use crate::{
    chat::ChatSession,
    config::{ConfigExt as _, Preferences, ToolSpec},
    events::{AppEvent, AppEvents},
    logging::LogEntry,
    toolbox::ToolStore,
    transmute::Transmuter,
    ui::ShowHelp,
    utils::{ErrorDistiller as _, ErrorList},
    workflow::{
        GraphId, PreviewData, Workflow,
        runner::{ExecId, NodeStateMap, WorkflowRun},
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

/// Data synced between client and server
#[derive(Clone, Serialize, Deserialize, Default, TypedBuilder)]
pub struct AppData {
    #[builder(default)]
    pub prefs: Arc<ArcSwap<Preferences>>,

    #[builder(default)]
    pub session: ChatSession,

    #[builder(default)]
    pub session_names: Vec<String>,
    // #[builder(default)]
    // pub show_help: Option<ShowHelp>,
    //
    // #[builder(default)]
    // pub events: Arc<AppEvents>,
    //
    // #[builder(default)]
    // pub errors: ErrorList<anyhow::Error>,
    //
    // #[builder(default)]
    // pub prompt: String,
    //
    // #[builder(default)]
    // pub images: Vec<String>,
    //
    // #[builder(default)]
    // pub image_input: Option<String>,
    //
    // #[builder(default)]
    // pub run_count: usize,
}

// TODO: Lift business-logic classes to top-level and leave this as pure data
// TODO: use AppActions to trigger server-side handlers which update state
#[derive(TypedBuilder)]
pub struct AppState {
    #[builder(default)]
    pub data: AppData,

    #[cfg(feature = "native")]
    pub rt: tokio::runtime::Handle,

    // TODO: migrate
    #[builder(default)]
    pub show_help: Option<ShowHelp>,

    // TODO: migrate
    #[builder(default)]
    pub events: Arc<AppEvents>,

    // TODO: migrate
    #[builder(default)]
    pub errors: ErrorList<anyhow::Error>,

    #[cfg(feature = "native")]
    pub state: ConfigStateStore,

    #[cfg(not(feature = "native"))]
    pub state: ConfigStateProxy,

    #[builder(default)]
    pub tools: ToolStore,

    #[builder(default)]
    pub task_count: Arc<AtomicU16>,

    #[builder(default)]
    pub log_history: Arc<arc_swap::ArcSwapAny<Arc<im::Vector<LogEntry>>>>,

    #[builder(default)]
    pub cache: CommonMarkCache,

    // TODO: migrate
    #[builder(default)]
    pub prompt: String,

    // TODO: migrate
    #[builder(default)]
    pub images: Vec<String>,

    // TODO: migrate
    #[builder(default)]
    pub image_input: Option<String>,

    // TODO: migrate
    #[builder(default)]
    pub run_count: usize,

    #[cfg(feature = "native")]
    pub agent_factory: AgentFactory,

    #[builder(default)]
    pub rename_session: Option<String>,

    #[builder(default)]
    pub rename_profile: Option<String>,

    // TODO: decompose
    #[builder(default)]
    pub branch_point: Option<Uuid>,

    #[builder(default)]
    pub new_branch: String,

    #[builder(default)]
    pub rename_branch: Option<String>,

    #[builder(default)]
    pub tool_editor: Option<ToolEditorState>,

    #[cfg(feature = "native")]
    pub workflows: WorkflowState<WorkflowStoreDir>,

    #[cfg(not(feature = "native"))]
    pub workflows: WorkflowState<WorkflowStoreStub>,

    #[cfg(feature = "wip")]
    #[builder(default)]
    pub message_graph: MessageGraph,

    #[builder(default)]
    pub transmuter: Transmuter,
}

impl AppState {
    #[cfg(feature = "native")]
    fn get_toolbox(&self) -> Toolbox {
        self.agent_factory.toolbox.clone()
    }
    #[cfg(not(feature = "native"))]
    fn get_toolbox(&self) -> Toolbox {
        Default::default()
    }

    pub fn workflow_viewer(&mut self) -> &mut WorkflowViewer {
        if self.workflows.viewer.is_none() {
            let stack = &self.workflows.view_stack;
            let shadow = stack.leaf().clone();

            let edit_ctx = EditContext::builder()
                .toolbox(self.get_toolbox())
                .events(self.events.clone())
                .current_graph(shadow.uuid)
                .metadata(self.workflows.shadow.metadata.clone())
                .parent_id(stack.parent_id())
                .flavor(stack.flavor())
                .errors(self.errors.clone())
                .previews(self.workflows.previews.clone())
                .build();

            let viewer = WorkflowViewer::builder()
                .edit_ctx(edit_ctx)
                .shadow(shadow)
                .node_state(self.workflows.node_state.clone())
                .view_id(stack.view_id().with(self.workflows.switch_count))
                .events(self.events.clone())
                .build();

            tracing::debug!("Changing view to graph {:?}", &viewer.shadow.uuid,);

            self.workflows.viewer = Some(viewer);
        }

        let viewer = self.workflows.viewer.as_mut().unwrap();
        viewer.alerts = self.workflows.alerts.clone();
        viewer.frozen = self.workflows.frozen;
        viewer.edit_ctx.exec_id = self.workflows.view_stack.exec_id();
        viewer.running = self
            .workflows
            .running
            .load(std::sync::atomic::Ordering::Relaxed);

        viewer
    }

    #[cfg(feature = "native")]
    pub fn handle_events(&mut self) {
        use AppEvent::*;

        // Only allow one run request per frame
        let mut executed = false;
        let errors = self.errors.clone();
        let settings = self.data.prefs.clone();

        while let Some(event) = self.events.pop() {
            let mut handled = false;

            handled = handled || self.workflows.handle_event(&event);

            handled = handled
                || match &event {
                    CreateWorkflow => {
                        let datetime = chrono::offset::Local::now();
                        let timestamp = datetime.format("unnamed-%Y-%m-%dT%H:%M:%S").to_string();
                        self.workflows.store.put(&timestamp, Default::default());
                        self.workflows.switch(&timestamp);
                        true
                    }
                    DeleteWorkflow(name) => {
                        self.workflows.editing = name.clone();
                        errors.distil(self.workflows.remove());
                        let next_name = self
                            .workflows
                            .names()
                            .next()
                            .map(|s| s.to_string())
                            .unwrap_or("default".to_string());
                        self.workflows.switch(&next_name);
                        true
                    }
                    SwitchWorkflow(name) => {
                        if settings.view(|s| s.autosave) {
                            // TODO:
                            // self.workflows.save();
                        }
                        self.workflows.switch(name);
                        true
                    }
                    RenameWorkflow(old_name, new_name) => {
                        self.workflows.editing = old_name.clone();
                        self.workflows.renaming = Some(new_name.clone());
                        errors.distil(self.workflows.rename());
                        true
                    }
                    UserRunWorkflow if !executed => {
                        self.run_count = 0;
                        self.workflows.node_state.clear();
                        self.exec_workflow();
                        executed = true;
                        true
                    }
                    NodesChanged(graph_id, nodes) => {
                        self.workflows.reset_nodes(
                            *graph_id,
                            nodes.clone(),
                            self.data.prefs.view(|s| s.cascade),
                        );
                        true
                    }
                    RerunNodes(graph_id, nodes) if !executed => {
                        self.workflows.reset_nodes(
                            *graph_id,
                            nodes.into(),
                            self.data.prefs.view(|s| s.cascade),
                        );
                        self.exec_workflow();
                        executed = true;
                        true
                    }
                    SetPrompt(prompt) => {
                        self.prompt = prompt.clone();
                        if prompt.is_empty() {
                            self.images.clear();
                        }
                        true
                    }
                    UpdatePreferences(prefs) => {
                        self.data.prefs.store(prefs.clone());
                        true
                    }
                    ClearModels => {
                        let _ = self.state.update(|state| {
                            state.prev_models.clear();
                        });

                        true
                    }
                    TouchModel(name) => {
                        let _ = self.state.update(|state| {
                            state.prev_models.retain(|m| m != name);
                            state.prev_models.push_front(name.clone());
                            state.prev_models =
                                state.prev_models.take(16.min(state.prev_models.len()));
                            // if let Some(i) = state.prev_models.index_of(llm_model) {
                            //     state.prev_models.remove(i);
                            // }
                            // state.prev_models.push_front(llm_model.clone());
                        });

                        true
                    }
                    CreateSession => {
                        let datetime = chrono::offset::Local::now();
                        let timestamp = datetime.format("unnamed-%Y-%m-%dT%H:%M:%S").to_string();
                        errors.distil(self.data.session.switch(&timestamp));
                        errors.distil(self.data.session.save());
                        self.state.set_session(self.data.session.name());
                        self.data.session_names = self.data.session.list();
                        true
                    }
                    SwitchSession(session_name) => {
                        errors.distil(self.data.session.switch(session_name));
                        self.state.set_session(self.data.session.name());
                        self.data.session_names = self.data.session.list();
                        true
                    }
                    RenameSession(_old_name, new_name) => {
                        errors.distil(self.data.session.rename(new_name));
                        self.state.set_session(new_name);
                        self.data.session_names = self.data.session.list();
                        true
                    }
                    _ => false,
                };

            if !handled {
                tracing::warn!("Unhandled event {event:?}");
            }
        }

        let shadow = self.workflows.view_stack.root();
        self.workflows.cast_shadow(shadow);
    }
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
            #[cfg(feature = "wip")]
            Pane::Logs => {
                self.logview_ui(ui);
            }
            #[cfg(feature = "wip")]
            Pane::Tools => {
                self.toolset_ui(ui);
            }
            Pane::Workflow => {
                if self.workflows.view_stack.is_empty() {
                    self.workflow_ui(ui);
                } else {
                    self.subgraph_ui(ui);
                }
            }
            #[cfg(feature = "wip")]
            Pane::Messages => {
                self.message_graph(ui);
            }
            Pane::Outputs => {
                self.outputs_ui(ui);
            }
            #[cfg(not(feature = "wip"))]
            _ => {}
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum MetaEdit {
    #[default]
    Description,
    Schema,
    Chain,
}

/// Portion of the UI state dealing with workflows.
pub struct WorkflowState<W: WorkflowStore> {
    pub view_stack: ViewStack,
    pub viewer: Option<WorkflowViewer>,

    pub frozen: bool,
    pub running: Arc<AtomicBool>,
    pub interrupt: Arc<AtomicBool>,
    pub editing: String,
    pub meta_edit: MetaEdit,
    pub renaming: Option<String>,
    pub modtime: SystemTime,
    pub switch_count: usize,

    /// Load/save shadow graphs to disk
    pub store: W,

    /// The version of the current shadow graph saved to disk
    pub baseline: Workflow,

    /// The shadow graph actively being edited
    pub shadow: Workflow,

    /// Snapshot of graph runner's state
    pub node_state: NodeStateMap,

    /// Undo/redo support
    pub undo_stack: im::OrdMap<String, VecDeque<(SystemTime, Workflow)>>,
    pub redo_stack: im::OrdMap<String, VecDeque<(SystemTime, Workflow)>>,

    pub previews: PreviewData,
    pub outputs: im::Vector<WorkflowRun>,

    pub graph_checked: bool,
    pub alerts: im::OrdMap<(GraphId, NodeId), Cow<'static, str>>,
}

impl<W: WorkflowStore> WorkflowState<W> {
    pub fn new(store: W, current: Option<String>) -> Self {
        let (name, graph) = if let Some(name) = current.as_ref() {
            if !store.exists(name) {
                tracing::warn!("No such workflow {name}");
                Default::default()
            } else {
                match store.load(name.as_ref()) {
                    Ok(baseline) => (name.clone(), baseline),
                    Err(e) => {
                        tracing::error!("Could not load workflow {name}: {e:}");
                        Default::default()
                    }
                }
            }
        } else {
            Default::default()
        };

        tracing::info!("Workflow requested: {:?}, resolved: {}", &current, &name);

        let view_stack = ViewStack::from_root(&name, graph.clone());

        Self {
            view_stack,
            viewer: None,
            frozen: false,
            running: Arc::new(AtomicBool::new(false)),
            interrupt: Arc::new(AtomicBool::new(false)),
            editing: name.clone(),
            meta_edit: Default::default(),
            renaming: None,
            store,
            baseline: graph.clone(),
            shadow: graph.clone(),
            modtime: SystemTime::now(),
            switch_count: Default::default(),
            node_state: Default::default(),
            undo_stack: Default::default(),
            redo_stack: Default::default(),
            previews: Default::default(),
            outputs: Default::default(),
            graph_checked: false,
            alerts: Default::default(),
        }
    }

    pub fn has_changes(&self) -> bool {
        // if cfg!(any(feature = "web-ui", feature = "backend")) {
        //     if self.shadow.fast_eq(&self.baseline) {
        //         false
        //     } else {
        //         !self.shadow.eq(&self.baseline)
        //     }
        // } else {
        !self.shadow.fast_eq(&self.baseline)
        // }
    }

    /// Runs recursive checks on workflow graphs after changes
    pub fn check_graph(&mut self) {
        if !self.graph_checked
            && let Some(viewer) = &self.viewer
        {
            let ctx = CheckContext::builder()
                .toolbox(viewer.edit_ctx.toolbox.clone())
                .graph_id(self.shadow.graph.uuid)
                .build();

            self.alerts = self.shadow.graph.check(&ctx);
            self.graph_checked = true;
        }
    }

    pub fn patch(&mut self, patch: crate::client::EditorPatch) {
        if self.baseline != patch.baseline {
            if self.shadow == patch.baseline {
                self.baseline = self.shadow.clone();
            } else {
                self.baseline = patch.baseline.clone();
            }
        }

        // if let Some(shadow) = patch.shadow {
        //     if self.shadow != shadow {
        //         self.shadow = shadow;
        //     }
        // } else {
        //     self.shadow = self.baseline.clone();
        // }

        if self.editing != patch.name {
            log::info!(
                "Patch changed name from {} to {}",
                &self.editing,
                &patch.name
            );

            self.switch_workflow(&patch.name, patch.baseline);
        }
    }

    pub fn switch(&mut self, workflow_name: &str) {
        if self.editing.as_str() == workflow_name {
            return;
        }

        let Some(target) = self.store.get(workflow_name) else {
            tracing::warn!("Could not load workflow {workflow_name}");
            if let Some(viewer) = &self.viewer {
                viewer
                    .edit_ctx
                    .errors
                    .push(anyhow::anyhow!("Could not load workflow {workflow_name}"));
            }

            return;
        };

        self.switch_workflow(workflow_name, target);
    }

    pub fn switch_workflow(&mut self, workflow_name: &str, target: Workflow) {
        // Stash current editee to preserve unsaved changes
        self.undo_stack
            .entry(self.editing.clone())
            .or_default()
            .push_front((self.modtime, self.shadow.clone()));

        self.baseline = target;
        if let Some(undos) = self.undo_stack.get_mut(workflow_name)
            && let Some((mt, sg)) = undos.pop_front()
        {
            // Unstash any workflows we were previously editing
            self.shadow = sg;
            self.modtime = mt;
        } else {
            self.shadow = self.baseline.clone();
            self.modtime = SystemTime::now();
        }

        self.frozen = false;
        self.editing = workflow_name.to_string();
        self.renaming = None;
        self.switch_count += 1;
        self.view_stack = ViewStack::from_root(workflow_name, self.shadow.clone());
        self.viewer = None;
        self.graph_checked = false;
    }

    #[cfg(feature = "wip")]
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

    #[cfg(feature = "wip")]
    pub fn remove(&mut self) -> anyhow::Result<()> {
        self.store.remove(&self.editing)
    }

    pub fn cast_shadow(&mut self, shadow: ShadowGraph<WorkNode>) {
        if self.frozen || self.shadow.graph.fast_eq(&shadow) {
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

        if let Some(viewer) = &self.viewer {
            // TODO: also compute changes for subgraph viewer
            let edited = shadow
                .nodes
                .iter()
                .filter(|(id, meta)| {
                    self.shadow
                        .graph
                        .nodes
                        .get(id)
                        .map(|other| !meta.value.weak_eq(&other.value))
                        .unwrap_or_default()
                })
                .map(|(id, _)| *id)
                .collect::<im::OrdSet<_>>();

            let connected = shadow
                .wires
                .clone()
                .difference(self.shadow.graph.wires.clone())
                .iter()
                .map(|w| w.in_pin.node)
                .collect::<im::OrdSet<NodeId>>();

            viewer
                .events
                .insert(AppEvent::GraphSnapshot(shadow.clone()));
            viewer.events.insert(AppEvent::NodesChanged(
                self.shadow.graph.uuid,
                edited.union(connected),
            ));
        }

        self.shadow = Workflow {
            graph: Arc::new(shadow),
            metadata: self.shadow.metadata.clone(),
        };

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

            self.shadow = shadow.clone();
            self.modtime = modtime;
            self.switch_count += 1;
            self.view_stack.switch(&self.editing, shadow.clone());
            self.viewer = None;
            self.frozen = true;
            self.graph_checked = false;
        }
        tracing::debug!(
            "Undid. undos={} redos={}, path={:?}",
            undo_stack.len(),
            redo_stack.len(),
            &self.view_stack.path
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
            self.shadow = shadow.clone();
            self.modtime = ts;
            self.switch_count += 1;
            self.view_stack.switch(&self.editing, shadow.clone());
            self.frozen = true;
            self.graph_checked = false;
        }
        tracing::debug!(
            "Redid. undos={} redos={}",
            undo_stack.len(),
            redo_stack.len()
        );
    }

    #[cfg(feature = "wip")]
    pub fn export(&mut self, path: &Path) -> anyhow::Result<()> {
        let writer = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)?;

        serde_yml::to_writer(writer, &self.shadow.repair())?;
        Ok(())
    }

    #[cfg(feature = "wip")]
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
        let data: Workflow = serde_yml::from_reader(reader)?;

        self.undo_stack
            .entry(name.clone())
            .or_default()
            .push_front((self.modtime, data.clone()));

        self.store.save(&name, data)?;
        self.switch(&name);
        self.baseline = Default::default();

        Ok(())
    }

    #[cfg(feature = "wip")]
    pub fn save(&mut self) {
        tracing::info!(
            "Saving {} to workflows...changed? {}",
            &self.editing,
            !self.shadow.fast_eq(&self.baseline)
        );

        self.store.save(&self.editing, self.shadow.clone()).unwrap();

        self.baseline = self.shadow.clone();
    }

    pub fn reset_nodes(&self, graph_id: GraphId, mut nodes: im::OrdSet<NodeId>, cascade: bool) {
        if nodes.is_empty() {
            return;
        }

        if graph_id != self.shadow.graph.uuid {
            todo!("Not implemented for subgraphs yet");
        }

        let graph = self.shadow.graph.clone();
        if cascade {
            tracing::debug!("Cascading nodes {nodes:?}");

            let mut frontier = nodes.clone();
            while !frontier.is_empty() {
                let expanded = graph
                    .wires
                    .iter()
                    .filter(|w| frontier.contains(&w.out_pin.node))
                    .map(|w| w.in_pin.node)
                    .collect();

                let expanded = nodes.clone().union(expanded);

                frontier = expanded.clone().difference(nodes);
                nodes = expanded;
            }

            tracing::debug!("Expansion: {nodes:?}");
        }

        let exec_id = ExecId::from(graph_id);
        let view = self.node_state.view(exec_id);
        for node in nodes {
            view.remove(node);
        }
    }

    // TODO: collect errors
    #[cfg(feature = "wip")]
    pub fn handle_event(&mut self, event: &AppEvent) -> bool {
        use AppEvent::*;
        match event {
            GraphSnapshot(graph) => {
                self.view_stack.switch_graph(graph);
                true
            }
            WorkflowSnapshot(data) => {
                self.shadow = data.clone();
                true
            }
            SaveWorkflow => {
                self.save();
                true
            }
            Freeze(Some(value)) => {
                self.frozen = *value;
                true
            }
            Freeze(None) => {
                self.frozen = !self.frozen;
                true
            }
            Undo => {
                self.undo();
                true
            }
            Redo => {
                self.redo();
                true
            }
            EnterSubgraph(node_id) => {
                self.view_stack.enter(*node_id).unwrap();
                self.viewer = None;
                true
            }
            LeaveSubgraph(levels) => {
                if !self.view_stack.is_empty() {
                    self.view_stack.exit(*levels).unwrap();
                    self.viewer = None;
                }
                true
            }
            DisableNode(graph_id, node_id) => {
                let _ = self.view_stack.propagate(self.view_stack.leaf(), |graph| {
                    if graph.uuid == *graph_id {
                        graph.disable_node(*node_id)
                    } else {
                        graph
                    }
                });
                true
            }
            // There must be a better way to synchronize the loop controller and start nodes
            OutPinCreated(graph_id, node_id, _kind) => {
                let _ = self
                    .view_stack
                    .propagate(self.view_stack.leaf(), |mut graph| {
                        if graph.uuid == *graph_id {
                            if graph.start == Some(*node_id)
                                && let Some(loop_id) = graph.looper
                            {
                                let fields = graph.start_node().map(|n| n.fields.clone()).unwrap();
                                let meta_loop = graph.nodes.get_mut(&loop_id).unwrap();
                                let looper = meta_loop.value.as_node_mut::<LoopControl>().unwrap();
                                looper.fields = fields;
                            }
                            graph
                        } else {
                            graph
                        }
                    });

                true
            }
            PinRenamed(graph_id, pin, name) => {
                use crate::workflow::AnyPin;
                tracing::debug!("Handling pin event {graph_id:?} {pin:?}");

                let _ = self
                    .view_stack
                    .propagate(self.view_stack.leaf(), |mut graph| {
                        if graph.uuid == *graph_id {
                            if let AnyPin::Out(pin) = pin
                                && graph.start == Some(pin.node)
                                && let Some(loop_id) = graph.looper
                            {
                                let meta_loop = graph.nodes.get_mut(&loop_id).unwrap();
                                let looper = meta_loop.value.as_node_mut::<LoopControl>().unwrap();
                                looper.fields[pin.output].0 = name.clone();
                            }
                            graph
                        } else {
                            graph
                        }
                    });

                true
            }
            PinRemoved(graph_id, pin) => {
                // At this point, the node already considers the pin gone,
                // but we need to update the wires to reflect that.
                use crate::workflow::AnyPin::*;
                tracing::debug!("Handling pin event {graph_id:?} {pin:?}");

                let _ = self
                    .view_stack
                    .propagate(self.view_stack.leaf(), |mut graph| {
                        if graph.uuid == *graph_id {
                            match pin {
                                Out(pin) => {
                                    // This is pretty terrible encapsulation
                                    if graph.start == Some(pin.node)
                                        && let Some(loop_id) = graph.looper
                                    {
                                        let meta_loop = graph.nodes.get_mut(&loop_id).unwrap();
                                        let looper =
                                            meta_loop.value.as_node_mut::<LoopControl>().unwrap();

                                        looper.fields.remove(pin.output);

                                        graph
                                            .shift_inputs(InPinId {
                                                node: loop_id,
                                                input: pin.output,
                                            })
                                            .shift_outputs(*pin)
                                    } else {
                                        graph.shift_outputs(*pin)
                                    }
                                }
                                In(pin) => graph.shift_inputs(*pin),
                            }
                        } else {
                            graph
                        }
                    });

                true
            }
            SwapInputs(graph_id, a, b) => {
                let _ = self.view_stack.propagate(self.view_stack.leaf(), |graph| {
                    if graph.uuid == *graph_id {
                        graph.swap_inputs(*a, *b)
                    } else {
                        graph
                    }
                });
                true
            }
            SwapOutputs(graph_id, a, b) => {
                let _ = self
                    .view_stack
                    .propagate(self.view_stack.leaf(), |mut graph| {
                        if graph.uuid == *graph_id {
                            if graph.start == Some(a.node)
                                && let Some(loop_id) = graph.looper
                            {
                                let meta_loop = graph.nodes.get_mut(&loop_id).unwrap();
                                let looper = meta_loop.value.as_node_mut::<LoopControl>().unwrap();

                                looper.fields.swap(a.output, b.output);

                                graph
                                    .swap_inputs(
                                        InPinId {
                                            node: loop_id,
                                            input: a.output,
                                        },
                                        InPinId {
                                            node: loop_id,
                                            input: b.output,
                                        },
                                    )
                                    .swap_outputs(*a, *b)
                            } else {
                                graph.swap_outputs(*a, *b)
                            }
                        } else {
                            graph
                        }
                    });
                true
            }
            ProgressBegin(id, max) => {
                if let Some(viewer) = &mut self.viewer {
                    viewer
                        .progress
                        .insert(*id, ProgressEntry(*max, Default::default()));
                }

                true
            }
            ProgressAdd(id, incr) => {
                if let Some(viewer) = &mut self.viewer {
                    let entry = viewer.progress.entry(*id).or_default();
                    entry
                        .1
                        .fetch_add(*incr, std::sync::atomic::Ordering::Relaxed);
                }

                true
            }
            ProgressEnd(id) => {
                if let Some(viewer) = &mut self.viewer {
                    viewer.progress.remove(id);
                }

                true
            }
            NodesChanged(_, _) => {
                self.graph_checked = false;
                false
            }
            ToolsChanged => {
                self.graph_checked = false;
                true
            }
            _ => false,
        }
    }
}
