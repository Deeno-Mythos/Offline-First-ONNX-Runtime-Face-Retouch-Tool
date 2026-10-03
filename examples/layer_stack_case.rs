//! A reviewable real workspace for layer UI and native-resolution validation.
use anyhow::Result;
use hastur_retouch::{
    engine::{self, Edit, Stroke, Target},
    layer_stack::{LayerColor, LayerType},
};
use std::path::Path;

fn main() -> Result<()> {
    let root = Path::new("output/layer-stack");
    std::fs::create_dir_all(root)?;
    let path = std::env::current_dir()?.join("example-img/demo-portrait.png");
    let photo = engine::load_photo(&path)?;
    let mut edit = Edit::default();
    edit.ensure_stack();
    edit.active_layer_mut().unwrap().name = "Clean-up".into();
    edit.active_layer_mut().unwrap().color = LayerColor::Blue;
    edit.clones.push(hastur_retouch::cleanup::CloneStamp {
        center: [0.57, 0.43],
        source: [0.59, 0.45],
        radius: 0.005,
        softness: 0.75,
        strength: 80.0,
    });
    edit.add_layer(LayerType::Adjustment);
    edit.active_layer_mut().unwrap().name = "Curves 1".into();
    edit.active_layer_mut().unwrap().color = LayerColor::Violet;
    edit.active_layer_mut().unwrap().opacity = 70.0;
    edit.settings.color.curves[0][4] = 0.52;
    let heal = edit.add_layer(LayerType::Retouch);
    edit.active_layer_mut().unwrap().name = "Spot heal".into();
    edit.active_layer_mut().unwrap().color = LayerColor::Orange;
    edit.strokes.push(Stroke {
        target: Target::Heal,
        center: [0.62, 0.43],
        radius: 0.005,
        softness: 0.7,
        erase: false,
        strength: 100.0,
    });
    let copy = edit.add_layer(LayerType::OriginalCopy);
    edit.reorder_layer(copy, 1);
    edit.select_layer(heal);
    edit.sync_active_layer();
    let workspace = format!(
        "(version:2,photos:[(path:Some({}),edit:{},rating:0)])",
        ron::to_string(&path)?,
        ron::to_string(&edit)?
    );
    std::fs::write(root.join("review.ron"), workspace)?;
    let output = engine::export(
        &photo,
        &edit,
        None,
        root,
        engine::ExportSize::Original,
        true,
        95,
    )?;
    anyhow::ensure!(
        image::open(&output)?.into_rgba8() == engine::render(&photo.original, &edit, None),
        "Layer export changed native pixels"
    );
    println!(
        "Saved 4 independent layers and exact {}×{} PNG to {}",
        photo.original.width(),
        photo.original.height(),
        root.display()
    );
    Ok(())
}
