use eframe::egui;
use egui::WidgetText;
use egui_commonmark::*;
use egui_snarl::{Snarl, ui::SnarlStyle};
use rig::message::Message;
use rmcp::model::Tool;
use std::{
    ops::DerefMut,
    path::Path,
    sync::{
        Arc, RwLock,
        atomic::{AtomicBool, AtomicU16},
    },
};
use uuid::Uuid;

use super::{Pane, tiles};
use crate::{
    AgentFactory, LogEntry, Settings, ToolSpec,
    chat::ChatSession,
    utils::ErrorList,
    workflow::{ShadowGraph, WorkNode, store::WorkflowStore},
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
    pub scratch: Arc<RwLock<Vec<Result<Message, String>>>>,
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
                let mut settings_rw = self.settings.write().unwrap();
                tiles::settings::settings_ui(ui, settings_rw.deref_mut());
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
        };

        Default::default()
    }
}

#[derive(Default)]
pub struct WorkflowState {
    pub running: Arc<AtomicBool>,
    pub editing: Option<String>,
    pub store: WorkflowStore,
    pub snarl: Arc<tokio::sync::RwLock<Snarl<WorkNode>>>,
    pub style: SnarlStyle,
    pub baseline: ShadowGraph<WorkNode>,
    pub shadow: ShadowGraph<WorkNode>,
}

impl WorkflowState {
    pub fn from_path(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        use egui_snarl::ui::{BackgroundPattern, Grid, NodeLayout, PinPlacement, SnarlStyle};
        let workflows = WorkflowStore::load(path)?;

        let edit_workflow = Some("default".to_string());
        let workflow = workflows.get(edit_workflow.as_deref().unwrap());

        let baseline: ShadowGraph<WorkNode> = ShadowGraph::from_snarl(&workflow);

        let snarl = Arc::new(tokio::sync::RwLock::new(workflow));

        Ok(Self {
            running: Arc::new(AtomicBool::new(false)),
            editing: edit_workflow.clone(),
            store: workflows.clone(),
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
        })
    }
}
