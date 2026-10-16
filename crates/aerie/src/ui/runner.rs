use egui::RichText;
use egui_phosphor::regular::{PLAY, STOP};

pub fn play_button() -> egui::Button<'static> {
    egui::Button::new(play_layout())
}

pub fn play_layout() -> egui::text::LayoutJob {
    use egui::{Align, FontSelection, Style, text::LayoutJob};

    let style = Style::default();
    let mut layout_job = LayoutJob::default();
    RichText::new(PLAY)
        .color(egui::Color32::GREEN)
        .strong()
        .heading()
        .append_to(
            &mut layout_job,
            &style,
            FontSelection::Default,
            Align::Center,
        );

    RichText::new(" Run")
        .color(style.visuals.text_color())
        .append_to(
            &mut layout_job,
            &style,
            FontSelection::Default,
            Align::Center,
        );

    layout_job
}

pub fn stop_button(stopping: bool) -> egui::Button<'static> {
    egui::Button::new(stop_layout(stopping))
}

pub fn stop_layout(stopping: bool) -> egui::text::LayoutJob {
    use egui::{Align, FontSelection, Style, text::LayoutJob};

    let style = Style::default();
    let mut layout_job = LayoutJob::default();
    RichText::new(STOP)
        .color(egui::Color32::RED)
        .strong()
        .heading()
        .append_to(
            &mut layout_job,
            &style,
            FontSelection::Default,
            Align::Center,
        );

    RichText::new(if stopping { " Stopping" } else { " Stop" })
        .color(style.visuals.text_color())
        .append_to(
            &mut layout_job,
            &style,
            FontSelection::Default,
            Align::Center,
        );

    layout_job
}
