//! Real growing strokes through the cached preview and native-region renderers.
use anyhow::{Result, ensure};
use hastur_retouch::{
    cleanup::CloneStamp,
    engine::{self, Edit, Renderer, Stroke, Target},
    geometry::WarpStroke,
};
use std::{path::Path, sync::Arc, time::Instant};

fn append(edit: &mut Edit, tool: &str, index: usize) {
    let center = [
        0.42 + (index % 16) as f32 * 0.006,
        0.4 + (index / 16) as f32 * 0.008,
    ];
    match tool {
        "liquify" => edit.warps.push(WarpStroke {
            center,
            delta: [0.0008, -0.0004],
            radius: 0.012,
            softness: 0.7,
            strength: 85.0,
        }),
        "heal" => edit.strokes.push(Stroke {
            target: Target::Heal,
            center,
            radius: 0.008,
            softness: 0.7,
            erase: false,
            strength: 100.0,
        }),
        "clone" => edit.clones.push(CloneStamp {
            center,
            source: [center[0] + 0.04, center[1] - 0.04],
            radius: 0.012,
            softness: 0.7,
            strength: 90.0,
        }),
        _ => unreachable!(),
    }
}

fn main() -> Result<()> {
    let reference = std::env::args().any(|a| a == "--reference");
    let root = Path::new("output/live-tools");
    std::fs::create_dir_all(root)?;
    let photo = engine::load_photo(Path::new(
        "example-img/CTU DUMANJUG ORG 09.28.26_JAMESBRO-522.JPG",
    ))?;
    let mut report = String::from("mode,tool,median_ms,p95_ms\n");
    for native in [false, true] {
        let mode = if native { "native" } else { "preview" };
        let source = if native {
            &photo.original
        } else {
            &photo.preview
        };
        let crop = engine::Crop {
            x: source.width() * 42 / 100,
            y: source.height() * 40 / 100,
            width: 512.min(source.width() / 3),
            height: 512.min(source.height() / 3),
        };
        for tool in ["liquify", "heal", "clone"] {
            let mut edit = Edit::default();
            for i in 0..48 {
                append(&mut edit, tool, i);
            }
            let mut renderer = Renderer::default();
            let mut render = |edit: &Edit| -> Result<_> {
                if native {
                    renderer.render_region(source, edit, None, crop, None)
                } else {
                    renderer.render(source, edit, None, None)
                }
            };
            render(&edit)?;
            let mut timings = Vec::new();
            let mut last = Arc::new(image::RgbaImage::new(1, 1));
            for i in 48..64 {
                append(&mut edit, tool, i);
                let start = Instant::now();
                last = Arc::new(render(&edit)?);
                timings.push(start.elapsed().as_secs_f64() * 1000.0);
            }
            timings.sort_by(f64::total_cmp);
            let median = timings[timings.len() / 2];
            let p95 = timings[timings.len() * 95 / 100];
            let baseline = root.join(format!("{mode}-{tool}-reference.png"));
            if reference {
                last.save(baseline)?;
            } else {
                ensure!(
                    *last == image::open(baseline)?.into_rgba8(),
                    "{mode}-{tool}: final pixels changed"
                );
            }
            println!(
                "{mode}-{tool}: median {median:.2} ms, p95 {p95:.2} ms, {}×{}",
                source.width(),
                source.height()
            );
            report.push_str(&format!("{mode},{tool},{median:.3},{p95:.3}\n"));
        }
    }
    std::fs::write(
        root.join(if reference { "before.csv" } else { "after.csv" }),
        report,
    )?;
    Ok(())
}
