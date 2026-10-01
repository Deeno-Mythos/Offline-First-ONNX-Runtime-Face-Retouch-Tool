use crate::{
    engine::{self, Edit, ExportOptions, ExportSize, Photo, Preset, Segmentation},
    model::{self, Kind, Provider},
};
use crossbeam_channel::{Receiver, Sender};
use image::RgbaImage;
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenderKind {
    Preview,
    Hover(usize),
}

pub type ExportPhoto = (Photo, Edit, Option<Arc<Segmentation>>, [f32; 2]);

pub enum Job {
    SaveSession {
        path: PathBuf,
        session: crate::session::Session,
    },
    ReadSession(PathBuf),
    SaveRecovery {
        path: PathBuf,
        session: crate::session::Session,
        generation: u64,
    },
    ReadRecovery(PathBuf),
    Import(Vec<PathBuf>),
    OpenSession {
        photos: Vec<Option<PathBuf>>,
    },
    Demo,
    Portrait {
        id: u64,
        force: bool,
        provider: Provider,
        image: Arc<RgbaImage>,
    },
    Presets {
        id: u64,
        revision: u64,
        image: Arc<RgbaImage>,
        edit: Edit,
        seg: Option<Arc<Segmentation>>,
        presets: Vec<Preset>,
    },
    Render {
        kind: RenderKind,
        id: u64,
        revision: u64,
        image: Arc<RgbaImage>,
        edit: Edit,
        seg: Option<Arc<Segmentation>>,
        cancel: Arc<AtomicBool>,
    },
    DetailRegion {
        id: u64,
        revision: u64,
        image: Arc<RgbaImage>,
        edit: Edit,
        seg: Option<Arc<Segmentation>>,
        crop: engine::Crop,
        cancel: Arc<AtomicBool>,
    },
    HoverRegion {
        id: u64,
        revision: u64,
        index: usize,
        image: Arc<RgbaImage>,
        edit: Edit,
        seg: Option<Arc<Segmentation>>,
        crop: engine::Crop,
        cancel: Arc<AtomicBool>,
    },
    Infer {
        id: u64,
        path: PathBuf,
        kind: Kind,
        provider: Provider,
        image: Arc<RgbaImage>,
    },
    Export {
        photos: Vec<ExportPhoto>,
        directory: PathBuf,
        size: ExportSize,
        png: bool,
        quality: u8,
        cancel: Arc<AtomicBool>,
    },
}
pub enum Message {
    HoverRegionRendered {
        id: u64,
        revision: u64,
        index: usize,
        crop: engine::Crop,
        image: RgbaImage,
    },
    HoverRegionCancelled {
        id: u64,
        revision: u64,
        index: usize,
        crop: engine::Crop,
    },
    DetailRegionRendered {
        id: u64,
        revision: u64,
        crop: engine::Crop,
        image: RgbaImage,
    },
    DetailRegionCancelled {
        id: u64,
        revision: u64,
        crop: engine::Crop,
    },
    SessionSaved(anyhow::Result<()>),
    RecoverySaved {
        generation: u64,
        result: anyhow::Result<()>,
    },
    SessionRead(anyhow::Result<crate::session::Session>),
    RenderCancelled {
        id: u64,
        revision: u64,
        kind: RenderKind,
    },
    Imported(anyhow::Result<Photo>),
    PresetThumb {
        id: u64,
        revision: u64,
        index: usize,
        image: RgbaImage,
    },
    RenderStarted {
        id: u64,
        revision: u64,
        kind: RenderKind,
    },
    ExportFinished {
        finished: usize,
        total: usize,
        cancelled: bool,
    },
    Rendered {
        kind: RenderKind,
        id: u64,
        revision: u64,
        image: RgbaImage,
    },
    Segmented {
        id: u64,
        kind: Kind,
        result: anyhow::Result<Segmentation>,
    },
    PortraitReady {
        id: u64,
        complete: bool,
        result: anyhow::Result<Segmentation>,
    },
    Exported {
        finished: usize,
        total: usize,
        result: anyhow::Result<PathBuf>,
    },
    ImportFinished,
    ExportReport(anyhow::Result<PathBuf>),
}

pub fn start(ctx: eframe::egui::Context) -> (Sender<Job>, Receiver<Message>) {
    let (tx, rx) = crossbeam_channel::unbounded();
    let (out, results) = crossbeam_channel::unbounded();
    let (portrait_tx, portrait_rx) =
        crossbeam_channel::unbounded::<(u64, Provider, bool, Arc<RgbaImage>)>();
    let portrait_out = out.clone();
    let portrait_ctx = ctx.clone();
    let (session_tx, session_rx) = crossbeam_channel::unbounded::<Job>();
    let session_out = out.clone();
    let session_ctx = ctx.clone();
    std::thread::Builder::new()
        .name("astra-session-worker".into())
        .spawn(move || {
            while let Ok(job) = session_rx.recv() {
                let message = match job {
                    Job::SaveSession { path, session } => {
                        Message::SessionSaved(crate::session::save(&path, &session))
                    }
                    Job::ReadSession(path) => Message::SessionRead(crate::session::read(&path)),
                    Job::ReadRecovery(path) => {
                        Message::SessionRead(crate::session::read_recovery(&path))
                    }
                    Job::SaveRecovery {
                        path,
                        session,
                        generation,
                    } => Message::RecoverySaved {
                        generation,
                        result: crate::session::save_recovery(&path, &session, generation),
                    },
                    _ => continue,
                };
                let _ = session_out.send(message);
                session_ctx.request_repaint();
            }
        })
        .expect("cannot start session worker");
    let (import_tx, import_rx) = crossbeam_channel::unbounded::<Vec<Option<PathBuf>>>();
    let import_out = out.clone();
    let import_ctx = ctx.clone();
    std::thread::Builder::new()
        .name("astra-import-worker".into())
        .spawn(move || {
            while let Ok(photos) = import_rx.recv() {
                let send = |message| {
                    let _ = import_out.send(message);
                    import_ctx.request_repaint();
                };
                for path in photos {
                    if let Some(path) = path {
                        send(Message::Imported(
                            engine::load_photo(&path)
                                .map_err(|e| e.context(path.display().to_string())),
                        ));
                    } else {
                        send(Message::Imported(
                            image::load_from_memory(include_bytes!("../assets/demo-portrait.png"))
                                .map(|i| {
                                    engine::photo_from_image(
                                        "Studio portrait.png".into(),
                                        None,
                                        i.into_rgba8(),
                                    )
                                })
                                .map_err(anyhow::Error::from),
                        ));
                    }
                }
                send(Message::ImportFinished);
            }
        })
        .expect("cannot start import worker");
    std::thread::Builder::new()
        .name("astra-portrait-ai".into())
        .spawn(move || {
            while let Ok((id, provider, force, image)) = portrait_rx.recv() {
                let result = model::analyze_shared_cached(&image, provider, force, |seg| {
                    let _ = portrait_out.send(Message::PortraitReady {
                        id,
                        complete: false,
                        result: Ok(seg),
                    });
                    portrait_ctx.request_repaint();
                });
                let _ = portrait_out.send(Message::PortraitReady {
                    id,
                    complete: true,
                    result,
                });
                portrait_ctx.request_repaint();
            }
        })
        .expect("cannot start portrait AI worker");
    std::thread::Builder::new()
        .name("astra-image-worker".into())
        .spawn(move || {
            let send = |message| {
                let _ = out.send(message);
                ctx.request_repaint();
            };
            let mut renderer = engine::Renderer::default();
            let mut queue = std::collections::VecDeque::new();
            loop {
                if queue.is_empty() {
                    match rx.recv() {
                        Ok(job) => queue.push_back(job),
                        Err(_) => break,
                    };
                }
                queue.extend(rx.try_iter());
                let index = queue
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, job)| priority(job))
                    .map(|(i, _)| i)
                    .unwrap();
                let job = queue.remove(index).unwrap();
                match job {
                    job @ (Job::SaveSession { .. }
                    | Job::ReadSession(_)
                    | Job::SaveRecovery { .. }
                    | Job::ReadRecovery(_)) => {
                        let _ = session_tx.send(job);
                    }
                    Job::Portrait {
                        id,
                        force,
                        provider,
                        image,
                    } => {
                        let _ = portrait_tx.send((id, provider, force, image));
                    }
                    Job::OpenSession { photos } => {
                        let _ = import_tx.send(photos);
                    }
                    Job::Import(paths) => {
                        let _ = import_tx.send(paths.into_iter().map(Some).collect());
                    }
                    Job::Demo => {
                        let _ = import_tx.send(vec![None]);
                    }
                    Job::Presets {
                        id,
                        revision,
                        image,
                        edit,
                        seg,
                        presets,
                    } => {
                        let small = image::imageops::thumbnail(&*image, 120, 150);
                        for (index, preset) in presets.into_iter().enumerate() {
                            let mut variant = edit.clone();
                            variant.settings = preset.settings;
                            send(Message::PresetThumb {
                                id,
                                revision,
                                index,
                                image: engine::render(&small, &variant, seg.as_deref()),
                            });
                        }
                    }
                    Job::Render {
                        kind,
                        id,
                        revision,
                        image,
                        edit,
                        seg,
                        cancel,
                    } => {
                        if !cancel.load(Ordering::Relaxed) {
                            send(Message::RenderStarted { id, revision, kind });
                        }
                        match renderer.render(&image, &edit, seg.as_ref(), Some(&cancel)) {
                            Ok(image) => send(Message::Rendered {
                                kind,
                                id,
                                revision,
                                image,
                            }),
                            Err(_) => send(Message::RenderCancelled { id, revision, kind }),
                        }
                    }
                    Job::DetailRegion {
                        id,
                        revision,
                        image,
                        edit,
                        seg,
                        crop,
                        cancel,
                    } => {
                        match renderer.render_region(
                            &image,
                            &edit,
                            seg.as_ref(),
                            crop,
                            Some(&cancel),
                        ) {
                            Ok(image) => send(Message::DetailRegionRendered {
                                id,
                                revision,
                                crop,
                                image,
                            }),
                            Err(_) => send(Message::DetailRegionCancelled { id, revision, crop }),
                        }
                    }
                    Job::HoverRegion {
                        id,
                        revision,
                        index,
                        image,
                        edit,
                        seg,
                        crop,
                        cancel,
                    } => {
                        match renderer.render_region(
                            &image,
                            &edit,
                            seg.as_ref(),
                            crop,
                            Some(&cancel),
                        ) {
                            Ok(image) => send(Message::HoverRegionRendered {
                                id,
                                revision,
                                index,
                                crop,
                                image,
                            }),
                            Err(_) => send(Message::HoverRegionCancelled {
                                id,
                                revision,
                                index,
                                crop,
                            }),
                        }
                    }
                    Job::Infer {
                        id,
                        path,
                        kind,
                        provider,
                        image,
                    } => send(Message::Segmented {
                        id,
                        kind,
                        result: model::infer(&path, kind, provider, &image),
                    }),
                    Job::Export {
                        photos,
                        directory,
                        size,
                        png,
                        quality,
                        cancel,
                    } => {
                        let export_out = out.clone();
                        let export_ctx = ctx.clone();
                        std::thread::Builder::new()
                            .name("astra-export-worker".into())
                            .spawn(move || {
                                let send = |message| {
                                    let _ = export_out.send(message);
                                    export_ctx.request_repaint();
                                };
                                let total = photos.len();
                                let mut finished = 0;
                                let mut report = Vec::new();
                                let reference_mean =
                                    photos.first().map_or(0.5, |(p, _, _, _)| p.analysis.mean);
                                for (photo, edit, seg, center) in photos {
                                    if cancel.load(Ordering::Relaxed) {
                                        break;
                                    }
                                    let options = ExportOptions {
                                        size,
                                        png,
                                        quality,
                                        center,
                                        cancel: Some(cancel.clone()),
                                    };
                                    let result = engine::export_with_options(
                                        &photo,
                                        &edit,
                                        seg.as_deref(),
                                        &directory,
                                        &options,
                                    );
                                    if cancel.load(Ordering::Relaxed) && result.is_err() {
                                        break;
                                    }
                                    let note =
                                        if (photo.analysis.mean - reference_mean).abs() > 0.15 {
                                            "Exposure outlier; review skin and shadows"
                                        } else {
                                            "Inspect local masks and framing"
                                        };
                                    report.push((
                                        photo.name.clone(),
                                        result.as_ref().map_or_else(
                                            |e| format!("Failed: {e}"),
                                            |p| {
                                                p.file_name()
                                                    .unwrap_or_default()
                                                    .to_string_lossy()
                                                    .into_owned()
                                            },
                                        ),
                                        engine::changelog(&edit),
                                        note.into(),
                                    ));
                                    finished += 1;
                                    send(Message::Exported {
                                        finished,
                                        total,
                                        result,
                                    });
                                }
                                if !report.is_empty() {
                                    send(Message::ExportReport(engine::export_report(
                                        &directory, &report,
                                    )));
                                }
                                send(Message::ExportFinished {
                                    finished,
                                    total,
                                    cancelled: cancel.load(Ordering::Relaxed),
                                });
                            })
                            .expect("cannot start export worker");
                    }
                }
            }
        })
        .expect("cannot start image worker");
    (tx, results)
}

fn priority(job: &Job) -> u8 {
    match job {
        Job::Portrait { .. }
        | Job::Export { .. }
        | Job::SaveSession { .. }
        | Job::ReadSession(_)
        | Job::SaveRecovery { .. }
        | Job::ReadRecovery(_) => 0,
        Job::Render {
            kind: RenderKind::Preview,
            ..
        } => 1,
        Job::DetailRegion { .. } | Job::Infer { .. } => 2,
        Job::Render {
            kind: RenderKind::Hover(_),
            ..
        }
        | Job::HoverRegion { .. } => 3,
        Job::Presets { .. } => 4,
        Job::Import(_) | Job::OpenSession { .. } | Job::Demo => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn thumbnail_worker_renders_distinct_preset_looks() {
        let (tx, rx) = start(eframe::egui::Context::default());
        let image = Arc::new(RgbaImage::from_pixel(
            120,
            150,
            image::Rgba([130, 90, 80, 255]),
        ));
        let presets = engine::presets();
        tx.send(Job::Presets {
            id: 1,
            revision: 7,
            image,
            edit: Edit::default(),
            seg: None,
            presets,
        })
        .unwrap();
        let mut results = Vec::new();
        for _ in 0..5 {
            match rx.recv_timeout(Duration::from_secs(5)).unwrap() {
                Message::PresetThumb {
                    id: 1,
                    revision: 7,
                    image,
                    ..
                } => results.push(image),
                _ => panic!("Unexpected thumbnail result"),
            }
        }
        assert_ne!(results[0], results[4]);
        assert_eq!(results[0].dimensions(), (120, 150));
    }

    #[test]
    fn cancel_batch_preserves_completed_file_and_stops_remaining_work() {
        let (tx, rx) = start(eframe::egui::Context::default());
        let small = engine::photo_from_image(
            "completed.png".into(),
            None,
            RgbaImage::from_pixel(32, 32, image::Rgba([80, 80, 80, 255])),
        );
        let large = engine::photo_from_image(
            "cancelled.png".into(),
            None,
            RgbaImage::from_pixel(2000, 2500, image::Rgba([100, 100, 100, 255])),
        );
        let original = large.original.clone();
        let directory = tempfile::tempdir().unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        tx.send(Job::Export {
            photos: vec![
                (small, Edit::default(), None, [0.5; 2]),
                (large.clone(), Edit::default(), None, [0.5; 2]),
                (large, Edit::default(), None, [0.5; 2]),
            ],
            directory: directory.path().into(),
            size: ExportSize::Original,
            png: true,
            quality: 95,
            cancel: cancel.clone(),
        })
        .unwrap();
        let mut finished = false;
        for _ in 0..8 {
            match rx.recv_timeout(Duration::from_secs(10)).unwrap() {
                Message::Exported {
                    finished: 1,
                    result,
                    ..
                } => {
                    assert!(result.unwrap().is_file());
                    cancel.store(true, Ordering::Relaxed);
                }
                Message::ExportFinished {
                    finished: 1,
                    total: 3,
                    cancelled: true,
                } => {
                    finished = true;
                    break;
                }
                Message::ExportReport(result) => {
                    result.unwrap();
                }
                _ => panic!("Batch continued after cancellation"),
            }
        }
        assert!(finished);
        let names: Vec<_> = std::fs::read_dir(directory.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert!(names.contains(&"completed_v1.png".into()));
        assert!(!names.iter().any(|s| s.starts_with("cancelled")));
        assert!(original.pixels().all(|p| p.0 == [100, 100, 100, 255]));
    }
}
