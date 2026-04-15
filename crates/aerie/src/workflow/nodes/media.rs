use egui_phosphor::regular::{IMAGE_BROKEN, IMAGES, X_CIRCLE};
use itertools::Itertools as _;
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

use crate::{
    utils::{
        ErrorDistiller as _, IMAGE_CACHE, cache_image, downscale_image, image_url_rig,
        resolve_image,
    },
    workflow::{DynNode, FlexNode, UiNode, Value, ValueKind, WorkNode, nodes::GraphSubmenu},
};

/// Stores images as data URIs in the graph
#[skip_serializing_none]
#[derive(Default, Debug, Clone, Hash, PartialEq, Eq, Deserialize, Serialize)]
pub struct LoadImages {
    #[serde(default)]
    images: im::Vector<String>,
}

#[typetag::serde]
impl FlexNode for LoadImages {}

impl DynNode for LoadImages {
    fn inputs(&self) -> usize {
        0
    }

    fn out_kind(&self, _out_pin: usize) -> super::ValueKind {
        super::ValueKind::Images
    }

    fn execute(
        &mut self,
        _ctx: &super::RunContext,
        _node_id: egui_snarl::NodeId,
        _inputs: Vec<Option<super::Value>>,
    ) -> Result<Vec<super::Value>, crate::workflow::WorkflowError> {
        let images = self
            .images
            .iter()
            .map(|uri| image_url_rig(uri, false))
            .collect::<anyhow::Result<im::Vector<_>>>()?;

        Ok(vec![Value::Images(images)])
    }
}

impl UiNode for LoadImages {
    fn title(&self) -> &str {
        "Images"
    }

    fn tooltip(&self) -> &str {
        "Load images from disk at edit-time and inline them into the workflow.\n\
            **warning**: this will significantly increase the file size of the workflow."
    }

    fn has_body(&self) -> bool {
        true
    }

    // TODO: a square-ish 4xN gallery
    fn show_body(&mut self, ui: &mut egui::Ui, ctx: &super::EditContext) {
        ui.vertical(|ui| {
            egui::ScrollArea::vertical()
                .max_height(300.0)
                .show(ui, |ui| {
                    ui.set_width(200.0);
                    ui.set_max_height(3.0 * ui.available_width() * self.images.len() as f32);

                    let mut remove_idx = None;
                    for (i, uri) in self.images.iter().enumerate() {
                        // TODO: needs optimization
                        let resp = match image_url_rig(uri, false) {
                            Ok(image) => {
                                let key = cache_image(&image);
                                let mut cache = IMAGE_CACHE.lock();
                                if let Some(image) = cache.get(&key) {
                                    let widget = egui::Image::new(image.clone())
                                        .max_size(egui::vec2(200.0, 200.0));
                                    let resp = ui.add(widget).on_hover_ui(|ui| {
                                        ui.add(
                                            egui::Image::new(image.clone())
                                                .max_size(ui.ctx().content_rect().size() * 0.75),
                                        );
                                    });
                                    Some(resp)
                                } else {
                                    None
                                }
                            }
                            _ => None,
                        };

                        let resp = resp.unwrap_or_else(|| {
                            ui.scope(|ui| {
                                ui.style_mut().text_styles.insert(
                                    egui::TextStyle::Body,
                                    egui::FontId::new(
                                        64.0,
                                        eframe::epaint::FontFamily::Proportional,
                                    ),
                                );
                                let width = ui.available_width();
                                let widget = egui::Label::new(IMAGE_BROKEN).selectable(false);
                                ui.add_sized(egui::vec2(width, width * 0.75), widget)
                            })
                            .inner
                        });

                        let button =
                            egui::Button::new(X_CIRCLE).fill(egui::Color32::from_black_alpha(0));
                        let rect = egui::Rect::from_center_size(
                            resp.rect.left_top() + egui::vec2(20.0, 8.0),
                            egui::vec2(16.0, 16.0),
                        );
                        let resp = ui.place(rect, button);
                        if resp.interact(egui::Sense::click()).clicked() {
                            remove_idx = Some(i);
                        }
                    }

                    if let Some(idx) = remove_idx {
                        self.images.remove(idx);
                    }
                });

            if ui
                .button(IMAGES)
                .on_hover_text("Load images from disk")
                .clicked()
                && let Some(paths) = rfd::FileDialog::new()
                    // .set_directory(settings.view(|s| s.last_export_dir.clone())) // TODO: get settings
                    .add_filter("images", &["png", "jpg", "jpeg", "webp"])
                    .add_filter("all", &[""])
                    .pick_files()
            {
                let paths = paths
                    .into_iter()
                    .filter_map(|p| p.to_str().map(|s| s.to_string()));

                tracing::info!("Loading images {paths:?}");

                let images = paths
                    .map(|p| {
                        resolve_image(&p, true).and_then(|(im, fmt)| downscale_image(&im, fmt))
                    })
                    .map_ok(|(bytes, format)| {
                        use base64::{Engine, prelude::BASE64_STANDARD};
                        let image_base64 = BASE64_STANDARD.encode(&bytes);
                        let mime_type = format.map(|f| f.to_mime_type()).unwrap_or_default();
                        format!("data:{mime_type};base64,{image_base64}")
                    })
                    .collect::<Result<Vec<_>, _>>();

                let Some(images) = ctx.errors.distil(images) else {
                    return;
                };

                // let Some(images) = ctx
                //     .errors
                //     .distil(images.context("Problem converting image to data URI"))
                // else {
                //     return;
                // };

                for img in &images {
                    tracing::info!("img {}", &img[..32]);
                }
                self.images.extend(images);
            }
        });
    }
}

#[skip_serializing_none]
#[derive(Default, Debug, Clone, Hash, PartialEq, Eq, Deserialize, Serialize)]
pub struct FetchImages {}

#[typetag::serde]
impl FlexNode for FetchImages {}

impl DynNode for FetchImages {
    fn in_kinds(&'_ self, _in_pin: usize) -> std::borrow::Cow<'_, [super::ValueKind]> {
        std::borrow::Cow::Borrowed(&[ValueKind::TextList, ValueKind::Text])
    }

    fn out_kind(&self, _out_pin: usize) -> ValueKind {
        ValueKind::Images
    }

    fn execute(
        &mut self,
        _ctx: &super::RunContext,
        _node_id: egui_snarl::NodeId,
        inputs: Vec<Option<Value>>,
    ) -> Result<Vec<Value>, crate::workflow::WorkflowError> {
        let texts = match &inputs[0] {
            Some(Value::Text(text)) => vec![text.as_str()],
            Some(Value::TextList(texts)) => texts.iter().map(|s| s.as_str()).collect(),
            None => vec![],
            _ => unreachable!(),
        };

        let images = texts
            .iter()
            .map(|uri| image_url_rig(uri, false))
            .collect::<anyhow::Result<im::Vector<_>>>()?;

        Ok(vec![Value::Images(images)])
    }
}

impl UiNode for FetchImages {
    fn title(&self) -> &str {
        "Fetch Images"
    }

    fn tooltip(&self) -> &str {
        "Fetch remote images from URLs"
    }
}

fn media_nodes(ui: &mut egui::Ui, snarl: &mut egui_snarl::Snarl<WorkNode>, pos: egui::Pos2) {
    ui.menu_button("Media", |ui| {
        if ui
            .button("Fetch Images")
            .on_hover_text("Retrieve remote images from URLs (at run time).")
            .clicked()
        {
            snarl.insert_node(pos, FetchImages::default().into());
        }

        if ui
            .button("Load Images")
            .on_hover_text("Load images from disk and embed them into the graph.")
            .clicked()
        {
            snarl.insert_node(pos, LoadImages::default().into());
        }
    });
}

inventory::submit! {
    GraphSubmenu("media", media_nodes)
}
