//! Real egui raw-input integration tests. No screenshot-only assertions.
use super::*;
use egui::{Event, Modifiers, PointerButton};

#[test]
fn color_curves_and_hsl_are_real_reversible_slider_and_drag_edits() {
    let mut h = Harness::new(1);
    h.size = vec2(1440.0, 2800.0);
    h.app.tool = Tool::Color;
    h.app.hsl_band = 0;
    h.frame(vec![], Modifiers::NONE);
    let original = h.app.photos[0].edit.clone();
    let hue_track = h.rect("slider-Hue shift · degrees");
    h.click_at(
        pos2(hue_track.right() - 6.0, hue_track.center().y),
        Modifiers::NONE,
    );
    assert!(h.app.photos[0].edit.settings.color.hsl[0].hue > 50.0);
    h.app.undo(&h.ctx);
    assert_eq!(h.app.photos[0].edit, original);
    h.frame(vec![], Modifiers::NONE);
    let graph = h.rect("curve-graph").shrink(12.0);
    h.drag(
        graph.center(),
        graph.center() - vec2(0.0, graph.height() * 0.25),
    );
    assert!((h.app.photos[0].edit.settings.color.curves[0][4] - 0.75).abs() < 0.01);
    h.app.undo(&h.ctx);
    assert_eq!(h.app.photos[0].edit, original);
    h.app.redo(&h.ctx);
    assert!((h.app.photos[0].edit.settings.color.curves[0][4] - 0.75).abs() < 0.01);
    h.frame(vec![], Modifiers::NONE);
    h.click("enable-Curves amount", Modifiers::NONE);
    assert_eq!(
        h.app.photos[0]
            .edit
            .settings
            .effective()
            .color
            .curves_strength,
        0.0
    );
    assert!(h.app.photos[0].edit.settings.color.curves[0][4] > 0.7);
}

#[test]
fn reference_import_is_async_undoable_and_batch_sync_preserves_other_adjustments() {
    let mut h = Harness::new(3);
    h.app.photos[1].selected = true;
    h.app.photos[1].edit.settings.exposure = 0.4;
    h.app.photos[2].selected = false;
    let profile = crate::color::ReferenceProfile::from_image(
        "Reference portrait".into(),
        &h.app.photos[0].photo.original,
    )
    .unwrap();
    let before = h.app.photos[0].edit.clone();
    let id = h.app.photos[0].id;
    h.app.pending_reference = Some((id, 7));
    h.app
        .reference_sender
        .send((id, 6, Ok(profile.clone())))
        .unwrap();
    h.app.poll_color_reference(&h.ctx);
    assert_eq!(
        h.app.photos[0].edit, before,
        "An obsolete reference request must not replace current edits"
    );
    h.app
        .reference_sender
        .send((id, 7, Ok(profile.clone())))
        .unwrap();
    h.app.poll_color_reference(&h.ctx);
    assert_eq!(
        h.app.photos[0].edit.settings.color.reference,
        Some(profile.clone())
    );
    assert_eq!(h.app.photos[0].edit.settings.color.reference_strength, 65.0);
    h.app.sync_reference();
    assert_eq!(h.app.photos[1].edit.settings.color.reference, Some(profile));
    assert_eq!(h.app.photos[1].edit.settings.exposure, 0.4);
    assert_eq!(h.app.photos[2].edit.settings.color.reference, None);
    h.app.undo(&h.ctx);
    assert_eq!(h.app.photos[0].edit, before);
    h.app.current = 1;
    h.app.undo(&h.ctx);
    assert_eq!(h.app.photos[1].edit.settings.color.reference, None);
    assert_eq!(h.app.photos[1].edit.settings.exposure, 0.4);
}

#[test]
fn native_detail_falls_back_to_the_last_sharp_frame_for_the_same_photo() {
    let sharp = Arc::new(RgbaImage::from_pixel(8, 8, image::Rgba([1, 2, 3, 255])));
    let crop = engine::Crop {
        x: 45,
        y: 26,
        width: 8,
        height: 8,
    };
    let detail = Some((7, 11, crop, sharp.clone()));

    let fallback = sharp_region_for_photo(&detail, 7).unwrap();
    assert!(Arc::ptr_eq(&fallback.3, &sharp));
    assert_eq!(fallback.2, crop);
    assert!(sharp_region_for_photo(&detail, 8).is_none());
    assert!(sharp_region_for_photo(&None, 8).is_none());
}

struct Harness {
    app: AstraApp,
    ctx: egui::Context,
    incoming: Sender<Message>,
    jobs: Receiver<Job>,
    time: f64,
    size: Vec2,
}
impl Harness {
    fn new(count: usize) -> Self {
        let ctx = egui::Context::default();
        theme(&ctx);
        let (tx, jobs) = crossbeam_channel::unbounded();
        let (incoming, rx) = crossbeam_channel::unbounded();
        let app = AstraApp::from_channels(tx, rx, false, vec![], None);
        let mut h = Self {
            app,
            ctx,
            incoming,
            jobs,
            time: 0.0,
            size: vec2(1440.0, 940.0),
        };
        for i in 0..count {
            let image = RgbaImage::from_fn(420, 640, |x, y| {
                image::Rgba([((x + y) % 230) as u8, 80, 120, 255])
            });
            h.incoming
                .send(Message::Imported(Ok(engine::photo_from_image(
                    format!("photo-{i}.png"),
                    Some(PathBuf::from(format!("photo-{i}.png"))),
                    image,
                ))))
                .unwrap();
        }
        h.incoming.send(Message::ImportFinished).unwrap();
        h.frame(vec![], Modifiers::NONE);
        h.app.current = 0;
        h.app.selection_anchor = 0;
        for (i, p) in h.app.photos.iter_mut().enumerate() {
            p.rendered_revision = p.revision;
            p.selected = i == 0;
        }
        h.app.render_busy = false;
        h.jobs.try_iter().for_each(drop);
        h.frame(vec![], Modifiers::NONE);
        h
    }
    fn frame(&mut self, events: Vec<Event>, modifiers: Modifiers) -> egui::FullOutput {
        self.time += 0.04;
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, self.size)),
            time: Some(self.time),
            events,
            modifiers,
            ..Default::default()
        };
        self.ctx.run(input, |ctx| self.app.draw(ctx))
    }
    fn rect(&self, key: &str) -> Rect {
        self.ctx
            .data(|d| d.get_temp(egui::Id::new(("hit", key))))
            .unwrap_or_else(|| panic!("Missing hit target {key}"))
    }
    fn click_at(&mut self, pos: Pos2, modifiers: Modifiers) {
        self.frame(vec![Event::PointerMoved(pos)], modifiers);
        self.frame(
            vec![Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed: true,
                modifiers,
            }],
            modifiers,
        );
        self.frame(
            vec![Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed: false,
                modifiers,
            }],
            modifiers,
        );
    }
    fn click(&mut self, key: &str, modifiers: Modifiers) {
        self.click_at(self.rect(key).center(), modifiers);
    }
    fn drag(&mut self, start: Pos2, end: Pos2) {
        let modifiers = Modifiers::NONE;
        self.frame(vec![Event::PointerMoved(start)], modifiers);
        self.frame(
            vec![Event::PointerButton {
                pos: start,
                button: PointerButton::Primary,
                pressed: true,
                modifiers,
            }],
            modifiers,
        );
        self.frame(vec![Event::PointerMoved(end)], modifiers);
        self.frame(
            vec![Event::PointerButton {
                pos: end,
                button: PointerButton::Primary,
                pressed: false,
                modifiers,
            }],
            modifiers,
        );
    }
    fn drag_path(&mut self, points: &[Pos2]) {
        let Some((&start, rest)) = points.split_first() else {
            return;
        };
        let modifiers = Modifiers::NONE;
        self.frame(vec![Event::PointerMoved(start)], modifiers);
        self.frame(
            vec![Event::PointerButton {
                pos: start,
                button: PointerButton::Primary,
                pressed: true,
                modifiers,
            }],
            modifiers,
        );
        for pos in rest {
            self.frame(vec![Event::PointerMoved(*pos)], modifiers);
        }
        let end = *points.last().unwrap();
        self.frame(
            vec![Event::PointerButton {
                pos: end,
                button: PointerButton::Primary,
                pressed: false,
                modifiers,
            }],
            modifiers,
        );
    }
}

#[test]
fn liquify_and_clone_gestures_are_reversible_and_source_selection_never_paints() {
    let mut h = Harness::new(1);
    h.app.brush = Some(Target::Liquify);
    h.app.brush_radius = 0.1;
    h.frame(vec![], Modifiers::NONE);
    let rect = h.rect("image");
    let center = rect.center();
    h.drag(center, center + vec2(25.0, 10.0));
    assert!(!h.app.photos[0].edit.warps.is_empty());
    assert!(h.app.photos[0].edit.strokes.is_empty());
    let warped = h.app.photos[0].edit.clone();
    let source = crate::geometry::source_uv([0.6, 0.5], &warped, None, (420, 640));
    h.app.brush = Some(Target::Clone);
    let anchor = rect.min + rect.size() * vec2(0.2, 0.3);
    h.click_at(anchor, Modifiers::ALT);
    assert!(h.app.photos[0].edit.clones.is_empty());
    assert_eq!(h.app.photos[0].edit, warped);
    assert!(h.app.clone_anchor.is_some());
    let start = rect.min + rect.size() * vec2(0.6, 0.5);
    h.drag(start, start + vec2(20.0, 5.0));
    let p = &mut h.app.photos[0];
    assert!(p.edit.clones.len() > 1);
    assert!((p.edit.clones[0].center[0] - source[0]).abs() < 0.001);
    let first = &p.edit.clones[0];
    let last = p.edit.clones.last().unwrap();
    for c in 0..2 {
        assert!(
            ((last.source[c] - last.center[c]) - (first.source[c] - first.center[c])).abs()
                < 0.00001
        );
    }
    assert!(p.history.undo(&mut p.edit));
    assert_eq!(p.edit, warped);
    assert!(p.history.undo(&mut p.edit));
    assert!(p.edit.warps.is_empty());
}

#[test]
fn cleanup_tools_track_global_image_coordinates_at_five_zooms_and_four_display_scales() {
    let mut cases = 0;
    let mut stroke_times = Vec::new();
    for dpi in [1.0, 1.25, 1.5, 2.0] {
        for zoom in [0.62, 0.9, 1.0, 2.0, 4.0] {
            for target in [Target::Heal, Target::Liquify, Target::Clone, Target::Patch] {
                let mut h = Harness::new(1);
                h.ctx.set_pixels_per_point(dpi);
                h.app.brush = Some(target);
                h.app.brush_radius = 0.08;
                h.app.photos[0].view = View {
                    fit: false,
                    scale: zoom,
                    pan: [-35.0, 20.0],
                };
                h.app.photos[0]
                    .edit
                    .warps
                    .push(crate::geometry::WarpStroke {
                        center: [0.5, 0.5],
                        delta: [0.025, -0.012],
                        radius: 0.25,
                        softness: 0.7,
                        strength: 65.0,
                    });
                h.frame(vec![], Modifiers::NONE);
                h.frame(vec![], Modifiers::NONE);
                let initial = h.app.photos[0].edit.clone();
                let space = |pressed| Event::Key {
                    key: egui::Key::Space,
                    physical_key: Some(egui::Key::Space),
                    pressed,
                    repeat: false,
                    modifiers: Modifiers::NONE,
                };
                let hand_start = h.rect("image").intersect(h.rect("canvas")).center();
                h.frame(
                    vec![Event::PointerMoved(hand_start), space(true)],
                    Modifiers::NONE,
                );
                h.drag(hand_start, hand_start + vec2(12.0, 15.0));
                h.frame(vec![space(false)], Modifiers::NONE);
                assert_eq!(
                    h.app.photos[0].edit, initial,
                    "Space pan edited {target:?} at {zoom}/{dpi}"
                );
                assert_eq!(h.app.brush, Some(target));
                h.time += 0.7;
                let rect = h.rect("image");
                assert!((rect.width() * dpi / 420.0 - zoom).abs() < 0.0001);
                let start = rect.intersect(h.rect("canvas")).center() - vec2(10.0, 7.0);
                let end = start + rect.size() * vec2(0.035, 0.024);
                let uv = |p: Pos2| {
                    [
                        (p.x - rect.left()) / rect.width(),
                        (p.y - rect.top()) / rect.height(),
                    ]
                };
                let expected_start =
                    crate::geometry::source_uv(uv(start), &initial, None, (420, 640));
                let expected_end = crate::geometry::source_uv(uv(end), &initial, None, (420, 640));
                let output = h.frame(vec![Event::PointerMoved(start)], Modifiers::NONE);
                let expected_radius = 0.08 * 420.0 * zoom / dpi;
                assert!(output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Circle(c)
                    if c.center.distance(start) < 0.01 && (c.radius - expected_radius).abs() < 0.01)),
                    "Cursor footprint wrong at {zoom}/{dpi}");
                let started = Instant::now();
                match target {
                    Target::Heal => {
                        h.drag(start, end);
                        let stamps = &h.app.photos[0].edit.strokes;
                        assert!(!stamps.is_empty());
                        for c in 0..2 {
                            assert!((stamps[0].center[c] - expected_start[c]).abs() < 0.00001);
                            assert!(
                                (stamps.last().unwrap().center[c] - expected_end[c]).abs()
                                    < 0.00001
                            );
                        }
                        assert!(stamps.iter().all(|s| (s.radius - 0.08).abs() < 0.00001));
                    }
                    Target::Liquify => {
                        h.drag(start, end);
                        let stamps = &h.app.photos[0].edit.warps[initial.warps.len()..];
                        assert!(!stamps.is_empty());
                        for c in 0..2 {
                            assert!(
                                (stamps.last().unwrap().center[c] - uv(end)[c]).abs() < 0.00001
                            );
                            let delta: f32 = stamps.iter().map(|s| s.delta[c]).sum();
                            assert!((delta - (uv(end)[c] - uv(start)[c])).abs() < 0.00001);
                        }
                    }
                    Target::Clone => {
                        let source_pos = start - rect.size() * vec2(0.08, 0.04);
                        let expected_source =
                            crate::geometry::source_uv(uv(source_pos), &initial, None, (420, 640));
                        h.click_at(source_pos, Modifiers::ALT);
                        assert_eq!(h.app.photos[0].edit, initial);
                        h.time += 0.7;
                        h.drag(start, end);
                        let stamps = &h.app.photos[0].edit.clones;
                        assert!(!stamps.is_empty());
                        for c in 0..2 {
                            assert!((stamps[0].center[c] - expected_start[c]).abs() < 0.00001);
                            assert!((stamps[0].source[c] - expected_source[c]).abs() < 0.00001);
                            let last = stamps.last().unwrap();
                            assert!((last.center[c] - expected_end[c]).abs() < 0.00001);
                            assert!(
                                ((last.source[c] - last.center[c])
                                    - (expected_source[c] - expected_start[c]))
                                    .abs()
                                    < 0.00001
                            );
                        }
                    }
                    Target::Patch => {
                        let offset = rect.size() * vec2(0.05, 0.04);
                        let corners = [
                            start - offset,
                            start + vec2(offset.x, -offset.y),
                            start + offset,
                            start + vec2(-offset.x, offset.y),
                            start - offset,
                        ];
                        h.drag_path(&corners);
                        assert!(h.app.patch_draft.boundary.len() >= 4);
                        h.time += 0.7;
                        h.drag(start, end);
                        let patches = &h.app.photos[0].edit.patches;
                        assert_eq!(patches.len(), 1);
                        let expected_first =
                            crate::geometry::source_uv(uv(corners[0]), &initial, None, (420, 640));
                        for c in 0..2 {
                            assert!(
                                (patches[0].boundary[0][c] - expected_first[c]).abs() < 0.00001
                            );
                            assert!(
                                (patches[0].offset[c] - (expected_end[c] - expected_start[c]))
                                    .abs()
                                    < 0.00001
                            );
                        }
                    }
                    _ => unreachable!(),
                }
                stroke_times.push(started.elapsed().as_secs_f64() * 1000.0);
                let p = &mut h.app.photos[0];
                assert!(p.history.undo(&mut p.edit));
                assert_eq!(
                    p.edit, initial,
                    "Stroke was not one undo step at {zoom}/{dpi}"
                );
                cases += 1;
            }
        }
    }
    stroke_times.sort_by(f64::total_cmp);
    eprintln!(
        "{cases} raw-egui cleanup cases: complete stroke gestures median {:.2} ms, p95 {:.2} ms",
        stroke_times[40], stroke_times[76]
    );
}

#[test]
fn patch_lasso_drag_commits_source_offset_and_is_one_undo_step() {
    let mut h = Harness::new(1);
    h.app.brush = Some(Target::Patch);
    h.frame(vec![], Modifiers::NONE);
    let rect = h.rect("image");
    let to_pos = |uv: [f32; 2]| rect.min + rect.size() * vec2(uv[0], uv[1]);
    h.drag_path(&[
        to_pos([0.18, 0.2]),
        to_pos([0.32, 0.2]),
        to_pos([0.32, 0.34]),
        to_pos([0.18, 0.34]),
        to_pos([0.18, 0.2]),
    ]);
    assert!(h.app.patch_draft.boundary.len() >= 4);
    assert!(!h.app.patch_draft.drawing);

    h.drag(to_pos([0.25, 0.27]), to_pos([0.55, 0.27]));
    let edit = &mut h.app.photos[0].edit;
    assert_eq!(edit.patches.len(), 1);
    assert!((edit.patches[0].offset[0] - 0.3).abs() < 0.01);
    assert!(h.app.patch_draft.boundary.is_empty());
    let photo = &mut h.app.photos[0];
    assert!(photo.history.undo(&mut photo.edit));
    assert!(h.app.photos[0].edit.patches.is_empty());
}

#[test]
fn space_drag_pans_without_deselecting_or_painting_with_active_cleanup_tool() {
    let mut h = Harness::new(1);
    h.app.brush = Some(Target::Heal);
    h.app.photos[0].view.fit = false;
    h.app.photos[0].view.scale = 2.0;
    h.frame(vec![], Modifiers::NONE);
    let rect = h.rect("image");
    let start = rect.center();
    let end = start + vec2(0.0, 30.0);

    let key = |pressed| Event::Key {
        key: egui::Key::Space,
        physical_key: Some(egui::Key::Space),
        pressed,
        repeat: false,
        modifiers: Modifiers::NONE,
    };
    let output = h.frame(vec![Event::PointerMoved(start), key(true)], Modifiers::NONE);
    assert_eq!(output.platform_output.cursor_icon, egui::CursorIcon::Grab);
    h.frame(
        vec![Event::PointerButton {
            pos: start,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        }],
        Modifiers::NONE,
    );
    let output = h.frame(vec![Event::PointerMoved(end)], Modifiers::NONE);
    assert_eq!(
        output.platform_output.cursor_icon,
        egui::CursorIcon::Grabbing
    );
    h.frame(
        vec![Event::PointerButton {
            pos: end,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        }],
        Modifiers::NONE,
    );
    h.frame(vec![key(false)], Modifiers::NONE);

    assert!(!h.app.photos[0].view.fit);
    assert!(h.app.photos[0].view.pan[1].abs() > 1.0);
    assert_eq!(h.app.brush, Some(Target::Heal));
    assert!(h.app.photos[0].edit.strokes.is_empty());

    let rect = h.rect("image");
    h.drag(rect.center(), rect.center() + vec2(12.0, 2.0));
    assert!(!h.app.photos[0].edit.strokes.is_empty());
    let expected = [
        (rect.center().x - rect.left()) / rect.width(),
        (rect.center().y - rect.top()) / rect.height(),
    ];
    assert!((h.app.photos[0].edit.strokes[0].center[0] - expected[0]).abs() < 0.001);
    assert!((h.app.photos[0].edit.strokes[0].center[1] - expected[1]).abs() < 0.001);
}

#[test]
fn pressing_space_during_a_brush_drag_switches_cleanly_to_hand_pan() {
    let mut h = Harness::new(1);
    h.app.brush = Some(Target::Liquify);
    h.app.photos[0].view.fit = false;
    h.app.photos[0].view.scale = 2.0;
    h.frame(vec![], Modifiers::NONE);
    let rect = h.rect("image");
    let start = rect.center();
    let end = start + vec2(0.0, 30.0);

    h.frame(vec![Event::PointerMoved(start)], Modifiers::NONE);
    h.frame(
        vec![Event::PointerButton {
            pos: start,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        }],
        Modifiers::NONE,
    );
    h.frame(
        vec![
            Event::Key {
                key: egui::Key::Space,
                physical_key: Some(egui::Key::Space),
                pressed: true,
                repeat: false,
                modifiers: Modifiers::NONE,
            },
            Event::PointerMoved(end),
        ],
        Modifiers::NONE,
    );
    h.frame(
        vec![Event::PointerButton {
            pos: end,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        }],
        Modifiers::NONE,
    );
    h.frame(
        vec![Event::Key {
            key: egui::Key::Space,
            physical_key: Some(egui::Key::Space),
            pressed: false,
            repeat: false,
            modifiers: Modifiers::NONE,
        }],
        Modifiers::NONE,
    );

    assert!(h.app.photos[0].view.pan[1].abs() > 1.0);
    assert!(h.app.photos[0].edit.warps.is_empty());
    assert_eq!(h.app.brush, Some(Target::Liquify));
}

#[test]
fn captured_cleanup_strokes_do_not_paint_under_surrounding_panels() {
    for target in [Target::Heal, Target::Liquify, Target::Clone] {
        let mut h = Harness::new(1);
        h.app.brush = Some(target);
        h.app.brush_radius = 0.08;
        h.app.photos[0].view = View {
            scale: 4.0,
            fit: false,
            pan: [0.0; 2],
        };
        h.frame(vec![], Modifiers::NONE);
        let canvas = h.rect("canvas");
        let start = canvas.center();
        let inside = start + vec2(18.0, 12.0);
        let outside = pos2(start.x, canvas.bottom() + 25.0);
        assert!(h.rect("image").contains(outside));
        assert!(!canvas.contains(outside));
        if target == Target::Clone {
            h.click_at(start - vec2(35.0, 20.0), Modifiers::ALT);
        }
        h.frame(vec![Event::PointerMoved(start)], Modifiers::NONE);
        h.frame(
            vec![Event::PointerButton {
                pos: start,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: Modifiers::NONE,
            }],
            Modifiers::NONE,
        );
        h.frame(vec![Event::PointerMoved(inside)], Modifiers::NONE);
        let valid_stroke = h.app.photos[0].edit.clone();
        assert!(h.app.gesture);
        h.frame(vec![Event::PointerMoved(outside)], Modifiers::NONE);
        assert_eq!(h.app.photos[0].edit, valid_stroke, "{target:?}");
        h.frame(
            vec![Event::PointerButton {
                pos: outside,
                button: PointerButton::Primary,
                pressed: false,
                modifiers: Modifiers::NONE,
            }],
            Modifiers::NONE,
        );
        assert_eq!(h.app.photos[0].edit, valid_stroke);
        assert_eq!(h.app.canvas_gesture, Gesture::None);
        assert!(!h.app.gesture);
        assert!(h.app.last_stroke.is_none());
        assert!(h.app.clone_origin.is_none());
        h.app.undo(&h.ctx);
        assert_eq!(h.app.photos[0].edit, Edit::default());
    }
}

#[test]
fn captured_patch_release_uses_only_visible_lasso_and_source_points() {
    let mut h = Harness::new(1);
    h.app.brush = Some(Target::Patch);
    h.app.photos[0].view = View {
        scale: 4.0,
        fit: false,
        pan: [0.0; 2],
    };
    h.frame(vec![], Modifiers::NONE);
    let canvas = h.rect("canvas");
    let rect = h.rect("image");
    let center = canvas.center();
    let outside = pos2(center.x, canvas.bottom() + 25.0);
    assert!(rect.contains(outside) && !canvas.contains(outside));
    let corners = [
        center - vec2(24.0, 24.0),
        center + vec2(24.0, -24.0),
        center + vec2(24.0, 24.0),
        center + vec2(-24.0, 24.0),
    ];
    h.frame(vec![Event::PointerMoved(corners[0])], Modifiers::NONE);
    h.frame(
        vec![Event::PointerButton {
            pos: corners[0],
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        }],
        Modifiers::NONE,
    );
    for corner in &corners[1..] {
        h.frame(vec![Event::PointerMoved(*corner)], Modifiers::NONE);
    }
    let visible_boundary = h.app.patch_draft.boundary.clone();
    h.frame(vec![Event::PointerMoved(outside)], Modifiers::NONE);
    h.frame(
        vec![Event::PointerButton {
            pos: outside,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        }],
        Modifiers::NONE,
    );
    assert_eq!(h.app.patch_draft.boundary, visible_boundary);
    assert!(!h.app.patch_draft.drawing);
    assert_eq!(h.app.canvas_gesture, Gesture::None);
    let visible_end = center + vec2(60.0, 0.0);
    h.frame(vec![Event::PointerMoved(center)], Modifiers::NONE);
    h.frame(
        vec![Event::PointerButton {
            pos: center,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        }],
        Modifiers::NONE,
    );
    h.frame(vec![Event::PointerMoved(visible_end)], Modifiers::NONE);
    h.frame(vec![Event::PointerMoved(outside)], Modifiers::NONE);
    h.frame(
        vec![Event::PointerButton {
            pos: outside,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        }],
        Modifiers::NONE,
    );
    let patches = &h.app.photos[0].edit.patches;
    assert_eq!(patches.len(), 1);
    assert!((patches[0].offset[0] - 60.0 / rect.width()).abs() < 0.00001);
    assert!(patches[0].offset[1].abs() < 0.00001);
    assert_eq!(patches[0].boundary, visible_boundary);
    assert_eq!(h.app.canvas_gesture, Gesture::None);
    assert!(h.app.patch_draft.boundary.is_empty());
}

#[test]
fn escape_ends_a_held_brush_without_panicking_or_adding_strokes() {
    let mut h = Harness::new(1);
    h.app.brush = Some(Target::Heal);
    let start = h.rect("image").intersect(h.rect("canvas")).center();
    h.frame(vec![Event::PointerMoved(start)], Modifiers::NONE);
    h.frame(
        vec![Event::PointerButton {
            pos: start,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        }],
        Modifiers::NONE,
    );
    let painted = h.app.photos[0].edit.clone();
    assert!(h.app.gesture && !painted.strokes.is_empty());
    h.frame(
        vec![Event::Key {
            key: egui::Key::Escape,
            physical_key: Some(egui::Key::Escape),
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }],
        Modifiers::NONE,
    );
    assert!(h.app.brush.is_none());
    assert_eq!(h.app.canvas_gesture, Gesture::None);
    assert!(!h.app.gesture);
    assert!(h.app.last_stroke.is_none() && h.app.clone_origin.is_none());
    h.frame(
        vec![Event::PointerMoved(start + vec2(30.0, 20.0))],
        Modifiers::NONE,
    );
    assert_eq!(h.app.photos[0].edit, painted);
    h.frame(
        vec![Event::PointerButton {
            pos: start,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        }],
        Modifiers::NONE,
    );
    h.app.undo(&h.ctx);
    assert_eq!(h.app.photos[0].edit, Edit::default());
}

#[test]
fn undo_and_redo_end_held_brush_gestures_and_new_strokes_clear_redo() {
    let mut h = Harness::new(1);
    h.app.brush = Some(Target::Heal);
    let start = h.rect("image").intersect(h.rect("canvas")).center();
    let key = |pressed, modifiers| Event::Key {
        key: egui::Key::Z,
        physical_key: Some(egui::Key::Z),
        pressed,
        repeat: false,
        modifiers,
    };
    h.frame(vec![Event::PointerMoved(start)], Modifiers::NONE);
    h.frame(
        vec![Event::PointerButton {
            pos: start,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        }],
        Modifiers::NONE,
    );
    h.frame(
        vec![Event::PointerMoved(start + vec2(20.0, 12.0))],
        Modifiers::NONE,
    );
    let painted = h.app.photos[0].edit.clone();
    assert!(h.app.gesture);
    h.frame(vec![key(true, Modifiers::COMMAND)], Modifiers::COMMAND);
    assert_eq!(h.app.photos[0].edit, Edit::default());
    assert!(h.app.photos[0].history.can_redo());
    assert_eq!(h.app.canvas_gesture, Gesture::None);
    assert!(!h.app.gesture);
    h.frame(
        vec![Event::PointerMoved(start + vec2(40.0, 20.0))],
        Modifiers::NONE,
    );
    assert_eq!(h.app.photos[0].edit, Edit::default());
    h.frame(
        vec![
            key(false, Modifiers::NONE),
            Event::PointerButton {
                pos: start,
                button: PointerButton::Primary,
                pressed: false,
                modifiers: Modifiers::NONE,
            },
        ],
        Modifiers::NONE,
    );
    h.app.brush = Some(Target::Liquify);
    h.frame(vec![Event::PointerMoved(start)], Modifiers::NONE);
    h.frame(
        vec![Event::PointerButton {
            pos: start,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        }],
        Modifiers::NONE,
    );
    assert_eq!(h.app.canvas_gesture, Gesture::Brush);
    let redo_modifiers = Modifiers::COMMAND | Modifiers::SHIFT;
    h.frame(vec![key(true, redo_modifiers)], redo_modifiers);
    assert_eq!(h.app.photos[0].edit, painted);
    assert_eq!(h.app.canvas_gesture, Gesture::None);
    assert!(h.app.last_stroke.is_none());
    h.frame(
        vec![Event::PointerMoved(start + vec2(40.0, 20.0))],
        Modifiers::NONE,
    );
    assert_eq!(h.app.photos[0].edit, painted);
    h.frame(
        vec![
            key(false, Modifiers::NONE),
            Event::PointerButton {
                pos: start,
                button: PointerButton::Primary,
                pressed: false,
                modifiers: Modifiers::NONE,
            },
        ],
        Modifiers::NONE,
    );
    h.app.undo(&h.ctx);
    h.app.brush = Some(Target::Heal);
    h.drag(start - vec2(30.0, 25.0), start - vec2(15.0, 15.0));
    let branched = h.app.photos[0].edit.clone();
    assert!(!h.app.photos[0].history.can_redo());
    assert_ne!(branched, painted);
    h.app.undo(&h.ctx);
    assert_eq!(h.app.photos[0].edit, Edit::default());
    h.app.redo(&h.ctx);
    assert_eq!(h.app.photos[0].edit, branched);
}

#[test]
fn portrait_analysis_updates_masks_and_preview_without_changing_user_edits() {
    let mut h = Harness::new(1);
    let before = h.app.photos[0].edit.clone();
    h.app.photos[0].ai_processing = true;
    let mut seg = Segmentation {
        width: 10,
        height: 10,
        skin: vec![1.0; 100].into(),
        under_eyes: vec![0.5; 100].into(),
        status: "ONNX CPU · face ready".into(),
        ..Default::default()
    };
    let revision = h.app.photos[0].revision;
    h.incoming
        .send(Message::PortraitReady {
            id: 1,
            complete: false,
            result: Ok(seg.clone()),
        })
        .unwrap();
    h.frame(vec![], Modifiers::NONE);
    assert_eq!(h.app.photos[0].edit, before);
    assert!(h.app.photos[0].ai_processing);
    assert!(h.app.photos[0].revision > revision);
    seg.neural_blend = vec![[0.7; 3]; 100].into();
    h.incoming
        .send(Message::PortraitReady {
            id: 1,
            complete: true,
            result: Ok(seg),
        })
        .unwrap();
    h.frame(vec![], Modifiers::NONE);
    assert!(!h.app.photos[0].ai_processing);
    assert!(
        !h.app.photos[0]
            .seg
            .as_ref()
            .unwrap()
            .neural_blend
            .is_empty()
    );
    assert_eq!(h.app.photos[0].edit, before);
}

#[test]
fn auto_retouch_preserves_color_shape_background_and_is_one_undo_step() {
    let mut h = Harness::new(1);
    let before = Edit {
        settings: Settings {
            exposure: 0.8,
            contrast: -23.0,
            shadows: 17.0,
            highlights: -12.0,
            warmth: 19.0,
            tint: 7.0,
            saturation: -8.0,
            sharpening: 31.0,
            vignette: 9.0,
            eye_size: 27.0,
            jawline: 18.0,
            contour: 11.0,
            background: Background::Solid,
            background_color: [10, 20, 30],
            disabled: vec![Adjustment::Contrast, Adjustment::Smoothing],
            ..Default::default()
        },
        preset: Some("Custom look".into()),
        ..Default::default()
    };
    h.app.photos[0].edit = before.clone();
    h.frame(vec![], Modifiers::NONE);
    h.click("auto-retouch", Modifiers::NONE);
    let p = &mut h.app.photos[0];
    assert_eq!(p.edit.settings.smoothing, 45.0);
    assert_eq!(p.edit.settings.blemishes, 55.0);
    assert_eq!(p.edit.settings.under_eyes, 45.0);
    assert_eq!(p.edit.settings.forehead, 28.0);
    let mut preserved = p.edit.settings.clone();
    for adjustment in [
        Adjustment::Smoothing,
        Adjustment::Blemishes,
        Adjustment::UnderEyes,
        Adjustment::Forehead,
        Adjustment::LaughLines,
        Adjustment::ToneEvenness,
        Adjustment::Redness,
    ] {
        *preserved.value_mut(adjustment) = 0.0;
    }
    preserved.disabled.push(Adjustment::Smoothing);
    assert_eq!(preserved, before.settings);
    assert!(p.edit.preset.is_none());
    assert!(p.history.undo(&mut p.edit));
    assert_eq!(p.edit, before);
}

#[test]
fn zoomed_gpu_canvas_uses_the_visible_rectangle_without_resizing_the_image() {
    let mut h = Harness::new(1);
    h.app.gpu = true;
    h.ctx.set_pixels_per_point(1.25);
    h.app.photos[0].view.fit = false;
    for scale in [1.0, 2.0, 4.0, 8.0] {
        h.app.photos[0].view.scale = scale;
        h.frame(vec![], Modifiers::NONE);
        let image = h.rect("image");
        assert!((image.width() / image.height() - 420.0 / 640.0).abs() < 0.0001);
        let paint = h.rect("canvas-paint");
        assert!(h.rect("canvas").contains_rect(paint));
        assert!(Rect::from_min_size(Pos2::ZERO, h.size).contains_rect(paint));
    }
}

#[test]
fn manual_kit_buttons_and_neural_slider_are_reachable_and_change_the_edit() {
    let mut h = Harness::new(1);
    h.click("tool-Liquify", Modifiers::NONE);
    assert_eq!(h.app.brush, Some(Target::Liquify));
    h.click("tool-Clone stamp", Modifiers::NONE);
    assert_eq!(h.app.brush, Some(Target::Clone));
    h.click("tool-Clone stamp", Modifiers::NONE);
    assert!(h.app.brush.is_none());
    h.frame(vec![], Modifiers::NONE);
    h.frame(vec![], Modifiers::NONE);
    h.time += 0.7;
    let track = h.rect("slider-Neural skin smoothing");
    h.click_at(pos2(track.right() - 1.0, track.center().y), Modifiers::NONE);
    assert!(
        h.app.photos[0].edit.settings.smoothing > 95.0,
        "value {}, track {:?}",
        h.app.photos[0].edit.settings.smoothing,
        track
    );
    let p = &mut h.app.photos[0];
    assert!(p.history.undo(&mut p.edit));
    assert_eq!(p.edit.settings.smoothing, 0.0);
}

#[test]
fn native_button_requests_original_and_displays_exact_source_pixels() {
    let mut h = Harness::new(1);
    let source = RgbaImage::from_fn(2000, 2500, |x, y| {
        image::Rgba([if x % 2 == 0 { 255 } else { 0 }, (y % 251) as u8, 83, 255])
    });
    let p = &mut h.app.photos[0];
    p.photo = engine::photo_from_image("large.png".into(), None, source.clone());
    p.edited = p.photo.preview.clone();
    p.edited_texture = texture(&h.ctx, "large-preview", &p.edited);
    h.ctx.set_pixels_per_point(1.25);
    h.frame(vec![], Modifiers::NONE);
    h.frame(vec![], Modifiers::NONE);
    h.frame(vec![], Modifiers::NONE);
    h.click("native", Modifiers::NONE);
    h.frame(vec![], Modifiers::NONE);
    assert_eq!(h.app.photos[0].view.scale, 1.0);
    assert!(h.jobs.try_iter().any(|job|matches!(job,Job::DetailRegion{image,crop,..}
        if image.dimensions()==source.dimensions() && crop.width * crop.height < source.width() * source.height())));
    let (_, _, crop) = h.app.detail_region_pending.unwrap();
    h.incoming
        .send(Message::DetailRegionRendered {
            id: 1,
            revision: 1,
            crop,
            image: image::imageops::crop_imm(&source, crop.x, crop.y, crop.width, crop.height)
                .to_image(),
        })
        .unwrap();
    h.frame(vec![], Modifiers::NONE);
    let tile = h.app.detail_tile.as_ref().unwrap();
    let crop = tile.key.3;
    assert_eq!(
        tile.original.get_pixel(0, 0),
        source.get_pixel(crop.x, crop.y)
    );
    assert_eq!(
        tile.edited.get_pixel(1, 1),
        source.get_pixel(crop.x + 1, crop.y + 1)
    );
    assert!((h.rect("image").width() - 2000.0 / 1.25).abs() < 0.01);
    let serial = h.app.detail_tile.as_ref().unwrap().serial;
    let center = h.rect("canvas").center();
    h.drag(center, center + vec2(24., 10.));
    assert_eq!(
        h.app.detail_tile.as_ref().unwrap().serial,
        serial,
        "small pan reuploaded native tiles"
    );
    h.drag(center, center + vec2(230., 0.));
    assert!(h.app.detail_tile.as_ref().unwrap().serial > serial);
    let tile = h.app.detail_tile.as_ref().unwrap();
    assert_eq!(
        tile.original.get_pixel(0, 0),
        source.get_pixel(tile.key.3.x, tile.key.3.y)
    );
}

#[test]
fn native_detail_waits_for_the_latest_preview_before_starting_a_visible_region_render() {
    let mut h = Harness::new(1);
    let source = RgbaImage::from_fn(1600, 2000, |x, y| {
        image::Rgba([(x % 251) as u8, (y % 241) as u8, 83, 255])
    });
    let photo = engine::photo_from_image("large.png".into(), None, source);
    let preview = photo.preview.clone();
    let p = &mut h.app.photos[0];
    p.photo = photo;
    p.edited = p.photo.preview.clone();
    p.edited_texture = texture(&h.ctx, "large-preview", &p.edited);
    p.revision += 1;
    p.view.fit = false;
    p.view.scale = 1.0;
    let revision = p.revision;
    h.app.last_edit = Instant::now();

    h.frame(vec![], Modifiers::NONE);
    assert!(
        !h.jobs
            .try_iter()
            .any(|job| matches!(job, Job::DetailRegion { .. }))
    );

    h.incoming
        .send(Message::Rendered {
            kind: RenderKind::Preview,
            id: 1,
            revision,
            image: preview.as_ref().clone(),
        })
        .unwrap();
    h.frame(vec![], Modifiers::NONE);
    assert!(h.jobs.try_iter().any(|job| matches!(
        job,
        Job::DetailRegion {
            revision: requested,
            ..
        } if requested == revision
    )));
}

#[test]
fn double_click_toggles_fit_native_and_wheel_and_buttons_share_limits() {
    let mut h = Harness::new(1);
    let pos = h.rect("image").center();
    h.click_at(pos, Modifiers::NONE);
    h.click_at(pos, Modifiers::NONE);
    assert!(!h.app.photos[0].view.fit);
    assert_eq!(h.app.photos[0].view.scale, 1.0);
    h.time += 0.7;
    h.click_at(pos, Modifiers::NONE);
    h.click_at(pos, Modifiers::NONE);
    assert!(h.app.photos[0].view.fit);
    h.app.photos[0].view.fit = false;
    h.app.photos[0].view.scale = 7.99;
    h.click("zoom-plus", Modifiers::NONE);
    assert_eq!(h.app.photos[0].view.scale, View::MAX);
    let pos = h.rect("canvas").center();
    h.frame(
        vec![
            Event::PointerMoved(pos),
            Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: vec2(0.0, 1000.0),
                modifiers: Modifiers::NONE,
            },
        ],
        Modifiers::NONE,
    );
    assert_eq!(h.app.photos[0].view.scale, View::MAX);
    h.app.photos[0].view.scale = View::MIN;
    h.click("zoom-minus", Modifiers::NONE);
    assert_eq!(h.app.photos[0].view.scale, View::MIN);
}

#[test]
fn native_region_cache_survives_small_pans_and_rejects_obsolete_pan_results() {
    let mut h = Harness::new(1);
    let source = RgbaImage::from_pixel(2400, 3200, image::Rgba([80, 95, 110, 255]));
    let p = &mut h.app.photos[0];
    p.photo = engine::photo_from_image("region.png".into(), None, source);
    p.edited = p.photo.preview.clone();
    p.edited_texture = texture(&h.ctx, "region-preview", &p.edited);
    p.view.fit = false;
    p.view.scale = 1.0;
    h.ctx.set_pixels_per_point(1.25);
    h.frame(vec![], Modifiers::NONE);
    h.frame(vec![], Modifiers::NONE);
    h.frame(vec![], Modifiers::NONE);
    let (id, revision, first) = h
        .app
        .detail_region_pending
        .expect("Native display must request a region");
    assert!(
        h.jobs
            .try_iter()
            .any(|job| matches!(job, Job::DetailRegion { crop, .. } if crop == first))
    );
    assert!(first.width * first.height < 2400 * 3200 / 2);
    let first_image =
        RgbaImage::from_pixel(first.width, first.height, image::Rgba([115, 95, 110, 255]));
    h.incoming
        .send(Message::DetailRegionRendered {
            id,
            revision,
            crop: first,
            image: first_image.clone(),
        })
        .unwrap();
    h.frame(vec![], Modifiers::NONE);
    assert!(
        h.app.detail_region.is_some(),
        "Region rejected: pending {:?}, photo revision {}, reply {revision}/{first:?}",
        h.app.detail_region_pending,
        h.app.photos[0].revision
    );
    assert_eq!(
        h.app.detail_tile.as_ref().unwrap().edited.get_pixel(0, 0)[0],
        115,
        "cache {:?}; tile {:?}; visible {:?}",
        h.app.detail_region.as_ref().map(|v| (v.0, v.1, v.2)),
        h.app.detail_tile.as_ref().unwrap().key,
        visible_source_crop(h.rect("image"), h.rect("canvas"), (2400, 3200))
    );
    let serial = h.app.detail_tile.as_ref().unwrap().serial;
    let center = h.rect("canvas").center();
    h.drag(center, center + vec2(20.0, 5.0));
    assert_eq!(h.app.detail_tile.as_ref().unwrap().serial, serial);
    assert!(
        !h.jobs
            .try_iter()
            .any(|j| matches!(j, Job::DetailRegion { .. }))
    );
    h.time += 0.7;
    h.drag(center, center + vec2(360.0, 0.0));
    let (_, _, second) = h
        .app
        .detail_region_pending
        .expect("Pan beyond cache must request a new region");
    assert_ne!(second, first);
    h.incoming
        .send(Message::DetailRegionCancelled {
            id,
            revision,
            crop: first,
        })
        .unwrap();
    h.incoming
        .send(Message::DetailRegionRendered {
            id,
            revision,
            crop: first,
            image: first_image,
        })
        .unwrap();
    h.frame(vec![], Modifiers::NONE);
    assert_eq!(h.app.detail_region_pending, Some((id, revision, second)));
    assert_eq!(h.app.detail_region.as_ref().unwrap().2, first);
    let fallback = h.app.detail_tile.as_ref().unwrap();
    let fallback_key = fallback.key;
    let fallback_serial = fallback.serial;
    let original = fallback.original.clone();
    let original_texture = fallback.original_texture.id();
    let edited_texture = fallback.edited_texture.id();
    h.incoming
        .send(Message::DetailRegionRendered {
            id,
            revision,
            crop: second,
            image: RgbaImage::from_pixel(
                second.width,
                second.height,
                image::Rgba([135, 95, 110, 255]),
            ),
        })
        .unwrap();
    h.frame(vec![], Modifiers::NONE);
    assert_eq!(h.app.detail_region.as_ref().unwrap().2, second);
    let completed = h.app.detail_tile.as_ref().unwrap();
    assert_eq!(completed.key, fallback_key);
    assert!(completed.serial > fallback_serial);
    assert!(Arc::ptr_eq(&completed.original, &original));
    assert_eq!(completed.original_texture.id(), original_texture);
    assert_eq!(completed.edited_texture.id(), edited_texture);
    assert_eq!(
        h.app.detail_tile.as_ref().unwrap().edited.get_pixel(0, 0)[0],
        135
    );
}

#[test]
fn native_preset_hover_requests_a_region_and_keeps_edits_unchanged() {
    let mut h = Harness::new(1);
    let p = &mut h.app.photos[0];
    p.photo = engine::photo_from_image(
        "region.png".into(),
        None,
        RgbaImage::from_pixel(2200, 2800, image::Rgba([100, 95, 110, 255])),
    );
    p.edited = p.photo.preview.clone();
    p.edited_texture = texture(&h.ctx, "hover-region-preview", &p.edited);
    p.view.fit = false;
    p.view.scale = 1.0;
    h.app.tool = Tool::Presets;
    h.frame(vec![], Modifiers::NONE);
    h.frame(vec![], Modifiers::NONE);
    h.frame(vec![], Modifiers::NONE);
    let before = h.app.photos[0].edit.clone();
    let pointer = h.rect("preset-2").center();
    h.frame(vec![Event::PointerMoved(pointer)], Modifiers::NONE);
    let (id, revision, index, crop) = h
        .jobs
        .try_iter()
        .find_map(|job| match job {
            Job::HoverRegion {
                id,
                revision,
                index,
                crop,
                ..
            } => Some((id, revision, index, crop)),
            _ => None,
        })
        .expect("Native preset hover must use a region job");
    assert_eq!(index, 2);
    assert!(crop.width * crop.height < 2200 * 2800 / 2);
    h.incoming
        .send(Message::HoverRegionRendered {
            id,
            revision,
            index,
            crop,
            image: RgbaImage::from_pixel(crop.width, crop.height, image::Rgba([150, 95, 110, 255])),
        })
        .unwrap();
    h.frame(vec![Event::PointerMoved(pointer)], Modifiers::NONE);
    assert_eq!(
        h.app.detail_tile.as_ref().unwrap().key.2,
        Some(2),
        "hover cache {:?}, pending {:?}, pointer index {:?}; tile {:?}",
        h.app.hover_region.as_ref().map(|v| (v.0, v.1, v.2, v.3)),
        h.app.hover_region_pending,
        h.app.hover_preset,
        h.app.detail_tile.as_ref().unwrap().key
    );
    assert_eq!(
        h.app.detail_tile.as_ref().unwrap().edited.get_pixel(0, 0)[0],
        150
    );
    assert_eq!(h.app.photos[0].edit, before);
    let tile = h.app.detail_tile.as_ref().unwrap();
    let first_key = tile.key;
    let first_serial = tile.serial;
    let original = tile.original.clone();
    let original_texture = tile.original_texture.id();
    let edited_texture = tile.edited_texture.id();
    // A new accepted result may replace cached content without changing its edit/crop key.
    h.app.hover_pending = Some((id, revision, index));
    h.app.hover_region_pending = Some((id, revision, index, crop));
    h.incoming
        .send(Message::HoverRegionRendered {
            id,
            revision,
            index,
            crop,
            image: RgbaImage::from_pixel(crop.width, crop.height, image::Rgba([170, 95, 110, 255])),
        })
        .unwrap();
    h.frame(vec![Event::PointerMoved(pointer)], Modifiers::NONE);
    let replacement = h.app.detail_tile.as_ref().unwrap();
    assert_eq!(replacement.key, first_key);
    assert!(replacement.serial > first_serial);
    assert_eq!(replacement.edited.get_pixel(0, 0)[0], 170);
    assert!(Arc::ptr_eq(&replacement.original, &original));
    assert_eq!(replacement.original_texture.id(), original_texture);
    assert_eq!(replacement.edited_texture.id(), edited_texture);
    assert_eq!(h.app.photos[0].edit, before);
}

#[test]
fn divider_drag_never_paints_or_pans_and_drag_ownership_is_stable() {
    let mut h = Harness::new(1);
    h.app.compare = true;
    h.app.brush = Some(Target::Teeth);
    h.frame(vec![], Modifiers::NONE);
    let start = h.rect("divider").center();
    let pan = h.app.photos[0].view.pan;
    h.drag(start, start + vec2(110.0, 30.0));
    assert!(h.app.split > 0.5);
    assert!(h.app.photos[0].edit.strokes.is_empty());
    assert_eq!(h.app.photos[0].view.pan, pan);
}

#[test]
fn fast_brush_stroke_is_continuous_erases_and_undoes_as_one_gesture() {
    let mut h = Harness::new(1);
    h.app.brush = Some(Target::Teeth);
    h.app.brush_softness = 0.0;
    h.app.brush_radius = 0.035;
    h.frame(vec![], Modifiers::NONE);
    let rect = h.rect("image");
    let a = rect.min + rect.size() * vec2(0.2, 0.4);
    let b = rect.min + rect.size() * vec2(0.8, 0.4);
    h.drag(a, b);
    let painted = h.app.photos[0].edit.strokes.len();
    assert!(painted > 40);
    let image = h.app.photos[0].photo.preview.clone();
    let mask = engine::mask_overlay(&image, &h.app.photos[0].edit, None, Target::Teeth);
    for x in 90..330 {
        assert!(mask.get_pixel(x, 256)[3] > 80);
    }
    h.app.erase = true;
    h.time += 0.7;
    h.drag(a, b);
    let mask = engine::mask_overlay(&image, &h.app.photos[0].edit, None, Target::Teeth);
    assert_eq!(mask.get_pixel(210, 256)[3], 0);
    h.app.undo(&h.ctx);
    assert_eq!(h.app.photos[0].edit.strokes.len(), painted);
    h.app.undo(&h.ctx);
    assert!(h.app.photos[0].edit.strokes.is_empty());
}

#[test]
fn continuous_brush_drags_deliver_progressive_previews_and_release_requires_latest_edit() {
    let mut h = Harness::new(1);
    h.app.brush = Some(Target::Heal);
    let start = h.rect("image").center();
    h.frame(vec![Event::PointerMoved(start)], Modifiers::NONE);
    h.frame(
        vec![Event::PointerButton {
            pos: start,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        }],
        Modifiers::NONE,
    );
    h.frame(
        vec![Event::PointerMoved(start + vec2(20.0, 12.0))],
        Modifiers::NONE,
    );
    h.app.last_edit = Instant::now();
    h.app.last_preview_request = Instant::now() - Duration::from_millis(70);
    h.frame(vec![], Modifiers::NONE);
    let (id, intermediate) = h
        .jobs
        .try_iter()
        .find_map(|job| match job {
            Job::Render {
                kind: RenderKind::Preview,
                id,
                revision,
                ..
            } => Some((id, revision)),
            _ => None,
        })
        .expect("A continuously moving brush must request a throttled live preview");
    h.frame(
        vec![Event::PointerMoved(start + vec2(45.0, 28.0))],
        Modifiers::NONE,
    );
    assert!(h.app.photos[0].revision > intermediate);
    assert!(
        !h.app
            .preview_cancel
            .as_ref()
            .unwrap()
            .2
            .load(Ordering::Relaxed),
        "Live snapshots must finish instead of being canceled by every stamp"
    );
    let history_frames = h.app.preview_history.len();
    let dimensions = h.app.photos[0].photo.preview.dimensions();
    h.incoming
        .send(Message::Rendered {
            kind: RenderKind::Preview,
            id,
            revision: intermediate,
            image: RgbaImage::from_pixel(
                dimensions.0,
                dimensions.1,
                image::Rgba([9, 80, 120, 255]),
            ),
        })
        .unwrap();
    h.frame(vec![], Modifiers::NONE);
    assert_eq!(h.app.photos[0].rendered_revision, intermediate);
    assert_eq!(h.app.photos[0].edited.get_pixel(0, 0)[0], 9);
    assert_eq!(
        h.app.preview_history.len(),
        history_frames,
        "An intermediate frame must not be cached under a later edit's undo state"
    );
    let end = start + vec2(65.0, 38.0);
    h.frame(vec![Event::PointerMoved(end)], Modifiers::NONE);
    h.frame(
        vec![Event::PointerButton {
            pos: end,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        }],
        Modifiers::NONE,
    );
    h.incoming
        .send(Message::Rendered {
            kind: RenderKind::Preview,
            id,
            revision: intermediate,
            image: RgbaImage::from_pixel(
                dimensions.0,
                dimensions.1,
                image::Rgba([7, 80, 120, 255]),
            ),
        })
        .unwrap();
    h.frame(vec![], Modifiers::NONE);
    assert_eq!(
        h.app.photos[0].edited.get_pixel(0, 0)[0],
        9,
        "A stale preview received after release must not replace the progressive frame"
    );
    h.app.last_edit = Instant::now() - Duration::from_millis(70);
    h.frame(vec![], Modifiers::NONE);
    let latest = h.app.photos[0].revision;
    assert!(h.jobs.try_iter().any(
        |job| matches!(job, Job::Render { kind: RenderKind::Preview, revision, .. }
        if revision == latest)
    ));
    h.incoming
        .send(Message::Rendered {
            kind: RenderKind::Preview,
            id,
            revision: latest,
            image: RgbaImage::from_pixel(
                dimensions.0,
                dimensions.1,
                image::Rgba([13, 80, 120, 255]),
            ),
        })
        .unwrap();
    h.frame(vec![], Modifiers::NONE);
    assert_eq!(h.app.photos[0].rendered_revision, latest);
    assert_eq!(h.app.photos[0].edited.get_pixel(0, 0)[0], 13);
}

#[test]
fn slider_reset_and_disable_preserve_value_and_change_effect() {
    let mut h = Harness::new(1);
    h.app.tool = Tool::Color;
    h.app.photos[0].edit.settings.exposure = 1.0;
    h.frame(vec![], Modifiers::NONE);
    h.click("enable-Exposure · EV", Modifiers::NONE);
    assert_eq!(h.app.photos[0].edit.settings.exposure, 1.0);
    assert_eq!(h.app.photos[0].edit.settings.effective().exposure, 0.0);
    h.time += 0.7;
    h.click("enable-Exposure · EV", Modifiers::NONE);
    let track = h.rect("slider-Exposure · EV");
    let pos = pos2(track.left() + track.width() * 0.75, track.center().y);
    h.time += 0.7;
    h.click_at(pos, Modifiers::NONE);
    h.click_at(pos, Modifiers::NONE);
    assert_eq!(h.app.photos[0].edit.settings.exposure, 0.0);
}

#[test]
fn shift_selection_and_view_survive_switching_photos() {
    let mut h = Harness::new(5);
    h.app.photos[1].view = View {
        fit: false,
        scale: 2.0,
        pan: [0.0, 25.0],
    };
    h.click("photo-1", Modifiers::NONE);
    h.click("photo-3", Modifiers::SHIFT);
    assert_eq!(h.app.current, 3);
    assert_eq!(
        h.app.photos.iter().map(|p| p.selected).collect::<Vec<_>>(),
        vec![false, true, true, true, false]
    );
    h.click("photo-4", Modifiers::CTRL);
    assert_eq!(h.app.current, 3);
    assert!(h.app.photos[4].selected);
    h.time += 0.7;
    h.click("photo-1", Modifiers::NONE);
    h.frame(vec![], Modifiers::NONE);
    assert_eq!(h.app.photos[1].view.scale, 2.0);
    assert_eq!(h.app.photos[1].view.pan[1], 25.0);
}

#[test]
fn preset_hover_is_temporary_and_thumbnail_jobs_use_real_settings() {
    let mut h = Harness::new(1);
    h.app.tool = Tool::Presets;
    h.frame(vec![], Modifiers::NONE);
    let before = h.app.photos[0].edit.clone();
    assert!(
        h.jobs
            .try_iter()
            .any(|job| matches!(job,Job::Presets{presets,..} if presets[4].settings.exposure<0.0))
    );
    let pos = h.rect("preset-4").center();
    h.frame(vec![Event::PointerMoved(pos)], Modifiers::NONE);
    assert_eq!(h.app.hover_preset, Some(4));
    assert_eq!(h.app.photos[0].edit, before);
    let mut variant = before.clone();
    variant.settings = h.app.presets[4].settings.clone();
    let result = engine::render(&h.app.photos[0].photo.preview, &variant, None);
    assert_ne!(result, *h.app.photos[0].edited);
    h.incoming
        .send(Message::Rendered {
            kind: RenderKind::Hover(4),
            id: 1,
            revision: 1,
            image: result,
        })
        .unwrap();
    let output = h.frame(vec![], Modifiers::NONE);
    let hover_id = h.app.hover_image.as_ref().unwrap().4.id();
    assert!(
        output
            .shapes
            .iter()
            .any(|s| matches!(&s.shape,egui::Shape::Mesh(m) if m.texture_id==hover_id))
    );
    let output = h.frame(
        vec![Event::PointerMoved(h.rect("canvas").center())],
        Modifiers::NONE,
    );
    assert_eq!(h.app.hover_preset, None);
    assert_eq!(h.app.photos[0].edit, before);
    assert!(!h.app.photos[0].history.can_undo());
    assert!(
        !output
            .shapes
            .iter()
            .any(|s| matches!(&s.shape,egui::Shape::Mesh(m) if m.texture_id==hover_id))
    );
}

#[test]
fn crop_drag_matches_engine_geometry_and_cancel_button_signals_worker() {
    let mut h = Harness::new(1);
    h.app.photos[0].photo.original = Arc::new(RgbaImage::new(840, 420));
    h.app.export_size = ExportSize::Story;
    h.app.export_open = true;
    h.frame(vec![], Modifiers::NONE);
    h.frame(vec![], Modifiers::NONE);
    h.frame(vec![], Modifiers::NONE);
    let image = h.rect("crop-preview");
    let start = h.rect("crop-frame").center();
    h.drag(start, start + vec2(40.0, 0.0));
    let center = h.app.photos[0].crop_center;
    assert!(center[0] > 0.6);
    let crop = engine::crop_rect(840, 420, ExportSize::Story, center);
    let expected = image.min.x + crop.x as f32 * image.width() / 840.0;
    assert!((h.rect("crop-frame").left() - expected).abs() < 0.01);
    h.app.export_open = false;
    h.app.exporting = Some((0, 3));
    let cancel = Arc::new(AtomicBool::new(false));
    h.app.export_cancel = Some(cancel.clone());
    h.frame(vec![], Modifiers::NONE);
    h.click("cancel-export", Modifiers::NONE);
    assert!(cancel.load(Ordering::Relaxed));
    h.incoming
        .send(Message::ExportFinished {
            finished: 0,
            total: 3,
            cancelled: true,
        })
        .unwrap();
    h.frame(vec![], Modifiers::NONE);
    assert!(h.app.exporting.is_none());
    assert!(h.app.export_cancel.is_none());
}

#[test]
fn batch_scheduler_updates_unopened_photos_and_rejects_stale_results() {
    let mut h = Harness::new(3);
    h.app.photos.iter_mut().for_each(|p| p.selected = true);
    h.app.photos[0].edit.settings.exposure = 0.3;
    h.app.sync();
    h.app.last_edit = Instant::now() - Duration::from_secs(1);
    h.frame(vec![], Modifiers::NONE);
    let job = h.jobs.try_recv().unwrap();
    assert!(matches!(
        job,
        Job::Render {
            kind: RenderKind::Preview,
            id: 2,
            revision: 2,
            ..
        }
    ));
    h.incoming
        .send(Message::RenderStarted {
            kind: RenderKind::Preview,
            id: 2,
            revision: 2,
        })
        .unwrap();
    h.frame(vec![], Modifiers::NONE);
    assert_eq!(h.app.photos[1].processing, Some(2));
    h.incoming
        .send(Message::Rendered {
            kind: RenderKind::Preview,
            id: 2,
            revision: 1,
            image: RgbaImage::new(420, 640),
        })
        .unwrap();
    h.frame(vec![], Modifiers::NONE);
    assert_ne!(h.app.photos[1].rendered_revision, 2);
    h.incoming
        .send(Message::Rendered {
            kind: RenderKind::Preview,
            id: 2,
            revision: 2,
            image: RgbaImage::new(420, 640),
        })
        .unwrap();
    h.frame(vec![], Modifiers::NONE);
    assert_eq!(h.app.photos[1].rendered_revision, 2);
    assert!(
        h.jobs
            .try_iter()
            .any(|j| matches!(j, Job::Render { id: 3, .. }))
    );
}

#[test]
fn session_restores_view_crop_disabled_adjustments_and_old_defaults() {
    let mut h = Harness::new(1);
    let mut edit = Edit::default();
    edit.settings.exposure = 0.8;
    edit.settings.disabled.push(Adjustment::Exposure);
    let saved = SavedPhoto {
        path: None,
        edit: edit.clone(),
        rating: 4,
        segmentation: None,
        view: View {
            fit: false,
            scale: 1.25,
            pan: [0.0, 21.0],
        },
        crop_center: [0.6, 0.4],
        history: History::default(),
        selected: true,
    };
    let text = ron::ser::to_string(&saved).unwrap();
    let restored: SavedPhoto = ron::from_str(&text).unwrap();
    assert_eq!(restored.view.scale, 1.25);
    assert_eq!(restored.crop_center, [0.6, 0.4]);
    assert_eq!(restored.edit, edit);
    let old = format!(
        "(path:None,edit:{},rating:0)",
        ron::ser::to_string(&Edit::default()).unwrap()
    );
    let restored: SavedPhoto = ron::from_str(&old).unwrap();
    assert!(restored.view.fit);
    assert_eq!(restored.crop_center, [0.5; 2]);
    h.app.photos[0].crop_center = [0.65, 0.5];
    h.app.export_open = true;
    h.app.export_size = ExportSize::Original;
    h.frame(vec![], Modifiers::NONE);
    assert_eq!(h.app.photos[0].crop_center, [0.65, 0.5]);
}

#[test]
fn opening_a_session_restores_each_photo_history_and_active_selection() {
    let mut h = Harness::new(2);
    h.app.current = 0;
    h.app.compare = true;
    h.app.split = 0.37;
    for (index, photo) in h.app.photos.iter_mut().enumerate() {
        photo.history.record(photo.edit.clone());
        photo.edit.settings.exposure = index as f32 + 0.25;
        photo.history.record(photo.edit.clone());
        photo.edit.settings.exposure = index as f32 + 0.5;
        assert!(photo.history.undo(&mut photo.edit));
        photo.selected = index == 0;
        photo.rating = index as u8 + 3;
    }
    let originals: Vec<_> = h.app.photos.iter().map(|p| p.photo.clone()).collect();
    let session = h.app.session_snapshot(false);
    h.incoming.send(Message::SessionRead(Ok(session))).unwrap();
    h.frame(vec![], Modifiers::NONE);
    let open = h
        .jobs
        .try_iter()
        .find_map(|job| match job {
            Job::OpenSession { photos } => Some(photos),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        open,
        originals.iter().map(|p| p.path.clone()).collect::<Vec<_>>()
    );
    for photo in originals {
        h.incoming.send(Message::Imported(Ok(photo))).unwrap();
    }
    h.incoming.send(Message::ImportFinished).unwrap();
    h.frame(vec![], Modifiers::NONE);
    assert_eq!(h.app.current, 0);
    assert_eq!(h.app.split, 0.37);
    assert!(h.app.compare);
    assert_eq!(h.app.photos[0].rating, 3);
    assert!(h.app.photos[0].selected);
    assert!(!h.app.photos[1].selected);
    for (index, photo) in h.app.photos.iter_mut().enumerate() {
        assert_eq!(photo.history.depths(), (1, 1));
        assert_eq!(photo.edit.settings.exposure, index as f32 + 0.25);
        assert!(photo.history.redo(&mut photo.edit));
        assert_eq!(photo.edit.settings.exposure, index as f32 + 0.5);
        assert!(photo.history.undo(&mut photo.edit));
        assert!(photo.history.undo(&mut photo.edit));
        assert_eq!(photo.edit.settings.exposure, 0.0);
    }
}

#[test]
fn autosave_keeps_edits_made_while_a_save_is_pending_and_shutdown_wins_races() {
    let directory = tempfile::tempdir().unwrap();
    let mut h = Harness::new(1);
    h.app.recovery_store = Some(RecoveryStore::in_directory(directory.path()).unwrap());
    let before = h.app.photos[0].edit.clone();
    h.app.photos[0].history.record(before);
    h.app.photos[0].edit.settings.exposure = 0.5;
    h.app.edited();
    h.app.autosave_tick(&h.ctx);
    h.app.autosave_last_change -= Duration::from_secs(3);
    h.app.autosave_dirty_since = Some(Instant::now() - Duration::from_secs(3));
    h.app.autosave_tick(&h.ctx);
    let (path, saved, generation) = h
        .jobs
        .try_iter()
        .find_map(|job| match job {
            Job::SaveRecovery {
                path,
                session,
                generation,
            } => Some((path, session, generation)),
            _ => None,
        })
        .unwrap();
    assert_eq!(saved.photos[0].history.depths(), (1, 0));
    assert_eq!(saved.photos[0].edit.settings.exposure, 0.5);
    h.app.photos[0].edit.settings.exposure = 0.8;
    h.app.edited();
    h.app.autosave_tick(&h.ctx);
    assert!(h.app.autosave_dirty_since.is_some());
    h.incoming
        .send(Message::RecoverySaved {
            generation,
            result: Ok(()),
        })
        .unwrap();
    h.app.poll(&h.ctx);
    assert!(h.app.autosave_pending.is_none());
    assert!(h.app.autosave_dirty_since.is_some());
    h.app.flush_recovery().unwrap();
    // The session worker can complete an old queued snapshot after on_exit begins.
    session::save_recovery(&path, &saved, generation).unwrap();
    let restored = session::read_recovery(&path).unwrap();
    assert!(restored.clean_shutdown);
    assert_eq!(restored.photos[0].edit.settings.exposure, 0.8);
    assert_eq!(restored.photos[0].history.depths(), (1, 0));
}

#[test]
fn reopening_duplicate_paths_keeps_distinct_edits_and_missing_sources_keep_recovery() {
    let mut h = Harness::new(2);
    h.app.photos[1].photo.path = h.app.photos[0].photo.path.clone();
    h.app.photos[0].edit.settings.exposure = 0.5;
    h.app.photos[1].edit.settings.exposure = -0.5;
    h.app.current = 1;
    let originals: Vec<_> = h.app.photos.iter().map(|p| p.photo.clone()).collect();
    h.incoming
        .send(Message::SessionRead(Ok(h.app.session_snapshot(false))))
        .unwrap();
    h.frame(vec![], Modifiers::NONE);
    for photo in &originals {
        h.incoming
            .send(Message::Imported(Ok(photo.clone())))
            .unwrap();
    }
    h.incoming.send(Message::ImportFinished).unwrap();
    h.frame(vec![], Modifiers::NONE);
    assert_eq!(h.app.current, 1);
    assert_eq!(h.app.photos[0].edit.settings.exposure, 0.5);
    assert_eq!(h.app.photos[1].edit.settings.exposure, -0.5);

    h.incoming
        .send(Message::SessionRead(Ok(h.app.session_snapshot(false))))
        .unwrap();
    h.app.recovery_source = Some((PathBuf::from("saved-workspace.ron"), 1));
    h.frame(vec![], Modifiers::NONE);
    h.incoming
        .send(Message::Imported(Err(anyhow::anyhow!("missing original"))))
        .unwrap();
    h.incoming
        .send(Message::Imported(Ok(originals[1].clone())))
        .unwrap();
    h.incoming.send(Message::ImportFinished).unwrap();
    h.frame(vec![], Modifiers::NONE);
    assert_eq!(h.app.photos.len(), 1);
    assert_eq!(h.app.photos[0].edit.settings.exposure, -0.5);
    assert_eq!(h.app.current, 0);
    assert!(
        h.app.recovery_source.is_none(),
        "Do not retire a recovery containing unavailable photos"
    );
    assert!(h.app.toast.as_ref().unwrap().0.contains("missing"));
}

#[test]
fn export_dialog_scroll_keeps_action_reachable_in_small_windows() {
    let mut h = Harness::new(1);
    h.size = vec2(640.0, 480.0);
    h.app.export_open = true;
    h.app.export_size = ExportSize::Story;
    for _ in 0..4 {
        h.frame(vec![], Modifiers::NONE);
    }
    let pos = h.rect("crop-preview").center();
    assert!(pos.y > 0.0 && pos.y < h.size.y);
    for _ in 0..12 {
        h.frame(
            vec![
                Event::PointerMoved(pos),
                Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: vec2(0.0, -150.0),
                    modifiers: Modifiers::NONE,
                },
            ],
            Modifiers::NONE,
        );
    }
    let button = h.rect("export-start");
    assert!(button.top() > 0.0 && button.bottom() < h.size.y);
}

#[test]
fn newer_edits_cancel_obsolete_previews_and_continue_after_worker_acknowledgement() {
    let mut h = Harness::new(1);
    h.app.photos[0].revision += 1;
    h.app.last_edit = Instant::now() - Duration::from_millis(200);
    h.frame(vec![], Modifiers::NONE);
    let (id, revision, cancel) = h
        .jobs
        .try_iter()
        .find_map(|job| match job {
            Job::Render {
                kind: RenderKind::Preview,
                id,
                revision,
                cancel,
                ..
            } => Some((id, revision, cancel)),
            _ => None,
        })
        .unwrap();
    h.app.photos[0].revision += 1;
    h.frame(vec![], Modifiers::NONE);
    assert!(cancel.load(Ordering::Relaxed));
    h.incoming
        .send(Message::RenderCancelled {
            id,
            revision,
            kind: RenderKind::Preview,
        })
        .unwrap();
    h.app.last_edit = Instant::now() - Duration::from_millis(200);
    h.frame(vec![], Modifiers::NONE);
    assert!(h.jobs.try_iter().any(|job|matches!(job,Job::Render{kind:RenderKind::Preview,revision:rev,cancel,..} if rev==h.app.photos[0].revision && !cancel.load(Ordering::Relaxed))));
}
