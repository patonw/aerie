use crate::LogEntry;

pub fn log_ui(ui: &mut egui::Ui, logs_r: &[LogEntry]) {
    egui::ScrollArea::both().show(ui, |ui| {
        let language = "json";
        let theme = egui_extras::syntax_highlighting::CodeTheme::from_memory(ui.ctx(), ui.style());

        for entry in logs_r.iter() {
            // ui.label(entry.message());
            egui_extras::syntax_highlighting::code_view_ui(ui, &theme, entry.message(), language);
        }
    });
}
