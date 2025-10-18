use crate::Settings;

pub fn settings_ui(ui: &mut egui::Ui, settings_rw: &mut Settings) {
    egui::ScrollArea::vertical().show(ui, |ui| {
        egui::ComboBox::from_label("Model")
            .selected_text(settings_rw.llm_model.to_string())
            .show_ui(ui, |ui| {
                ui.selectable_value(
                    &mut settings_rw.llm_model,
                    "devstral:latest".to_string(),
                    "Devstral",
                );
                ui.selectable_value(
                    &mut settings_rw.llm_model,
                    "magistral:latest".to_string(),
                    "Magistral",
                );
                ui.selectable_value(
                    &mut settings_rw.llm_model,
                    "my-qwen3-coder:30b".to_string(),
                    "Qwen3 Coder",
                );
            });

        ui.add(
            egui::TextEdit::multiline(&mut settings_rw.preamble)
                .hint_text("Preamble")
                .desired_width(f32::INFINITY),
        );

        ui.add(egui::Slider::new(&mut settings_rw.temperature, 0.0..=1.0).text("T"))
            .on_hover_text("temperature");

        egui::CollapsingHeader::new("Flags")
            .default_open(true)
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    // ui.spacing_mut().item_spacing.x = 0.0;
                    ui.toggle_value(&mut settings_rw.autoscroll, "autoscroll");
                    // ui.toggle_value(&mut settings_rw.show_logs, "logs");
                });
            });
        ui.allocate_space(egui::vec2(ui.available_width(), 0.0))
    });
}
