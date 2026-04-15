use ::image::ImageFormat;
use anyhow::Context as _;
use cached::proc_macro::cached;
use egui::mutex::Mutex;
use lru::LruCache;
use regex::Regex;
use std::{
    borrow::Cow,
    hash::{DefaultHasher, Hash, Hasher},
    path::Path,
    sync::{LazyLock, atomic::AtomicU32},
};

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

pub static IMAGE_CACHE: LazyLock<Mutex<LruCache<String, egui::ImageSource<'static>>>> =
    LazyLock::new(|| Mutex::new(LruCache::unbounded()));

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

/// Converts a rig image to egui, caching the result and returning a lookup key
pub fn cache_image(img: &rig::message::Image) -> String {
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

/// Load image into memory and downscale as JPEG base64
pub fn preprocess_image(
    image: &rig::message::Image,
    allow_local: bool,
) -> anyhow::Result<Cow<'_, rig::message::Image>> {
    use base64::{Engine, prelude::BASE64_STANDARD};
    match image {
        img @ rig::message::Image {
            data: rig::message::DocumentSourceKind::Raw(bytes),
            ..
        } => {
            let format = img
                .media_type
                .as_ref()
                .and_then(|m| ImageFormat::from_mime_type(m.to_mime_type()));

            let (image_base64, media_type) = if let Ok((image_bytes, format)) =
                downscale_image(bytes, format)
            {
                let media = format.and_then(|m| ImageMediaType::from_mime_type(m.to_mime_type()));
                (BASE64_STANDARD.encode(&image_bytes), media)
            } else {
                (BASE64_STANDARD.encode(bytes), img.media_type.clone())
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
            let image = image_url_rig(url, allow_local)?;

            Ok(Cow::Owned(image))
        }
        img => Ok(Cow::Borrowed(img)),
    }
}

#[cached(
    result = true,
    key = "String",
    convert = r#"{ format!("{}", url) }"#,
    time = 300,
    time_refresh = true
)]
pub fn image_url_rig(url: &str, allow_local: bool) -> anyhow::Result<rig::message::Image> {
    use base64::{Engine, prelude::BASE64_STANDARD};
    let (image_bytes, format) = resolve_image(url, allow_local)?;
    let (image_bytes, format) = downscale_image(&image_bytes, format)?;
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

pub fn resolve_image(
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

pub fn downscale_image(
    image_bytes: &[u8],
    format: Option<ImageFormat>,
) -> anyhow::Result<(Vec<u8>, Option<ImageFormat>)> {
    use image::{
        ImageEncoder, ImageReader, codecs::jpeg::JpegEncoder, imageops::FilterType::Lanczos3,
    };

    let max_dim = MAX_IMAGE_DIM.load(std::sync::atomic::Ordering::Relaxed);

    let mut image_reader = ImageReader::new(std::io::Cursor::new(image_bytes));
    if let Some(format) = format {
        image_reader.set_format(format);
    }

    let image = image_reader.decode().unwrap();
    let image = if image.width() > max_dim || image.height() > max_dim {
        tracing::debug!(
            "Downscaling image from {}x{} to {max_dim}x{max_dim}",
            image.width(),
            image.height()
        );

        image.resize(max_dim, max_dim, Lanczos3)
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
