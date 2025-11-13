use eframe::egui;
use egui::WidgetText;
use egui_commonmark::*;
use rig::message::Message;
use std::{
    ops::DerefMut,
    sync::{Arc, RwLock, atomic::AtomicU16},
};
use uuid::Uuid;

use super::{Pane, tiles};
use crate::{AgentFactory, LogEntry, Settings, chat::ChatSession};

pub struct AppBehavior {
    pub rt: tokio::runtime::Handle,
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
    pub dest_branch: String,

    pub create_toolset: Option<String>,
    pub edit_toolset: String,
}

impl egui_tiles::Behavior<Pane> for AppBehavior {
    fn tab_title_for_pane(&mut self, pane: &Pane) -> WidgetText {
        match pane {
            Pane::Settings => "Settings".into(),
            Pane::Navigator => "Branches".into(),
            Pane::Chat => "Chat".into(),
            Pane::Logs => "Logs".into(),
            Pane::Workflow => "Workflow".into(),
            Pane::Tools => "Tools".into(),
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
                let _ = self.session.update(|history| {
                    tiles::navigator::nav_ui(ui, history);
                });
            }
            Pane::Chat => {
                self.chat_ui(ui);
            }
            Pane::Logs => {
                let logs_r = self.log_history.read().unwrap();
                tiles::logview::log_ui(ui, logs_r.as_ref());
            }
            Pane::Workflow => {
                self.workflow_ui(ui);
            }
            Pane::Tools => {
                self.toolset_ui(ui);
            }
        };

        Default::default()
    }
}
