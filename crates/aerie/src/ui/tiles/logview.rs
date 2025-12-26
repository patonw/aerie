use std::sync::atomic::Ordering;

use crate::config::ConfigExt as _;

impl super::AppState {
    pub fn logview_ui(&mut self, ui: &mut egui::Ui) {
        let scroll_bottom =
            self.task_count.load(Ordering::Relaxed) > 0 && self.settings.view(|s| s.autoscroll);

        egui::ScrollArea::both().show(ui, |ui| {
            let language = "json";
            let theme =
                egui_extras::syntax_highlighting::CodeTheme::from_memory(ui.ctx(), ui.style());

            let logs_r = self.log_history.load();
            for entry in logs_r.iter() {
                // ui.label(entry.message());
                egui_extras::syntax_highlighting::code_view_ui(
                    ui,
                    &theme,
                    entry.message(),
                    language,
                );
            }
            if scroll_bottom {
                ui.scroll_to_cursor(Some(egui::Align::BOTTOM));
            }
        });
    }
}
