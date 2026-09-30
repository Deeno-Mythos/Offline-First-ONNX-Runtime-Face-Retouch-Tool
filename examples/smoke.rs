use anyhow::Result;
use astra_retouch::engine::{self, Edit, ExportSize};
use std::path::Path;

fn main() -> Result<()> {
    let photo = engine::load_photo(Path::new("assets/demo-portrait.png"))?;
    let source = std::fs::read("assets/demo-portrait.png")?;
    let directory = Path::new("output/smoke");
    let started = std::time::Instant::now();
    let mut report = Vec::new();
    for preset in engine::presets().into_iter().take(3) {
        let edit = Edit {
            settings: preset.settings,
            preset: Some(preset.name.clone()),
            ..Default::default()
        };
        let path = engine::export(
            &photo,
            &edit,
            None,
            directory,
            ExportSize::Original,
            true,
            95,
        )?;
        let exported = engine::load_photo(&path)?;
        assert_eq!(exported.original.dimensions(), photo.original.dimensions());
        assert_ne!(exported.original, photo.original);
        report.push((
            photo.name.clone(),
            path.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            engine::changelog(&edit),
            "Generated adult demo; source unchanged".into(),
        ));
        println!(
            "{}: {} ({} × {})",
            preset.name,
            path.display(),
            exported.original.width(),
            exported.original.height()
        );
    }
    assert_eq!(std::fs::read("assets/demo-portrait.png")?, source);
    println!(
        "Report: {}",
        engine::export_report(directory, &report)?.display()
    );
    println!(
        "Three full-resolution exports verified in {:.2}s. Original unchanged.",
        started.elapsed().as_secs_f32()
    );
    Ok(())
}
