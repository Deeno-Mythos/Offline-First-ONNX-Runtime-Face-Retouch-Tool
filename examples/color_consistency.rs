//! Measures source-profile differences between preview and original on actual input photographs.
use astra_retouch::{
    color::{ColorSettings, PreparedColor, ReferenceProfile},
    engine,
};

fn main() -> anyhow::Result<()> {
    for path in std::env::args().skip(1) {
        let photo = engine::load_photo(std::path::Path::new(&path))?;
        let profile = ReferenceProfile::from_image(photo.name.clone(), &photo.original)?;
        for strength in [65.0, 100.0] {
            let settings = ColorSettings {
                reference: Some(profile.clone()),
                reference_strength: strength,
                ..Default::default()
            };
            let preview = PreparedColor::new(&settings, &photo.preview);
            let original = PreparedColor::new(&settings, &photo.original);
            let mut differences = Vec::with_capacity(16384);
            for y in 0..128u64 {
                for x in 0..128u64 {
                    let sx = ((x * 2 + 1) * photo.original.width() as u64 / 256)
                        .min(photo.original.width() as u64 - 1);
                    let sy = ((y * 2 + 1) * photo.original.height() as u64 / 256)
                        .min(photo.original.height() as u64 - 1);
                    let p = photo.original.get_pixel(sx as u32, sy as u32).0;
                    let pixel = [
                        engine::srgb_to_linear(p[0] as f32 / 255.0),
                        engine::srgb_to_linear(p[1] as f32 / 255.0),
                        engine::srgb_to_linear(p[2] as f32 / 255.0),
                        p[3] as f32 / 255.0,
                    ];
                    let a = preview.apply_linear(pixel);
                    let b = original.apply_linear(pixel);
                    let error = (0..3)
                        .map(|c| {
                            (engine::linear_to_srgb(a[c]).clamp(0.0, 1.0)
                                - engine::linear_to_srgb(b[c]).clamp(0.0, 1.0))
                            .abs()
                                * 255.0
                        })
                        .sum::<f32>()
                        / 3.0;
                    differences.push(error);
                }
            }
            differences.sort_by(f32::total_cmp);
            let n = differences.len();
            println!(
                "{}; match {}%; mean RGB byte difference {:.4}; median {:.4}; p95 {:.4}; max {:.4}; original {:?}, preview {:?}",
                photo.name,
                strength,
                differences.iter().sum::<f32>() / n as f32,
                differences[n / 2],
                differences[n * 95 / 100],
                differences[n - 1],
                photo.original.dimensions(),
                photo.preview.dimensions()
            );
        }
    }
    Ok(())
}
