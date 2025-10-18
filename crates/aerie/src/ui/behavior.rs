use eframe::egui;
use egui::WidgetText;
use egui_commonmark::*;
use rig::{agent::Agent, message::Message, providers::ollama::CompletionModel};
use std::{
    ops::DerefMut,
    sync::{Arc, RwLock, atomic::AtomicU16},
};

use super::{Pane, tiles};
use crate::{LogEntry, Settings};

// TODO: persist/restore sessions
pub struct AppBehavior {
    pub rt: tokio::runtime::Handle,
    pub settings: Arc<RwLock<Settings>>,
    pub task_count: Arc<AtomicU16>,
    pub log_history: Arc<RwLock<Vec<LogEntry>>>,
    pub chat: Arc<RwLock<Vec<Result<Message, String>>>>,
    pub cache: CommonMarkCache,
    pub prompt: Arc<RwLock<String>>,
    pub llm_agent: Arc<Agent<CompletionModel>>,
}

impl egui_tiles::Behavior<Pane> for AppBehavior {
    fn tab_title_for_pane(&mut self, pane: &Pane) -> WidgetText {
        match pane {
            Pane::Settings => "Settings".into(),
            Pane::Chat => "Chat".into(),
            Pane::Logs => "Logs".into(),
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
                {
                    let mut settings_rw = self.settings.write().unwrap();
                    tiles::settings::settings_ui(ui, settings_rw.deref_mut());
                };
            }
            Pane::Chat => {
                self.chat_ui(ui);
            }
            Pane::Logs => {
                {
                    let logs_r = self.log_history.read().unwrap();
                    tiles::logview::log_ui(ui, logs_r.as_ref());
                };
            }
        };

        Default::default()
    }
}
