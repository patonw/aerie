use eframe::egui;
use egui::WidgetText;
use egui_tiles::SimplificationOptions;

use crate::ui::Pane;

impl egui_tiles::Behavior<Pane> for super::AppState {
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
