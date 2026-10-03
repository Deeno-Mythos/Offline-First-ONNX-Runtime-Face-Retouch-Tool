//! Bounded, color-managed project thumbnails. Original pixels live on disk until opened.
use crate::engine::{self, Photo};
use anyhow::{Context, Result};
use image::{DynamicImage, ImageDecoder, RgbaImage};
use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    sync::Arc,
};

pub const THUMBNAIL_SIDE: u32 = 512;
pub fn thumbnail_dimensions(width: u32, height: u32) -> (u32, u32) {
    let scale = (THUMBNAIL_SIDE as f64 / width.max(height).max(1) as f64).min(1.);
    (
        (width as f64 * scale).round().max(1.) as u32,
        (height as f64 * scale).round().max(1.) as u32,
    )
}

fn cache_path(path: &Path) -> Result<PathBuf> {
    let stamp = std::fs::metadata(path)?;
    let mut hash = DefaultHasher::new();
    // Invalidate older decoders/color handling as well as modified source files.
    ("srgb-oriented-v1", path, stamp.len(), stamp.modified()?).hash(&mut hash);
    Ok(std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("HasturRetouch/asset-thumbnails")
        .join(format!("{:016x}.png", hash.finish())))
}

pub fn purge_thumbnail(path: &Path) {
    if let Ok(path) = cache_path(path) {
        let _ = std::fs::remove_file(path);
    }
}

pub fn stage(path: &Path) -> Result<Photo> {
    let path = std::fs::canonicalize(path)?;
    let reader = image::ImageReader::open(&path)?.with_guessed_format()?;
    let mut decoder = reader.into_decoder()?;
    let (width, height) = decoder.dimensions();
    anyhow::ensure!(
        width > 0 && height > 0 && u64::from(width) * u64::from(height) <= 80_000_000,
        "Image exceeds the 80 megapixel working limit"
    );
    let orientation = decoder.orientation()?;
    let icc = decoder.icc_profile()?;
    let dimensions = match orientation {
        image::metadata::Orientation::Rotate90
        | image::metadata::Orientation::Rotate270
        | image::metadata::Orientation::Rotate90FlipH
        | image::metadata::Orientation::Rotate270FlipH => (height, width),
        _ => (width, height),
    };
    let cache = cache_path(&path)?;
    let cached = image::open(&cache)
        .ok()
        .map(|i| i.into_rgba8())
        .filter(|i| i.dimensions() == thumbnail_dimensions(dimensions.0, dimensions.1));
    let thumbnail = match cached {
        Some(image) => image,
        None => {
            let (w, h) = thumbnail_dimensions(width, height);
            // Windows requests only reduced output pixels from the codec. Full
            // source pixels are reserved for the independent active-photo loader.
            #[cfg(windows)]
            let mut thumb = DynamicImage::ImageRgba8(
                scaled_windows(&path, w, h)
                    .context("System thumbnail decoder could not read this image")?,
            );
            #[cfg(not(windows))]
            let mut thumb = DynamicImage::from_decoder(decoder)?.resize_exact(
                w,
                h,
                image::imageops::FilterType::Lanczos3,
            );
            thumb.apply_orientation(orientation);
            let mut thumb = thumb.into_rgba8();
            engine::convert_to_srgb(&mut thumb, icc.as_deref())?;
            if let Some(parent) = cache.parent() {
                let _ = std::fs::create_dir_all(parent);
                // Atomic replacement prevents simultaneous import/export readers
                // from observing a partial PNG. Failed cache writes are optional.
                static SERIAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
                let serial = SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let pending =
                    cache.with_extension(format!("{}-{serial}.pending", std::process::id()));
                if thumb
                    .save_with_format(&pending, image::ImageFormat::Png)
                    .is_ok()
                {
                    let _ = std::fs::rename(&pending, &cache);
                    let _ = std::fs::remove_file(&pending);
                }
                trim_cache(parent);
            }
            thumb
        }
    };
    let thumbnail = Arc::new(thumbnail);
    Ok(Photo {
        name: path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into(),
        path: Some(path),
        original: thumbnail.clone(),
        preview: thumbnail.clone(),
        analysis: engine::analyze(&thumbnail),
        thumbnail,
        resident: false,
        source_dimensions: dimensions,
    })
}

fn trim_cache(directory: &Path) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    let mut files: Vec<_> = entries
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            if path.extension().is_none_or(|e| e != "png") {
                return None;
            }
            let m = e.metadata().ok()?;
            Some((m.modified().ok()?, m.len(), path))
        })
        .collect();
    let mut bytes: u64 = files.iter().map(|f| f.1).sum();
    files.sort_by_key(|f| f.0);
    for (_, size, path) in files {
        if bytes <= 256 * 1024 * 1024 {
            break;
        }
        if std::fs::remove_file(path).is_ok() {
            bytes -= size;
        }
    }
}

pub fn folder_files(directory: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    // One folder per staging operation; avoiding recursive symlinks prevents
    // duplicates, accidental network walks and unbounded project expansion.
    for entry in std::fs::read_dir(directory)? {
        let path = entry?.path();
        if path.is_file() && supported(&path) {
            files.push(path);
        }
    }
    files.sort();
    Ok(files)
}
pub fn supported(path: &Path) -> bool {
    path.extension().is_some_and(|e| {
        matches!(
            e.to_string_lossy().to_ascii_lowercase().as_str(),
            "jpg" | "jpeg" | "png" | "webp" | "tif" | "tiff" | "bmp"
        )
    })
}

#[cfg(windows)]
fn scaled_windows(path: &Path, width: u32, height: u32) -> Result<RgbaImage> {
    use std::os::windows::ffi::OsStrExt;
    use windows::{
        Win32::{
            Foundation::GENERIC_READ,
            Graphics::Imaging::*,
            System::Com::{
                CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
                CoUninitialize,
            },
        },
        core::PCWSTR,
    };
    struct Apartment;
    impl Drop for Apartment {
        fn drop(&mut self) {
            unsafe { CoUninitialize() }
        }
    }
    // All WIC interfaces stay on the calling worker thread, inside this apartment.
    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
        let _apartment = Apartment;
        let factory: IWICImagingFactory =
            CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let decoder = factory.CreateDecoderFromFilename(
            PCWSTR(wide.as_ptr()),
            None,
            GENERIC_READ,
            WICDecodeMetadataCacheOnDemand,
        )?;
        let frame = decoder.GetFrame(0)?;
        let scaler = factory.CreateBitmapScaler()?;
        scaler.Initialize(&frame, width, height, WICBitmapInterpolationModeFant)?;
        let converter = factory.CreateFormatConverter()?;
        converter.Initialize(
            &scaler,
            &GUID_WICPixelFormat32bppRGBA,
            WICBitmapDitherTypeNone,
            None,
            0.,
            WICBitmapPaletteTypeCustom,
        )?;
        let mut pixels = vec![0; width as usize * height as usize * 4];
        converter.CopyPixels(std::ptr::null(), width * 4, &mut pixels)?;
        RgbaImage::from_raw(width, height, pixels).context("Invalid WIC thumbnail")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn staging_is_bounded_and_export_reopens_lossless_native_pixels() {
        let dir = tempfile::tempdir().unwrap();
        let source = RgbaImage::from_fn(1800, 2400, |x, y| {
            image::Rgba([(x % 253) as u8, (y % 251) as u8, 91, 255])
        });
        let path = dir.path().join("original.png");
        source.save(&path).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let staged = stage(&path).unwrap();
        assert!(!staged.resident);
        assert_eq!(staged.dimensions(), source.dimensions());
        assert_eq!(staged.preview.dimensions(), (384, 512));
        assert!(Arc::ptr_eq(&staged.original, &staged.preview));
        assert!(staged.original.as_raw().len() < 1_048_577);
        let native = staged.open_native().unwrap();
        assert_eq!(*native.original, source);
        let weak = Arc::downgrade(&native.original);
        let mut released = native;
        released.release_pixels();
        assert!(weak.upgrade().is_none());
        let options = engine::ExportOptions {
            png: true,
            size: engine::ExportSize::Original,
            quality: 95,
            center: [0.5; 2],
            cancel: None,
        };
        let exported = engine::export_with_options(
            &staged,
            &engine::Edit::default(),
            None,
            dir.path(),
            &options,
        )
        .unwrap();
        assert_eq!(image::open(exported).unwrap().into_rgba8(), source);
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        purge_thumbnail(staged.path.as_deref().unwrap());
        assert!(
            !cache_path(staged.path.as_deref().unwrap())
                .unwrap()
                .exists()
        );
        assert!(path.exists());
    }
    #[test]
    fn all_import_formats_stage_and_changed_sources_invalidate_thumbnails() {
        let dir = tempfile::tempdir().unwrap();
        for ext in ["png", "jpg", "tif", "bmp", "webp"] {
            let path = dir.path().join(format!("image.{ext}"));
            DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
                1024,
                768,
                image::Rgb([100, 140, 180]),
            ))
            .save(&path)
            .unwrap();
            let photo = stage(&path).unwrap();
            #[cfg(windows)]
            assert!(
                scaled_windows(&path, 512, 384).is_ok(),
                "System scaler unavailable for {ext}"
            );
            assert_eq!(photo.preview.dimensions(), (512, 384), "{ext}");
            let pixel = photo.preview.get_pixel(200, 200);
            assert!(
                pixel[0].abs_diff(100) <= 2
                    && pixel[1].abs_diff(140) <= 2
                    && pixel[2].abs_diff(180) <= 2,
                "{ext}: {pixel:?}"
            );
            let first = cache_path(photo.path.as_deref().unwrap()).unwrap();
            DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
                640,
                480,
                image::Rgb([40, 60, 80]),
            ))
            .save(&path)
            .unwrap();
            let changed = stage(&path).unwrap();
            assert_ne!(cache_path(changed.path.as_deref().unwrap()).unwrap(), first);
            assert_eq!(changed.dimensions(), (640, 480));
            assert!(changed.preview.get_pixel(200, 200)[0].abs_diff(40) <= 2);
            purge_thumbnail(changed.path.as_deref().unwrap());
            let _ = std::fs::remove_file(first);
        }
        assert_eq!(folder_files(dir.path()).unwrap().len(), 5);
    }
}
