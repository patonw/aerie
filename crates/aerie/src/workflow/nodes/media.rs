#[cfg(feature = "ui")]
use std::borrow::Cow;
use std::hash::Hash;
use std::sync::Arc;
use std::time::Instant;

use arc_swap::ArcSwap;
#[cfg(feature = "ui")]
use egui_phosphor::regular::{IMAGE_BROKEN, IMAGES, X_CIRCLE};
use rayon::iter::{IntoParallelIterator, ParallelIterator as _};
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

#[cfg(feature = "ui")]
use crate::utils::{ArcVecExt as _, IMAGE_CACHE, rig_image_to_egui, spawn_wherever};
#[cfg(feature = "ui")]
use crate::{
    utils::ErrorDistiller as _,
    workflow::{UiNode, WorkNode, nodes::GraphSubmenu},
};

use crate::{
    utils::ImageResolver,
    workflow::{DynNode, FlexNode, Value, ValueKind, WorkflowError},
};

/// Stores images as data URIs in the graph
#[skip_serializing_none]
#[derive(Default, Debug, Clone, Deserialize, Serialize)]
pub struct LoadImages {
    #[serde(default)]
    // images: im::Vector<String>,
    images: Arc<ArcSwap<im::Vector<String>>>,
}

impl Hash for LoadImages {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.images.load().hash(state);
    }
}

impl PartialEq for LoadImages {
    fn eq(&self, other: &Self) -> bool {
        self.images.load().as_ref() == other.images.load().as_ref()
    }
}

impl Eq for LoadImages {}

#[typetag::serde]
impl FlexNode for LoadImages {}

impl DynNode for LoadImages {
    fn title(&self) -> &str {
        "Images"
    }

    fn inputs(&self) -> usize {
        0
    }

    fn out_kind(&self, _out_pin: usize) -> super::ValueKind {
        super::ValueKind::Images
    }

    #[cfg(feature = "native")]
    fn execute(
        &mut self,
        _ctx: &super::RunContext,
        _node_id: egui_snarl::NodeId,
        _inputs: Vec<Option<super::Value>>,
    ) -> Result<Vec<super::Value>, crate::workflow::WorkflowError> {
        let images = self
            .images
            .load()
            .iter()
            .map(|uri| ImageResolver::default().to_rig_image(uri))
            .collect::<anyhow::Result<im::Vector<_>>>()?;

        Ok(vec![Value::Images(images)])
    }
}

#[cfg(feature = "ui")]
impl UiNode for LoadImages {
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
                    ui.set_max_height(3.0 * ui.available_width() * self.images.load().len() as f32);

                    let mut remove_idx = None;
                    for (i, uri) in self.images.load().iter().enumerate() {
                        // TODO: needs optimization
                        let resp = match ImageResolver::default().to_rig_image(uri) {
                            Ok(image) => {
                                let key = rig_image_to_egui(&image);
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
            {
                let images = self.images.clone();
                let errors = ctx.errors.clone();

                spawn_wherever(async move {
                    if let Some(fhs) = rfd::AsyncFileDialog::new()
                        // .set_directory(settings.view(|s| s.last_export_dir.clone())) // TODO: get settings
                        .add_filter("images", &["png", "jpg", "jpeg", "webp"])
                        .add_filter("all", &[""])
                        .pick_files()
                        .await
                    {
                        use futures_util::future::join_all;

                        let results = join_all(fhs.into_iter().map(|fh| async move {
                            let bytes = fh.read().await;
                            ImageResolver::builder()
                                .allow_local(true)
                                .build()
                                .bytes_to_data_uri(&fh.file_name(), Cow::Owned(bytes))
                        }))
                        .await;

                        let mut new_imgs = Vec::new();
                        for result in results {
                            match result {
                                Ok(img) => new_imgs.push(img),
                                Err(err) => errors.push(err),
                            }
                        }

                        for img in &new_imgs {
                            tracing::info!("img {}", &img[..32]);
                        }

                        images.extend(new_imgs);
                    }
                });
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
    fn title(&self) -> &str {
        "Fetch Images"
    }

    fn in_kinds(&'_ self, _in_pin: usize) -> std::borrow::Cow<'_, [super::ValueKind]> {
        std::borrow::Cow::Borrowed(&[ValueKind::TextList, ValueKind::Text])
    }

    fn out_kind(&self, _out_pin: usize) -> ValueKind {
        ValueKind::Images
    }

    #[cfg(feature = "native")]
    fn execute(
        &mut self,
        run_ctx: &super::RunContext,
        _node_id: egui_snarl::NodeId,
        inputs: Vec<Option<Value>>,
    ) -> Result<Vec<Value>, crate::workflow::WorkflowError> {
        let texts = match &inputs[0] {
            Some(Value::Text(text)) => vec![text.to_string()],
            Some(Value::TextList(texts)) => texts.iter().map(|s| s.to_string()).collect(),
            None => vec![],
            _ => unreachable!(),
        };

        let images = texts
            .into_par_iter()
            .map(|uri| {
                if let Some(deadline) = run_ctx.deadline
                    && deadline < Instant::now()
                {
                    Err(WorkflowError::Timeout)?;
                }

                // Unfortunately, most of the time is spent in resizing the image
                // with a compute-bound function lacking cancel support.
                // It's an order of magnitude slower on debug builds but snappy
                // on release builds so not worth the effort to fork.
                ImageResolver::default().to_rig_image(&uri)
            })
            .collect::<anyhow::Result<Vec<_>>>()?;

        let images = im::Vector::from(images);

        Ok(vec![Value::Images(images)])
    }
}

#[cfg(feature = "ui")]
impl UiNode for FetchImages {
    fn tooltip(&self) -> &str {
        "Fetch remote images from URLs"
    }
}

#[cfg(feature = "ui")]
fn media_nodes(ui: &mut egui::Ui, snarl: &mut egui_snarl::Snarl<WorkNode>, pos: egui::Pos2) {
    ui.menu_button("Media", |ui| {
        if ui
            .button("Fetch Images")
            .on_hover_text("Retrieve remote images from URLs (at run time).")
            .clicked()
        {
            snarl.insert_node(pos.into(), FetchImages::default().into());
        }

        if ui
            .button("Load Images")
            .on_hover_text("Load images from disk and embed them into the graph.")
            .clicked()
        {
            snarl.insert_node(pos.into(), LoadImages::default().into());
        }
    });
}

#[cfg(feature = "ui")]
inventory::submit! {
    GraphSubmenu("media", media_nodes)
}
