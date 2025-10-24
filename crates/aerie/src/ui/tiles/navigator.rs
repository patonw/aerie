use std::collections::BTreeSet;

use crate::ChatHistory;

pub fn nav_ui(ui: &mut egui::Ui, history: &mut ChatHistory) {
    egui::CentralPanel::default().show_inside(ui, |ui| {
        egui::ScrollArea::vertical().show(ui, |ui| {
            let lineage = history.lineage();

            if let Some(children) = lineage.get("") {
                for child in children {
                    render_subtree(ui, &lineage, child, history);
                }
            }
        });
    });
}

fn render_subtree(
    ui: &mut egui::Ui,
    lineage: &std::collections::BTreeMap<String, BTreeSet<String>>,
    cursor: &str,
    history: &mut ChatHistory,
) {
    if let Some(children) = lineage.get(cursor) {
        let id = ui.make_persistent_id(format!("navigator_{cursor}"));
        egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, true)
            .show_header(ui, |ui| {
                ui.selectable_value(&mut history.head, Some(cursor.to_string()), cursor);
            })
            .body(|ui| {
                for child in children {
                    render_subtree(ui, lineage, child, history);
                }
            });
    } else {
        ui.selectable_value(&mut history.head, Some(cursor.to_string()), cursor);
    }
}
