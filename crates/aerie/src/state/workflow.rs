use anyhow::Context as _;
use arc_swap::ArcSwap;
use egui_snarl::NodeId;
use egui_snarl::Snarl;
use im::vector;
use itertools::Itertools;
use itertools::izip;
use std::{
    borrow::Cow,
    collections::VecDeque,
    hash::Hash,
    iter,
    sync::{Arc, atomic::AtomicBool},
};
use tokio::sync::broadcast;
use typed_builder::TypedBuilder;
use uuid::Uuid;
use web_time::{Duration, SystemTime};

#[cfg(feature = "native")]
use egui_snarl::InPinId;
#[cfg(feature = "native")]
use serde_yaml_ng as serde_yml;
#[cfg(feature = "native")]
use std::fs::OpenOptions;
#[cfg(feature = "native")]
use std::path::Path;

#[cfg(feature = "native")]
use crate::workflow::store::WorkflowStore;
#[cfg(not(feature = "native"))]
use crate::workflow::store::WorkflowStore;

#[cfg(feature = "native")]
use crate::workflow::nodes::LoopControl;

#[cfg(feature = "ui")]
use crate::{events::ProgressEvent, ui::workflow::WorkflowViewer};

use crate::{
    core::Id,
    events::{AppEvent, WorkflowEvent},
    workflow::{
        DynNode, GraphId, MetaNode, PreviewData, ShadowGraph, WorkNode, Workflow,
        nodes::{Flavor, Subgraph},
        runner::{ExecId, NodeStateMap, WorkflowRun},
    },
};

const SOFT_LIMIT: usize = 128;

#[derive(Default, Clone, Debug)]
pub struct ProgressEntry(pub u64, pub u64);

pub type ProgressMap = im::OrdMap<Uuid, ProgressEntry>;

impl std::ops::Add<u64> for ProgressEntry {
    type Output = Self;

    fn add(self, rhs: u64) -> Self::Output {
        Self(self.0, self.1 + rhs)
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
    #[cfg(feature = "ui")]
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
    pub progress: Arc<ArcSwap<ProgressMap>>,

    /// Undo/redo support
    pub undo_stack: im::OrdMap<String, VecDeque<(SystemTime, Workflow)>>,
    pub redo_stack: im::OrdMap<String, VecDeque<(SystemTime, Workflow)>>,

    pub previews: PreviewData,
    pub outputs: im::Vector<WorkflowRun>,

    pub graph_checked: bool,
    pub alerts: im::OrdMap<(GraphId, NodeId), Cow<'static, str>>,

    pub broadcast: broadcast::Sender<WorkflowEvent>,
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
            #[cfg(feature = "ui")]
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
            progress: Default::default(),
            undo_stack: Default::default(),
            redo_stack: Default::default(),
            previews: Default::default(),
            outputs: Default::default(),
            graph_checked: false,
            alerts: Default::default(),
            broadcast: broadcast::channel(256).0,
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<WorkflowEvent> {
        self.broadcast.subscribe()
    }

    pub fn with_node_state(mut self, node_state: NodeStateMap) -> Self {
        self.node_state = node_state;
        self
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
        #[cfg(feature = "ui")]
        if !self.graph_checked
            && let Some(viewer) = &self.viewer
        {
            use crate::workflow::CheckContext;

            let ctx = CheckContext::builder()
                .toolbox(viewer.edit_ctx.toolbox.clone())
                .graph_id(self.shadow.graph.uuid)
                .build();

            self.alerts = self.shadow.graph.check(&ctx);
            self.graph_checked = true;
        }
    }

    #[cfg(feature = "web-ui")]
    pub fn patch(&mut self, patch: crate::client::EditorPatch) {
        use std::sync::atomic::Ordering;

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

        self.running.store(patch.running, Ordering::Relaxed);
        self.interrupt.store(patch.interrupt, Ordering::Relaxed);
    }

    pub fn switch(&mut self, workflow_name: &str) {
        if self.editing.as_str() == workflow_name {
            return;
        }

        let Some(target) = self.store.get(workflow_name) else {
            tracing::warn!("Could not load workflow {workflow_name}");
            #[cfg(feature = "ui")]
            if let Some(viewer) = &self.viewer {
                use crate::utils::ErrorDistiller as _;

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
        #[cfg(feature = "ui")]
        {
            self.viewer = None;
        }
        self.graph_checked = false;
    }

    #[cfg(feature = "native")]
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

    #[cfg(feature = "native")]
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

        #[cfg(feature = "ui")]
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
            #[cfg(feature = "ui")]
            {
                self.viewer = None;
            }
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

    #[cfg(feature = "native")]
    pub fn export(&mut self, path: &Path) -> anyhow::Result<()> {
        let writer = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)?;

        serde_yml::to_writer(writer, &self.shadow.repair())?;
        Ok(())
    }

    #[cfg(feature = "native")]
    pub fn import(&mut self, path: &Path) -> anyhow::Result<()> {
        use std::io::Read as _;

        if !path.is_file() {
            anyhow::bail!("Invalid file: {path:?}");
        }

        let name = path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or("unknown".into());

        let mut bytes = Vec::new();
        let mut reader = OpenOptions::new().read(true).open(path)?;
        reader.read_to_end(&mut bytes)?;

        self.import_bytes(&name, &bytes)?;
        Ok(())
    }

    #[cfg(feature = "native")]
    pub fn import_bytes(&mut self, name: &str, bytes: &[u8]) -> anyhow::Result<()> {
        let name = Path::new(name)
            .file_stem()
            .and_then(|s| s.to_os_string().into_string().ok())
            .unwrap_or("unknown".into());

        let datetime = chrono::offset::Local::now();
        let timestamp = datetime.format("%Y-%m-%dT%H:%M:%S").to_string();
        let name = if name.is_empty() || self.names().contains(name.as_str()) {
            std::iter::chain([name], [timestamp]).join("-")
        } else {
            name
        };

        let data: Workflow = serde_yml::from_slice(bytes)?;

        self.undo_stack
            .entry(name.clone())
            .or_default()
            .push_front((self.modtime, data.clone()));

        self.store.save(&name, data)?;
        self.switch(&name);
        self.baseline = Default::default();

        Ok(())
    }

    #[cfg(feature = "native")]
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

    #[cfg(feature = "ui")]
    pub fn handle_progress(&mut self, event: &ProgressEvent) {
        match event {
            ProgressEvent::Begin(id, max) => {
                self.progress
                    .rcu(|progress| progress.update(*id, ProgressEntry(*max, Default::default())));
            }
            ProgressEvent::Add(id, incr) => {
                self.progress
                    .rcu(|progress| progress.alter(|it| it.map(|x| x + *incr), *id));
            }
            ProgressEvent::End(id) => {
                self.progress.rcu(|progress| progress.without(id));
            }
        }
    }

    // TODO: collect errors
    #[cfg(feature = "native")]
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
                #[cfg(feature = "ui")]
                {
                    self.viewer = None;
                }
                true
            }
            LeaveSubgraph(levels) => {
                if !self.view_stack.is_empty() {
                    self.view_stack.exit(*levels).unwrap();
                    #[cfg(feature = "ui")]
                    {
                        self.viewer = None;
                    }
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
            Progress(_ev) => {
                #[cfg(feature = "ui")]
                self.handle_progress(_ev);

                // #[cfg(feature = "backend")]
                // let _ = self.broadcast.send(WorkflowEvent::Progress(ev.clone()));

                true
            }
            NodesChanged(_, _) => {
                self.graph_checked = false;
                false
            }
            ToolsChanged => {
                // TODO: extract tool info from providers into struct for client
                self.graph_checked = false;
                false
            }
            _ => false,
        }
    }
}

#[derive(TypedBuilder, Debug, Clone, Hash, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ViewStack {
    pub root_id: Id,

    pub root_name: String,

    #[builder(default)]
    pub path: im::Vector<NodeId>,

    #[builder(default)]
    pub flavor: im::Vector<Flavor>,

    pub levels: im::Vector<ShadowGraph<WorkNode>>,

    #[builder(default)]
    pub passes: im::Vector<usize>,
}

impl ViewStack {
    pub fn new(name: &str, workflow: Workflow, path: impl Iterator<Item = NodeId>) -> Self {
        let mut me = Self::builder()
            .root_id(Id::new(workflow.graph.uuid))
            .root_name(name.into())
            .flavor(vector![Flavor::Simple])
            .levels(vector![workflow.graph.as_ref().clone()])
            .passes(vector![0])
            .build();

        for id in path {
            if me.enter(id).is_err() {
                break;
            }
        }

        me
    }

    pub fn from_graph(
        name: &str,
        graph: &ShadowGraph<WorkNode>,
        path: impl Iterator<Item = NodeId>,
    ) -> Self {
        let mut me = Self::builder()
            .root_id(Id::new(graph.uuid))
            .root_name(name.into())
            .flavor(vector![Flavor::Simple])
            .levels(vector![graph.clone()])
            .passes(vector![0])
            .build();

        for id in path {
            if let Err(err) = me.enter(id) {
                tracing::error!(?err, "Could not restore view stack");
                break;
            }
        }

        me
    }

    pub fn from_root(name: &str, workflow: Workflow) -> Self {
        Self::new(name, workflow, iter::empty())
    }

    /// Replace root graph with a different version.
    ///
    /// Attempts to preserve path, but will navigate as far as possible
    /// if subgraphs are absent.
    pub fn switch(&mut self, name: &str, workflow: Workflow) {
        let path = self.path.clone();
        *self = Self::new(name, workflow, path.into_iter());
    }

    pub fn switch_graph(&mut self, graph: &ShadowGraph<WorkNode>) {
        let path = self.path.clone();
        *self = Self::from_graph(&self.root_name, graph, path.into_iter().rev());
    }

    pub fn is_empty(&self) -> bool {
        self.path.is_empty()
    }

    pub fn names(&'_ self) -> impl DoubleEndedIterator<Item = String> {
        self.path
            .iter()
            .zip(self.levels.skip(1))
            .map(|(node_id, graph)| {
                graph
                    .nodes
                    .get(node_id)
                    .map(|m| m.value.title().to_string())
                    .unwrap_or("???".to_string())
            })
            .chain(std::iter::once(self.root_name.clone()))
    }

    pub fn root(&self) -> ShadowGraph<WorkNode> {
        assert!(!self.levels.is_empty());
        self.levels.back().cloned().unwrap()
    }

    pub fn root_snarl(&self) -> anyhow::Result<Snarl<WorkNode>> {
        egui_snarl::Snarl::try_from(self.root())
    }

    pub fn leaf(&self) -> ShadowGraph<WorkNode> {
        assert!(!self.levels.is_empty());
        self.levels.front().cloned().unwrap()
    }

    pub fn leaf_snarl(&self) -> anyhow::Result<Snarl<WorkNode>> {
        egui_snarl::Snarl::try_from(self.leaf())
    }

    /// The id of the parent graph and node id of the subgraph container
    pub fn parent_id(&self) -> Option<(GraphId, NodeId)> {
        if self.is_empty() {
            None
        } else {
            Some((self.levels[1].uuid, self.path[0]))
        }
    }
    pub fn flavor(&self) -> Option<Flavor> {
        if self.is_empty() {
            None
        } else {
            Some(self.flavor[0])
        }
    }

    #[cfg(feature = "ui")]
    pub fn view_id(&self) -> egui::Id {
        egui::Id::new(self.root_id.with(&self.path))
    }

    pub fn exec_ids(&self) -> Vec<ExecId> {
        let mut result = vec![];

        for (level, pass) in self.levels.iter().rev().zip_eq(self.passes.iter().rev()) {
            if result.is_empty() {
                result.push(ExecId::from(level.uuid));
            } else {
                let child = result[result.len() - 1].scope(level.uuid, *pass);
                result.push(child);
            }
        }

        result.reverse();
        result
    }

    pub fn exec_id(&self) -> Option<ExecId> {
        self.exec_ids().first().cloned()
    }

    pub fn exit(&mut self, levels: usize) -> anyhow::Result<()> {
        for _ in 0..levels {
            if self.path.pop_front().is_none()
                || self.flavor.pop_front().is_none()
                || self.passes.pop_front().is_none()
                || self.levels.pop_front().is_none()
            {
                anyhow::bail!("stack is empty")
            }
        }

        Ok(())
    }

    pub fn enter(&mut self, node: NodeId) -> anyhow::Result<()> {
        let meta = self.leaf().nodes.get(&node).cloned().context(format!(
            "No such node {node:?} in graph {:?}",
            self.leaf().uuid,
        ))?;

        let container = meta.value;
        if !container.is_subgraph() {
            anyhow::bail!(
                "Node {node:?} {container:?} is not a subgraph of {:?}",
                self.leaf().uuid
            );
        }

        let Some(subgraph) = container.0.downcast_ref::<Subgraph>() else {
            anyhow::bail!("Could not extract subgraph");
        };

        tracing::info!(
            "Entering subgraph {:?} from {:?} node {node:?}",
            subgraph.uuid(),
            self.leaf().uuid
        );
        self.path.push_front(node);
        self.flavor.push_front(subgraph.flavor);
        self.passes.push_front(0);
        self.levels.push_front(subgraph.graph.clone());

        Ok(())
    }

    /// Cascades changes in the subgraphs up to the root,
    /// transforming each level with the callback.
    pub fn propagate<F>(&mut self, shadow: ShadowGraph<WorkNode>, f: F) -> anyhow::Result<()>
    where
        F: Fn(ShadowGraph<WorkNode>) -> ShadowGraph<WorkNode>,
    {
        let mut ids = self.path.iter();
        let mut graphs = self.levels.iter_mut();

        let Some(child_graph) = graphs.next() else {
            unreachable!()
        };

        // Transform the new leaf before assigning it to the stack
        let shadow = f(shadow);
        if !child_graph.fast_eq(&shadow) {
            *child_graph = shadow.clone();
        }

        let mut child_graph = shadow;

        let Some(mut child_id) = ids.next() else {
            return Ok(());
        };

        loop {
            let Some(parent_graph) = graphs.next() else {
                unreachable!()
            };

            let mut target = parent_graph.clone();

            match target.nodes.get(child_id) {
                Some(meta) if !meta.value.is_subgraph() => anyhow::bail!(
                    "Child node {child_id:?} of {:?} is not a subgraph",
                    parent_graph.uuid
                ),
                Some(meta) => {
                    if let Some(node) = meta.value.as_node::<Subgraph>() {
                        if !node.graph.fast_eq(&child_graph) {
                            // Replace the subgraph in the parent's node
                            let node = Subgraph {
                                graph: child_graph.clone(),
                                ..node.clone()
                            };

                            let meta = MetaNode {
                                value: node.into(),
                                ..meta.clone()
                            };

                            target.nodes.insert(*child_id, meta);
                        }

                        // Transform and save the parent graph
                        let target = f(target);
                        if !parent_graph.fast_eq(&target) {
                            *parent_graph = target.clone();
                        }
                        child_graph = target;
                    } else {
                        unreachable!()
                    }
                }
                None => anyhow::bail!(
                    "No such subgraph {child_id:?} in parent graph {:?}",
                    parent_graph.uuid
                ),
            }

            if let Some(parent_id) = ids.next() {
                child_id = parent_id;
            } else {
                break;
            }
        }

        Ok(())
    }

    // TODO: encapsulate state in an iterator struct instead of leveraging a Vec
    /// Iterate from the root to the leaf
    pub fn iter_levels(&mut self) -> impl Iterator<Item = ViewLevelInfo<'_>> {
        let names = self.names().collect_vec();
        let flavors = self.flavor.clone();
        let exec_ids = self.exec_ids();

        assert_eq!(names.len(), flavors.len());
        assert_eq!(names.len(), self.passes.len());

        let name_pass = izip!(names, flavors, exec_ids, self.passes.iter_mut()).collect_vec();
        let things = name_pass.into_iter().enumerate().rev().collect_vec();

        things.into_iter().map(|(i, it)| {
            ViewLevelInfo::builder()
                .depth(i)
                .name(Cow::Owned(it.0))
                .flavor(it.1)
                .exec_id(it.2)
                .pass(it.3)
                .build()
        })
    }
}

#[derive(TypedBuilder)]
pub struct ViewLevelInfo<'a> {
    /// Distance from the stack leaf (current view)
    pub depth: usize,

    /// Name of the subgraph
    pub name: Cow<'a, str>,

    /// Lookup key for node states
    pub exec_id: ExecId,

    /// Graph/Subgraph variant
    #[builder(default)]
    pub flavor: Flavor,

    /// Parent graph and node containing current subgraph
    #[builder(default)]
    pub container: Option<(GraphId, NodeId)>,

    /// Loop or iteration pass for current subgraph
    pub pass: &'a mut usize,
    // Total number of passes for subgraph in latest run
    // #[builder(default)]
    // pub max_pass: Option<usize>,
}

impl<'a> Iterator for &'a ViewStack {
    type Item = ViewLevelInfo<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        todo!()
    }
}
