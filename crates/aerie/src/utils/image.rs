use ::image::ImageFormat;
use anyhow::Context as _;
use cached::proc_macro::cached;
#[cfg(feature = "ui")]
use egui::{Sense, mutex::Mutex};
#[cfg(feature = "ui")]
use lru::LruCache;
use regex::Regex;
use std::{
    borrow::Cow,
    hash::{DefaultHasher, Hash, Hasher},
    io::Cursor,
    path::Path,
    sync::{LazyLock, atomic::AtomicU32},
};
use typed_builder::TypedBuilder;

use crate::rig::{
    self,
    message::{DocumentSourceKind, ImageMediaType, MimeType as _},
};

pub static DATA_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^data:(?<mime>image/\w+);base64,(?<data>[-A-Za-z0-9+/]*={0,3})$").unwrap()
});

pub static MERMAID_MD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?ms)```mermaid(.*)```").unwrap());

pub static MAX_IMAGE_DIM: AtomicU32 = AtomicU32::new(512);

#[cfg(feature = "ui")]
pub static IMAGE_CACHE: LazyLock<Mutex<LruCache<String, egui::ImageSource<'static>>>> =
    LazyLock::new(|| Mutex::new(LruCache::unbounded()));

#[cfg(feature = "ui")]
pub fn prune_image_cache(ctx: &egui::Context) {
    let mut cache = IMAGE_CACHE.lock();
    while cache.len() > 100
        && let Some((_, item)) = cache.pop_lru()
    {
        if let Some(uri) = item.uri() {
            ctx.forget_image(uri);
        }
    }
}

#[cfg(feature = "ui")]
/// Converts a rig image to egui, caching the result and returning a lookup key
pub fn rig_image_to_egui(img: &rig::message::Image) -> String {
    let key = format!("{:x}", image_fingerprint(img));
    let mut cache = IMAGE_CACHE.lock();
    if cache.contains(&key) {
        cache.promote(&key);
    } else if let DocumentSourceKind::Url(url) = &img.data {
        let url = if let Ok(exists) = std::fs::exists(url)
            && exists
        {
            format!("file://{url}")
        } else {
            url.clone()
        };

        tracing::trace!("[{key}] Inserting url to image cache: {url}");
        cache.put(key.clone(), url.into());
    } else if let Some(media) = &img.media_type {
        let data = match &img.data {
            DocumentSourceKind::Base64(data) => {
                use base64::{Engine, prelude::BASE64_STANDARD};
                let bytes = BASE64_STANDARD.decode(data).unwrap();
                egui::ImageSource::from((
                    format!("bytes://{key}.{media:?}").to_lowercase(),
                    bytes.clone(),
                ))
            }
            DocumentSourceKind::Raw(bytes) => egui::ImageSource::from((
                format!("bytes://{key}.{media:?}").to_lowercase(),
                bytes.clone(),
            )),
            _ => todo!(),
        };

        tracing::trace!("[{key}] Inserting bytes to image cache: {:?}", data.uri());
        cache.put(key.clone(), data);
    }
    key
}

pub fn image_fingerprint(img: &rig::message::Image) -> u64 {
    let mut s = DefaultHasher::new();
    match &img.data {
        DocumentSourceKind::Url(url) => url.hash(&mut s),
        DocumentSourceKind::Base64(data) => data.hash(&mut s),
        DocumentSourceKind::Raw(items) => items.hash(&mut s),
        _ => todo!(),
    }

    s.finish()
}

#[derive(TypedBuilder)]
pub struct ImageResolver {
    #[builder(default)]
    pub allow_local: bool,

    #[builder(default_code = r#" {
        let max_dim = MAX_IMAGE_DIM.load(std::sync::atomic::Ordering::Relaxed);
        Some((max_dim, max_dim))
    } "#)]
    pub max_size: Option<(u32, u32)>,
}

impl Default for ImageResolver {
    fn default() -> Self {
        Self::builder().build()
    }
}

impl ImageResolver {
    /// Fetch the image by URL and return a rig Image
    pub fn to_rig_image(&self, url: &str) -> anyhow::Result<rig::message::Image> {
        rig_image_from_path(url, self.allow_local, self.max_size)
    }

    /// Fetch the image by URL and return a base64-encoded data URI
    pub fn to_data_uri(&self, url: &str) -> anyhow::Result<String> {
        data_uri_from_image_path(url, self.allow_local, self.max_size)
    }

    /// Take an existing rig Image, inline remote data nad downsample
    pub fn preprocess<'a>(
        &self,
        image: &'a rig::message::Image,
    ) -> anyhow::Result<Cow<'a, rig::message::Image>> {
        use base64::{Engine, prelude::BASE64_STANDARD};
        match image {
            rig::message::Image {
                data: rig::message::DocumentSourceKind::Raw(bytes),
                media_type,
                ..
            } => {
                let format = media_type
                    .as_ref()
                    .and_then(|m| ImageFormat::from_mime_type(m.to_mime_type()));

                let (image_base64, media_type) = if let Some((w, h)) = self.max_size
                    && let Ok((image_bytes, format)) = downsample_image_bytes(bytes, format, w, h)
                {
                    let media =
                        format.and_then(|m| ImageMediaType::from_mime_type(m.to_mime_type()));
                    (BASE64_STANDARD.encode(&image_bytes), media)
                } else {
                    (BASE64_STANDARD.encode(bytes), media_type.clone())
                };

                Ok(Cow::Owned(rig::message::Image {
                    data: DocumentSourceKind::Base64(image_base64),
                    media_type,
                    detail: None,
                    additional_params: None,
                }))
            }
            rig::message::Image {
                data: rig::message::DocumentSourceKind::Url(url),
                ..
            } => {
                let image = { self.to_rig_image(url) }?;

                Ok(Cow::Owned(image))
            }
            img => Ok(Cow::Borrowed(img)),
        }
    }
}

#[cached(
    result = true,
    key = "String",
    convert = r#"{ format!("{}", path) }"#,
    time = 300,
    time_refresh = true
)]
fn rig_image_from_path(
    path: &str,
    allow_local: bool,
    max_size: Option<(u32, u32)>,
) -> anyhow::Result<rig::message::Image> {
    use base64::{Engine, prelude::BASE64_STANDARD};
    let (image_bytes, format) = resolve_image_to_bytes(path, allow_local)?;
    let (image_bytes, format) = if let Some((w, h)) = max_size {
        downsample_image_bytes(&image_bytes, format, w, h)?
    } else {
        (image_bytes, format)
    };

    let media_type = format
        .map(|f| f.to_mime_type())
        .and_then(ImageMediaType::from_mime_type);
    let image_base64 = BASE64_STANDARD.encode(&image_bytes);
    let image = rig::message::Image {
        data: DocumentSourceKind::Base64(image_base64),
        media_type,
        detail: None,
        additional_params: None,
    };

    Ok(image)
}

#[cached(
    result = true,
    key = "String",
    convert = r#"{ format!("{}", path) }"#,
    time = 300,
    time_refresh = true
)]
fn data_uri_from_image_path(
    path: &str,
    allow_local: bool,
    max_size: Option<(u32, u32)>,
) -> anyhow::Result<String> {
    use base64::{Engine, prelude::BASE64_STANDARD};
    let (image_bytes, format) = resolve_image_to_bytes(path, allow_local)?;
    let (image_bytes, format) = if let Some((w, h)) = max_size {
        downsample_image_bytes(&image_bytes, format, w, h)?
    } else {
        (image_bytes, format)
    };

    let image_base64 = BASE64_STANDARD.encode(&image_bytes);
    let mime_type = format.map(|f| f.to_mime_type()).unwrap_or_default();
    Ok(format!("data:{mime_type};base64,{image_base64}"))
}

fn resolve_image_to_bytes(
    image: &str,
    allow_local: bool,
) -> anyhow::Result<(Vec<u8>, Option<ImageFormat>)> {
    let path = if image.starts_with("file://") {
        image.strip_prefix("file://").unwrap()
    } else {
        image
    };

    let (image_bytes, media_type) = if allow_local
        && let Ok(exists) = std::fs::exists(path)
        && exists
    {
        load_image_file(image)?
    } else if let Ok((image_bytes, media_type)) = parse_data_url(image) {
        (image_bytes, media_type)
    } else if cfg!(feature = "fetch-image")
        && let Ok((image_bytes, mime_type)) = load_image_url(image)
    {
        let media_type = mime_type.and_then(ImageFormat::from_mime_type);
        (image_bytes, media_type)
    } else {
        anyhow::bail!("Unsupported image source: {image}");
    };

    Ok((image_bytes, media_type))
}

#[cfg(feature = "fetch-image")]
#[cached::proc_macro::io_cached(
    disk = true,
    time = 3600,
    time_refresh = true,
    key = "String",
    convert = r#"{ format!("{}", url) }"#,
    map_error = r##"|e| anyhow::Error::from(e)"##
)]
fn load_image_url(url: &str) -> anyhow::Result<(Vec<u8>, Option<String>)> {
    use reqwest::header::CONTENT_TYPE;

    let parsed = url::Url::parse(url)?;
    let resp = reqwest::blocking::get(parsed)?;
    tracing::debug!("HTTP response: {resp:?}");

    let headers = resp.headers();

    let media_type = headers
        .get(CONTENT_TYPE)
        .and_then(|h| h.to_str().ok())
        .map(String::from);

    let image_bytes = resp.bytes()?;

    Ok((image_bytes.to_vec(), media_type))
}

#[cfg(not(feature = "fetch-image"))]
fn load_image_url(_url: &str) -> anyhow::Result<(Vec<u8>, Option<String>)> {
    unreachable!()
}

fn parse_data_url(image: &str) -> anyhow::Result<(Vec<u8>, Option<ImageFormat>)> {
    use base64::{Engine, prelude::BASE64_STANDARD};

    let caps = DATA_URL
        .captures(image)
        .context("Input is not a proper data url image")?;

    let mime_type = caps
        .name("mime")
        .map(|c| c.as_str())
        .context("No mime type detected")?;

    let media_type = ImageFormat::from_mime_type(mime_type);

    let image_base64 = caps.name("data").context("Cannot extract base64 data")?;
    let image_bytes = BASE64_STANDARD.decode(image_base64.as_str())?;

    Ok((image_bytes, media_type))
}

fn load_image_file(image: impl AsRef<Path>) -> anyhow::Result<(Vec<u8>, Option<ImageFormat>)> {
    let path = image.as_ref();
    let image_bytes = std::fs::read(&image)?;

    let media_type = path
        .extension()
        .and_then(std::ffi::OsStr::to_str)
        .and_then(ImageFormat::from_extension);

    Ok((image_bytes, media_type))
}

fn downsample_image_bytes(
    image_bytes: &[u8],
    format: Option<ImageFormat>,
    max_width: u32,
    max_height: u32,
) -> Result<(Vec<u8>, Option<ImageFormat>), anyhow::Error> {
    use image::{
        ImageEncoder, ImageReader, codecs::jpeg::JpegEncoder, imageops::FilterType::Lanczos3,
    };
    let mut image_reader = ImageReader::new(std::io::Cursor::new(image_bytes));
    if let Some(format) = format {
        image_reader.set_format(format);
    }

    let image = image_reader.decode().unwrap();
    let image = if image.width() > max_width || image.height() > max_height {
        tracing::debug!(
            "Downscaling image from {}x{} to {max_width}x{max_height}",
            image.width(),
            image.height()
        );

        image.resize(max_width, max_height, Lanczos3)
    } else if format == Some(ImageFormat::Jpeg) {
        // Avoid re-compression
        return Ok((image_bytes.to_vec(), format));
    } else {
        image
    };

    // Convert to JPEG
    let mut buffer = std::io::BufWriter::new(Vec::new());

    JpegEncoder::new(&mut buffer).write_image(
        image.as_bytes(),
        image.width(),
        image.height(),
        image.color().into(),
    )?;

    // Encode bytes to string
    let image_bytes = buffer.into_inner()?;
    Ok((image_bytes, Some(ImageFormat::Jpeg)))
}

#[cfg(feature = "ui")]
pub fn show_image(ui: &mut egui::Ui, image: &egui::ImageSource<'static>, max_dim: f32) {
    let widget = egui::Image::new(image.clone()).fit_to_exact_size(egui::vec2(max_dim, max_dim));
    let response = ui.add(widget).on_hover_ui(|ui| {
        ui.add(egui::Image::new(image.clone()).max_size(ui.ctx().content_rect().size() * 0.75));
    });

    if let egui::ImageSource::Bytes { bytes, .. } = image {
        response.interact(Sense::CLICK).context_menu(|ui| {
            if ui.button("Save").clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .set_file_name("image.jpg")
                    .add_filter("images", &["png", "jpg", "jpeg", "webp"])
                    .add_filter("all", &[""])
                    .save_file()
                && let Err(e) = save_image_bytes(bytes, &path)
            {
                tracing::warn!("Could not save image: {e:?}");
            }
        });
    }
}

pub fn save_image_bytes(bytes: &[u8], path: impl AsRef<Path>) -> anyhow::Result<()> {
    let image = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()?
        .decode()?;
    image.save(path)?;

    Ok(())
}
