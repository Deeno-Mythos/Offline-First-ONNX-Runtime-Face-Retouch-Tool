//! Offscreen integration measurements from egui pointer input through the actual
//! image workers and completed GPU canvas. Physical monitor refresh is excluded.
use super::*;
use anyhow::{Context, Result, ensure};
use egui::{Event, Modifiers, PointerButton};

struct Studio {
    app: HasturApp,
    ctx: egui::Context,
    gpu: gpu::diagnostics::FrameProfiler,
    start: Instant,
}
fn button(pos: Pos2, pressed: bool) -> Event {
    Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    }
}
impl Studio {
    fn new(path: &std::path::Path, baseline: bool) -> Result<Self> {
        let ctx = egui::Context::default();
        theme(&ctx);
        let (tx, rx) = worker::start(ctx.clone());
        let mut app = HasturApp::from_channels(tx, rx, true, vec!["--hide-panels".into()], None);
        app.sharp_canvas_first = !baseline;
        app.tx.send(Job::DiagnosticsPreviewPolicy(!baseline))?;
        app.tx.send(Job::Import(vec![path.to_path_buf()]))?;
        let mut studio = Self {
            app,
            ctx,
            gpu: gpu::diagnostics::FrameProfiler::new([1440, 940])?,
            start: Instant::now(),
        };
        studio.settle()?;
        Ok(studio)
    }
    fn frame(&mut self, events: Vec<Event>) -> Result<f64> {
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1440.0, 940.0))),
            time: Some(self.start.elapsed().as_secs_f64()),
            events,
            ..Default::default()
        };
        let output = self.ctx.run(input, |ctx| self.app.draw(ctx));
        self.gpu.frame(&self.ctx, output)
    }
    fn rect(&self, key: &str) -> Result<Rect> {
        let viewport = self
            .app
            .canvas_viewport
            .context("Missing canvas viewport")?;
        if key == "canvas" {
            return Ok(viewport);
        }
        let photo = self
            .app
            .photos
            .get(self.app.current)
            .context("No active photo")?;
        let (w, h) = photo.photo.dimensions();
        let mut view = photo.view;
        Ok(view.layout(
            viewport,
            vec2(w as f32, h as f32),
            self.ctx.pixels_per_point(),
        ))
    }
    fn ready(&self) -> bool {
        self.app.photos.get(self.app.current).is_some_and(|p| {
            p.photo.resident
                && p.rendered_revision == p.revision
                && (!needs_detail_resolution(&p.photo, p.view.scale)
                    || p.photo.dimensions() == p.photo.preview.dimensions()
                    || self.app.detail_tile.as_ref().is_some_and(|tile| {
                        self.app.detail_region_pending.is_none()
                            && self
                                .app
                                .detail_region
                                .as_ref()
                                .is_some_and(|(id, rev, crop, _)| {
                                    *id == p.id
                                        && *rev == p.revision
                                        && tile.key.0 == *id
                                        && tile.key.1 == *rev
                                        && tile.key.3 == *crop
                                })
                    }))
        })
    }
    fn settle(&mut self) -> Result<()> {
        let start = Instant::now();
        loop {
            self.frame(vec![])?;
            if self.ready() {
                return Ok(());
            }
            ensure!(
                start.elapsed() < Duration::from_secs(20),
                "Workspace did not settle: photos={}, importing={}, busy={}, state={:?}, toast={:?}",
                self.app.photos.len(),
                self.app.importing,
                self.app.render_busy,
                self.app.photos.first().map(|p| (
                    p.photo.resident,
                    p.revision,
                    p.rendered_revision,
                    p.view.fit,
                    p.view.scale
                )),
                self.app.toast.as_ref().map(|t| &t.0)
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    fn displayed(&self, revision: u64, patch_offset: Option<[f32; 2]>) -> bool {
        if let Some(offset) = patch_offset {
            return self.app.patch_feedback.frame.as_ref().is_some_and(|f| {
                f.revision == revision
                    && f.patch.boundary == self.app.patch_draft.boundary
                    && f.patch
                        .offset
                        .iter()
                        .zip(offset)
                        .all(|(a, b)| (*a - b).abs() < 0.00001)
            });
        }
        if self.app.brush == Some(Target::Patch)
            && self.app.patch_feedback.frame.as_ref().is_some_and(|f| {
                f.committed
                    && f.revision == revision
                    && self.app.photos[0].edit.patches.last() == Some(&f.patch)
            })
        {
            return true;
        }
        let p = &self.app.photos[0];
        if needs_detail_resolution(&p.photo, p.view.scale)
            && p.photo.dimensions() != p.photo.preview.dimensions()
        {
            self.app
                .detail_tile
                .as_ref()
                .is_some_and(|tile| tile.key.0 == p.id && tile.key.1 == revision)
        } else {
            p.rendered_revision == revision
        }
    }
}

/// Diagnostic entry point only; never invoked by the application's render loop.
pub fn profile(path: &std::path::Path, output: &std::path::Path, baseline: bool) -> Result<()> {
    std::fs::create_dir_all(output)?;
    let mut studio = Studio::new(path, baseline)?;
    let mut csv = String::from(
        "tool,view,gesture,sample,input_to_canvas_ready_ms,input_to_gpu_complete_ms,final_gpu_frame_ms\n",
    );
    let mut report = format!(
        "Input: {}\nAdapter: {}\nPointer events -> production egui/worker/canvas -> completed offscreen GPU frame.\nExcludes OS input delivery and physical display refresh.\n",
        path.display(),
        studio.gpu.adapter
    );
    for (view, zoom) in [
        ("fit", None),
        ("62", Some(0.62)),
        ("90", Some(0.9)),
        ("100", Some(1.0)),
    ] {
        for target in [
            Target::Liquify,
            Target::Heal,
            Target::Clone,
            Target::Patch,
            Target::Clothing,
            Target::Dodge,
            Target::Burn,
            Target::StrayHair,
        ] {
            let p = &mut studio.app.photos[0];
            p.edit = Edit::default();
            p.edit.ensure_stack();
            p.revision += 1;
            p.view = View {
                fit: zoom.is_none(),
                scale: zoom.unwrap_or(1.0),
                ..Default::default()
            };
            studio.app.brush = Some(target);
            studio.app.brush_radius = 0.008;
            studio.app.brush_strength = 90.0;
            studio.app.patch_draft = PatchDraft::default();
            studio.app.end_canvas_gesture();
            studio.app.last_edit = Instant::now() - Duration::from_millis(70);
            studio.settle()?;
            if zoom.is_some() {
                let rect = studio.rect("image")?;
                let viewport = studio.rect("canvas")?;
                let face = rect.min + rect.size() * vec2(0.62, 0.33);
                let delta = viewport.center() - face;
                studio.app.photos[0].view.pan = [delta.x, delta.y];
                studio.settle()?;
            }
            let rect = studio.rect("image")?;
            let start = if zoom.is_some() {
                studio.rect("canvas")?.center()
            } else {
                rect.min + rect.size() * vec2(0.62, 0.33)
            };
            let uv = |pos: Pos2| {
                [
                    (pos.x - rect.left()) / rect.width(),
                    (pos.y - rect.top()) / rect.height(),
                ]
            };
            studio.app.clone_anchor = Some((studio.app.photos[0].id, uv(start - vec2(25.0, 12.0))));
            let mut totals = vec![];
            for gesture in ["held", "short"] {
                for sample in 0..8 {
                    studio.settle()?;
                    let pos = start + vec2((sample % 4) as f32 * 3.0, (sample / 4) as f32 * 4.0);
                    if target == Target::Patch {
                        let a = uv(pos - vec2(9.0, 8.0));
                        let b = uv(pos + vec2(9.0, 8.0));
                        studio.app.patch_draft = PatchDraft {
                            boundary: vec![a, [b[0], a[1]], b, [a[0], b[1]]],
                            drawing: false,
                            source_drag: None,
                        };
                    }
                    studio.frame(vec![Event::PointerMoved(pos)])?;
                    studio.frame(vec![button(pos, true)])?;
                    let end = pos + vec2(12.0, 7.0);
                    let started = Instant::now();
                    let mut gpu_ms = studio.frame(vec![Event::PointerMoved(end)])?;
                    if gesture == "short" {
                        gpu_ms = studio.frame(vec![button(end, false)])?;
                    }
                    let revision = studio.app.photos[0].revision;
                    let patch_offset = if target == Target::Patch && gesture == "held" {
                        Some([uv(end)[0] - uv(pos)[0], uv(end)[1] - uv(pos)[1]])
                    } else {
                        None
                    };
                    loop {
                        let displayed = studio.displayed(revision, patch_offset);
                        if displayed {
                            let total = started.elapsed().as_secs_f64() * 1000.0;
                            // UI accepts the completed worker pixels before this frame's GPU stage.
                            let accepted = (total - gpu_ms).max(0.0);
                            csv.push_str(&format!("{},{view},{gesture},{sample},{accepted:.3},{total:.3},{gpu_ms:.3}\n", target.label()));
                            totals.push(total);
                            break;
                        }
                        ensure!(
                            started.elapsed() < Duration::from_secs(8),
                            "No visible {target:?} feedback at {view}/{gesture}"
                        );
                        std::thread::sleep(Duration::from_millis(1));
                        gpu_ms = studio.frame(vec![])?;
                    }
                    if gesture == "held" {
                        studio.frame(vec![button(end, false)])?;
                    }
                }
            }
            totals.sort_by(f64::total_cmp);
            report.push_str(&format!(
                "{} / {view}: median {:.2} ms; p95 {:.2} ms\n",
                target.label(),
                totals[totals.len() / 2],
                totals[totals.len() * 95 / 100]
            ));
            studio.settle()?;
            studio
                .gpu
                .screenshot()?
                .save(output.join(format!("{}-{view}.png", target.label().replace(' ', "-"))))?;
            println!(
                "{} / {view}: {:.2} ms median",
                target.label(),
                totals[totals.len() / 2]
            );
        }
    }
    std::fs::write(output.join("timings.csv"), csv)?;
    std::fs::write(output.join("results.txt"), report)?;
    Ok(())
}
