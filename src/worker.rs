use crate::{
    engine::{self, Edit, ExportSize, Photo, Segmentation},
    model::{self, Kind, Provider},
};
use crossbeam_channel::{Receiver, Sender};
use image::RgbaImage;
use std::{path::PathBuf, sync::Arc};

pub enum Job {
    Import(Vec<PathBuf>),
    OpenSession {
        paths: Vec<PathBuf>,
        demo: bool,
    },
    Demo,
    Render {
        id: u64,
        revision: u64,
        image: Arc<RgbaImage>,
        edit: Edit,
        seg: Option<Arc<Segmentation>>,
    },
    Infer {
        id: u64,
        path: PathBuf,
        kind: Kind,
        provider: Provider,
        image: Arc<RgbaImage>,
    },
    Export {
        photos: Vec<(Photo, Edit, Option<Arc<Segmentation>>)>,
        directory: PathBuf,
        size: ExportSize,
        png: bool,
        quality: u8,
    },
}
pub enum Message {
    Imported(anyhow::Result<Photo>),
    Rendered {
        id: u64,
        revision: u64,
        image: RgbaImage,
    },
    Segmented {
        id: u64,
        kind: Kind,
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
    std::thread::Builder::new()
        .name("astra-image-worker".into())
        .spawn(move || {
            let send = |message| {
                let _ = out.send(message);
                ctx.request_repaint();
            };
            while let Ok(job) = rx.recv() {
                match job {
                    Job::OpenSession { paths, demo } => {
                        if demo {
                            send(Message::Imported(
                                image::load_from_memory(include_bytes!(
                                    "../assets/demo-portrait.png"
                                ))
                                .map(|image| {
                                    engine::photo_from_image(
                                        "Studio portrait.png".into(),
                                        None,
                                        image.into_rgba8(),
                                    )
                                })
                                .map_err(anyhow::Error::from),
                            ));
                        }
                        for path in paths {
                            send(Message::Imported(
                                engine::load_photo(&path)
                                    .map_err(|e| e.context(path.display().to_string())),
                            ));
                        }
                        send(Message::ImportFinished);
                    }
                    Job::Import(paths) => {
                        for path in paths {
                            send(Message::Imported(
                                engine::load_photo(&path)
                                    .map_err(|e| e.context(path.display().to_string())),
                            ));
                        }
                        send(Message::ImportFinished);
                    }
                    Job::Demo => {
                        let photo =
                            image::load_from_memory(include_bytes!("../assets/demo-portrait.png"))
                                .map(|image| {
                                    engine::photo_from_image(
                                        "Studio portrait.png".into(),
                                        None,
                                        image.into_rgba8(),
                                    )
                                })
                                .map_err(anyhow::Error::from);
                        send(Message::Imported(photo));
                        send(Message::ImportFinished);
                    }
                    Job::Render {
                        id,
                        revision,
                        image,
                        edit,
                        seg,
                    } => send(Message::Rendered {
                        id,
                        revision,
                        image: engine::render(&image, &edit, seg.as_deref()),
                    }),
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
                    } => {
                        let total = photos.len();
                        let reference_mean =
                            photos.first().map_or(0.5, |(p, _, _)| p.analysis.mean);
                        let mut report = Vec::new();
                        for (index, (photo, edit, seg)) in photos.into_iter().enumerate() {
                            let result = engine::export(
                                &photo,
                                &edit,
                                seg.as_deref(),
                                &directory,
                                size,
                                png,
                                quality,
                            );
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
                            send(Message::Exported {
                                finished: index + 1,
                                total,
                                result,
                            });
                        }
                        send(Message::ExportReport(engine::export_report(
                            &directory, &report,
                        )));
                    }
                }
            }
        })
        .expect("cannot start image worker");
    (tx, results)
}
