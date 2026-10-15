//! Cross-path invariants for the portrait and batch workflow controls.
use crate::{
    engine::{self, Crop, Edit, ExportSize, Segmentation, Stroke, Target},
    skin_tone::{SkinTarget, SkinToneSettings},
};
use image::{Rgba, RgbaImage};
use std::sync::Arc;

fn fixture() -> (Arc<RgbaImage>, Arc<Segmentation>) {
    let (w, h) = (320, 256);
    let image = RgbaImage::from_fn(w, h, |x, y| {
        let crease = if (125..135).contains(&x) { 30 } else { 0 };
        let grain = (((x * 13 + y * 7) % 7) * 4) as u8;
        Rgba([
            165 - crease + grain,
            124 - crease + grain,
            102 - crease + grain,
            255,
        ])
    });
    let n = (w * h) as usize;
    let skin = (0..h)
        .flat_map(|_| (0..w).map(|x| if x < 180 { 1. } else { 0. }))
        .collect::<Vec<_>>();
    (
        Arc::new(image),
        Arc::new(Segmentation {
            width: w,
            height: h,
            skin: skin.into(),
            forehead: vec![0.4; n].into(),
            under_eyes: vec![0.3; n].into(),
            laugh_lines: vec![0.2; n].into(),
            ..Default::default()
        }),
    )
}
fn stroke(target: Target) -> Stroke {
    Stroke {
        target,
        center: [0.4, 0.5],
        radius: 0.12,
        softness: 0.6,
        strength: 100.,
        erase: false,
    }
}
fn close(a: &RgbaImage, b: &RgbaImage) {
    assert_eq!(a.dimensions(), b.dimensions());
    let max = a
        .as_raw()
        .iter()
        .zip(b.as_raw())
        .map(|(a, b)| a.abs_diff(*b))
        .max()
        .unwrap();
    assert!(max <= 1, "native/full mismatch: {max}");
}

#[test]
fn painted_garment_and_lighting_strokes_are_local_reversible_and_match_native_export() {
    let (source, seg) = fixture();
    assert_eq!(
        engine::render(&source, &Edit::default(), Some(&seg)),
        *source
    );
    for target in [Target::Clothing, Target::Dodge, Target::Burn] {
        let mut edit = Edit::default();
        edit.strokes.push(stroke(target));
        let out = engine::render(&source, &edit, Some(&seg));
        assert_ne!(out, *source, "{target:?} has no effect");
        assert_eq!(out.get_pixel(15, 15), source.get_pixel(15, 15));
        assert_eq!(out.get_pixel(285, 200), source.get_pixel(285, 200));
        let crop = Crop {
            x: 82,
            y: 96,
            width: 110,
            height: 74,
        };
        let expected =
            image::imageops::crop_imm(&out, crop.x, crop.y, crop.width, crop.height).to_image();
        let mut renderer = engine::Renderer::default();
        close(
            &renderer
                .render_region(&source, &edit, Some(&seg), crop, None)
                .unwrap(),
            &expected,
        );
        let photo = engine::photo_from_image("fixture.png".into(), None, source.as_ref().clone());
        let dir = tempfile::tempdir().unwrap();
        let export = engine::export(
            &photo,
            &edit,
            Some(&seg),
            dir.path(),
            ExportSize::Original,
            true,
            95,
        )
        .unwrap();
        assert_eq!(image::open(export).unwrap().to_rgba8(), out);
        let mut history = engine::History::default();
        history.record(Edit::default());
        let saved = edit.clone();
        assert!(history.undo(&mut edit));
        assert_eq!(engine::render(&source, &edit, Some(&seg)), *source);
        assert!(history.redo(&mut edit));
        assert_eq!(edit, saved);
        let mut erase = stroke(target);
        erase.erase = true;
        edit.strokes.push(erase);
        assert_ne!(engine::render(&source, &edit, Some(&seg)), out);
        assert_eq!(*source, photo.original.as_ref().clone());
    }
}

#[test]
fn skin_target_and_detail_retention_survive_serialization_disable_and_native_paths() {
    let (source, seg) = fixture();
    let mut edit = Edit::default();
    edit.settings.skin_tone = SkinToneSettings {
        strength: 75.,
        target: Some(SkinTarget {
            name: "reference".into(),
            chroma: [0.02, 0.045],
        }),
    };
    let out = engine::render(&source, &edit, Some(&seg));
    assert_ne!(out.get_pixel(90, 90), source.get_pixel(90, 90));
    assert_eq!(out.get_pixel(260, 100), source.get_pixel(260, 100));
    let loaded: Edit = ron::from_str(&ron::to_string(&edit).unwrap()).unwrap();
    assert_eq!(edit, loaded);
    let crop = Crop {
        x: 80,
        y: 70,
        width: 150,
        height: 100,
    };
    close(
        &engine::render_region(&source, &loaded, Some(&seg), crop, None).unwrap(),
        &image::imageops::crop_imm(&out, crop.x, crop.y, crop.width, crop.height).to_image(),
    );
    edit.settings.disabled.push(engine::Adjustment::SkinTone);
    assert_eq!(engine::render(&source, &edit, Some(&seg)), *source);
    edit.settings.skin_tone = Default::default();
    edit.settings.smoothing = 100.;
    edit.settings.skin_detail_retention = 0.;
    let soft = engine::render(&source, &edit, Some(&seg));
    edit.settings.skin_detail_retention = 100.;
    let texture = engine::render(&source, &edit, Some(&seg));
    let variation = |img: &RgbaImage| {
        (30..80)
            .map(|x| img.get_pixel(x, 80)[0].abs_diff(img.get_pixel(x + 1, 80)[0]) as u32)
            .sum::<u32>()
    };
    assert!(
        variation(&texture) > variation(&soft),
        "detail retention must preserve grain: retained={}, soft={}",
        variation(&texture),
        variation(&soft)
    );
}

#[test]
fn selected_strands_repair_only_painted_lines_preserving_spots_edges_and_alpha() {
    let mut source = RgbaImage::from_pixel(160, 160, Rgba([190, 160, 140, 255]));
    for y in 20..140 {
        source.put_pixel(80, y, Rgba([40, 35, 30, 255]));
    }
    source.put_pixel(84, 82, Rgba([40, 35, 30, 255]));
    for y in 65..95 {
        for x in 90..160 {
            source.put_pixel(x, y, Rgba([40, 35, 30, 255]));
        }
    }
    let mut paint = stroke(Target::StrayHair);
    paint.center = [0.5, 0.5];
    paint.radius = 0.12;
    paint.softness = 0.;
    let map = crate::flyaway::selected(&source, &[paint.clone()], None).unwrap();
    assert_ne!(map.delta(80 * 160 + 80), [0.; 3]);
    assert_eq!(map.delta(30 * 160 + 80), [0.; 3]);
    assert_eq!(map.delta(82 * 160 + 84), [0.; 3], "isolated mark changed");
    assert_eq!(map.delta(80 * 160 + 90), [0.; 3], "broad edge changed");
    let mut erase = paint.clone();
    erase.erase = true;
    assert_eq!(
        crate::flyaway::selected(&source, &[paint.clone(), erase], None)
            .unwrap()
            .correction_count(),
        0
    );
    source.get_pixel_mut(80, 80)[3] = 120;
    assert_eq!(
        crate::flyaway::selected(&source, &[paint], None)
            .unwrap()
            .delta(80 * 160 + 80),
        [0.; 3]
    );
}

#[test]
fn manual_hair_cache_and_layer_visibility_follow_history_without_loading_ai() {
    let mut image = RgbaImage::from_pixel(160, 160, Rgba([180, 140, 120, 255]));
    for y in 10..150 {
        image.put_pixel(65, y, Rgba([35, 25, 20, 255]));
    }
    let image = Arc::new(image);
    let mut edit = Edit::default();
    let mut paint = stroke(Target::StrayHair);
    paint.center = [65.5 / 160., 0.5];
    paint.radius = 0.15;
    edit.strokes.push(paint);
    assert!(edit.portrait_demand().is_empty());
    let out = engine::render(&image, &edit, None);
    assert_ne!(out, *image);
    let crop = Crop {
        x: 40,
        y: 50,
        width: 60,
        height: 55,
    };
    let mut renderer = engine::Renderer::default();
    close(
        &renderer
            .render_region(&image, &edit, None, crop, None)
            .unwrap(),
        &image::imageops::crop_imm(&out, crop.x, crop.y, crop.width, crop.height).to_image(),
    );
    edit.layers
        .get_mut(crate::layers::LayerKind::SpotHeal)
        .visible = false;
    assert_eq!(engine::render(&image, &edit, None), *image);
    assert_eq!(
        renderer
            .render_region(&image, &edit, None, crop, None)
            .unwrap(),
        image::imageops::crop_imm(image.as_ref(), crop.x, crop.y, crop.width, crop.height)
            .to_image()
    );
}
