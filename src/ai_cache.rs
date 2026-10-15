//! Bounded local cache of registered neural maps; corrupt or outdated caches are ignored.
use crate::shared::SharedVec;
use crate::{engine::Segmentation, geometry::FaceMesh, model};
use anyhow::{Result, ensure};
use image::RgbaImage;
use std::{
    fs::{self, File},
    io::{BufReader, BufWriter, Read, Write},
    path::{Path, PathBuf},
};

const MODELS: &[&str] = &[
    "face_detection_short_range.onnx",
    "face_landmarker_Nx3x256x256.onnx",
    "retouch_generator.onnx",
    "local_detection.onnx",
    "local_inpainting.onnx",
];
// File-format identifiers and the hash seed intentionally stay stable across the rename.
const SIGNATURE: &[u8; 8] = b"ASTRAAI2";
const COMPACT_SIGNATURE: &[u8; 8] = b"ASTRAAI3";
const STAGED_SIGNATURE: &[u8; 8] = b"ASTRAAI4";
const MAX_CACHE_BYTES: u64 = 128 * 1024 * 1024;
struct MemoryEntry {
    path: PathBuf,
    stamp: (u64, std::time::SystemTime),
    seg: Segmentation,
    provider: crate::model::Provider,
    models: u64,
    source: Option<std::sync::Arc<RgbaImage>>,
}
fn model_fingerprint() -> Option<u64> {
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    for name in MODELS {
        let meta = fs::metadata(model::model_path(name)).ok()?;
        meta.len().hash(&mut hash);
        meta.modified().ok()?.hash(&mut hash);
    }
    Some(hash.finish())
}
fn memory() -> &'static std::sync::Mutex<std::collections::VecDeque<MemoryEntry>> {
    static CACHE: std::sync::OnceLock<std::sync::Mutex<std::collections::VecDeque<MemoryEntry>>> =
        std::sync::OnceLock::new();
    CACHE.get_or_init(Default::default)
}
/// Forget retained maps/source buffers without deleting reusable disk results.
pub(crate) fn release_source(image: &std::sync::Arc<RgbaImage>) -> Vec<PathBuf> {
    let Ok(mut cache) = memory().lock() else {
        return vec![];
    };
    let mut paths = Vec::new();
    cache.retain(|entry| {
        let remove = entry
            .source
            .as_ref()
            .is_some_and(|source| std::sync::Arc::ptr_eq(source, image));
        if remove {
            paths.push(entry.path.clone());
        }
        !remove
    });
    paths
}
fn remember(path: PathBuf, seg: &Segmentation, provider: crate::model::Provider) {
    let Ok(meta) = fs::metadata(&path) else {
        return;
    };
    let Ok(modified) = meta.modified() else {
        return;
    };
    let Ok(mut cache) = memory().lock() else {
        return;
    };
    cache.retain(|e| e.path != path);
    cache.push_back(MemoryEntry {
        path,
        stamp: (meta.len(), modified),
        seg: seg.clone(),
        provider,
        models: model_fingerprint().unwrap_or(0),
        source: None,
    });
    while cache.len() > 4
        || cache
            .iter()
            .map(|e| e.seg.map_bytes() + e.source.as_ref().map_or(0, |i| i.as_raw().len()))
            .sum::<usize>()
            > 128 * 1024 * 1024
    {
        cache.pop_front();
    }
}
pub(crate) fn directory() -> PathBuf {
    std::env::var_os("HASTUR_AI_CACHE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::var_os("LOCALAPPDATA")
                .or_else(|| std::env::var_os("XDG_CACHE_HOME"))
                .map(PathBuf::from)
                .unwrap_or_else(std::env::temp_dir)
                .join("HasturRetouch")
                .join("ai-cache")
        })
}
fn cache_paths(current: &Path, legacy_root: &Path, filename: &str) -> [PathBuf; 3] {
    [
        current.join(filename),
        legacy_root.join(".hastur-cache").join(filename),
        legacy_root.join(".astra-cache").join(filename),
    ]
}
fn paths(image: &RgbaImage, provider: crate::model::Provider) -> Option<[PathBuf; 3]> {
    let mut hash = 0xcbf29ce484222325u64;
    let mut update = |data: &[u8]| {
        for b in data {
            hash = (hash ^ *b as u64).wrapping_mul(0x100000001b3);
        }
    };
    update(SIGNATURE);
    update(b"portrait-masks-v3-hair-edge-lower-lid");
    if provider != crate::model::Provider::Cpu {
        update(provider.label().as_bytes());
    }
    update(&image.width().to_le_bytes());
    update(&image.height().to_le_bytes());
    update(image.as_raw());
    for name in MODELS {
        let meta = fs::metadata(model::model_path(name)).ok()?;
        update(&meta.len().to_le_bytes());
        update(
            &meta
                .modified()
                .ok()?
                .duration_since(std::time::UNIX_EPOCH)
                .ok()?
                .as_nanos()
                .to_le_bytes(),
        );
    }
    let model = model::model_path(MODELS[0]);
    let root = model.parent()?.parent()?;
    let filename = format!("{hash:016x}.bin");
    Some(cache_paths(&directory(), root, &filename))
}
pub fn load(image: &RgbaImage, provider: crate::model::Provider) -> Option<Segmentation> {
    load_paths(&paths(image, provider)?, image.dimensions(), provider)
}
fn load_paths(
    paths: &[PathBuf],
    dimensions: (u32, u32),
    provider: crate::model::Provider,
) -> Option<Segmentation> {
    paths
        .iter()
        .find_map(|path| load_path(path, dimensions, provider))
}
fn load_path(
    path: &Path,
    dimensions: (u32, u32),
    provider: crate::model::Provider,
) -> Option<Segmentation> {
    let meta = fs::metadata(path).ok()?;
    let stamp = (meta.len(), meta.modified().ok()?);
    if let Ok(mut cache) = memory().lock()
        && let Some(index) = cache
            .iter()
            .position(|e| e.path == path && e.stamp == stamp)
    {
        let entry = cache.remove(index)?;
        let seg = entry.seg.clone();
        cache.push_back(entry);
        return Some(seg);
    }
    let mut reader = BufReader::new(File::open(path).ok()?);
    let seg = read(&mut reader, dimensions).ok()?;
    remember(path.to_path_buf(), &seg, provider);
    Some(seg)
}
pub(crate) fn associate(
    image: &std::sync::Arc<RgbaImage>,
    seg: &Segmentation,
    provider: crate::model::Provider,
) {
    if let Ok(mut cache) = memory().lock()
        && let Some(entry) = cache.iter_mut().find(|e| {
            e.provider == provider
                && e.seg.skin.same_storage(&seg.skin)
                && e.seg.neural_blend.same_storage(&seg.neural_blend)
        })
    {
        entry.source = Some(image.clone());
        while cache
            .iter()
            .map(|e| e.seg.map_bytes() + e.source.as_ref().map_or(0, |i| i.as_raw().len()))
            .sum::<usize>()
            > 128 * 1024 * 1024
        {
            cache.pop_front();
        }
    }
}
pub(crate) fn load_shared(
    image: &std::sync::Arc<RgbaImage>,
    provider: crate::model::Provider,
) -> Option<Segmentation> {
    let models = model_fingerprint()?;
    if let Ok(mut cache) = memory().lock()
        && let Some(index) = cache.iter().position(|e| {
            e.provider == provider
                && e.models == models
                && e.source
                    .as_ref()
                    .is_some_and(|i| std::sync::Arc::ptr_eq(i, image))
        })
    {
        let entry = &cache[index];
        if let Ok(meta) = fs::metadata(&entry.path)
            && meta
                .modified()
                .is_ok_and(|modified| (meta.len(), modified) == entry.stamp)
        {
            let entry = cache.remove(index)?;
            let seg = entry.seg.clone();
            cache.push_back(entry);
            return Some(seg);
        }
    }
    let seg = load(image, provider)?;
    associate(image, &seg, provider);
    Some(seg)
}
fn number(reader: &mut (impl Read + ?Sized)) -> Result<u32> {
    let mut bytes = [0; 4];
    reader.read_exact(&mut bytes)?;
    Ok(u32::from_le_bytes(bytes))
}
fn values(reader: &mut (impl Read + ?Sized), n: usize) -> Result<Vec<f32>> {
    let mut values = vec![0.0f32; n];
    reader.read_exact(bytemuck::cast_slice_mut(&mut values))?;
    #[cfg(target_endian = "big")]
    for v in &mut values {
        *v = f32::from_bits(v.to_bits().swap_bytes());
    }
    ensure!(values.iter().all(|v| v.is_finite()), "Invalid cache values");
    Ok(values)
}
fn vector(reader: &mut (impl Read + ?Sized), n: usize, full: usize) -> Result<SharedVec<f32>> {
    let count = number(reader)? as usize;
    ensure!(
        count == 0 || count == n || count == full,
        "Invalid cache vector length"
    );
    values(reader, count).map(SharedVec::from)
}
fn rgb(reader: &mut (impl Read + ?Sized), n: usize, full: usize) -> Result<SharedVec<[f32; 3]>> {
    let count = number(reader)? as usize;
    ensure!(
        count == 0 || count == n || count == full,
        "Invalid cache RGB length"
    );
    let mut data = vec![[0.0f32; 3]; count];
    reader.read_exact(bytemuck::cast_slice_mut(&mut data))?;
    #[cfg(target_endian = "big")]
    for v in data.iter_mut().flatten() {
        *v = f32::from_bits(v.to_bits().swap_bytes());
    }
    ensure!(
        data.iter().flatten().all(|v| v.is_finite()),
        "Invalid cache RGB values"
    );
    Ok(data.into())
}
fn read(reader: &mut impl Read, dims: (u32, u32)) -> Result<Segmentation> {
    read_inner(reader, dims, true)
}
fn read_inner(reader: &mut dyn Read, dims: (u32, u32), envelope: bool) -> Result<Segmentation> {
    let mut signature = [0; 8];
    reader.read_exact(&mut signature)?;
    if &signature == STAGED_SIGNATURE {
        ensure!(envelope, "Nested AI cache envelope");
        let mut header = [0; 2];
        reader.read_exact(&mut header)?;
        let [bits, compression] = header;
        ensure!(
            bits > 0 && bits <= 7 && bits & 1 != 0,
            "Invalid cached AI readiness"
        );
        let prepared = crate::nullstate::PortraitDemand {
            geometry: true,
            skin: bits & 2 != 0,
            blemishes: bits & 4 != 0,
        };
        let mut seg = match compression {
            0 => read_inner(&mut reader.take(MAX_CACHE_BYTES + 1), dims, false)?,
            1 => {
                let mut decoder = flate2::bufread::ZlibDecoder::new(BufReader::new(reader));
                let seg = read_inner(&mut decoder.by_ref().take(MAX_CACHE_BYTES + 1), dims, false)?;
                ensure!(decoder.total_out() <= MAX_CACHE_BYTES, "Oversized AI cache");
                let mut rest = decoder.into_inner();
                let mut extra = [0];
                ensure!(
                    rest.read(&mut extra)? == 0,
                    "Unexpected compressed cache payload"
                );
                seg
            }
            _ => anyhow::bail!("Unknown AI cache encoding"),
        };
        ensure!(
            seg.faces.is_empty() || !prepared.skin || !seg.neural_blend.is_empty(),
            "Missing cached skin stage"
        );
        ensure!(
            seg.faces.is_empty() || !prepared.blemishes || !seg.repair_delta.is_empty(),
            "Missing cached repair stage"
        );
        seg.prepared = prepared;
        return Ok(seg);
    }
    ensure!(
        &signature == SIGNATURE || &signature == COMPACT_SIGNATURE,
        "Old cache"
    );
    let (w, h) = (number(reader)?, number(reader)?);
    ensure!(
        (w, h) == dims && w > 0 && h > 0 && u64::from(w) * u64::from(h) <= 2_100_000,
        "Invalid cache dimensions"
    );
    let full = (w * h) as usize;
    let count = number(reader)?;
    ensure!(count <= 64, "Invalid face count");
    let map_crop = if &signature == COMPACT_SIGNATURE {
        let (x, y, cw, ch) = (
            number(reader)?,
            number(reader)?,
            number(reader)?,
            number(reader)?,
        );
        if cw == 0 && ch == 0 {
            None
        } else {
            ensure!(
                cw > 0
                    && ch > 0
                    && u64::from(x) + u64::from(cw) <= u64::from(w)
                    && u64::from(y) + u64::from(ch) <= u64::from(h),
                "Invalid cache crop"
            );
            Some(crate::engine::Crop {
                x,
                y,
                width: cw,
                height: ch,
            })
        }
    } else {
        None
    };
    let n = map_crop.map_or(full, |c| (c.width * c.height) as usize);
    let mut faces = vec![];
    for _ in 0..count {
        let header = values(reader, 5)?;
        let landmarks = values(reader, 478 * 3)?.as_chunks::<3>().0.to_vec();
        faces.push(FaceMesh {
            confidence: header[0],
            bounds: header[1..5].try_into().unwrap(),
            landmarks,
        });
    }
    let len = number(reader)? as usize;
    ensure!(len <= 1024, "Invalid cache status length");
    let mut status = vec![0; len];
    reader.read_exact(&mut status)?;
    let mut seg = Segmentation {
        width: w,
        height: h,
        // Legacy cache writes followed the complete all-or-error portrait pipeline.
        prepared: crate::nullstate::PortraitDemand::ALL,
        map_crop,
        faces,
        status: format!("Cached · {}", String::from_utf8(status)?),
        ..Default::default()
    };
    seg.skin = vector(reader, n, full)?;
    seg.teeth = vector(reader, n, full)?;
    seg.eyes = vector(reader, n, full)?;
    seg.under_eyes = vector(reader, n, full)?;
    seg.forehead = vector(reader, n, full)?;
    seg.laugh_lines = vector(reader, n, full)?;
    seg.contour = vector(reader, n, full)?;
    seg.highlight = vector(reader, n, full)?;
    seg.blemish = vector(reader, n, full)?;
    seg.neural_blend = rgb(reader, n, full)?;
    seg.repair_delta = rgb(reader, n, full)?;
    let mut extra = [0; 1];
    ensure!(reader.read(&mut extra)? == 0, "Unexpected cache payload");
    seg.compact_generated_maps();
    Ok(seg)
}
fn write(writer: &mut impl Write, seg: &Segmentation) -> Result<()> {
    writer.write_all(COMPACT_SIGNATURE)?;
    for n in [seg.width, seg.height, seg.faces.len() as u32] {
        writer.write_all(&n.to_le_bytes())?;
    }
    let crop = seg.map_crop.unwrap_or(crate::engine::Crop {
        x: 0,
        y: 0,
        width: 0,
        height: 0,
    });
    for n in [crop.x, crop.y, crop.width, crop.height] {
        writer.write_all(&n.to_le_bytes())?;
    }
    for face in &seg.faces {
        ensure!(face.landmarks.len() == 478, "Invalid cache mesh");
        for v in std::iter::once(&face.confidence)
            .chain(face.bounds.iter())
            .chain(face.landmarks.iter().flatten())
        {
            writer.write_all(&v.to_le_bytes())?;
        }
    }
    writer.write_all(&(seg.status.len() as u32).to_le_bytes())?;
    writer.write_all(seg.status.as_bytes())?;
    for data in [
        &seg.skin,
        &seg.teeth,
        &seg.eyes,
        &seg.under_eyes,
        &seg.forehead,
        &seg.laugh_lines,
        &seg.contour,
        &seg.highlight,
        &seg.blemish,
    ] {
        writer.write_all(&(data.len() as u32).to_le_bytes())?;
        write_values(writer, data)?;
    }
    for data in [&seg.neural_blend, &seg.repair_delta] {
        writer.write_all(&(data.len() as u32).to_le_bytes())?;
        write_values(writer, bytemuck::cast_slice(data.as_slice()))?;
    }
    Ok(())
}
fn write_values(writer: &mut impl Write, data: &[f32]) -> Result<()> {
    #[cfg(target_endian = "little")]
    writer.write_all(bytemuck::cast_slice(data))?;
    #[cfg(target_endian = "big")]
    for v in data {
        writer.write_all(&v.to_le_bytes())?;
    }
    Ok(())
}
fn write_staged(writer: &mut impl Write, seg: &Segmentation) -> Result<()> {
    ensure!(seg.prepared.geometry, "Cannot cache unprepared geometry");
    let bits = 1 | (u8::from(seg.prepared.skin) << 1) | (u8::from(seg.prepared.blemishes) << 2);
    // Partial maps are cheap to compress and otherwise were recomputed on every
    // reopen. Leave complete maps raw to preserve fast full-cache reads on SSDs.
    let compressed = !seg.prepared.contains(crate::nullstate::PortraitDemand::ALL);
    writer.write_all(STAGED_SIGNATURE)?;
    writer.write_all(&[bits, u8::from(compressed)])?;
    if compressed {
        let mut encoder = flate2::write::ZlibEncoder::new(writer, flate2::Compression::fast());
        write(&mut encoder, seg)?;
        encoder.finish()?;
    } else {
        write(writer, seg)?;
    }
    Ok(())
}
pub fn save(image: &RgbaImage, seg: &Segmentation, provider: crate::model::Provider) -> Result<()> {
    let Some([path, ..]) = paths(image, provider) else {
        return Ok(());
    };
    let dir = path.parent().unwrap();
    fs::create_dir_all(dir)?;
    static SERIAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let serial = SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let temp = path.with_extension(format!("{}.{}.tmp", std::process::id(), serial));
    let result = (|| -> Result<()> {
        let file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        let mut writer = BufWriter::new(file);
        write_staged(&mut writer, seg)?;
        writer.flush()?;
        Ok(())
    })();
    if let Err(error) = result {
        let _ = fs::remove_file(&temp);
        return Err(error);
    }
    // Renaming a completed file makes cancellation or power loss recoverable.
    if path.exists() {
        fs::remove_file(&path)?;
    }
    fs::rename(temp, &path)?;
    remember(path.clone(), seg, provider);
    trim(dir, &path);
    Ok(())
}
fn trim(dir: &Path, current: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|e| e == "bin"))
        .filter_map(|e| {
            e.metadata()
                .ok()
                .map(|m| (e.path(), m.len(), m.modified().ok()))
        })
        .collect();
    entries.sort_by_key(|e| e.2);
    let mut total: u64 = entries.iter().map(|e| e.1).sum();
    for (p, len, _) in entries {
        if total <= 512 * 1024 * 1024 {
            break;
        }
        if p != current && fs::remove_file(p).is_ok() {
            total -= len;
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn staged_cache_keeps_exact_partial_readiness_and_compresses_without_quantizing() {
        for prepared in [
            crate::nullstate::PortraitDemand::GEOMETRY,
            crate::nullstate::PortraitDemand {
                skin: true,
                ..crate::nullstate::PortraitDemand::GEOMETRY
            },
            crate::nullstate::PortraitDemand::ALL,
        ] {
            let seg = Segmentation {
                prepared,
                width: 128,
                height: 128,
                skin: vec![0.34; 128 * 128].into(),
                neural_blend: if prepared.skin {
                    vec![[0.49991, 0.50003, 0.6]; 128 * 128].into()
                } else {
                    Default::default()
                },
                ..Default::default()
            };
            let mut bytes = vec![];
            write_staged(&mut bytes, &seg).unwrap();
            let decoded = read(&mut bytes.as_slice(), (128, 128)).unwrap();
            assert_eq!(decoded.prepared, prepared);
            assert_eq!(decoded.skin, seg.skin);
            assert_eq!(decoded.neural_blend, seg.neural_blend);
            assert!(read(&mut &bytes[..bytes.len() - 1], (128, 128)).is_err());
            let mut extra = bytes.clone();
            extra.push(0);
            assert!(read(&mut extra.as_slice(), (128, 128)).is_err());
            if !prepared.contains(crate::nullstate::PortraitDemand::ALL) {
                let mut raw = vec![];
                write(&mut raw, &seg).unwrap();
                assert!(bytes.len() * 10 < raw.len());
                let last = bytes.len() - 1;
                bytes[last] ^= 1;
                assert!(read(&mut bytes.as_slice(), (128, 128)).is_err());
            }
        }
    }
    #[test]
    fn malformed_readiness_and_recursive_envelopes_are_rejected() {
        for bits in [0, 2, 4, 8, 255] {
            let mut bytes = STAGED_SIGNATURE.to_vec();
            bytes.extend_from_slice(&[bits, 0]);
            assert!(read(&mut bytes.as_slice(), (128, 128)).is_err());
        }
        let mut bytes = STAGED_SIGNATURE.to_vec();
        bytes.extend_from_slice(&[1, 0]);
        bytes.extend_from_slice(STAGED_SIGNATURE);
        assert!(read(&mut bytes.as_slice(), (128, 128)).is_err());
    }
    #[test]
    fn installed_cache_writes_are_separate_from_read_only_legacy_models() {
        let paths = cache_paths(
            Path::new("user/cache"),
            Path::new("installed"),
            "fixture.bin",
        );
        assert_eq!(paths[0], Path::new("user/cache/fixture.bin"));
        assert_eq!(paths[1], Path::new("installed/.hastur-cache/fixture.bin"));
        assert_eq!(paths[2], Path::new("installed/.astra-cache/fixture.bin"));
    }
    #[test]
    fn legacy_cache_is_a_read_only_fallback_and_new_valid_cache_takes_precedence() {
        let directory = tempfile::tempdir().unwrap();
        let paths = [
            directory.path().join(".hastur-cache/fixture.bin"),
            directory.path().join(".astra-cache/fixture.bin"),
        ];
        for path in &paths {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
        }
        let mut legacy = Segmentation {
            width: 3,
            height: 2,
            skin: vec![0.25; 6].into(),
            neural_blend: vec![[0.34, 0.51, 0.82]; 6].into(),
            status: "Cached ONNX CPU ready".into(),
            ..Default::default()
        };
        let mut bytes = Vec::new();
        write(&mut bytes, &legacy).unwrap();
        fs::write(&paths[1], &bytes).unwrap();
        let loaded = load_paths(&paths, (3, 2), model::Provider::Cpu).unwrap();
        assert_eq!(loaded.skin, legacy.skin);
        assert!(!paths[0].exists());
        assert!(
            memory()
                .lock()
                .unwrap()
                .iter()
                .any(|entry| entry.path == paths[1] && entry.seg.skin.same_storage(&loaded.skin))
        );
        fs::write(&paths[0], "interrupted new cache write").unwrap();
        assert_eq!(
            load_paths(&paths, (3, 2), model::Provider::Cpu)
                .unwrap()
                .neural_blend,
            legacy.neural_blend
        );
        legacy.width = 2;
        legacy.height = 3;
        let mut wrong_photo = Vec::new();
        write(&mut wrong_photo, &legacy).unwrap();
        fs::write(&paths[0], wrong_photo).unwrap();
        assert_eq!(
            load_paths(&paths, (3, 2), model::Provider::Cpu)
                .unwrap()
                .skin[0],
            0.25
        );
        legacy.width = 3;
        legacy.height = 2;
        legacy.skin = vec![0.8; 6].into();
        let mut replacement = Vec::new();
        write(&mut replacement, &legacy).unwrap();
        fs::write(&paths[0], replacement).unwrap();
        assert_eq!(
            load_paths(&paths, (3, 2), model::Provider::Cpu)
                .unwrap()
                .skin[0],
            0.8
        );
        assert_eq!(fs::read(&paths[1]).unwrap(), bytes);
    }
    #[test]
    fn cache_keeps_exact_neural_values_and_rejects_truncation_and_wrong_photos() {
        let seg = Segmentation {
            width: 3,
            height: 2,
            skin: vec![0.25; 6].into(),
            neural_blend: vec![[0.34, 0.51, 0.82]; 6].into(),
            repair_delta: vec![[-0.12, 0.1, 0.004]; 6].into(),
            status: "ONNX CPU ready".into(),
            ..Default::default()
        };
        let mut bytes = vec![];
        write(&mut bytes, &seg).unwrap();
        let loaded = read(&mut bytes.as_slice(), (3, 2)).unwrap();
        assert_eq!(loaded.neural_blend, seg.neural_blend);
        assert_eq!(loaded.repair_delta, seg.repair_delta);
        assert_eq!(loaded.skin, seg.skin);
        let mut legacy = bytes.clone();
        legacy[..8].copy_from_slice(SIGNATURE);
        legacy.drain(20..36);
        let loaded = read(&mut legacy.as_slice(), (3, 2)).unwrap();
        assert_eq!(loaded.neural_blend, seg.neural_blend);
        assert!(read(&mut &bytes[..bytes.len() - 1], (3, 2)).is_err());
        assert!(read(&mut bytes.as_slice(), (6, 1)).is_err());
    }
    #[test]
    fn compact_cache_preserves_the_map_rectangle_and_rejects_invalid_coordinates() {
        let mut seg = Segmentation {
            width: 7,
            height: 5,
            skin: vec![0.; 35].into(),
            neural_blend: vec![[0.5; 3]; 35].into(),
            ..Default::default()
        };
        seg.skin[17] = 0.7;
        seg.neural_blend[17] = [0.4, 0.55, 0.6];
        seg.compact_generated_maps();
        let mut bytes = vec![];
        write(&mut bytes, &seg).unwrap();
        let loaded = read(&mut bytes.as_slice(), (7, 5)).unwrap();
        assert_eq!(loaded.map_crop, seg.map_crop);
        assert_eq!(loaded.skin, seg.skin);
        assert_eq!(loaded.neural_blend, seg.neural_blend);
        bytes[20..24].copy_from_slice(&99u32.to_le_bytes());
        assert!(read(&mut bytes.as_slice(), (7, 5)).is_err());
    }
}
