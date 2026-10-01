//! Benchmarks history operations independently of render latency or source-photo resolution.
//! cargo run --release --no-default-features --example history_performance
use astra_retouch::engine::{Edit, History, Stroke, Target};
use std::{hint::black_box, time::Instant};

fn main() -> anyhow::Result<()> {
    let mut report =
        String::from("strokes,history_steps,record_us,undo_us,redo_us,shared_stroke_bytes\n");
    for count in [1_000usize, 100_000] {
        let mut edit = Edit {
            strokes: (0..count)
                .map(|index| Stroke {
                    target: Target::Heal,
                    center: [
                        (index % 1000) as f32 / 1000.0,
                        (index / 1000) as f32 / 1000.0,
                    ],
                    radius: 0.002,
                    erase: false,
                    softness: 0.65,
                })
                .collect(),
            ..Default::default()
        };
        let mut history = History::default();
        let start = Instant::now();
        for step in 0..10_000 {
            history.record(edit.clone());
            edit.settings.exposure = (step % 100) as f32 / 100.0;
        }
        let record_us = start.elapsed().as_secs_f64() * 1_000_000.0 / 10_000.0;
        assert_eq!(history.depths(), (100, 0));
        let snapshots: Vec<_> = history.snapshots().0.collect();
        assert!(
            snapshots
                .iter()
                .all(|snapshot| snapshot.strokes.same_storage(&edit.strokes))
        );
        let start = Instant::now();
        for _ in 0..100 {
            assert!(history.undo(black_box(&mut edit)));
        }
        let undo_us = start.elapsed().as_secs_f64() * 1_000_000.0 / 100.0;
        let start = Instant::now();
        for _ in 0..100 {
            assert!(history.redo(black_box(&mut edit)));
        }
        let redo_us = start.elapsed().as_secs_f64() * 1_000_000.0 / 100.0;
        let bytes = edit.strokes.len() * std::mem::size_of::<Stroke>();
        println!(
            "{count} strokes / 100 retained steps: record {record_us:.3} µs, undo {undo_us:.3} µs, redo {redo_us:.3} µs; {:.2} MiB shared stroke storage",
            bytes as f64 / 1_048_576.0
        );
        report.push_str(&format!(
            "{count},100,{record_us:.6},{undo_us:.6},{redo_us:.6},{bytes}\n"
        ));
    }
    std::fs::create_dir_all("output/performance")?;
    std::fs::write("output/performance/history.csv", report)?;
    // A new brush gesture changes its shared vector, unlike the scalar-only case above.
    // Include copy-on-write allocation and budget trimming in this separate measurement.
    let mut growing = String::from(
        "initial_strokes,final_strokes,retained_steps,gesture_median_ms,gesture_p95_ms,undo_us,redo_us,history_instruction_bytes\n",
    );
    for count in [1_000usize, 100_000] {
        let mut edit = Edit {
            strokes: (0..count)
                .map(|index| Stroke {
                    target: Target::Heal,
                    center: [(index % 1000) as f32 / 1000.0, 0.5],
                    radius: 0.002,
                    erase: false,
                    softness: 0.65,
                })
                .collect(),
            ..Default::default()
        };
        let mut history = History::default();
        let mut gestures = Vec::with_capacity(200);
        for gesture in 0..200 {
            let start = Instant::now();
            history.record(edit.clone());
            for stamp in 0..16 {
                edit.strokes.push(Stroke {
                    target: Target::Heal,
                    center: [stamp as f32 / 16.0, gesture as f32 / 200.0],
                    radius: 0.002,
                    erase: false,
                    softness: 0.65,
                });
            }
            gestures.push(start.elapsed().as_secs_f64() * 1000.0);
        }
        gestures.sort_by(f64::total_cmp);
        let retained = history.depths().0;
        let bytes = history.instruction_storage_bytes();
        assert!(retained > 0 && bytes <= 64 * 1024 * 1024);
        let final_count = edit.strokes.len();
        let start = Instant::now();
        for _ in 0..retained {
            assert!(history.undo(black_box(&mut edit)));
        }
        let undo_us = start.elapsed().as_secs_f64() * 1_000_000.0 / retained as f64;
        assert_eq!(edit.strokes.len(), final_count - retained * 16);
        let start = Instant::now();
        for _ in 0..retained {
            assert!(history.redo(black_box(&mut edit)));
        }
        let redo_us = start.elapsed().as_secs_f64() * 1_000_000.0 / retained as f64;
        assert_eq!(edit.strokes.len(), final_count);
        println!(
            "Growing {count} to {final_count} strokes / {retained} retained steps: record+16 new stamps median {:.3} ms, p95 {:.3} ms; undo {undo_us:.3} µs, redo {redo_us:.3} µs; {:.2} MiB retained history instructions (current edit/render excluded)",
            gestures[100],
            gestures[190],
            bytes as f64 / 1_048_576.0
        );
        growing.push_str(&format!(
            "{count},{final_count},{retained},{:.6},{:.6},{undo_us:.6},{redo_us:.6},{bytes}\n",
            gestures[100], gestures[190]
        ));
    }
    std::fs::write("output/performance/history-growing.csv", growing)?;
    Ok(())
}
