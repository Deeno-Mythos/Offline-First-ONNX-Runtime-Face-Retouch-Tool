use crate::{
    engine::{self, Edit, ExportOptions, ExportSize, Photo, Preset, Segmentation},
    model::{self, Kind, Provider},
    nullstate::PortraitDemand,
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

struct PortraitRequest {
    cancel: Arc<AtomicBool>,
    id: u64,
    request: u64,
    provider: Provider,
    force: bool,
    demand: PortraitDemand,
    base: Option<Arc<Segmentation>>,
    image: Arc<RgbaImage>,
}

/// Keep models warm for a series of edits, then return their weights to NullState.
const AI_IDLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(90);

pub enum Job {
    ReleaseMemory,
    LoadAsset {
        id: u64,
        request: u64,
        path: PathBuf,
        cancel: Arc<AtomicBool>,
    },
    ImportFolder(PathBuf),
    PatchPreview {
        id: u64,
        revision: u64,
        request: u64,
        image: Arc<RgbaImage>,
        edit: Edit,
        seg: Option<Arc<Segmentation>>,
        crop: Option<engine::Crop>,
        cancel: Arc<AtomicBool>,
    },
    LayerThumbnails {
        id: u64,
        revision: u64,
        image: Arc<RgbaImage>,
        layers: Vec<crate::layer_stack::Layer>,
        seg: Option<Arc<Segmentation>>,
    },
    #[cfg(test)]
    BackgroundBarrier {
        entered: Sender<()>,
        release: Receiver<()>,
    },
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
        cancel: Arc<AtomicBool>,
        id: u64,
        request: u64,
        force: bool,
        provider: Provider,
        demand: PortraitDemand,
        base: Option<Arc<Segmentation>>,
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
        concurrency: u8,
        queue: u64,
        provider: Provider,
        photos: Vec<ExportPhoto>,
        directory: PathBuf,
        size: ExportSize,
        png: bool,
        quality: u8,
        cancel: Arc<AtomicBool>,
    },
}
pub enum Message {
    AssetCaches {
        id: u64,
        paths: Vec<PathBuf>,
    },
    AssetLoaded {
        id: u64,
        request: u64,
        result: anyhow::Result<Photo>,
    },
    PatchPreview {
        id: u64,
        revision: u64,
        request: u64,
        crop: Option<engine::Crop>,
        result: anyhow::Result<RgbaImage>,
    },
    LayerThumbnails {
        id: u64,
        revision: u64,
        images: Vec<(u64, RgbaImage)>,
    },
    AiStatus {
        id: u64,
        request: u64,
        status: model::AiStatus,
    },
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
        queue: u64,
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
        request: u64,
        complete: bool,
        result: anyhow::Result<Segmentation>,
    },
    Exported {
        queue: u64,
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
    let (preview_tx, preview_rx) = crossbeam_channel::unbounded::<Job>();
    let (background_tx, background_rx) = crossbeam_channel::unbounded::<Job>();
    let (detail_tx, detail_rx) = crossbeam_channel::unbounded::<Job>();
    let detail_out = out.clone();
    let detail_ctx = ctx.clone();
    std::thread::Builder::new()
        .name("hastur-native-preview".into())
        .spawn(move || {
            let mut renderer = engine::Renderer::default();
            let pool = rayon::ThreadPoolBuilder::new()
                .num_threads(
                    std::thread::available_parallelism()
                        .map_or(2, usize::from)
                        .div_ceil(2)
                        .clamp(1, 4),
                )
                .thread_name(|i| format!("hastur-native-pixel-{i}"))
                .build()
                .expect("cannot start native pixel workers");
            while let Ok(job) = detail_rx.recv() {
                let message = match job {
                    Job::DetailRegion {
                        id,
                        revision,
                        image,
                        edit,
                        seg,
                        crop,
                        cancel,
                    } => {
                        match pool.install(|| {
                            renderer.render_viewport(
                                &image,
                                &edit,
                                seg.as_ref(),
                                crop,
                                Some(&cancel),
                            )
                        }) {
                            Ok(image) => Message::DetailRegionRendered {
                                id,
                                revision,
                                crop,
                                image,
                            },
                            Err(_) => Message::DetailRegionCancelled { id, revision, crop },
                        }
                    }
                    Job::PatchPreview {
                        id,
                        revision,
                        request,
                        image,
                        edit,
                        seg,
                        crop,
                        cancel,
                    } => {
                        let result = pool.install(|| match crop {
                            Some(crop) => renderer.render_viewport(
                                &image,
                                &edit,
                                seg.as_ref(),
                                crop,
                                Some(&cancel),
                            ),
                            None => renderer.render(&image, &edit, seg.as_ref(), Some(&cancel)),
                        });
                        Message::PatchPreview {
                            id,
                            revision,
                            request,
                            crop,
                            result,
                        }
                    }
                    Job::ReleaseMemory => {
                        renderer = engine::Renderer::default();
                        continue;
                    }
                    _ => continue,
                };
                let _ = detail_out.send(message);
                detail_ctx.request_repaint();
            }
        })
        .expect("cannot start native preview worker");
    let preview_out = out.clone();
    let preview_ctx = ctx.clone();
    std::thread::Builder::new()
        .name("hastur-live-preview".into())
        .spawn(move || {
            let mut renderer = engine::Renderer::default();
            let threads = std::thread::available_parallelism()
                .map_or(2, usize::from)
                .div_ceil(2)
                .clamp(1, 4);
            let pool = rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .thread_name(|i| format!("hastur-live-pixel-{i}"))
                .build()
                .expect("cannot start live pixel workers");
            while let Ok(job) = preview_rx.recv() {
                if matches!(job, Job::ReleaseMemory) {
                    renderer = engine::Renderer::default();
                    continue;
                }
                let Job::Render {
                    kind,
                    id,
                    revision,
                    image,
                    edit,
                    seg,
                    cancel,
                } = job
                else {
                    continue;
                };
                if !cancel.load(Ordering::Relaxed) {
                    let _ = preview_out.send(Message::RenderStarted { id, revision, kind });
                    preview_ctx.request_repaint();
                }
                let message = match pool
                    .install(|| renderer.render(&image, &edit, seg.as_ref(), Some(&cancel)))
                {
                    Ok(image) => Message::Rendered {
                        kind,
                        id,
                        revision,
                        image,
                    },
                    Err(_) => Message::RenderCancelled { id, revision, kind },
                };
                let _ = preview_out.send(message);
                preview_ctx.request_repaint();
            }
        })
        .expect("cannot start live preview worker");
    let (asset_tx, asset_rx) = crossbeam_channel::unbounded::<Job>();
    let asset_out = out.clone();
    let asset_ctx = ctx.clone();
    std::thread::Builder::new()
        .name("hastur-active-original".into())
        .spawn(move || {
            while let Ok(Job::LoadAsset {
                id,
                request,
                path,
                cancel,
            }) = asset_rx.recv()
            {
                if cancel.load(Ordering::Relaxed) {
                    continue;
                }
                let result = engine::load_photo(&path);
                if !cancel.load(Ordering::Relaxed) {
                    let _ = asset_out.send(Message::AssetLoaded {
                        id,
                        request,
                        result,
                    });
                    asset_ctx.request_repaint();
                }
            }
        })
        .expect("cannot start original loader");
    // Routing must stay responsive even while a native region, mask or thumbnail renders.
    std::thread::Builder::new()
        .name("hastur-image-dispatch".into())
        .spawn(move || {
            while let Ok(job) = rx.recv() {
                if matches!(job, Job::ReleaseMemory) {
                    let _ = preview_tx.send(Job::ReleaseMemory);
                    let _ = detail_tx.send(Job::ReleaseMemory);
                    let _ = background_tx.send(Job::ReleaseMemory);
                    continue;
                }
                let target = if matches!(job, Job::LoadAsset { .. }) {
                    &asset_tx
                } else if matches!(
                    job,
                    Job::Render {
                        kind: RenderKind::Preview,
                        ..
                    }
                ) {
                    &preview_tx
                } else if matches!(job, Job::DetailRegion { .. } | Job::PatchPreview { .. }) {
                    &detail_tx
                } else {
                    &background_tx
                };
                if target.send(job).is_err() {
                    break;
                }
            }
        })
        .expect("cannot start image dispatcher");
    let (portrait_tx, portrait_rx) = crossbeam_channel::unbounded::<PortraitRequest>();
    let portrait_out = out.clone();
    let portrait_ctx = ctx.clone();
    let (session_tx, session_rx) = crossbeam_channel::unbounded::<Job>();
    let session_out = out.clone();
    let session_ctx = ctx.clone();
    std::thread::Builder::new()
        .name("hastur-session-worker".into())
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
        .name("hastur-import-worker".into())
        .spawn(move || {
            while let Ok(photos) = import_rx.recv() {
                let send = |message| {
                    let _ = import_out.send(message);
                    import_ctx.request_repaint();
                };
                for path in photos {
                    if let Some(path) = path {
                        send(Message::Imported(
                            crate::assets::stage(&path)
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
        .name("hastur-portrait-ai".into())
        .spawn(move || {
            let mut queue = std::collections::VecDeque::<PortraitRequest>::new();
            loop {
                if queue.is_empty() {
                    match portrait_rx.recv_timeout(AI_IDLE_TIMEOUT) {
                        Ok(job) => queue.push_back(job),
                        Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                            model::release_idle_resources();
                            continue;
                        }
                        Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
                    }
                }
                queue.extend(portrait_rx.try_iter());
                let job = queue.pop_front().unwrap();
                if job.cancel.load(Ordering::Relaxed) {
                    continue;
                }
                if queue
                    .iter()
                    .any(|new| new.id == job.id && new.request > job.request)
                {
                    continue;
                }
                let result = model::analyze_shared_demand_cached_with_status(
                    &job.image,
                    job.provider,
                    job.force,
                    job.demand,
                    job.base.as_deref(),
                    |seg| {
                        if job.cancel.load(Ordering::Relaxed) {
                            return;
                        }
                        let _ = portrait_out.send(Message::PortraitReady {
                            id: job.id,
                            request: job.request,
                            complete: false,
                            result: Ok(seg),
                        });
                        portrait_ctx.request_repaint();
                    },
                    |status| {
                        if job.cancel.load(Ordering::Relaxed) {
                            return;
                        }
                        let _ = portrait_out.send(Message::AiStatus {
                            id: job.id,
                            request: job.request,
                            status,
                        });
                        portrait_ctx.request_repaint();
                    },
                );
                if job.cancel.load(Ordering::Relaxed) {
                    let paths = crate::ai_cache::release_source(&job.image);
                    let _ = portrait_out.send(Message::AssetCaches { id: job.id, paths });
                    portrait_ctx.request_repaint();
                    continue;
                }
                let _ = portrait_out.send(Message::PortraitReady {
                    id: job.id,
                    request: job.request,
                    complete: true,
                    result,
                });
                portrait_ctx.request_repaint();
            }
        })
        .expect("cannot start portrait AI worker");
    let (export_tx, export_rx) = crossbeam_channel::unbounded::<Job>();
    let export_slots = Arc::new((
        std::sync::Mutex::new((0usize, 1usize)),
        std::sync::Condvar::new(),
    ));
    for worker in 0..2 {
        let export_rx = export_rx.clone();
        let export_out = out.clone();
        let export_ctx = ctx.clone();
        let slots = export_slots.clone();
        std::thread::Builder::new()
            .name(format!("hastur-export-queue-{worker}"))
            .spawn(move || {
                let pool = rayon::ThreadPoolBuilder::new()
                    .num_threads(1)
                    .thread_name(move |i| format!("hastur-export-{worker}-pixel-{i}"))
                    .build()
                    .expect("export pixel pool");
                loop {
                    {
                        let mut usage = slots.0.lock().unwrap_or_else(|e| e.into_inner());
                        while usage.0 >= usage.1 {
                            usage = slots.1.wait(usage).unwrap_or_else(|e| e.into_inner());
                        }
                        usage.0 += 1;
                    }
                    let Ok(job) = export_rx.recv() else {
                        let mut usage = slots.0.lock().unwrap_or_else(|e| e.into_inner());
                        usage.0 -= 1;
                        slots.1.notify_all();
                        break;
                    };
                    pool.install(|| run_export(job, &export_out, &export_ctx));
                    // Export models belong to these threads; release at the end of
                    // each job instead of keeping two idle copies of AI weights.
                    model::release_idle_resources();
                    let mut usage = slots.0.lock().unwrap_or_else(|e| e.into_inner());
                    usage.0 -= 1;
                    slots.1.notify_all();
                }
            })
            .expect("cannot start export queue");
    }
    std::thread::Builder::new()
        .name("hastur-image-worker".into())
        .spawn(move || {
            let send = |message| {
                let _ = out.send(message);
                ctx.request_repaint();
            };
            let mut renderer = engine::Renderer::default();
            let mut queue = std::collections::VecDeque::new();
            loop {
                if queue.is_empty() {
                    match background_rx.recv() {
                        Ok(job) => queue.push_back(job),
                        Err(_) => break,
                    };
                }
                queue.extend(background_rx.try_iter());
                let index = queue
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, job)| priority(job))
                    .map(|(i, _)| i)
                    .unwrap();
                let job = queue.remove(index).unwrap();
                match job {
                    Job::ReleaseMemory => {
                        renderer = engine::Renderer::default();
                    }
                    Job::LoadAsset { .. } => continue,
                    Job::ImportFolder(directory) => match crate::assets::folder_files(&directory) {
                        Ok(paths) => {
                            let _ = import_tx.send(paths.into_iter().take(64).map(Some).collect());
                        }
                        Err(error) => {
                            send(Message::Imported(Err(error)));
                            send(Message::ImportFinished);
                        }
                    },
                    // Patch previews are dispatched to the independent native worker.
                    Job::PatchPreview { .. } => continue,
                    Job::LayerThumbnails {
                        id,
                        revision,
                        image,
                        layers,
                        seg,
                    } => {
                        let small = image::imageops::thumbnail(&*image, 80, 80);
                        let images = layers
                            .into_iter()
                            .map(|layer| {
                                let mut rendered =
                                    engine::render(&small, &layer.edit, seg.as_deref());
                                if layer.kind == crate::layer_stack::LayerType::Retouch {
                                    for (p, original) in rendered.pixels_mut().zip(small.pixels()) {
                                        if p == original {
                                            p[3] = 0;
                                        }
                                    }
                                }
                                (layer.id, rendered)
                            })
                            .collect();
                        send(Message::LayerThumbnails {
                            id,
                            revision,
                            images,
                        });
                    }
                    #[cfg(test)]
                    Job::BackgroundBarrier { entered, release } => {
                        let _ = entered.send(());
                        let _ = release.recv();
                    }
                    job @ (Job::SaveSession { .. }
                    | Job::ReadSession(_)
                    | Job::SaveRecovery { .. }
                    | Job::ReadRecovery(_)) => {
                        let _ = session_tx.send(job);
                    }
                    Job::Portrait {
                        cancel,
                        id,
                        request,
                        force,
                        provider,
                        demand,
                        base,
                        image,
                    } => {
                        let _ = portrait_tx.send(PortraitRequest {
                            cancel,
                            id,
                            request,
                            force,
                            provider,
                            demand,
                            base,
                            image,
                        });
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
                        match renderer.render_viewport(
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
                    job @ Job::Export { concurrency, .. } => {
                        export_slots.0.lock().unwrap_or_else(|e| e.into_inner()).1 =
                            usize::from(concurrency.clamp(1, 2));
                        export_slots.1.notify_all();
                        let _ = export_tx.send(job);
                    }
                }
            }
        })
        .expect("cannot start image worker");
    (tx, results)
}

fn priority(job: &Job) -> u8 {
    match job {
        Job::ReleaseMemory | Job::LoadAsset { .. } | Job::ImportFolder(_) => 0,
        #[cfg(test)]
        Job::BackgroundBarrier { .. } => 0,
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
        Job::DetailRegion { .. } | Job::PatchPreview { .. } | Job::Infer { .. } => 2,
        Job::Render {
            kind: RenderKind::Hover(_),
            ..
        }
        | Job::HoverRegion { .. } => 3,
        Job::Presets { .. } | Job::LayerThumbnails { .. } => 4,
        Job::Import(_) | Job::OpenSession { .. } | Job::Demo => 0,
    }
}

fn run_export(job: Job, out: &Sender<Message>, ctx: &eframe::egui::Context) {
    let Job::Export {
        concurrency: _,
        queue,
        provider,
        photos,
        directory,
        size,
        png,
        quality,
        cancel,
    } = job
    else {
        return;
    };
    let send = |message| {
        let _ = out.send(message);
        ctx.request_repaint();
    };
    let total = photos.len();
    let mut finished = 0;
    let mut report = Vec::new();
    let reference_mean = photos.first().map_or(0.5, |(p, _, _, _)| p.analysis.mean);
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
        let result = export_prepared(
            &photo,
            &edit,
            seg.as_deref(),
            provider,
            &directory,
            &options,
        );
        if cancel.load(Ordering::Relaxed) && result.is_err() {
            break;
        }
        let note = if (photo.analysis.mean - reference_mean).abs() > 0.15 {
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
            queue,
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
        queue,
        finished,
        total,
        cancelled: cancel.load(Ordering::Relaxed),
    });
}

fn export_prepared(
    photo: &Photo,
    edit: &Edit,
    base: Option<&Segmentation>,
    provider: Provider,
    directory: &std::path::Path,
    options: &ExportOptions,
) -> anyhow::Result<PathBuf> {
    if options
        .cancel
        .as_ref()
        .is_some_and(|c| c.load(Ordering::Relaxed))
    {
        anyhow::bail!("Export cancelled");
    }
    let native = photo.open_native()?;
    let demand = edit.portrait_demand();
    let mut seg = if !demand.is_empty() && !base.is_some_and(|s| s.prepared.contains(demand)) {
        Some(model::analyze_shared_demand_cached_with_status(
            &native.preview,
            provider,
            false,
            demand,
            base,
            |_| {},
            |_| {},
        )?)
    } else {
        None
    };
    if let (Some(seg), Some(saved)) = (&mut seg, base) {
        let saved = saved.session_copy().resample(seg.width, seg.height);
        seg.background = saved.background;
        if saved.custom_skin {
            seg.skin = saved.skin;
            seg.teeth = saved.teeth;
            seg.eyes = saved.eyes;
            seg.custom_skin = true;
        }
    }
    engine::export_with_options(&native, edit, seg.as_ref().or(base), directory, options)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn queued_projects_keep_independent_ids_formats_and_continue_after_cancellation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("source.png");
        let source = RgbaImage::from_pixel(64, 80, image::Rgba([90, 120, 170, 255]));
        source.save(&path).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let photo = crate::assets::stage(&path).unwrap();
        let (tx, rx) = start(eframe::egui::Context::default());
        for id in 1..=3 {
            tx.send(Job::Export {
                queue: id,
                concurrency: 2,
                provider: Provider::Cpu,
                photos: vec![(photo.clone(), Edit::default(), None, [0.5; 2])],
                directory: dir.path().join(format!("project-{id}")),
                size: ExportSize::Original,
                png: id == 2,
                quality: 95,
                cancel: Arc::new(AtomicBool::new(id == 1)),
            })
            .unwrap();
        }
        let mut done = std::collections::HashSet::new();
        while done.len() < 3 {
            match rx.recv_timeout(Duration::from_secs(10)).unwrap() {
                Message::Exported { queue, result, .. } => {
                    let exported = result.unwrap();
                    assert_eq!(
                        exported.parent().unwrap(),
                        dir.path().join(format!("project-{queue}"))
                    );
                    assert_eq!(
                        image::open(&exported).unwrap().into_rgba8().dimensions(),
                        source.dimensions()
                    );
                    assert_eq!(
                        exported.extension().unwrap(),
                        if queue == 2 { "png" } else { "jpg" }
                    );
                }
                Message::ExportFinished {
                    queue,
                    cancelled,
                    finished,
                    ..
                } => {
                    assert_eq!(cancelled, queue == 1);
                    assert_eq!(finished, usize::from(queue != 1));
                    done.insert(queue);
                }
                Message::ExportReport(result) => {
                    result.unwrap();
                }
                _ => panic!("Unrelated queue result"),
            }
        }
        assert_eq!(std::fs::read(path).unwrap(), bytes);
        crate::assets::purge_thumbnail(photo.path.as_deref().unwrap());
    }

    #[test]
    fn live_preview_renders_and_acknowledges_cancellation_while_background_worker_is_blocked() {
        let (tx, rx) = start(eframe::egui::Context::default());
        let (entered, ready) = crossbeam_channel::bounded(1);
        let (release, resume) = crossbeam_channel::bounded(1);
        tx.send(Job::BackgroundBarrier {
            entered,
            release: resume,
        })
        .unwrap();
        ready.recv_timeout(Duration::from_secs(5)).unwrap();
        let image = Arc::new(RgbaImage::from_pixel(
            64,
            80,
            image::Rgba([110, 90, 80, 255]),
        ));
        let mut edit = Edit::default();
        edit.settings.exposure = 0.5;
        for revision in [1, 2] {
            tx.send(Job::Render {
                kind: RenderKind::Preview,
                id: 7,
                revision,
                image: image.clone(),
                edit: edit.clone(),
                seg: None,
                cancel: Arc::new(AtomicBool::new(revision == 1)),
            })
            .unwrap();
        }
        assert!(matches!(
            rx.recv_timeout(Duration::from_secs(5)).unwrap(),
            Message::RenderCancelled {
                id: 7,
                revision: 1,
                kind: RenderKind::Preview
            }
        ));
        assert!(matches!(
            rx.recv_timeout(Duration::from_secs(5)).unwrap(),
            Message::RenderStarted {
                id: 7,
                revision: 2,
                kind: RenderKind::Preview
            }
        ));
        match rx.recv_timeout(Duration::from_secs(5)).unwrap() {
            Message::Rendered {
                id: 7,
                revision: 2,
                kind: RenderKind::Preview,
                image: output,
            } => {
                assert_eq!(output, engine::render(&image, &edit, None));
            }
            _ => panic!("Live preview was blocked by background work"),
        }
        let crop = engine::Crop {
            x: 8,
            y: 9,
            width: 24,
            height: 28,
        };
        // Patch feedback must use the independent native worker, and must not
        // terminate its receive loop before later detail requests.
        for (request, patch_crop, cancelled) in [
            (1, None, false),
            (2, Some(crop), false),
            (3, Some(crop), true),
        ] {
            tx.send(Job::PatchPreview {
                id: 7,
                revision: 2,
                request,
                image: image.clone(),
                edit: edit.clone(),
                seg: None,
                crop: patch_crop,
                cancel: Arc::new(AtomicBool::new(cancelled)),
            })
            .unwrap();
            match rx.recv_timeout(Duration::from_secs(5)).unwrap() {
                Message::PatchPreview {
                    id: 7,
                    revision: 2,
                    request: done,
                    crop: done_crop,
                    result,
                } => {
                    assert_eq!(done, request);
                    assert_eq!(done_crop, patch_crop);
                    if cancelled {
                        assert!(result.is_err());
                    } else {
                        let expected = patch_crop.map_or_else(
                            || engine::render(&image, &edit, None),
                            |crop| engine::render_region(&image, &edit, None, crop, None).unwrap(),
                        );
                        assert_eq!(result.unwrap(), expected);
                    }
                }
                _ => panic!("Patch feedback was blocked by background work"),
            }
        }
        tx.send(Job::DetailRegion {
            id: 7,
            revision: 3,
            image: image.clone(),
            edit: edit.clone(),
            seg: None,
            crop,
            cancel: Arc::new(AtomicBool::new(false)),
        })
        .unwrap();
        match rx.recv_timeout(Duration::from_secs(5)).unwrap() {
            Message::DetailRegionRendered {
                id: 7,
                revision: 3,
                crop: done,
                image: output,
            } => {
                assert_eq!(done, crop);
                assert_eq!(
                    output,
                    engine::render_region(&image, &edit, None, crop, None).unwrap()
                );
            }
            _ => panic!("Native preview was blocked by background work"),
        }
        release.send(()).unwrap();
    }

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
            concurrency: 1,
            queue: 0,
            provider: Provider::Cpu,
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
                    queue: 0,
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
