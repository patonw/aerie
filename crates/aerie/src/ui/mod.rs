use eframe::egui;
use egui::WidgetText;

mod behavior;
mod tiles;

pub use behavior::AppBehavior;

pub enum Pane {
    Settings,
    Navigator,
    Chat,
    Logs,
    Workflow,
    Tools,
}

fn user_bubble<R>(ui: &mut egui::Ui, cb_r: impl FnMut(&mut egui::Ui) -> R) -> R {
    egui::Sides::new()
        .show(
            ui,
            |_| {},
            |ui| {
                egui::Frame::new()
                    .stroke(egui::Stroke::new(1.0, egui::Color32::GRAY))
                    .corner_radius(16)
                    .outer_margin(4)
                    .inner_margin(8)
                    .show(ui, cb_r)
                    .inner
            },
        )
        .1
}

fn agent_bubble<R>(
    ui: &mut egui::Ui,
    cb: impl FnMut(&mut egui::Ui) -> R,
) -> egui::InnerResponse<R> {
    ui.with_layout(egui::Layout::left_to_right(egui::Align::TOP), |ui| {
        egui::Frame::new()
            .stroke(egui::Stroke::new(1.0, egui::Color32::GRAY))
            .corner_radius(16)
            .outer_margin(4)
            .inner_margin(8)
            .show(ui, cb)
            .inner
    })
}

fn error_bubble<R>(
    ui: &mut egui::Ui,
    cb: impl FnMut(&mut egui::Ui) -> R,
) -> egui::InnerResponse<R> {
    ui.with_layout(egui::Layout::left_to_right(egui::Align::TOP), |ui| {
        egui::Frame::new()
            .inner_margin(12)
            .outer_margin(24)
            .corner_radius(14)
            .shadow(egui::Shadow {
                offset: [8, 12],
                blur: 16,
                spread: 0,
                color: egui::Color32::from_black_alpha(180),
            })
            // .fill(egui::Color32::from_rgba_unmultiplied(97, 0, 255, 128))
            .stroke(egui::Stroke::new(1.0, egui::Color32::RED))
            .show(ui, cb)
            .inner
    })
}

fn toggled_field<'a, T: Default>(
    ui: &mut egui::Ui,
    label: impl egui::IntoAtoms<'a>,
    tooltip: Option<impl Into<WidgetText>>,
    value: &mut Option<T>,
    cb: impl Fn(&mut egui::Ui, &mut T),
) {
    let widget = ui.selectable_label(value.is_some(), label);
    let widget = if let Some(text) = tooltip {
        widget.on_hover_text(text)
    } else {
        widget
    };

    if widget.clicked() {
        *value = match value {
            Some(_) => None,
            None => Some(Default::default()),
        };
    }

    if let Some(current) = value {
        cb(ui, current);
    } else {
        ui.label("Toggle label to edit");
    }
}
