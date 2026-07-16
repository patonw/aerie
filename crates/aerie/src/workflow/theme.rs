#![cfg(feature = "ui")]

use egui::{Color32, Stroke};
use std::fmt::Debug;
use typed_builder::TypedBuilder;

#[derive(Default, Debug, Clone, Copy, TypedBuilder)]
#[builder(field_defaults(default, setter(strip_option)))]
pub struct NodeTheme {
    #[builder(setter(into))]
    pub frame_stroke: Option<Stroke>,

    pub body_fill: Option<Color32>,

    pub collapsed_fill: Option<Color32>,
}

impl NodeTheme {
    pub fn apply_body(&self, mut frame: egui::Frame) -> egui::Frame {
        if let Some(stroke) = self.frame_stroke {
            frame = frame.stroke(stroke);
        }

        if let Some(color) = self.body_fill {
            frame = frame.fill(color);
        }

        frame
    }

    pub fn apply_header(&self, mut frame: egui::Frame) -> egui::Frame {
        if let Some(stroke) = self.frame_stroke {
            frame = frame.stroke(stroke);
        }

        if let Some(color) = self.body_fill {
            frame = frame.fill(color);
        }

        frame
    }

    pub fn apply_collapsed(&self, mut frame: egui::Frame) -> egui::Frame {
        frame = self.apply_header(frame);

        if let Some(color) = self.collapsed_fill {
            frame = frame.fill(color);
        }

        frame
    }

    pub fn apply_overview(&self, mut frame: egui::Frame, scaling: f32) -> egui::Frame {
        if let Some(mut stroke) = self.frame_stroke {
            // Scale-invariant stroke to make themed nodes easier to spot in overview
            stroke.width /= scaling;
            frame = frame.stroke(stroke);
        }

        frame
    }
}

// trait overkill?
pub trait ThemeCodex {
    fn comment_theme(&self) -> NodeTheme;

    fn neutral_theme(&self) -> NodeTheme;

    fn finisher_theme(&self) -> NodeTheme;

    fn branching_theme(&self) -> NodeTheme;

    fn remote_theme(&self) -> NodeTheme;

    fn nesting_theme(&self) -> NodeTheme;
}

// TODO: import themes from preferences
#[derive(Default, Debug, Clone)]
pub struct StandardTheme;

impl ThemeCodex for StandardTheme {
    fn comment_theme(&self) -> NodeTheme {
        NodeTheme::builder()
            .body_fill(egui::Color32::LIGHT_YELLOW.gamma_multiply(0.75))
            .collapsed_fill(Color32::from_rgb(0x88, 0x88, 0))
            .build()
    }

    fn neutral_theme(&self) -> NodeTheme {
        NodeTheme::builder()
            .frame_stroke((2.0, Color32::DARK_GRAY))
            .build()
    }

    fn finisher_theme(&self) -> NodeTheme {
        NodeTheme::builder()
            .frame_stroke((2.0, egui::Color32::from_rgb(0x26, 0x9f, 0x4c)))
            .build()
    }

    fn branching_theme(&self) -> NodeTheme {
        NodeTheme::builder()
            .frame_stroke((2.0, egui::Color32::from_rgb(0xe1, 0xa3, 0x72)))
            .build()
    }

    fn remote_theme(&self) -> NodeTheme {
        NodeTheme::builder()
            .frame_stroke((2.0, egui::Color32::from_rgb(0x32, 0x84, 0xa9)))
            .build()
    }

    fn nesting_theme(&self) -> NodeTheme {
        NodeTheme::builder()
            .frame_stroke((2.0, egui::Color32::from_rgb(0x8b, 0x44, 0x3a)))
            .build()
    }
}
