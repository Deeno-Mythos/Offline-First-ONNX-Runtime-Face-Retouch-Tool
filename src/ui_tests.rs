//! Real egui raw-input integration tests. No screenshot-only assertions.
use super::*;
use egui::{Event, Modifiers, PointerButton};

fn primary(pos: Pos2, pressed: bool, modifiers: Modifiers) -> Event {
    Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers,
    }
}

#[test]
fn staged_import_preserves_active_photo_and_deleted_assets_reject_late_loads() {
    let directory = tempfile::tempdir().unwrap();
    let paths: Vec<_> = (0..2)
        .map(|i| directory.path().join(format!("source-{i}.png")))
        .collect();
    for path in &paths {
        RgbaImage::from_pixel(1600, 1800, image::Rgba([100, 120, 150, 255]))
            .save(path)
            .unwrap();
    }
    let mut h = Harness::new(0);
    h.incoming
        .send(Message::Imported(Ok(
            crate::assets::stage(&paths[0]).unwrap()
        )))
        .unwrap();
    h.frame(vec![], Modifiers::NONE);
    let (id, request) = h
        .jobs
        .try_iter()
        .find_map(|j| match j {
            Job::LoadAsset { id, request, .. } => Some((id, request)),
            _ => None,
        })
        .unwrap();
    h.incoming
        .send(Message::Imported(Ok(
            crate::assets::stage(&paths[1]).unwrap()
        )))
        .unwrap();
    h.incoming
        .send(Message::AssetLoaded {
            id,
            request,
            result: engine::load_photo(&paths[0]),
        })
        .unwrap();
    h.frame(vec![], Modifiers::NONE);
    assert_eq!(h.app.current, 0);
    assert!(h.app.photos[0].photo.resident);
    assert!(!h.app.photos[1].photo.resident);
    assert_eq!(h.app.photos[0].photo.original.dimensions(), (1600, 1800));
    let weak = Arc::downgrade(&h.app.photos[0].photo.original);
    h.app.delete_from_project(id);
    h.jobs.try_iter().for_each(drop);
    assert!(weak.upgrade().is_none());
    assert!(paths[0].exists());
    h.incoming
        .send(Message::AssetLoaded {
            id,
            request,
            result: engine::load_photo(&paths[0]),
        })
        .unwrap();
    h.frame(vec![], Modifiers::NONE);
    assert_eq!(h.app.photos.len(), 1);
    assert_ne!(h.app.photos[0].id, id);
    assert!(!h.app.photos[0].photo.resident);
    assert_eq!(h.app.session_snapshot(false).photos.len(), 1);
    for path in paths {
        crate::assets::purge_thumbnail(&std::fs::canonicalize(path).unwrap());
    }
}

#[test]
fn deleting_last_photo_saves_an_empty_workspace_without_resurrecting_old_assets() {
    let directory = tempfile::tempdir().unwrap();
    let mut h = Harness::new(1);
    h.app.recovery_store = Some(RecoveryStore::in_directory(directory.path()).unwrap());
    h.app.delete_from_project(h.app.photos[0].id);
    let (path, session, generation) = h
        .jobs
        .try_iter()
        .find_map(|j| match j {
            Job::SaveRecovery {
                path,
                session,
                generation,
            } => Some((path, session, generation)),
            _ => None,
        })
        .unwrap();
    assert!(session.photos.is_empty());
    session::save_recovery(&path, &session, generation).unwrap();
    h.app.automatic_recovery = true;
    h.incoming
        .send(Message::SessionRead(Ok(
            session::read_recovery(&path).unwrap()
        )))
        .unwrap();
    h.incoming.send(Message::ImportFinished).unwrap();
    h.frame(vec![], Modifiers::NONE);
    assert!(h.app.photos.is_empty());
    assert!(!h.app.automatic_recovery);
    assert!(
        !h.jobs
            .try_iter()
            .any(|j| matches!(j, Job::Demo | Job::ReadRecovery(_)))
    );
}

#[test]
fn custom_shortcuts_replace_defaults_and_respect_typing_and_preferences() {
    let mut h = Harness::new(1);
    h.app.preferences.shortcuts.insert(
        "Use liquify brush".into(),
        crate::shortcuts::Shortcut::parse("Q"),
    );
    h.app
        .preferences
        .shortcuts
        .insert("Toggle before / after comparison".into(), None);
    h.key(egui::Key::Q, Modifiers::NONE);
    assert_eq!(h.app.brush, Some(Target::Liquify));
    h.key(egui::Key::B, Modifiers::NONE);
    assert!(!h.app.compare);
    h.app.open_preferences();
    h.app.preferences_draft.tooltips = false;
    h.frame(vec![], Modifiers::NONE);
    h.frame(vec![], Modifiers::NONE);
    let apply = h.rect("preferences-apply");
    h.frame(vec![Event::PointerMoved(apply.center())], Modifiers::NONE);
    assert_eq!(
        h.rect("preferences-apply"),
        apply,
        "Preferences footer must stay under the pointer"
    );
    h.click("preferences-apply", Modifiers::NONE);
    assert!(!h.app.preferences_open);
    assert!(h.ctx.style().interaction.tooltip_delay.is_infinite());
    h.app.open_commands();
    h.frame(vec![], Modifiers::NONE);
    h.app.brush = None;
    h.frame(vec![Event::Text("Q".into())], Modifiers::NONE);
    assert!(h.app.brush.is_none());
}

#[test]
fn customized_layer_shortcuts_replace_scoped_defaults() {
    let mut h = Harness::new(1);
    h.app.preferences.shortcuts.insert(
        "Duplicate layer".into(),
        crate::shortcuts::Shortcut::parse("Alt+D"),
    );
    h.app
        .preferences
        .shortcuts
        .insert("Delete layer".into(), None);
    h.click("toggle-Layers", Modifiers::NONE);
    h.key(egui::Key::J, Modifiers::COMMAND);
    assert_eq!(h.app.photos[0].edit.stack.layers.len(), 1);
    h.key(egui::Key::D, Modifiers::ALT);
    assert_eq!(h.app.photos[0].edit.stack.layers.len(), 2);
    h.key(egui::Key::Delete, Modifiers::NONE);
    assert_eq!(h.app.photos[0].edit.stack.layers.len(), 2);
    assert!(
        h.app
            .command_label(
                app_chrome::AppCommand::Layer(LayerAction::Duplicate),
                "Duplicate"
            )
            .contains("Alt+D")
    );
    h.click("inspector-Tools", Modifiers::NONE);
    h.key(egui::Key::D, Modifiers::ALT);
    assert_eq!(
        h.app.photos[0].edit.stack.layers.len(),
        2,
        "Layer bindings retain panel scope"
    );
}

#[test]
fn shortcuts_manager_records_keys_rejects_conflicts_and_persists_the_binding() {
    let directory = tempfile::tempdir().unwrap();
    let mut h = Harness::new(1);
    h.app.preferences_path = Some(directory.path().join("prefs.ron"));
    h.app.open_preferences();
    h.app.preferences_tab = PreferencesTab::Shortcuts;
    h.frame(vec![], Modifiers::NONE);
    h.frame(vec![], Modifiers::NONE);
    let first = h.rect("binding-Preferences");
    h.frame(vec![Event::PointerMoved(first.center())], Modifiers::NONE);
    assert_eq!(
        h.rect("binding-Preferences"),
        first,
        "Recording rows must stay under the pointer"
    );
    h.click("binding-Preferences", Modifiers::NONE);
    assert_eq!(h.app.shortcut_capture.as_deref(), Some("Preferences"));
    h.key(egui::Key::O, Modifiers::COMMAND);
    assert!(
        h.app
            .preferences_error
            .as_ref()
            .unwrap()
            .contains("already assigned")
    );
    assert!(h.app.preferences_draft.shortcuts.is_empty());
    h.key(egui::Key::Q, Modifiers::ALT);
    assert!(h.app.shortcut_capture.is_none());
    assert_eq!(
        h.app.preferences_draft.shortcuts["Preferences"]
            .as_ref()
            .unwrap()
            .label(),
        "Alt+Q"
    );
    h.click("preferences-apply", Modifiers::NONE);
    h.key(egui::Key::Comma, Modifiers::COMMAND);
    assert!(!h.app.preferences_open);
    h.key(egui::Key::Q, Modifiers::ALT);
    assert!(h.app.preferences_open);
    let loaded = preferences::load(h.app.preferences_path.as_deref().unwrap()).unwrap();
    assert_eq!(loaded.shortcuts, h.app.preferences.shortcuts);
}

#[test]
fn hidden_manual_target_rejects_invisible_edits_and_show_layer_restores_saved_work_undoably() {
    for target in [Target::Heal, Target::Liquify, Target::Clone, Target::Patch] {
        let mut h = Harness::new(1);
        h.app.brush = Some(target);
        let p = &mut h.app.photos[0];
        p.edit.warps.push(crate::geometry::WarpStroke {
            center: [0.5, 0.5],
            delta: [0.05, 0.02],
            radius: 0.15,
            softness: 0.5,
            strength: 100.,
        });
        p.edit.ensure_stack();
        p.edit.active_layer_mut().unwrap().visible = false;
        let saved = p.edit.clone();
        let source = p.photo.preview.clone();
        assert_eq!(engine::render(&source, &saved, None), *source);
        h.frame(vec![], Modifiers::NONE);
        assert!(h.app.manual_blocked());
        let depths = h.app.photos[0].history.depths();
        let center = h.rect("image").center();
        h.drag(center, center + vec2(24., 8.));
        assert_eq!(
            h.app.photos[0].edit, saved,
            "{target:?} recorded invisible work"
        );
        assert_eq!(h.app.photos[0].history.depths(), depths);
        h.click("manual-target-fix", Modifiers::NONE);
        assert!(!h.app.manual_blocked());
        assert!(h.app.photos[0].edit.active_layer().unwrap().visible);
        assert_eq!(h.app.photos[0].edit.warps, saved.warps);
        assert_ne!(
            engine::render(&source, &h.app.photos[0].edit, None),
            *source
        );
        h.app.undo(&h.ctx);
        assert_eq!(h.app.photos[0].edit, saved);
        h.app.redo(&h.ctx);
        assert!(!h.app.manual_blocked());
    }
}

#[test]
fn manual_target_remedies_cover_opacity_lock_original_copy_and_disabled_healing() {
    for case in 0..6 {
        let mut h = Harness::new(1);
        h.app.brush = Some(Target::Heal);
        let edit = &mut h.app.photos[0].edit;
        edit.ensure_stack();
        match case {
            0 => edit.active_layer_mut().unwrap().opacity = 0.,
            1 => edit.active_layer_mut().unwrap().locked = true,
            2 => {
                let original = edit.stack.active;
                edit.add_layer(LayerType::OriginalCopy);
                edit.select_layer(original);
            }
            3 => {
                edit.layers
                    .get_mut(crate::layers::LayerKind::SpotHeal)
                    .visible = false
            }
            4 => edit.settings.disabled.push(Adjustment::Healing),
            5 => h.app.brush_strength = 0.,
            _ => unreachable!(),
        }
        let before = h.app.photos[0].edit.clone();
        h.frame(vec![], Modifiers::NONE);
        assert!(h.app.manual_blocked(), "case {case}");
        h.click("manual-target-fix", Modifiers::NONE);
        assert!(!h.app.manual_blocked(), "case {case}");
        if [1, 2].contains(&case) {
            let stack = &h.app.photos[0].edit.stack;
            assert_eq!(stack.layers.last().unwrap().id, stack.active);
            assert_eq!(stack.layers.last().unwrap().kind, LayerType::Retouch);
        }
        if case != 5 {
            h.app.undo(&h.ctx);
            assert_eq!(h.app.photos[0].edit, before);
        } else {
            assert_eq!(h.app.photos[0].edit, before);
            assert_eq!(h.app.brush_strength, 100.);
        }
    }
}

#[test]
fn clone_source_mode_never_paints_while_held_and_alignment_survives_separate_strokes() {
    let mut h = Harness::new(1);
    h.app.compare = true;
    h.app.toggle_brush(Target::Clone);
    assert!(
        !h.app.compare,
        "Selecting a manual tool shows the entire edited image"
    );
    h.frame(vec![], Modifiers::NONE);
    h.click("clone-pick-source", Modifiers::NONE);
    assert!(h.app.clone_pick_source);
    let rect = h.rect("image");
    let source = rect.min + rect.size() * vec2(0.2, 0.3);
    let first = rect.min + rect.size() * vec2(0.6, 0.5);
    let second = first + vec2(20., 30.);
    h.frame(
        vec![
            Event::PointerMoved(source),
            primary(source, true, Modifiers::NONE),
        ],
        Modifiers::NONE,
    );
    for _ in 0..3 {
        h.frame(
            vec![Event::PointerMoved(source + vec2(8., 3.))],
            Modifiers::NONE,
        );
    }
    h.frame(
        vec![primary(source + vec2(8., 3.), false, Modifiers::NONE)],
        Modifiers::NONE,
    );
    assert!(h.app.photos[0].edit.clones.is_empty());
    assert!(!h.app.photos[0].history.can_undo());
    let anchor = h.app.clone_anchor.unwrap().1;
    assert!((anchor[0] - 0.2).abs() < 0.0001);
    h.click_at(first, Modifiers::NONE);
    let first_stamp = h.app.photos[0].edit.clones[0].clone();
    h.time += 0.7;
    h.click_at(second, Modifiers::NONE);
    let last = h.app.photos[0].edit.clones.last().unwrap();
    for c in 0..2 {
        assert!(
            ((last.source[c] - last.center[c]) - (first_stamp.source[c] - first_stamp.center[c]))
                .abs()
                < 0.00001
        );
    }
    h.app.clone_aligned = false;
    h.time += 0.7;
    h.click_at(first + vec2(30., -20.), Modifiers::NONE);
    let last = h.app.photos[0].edit.clones.last().unwrap();
    for (c, &value) in anchor.iter().enumerate() {
        assert!((last.source[c] - value).abs() < 0.00001);
    }
    let before = h.app.photos[0].edit.clone();
    h.time += 0.7;
    h.frame(vec![primary(source, true, Modifiers::ALT)], Modifiers::ALT);
    h.frame(
        vec![Event::PointerMoved(source + vec2(14., 9.))],
        Modifiers::NONE,
    );
    h.frame(
        vec![primary(source + vec2(14., 9.), false, Modifiers::NONE)],
        Modifiers::NONE,
    );
    assert_eq!(
        h.app.photos[0].edit, before,
        "Releasing Alt while the source click is held must not paint"
    );
}

#[test]
fn fast_clicks_and_first_frame_motion_commit_strength_and_pixels_as_one_undo_step() {
    for target in [Target::Heal, Target::Clone, Target::Liquify] {
        let mut h = Harness::new(1);
        // Healing should preserve clean gradients; give it a real defect.
        let mut source = (*h.app.photos[0].photo.original).clone();
        for y in 317..324 {
            for x in 207..214 {
                source.put_pixel(x, y, image::Rgba([5, 5, 5, 255]));
            }
        }
        h.app.photos[0].photo = engine::photo_from_image("defect.png".into(), None, source);
        h.app.brush = Some(target);
        h.app.brush_strength = 37.;
        h.frame(vec![], Modifiers::NONE);
        let start = h.rect("image").center();
        let end = start + vec2(18., 6.);
        h.app.clone_anchor = Some((h.app.photos[0].id, [0.2, 0.3]));
        let before = h.app.photos[0].edit.clone();
        h.frame(
            vec![
                Event::PointerMoved(start),
                primary(start, true, Modifiers::NONE),
                Event::PointerMoved(end),
                primary(end, false, Modifiers::NONE),
            ],
            Modifiers::NONE,
        );
        let p = &mut h.app.photos[0];
        assert_ne!(
            p.edit, before,
            "A same-frame {target:?} press/move/release was lost"
        );
        match target {
            Target::Heal => assert!(p.edit.strokes.iter().all(|s| s.strength == 37.)),
            Target::Clone => assert!(p.edit.clones.iter().all(|s| s.strength == 37.)),
            Target::Liquify => {
                let sum: f32 = p.edit.warps.iter().map(|s| s.delta[0]).sum();
                assert!(sum > 0.02);
            }
            _ => unreachable!(),
        }
        assert!(
            engine::render(&p.photo.preview, &p.edit, None) != *p.photo.preview,
            "{target:?} instructions must change rendered pixels"
        );
        assert!(p.history.undo(&mut p.edit));
        assert_eq!(p.edit, before);
        assert!(!p.history.can_undo());
    }
}

#[test]
fn patch_feedback_is_temporary_exact_and_retained_until_committed_preview_arrives() {
    let mut h = Harness::new(1);
    h.app.brush = Some(Target::Patch);
    h.app.patch_draft.boundary = vec![[0.4, 0.4], [0.6, 0.4], [0.6, 0.6], [0.4, 0.6]];
    h.frame(vec![], Modifiers::NONE);
    let rect = h.rect("image");
    let start = rect.center();
    let end = start + rect.size() * vec2(-0.15, 0.);
    let before = h.app.photos[0].edit.clone();
    h.frame(
        vec![
            Event::PointerMoved(start),
            primary(start, true, Modifiers::NONE),
        ],
        Modifiers::NONE,
    );
    h.frame(vec![Event::PointerMoved(end)], Modifiers::NONE);
    let job = h
        .jobs
        .try_iter()
        .find(|j| matches!(j, Job::PatchPreview { .. }))
        .expect("Patch source drag needs a live preview");
    let Job::PatchPreview {
        id,
        revision,
        request,
        image,
        edit,
        seg,
        crop,
        cancel,
    } = job
    else {
        unreachable!()
    };
    assert_eq!(h.app.photos[0].edit, before);
    assert!(!h.app.photos[0].history.can_undo());
    let preview = engine::render(&image, &edit, seg.as_deref());
    h.incoming
        .send(Message::PatchPreview {
            id,
            revision,
            request,
            crop,
            result: Ok(preview.clone()),
        })
        .unwrap();
    h.frame(vec![], Modifiers::NONE);
    assert_eq!(*h.app.patch_feedback.frame.as_ref().unwrap().image, preview);
    assert_eq!(h.app.photos[0].edit, before);
    assert!(!cancel.load(Ordering::Relaxed));
    h.frame(vec![primary(end, false, Modifiers::NONE)], Modifiers::NONE);
    assert_eq!(h.app.photos[0].edit.patches.len(), 1);
    assert_eq!(engine::render(&image, &h.app.photos[0].edit, None), preview);
    assert!(h.app.patch_feedback.frame.as_ref().unwrap().committed);
    let p = &h.app.photos[0];
    h.incoming
        .send(Message::Rendered {
            id: p.id,
            revision: p.revision,
            kind: RenderKind::Preview,
            image: preview,
        })
        .unwrap();
    h.frame(vec![], Modifiers::NONE);
    assert!(h.app.patch_feedback.frame.is_none());
    h.app.undo(&h.ctx);
    assert_eq!(h.app.photos[0].edit, before);
}

#[test]
fn clearing_patch_selection_cancels_preview_and_discards_its_late_result() {
    let mut h = Harness::new(1);
    h.app.brush = Some(Target::Patch);
    h.app.patch_draft.boundary = vec![[0.4, 0.4], [0.6, 0.4], [0.6, 0.6], [0.4, 0.6]];
    h.app.patch_draft.source_drag = Some(([0.5, 0.5], [0.3, 0.5]));
    h.frame(vec![], Modifiers::NONE);
    let Job::PatchPreview {
        id,
        revision,
        request,
        image,
        crop,
        cancel,
        ..
    } = h
        .jobs
        .try_iter()
        .find(|j| matches!(j, Job::PatchPreview { .. }))
        .unwrap()
    else {
        unreachable!()
    };
    h.click("patch-clear", Modifiers::NONE);
    assert!(cancel.load(Ordering::Relaxed));
    h.incoming
        .send(Message::PatchPreview {
            id,
            revision,
            request,
            crop,
            result: Ok((*image).clone()),
        })
        .unwrap();
    h.frame(vec![], Modifiers::NONE);
    assert!(h.app.patch_feedback.frame.is_none());
    assert!(h.app.photos[0].edit.patches.is_empty());
    assert!(!h.app.photos[0].history.can_undo());
}

#[test]
fn native_patch_feedback_uses_original_pixels_and_one_gpu_canvas_callback() {
    let mut h = Harness::new(1);
    h.app.gpu = true;
    h.app.photos[0].photo = engine::photo_from_image(
        "native.png".into(),
        None,
        RgbaImage::from_fn(1600, 1800, |x, y| {
            image::Rgba([(x % 220) as u8, (y % 220) as u8, 80, 255])
        }),
    );
    h.app.photos[0].view = View {
        fit: false,
        scale: 1.,
        pan: [0., 0.],
    };
    h.app.brush = Some(Target::Patch);
    h.app.patch_draft.boundary = vec![[0.4, 0.4], [0.6, 0.4], [0.6, 0.6], [0.4, 0.6]];
    h.app.patch_draft.source_drag = Some(([0.5, 0.5], [0.3, 0.5]));
    let before = h.app.photos[0].edit.clone();
    h.frame(vec![], Modifiers::NONE);
    let Job::PatchPreview {
        id,
        revision,
        request,
        image,
        edit,
        seg,
        crop: Some(crop),
        ..
    } = h
        .jobs
        .try_iter()
        .find(|j| matches!(j, Job::PatchPreview { .. }))
        .unwrap()
    else {
        panic!("Native feedback must request original-resolution pixels")
    };
    assert_eq!(image.dimensions(), (1600, 1800));
    let expected = engine::render_region(&image, &edit, seg.as_deref(), crop, None).unwrap();
    h.incoming
        .send(Message::PatchPreview {
            id,
            revision,
            request,
            crop: Some(crop),
            result: Ok(expected.clone()),
        })
        .unwrap();
    let output = h.frame(vec![], Modifiers::NONE);
    let frame = h.app.patch_feedback.frame.as_ref().unwrap();
    assert_eq!(frame.crop, Some(crop));
    assert!(frame.image.as_ref() == &expected);
    assert_eq!(frame.original.dimensions(), (crop.width, crop.height));
    assert_eq!(
        frame.texture.size(),
        [1, 1],
        "GPU canvas must not build an unused native thumbnail"
    );
    assert_eq!(frame.original_texture.size(), [1, 1]);
    assert_eq!(
        output
            .shapes
            .iter()
            .filter(|s| matches!(s.shape, egui::Shape::Callback(_)))
            .count(),
        1,
        "Multiple GPU callbacks would overwrite their shared image binding"
    );
    assert_eq!(h.app.photos[0].edit, before);
    assert!(!h.app.photos[0].history.can_undo());
    let original = frame.original.clone();
    h.app.patch_draft.source_drag = Some(([0.5, 0.5], [0.32, 0.5]));
    h.frame(vec![], Modifiers::NONE);
    let Job::PatchPreview {
        id,
        revision,
        request,
        image,
        edit,
        seg,
        crop,
        ..
    } = h
        .jobs
        .try_iter()
        .find(|j| matches!(j, Job::PatchPreview { .. }))
        .unwrap()
    else {
        unreachable!()
    };
    let expected =
        engine::render_region(&image, &edit, seg.as_deref(), crop.unwrap(), None).unwrap();
    h.incoming
        .send(Message::PatchPreview {
            id,
            revision,
            request,
            crop,
            result: Ok(expected),
        })
        .unwrap();
    h.frame(vec![], Modifiers::NONE);
    assert!(
        Arc::ptr_eq(
            &original,
            &h.app.patch_feedback.frame.as_ref().unwrap().original
        ),
        "Moving the patch source must reuse the immutable native source crop"
    );
    h.app.photos[0].edit = edit.clone();
    h.app.edited();
    h.app.commit_patch_feedback(edit.patches.last().unwrap());
    h.app.patch_draft = PatchDraft::default();
    let revision = h.app.photos[0].revision;
    // Under a busy test runner an older overview may already be pending.
    // Acknowledge its cancellation, then accept the actual final request,
    // rather than relying on the stroke fitting inside the debounce interval.
    if let Some((id, revision, _)) = &h.app.preview_cancel {
        h.incoming
            .send(Message::RenderCancelled {
                id: *id,
                revision: *revision,
                kind: RenderKind::Preview,
            })
            .unwrap();
    }
    h.app.last_edit = Instant::now() - Duration::from_millis(70);
    h.app.last_preview_request = Instant::now() - Duration::from_millis(70);
    h.frame(vec![], Modifiers::NONE);
    let overview = engine::render(&h.app.photos[0].photo.preview, &edit, None);
    h.incoming
        .send(Message::Rendered {
            id,
            revision,
            kind: RenderKind::Preview,
            image: overview,
        })
        .unwrap();
    h.frame(vec![], Modifiers::NONE);
    assert!(
        h.app
            .patch_feedback
            .frame
            .as_ref()
            .is_some_and(|f| f.committed),
        "An overview must not discard the committed original-pixel patch feedback"
    );
    let Job::DetailRegion {
        id,
        revision,
        image,
        edit,
        seg,
        crop,
        ..
    } = h
        .jobs
        .try_iter()
        .find(|j| matches!(j,Job::DetailRegion {revision:rev,..} if *rev==revision))
        .unwrap()
    else {
        unreachable!()
    };
    let ready = engine::render_region(&image, &edit, seg.as_deref(), crop, None).unwrap();
    h.incoming
        .send(Message::DetailRegionRendered {
            id,
            revision,
            crop,
            image: ready,
        })
        .unwrap();
    h.frame(vec![], Modifiers::NONE);
    assert!(
        h.app.patch_feedback.frame.is_none(),
        "The completed native tile now owns the canvas"
    );
}

#[test]
fn clone_source_on_the_original_comparison_side_uses_unwarped_source_coordinates() {
    let mut h = Harness::new(1);
    h.app.brush = Some(Target::Clone);
    h.app.compare = true;
    h.app.split = 0.8;
    h.app.photos[0]
        .edit
        .warps
        .push(crate::geometry::WarpStroke {
            center: [0.4, 0.5],
            delta: [0.08, 0.02],
            radius: 0.3,
            softness: 0.5,
            strength: 100.,
        });
    h.frame(vec![], Modifiers::NONE);
    let before = h.app.photos[0].edit.clone();
    let rect = h.rect("image");
    let source = rect.min + rect.size() * vec2(0.4, 0.5);
    h.click_at(source, Modifiers::ALT);
    let anchor = h.app.clone_anchor.unwrap().1;
    assert!((anchor[0] - 0.4).abs() < 0.00001 && (anchor[1] - 0.5).abs() < 0.00001);
    assert_eq!(h.app.photos[0].edit, before);
    h.click_at(source, Modifiers::NONE);
    assert_eq!(
        h.app.photos[0].edit, before,
        "Original-view clicks must not record invisible brush edits"
    );
}

#[test]
fn preferences_apply_save_and_cancel_are_separate_from_photo_edits() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("preferences.ron");
    let mut h = Harness::new(1);
    h.app.preferences_path = Some(path.clone());
    let before = h.app.photos[0].edit.clone();
    let depths = h.app.photos[0].history.depths();
    h.key(egui::Key::Comma, Modifiers::COMMAND);
    assert!(h.app.preferences_open);
    h.frame(vec![], Modifiers::NONE);
    h.app.preferences_draft.ui_scale = 1.0;
    h.app.preferences_draft.reduced_motion = true;
    h.app.preferences_draft.brush_radius_percent = 3.0;
    h.app.preferences_draft.brush_strength = 45.0;
    h.app.preferences_draft.autosave_seconds = 5;
    h.app.preferences_draft.export_png = true;
    h.app.preferences_draft.export_size = ExportSize::Web;
    h.click("preferences-apply", Modifiers::NONE);
    assert!(!h.app.preferences_open);
    assert_eq!(preferences::load(&path).unwrap(), h.app.preferences);
    assert_eq!(h.ctx.style().animation_time, 0.0);
    assert!((h.app.brush_radius - 0.03).abs() < 0.00001);
    assert_eq!(h.app.brush_strength, 45.0);
    assert!(h.app.export_png);
    assert_eq!(h.app.export_size, ExportSize::Web);
    assert_eq!(h.app.photos[0].edit, before);
    assert_eq!(h.app.photos[0].history.depths(), depths);
    h.app.open_preferences();
    h.frame(vec![], Modifiers::NONE);
    h.click("preferences-reset", Modifiers::NONE);
    assert_eq!(h.app.preferences_draft, Preferences::default());
    h.click("preferences-cancel", Modifiers::NONE);
    assert_eq!(preferences::load(&path).unwrap(), h.app.preferences);
    assert_eq!(h.app.preferences.autosave_seconds, 5);
    // Applying preferences during focus preserves the panel that F will restore.
    let panel = h.app.inspector;
    h.app.toggle_focus();
    h.app.open_preferences();
    h.frame(vec![], Modifiers::NONE);
    h.click("preferences-apply", Modifiers::NONE);
    assert!(!h.app.filmstrip_visible);
    h.app.toggle_focus();
    assert!(h.app.inspector == panel);
    assert!(h.app.filmstrip_visible);
}

#[test]
fn preferences_failure_keeps_draft_and_compact_modal_blocks_background_strokes() {
    let directory = tempfile::tempdir().unwrap();
    let occupied = directory.path().join("occupied");
    std::fs::write(&occupied, "keep this file").unwrap();
    let mut h = Harness::new(1);
    h.size = vec2(600.0, 560.0);
    h.app.brush = Some(Target::Heal);
    h.app.preferences_path = Some(occupied.join("preferences.ron"));
    h.frame(vec![], Modifiers::NONE);
    h.app.open_preferences();
    for _ in 0..3 {
        h.frame(vec![], Modifiers::NONE);
    }
    let screen = Rect::from_min_size(Pos2::ZERO, h.size);
    for tab in ["Workspace", "Editing", "Workflow", "Export"] {
        h.click(&format!("preferences-tab-{tab}"), Modifiers::NONE);
        for _ in 0..2 {
            h.frame(vec![], Modifiers::NONE);
        }
        for key in [
            "preferences-apply",
            "preferences-cancel",
            "preferences-reset",
        ] {
            assert!(
                screen.contains_rect(h.rect(key)),
                "{tab}: {key} must stay reachable"
            );
        }
    }
    h.app.preferences_draft.jpeg_quality = 80;
    h.click("preferences-apply", Modifiers::NONE);
    assert!(h.app.preferences_open);
    assert!(
        h.app
            .preferences_error
            .as_ref()
            .unwrap()
            .contains("Could not save")
    );
    h.frame(vec![], Modifiers::NONE);
    assert!(screen.contains_rect(h.rect("preferences-apply")));
    assert!(screen.contains_rect(h.rect("preferences-cancel")));
    assert_eq!(h.app.preferences.jpeg_quality, 95);
    assert_eq!(std::fs::read_to_string(occupied).unwrap(), "keep this file");
    let before = h.app.photos[0].edit.clone();
    let image = h.rect("image");
    let outside = pos2(image.left() + 4., image.top() + 4.);
    h.drag(outside, outside + vec2(10., 12.));
    assert_eq!(
        h.app.photos[0].edit, before,
        "Modal pointer input must not paint underneath"
    );
}

#[test]
fn command_search_keyboard_execution_and_disabled_actions_respect_editing_state() {
    let mut h = Harness::new(1);
    h.key(egui::Key::K, Modifiers::COMMAND);
    assert!(h.app.command_open);
    h.frame(vec![Event::Text("liquify".into())], Modifiers::NONE);
    assert_eq!(h.app.command_query, "liquify");
    h.key(egui::Key::Num5, Modifiers::NONE);
    assert_eq!(
        h.app.photos[0].rating, 0,
        "Search typing cannot rate a photo"
    );
    h.key(egui::Key::Enter, Modifiers::NONE);
    assert!(!h.app.command_open);
    assert_eq!(h.app.brush, Some(Target::Liquify));
    h.key(egui::Key::K, Modifiers::COMMAND);
    h.frame(vec![Event::Text("new layer".into())], Modifiers::NONE);
    h.key(egui::Key::ArrowDown, Modifiers::NONE);
    h.key(egui::Key::Enter, Modifiers::NONE);
    assert_eq!(h.app.photos[0].edit.stack.layers.len(), 2);
    assert_eq!(
        h.app.photos[0].edit.active_layer().unwrap().kind,
        LayerType::Adjustment
    );
    h.key(egui::Key::Z, Modifiers::COMMAND);
    assert_eq!(h.app.photos[0].edit.stack.layers.len(), 1);
    h.app.photos[0].edit.select_layer(1);
    h.app.photos[0].edit.active_layer_mut().unwrap().locked = true;
    h.key(egui::Key::K, Modifiers::COMMAND);
    h.frame(vec![Event::Text("clone stamp".into())], Modifiers::NONE);
    h.key(egui::Key::Enter, Modifiers::NONE);
    assert!(
        h.app.command_open,
        "A disabled command must keep the menu open"
    );
    assert_eq!(h.app.brush, Some(Target::Liquify));
    h.key(egui::Key::Escape, Modifiers::NONE);
    assert!(!h.app.command_open);
}

#[test]
fn focus_and_filmstrip_shortcuts_expand_canvas_and_restore_previous_layout() {
    let mut h = Harness::new(2);
    h.click("toggle-Layers", Modifiers::NONE);
    let canvas = h.rect("canvas");
    let edit = h.app.photos[0].edit.clone();
    h.key(egui::Key::F, Modifiers::NONE);
    let focused = h.rect("canvas");
    assert!(focused.width() > canvas.width() + 250.);
    assert!(focused.height() > canvas.height() + 100.);
    assert!(h.app.inspector.is_none());
    assert!(!h.app.filmstrip_visible);
    h.key(egui::Key::F, Modifiers::NONE);
    assert_eq!(h.rect("canvas"), canvas);
    assert!(h.app.inspector == Some(Inspector::Layers));
    h.key(egui::Key::F6, Modifiers::NONE);
    assert!(!h.app.filmstrip_visible);
    assert!(h.rect("canvas").height() > canvas.height() + 100.);
    h.key(egui::Key::F6, Modifiers::NONE);
    assert_eq!(h.rect("canvas"), canvas);
    assert_eq!(h.app.photos[0].edit, edit);
}

#[test]
fn brush_shortcuts_set_actual_warp_parameters_and_do_not_steal_layer_rename_input() {
    let mut h = Harness::new(1);
    h.app.brush = Some(Target::Liquify);
    let initial = h.app.brush_radius;
    h.key(egui::Key::CloseBracket, Modifiers::NONE);
    h.key(egui::Key::OpenBracket, Modifiers::SHIFT);
    assert!((h.app.brush_radius - initial * 1.25).abs() < 0.00001);
    assert_eq!(h.app.brush_strength, 95.);
    let center = h.rect("image").center();
    h.drag(center, center + vec2(25., 10.));
    let warp = &h.app.photos[0].edit.warps[0];
    assert_eq!(warp.radius, h.app.brush_radius);
    assert_eq!(warp.strength, 95.);
    let overlay = h.app.show_mask;
    h.key(egui::Key::M, Modifiers::NONE);
    assert_ne!(h.app.show_mask, overlay);
    h.app.brush = Some(Target::Skin);
    h.key(egui::Key::X, Modifiers::NONE);
    assert!(h.app.erase);
    h.click("toggle-Layers", Modifiers::NONE);
    h.key(egui::Key::F2, Modifiers::NONE);
    let radius = h.app.brush_radius;
    h.key(egui::Key::CloseBracket, Modifiers::NONE);
    h.key(egui::Key::Num5, Modifiers::NONE);
    assert_eq!(h.app.brush_radius, radius);
    assert_eq!(h.app.photos[0].rating, 0);
}

#[test]
fn photo_navigation_range_selection_and_ratings_survive_sessions_without_touching_edits() {
    let mut h = Harness::new(4);
    h.app.photos[0].edit.settings.exposure = 0.5;
    let before = h.app.photos[0].edit.clone();
    h.key(egui::Key::ArrowRight, Modifiers::NONE);
    assert_eq!(h.app.current, 1);
    h.key(egui::Key::ArrowRight, Modifiers::SHIFT);
    assert_eq!(h.app.current, 2);
    assert_eq!(
        h.app.photos.iter().map(|p| p.selected).collect::<Vec<_>>(),
        [false, true, true, false]
    );
    h.key(egui::Key::Num4, Modifiers::NONE);
    assert_eq!(h.app.photos[2].rating, 4);
    h.key(egui::Key::A, Modifiers::COMMAND);
    assert!(h.app.photos.iter().all(|p| p.selected));
    h.key(egui::Key::I, Modifiers::COMMAND);
    assert!(h.app.photos.iter().all(|p| !p.selected));
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("ratings.ron");
    session::save(&path, &h.app.session_snapshot(false)).unwrap();
    let reopened = session::read(&path).unwrap();
    assert_eq!(reopened.photos[2].rating, 4);
    assert_eq!(reopened.photos[0].edit, before);
    h.key(egui::Key::Num0, Modifiers::SHIFT);
    assert_eq!(h.app.photos[2].rating, 0);
}

#[test]
fn autosave_pause_and_configured_quiet_interval_preserve_the_current_workspace() {
    let directory = tempfile::tempdir().unwrap();
    let mut h = Harness::new(1);
    h.app.recovery_store = Some(RecoveryStore::in_directory(directory.path()).unwrap());
    h.app.preferences.autosave_seconds = 5;
    h.app.autosave_tick(&h.ctx);
    h.app.autosave_last_change -= Duration::from_secs(3);
    h.app.autosave_dirty_since = Some(Instant::now() - Duration::from_secs(3));
    h.app.autosave_tick(&h.ctx);
    assert!(
        !h.jobs
            .try_iter()
            .any(|j| matches!(j, Job::SaveRecovery { .. }))
    );
    h.app.preferences.autosave = false;
    h.app.autosave_last_change -= Duration::from_secs(10);
    h.app.autosave_tick(&h.ctx);
    h.app.flush_recovery().unwrap();
    assert!(!h.app.recovery_store.as_ref().unwrap().path.exists());
    assert!(
        !h.jobs
            .try_iter()
            .any(|j| matches!(j, Job::SaveRecovery { .. }))
    );
    h.app.preferences.autosave = true;
    h.app.autosave_tick(&h.ctx);
    let saved = h
        .jobs
        .try_iter()
        .find_map(|job| match job {
            Job::SaveRecovery { session, .. } => Some(session),
            _ => None,
        })
        .unwrap();
    assert_eq!(saved.photos[0].edit, h.app.photos[0].edit);
}

#[test]
fn wheel_preferences_change_navigation_and_ui_scale_preserves_native_photo_pixels() {
    fn scroll(h: &mut Harness) {
        let pos = h.rect("canvas").center();
        h.frame(
            vec![
                Event::PointerMoved(pos),
                Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: vec2(0., 60.),
                    modifiers: Modifiers::NONE,
                },
            ],
            Modifiers::NONE,
        );
    }
    let mut slow = Harness::new(1);
    slow.app.photos[0].view.fit = false;
    slow.app.photos[0].view.scale = 2.0;
    slow.app.preferences.zoom_speed = 0.5;
    slow.frame(vec![], Modifiers::NONE);
    scroll(&mut slow);
    let mut fast = Harness::new(1);
    fast.app.photos[0].view.fit = false;
    fast.app.photos[0].view.scale = 2.0;
    fast.app.preferences.zoom_speed = 2.0;
    fast.frame(vec![], Modifiers::NONE);
    scroll(&mut fast);
    assert!(fast.app.photos[0].view.scale > slow.app.photos[0].view.scale);
    let mut pan = Harness::new(1);
    pan.app.photos[0].view.fit = false;
    pan.app.photos[0].view.scale = 2.0;
    pan.app.preferences.wheel_zoom = false;
    pan.app.preferences.invert_scroll = true;
    pan.frame(vec![], Modifiers::NONE);
    scroll(&mut pan);
    assert_eq!(pan.app.photos[0].view.scale, 2.0);
    assert!(pan.app.photos[0].view.pan[1] < 0.);
    let edit = pan.app.photos[0].edit.clone();
    pan.app.preferences.ui_scale = 1.25;
    pan.app.apply_preferences(&pan.ctx);
    pan.frame(vec![], Modifiers::NONE);
    pan.app
        .execute_command(&pan.ctx, app_chrome::AppCommand::Native);
    pan.frame(vec![], Modifiers::NONE);
    let physical = pan.rect("image").size() * pan.ctx.pixels_per_point();
    assert!((physical.x - 420.).abs() < 0.01);
    assert!((physical.y - 640.).abs() < 0.01);
    assert_eq!(pan.app.photos[0].edit, edit);
}

#[test]
fn command_menu_scrolls_to_tools_below_the_initial_selection_without_jumping_back() {
    let mut h = Harness::new(1);
    h.app.open_commands();
    for _ in 0..3 {
        h.frame(vec![], Modifiers::NONE);
    }
    let pos = h.rect("command-search").center() + vec2(0., 90.);
    h.frame(
        vec![
            Event::PointerMoved(pos),
            Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                // Reach the end regardless of how many commands the manager exposes.
                delta: vec2(0., -10000.),
                modifiers: Modifiers::NONE,
            },
        ],
        Modifiers::NONE,
    );
    for _ in 0..12 {
        h.frame(vec![], Modifiers::NONE);
    }
    let last = h.rect("command-Open AI models");
    assert!(Rect::from_min_size(Pos2::ZERO, h.size).contains_rect(last));
    h.click("command-Open AI models", Modifiers::NONE);
    assert!(!h.app.command_open);
    assert!(h.app.tool == Tool::Models);
}

#[test]
fn auto_retouch_shortcut_preserves_color_and_shape_and_records_undo() {
    let mut h = Harness::new(1);
    let p = &mut h.app.photos[0];
    p.edit.settings.contrast = 28.;
    p.edit.settings.exposure = -0.4;
    p.edit.settings.eye_size = 18.;
    let before = p.edit.clone();
    h.key(egui::Key::A, Modifiers::NONE);
    let after = &h.app.photos[0].edit;
    assert_eq!(after.settings.contrast, before.settings.contrast);
    assert_eq!(after.settings.exposure, before.settings.exposure);
    assert_eq!(after.settings.eye_size, before.settings.eye_size);
    assert!(after.settings.smoothing > 0.);
    h.app.undo(&h.ctx);
    assert_eq!(h.app.photos[0].edit, before);
}

#[test]
fn layer_eye_solo_restores_hidden_layers_and_survives_session_history() {
    let mut h = Harness::new(1);
    h.click("toggle-Layers", Modifiers::NONE);
    h.click("stack-new", Modifiers::NONE);
    h.click("stack-adjust", Modifiers::NONE);
    h.click("stack-visible-1", Modifiers::NONE);
    let visibility: Vec<_> = h.app.photos[0]
        .edit
        .stack
        .layers
        .iter()
        .map(|l| (l.id, l.visible))
        .collect();
    let alt = Modifiers {
        alt: true,
        ..Modifiers::NONE
    };
    h.click("stack-visible-2", alt);
    assert_eq!(h.app.photos[0].edit.stack.solo.as_ref().unwrap().layer, 2);
    assert!(
        h.app.photos[0]
            .edit
            .stack
            .layers
            .iter()
            .all(|l| l.visible == (l.id == 2))
    );
    h.click("stack-visible-3", alt);
    assert_eq!(h.app.photos[0].edit.stack.solo.as_ref().unwrap().layer, 3);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("solo.ron");
    session::save(&path, &h.app.session_snapshot(false)).unwrap();
    let reopened = session::read(&path).unwrap();
    let mut edit = reopened.photos[0].edit.clone();
    assert_eq!(edit, h.app.photos[0].edit);
    edit.restore_solo();
    assert_eq!(
        edit.stack
            .layers
            .iter()
            .map(|l| (l.id, l.visible))
            .collect::<Vec<_>>(),
        visibility
    );
    h.click("stack-solo-exit", Modifiers::NONE);
    assert_eq!(
        h.app.photos[0]
            .edit
            .stack
            .layers
            .iter()
            .map(|l| (l.id, l.visible))
            .collect::<Vec<_>>(),
        visibility
    );
    h.app.undo(&h.ctx);
    assert_eq!(h.app.photos[0].edit.stack.solo.as_ref().unwrap().layer, 3);
    h.app.redo(&h.ctx);
    assert!(h.app.photos[0].edit.stack.solo.is_none());
}

fn layer_right_click(h: &mut Harness, id: u64) {
    let pos = h.rect(&format!("stack-select-{id}")).center();
    h.frame(vec![Event::PointerMoved(pos)], Modifiers::NONE);
    for pressed in [true, false] {
        h.frame(
            vec![Event::PointerButton {
                pos,
                button: PointerButton::Secondary,
                pressed,
                modifiers: Modifiers::NONE,
            }],
            Modifiers::NONE,
        );
    }
    h.frame(vec![], Modifiers::NONE);
}

#[test]
fn real_layer_context_menu_color_copy_paste_and_reset_keep_manual_strokes() {
    use crate::layer_stack::LayerColor;
    let mut h = Harness::new(1);
    h.click("toggle-Layers", Modifiers::NONE);
    h.app.photos[0].edit.settings.exposure = 0.4;
    h.app.photos[0].edit.sync_active_layer();
    h.click("stack-copy-settings", Modifiers::NONE);
    h.click("stack-new", Modifiers::NONE);
    h.app.photos[0].edit.strokes.push(Stroke {
        target: Target::Heal,
        center: [0.5, 0.5],
        radius: 0.02,
        softness: 0.7,
        erase: false,
        strength: 100.0,
    });
    h.app.photos[0].edit.sync_active_layer();
    let strokes = h.app.photos[0].edit.strokes.clone();
    h.frame(vec![], Modifiers::NONE);
    h.click("stack-paste-settings", Modifiers::NONE);
    assert_eq!(h.app.photos[0].edit.settings.exposure, 0.4);
    assert_eq!(h.app.photos[0].edit.strokes, strokes);
    h.click("stack-reset-settings", Modifiers::NONE);
    assert_eq!(h.app.photos[0].edit.settings, Settings::default());
    assert_eq!(h.app.photos[0].edit.strokes, strokes);
    h.app.undo(&h.ctx);
    assert_eq!(h.app.photos[0].edit.settings.exposure, 0.4);
    h.frame(vec![], Modifiers::NONE);
    layer_right_click(&mut h, 2);
    assert!(egui::Popup::is_any_open(&h.ctx));
    h.click("stack-color-Blue", Modifiers::NONE);
    assert_eq!(
        h.app.photos[0].edit.active_layer().unwrap().color,
        LayerColor::Blue
    );
    h.app.undo(&h.ctx);
    assert_eq!(
        h.app.photos[0].edit.active_layer().unwrap().color,
        LayerColor::None
    );
    h.app.redo(&h.ctx);
    h.frame(vec![], Modifiers::NONE);
    layer_right_click(&mut h, 2);
    h.click("stack-menu-copy", Modifiers::NONE);
    assert_eq!(h.app.photos[0].edit.stack.layers.len(), 3);
    assert_eq!(
        h.app.photos[0].edit.active_layer().unwrap().color,
        LayerColor::Blue
    );
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("labels.ron");
    session::save(&path, &h.app.session_snapshot(false)).unwrap();
    assert_eq!(
        session::read(&path).unwrap().photos[0].edit,
        h.app.photos[0].edit
    );
}

#[test]
fn layer_search_and_kind_filters_only_affect_rows_and_typing_blocks_shortcuts() {
    let mut h = Harness::new(1);
    h.click("toggle-Layers", Modifiers::NONE);
    h.click("stack-new", Modifiers::NONE);
    h.click("stack-adjust", Modifiers::NONE);
    h.app.photos[0].edit.active_layer_mut().unwrap().name = "Curves polish".into();
    let before = h.app.photos[0].edit.clone();
    let depth = h.app.photos[0].history.depths();
    h.click("stack-search", Modifiers::NONE);
    h.frame(vec![Event::Text("curves".into())], Modifiers::NONE);
    for key in [
        "stack-select-1",
        "stack-select-2",
        "stack-select-3",
        "stack-background",
    ] {
        h.ctx
            .data_mut(|d| d.remove::<Rect>(egui::Id::new(("hit", key))));
    }
    h.frame(vec![], Modifiers::NONE);
    assert!(
        h.ctx
            .data(|d| d.get_temp::<Rect>(egui::Id::new(("hit", "stack-select-3"))))
            .is_some()
    );
    assert!(
        h.ctx
            .data(|d| d.get_temp::<Rect>(egui::Id::new(("hit", "stack-select-1"))))
            .is_none()
    );
    h.key(egui::Key::Delete, Modifiers::NONE);
    h.key(egui::Key::J, Modifiers::COMMAND);
    assert_eq!(h.app.photos[0].edit, before);
    assert_eq!(h.app.photos[0].history.depths(), depth);
    h.click("stack-search-clear", Modifiers::NONE);
    h.click("stack-kind-filter", Modifiers::NONE);
    h.frame(vec![], Modifiers::NONE);
    h.click("stack-filter-Retouch", Modifiers::NONE);
    h.ctx
        .data_mut(|d| d.remove::<Rect>(egui::Id::new(("hit", "stack-select-3"))));
    h.frame(vec![], Modifiers::NONE);
    assert!(
        h.ctx
            .data(|d| d.get_temp::<Rect>(egui::Id::new(("hit", "stack-select-3"))))
            .is_none()
    );
    assert_eq!(h.app.photos[0].edit, before);
    h.click("stack-new", Modifiers::NONE);
    assert!(h.app.layer_filter == LayerFilter::All);
    assert!(h.app.layer_query.is_empty());
}

#[test]
fn layer_shortcuts_are_scoped_and_locked_controls_are_disabled() {
    let mut h = Harness::new(1);
    h.click("toggle-Layers", Modifiers::NONE);
    h.key(egui::Key::N, Modifiers::COMMAND | Modifiers::SHIFT);
    assert_eq!(h.app.photos[0].edit.stack.layers.len(), 2);
    h.key(egui::Key::F2, Modifiers::NONE);
    assert!(h.app.layer_rename.is_some());
    h.key(egui::Key::Escape, Modifiers::NONE);
    assert!(h.app.layer_rename.is_none());
    h.key(egui::Key::J, Modifiers::COMMAND);
    assert_eq!(h.app.photos[0].edit.stack.layers.len(), 3);
    h.key(egui::Key::OpenBracket, Modifiers::COMMAND);
    assert_eq!(h.app.photos[0].edit.stack.layers[1].id, 3);
    h.key(egui::Key::CloseBracket, Modifiers::COMMAND);
    assert_eq!(h.app.photos[0].edit.stack.layers[2].id, 3);
    h.click("stack-lock", Modifiers::NONE);
    let before = h.app.photos[0].edit.clone();
    h.key(egui::Key::Delete, Modifiers::NONE);
    h.click("stack-up", Modifiers::NONE);
    h.click("stack-reset-settings", Modifiers::NONE);
    h.drag(
        h.rect("layer-opacity").left_center(),
        h.rect("layer-opacity").center(),
    );
    assert_eq!(h.app.photos[0].edit, before);
    h.click("stack-row-lock-3", Modifiers::NONE);
    h.key(egui::Key::Delete, Modifiers::NONE);
    assert_eq!(h.app.photos[0].edit.stack.layers.len(), 2);
    h.click("inspector-Tools", Modifiers::NONE);
    h.key(egui::Key::J, Modifiers::COMMAND);
    h.key(egui::Key::Delete, Modifiers::NONE);
    assert_eq!(h.app.photos[0].edit.stack.layers.len(), 2);
}

#[test]
fn icon_layer_footer_and_search_remain_reachable_in_compact_windows() {
    let mut h = Harness::new(1);
    h.size = vec2(600., 560.);
    h.frame(vec![], Modifiers::NONE);
    h.click("view-menu", Modifiers::NONE);
    h.frame(vec![], Modifiers::NONE);
    h.click("view-Layers", Modifiers::NONE);
    for _ in 0..4 {
        h.frame(vec![], Modifiers::NONE);
    }
    for key in [
        "stack-search",
        "stack-new",
        "stack-adjust",
        "stack-copy",
        "stack-delete",
        "stack-background-copy",
        "stack-kind-filter",
    ] {
        let r = h.rect(key);
        assert!(
            r.top() >= 0. && r.bottom() < h.size.y && r.left() >= 0. && r.right() < h.size.x,
            "{key}: {r:?}"
        );
        assert!(r.height() >= 28., "{key} must remain easy to point at");
    }
    assert!(h.rect("layer-opacity").width() > 300.);
    h.click("stack-adjust", Modifiers::NONE);
    assert!(h.app.photos[0].edit.active_layer().unwrap().kind == LayerType::Adjustment);
}

#[test]
fn layer_rows_drag_reorder_rename_and_select_the_locked_background() {
    let mut h = Harness::new(1);
    h.click("toggle-Layers", Modifiers::NONE);
    h.click("stack-new", Modifiers::NONE);
    h.frame(vec![], Modifiers::NONE);
    h.click("stack-new", Modifiers::NONE);
    h.frame(vec![], Modifiers::NONE);
    h.time += 0.7;
    h.drag(
        h.rect("stack-select-3").center(),
        h.rect("stack-select-1").center(),
    );
    assert_eq!(h.app.photos[0].edit.stack.layers[0].id, 3);
    h.app.undo(&h.ctx);
    assert_eq!(h.app.photos[0].edit.stack.layers[0].id, 1);
    h.app.redo(&h.ctx);
    assert_eq!(h.app.photos[0].edit.stack.layers[0].id, 3);
    h.frame(vec![], Modifiers::NONE);
    h.time += 0.7;
    let row = h.rect("stack-select-3").center();
    h.click_at(row, Modifiers::NONE);
    h.click_at(row, Modifiers::NONE);
    assert!(h.app.layer_rename.is_some());
    h.frame(vec![], Modifiers::NONE);
    h.click("stack-rename-text", Modifiers::NONE);
    h.key(
        egui::Key::A,
        Modifiers {
            ctrl: true,
            ..Modifiers::NONE
        },
    );
    h.frame(vec![Event::Text("Detail clean-up".into())], Modifiers::NONE);
    h.click("stack-rename-save", Modifiers::NONE);
    assert_eq!(h.app.photos[0].edit.stack.layers[0].name, "Detail clean-up");
    h.click("stack-background", Modifiers::NONE);
    assert_eq!(h.app.photos[0].edit.stack.active, 0);
    assert!(h.app.photos[0].edit.layer_locked());
    h.click("stack-new", Modifiers::NONE);
    assert_eq!(h.app.photos[0].edit.stack.active, 4);
}

#[test]
fn real_layer_creation_edit_selection_lock_duplicate_reorder_and_delete_are_undoable() {
    let mut h = Harness::new(1);
    h.click("toggle-Layers", Modifiers::NONE);
    h.click("stack-new", Modifiers::NONE);
    assert_eq!(h.app.photos[0].edit.stack.layers.len(), 2);
    assert_eq!(h.app.photos[0].edit.stack.active, 2);
    h.click("inspector-Tools", Modifiers::NONE);
    h.app.brush = Some(Target::Heal);
    let center = h.rect("image").center();
    h.drag_path(&[center, center + vec2(28.0, 4.0)]);
    assert!(!h.app.photos[0].edit.strokes.is_empty());
    assert!(h.app.photos[0].edit.stack.layers[0].edit.strokes.is_empty());
    assert_eq!(
        h.app.photos[0].edit.stack.layers[1].edit.strokes,
        h.app.photos[0].edit.strokes
    );
    h.app.undo(&h.ctx);
    assert!(h.app.photos[0].edit.strokes.is_empty());
    h.app.redo(&h.ctx);
    assert!(!h.app.photos[0].edit.strokes.is_empty());
    h.click("inspector-Layers", Modifiers::NONE);
    h.click("stack-lock", Modifiers::NONE);
    assert!(h.app.photos[0].edit.layer_locked());
    let locked = h.app.photos[0].edit.clone();
    h.click("stack-delete", Modifiers::NONE);
    assert_eq!(h.app.photos[0].edit, locked);
    h.click("inspector-Tools", Modifiers::NONE);
    h.drag_path(&[center, center + vec2(40.0, 12.0)]);
    assert_eq!(h.app.photos[0].edit, locked);
    h.click("inspector-Layers", Modifiers::NONE);
    h.click("stack-copy", Modifiers::NONE);
    assert_eq!(h.app.photos[0].edit.stack.active, 3);
    assert!(!h.app.photos[0].edit.layer_locked());
    h.click("stack-down", Modifiers::NONE);
    assert_eq!(h.app.photos[0].edit.stack.layers[1].id, 3);
    h.click("stack-delete", Modifiers::NONE);
    assert_eq!(h.app.photos[0].edit.stack.layers.len(), 2);
    h.app.undo(&h.ctx);
    assert_eq!(h.app.photos[0].edit.stack.layers.len(), 3);
    h.app.redo(&h.ctx);
    assert_eq!(h.app.photos[0].edit.stack.layers.len(), 2);
}

#[test]
fn layer_stack_session_save_and_reopen_preserve_every_instruction_history_and_native_pixel() {
    let mut h = Harness::new(1);
    h.click("toggle-Layers", Modifiers::NONE);
    h.app.photos[0].edit.settings.exposure = 0.4;
    h.click("stack-adjust", Modifiers::NONE);
    h.app.photos[0].edit.settings.contrast = 20.0;
    h.app.photos[0].edit.sync_active_layer();
    h.click("stack-background-copy", Modifiers::NONE);
    h.app.photos[0].edit.active_layer_mut().unwrap().opacity = 25.0;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("layers.ron");
    let snapshot = h.app.session_snapshot(false);
    session::save(&path, &snapshot).unwrap();
    let restored = session::read(&path).unwrap();
    assert_eq!(restored.photos[0].edit, h.app.photos[0].edit);
    assert_eq!(
        restored.photos[0].history.depths(),
        h.app.photos[0].history.depths()
    );
    let source = &h.app.photos[0].photo.original;
    assert_eq!(
        engine::render(source, &restored.photos[0].edit, None),
        engine::render(source, &h.app.photos[0].edit, None)
    );
    let current = h.app.photos[0].edit.clone();
    let mut restored_edit = restored.photos[0].edit.clone();
    let mut restored_history = restored.photos[0].history.clone();
    restored_history.undo(&mut restored_edit);
    h.app.undo(&h.ctx);
    assert_eq!(restored_edit, h.app.photos[0].edit);
    restored_history.redo(&mut restored_edit);
    assert_eq!(restored_edit, current);
}

#[test]
fn brush_changes_submit_in_the_same_frame_and_do_not_wait_for_pause_debounce() {
    for target in [Target::Liquify, Target::Heal, Target::Clone] {
        let mut h = Harness::new(1);
        h.app.brush = Some(target);
        h.app.clone_anchor = Some((h.app.photos[0].id, [0.3, 0.3]));
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
            vec![Event::PointerMoved(start + vec2(24.0, 8.0))],
            Modifiers::NONE,
        );
        assert!(h.app.last_edit.elapsed() < Duration::from_millis(65));
        assert!(
            h.jobs.try_iter().any(|job| matches!(
                job,
                Job::Render {
                    kind: RenderKind::Preview,
                    ..
                }
            )),
            "{target:?} must submit without waiting for a stopped pointer"
        );
        assert!(h.app.render_busy);
        assert!(h.app.brush_preview.is_some());
    }
}

#[test]
fn zoomed_brush_keeps_native_pixels_while_dragging_after_release_and_after_save() {
    let mut h = Harness::new(0);
    let image = RgbaImage::from_fn(3000, 2000, |x, y| {
        image::Rgba([((x + y) % 230) as u8, 80, 120, 255])
    });
    h.incoming
        .send(Message::Imported(Ok(engine::photo_from_image(
            "large.png".into(),
            Some(PathBuf::from("large.png")),
            image,
        ))))
        .unwrap();
    h.incoming.send(Message::ImportFinished).unwrap();
    h.frame(vec![], Modifiers::NONE);
    let photo = &mut h.app.photos[0];
    photo.rendered_revision = photo.revision;
    photo.view.fit = false;
    photo.view.scale = 0.9;
    h.app.render_busy = false;
    h.jobs.try_iter().for_each(drop);
    h.frame(vec![], Modifiers::NONE);
    let (id, original_revision, crop) = h.app.detail_region_pending.unwrap();
    let old_tile = image::imageops::crop_imm(
        &*h.app.photos[0].photo.original,
        crop.x,
        crop.y,
        crop.width,
        crop.height,
    )
    .to_image();
    h.incoming
        .send(Message::DetailRegionRendered {
            id,
            revision: original_revision,
            crop,
            image: old_tile,
        })
        .unwrap();
    h.frame(vec![], Modifiers::NONE);
    h.app.brush = Some(Target::Liquify);
    let start = h.rect("canvas").center();
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
        vec![Event::PointerMoved(start + vec2(28.0, 9.0))],
        Modifiers::NONE,
    );
    let (id, revision, crop) = h
        .app
        .detail_region_pending
        .expect("Native feedback starts during the stroke");
    let tile = engine::render_region(
        &h.app.photos[0].photo.original,
        &h.app.photos[0].edit,
        None,
        crop,
        None,
    )
    .unwrap();
    let exact = Arc::new(tile.clone());
    h.incoming
        .send(Message::DetailRegionRendered {
            id,
            revision,
            crop,
            image: tile,
        })
        .unwrap();
    h.frame(vec![], Modifiers::NONE);
    assert_eq!(
        h.app.detail_tile.as_ref().unwrap().edited.as_ref(),
        exact.as_ref()
    );
    assert_eq!(h.app.detail_tile.as_ref().unwrap().key.1, revision);
    h.frame(
        vec![Event::PointerButton {
            pos: start + vec2(28.0, 9.0),
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        }],
        Modifiers::NONE,
    );
    let preview = engine::render(&h.app.photos[0].photo.preview, &h.app.photos[0].edit, None);
    h.incoming
        .send(Message::Rendered {
            kind: RenderKind::Preview,
            id,
            revision,
            image: preview,
        })
        .unwrap();
    h.incoming.send(Message::SessionSaved(Ok(()))).unwrap();
    h.frame(vec![], Modifiers::NONE);
    assert_eq!(
        h.app.detail_tile.as_ref().unwrap().edited.as_ref(),
        exact.as_ref(),
        "A completed overview or save must never replace native pixels with an enlarged preview"
    );
    assert_eq!(h.app.photos[0].history.depths(), (1, 0));
}

#[test]
fn inspector_switches_preserve_tool_gestures_history_and_the_larger_canvas() {
    let mut h = Harness::new(1);
    h.app.brush = Some(Target::Clone);
    h.app.clone_anchor = Some((h.app.photos[0].id, [0.2, 0.3]));
    let initial = h.app.photos[0].edit.clone();
    h.app.photos[0].history.record(initial);
    h.app.photos[0].edit.settings.exposure = 0.6;
    h.frame(vec![], Modifiers::NONE);
    h.app.photos[0].edit.ensure_stack();
    let edit = h.app.photos[0].edit.clone();
    let depths = h.app.photos[0].history.depths();
    let anchor = h.app.clone_anchor;
    let viewport = h.rect("canvas");
    let screen = h.ctx.content_rect();
    assert!(
        viewport.width() >= screen.width() - 395.0,
        "Tighter chrome must leave more canvas width: {viewport:?}, screen {screen:?}"
    );
    assert!(
        viewport.height() >= screen.height() - 322.0,
        "Tighter chrome must leave more canvas height: {viewport:?}, screen {screen:?}"
    );
    for (key, panel) in [
        ("toggle-History", Inspector::History),
        ("inspector-Layers", Inspector::Layers),
        ("inspector-Tools", Inspector::Tools),
    ] {
        h.click(key, Modifiers::NONE);
        assert!(h.app.inspector == Some(panel));
        assert_eq!(
            h.rect("canvas"),
            viewport,
            "Changing inspector tabs keeps the canvas stable"
        );
        assert_eq!(h.app.brush, Some(Target::Clone));
        assert_eq!(h.app.clone_anchor, anchor);
        assert_eq!(h.app.photos[0].edit, edit);
        assert_eq!(h.app.photos[0].history.depths(), depths);
    }
    h.click("inspector-close", Modifiers::NONE);
    assert!(h.app.inspector.is_none());
    // Closing from inside the already allocated SidePanel releases its width on
    // the following layout pass.
    h.frame(vec![], Modifiers::NONE);
    let closed = h.rect("canvas");
    assert!(closed.width() >= viewport.width() + 290.0);
    eprintln!(
        "Inspector layout: screen {:.1}×{:.1}, docked canvas {:.1}×{:.1}, panels hidden {:.1}×{:.1} ({:.1}% more canvas area)",
        screen.width(),
        screen.height(),
        viewport.width(),
        viewport.height(),
        closed.width(),
        closed.height(),
        (closed.area() / viewport.area() - 1.0) * 100.0
    );
    assert_eq!(h.app.brush, Some(Target::Clone));
    assert_eq!(h.app.photos[0].edit, edit);
    h.click("toggle-Layers", Modifiers::NONE);
    assert!(h.app.inspector == Some(Inspector::Layers));
    assert_eq!(h.rect("canvas"), viewport);
}

#[test]
fn history_rows_jump_to_past_and_future_once_and_new_edits_replace_redo() {
    let mut h = Harness::new(1);
    let before = Edit::default();
    let mut after = before.clone();
    after.settings.exposure = 0.15;
    assert_eq!(history_label(&before, &after), "Exposure · +0.15 EV");
    let earlier = after.clone();
    after.settings.exposure = 0.30;
    assert_eq!(history_label(&earlier, &after), "Exposure · +0.30 EV");
    for exposure in [0.4, 0.8, 1.2] {
        let photo = &mut h.app.photos[0];
        photo.history.record(photo.edit.clone());
        photo.edit.settings.exposure = exposure;
    }
    h.click("toggle-History", Modifiers::NONE);
    let revision = h.app.photos[0].revision;
    h.click("history-0", Modifiers::NONE);
    assert_eq!(h.app.photos[0].revision, revision + 1);
    assert_eq!(h.app.photos[0].edit.settings.exposure, 0.0);
    assert_eq!(h.app.photos[0].history.depths(), (0, 3));
    h.click("history-2", Modifiers::NONE);
    assert_eq!(h.app.photos[0].revision, revision + 2);
    assert_eq!(h.app.photos[0].edit.settings.exposure, 0.8);
    assert_eq!(h.app.photos[0].history.depths(), (2, 1));
    h.time += 0.7;
    h.click("history-2", Modifiers::NONE);
    assert_eq!(
        h.app.photos[0].revision,
        revision + 2,
        "Clicking current state is a no-op"
    );
    h.app.tool = Tool::Color;
    h.click("inspector-Tools", Modifiers::NONE);
    let slider = h.rect("slider-Exposure · EV");
    h.click_at(
        pos2(slider.right() - 6.0, slider.center().y),
        Modifiers::NONE,
    );
    assert!(h.app.photos[0].edit.settings.exposure > 1.5);
    assert_eq!(h.app.photos[0].history.depths(), (3, 0));
    h.click("inspector-History", Modifiers::NONE);
    assert!(h.rect("history-3").height() >= 36.0);
}

#[test]
fn layer_visibility_is_an_undoable_ui_edit_that_changes_pixels_without_erasing_sliders() {
    let mut h = Harness::new(1);
    h.app.photos[0].edit.settings.exposure = 1.0;
    h.click("toggle-Layers", Modifiers::NONE);
    let saved_settings = h.app.photos[0].edit.settings.clone();
    let original = h.app.photos[0].photo.preview.clone();
    let enabled = engine::render(&original, &h.app.photos[0].edit, None);
    assert_ne!(&enabled, original.as_ref());
    h.click("stack-visible-1", Modifiers::NONE);
    assert!(!h.app.photos[0].edit.active_layer().unwrap().visible);
    assert_eq!(h.app.photos[0].edit.settings, saved_settings);
    assert_eq!(h.app.photos[0].history.depths(), (1, 0));
    assert_eq!(
        &engine::render(&original, &h.app.photos[0].edit, None),
        original.as_ref()
    );
    h.app.undo(&h.ctx);
    assert!(h.app.photos[0].edit.active_layer().unwrap().visible);
    assert_eq!(
        engine::render(&original, &h.app.photos[0].edit, None),
        enabled
    );
    h.app.redo(&h.ctx);
    assert!(!h.app.photos[0].edit.active_layer().unwrap().visible);
    assert_eq!(h.app.photos[0].edit.settings, saved_settings);
}

#[test]
fn layer_opacity_drag_is_one_undo_step_and_retains_raw_adjustments() {
    let mut h = Harness::new(1);
    h.app.photos[0].edit.settings.exposure = 1.0;
    h.click("toggle-Layers", Modifiers::NONE);
    h.click("stack-select-1", Modifiers::NONE);
    let before = h.app.photos[0].edit.clone();
    let slider = h.rect("layer-opacity");
    let points = [0.08, 0.18, 0.32, 0.46]
        .map(|fraction| pos2(slider.left() + slider.width() * fraction, slider.center().y));
    h.drag_path(&points);
    let opacity = h.app.photos[0].edit.active_layer().unwrap().opacity;
    assert!(
        opacity > 0.0 && opacity < 100.0,
        "Drag must fade this layer: {opacity}"
    );
    assert_eq!(h.app.photos[0].edit.settings.exposure, 1.0);
    assert_eq!(h.app.photos[0].edit.effective_settings().exposure, 1.0);
    assert_eq!(
        h.app.photos[0].history.depths(),
        (1, 0),
        "A slider drag coalesces its pointer moves"
    );
    let after = h.app.photos[0].edit.clone();
    h.app.undo(&h.ctx);
    assert_eq!(h.app.photos[0].edit, before);
    h.app.redo(&h.ctx);
    assert_eq!(h.app.photos[0].edit, after);
}

#[test]
fn hidden_portrait_layers_stay_dormant_and_layer_controls_are_photo_local() {
    let mut h = Harness::new(2);
    h.app.photos[0].edit.settings.smoothing = 75.0;
    h.app.photos[0].edit.settings.blemishes = 80.0;
    h.click("toggle-Layers", Modifiers::NONE);
    h.click("stack-visible-1", Modifiers::NONE);
    h.jobs.try_iter().for_each(drop);
    h.app.auto_ai = true;
    h.frame(vec![], Modifiers::NONE);
    assert_eq!(h.app.photos[0].edit.portrait_demand(), PortraitDemand::NONE);
    assert!(
        !h.jobs
            .try_iter()
            .any(|job| matches!(job, Job::Portrait { .. }))
    );
    let first = h.app.photos[0].edit.clone();
    let first_history = h.app.photos[0].history.depths();
    h.click("photo-1", Modifiers::NONE);
    assert_eq!(h.app.current, 1);
    assert!(h.app.photos[1].edit.layers.is_default());
    h.click("stack-visible-1", Modifiers::NONE);
    assert_eq!(h.app.photos[0].edit, first);
    assert_eq!(h.app.photos[0].history.depths(), first_history);
    assert!(!h.app.photos[1].edit.active_layer().unwrap().visible);
    assert_eq!(h.app.photos[1].history.depths(), (1, 0));
    assert!(
        !h.jobs
            .try_iter()
            .any(|job| matches!(job, Job::Portrait { .. }))
    );
}

#[test]
fn compact_view_menu_and_tab_open_close_panels_without_crowding_zoom_controls() {
    let mut h = Harness::new(1);
    h.size = vec2(600.0, 560.0);
    h.frame(vec![], Modifiers::NONE);
    assert!(!h.app.compact_open);
    h.click("view-menu", Modifiers::NONE);
    // A newly opened egui Area first performs an invisible sizing pass. Read the
    // menu item bounds after its measured size has been used to place the popup.
    h.frame(vec![], Modifiers::NONE);
    h.click("view-Tools", Modifiers::NONE);
    assert!(h.app.inspector == Some(Inspector::Tools));
    assert!(h.app.compact_open);
    for key in [
        "inspector-Tools",
        "inspector-History",
        "inspector-Layers",
        "inspector-close",
    ] {
        assert!(
            h.ctx.content_rect().contains_rect(h.rect(key)),
            "Compact panel control {key} stays reachable"
        );
    }
    h.key(egui::Key::Tab, Modifiers::NONE);
    assert!(h.app.inspector.is_none());
    assert!(!h.app.compact_open);
    h.key(egui::Key::Tab, Modifiers::NONE);
    assert!(h.app.inspector == Some(Inspector::Tools));
    assert!(h.app.compact_open);
    h.click("inspector-close", Modifiers::NONE);
    h.click("view-menu", Modifiers::NONE);
    h.frame(vec![], Modifiers::NONE);
    h.click("view-Layers", Modifiers::NONE);
    assert!(h.app.inspector == Some(Inspector::Layers));
    assert!(h.app.compact_open);
    h.key(egui::Key::Tab, Modifiers::NONE);
    assert!(h.app.inspector.is_none());
    let content = h.rect("canvas-content");
    for key in ["fit", "native", "zoom-plus", "zoom-minus"] {
        assert!(
            content.contains_rect(h.rect(key)),
            "Compact zoom control {key} remains visible"
        );
    }
}

#[test]
fn tab_in_an_actual_numeric_text_editor_keeps_panels_open_and_commits_the_value() {
    let mut h = Harness::new(1);
    h.app.tool = Tool::Color;
    h.frame(vec![], Modifiers::NONE);
    h.click("value-Exposure · EV", Modifiers::NONE);
    h.frame(vec![], Modifiers::NONE);
    assert!(
        h.ctx.wants_keyboard_input(),
        "Clicking the numeric value enters text editing"
    );
    h.frame(vec![Event::Text("0.75".into())], Modifiers::NONE);
    assert!((h.app.photos[0].edit.settings.exposure - 0.75).abs() < 0.0001);
    assert!(h.app.inspector == Some(Inspector::Tools));
    h.key(egui::Key::Tab, Modifiers::NONE);
    assert!(
        h.app.inspector == Some(Inspector::Tools),
        "Tab while editing a value navigates form fields instead of hiding the inspector"
    );
    assert!((h.app.photos[0].edit.settings.exposure - 0.75).abs() < 0.0001);
    assert!(
        h.rect("slider-Contrast").height() > 0.0,
        "Other controls remain reachable after committing the value"
    );
}

#[test]
fn nullstate_neutral_import_and_manual_color_work_do_not_call_ai() {
    let mut h = Harness::new(0);
    h.app.auto_ai = true;
    let image = RgbaImage::from_pixel(420, 640, image::Rgba([80, 110, 120, 255]));
    h.incoming
        .send(Message::Imported(Ok(engine::photo_from_image(
            "untouched.png".into(),
            Some(PathBuf::from("untouched.png")),
            image,
        ))))
        .unwrap();
    h.incoming.send(Message::ImportFinished).unwrap();
    h.frame(vec![], Modifiers::NONE);
    assert!(!h.app.photos[0].ai_processing);
    assert!(h.app.photos[0].seg.is_none());
    assert!(
        !h.jobs
            .try_iter()
            .any(|job| matches!(job, Job::Portrait { .. }))
    );
    h.app.photos[0].edit.settings.exposure = 0.6;
    h.app.photos[0].edit.settings.contrast = 24.0;
    h.app.photos[0].edit.strokes.push(Stroke {
        target: Target::Heal,
        center: [0.5, 0.5],
        radius: 0.02,
        erase: false,
        strength: 100.0,
        softness: 0.7,
    });
    for target in [Target::Liquify, Target::Heal, Target::Clone, Target::Patch] {
        h.app.toggle_brush(target);
        h.frame(vec![], Modifiers::NONE);
        assert!(
            !h.jobs
                .try_iter()
                .any(|job| matches!(job, Job::Portrait { .. }))
        );
    }
    h.app.photos[0].edit.settings.smoothing = 70.0;
    h.app.photos[0]
        .edit
        .settings
        .disabled
        .push(Adjustment::Smoothing);
    h.frame(vec![], Modifiers::NONE);
    assert!(
        !h.jobs
            .try_iter()
            .any(|job| matches!(job, Job::Portrait { .. }))
    );
}

#[test]
fn nullstate_under_eye_slider_and_later_blemish_demand_coalesce_and_reuse_results() {
    let mut h = Harness::new(1);
    h.app.auto_ai = true;
    h.size = vec2(1440.0, 2400.0);
    h.frame(vec![], Modifiers::NONE);
    let slider = h.rect("slider-Under-eye bags");
    h.click_at(
        pos2(slider.right() - 8.0, slider.center().y),
        Modifiers::NONE,
    );
    let request = h
        .jobs
        .try_iter()
        .find_map(|job| match job {
            Job::Portrait {
                request,
                demand,
                base,
                ..
            } => {
                assert!(demand.geometry && demand.skin && !demand.blemishes);
                assert!(base.is_none());
                Some(request)
            }
            _ => None,
        })
        .unwrap();
    h.app.photos[0].edit.settings.blemishes = 65.0;
    h.frame(vec![], Modifiers::NONE);
    assert!(
        !h.jobs
            .try_iter()
            .any(|job| matches!(job, Job::Portrait { .. }))
    );
    let mut seg = Segmentation {
        width: 420,
        height: 640,
        prepared: PortraitDemand {
            geometry: true,
            skin: true,
            blemishes: false,
        },
        neural_blend: vec![[0.5; 3]; 420 * 640].into(),
        status: "Neural skin ready".into(),
        ..Default::default()
    };
    h.incoming
        .send(Message::PortraitReady {
            id: 1,
            request,
            complete: true,
            result: Ok(seg.clone()),
        })
        .unwrap();
    h.frame(vec![], Modifiers::NONE);
    let upgraded = h
        .jobs
        .try_iter()
        .find_map(|job| match job {
            Job::Portrait {
                request,
                demand,
                base,
                ..
            } => {
                assert_eq!(demand, PortraitDemand::ALL);
                assert_eq!(base.unwrap().prepared, seg.prepared);
                Some(request)
            }
            _ => None,
        })
        .unwrap();
    seg.prepared = PortraitDemand::ALL;
    h.incoming
        .send(Message::PortraitReady {
            id: 1,
            request: upgraded,
            complete: true,
            result: Ok(seg),
        })
        .unwrap();
    h.frame(vec![], Modifiers::NONE);
    assert!(!h.app.photos[0].ai_processing);
    h.app.undo(&h.ctx);
    h.app.redo(&h.ctx);
    h.frame(vec![], Modifiers::NONE);
    assert!(
        !h.jobs
            .try_iter()
            .any(|job| matches!(job, Job::Portrait { .. }))
    );
}

#[test]
fn nullstate_ignores_stale_provider_results_and_does_not_retry_failed_work_each_frame() {
    let mut h = Harness::new(1);
    h.app.auto_ai = true;
    h.app.photos[0].edit.settings.under_eyes = 50.0;
    h.frame(vec![], Modifiers::NONE);
    let old_request = h.app.photos[0].ai_request;
    h.jobs.try_iter().for_each(drop);
    h.app.provider = Provider::Cpu;
    h.frame(vec![], Modifiers::NONE);
    let current_request = h.app.photos[0].ai_request;
    assert!(current_request > old_request);
    h.jobs.try_iter().for_each(drop);
    let revision = h.app.photos[0].revision;
    h.incoming
        .send(Message::PortraitReady {
            id: 1,
            request: old_request,
            complete: true,
            result: Ok(Segmentation {
                prepared: PortraitDemand::ALL,
                ..Default::default()
            }),
        })
        .unwrap();
    h.incoming
        .send(Message::AiStatus {
            id: 1,
            request: old_request,
            status: model::AiStatus::new("Old request", "obsolete", None, 1.0),
        })
        .unwrap();
    h.frame(vec![], Modifiers::NONE);
    assert!(h.app.photos[0].ai_processing);
    assert_eq!(h.app.photos[0].revision, revision);
    assert!(h.app.photos[0].seg.is_none());
    h.incoming
        .send(Message::PortraitReady {
            id: 1,
            request: current_request,
            complete: true,
            result: Err(anyhow::anyhow!("missing model")),
        })
        .unwrap();
    for _ in 0..4 {
        h.frame(vec![], Modifiers::NONE);
    }
    assert!(!h.app.photos[0].ai_processing);
    assert!(
        !h.jobs
            .try_iter()
            .any(|job| matches!(job, Job::Portrait { .. }))
    );
    h.app.request_portrait(0, PortraitDemand::ALL, true);
    assert!(
        h.jobs
            .try_iter()
            .any(|job| matches!(job, Job::Portrait { force: true, .. }))
    );
}

#[test]
fn nullstate_no_face_completion_and_custom_masks_survive_incremental_results() {
    let mut h = Harness::new(1);
    h.app.auto_ai = true;
    h.app.photos[0].edit.settings.under_eyes = 50.0;
    let original = Segmentation {
        width: 420,
        height: 640,
        background: vec![0.6; 420 * 640].into(),
        skin: vec![0.3; 420 * 640].into(),
        custom_skin: true,
        ..Default::default()
    };
    h.app.photos[0].seg = Some(Arc::new(original.clone()));
    h.frame(vec![], Modifiers::NONE);
    let request = h.app.photos[0].ai_request;
    h.jobs.try_iter().for_each(drop);
    h.incoming
        .send(Message::PortraitReady {
            id: 1,
            request,
            complete: true,
            result: Ok(Segmentation {
                width: 420,
                height: 640,
                prepared: PortraitDemand::ALL,
                status: "No confident face".into(),
                ..Default::default()
            }),
        })
        .unwrap();
    for _ in 0..3 {
        h.frame(vec![], Modifiers::NONE);
    }
    let ready = h.app.photos[0].seg.as_ref().unwrap();
    assert_eq!(ready.skin, original.skin);
    assert_eq!(ready.background, original.background);
    assert!(ready.custom_skin);
    assert!(!h.app.photos[0].ai_processing);
    assert!(
        !h.jobs
            .try_iter()
            .any(|job| matches!(job, Job::Portrait { .. }))
    );
}

#[test]
fn startup_splash_blocks_edits_and_opens_after_real_ai_and_render_completion() {
    let mut h = Harness::new(1);
    h.app.startup = true;
    h.app.photos[0].ai_processing = true;
    let before = h.app.photos[0].edit.clone();
    h.frame(
        vec![Event::Key {
            key: egui::Key::A,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }],
        Modifiers::NONE,
    );
    assert!(h.app.startup);
    assert_eq!(h.app.photos[0].edit, before);
    h.app.photos[0].ai_processing = false;
    h.app.photos[0].revision += 1;
    h.frame(vec![], Modifiers::NONE);
    assert!(
        h.app.startup,
        "AI completion still needs a rendered preview"
    );
    h.app.photos[0].rendered_revision = h.app.photos[0].revision;
    h.frame(vec![], Modifiers::NONE);
    assert!(!h.app.startup);
}

#[test]
fn automatic_recovery_restores_latest_history_without_starting_a_demo() {
    let directory = tempfile::tempdir().unwrap();
    let latest = directory.path().join("latest.ron");
    let older = directory.path().join("older.ron");
    let mut h = Harness::new(1);
    h.app.photos[0].photo.path = None;
    let original_edit = h.app.photos[0].edit.clone();
    h.app.photos[0].history.record(original_edit);
    h.app.photos[0].edit.settings.under_eyes = 65.0;
    let current_edit = h.app.photos[0].edit.clone();
    h.app.photos[0].history.record(current_edit);
    h.app.photos[0].edit.settings.under_eyes = 85.0;
    let photo = &mut h.app.photos[0];
    assert!(photo.history.undo(&mut photo.edit));
    let original = photo.photo.clone();
    let saved = h.app.session_snapshot(false);
    session::save(&latest, &saved).unwrap();
    h.app.photos.clear();
    assert!(h.app.resume_latest_workspace(vec![latest.clone(), older]));
    assert!(matches!(h.jobs.try_recv().unwrap(), Job::ReadRecovery(path) if path == latest));
    assert!(
        h.jobs.is_empty(),
        "No demo import can race the saved session"
    );
    h.incoming
        .send(Message::SessionRead(Ok(
            session::read_recovery(&latest).unwrap()
        )))
        .unwrap();
    h.frame(vec![], Modifiers::NONE);
    assert!(
        h.jobs
            .try_iter()
            .any(|job| matches!(job, Job::OpenSession { photos } if photos == vec![None]))
    );
    h.incoming.send(Message::Imported(Ok(original))).unwrap();
    h.incoming.send(Message::ImportFinished).unwrap();
    h.frame(vec![], Modifiers::NONE);
    assert!(!h.app.automatic_recovery);
    assert!(h.app.recovery_candidates.is_empty());
    assert_eq!(h.app.photos[0].history.depths(), (1, 1));
    assert_eq!(h.app.photos[0].edit.settings.under_eyes, 65.0);
    assert!(
        latest.is_file(),
        "Keep recovery until the replacement saves"
    );
}

#[test]
fn automatic_recovery_skips_unusable_generations_before_starting_a_demo() {
    let directory = tempfile::tempdir().unwrap();
    let latest = directory.path().join("latest.ron");
    let previous = latest.with_extension("previous");
    let older = directory.path().join("older.ron");
    std::fs::write(&previous, "saved previous generation").unwrap();
    let mut h = Harness::new(1);
    let mut saved = h.app.session_snapshot(false);
    saved.photos[0].path = Some(directory.path().join("invalid.png"));
    std::fs::write(
        saved.photos[0].path.as_ref().unwrap(),
        "invalid image bytes",
    )
    .unwrap();
    h.app.photos.clear();
    assert!(
        h.app
            .resume_latest_workspace(vec![latest.clone(), older.clone()])
    );
    h.jobs.try_iter().for_each(drop);
    h.incoming.send(Message::SessionRead(Ok(saved))).unwrap();
    h.frame(vec![], Modifiers::NONE);
    h.jobs.try_iter().for_each(drop);
    h.incoming
        .send(Message::Imported(Err(anyhow::anyhow!("invalid image"))))
        .unwrap();
    h.incoming.send(Message::ImportFinished).unwrap();
    h.frame(vec![], Modifiers::NONE);
    assert!(
        h.jobs
            .try_iter()
            .any(|job| matches!(job, Job::ReadRecovery(path) if path == previous))
    );
    h.incoming
        .send(Message::SessionRead(Err(anyhow::anyhow!(
            "corrupt previous recovery"
        ))))
        .unwrap();
    h.frame(vec![], Modifiers::NONE);
    assert!(
        h.jobs
            .try_iter()
            .any(|job| matches!(job, Job::ReadRecovery(path) if path == older))
    );
    h.incoming
        .send(Message::SessionRead(Err(anyhow::anyhow!(
            "corrupt recovery"
        ))))
        .unwrap();
    h.frame(vec![], Modifiers::NONE);
    assert!(h.jobs.try_iter().any(|job| matches!(job, Job::Demo)));
    assert!(!h.app.automatic_recovery);
    assert!(
        previous.is_file(),
        "Skipped generations must remain untouched"
    );
}

#[test]
fn minimum_window_keeps_zoom_controls_inside_canvas_content() {
    let mut h = Harness::new(1);
    h.size = vec2(600.0 / 1.1, 560.0 / 1.1);
    h.frame(vec![], Modifiers::NONE);
    let content = h.rect("canvas-content");
    for control in ["fit", "native", "zoom-plus", "zoom-minus"] {
        assert!(
            content.contains_rect(h.rect(control)),
            "{control} must remain fully visible above the filmstrip"
        );
    }
    h.click("native", Modifiers::NONE);
    assert_eq!(h.app.photos[0].view.scale, 1.0);
    assert!(!h.app.photos[0].view.fit);
    h.click("fit", Modifiers::NONE);
    assert!(h.app.photos[0].view.fit);
}

#[test]
fn compact_startup_can_open_workspace_while_batch_import_continues() {
    let mut h = Harness::new(1);
    h.size = vec2(600.0, 560.0);
    h.app.startup = true;
    h.app.importing = true;
    h.app.photos[0].ai_processing = true;
    h.frame(vec![], Modifiers::NONE);
    assert!(h.rect("startup-open").height() >= 32.0);
    h.click("startup-open", Modifiers::NONE);
    assert!(!h.app.startup);
    assert!(h.app.importing);
    assert!(h.app.photos[0].ai_processing);
}

#[test]
fn failed_final_ai_analysis_is_visible_and_does_not_block_startup() {
    let mut h = Harness::new(1);
    h.app.startup = true;
    h.app.photos[0].ai_processing = true;
    h.app.photos[0].seg = Some(Arc::new(Segmentation::default()));
    h.incoming
        .send(Message::PortraitReady {
            request: 0,
            id: h.app.photos[0].id,
            complete: true,
            result: Err(anyhow::anyhow!("GPU and CPU unavailable")),
        })
        .unwrap();
    h.frame(vec![], Modifiers::NONE);
    assert!(!h.app.startup);
    let status = h.app.photos[0].ai_status.as_ref().unwrap();
    assert_eq!(status.stage, "Portrait AI unavailable");
    assert!(status.detail.contains("GPU and CPU unavailable"));
}

#[test]
fn ai_progress_is_photo_local_and_preserved_through_repeated_face_crops() {
    let mut h = Harness::new(2);
    let id = h.app.photos[0].id;
    h.app.photos[0].ai_processing = true;
    h.app.photos[1].ai_processing = true;
    let status = |progress| {
        model::AiStatus::new(
            "Checking GPU accuracy",
            "CPU comparison",
            Some(Provider::DirectMl),
            progress,
        )
    };
    h.incoming
        .send(Message::AiStatus {
            request: 0,
            id: h.app.photos[1].id,
            status: status(0.9),
        })
        .unwrap();
    h.frame(vec![], Modifiers::NONE);
    assert!(h.app.photos[0].ai_status.is_none());
    assert_eq!(h.app.photos[1].ai_status.as_ref().unwrap().progress, 0.9);
    h.incoming
        .send(Message::AiStatus {
            request: 0,
            id: 999,
            status: status(0.99),
        })
        .unwrap();
    h.incoming
        .send(Message::AiStatus {
            request: 0,
            id,
            status: status(0.7),
        })
        .unwrap();
    h.incoming
        .send(Message::AiStatus {
            request: 0,
            id,
            status: status(0.2),
        })
        .unwrap();
    h.frame(vec![], Modifiers::NONE);
    assert_eq!(h.app.photos[0].ai_status.as_ref().unwrap().progress, 0.7);
    h.incoming
        .send(Message::AiStatus {
            request: 0,
            id,
            status: status(1.0),
        })
        .unwrap();
    h.frame(vec![], Modifiers::NONE);
    h.incoming
        .send(Message::AiStatus {
            request: 0,
            id,
            status: status(0.05),
        })
        .unwrap();
    h.frame(vec![], Modifiers::NONE);
    assert_eq!(
        h.app.photos[0].ai_status.as_ref().unwrap().progress,
        0.05,
        "A new analysis starts its own progress"
    );
}

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
    app: HasturApp,
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
        let app = HasturApp::from_channels(tx, rx, false, vec![], None);
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
                    None,
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
    fn key(&mut self, key: egui::Key, modifiers: Modifiers) {
        for pressed in [true, false] {
            self.frame(
                vec![Event::Key {
                    key,
                    physical_key: None,
                    pressed,
                    repeat: false,
                    modifiers,
                }],
                modifiers,
            );
        }
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
                assert!(target == Target::Patch || output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Circle(c)
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
            request: 0,
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
            request: 0,
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
    // A stale duplicate must not release a newer in-flight render. Acknowledge
    // the actual pending request before expecting the final brush revision.
    if let Some((id, revision, _)) = &h.app.preview_cancel {
        h.incoming
            .send(Message::RenderCancelled {
                id: *id,
                revision: *revision,
                kind: RenderKind::Preview,
            })
            .unwrap();
    }
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
fn actual_manual_workers_update_fitted_and_native_canvases_before_pointer_release() {
    fn pump(h: &mut Harness, ready: impl Fn(&HasturApp) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            h.frame(vec![], Modifiers::NONE);
            if ready(&h.app) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "real worker did not update the canvas while dragging"
            );
            std::thread::sleep(Duration::from_millis(4));
        }
    }
    let tools = [Target::Liquify, Target::Heal, Target::Clone, Target::Patch];
    for (native, tool) in [false, true]
        .into_iter()
        .flat_map(|native| tools.map(|tool| (native, tool)))
    {
        let mut h = Harness::new(0);
        let image = RgbaImage::from_fn(1600, 2100, |x, y| {
            image::Rgba([
                (60 + (x * 5 + y * 3) % 140) as u8,
                (80 + (x * 3 + y * 7) % 130) as u8,
                110,
                255,
            ])
        });
        h.incoming
            .send(Message::Imported(Ok(engine::photo_from_image(
                "live.png".into(),
                None,
                image,
            ))))
            .unwrap();
        h.incoming.send(Message::ImportFinished).unwrap();
        h.frame(vec![], Modifiers::NONE);
        let p = &mut h.app.photos[0];
        p.rendered_revision = p.revision;
        p.view.fit = false;
        p.view.scale = if native { 0.9 } else { 0.25 };
        let initial_revision = p.revision;
        h.app.render_busy = false;
        h.app.preview_cancel = None;
        h.app.detail_region_pending = None;
        h.app.detail_cancel = None;
        let (tx, rx) = worker::start(h.ctx.clone());
        h.app.tx = tx;
        h.app.rx = rx;
        if native {
            pump(&mut h, |app| app.detail_region.is_some());
        } else {
            h.frame(vec![], Modifiers::NONE);
        }
        h.app.brush = Some(tool);
        h.app.brush_radius = 0.04;
        h.app.brush_strength = 100.;
        let rect = h.rect("image");
        let start = h.rect("canvas").center();
        h.app.clone_anchor = Some((h.app.photos[0].id, [0.38, 0.46]));
        if tool == Target::Patch {
            let c = [
                (start.x - rect.left()) / rect.width(),
                (start.y - rect.top()) / rect.height(),
            ];
            h.app.patch_draft.boundary = vec![
                [c[0] - 0.025, c[1] - 0.025],
                [c[0] + 0.025, c[1] - 0.025],
                [c[0] + 0.025, c[1] + 0.025],
                [c[0] - 0.025, c[1] + 0.025],
            ];
        }
        h.frame(
            vec![
                Event::PointerMoved(start),
                primary(start, true, Modifiers::NONE),
            ],
            Modifiers::NONE,
        );
        h.frame(
            vec![Event::PointerMoved(start + vec2(31., 12.))],
            Modifiers::NONE,
        );
        if tool == Target::Patch {
            pump(&mut h, |app| {
                app.patch_feedback
                    .frame
                    .as_ref()
                    .is_some_and(|f| f.image != f.original)
            });
            let first = h.app.patch_feedback.frame.as_ref().unwrap().request;
            h.frame(
                vec![Event::PointerMoved(start + vec2(57., 21.))],
                Modifiers::NONE,
            );
            pump(&mut h, |app| {
                app.patch_feedback
                    .frame
                    .as_ref()
                    .is_some_and(|f| f.request > first)
            });
            assert!(
                h.app.photos[0].edit.patches.is_empty(),
                "donor feedback is temporary"
            );
        } else {
            pump(&mut h, |app| {
                if native {
                    app.detail_tile
                        .as_ref()
                        .is_some_and(|t| t.key.1 > initial_revision && t.edited != t.original)
                } else {
                    app.photos[0].rendered_revision > initial_revision
                        && app.photos[0].edited != app.photos[0].photo.preview
                }
            });
            let first = if native {
                h.app.detail_tile.as_ref().unwrap().key.1
            } else {
                h.app.photos[0].rendered_revision
            };
            h.frame(
                vec![Event::PointerMoved(start + vec2(57., 21.))],
                Modifiers::NONE,
            );
            pump(&mut h, |app| {
                if native {
                    app.detail_tile.as_ref().is_some_and(|t| t.key.1 > first)
                } else {
                    app.photos[0].rendered_revision > first
                }
            });
        }
        assert!(
            h.ctx.input(|i| i.pointer.primary_down()),
            "updates must arrive before release: {tool:?}"
        );
        h.frame(
            vec![primary(start + vec2(57., 21.), false, Modifiers::NONE)],
            Modifiers::NONE,
        );
        assert_eq!(
            h.app.photos[0].history.depths(),
            (1, 0),
            "one gesture remains one undo: {tool:?}"
        );
    }
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
    h.app.photos[0].photo.source_dimensions = (840, 420);
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
            queue: 0,
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
fn batch_scheduler_defers_unopened_photos_and_rejects_their_stale_results() {
    let mut h = Harness::new(3);
    h.app.photos.iter_mut().for_each(|p| p.selected = true);
    h.app.photos[0].edit.settings.exposure = 0.3;
    h.app.sync();
    h.app.last_edit = Instant::now() - Duration::from_secs(1);
    h.frame(vec![], Modifiers::NONE);
    assert!(
        !h.jobs
            .try_iter()
            .any(|j| matches!(j, Job::Render { id: 2 | 3, .. }))
    );
    let id = h.app.photos[1].id;
    let revision = h.app.photos[1].revision;
    h.incoming
        .send(Message::Rendered {
            kind: RenderKind::Preview,
            id,
            revision,
            image: RgbaImage::new(420, 640),
        })
        .unwrap();
    h.frame(vec![], Modifiers::NONE);
    assert_ne!(h.app.photos[1].rendered_revision, revision);
    h.app.select_photo(1, false, false);
    h.frame(vec![], Modifiers::NONE);
    assert!(
        h.jobs
            .try_iter()
            .any(|j| matches!(j, Job::Render { id: 2, .. }))
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
