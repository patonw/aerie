use arc_swap::ArcSwap;
#[cfg(feature = "ui")]
use egui_commonmark::CommonMarkCache;
use footnote::Footnote;
use serde::{Deserialize, Serialize};
use std::{
    hash::Hash,
    sync::{Arc, atomic::AtomicU16},
};
use struct_patch::Patch;
use typed_builder::TypedBuilder;
use uuid::Uuid;

#[cfg(feature = "native")]
use serde_yaml_ng as serde_yml;
#[cfg(feature = "native")]
use std::path::Path;

#[cfg(feature = "ui")]
use rmcp::model::Tool;

#[cfg(feature = "wip")]
#[cfg(feature = "ui")]
use crate::ui::tiles::messages::MessageGraph;

#[cfg(feature = "native")]
use crate::{
    config::{ConfigExt as _, ConfigStateStore},
    workflow::store::{WorkflowStore, WorkflowStoreDir},
};

#[cfg(not(feature = "native"))]
use crate::{config::ConfigStateProxy, workflow::store::WorkflowStoreStub};

#[cfg(feature = "wip")]
use crate::agent::AgentFactory;

#[cfg(feature = "ui")]
use crate::{
    toolbox::Toolbox,
    ui::{ShowHelp, workflow::WorkflowViewer},
};

use crate::{
    chat::ChatSession,
    config::{Preferences, ToolSpec},
    events::AppEvents,
    logging::LogEntry,
    state::workflow::WorkflowState,
    toolbox::ToolStore,
    transmute::Transmuter,
    utils::ErrorList,
    workflow::runner::RunEventCast,
};

#[cfg(feature = "ui")]
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
#[derive(Debug, Clone, Serialize, Deserialize, Default, TypedBuilder, Footnote, Patch)]
#[patch(attribute(derive(Debug, Default, Clone, Serialize, Deserialize)))]
pub struct AppData {
    #[patch(skip)] // TODO: impl Hash via newtype
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

impl Hash for AppData {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.get_note().hash(state);
    }
}

impl PartialEq for AppData {
    fn eq(&self, other: &Self) -> bool {
        self.get_note() == other.get_note()
    }
}

impl Eq for AppData {}

// TODO: Lift business-logic classes to top-level and leave this as pure data
// TODO: use AppActions to trigger server-side handlers which update state
#[derive(TypedBuilder)]
pub struct AppState {
    #[builder(default)]
    pub data: AppData,

    #[cfg(feature = "native")]
    pub rt: tokio::runtime::Handle,

    /// Tells event loop when something interesting happens from a background task
    #[cfg(feature = "native")]
    #[builder(default)]
    pub notify: Arc<tokio::sync::Notify>,

    // TODO: migrate
    #[cfg(feature = "ui")]
    #[builder(default)]
    pub show_help: Option<ShowHelp>,

    // TODO: migrate
    #[builder(default)]
    pub events: Arc<AppEvents>,

    #[builder(default)]
    pub run_cast: Option<RunEventCast>,

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

    #[cfg(feature = "ui")]
    #[builder(default)]
    pub cache: CommonMarkCache,

    // TODO: migrate
    #[builder(default)]
    pub prompt: String,

    // TODO: migrate
    #[builder(default)]
    pub images: Arc<ArcSwap<im::Vector<String>>>,

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

    #[cfg(feature = "ui")]
    #[builder(default)]
    pub tool_editor: Option<ToolEditorState>,

    #[cfg(feature = "native")]
    pub workflows: WorkflowState<WorkflowStoreDir>,

    #[cfg(not(feature = "native"))]
    pub workflows: WorkflowState<WorkflowStoreStub>,

    #[cfg(feature = "ui")]
    #[cfg(feature = "wip")]
    #[builder(default)]
    pub message_graph: MessageGraph,

    #[builder(default)]
    pub transmuter: Transmuter,
}

impl AppState {
    #[cfg(feature = "ui")]
    #[cfg(feature = "native")]
    fn get_toolbox(&self) -> Toolbox {
        self.agent_factory.toolbox.clone()
    }

    #[cfg(feature = "ui")]
    #[cfg(not(feature = "native"))]
    fn get_toolbox(&self) -> Toolbox {
        use crate::toolbox::ToolProvider;

        let toolbox = Toolbox::default();

        toolbox.with_provider(
            "chainer",
            ToolProvider::Chainer {
                workflows: self.workflows.store.clone(),
                next_workflow: Default::default(),
                next_prompt: Default::default(),
                next_images: Default::default(),
            },
        );

        for (provider, tools) in &self.tools.tool_info {
            let entry = ToolProvider::MCP {
                tools: tools.clone(),
            };
            toolbox.with_provider(provider, entry);
        }

        toolbox
    }

    #[cfg(feature = "ui")]
    pub fn workflow_viewer(&mut self) -> &mut WorkflowViewer {
        if self.workflows.viewer.is_none() {
            use crate::workflow::EditContext;

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
                .progress(self.workflows.progress.clone())
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
        use crate::{events::AppEvent, utils::ErrorDistiller as _};
        use AppEvent::*;
        use itertools::Itertools;

        // Only allow one run request per frame
        let mut executed = false;
        let errors = self.errors.clone();
        let settings = self.data.prefs.clone();

        while let Some(event) = self.events.pop() {
            let mut handled = false;

            handled = handled || self.workflows.handle_event(&event);

            handled = handled
                || match &event {
                    DiscardErrors => {
                        self.errors.discard();
                        true
                    }
                    CreateWorkflow => {
                        let datetime = chrono::offset::Local::now();
                        let timestamp = datetime.format("unnamed-%Y-%m-%dT%H:%M:%S").to_string();
                        self.workflows.store.put(&timestamp, Default::default());
                        self.workflows.switch(&timestamp);
                        true
                    }
                    ImportWorkflow(name, bytes) => {
                        errors.distil(self.workflows.import_bytes(name, bytes));
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
                    InterruptWorkflow => {
                        use std::sync::atomic::Ordering;
                        self.workflows.interrupt.store(true, Ordering::Relaxed);
                        true
                    }
                    ImportTools(file_name, bytes) => {
                        use anyhow::Context as _;

                        use crate::storage::CachedDirStore as _;

                        let name = Path::new(file_name)
                            .file_stem()
                            .and_then(|s| s.to_os_string().into_string().ok())
                            .unwrap_or_default();

                        let exists = self.tools.contains(&name);

                        let name = if name.is_empty() || exists {
                            let datetime = chrono::offset::Local::now();
                            let timestamp = datetime.format("%Y-%m-%dT%H:%M:%S").to_string();
                            std::iter::chain([name], [timestamp]).join("-")
                        } else {
                            name
                        };

                        if let Some(mut data) = errors.distil(
                            serde_yml::from_slice::<ToolSpec>(bytes)
                                .context("Could not deserialize tool spec"),
                        ) {
                            data.set_enabled(false);
                            self.errors.distil(self.tools.save(&name, &data));
                            self.events.insert(AppEvent::ToolsChanged);
                        }

                        true
                    }
                    SaveToolProvider(old_name, name, spec) => {
                        use crate::storage::CachedDirStore as _;

                        if let Some(old_name) = old_name {
                            self.agent_factory.stop_provider(old_name);

                            errors.distil(self.tools.remove(old_name));
                        }

                        errors.distil(self.tools.save(name, spec));
                        self.events
                            .insert(AppEvent::ReloadToolProvider(name.to_string()));

                        true
                    }
                    ReloadToolProvider(name) => {
                        self.agent_factory.stop_provider(name);

                        let name = name.to_string();
                        let agent_factory = self.agent_factory.clone();
                        let errors = self.errors.clone();
                        let events = self.events.clone();
                        let notify = self.notify.clone();

                        self.rt.spawn(async move {
                            errors.distil(agent_factory.reload_provider(&name).await);
                            events.insert(AppEvent::ToolsChanged);
                            notify.notify_one();
                        });

                        true
                    }
                    RemoveToolProvider(name) => {
                        use crate::storage::CachedDirStore as _;

                        errors.distil(self.tools.remove(name));
                        self.agent_factory.stop_provider(name);
                        self.events.insert(AppEvent::ToolsChanged);

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
                            self.images.store(Default::default());
                        }
                        true
                    }
                    AddChatImages(new_images) => {
                        self.images.rcu(|images| {
                            let mut images = im::Vector::clone(images);
                            images.extend(new_images.iter().cloned());
                            Arc::new(images)
                        });
                        true
                    }
                    RemoveChatImage(idx) => {
                        let idx = *idx as usize;
                        self.images.rcu(|images| {
                            let mut images = im::Vector::clone(images);
                            if idx < images.len() {
                                images.remove(idx);
                            }
                            Arc::new(images)
                        });
                        true
                    }
                    ClearChatImages => {
                        self.images.store(Default::default());
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
                        self.data.session_names = self.data.session.relist();
                        true
                    }
                    SwitchSession(session_name) => {
                        errors.distil(self.data.session.switch(session_name));
                        self.state.set_session(self.data.session.name());
                        self.data.session_names = self.data.session.relist();
                        true
                    }
                    RenameSession(_old_name, new_name) => {
                        errors.distil(self.data.session.rename(new_name));
                        self.state.set_session(new_name);
                        self.data.session_names = self.data.session.relist();
                        true
                    }
                    ImportSession(fname, bytes) => {
                        // self.state.set_export_dir(path.parent());
                        errors.distil(self.data.session.import_bytes(fname, bytes));
                        self.state.set_session(self.data.session.name());
                        self.data.session_names = self.data.session.relist();
                        true
                    }
                    DeleteSession(session_name) => {
                        errors.distil(self.data.session.switch(""));
                        errors.distil(self.data.session.delete(session_name));
                        self.state.set_session(self.data.session.name());
                        self.data.session_names = self.data.session.relist();
                        true
                    }
                    SwitchBranch(name) => {
                        errors.distil(
                            self.data
                                .session
                                .transform(|history| Ok(history.switch(name))),
                        );

                        true
                    }
                    ToolsChanged => {
                        use crate::{storage::CachedDirStore, toolbox::ToolProvider};

                        self.tools.tool_info.clear();
                        let names = self.tools.scan_names().map(|s| s.to_string()).collect_vec();

                        for provider in names {
                            self.tools
                                .tool_info
                                .insert(provider.to_string(), Default::default());
                        }

                        let providers = self.agent_factory.toolbox.providers.load();
                        let mcp_tools = providers.iter().filter_map(|(n, p)| match p {
                            ToolProvider::Chainer { .. } => None,
                            ToolProvider::MCP { tools, .. } => Some((n, tools)),
                        });

                        for (provider, tools) in mcp_tools {
                            tracing::debug!(provider, "Extracting tool info");

                            self.tools.tool_info.insert(provider.clone(), tools.clone());
                        }

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
