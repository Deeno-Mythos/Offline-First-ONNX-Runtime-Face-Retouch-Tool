use crate::{
    engine::{
        self, Background, Edit, ExportSize, History, Photo, Preset, Segmentation, Stroke, Target,
    },
    gpu,
    model::{self, Kind, Provider},
    worker::{self, Job, Message},
};
use crossbeam_channel::{Receiver, Sender};
use eframe::egui::{
    self, Align2, Color32, FontId, Pos2, Rect, RichText, Sense, Stroke as Line, Vec2, pos2, vec2,
};
use glam::Vec2 as Pan;
use image::RgbaImage;
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

const BG: Color32 = Color32::from_rgb(11, 11, 12);
const PANEL: Color32 = Color32::from_rgb(20, 20, 22);
const RAISED: Color32 = Color32::from_rgb(29, 29, 31);
const YELLOW: Color32 = Color32::from_rgb(255, 214, 10);
const TEXT: Color32 = Color32::from_rgb(239, 239, 241);
const MUTED: Color32 = Color32::from_rgb(146, 146, 154);
const BORDER: Color32 = Color32::from_rgb(39, 39, 43);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tool {
    Portrait,
    Skin,
    Color,
    Background,
    Presets,
    Batch,
    Models,
}
impl Tool {
    fn title(self) -> &'static str {
        match self {
            Self::Portrait => "Portrait",
            Self::Skin => "Skin tone",
            Self::Color => "Color & light",
            Self::Background => "Background",
            Self::Presets => "Presets",
            Self::Batch => "Batch studio",
            Self::Models => "AI models",
        }
    }
    fn description(self) -> &'static str {
        match self {
            Self::Portrait => "A little polish. Still entirely you.",
            Self::Skin => "Even tones. Natural texture.",
            Self::Color => "Make the light feel right.",
            Self::Background => "Give your subject room to shine.",
            Self::Presets => "A starting point for your signature.",
            Self::Batch => "One look, beautifully consistent.",
            Self::Models => "Your models. On your device.",
        }
    }
}

struct PhotoState {
    id: u64,
    photo: Photo,
    edit: Edit,
    history: History,
    original_texture: egui::TextureHandle,
    edited_texture: egui::TextureHandle,
    edited: Arc<RgbaImage>,
    revision: u64,
    rendered_revision: u64,
    selected: bool,
    rating: u8,
    seg: Option<Arc<Segmentation>>,
    note: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct Session {
    version: u32,
    photos: Vec<SavedPhoto>,
}
#[derive(Serialize, Deserialize)]
struct SavedPhoto {
    path: Option<PathBuf>,
    edit: Edit,
    rating: u8,
    #[serde(default)]
    segmentation: Option<Segmentation>,
}

pub struct AstraApp {
    photos: Vec<PhotoState>,
    current: usize,
    next_id: u64,
    tool: Tool,
    tx: Sender<Job>,
    rx: Receiver<Message>,
    importing: bool,
    render_busy: bool,
    last_edit: Instant,
    gesture: bool,
    compare: bool,
    split: f32,
    zoom: f32,
    pan: Pan,
    brush: Option<Target>,
    brush_radius: f32,
    erase: bool,
    last_stroke: Option<[f32; 2]>,
    export_open: bool,
    export_size: ExportSize,
    export_png: bool,
    quality: u8,
    export_all: bool,
    exporting: Option<(usize, usize)>,
    export_errors: usize,
    exported_paths: Vec<PathBuf>,
    toast: Option<(String, Instant)>,
    help: bool,
    presets: Vec<Preset>,
    model_kind: Kind,
    provider: Provider,
    model_busy: bool,
    pending_session: Option<Session>,
    gpu: bool,
    screenshot: Option<PathBuf>,
    screenshot_requested: bool,
    frames: u64,
    compact_open: bool,
    export_report: Option<PathBuf>,
}

fn texture(ctx: &egui::Context, name: &str, image: &RgbaImage) -> egui::TextureHandle {
    ctx.load_texture(
        name,
        egui::ColorImage::from_rgba_unmultiplied(
            [image.width() as usize, image.height() as usize],
            image.as_raw(),
        ),
        egui::TextureOptions::LINEAR,
    )
}

impl AstraApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme(&cc.egui_ctx);
        let gpu = gpu::install(cc);
        let (tx, rx) = worker::start(cc.egui_ctx.clone());
        let arguments: Vec<String> = std::env::args().collect();
        let screenshot = arguments
            .iter()
            .position(|a| a == "--screenshot")
            .and_then(|i| arguments.get(i + 1))
            .map(PathBuf::from);
        let paths: Vec<PathBuf> = arguments
            .iter()
            .skip(1)
            .filter(|a| !a.starts_with('-'))
            .map(PathBuf::from)
            .filter(|p| p.is_file() && Some(p) != screenshot.as_ref())
            .collect();
        let initial = if paths.is_empty() {
            Job::Demo
        } else {
            Job::Import(paths)
        };
        let _ = tx.send(initial);
        Self {
            photos: vec![],
            current: 0,
            next_id: 1,
            tool: arguments
                .iter()
                .position(|a| a == "--tool")
                .and_then(|i| arguments.get(i + 1))
                .map(|name| match name.as_str() {
                    "presets" => Tool::Presets,
                    "color" => Tool::Color,
                    "background" => Tool::Background,
                    "batch" => Tool::Batch,
                    "models" => Tool::Models,
                    _ => Tool::Portrait,
                })
                .unwrap_or(Tool::Portrait),
            tx,
            rx,
            importing: true,
            render_busy: false,
            last_edit: Instant::now(),
            gesture: false,
            compare: arguments.iter().any(|a| a == "--compare"),
            split: 0.5,
            zoom: 1.0,
            pan: Pan::ZERO,
            brush: None,
            brush_radius: 0.018,
            erase: false,
            last_stroke: None,
            export_open: false,
            export_size: ExportSize::Original,
            export_png: false,
            quality: 95,
            export_all: false,
            exporting: None,
            export_errors: 0,
            exported_paths: vec![],
            toast: None,
            help: false,
            presets: engine::presets(),
            model_kind: Kind::FaceParsing,
            provider: Provider::Cpu,
            model_busy: false,
            pending_session: None,
            gpu,
            screenshot,
            screenshot_requested: false,
            frames: 0,
            compact_open: false,
            export_report: None,
        }
    }

    fn notify(&mut self, text: impl Into<String>) {
        self.toast = Some((text.into(), Instant::now()));
    }
    fn edited(&mut self) {
        if let Some(photo) = self.photos.get_mut(self.current) {
            photo.revision += 1;
        }
        self.last_edit = Instant::now();
    }

    fn poll(&mut self, ctx: &egui::Context) {
        while let Ok(message) = self.rx.try_recv() {
            match message {
                Message::ExportReport(result) => match result {
                    Ok(path) => self.export_report = Some(path),
                    Err(e) => self.notify(format!(
                        "Images exported, but report could not be saved: {e}"
                    )),
                },
                Message::Imported(result) => match result {
                    Ok(photo) => {
                        let id = self.next_id;
                        self.next_id += 1;
                        let tex = texture(ctx, &format!("original-{id}"), &photo.preview);
                        let mut edit = Edit::default();
                        let mut rating = 0;
                        let mut segmentation = None;
                        if photo.path.is_none() {
                            edit.settings = self.presets[0].settings.clone();
                            edit.preset = Some("Natural Headshot".into());
                        }
                        if let Some(session) = &self.pending_session
                            && let Some(saved) =
                                session.photos.iter().find(|p| p.path == photo.path)
                        {
                            edit = saved.edit.clone();
                            rating = saved.rating;
                            segmentation = saved.segmentation.clone().map(Arc::new);
                        }
                        self.photos.push(PhotoState {
                            id,
                            edited: photo.preview.clone(),
                            photo,
                            edit,
                            history: History::default(),
                            original_texture: tex.clone(),
                            edited_texture: tex,
                            revision: 1,
                            rendered_revision: 0,
                            selected: true,
                            rating,
                            seg: segmentation,
                            note: None,
                        });
                        self.current = self.photos.len() - 1;
                    }
                    Err(e) => self.notify(format!("Import failed: {e:#}")),
                },
                Message::ImportFinished => {
                    self.importing = false;
                    self.pending_session = None;
                    self.zoom = 1.0;
                    self.pan = Pan::ZERO;
                }
                Message::Rendered {
                    id,
                    revision,
                    image,
                } => {
                    self.render_busy = false;
                    if let Some(photo) = self.photos.iter_mut().find(|p| p.id == id)
                        && revision == photo.revision
                    {
                        photo.edited_texture = texture(ctx, &format!("edited-{id}"), &image);
                        photo.edited = Arc::new(image);
                        photo.rendered_revision = revision;
                    }
                }
                Message::Segmented { id, kind, result } => {
                    self.model_busy = false;
                    match result {
                        Ok(new) => {
                            if let Some(photo) = self.photos.iter_mut().find(|p| p.id == id) {
                                let mut seg = photo.seg.as_deref().cloned().unwrap_or_default();
                                if seg.width != new.width || seg.height != new.height {
                                    seg = seg.resample(new.width, new.height);
                                }
                                if kind == Kind::FaceParsing {
                                    seg.skin = new.skin;
                                    seg.eyes = new.eyes;
                                    seg.teeth = new.teeth;
                                } else {
                                    seg.background = new.background;
                                }
                                photo.seg = Some(Arc::new(seg));
                                photo.revision += 1;
                            }
                            self.notify("Mask ready. Inspect edges and refine with the brush.");
                        }
                        Err(e) => self.notify(format!("Model could not run: {e:#}")),
                    }
                }
                Message::Exported {
                    finished,
                    total,
                    result,
                } => {
                    if let Ok(path) = result {
                        self.exported_paths.push(path);
                    } else if let Err(e) = result {
                        self.export_errors += 1;
                        self.notify(format!("Export failed: {e:#}"));
                    }
                    self.exporting = if finished == total {
                        None
                    } else {
                        Some((finished, total))
                    };
                    if finished == total {
                        self.notify(format!(
                            "Exported {} photo{} · {} failure{}",
                            self.exported_paths.len(),
                            if self.exported_paths.len() == 1 {
                                ""
                            } else {
                                "s"
                            },
                            self.export_errors,
                            if self.export_errors == 1 { "" } else { "s" }
                        ));
                    }
                }
            }
        }
        if !self.render_busy
            && self.last_edit.elapsed() > Duration::from_millis(65)
            && let Some(photo) = self.photos.get(self.current)
            && photo.rendered_revision != photo.revision
        {
            self.render_busy = self
                .tx
                .send(Job::Render {
                    id: photo.id,
                    revision: photo.revision,
                    image: photo.photo.preview.clone(),
                    edit: photo.edit.clone(),
                    seg: photo.seg.clone(),
                })
                .is_ok();
        }
        if self.render_busy || self.importing || self.model_busy || self.exporting.is_some() {
            ctx.request_repaint_after(Duration::from_millis(33));
        } else if self
            .photos
            .get(self.current)
            .is_some_and(|p| p.revision != p.rendered_revision)
        {
            ctx.request_repaint_after(Duration::from_millis(70));
        }
    }

    fn import(&mut self) {
        if let Some(paths) = rfd::FileDialog::new()
            .set_title("Import portraits you own or have permission to edit")
            .add_filter(
                "Photos",
                &["jpg", "jpeg", "png", "webp", "tiff", "tif", "bmp"],
            )
            .pick_files()
        {
            if self.photos.len() + paths.len() > 64 {
                self.notify("A session supports up to 64 photos. Import a smaller batch.");
                return;
            }
            self.importing = true;
            let _ = self.tx.send(Job::Import(paths));
        }
    }

    fn undo(&mut self) {
        if let Some(p) = self.photos.get_mut(self.current)
            && p.history.undo(&mut p.edit)
        {
            self.edited();
        }
    }
    fn redo(&mut self) {
        if let Some(p) = self.photos.get_mut(self.current)
            && p.history.redo(&mut p.edit)
        {
            self.edited();
        }
    }

    fn apply_preset(&mut self, index: usize) {
        if let Some(photo) = self.photos.get_mut(self.current) {
            photo.history.record(photo.edit.clone());
            photo.edit.settings = self.presets[index].settings.clone();
            photo.edit.preset = Some(self.presets[index].name.clone());
            self.edited();
        }
    }

    fn sync(&mut self) {
        let Some(source) = self.photos.get(self.current) else {
            return;
        };
        let settings = source.edit.settings.clone();
        let preset = source.edit.preset.clone();
        let mean = source.photo.analysis.mean;
        let mut count = 0;
        for (index, photo) in self.photos.iter_mut().enumerate() {
            if index != self.current && photo.selected {
                photo.history.record(photo.edit.clone());
                photo.edit.settings = settings.clone();
                photo.edit.preset = preset.clone();
                photo.revision += 1;
                count += 1;
                photo.note = if (photo.photo.analysis.mean - mean).abs() > 0.15 {
                    Some("Different exposure · review shadows and skin before export".into())
                } else {
                    None
                };
            }
        }
        self.notify(format!(
            "Synced settings to {count} photos. Each photo keeps its own local masks."
        ));
    }

    fn save_session(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Astra session", &["ron"])
            .set_file_name("astra-session.ron")
            .save_file()
        {
            let session = Session {
                version: 1,
                photos: self
                    .photos
                    .iter()
                    .map(|p| SavedPhoto {
                        path: p.photo.path.clone(),
                        edit: p.edit.clone(),
                        rating: p.rating,
                        segmentation: p.seg.as_deref().cloned(),
                    })
                    .collect(),
            };
            match ron::ser::to_string_pretty(&session, Default::default())
                .map_err(anyhow::Error::from)
                .and_then(|text| std::fs::write(path, text).map_err(anyhow::Error::from))
            {
                Ok(()) => self.notify("Session saved · original files are untouched"),
                Err(e) => self.notify(format!("Could not save session: {e}")),
            }
        }
    }

    fn open_session(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Astra session", &["ron"])
            .pick_file()
        {
            let result = std::fs::read_to_string(&path)
                .map_err(anyhow::Error::from)
                .and_then(|text| ron::from_str::<Session>(&text).map_err(anyhow::Error::from));
            match result {
                Ok(session) if session.version == 1 && session.photos.len() <= 64 => {
                    self.photos.clear();
                    self.current = 0;
                    self.importing = true;
                    let paths = session
                        .photos
                        .iter()
                        .filter_map(|p| p.path.clone())
                        .collect();
                    let demo = session.photos.iter().any(|p| p.path.is_none());
                    self.pending_session = Some(session);
                    let _ = self.tx.send(Job::OpenSession { paths, demo });
                }
                Ok(_) => self.notify("Unsupported session version or too many photos"),
                Err(e) => self.notify(format!("Could not open session: {e}")),
            }
        }
    }

    fn header(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("topbar")
            .exact_height(62.0)
            .frame(
                egui::Frame::new()
                    .fill(PANEL)
                    .inner_margin(egui::Margin::symmetric(22, 12))
                    .stroke(Line::new(1.0_f32, BORDER)),
            )
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(26.0, 28.0), Sense::hover());
                    star(ui.painter(), r.center(), 12.0, YELLOW);
                    ui.add_space(6.0);
                    ui.label(RichText::new("astra").size(23.0).strong().color(TEXT));
                    ui.label(RichText::new("RETOUCH").size(10.0).color(MUTED));
                    ui.add_space(20.0);
                    ui.menu_button(RichText::new("File").color(MUTED), |ui| {
                        if ui.button("Import photos…    Ctrl+O").clicked() {
                            self.import();
                            ui.close();
                        }
                        if ui.button("Open session…").clicked() {
                            self.open_session();
                            ui.close();
                        }
                        if ui.button("Save session…    Ctrl+S").clicked() {
                            self.save_session();
                            ui.close();
                        }
                        ui.separator();
                        if ui.button("Keyboard shortcuts").clicked() {
                            self.help = true;
                            ui.close();
                        }
                    });
                    ui.add_space(12.0);
                    let undo = self
                        .photos
                        .get(self.current)
                        .is_some_and(|p| p.history.can_undo());
                    let redo = self
                        .photos
                        .get(self.current)
                        .is_some_and(|p| p.history.can_redo());
                    if ui
                        .add_enabled(undo, egui::Button::new("↶").frame(false))
                        .on_hover_text("Undo · Ctrl+Z")
                        .clicked()
                    {
                        self.undo();
                    }
                    if ui
                        .add_enabled(redo, egui::Button::new("↷").frame(false))
                        .on_hover_text("Redo · Ctrl+Shift+Z")
                        .clicked()
                    {
                        self.redo();
                    }
                    let name = self
                        .photos
                        .get(self.current)
                        .map(|p| p.photo.name.as_str())
                        .unwrap_or("Your next great portrait");
                    if ctx.content_rect().width() >= 1050.0 {
                        ui.add_space(12.0);
                        ui.label(RichText::new(name).size(12.0).color(MUTED));
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add_enabled(
                                !self.photos.is_empty() && self.exporting.is_none(),
                                primary("Export  ↗").min_size(vec2(104.0, 35.0)),
                            )
                            .clicked()
                        {
                            self.export_open = true;
                        }
                        if ctx.content_rect().width() >= 1050.0 {
                            ui.add_space(16.0);
                            ui.label(RichText::new("ON DEVICE").size(10.0).color(MUTED));
                        }
                    });
                });
            });
    }

    fn rail(&mut self, ctx: &egui::Context) {
        if ctx.content_rect().width() < 1050.0 {
            egui::TopBottomPanel::bottom("compact-tabs")
                .exact_height(58.0)
                .frame(
                    egui::Frame::new()
                        .fill(PANEL)
                        .inner_margin(egui::Margin::symmetric(12, 6)),
                )
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        if rail_button(ui, None, false, "Import").clicked() {
                            self.import();
                        }
                        for tool in [
                            Tool::Portrait,
                            Tool::Skin,
                            Tool::Color,
                            Tool::Background,
                            Tool::Presets,
                            Tool::Batch,
                            Tool::Models,
                        ] {
                            if rail_button(
                                ui,
                                Some(tool),
                                self.tool == tool && self.compact_open,
                                tool.title(),
                            )
                            .clicked()
                            {
                                self.tool = tool;
                                self.compact_open = true;
                                self.brush = None;
                            }
                        }
                    });
                });
            return;
        }
        egui::SidePanel::left("rail")
            .exact_width(76.0)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(PANEL)
                    .inner_margin(egui::Margin::symmetric(12, 20))
                    .stroke(Line::new(1.0_f32, BORDER)),
            )
            .show(ctx, |ui| {
                if rail_button(ui, None, false, "Import photos · Ctrl+O").clicked() {
                    self.import();
                }
                ui.add_space(22.0);
                for tool in [
                    Tool::Portrait,
                    Tool::Skin,
                    Tool::Color,
                    Tool::Background,
                    Tool::Presets,
                    Tool::Batch,
                ] {
                    if rail_button(ui, Some(tool), self.tool == tool, tool.title()).clicked() {
                        self.tool = tool;
                        self.brush = None;
                    }
                    ui.add_space(10.0);
                }
                ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| {
                    if rail_button(
                        ui,
                        Some(Tool::Models),
                        self.tool == Tool::Models,
                        "AI models",
                    )
                    .clicked()
                    {
                        self.tool = Tool::Models;
                    }
                    if ui
                        .add(egui::Button::new("?").frame(false))
                        .on_hover_text("Shortcuts & help")
                        .clicked()
                    {
                        self.help = true;
                    }
                });
            });
    }

    fn tools(&mut self, ctx: &egui::Context) {
        if ctx.content_rect().width() < 1050.0 {
            if self.compact_open {
                egui::Window::new("Tool sheet")
                    .title_bar(false)
                    .resizable(false)
                    .movable(false)
                    .fixed_pos(pos2(12.0, ctx.content_rect().height() * 0.38))
                    .fixed_size(vec2(
                        ctx.content_rect().width() - 24.0,
                        ctx.content_rect().height() * 0.53,
                    ))
                    .frame(
                        egui::Frame::window(&ctx.style())
                            .fill(PANEL)
                            .corner_radius(16)
                            .inner_margin(20),
                    )
                    .show(ctx, |ui| {
                        if ui.button("Done").clicked() {
                            self.compact_open = false;
                        }
                        self.controls_content(ui);
                    });
            }
            return;
        }
        egui::SidePanel::right("controls")
            .exact_width(328.0)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(PANEL)
                    .inner_margin(egui::Margin::symmetric(22, 24))
                    .stroke(Line::new(1.0_f32, BORDER)),
            )
            .show(ctx, |ui| self.controls_content(ui));
    }

    fn controls_content(&mut self, ui: &mut egui::Ui) {
        let tool = self.tool;
        let ctx = ui.ctx().clone();
        ui.label(RichText::new(tool.title()).size(20.0).strong());
        ui.add_space(5.0);
        ui.label(RichText::new(tool.description()).size(11.0).color(MUTED));
        ui.add_space(22.0);
        egui::ScrollArea::vertical()
            .id_salt("tool-scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if self.photos.is_empty() {
                    ui.label(RichText::new("Import a photo to start editing.").color(MUTED));
                    return;
                }
                let before = self.photos[self.current].edit.clone();
                match tool {
                    Tool::Portrait => self.portrait_panel(ui),
                    Tool::Skin => self.skin_panel(ui),
                    Tool::Color => self.color_panel(ui),
                    Tool::Background => self.background_panel(ui),
                    Tool::Presets => self.preset_panel(ui),
                    Tool::Batch => self.batch_panel(ui),
                    Tool::Models => self.model_panel(ui),
                }
                if before != self.photos[self.current].edit {
                    if !self.gesture {
                        self.photos[self.current].history.record(before);
                    }
                    self.gesture = ctx.input(|i| i.pointer.any_down());
                    self.edited();
                }
                ui.add_space(24.0);
                ui.separator();
                ui.add_space(14.0);
                ui.label(RichText::new("NATURAL BY DESIGN").size(9.0).color(MUTED));
                ui.label(
                    RichText::new("Your features. Your texture. Your story.")
                        .size(11.0)
                        .color(MUTED),
                );
            });
    }

    fn portrait_panel(&mut self, ui: &mut egui::Ui) {
        if ui
            .add_sized([ui.available_width(), 42.0], primary("✧  Auto Retouch"))
            .on_hover_text("Apply conservative local adjustments. No face or body reshaping.")
            .clicked()
        {
            self.photos[self.current].edit.settings = self.presets[0].settings.clone();
            self.photos[self.current].edit.preset = Some("Natural Headshot".into());
        }
        ui.add_space(10.0);
        ui.label(
            RichText::new("Local processing · no photo uploads")
                .size(10.0)
                .color(MUTED),
        );
        ui.add_space(24.0);
        section(ui, "SKIN", |ui| {
            let settings = &mut self.photos[self.current].edit.settings;
            slider(
                ui,
                "Skin smoothing",
                &mut settings.smoothing,
                0.0..=100.0,
                "Soften medium-scale texture while retaining fine pores. Color mask can be refined.",
            );
            slider(
                ui,
                "Blemish removal",
                &mut settings.blemishes,
                0.0..=100.0,
                "Strength of spots you mark with the spot-heal brush. Permanent features are preserved until you mark them.",
            );
            slider(
                ui,
                "Redness reduction",
                &mut settings.redness,
                0.0..=100.0,
                "Gently reduce excess red in the skin mask.",
            );
            ui.add_space(5.0);
            ui.horizontal(|ui| {
                if small_chip(ui, "Spot heal", self.brush == Some(Target::Heal)).clicked() {
                    self.brush = if self.brush == Some(Target::Heal) {
                        None
                    } else {
                        Some(Target::Heal)
                    };
                }
                if small_chip(ui, "Refine mask", self.brush == Some(Target::Skin)).clicked() {
                    self.brush = if self.brush == Some(Target::Skin) {
                        None
                    } else {
                        Some(Target::Skin)
                    };
                }
            });
        });
        ui.add_space(20.0);
        section(ui, "FACE DETAILS", |ui| {
            let settings = &mut self.photos[self.current].edit.settings;
            slider(
                ui,
                "Under-eye brightening",
                &mut settings.under_eyes,
                0.0..=100.0,
                "Only affects a painted under-eye mask.",
            );
            slider(
                ui,
                "Teeth whitening",
                &mut settings.teeth,
                0.0..=100.0,
                "Only affects a painted teeth mask.",
            );
            slider(
                ui,
                "Eye clarity",
                &mut settings.eyes,
                0.0..=100.0,
                "Only affects a painted or model-generated eye mask.",
            );
            ui.horizontal_wrapped(|ui| {
                for target in [Target::UnderEyes, Target::Teeth, Target::Eyes] {
                    if small_chip(ui, target.label(), self.brush == Some(target)).clicked() {
                        self.brush = if self.brush == Some(target) {
                            None
                        } else {
                            Some(target)
                        };
                    }
                }
            });
            ui.add_space(7.0);
            ui.label(
                RichText::new("Paint a target first; only that area changes.")
                    .size(10.0)
                    .color(MUTED),
            );
        });
        ui.add_space(20.0);
        section(ui, "FINISH", |ui| {
            slider(
                ui,
                "Detail sharpening",
                &mut self.photos[self.current].edit.settings.sharpening,
                0.0..=100.0,
                "Subtle high-pass sharpening in linear light.",
            );
        });
        self.brush_controls(ui);
        ui.add_space(18.0);
        if ui
            .add_sized([ui.available_width(), 32.0], secondary("Reset adjustments"))
            .clicked()
        {
            self.photos[self.current].edit = Edit::default();
            self.brush = None;
        }
    }

    fn skin_panel(&mut self, ui: &mut egui::Ui) {
        section(ui, "TONE & TEXTURE", |ui| {
            let settings = &mut self.photos[self.current].edit.settings;
            slider(
                ui,
                "Tone evenness",
                &mut settings.tone_evenness,
                0.0..=100.0,
                "Balance patchy skin tones without flattening fine texture.",
            );
            slider(
                ui,
                "Redness reduction",
                &mut settings.redness,
                0.0..=100.0,
                "Correct only the skin mask.",
            );
            slider(
                ui,
                "Skin smoothing",
                &mut settings.smoothing,
                0.0..=100.0,
                "Preserve natural pores.",
            );
        });
        ui.add_space(20.0);
        if ui
            .add_sized([ui.available_width(), 36.0], secondary("Refine skin mask"))
            .clicked()
        {
            self.brush = Some(Target::Skin);
        }
        self.brush_controls(ui);
        ui.add_space(20.0);
        section(ui, "PHOTO ANALYSIS", |ui| {
            for note in &self.photos[self.current].photo.analysis.notes {
                ui.label(RichText::new(note).size(11.0).color(MUTED));
                ui.add_space(7.0);
            }
        });
    }

    fn color_panel(&mut self, ui: &mut egui::Ui) {
        let s = &mut self.photos[self.current].edit.settings;
        section(ui, "LIGHT", |ui| {
            slider(
                ui,
                "Exposure · EV",
                &mut s.exposure,
                -2.0..=2.0,
                "Linear-light exposure adjustment in stops.",
            );
            slider(
                ui,
                "Contrast",
                &mut s.contrast,
                -100.0..=100.0,
                "Contrast around middle gray.",
            );
            slider(
                ui,
                "Highlights",
                &mut s.highlights,
                -100.0..=100.0,
                "Recover or lift bright tones.",
            );
            slider(
                ui,
                "Shadows",
                &mut s.shadows,
                -100.0..=100.0,
                "Lift or deepen dark tones.",
            );
        });
        ui.add_space(20.0);
        section(ui, "WHITE BALANCE", |ui| {
            slider(
                ui,
                "Temperature",
                &mut s.warmth,
                -100.0..=100.0,
                "Cooler to warmer; relative correction, not Kelvin.",
            );
            slider(
                ui,
                "Tint",
                &mut s.tint,
                -100.0..=100.0,
                "Green to magenta balance.",
            );
        });
        ui.add_space(20.0);
        section(ui, "COLOR & FINISH", |ui| {
            slider(
                ui,
                "Saturation",
                &mut s.saturation,
                -100.0..=100.0,
                "Linear luminance-preserving saturation.",
            );
            slider(
                ui,
                "Vignette",
                &mut s.vignette,
                0.0..=100.0,
                "A gentle edge falloff.",
            );
            slider(
                ui,
                "Sharpening",
                &mut s.sharpening,
                0.0..=100.0,
                "Keep the smallest details clear.",
            );
        });
    }

    fn background_panel(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Choose a treatment").size(12.0).color(MUTED));
        ui.add_space(12.0);
        for (mode, label, detail) in [
            (Background::Original, "Original", "Keep the scene as it is"),
            (
                Background::Blur,
                "Lens blur",
                "Soft separation, natural depth",
            ),
            (Background::Solid, "Studio backdrop", "A clean, solid color"),
            (
                Background::Transparent,
                "Remove background",
                "Export a transparent PNG",
            ),
        ] {
            let selected = self.photos[self.current].edit.settings.background == mode;
            let response = ui
                .add_sized(
                    [ui.available_width(), 40.0],
                    if selected {
                        secondary(label)
                            .fill(Color32::from_rgb(48, 43, 21))
                            .stroke(Line::new(1.0_f32, YELLOW))
                    } else {
                        secondary(label)
                    },
                )
                .on_hover_text(detail);
            if response.clicked() {
                self.photos[self.current].edit.settings.background = mode;
            }
            ui.add_space(7.0);
        }
        let s = &mut self.photos[self.current].edit.settings;
        if s.background == Background::Blur {
            ui.add_space(12.0);
            slider(
                ui,
                "Blur strength",
                &mut s.background_blur,
                0.0..=100.0,
                "Only affects the background mask.",
            );
        }
        if s.background == Background::Solid {
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                ui.label("Backdrop color");
                ui.color_edit_button_srgb(&mut s.background_color);
            });
        }
        ui.add_space(20.0);
        ui.label(RichText::new("A background mask is required.").size(12.0));
        ui.add_space(6.0);
        ui.label(RichText::new("Paint the background, or load a compatible segmentation model in AI models. Inspect hair and edges at 100%.").size(11.0).color(MUTED));
        ui.add_space(14.0);
        if ui
            .add_sized(
                [ui.available_width(), 36.0],
                secondary("Paint background mask"),
            )
            .clicked()
        {
            self.brush = Some(Target::Background);
        }
        self.brush_controls(ui);
    }

    fn preset_panel(&mut self, ui: &mut egui::Ui) {
        let mut choose = None;
        for (index, preset) in self.presets.iter().enumerate() {
            let selected =
                self.photos[self.current].edit.preset.as_deref() == Some(preset.name.as_str());
            let (rect, response) =
                ui.allocate_exact_size(vec2(ui.available_width(), 76.0), Sense::click());
            ui.painter().rect_filled(
                rect,
                12.0,
                if selected {
                    Color32::from_rgb(40, 37, 23)
                } else {
                    RAISED
                },
            );
            ui.painter().rect_stroke(
                rect,
                12.0,
                Line::new(1.0_f32, if selected { YELLOW } else { BORDER }),
                egui::StrokeKind::Inside,
            );
            let thumb = Rect::from_min_size(rect.min + vec2(8.0, 8.0), vec2(48.0, 60.0));
            ui.painter().image(
                self.photos[self.current].original_texture.id(),
                thumb,
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                Color32::WHITE,
            );
            ui.painter().text(
                rect.min + vec2(70.0, 22.0),
                Align2::LEFT_CENTER,
                &preset.name,
                FontId::proportional(12.0),
                TEXT,
            );
            ui.painter().text(
                rect.min + vec2(70.0, 45.0),
                Align2::LEFT_CENTER,
                &preset.description,
                FontId::proportional(10.0),
                MUTED,
            );
            if selected {
                ui.painter()
                    .circle_filled(rect.right_top() + vec2(-12.0, 12.0), 3.0, YELLOW);
            }
            if response.clicked() {
                choose = Some(index);
            }
            ui.add_space(10.0);
        }
        if let Some(index) = choose {
            self.photos[self.current].edit.settings = self.presets[index].settings.clone();
            self.photos[self.current].edit.preset = Some(self.presets[index].name.clone());
        }
        ui.add_space(10.0);
        if ui
            .add_sized(
                [ui.available_width(), 34.0],
                secondary("Save current as preset…"),
            )
            .clicked()
            && let Some(path) = rfd::FileDialog::new()
                .add_filter("Astra preset", &["ron"])
                .set_file_name("My look.ron")
                .save_file()
        {
            let name = path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            let preset = Preset {
                name,
                description: "Your signature look.".into(),
                settings: self.photos[self.current].edit.settings.clone(),
            };
            match ron::ser::to_string_pretty(&preset, Default::default())
                .map_err(anyhow::Error::from)
                .and_then(|text| std::fs::write(&path, text).map_err(anyhow::Error::from))
            {
                Ok(()) => {
                    self.presets.push(preset);
                    self.notify("Preset saved");
                }
                Err(e) => self.notify(format!("Could not save preset: {e}")),
            }
        }
        ui.add_space(8.0);
        if ui
            .add_sized([ui.available_width(), 34.0], secondary("Load preset…"))
            .clicked()
            && let Some(path) = rfd::FileDialog::new()
                .add_filter("Astra preset", &["ron"])
                .pick_file()
        {
            match std::fs::read_to_string(path)
                .map_err(anyhow::Error::from)
                .and_then(|text| ron::from_str::<Preset>(&text).map_err(anyhow::Error::from))
            {
                Ok(preset) => self.presets.push(preset),
                Err(e) => self.notify(format!("Could not load preset: {e}")),
            }
        }
        ui.add_space(14.0);
        ui.label(RichText::new("Presets preserve the original background. Choose a backdrop explicitly in Background.").size(10.0).color(MUTED));
    }

    fn batch_panel(&mut self, ui: &mut egui::Ui) {
        if let Some(path) = &self.export_report {
            ui.label(RichText::new("EXPORT REPORT").size(10.0).color(MUTED));
            ui.label(
                RichText::new(path.file_name().unwrap_or_default().to_string_lossy()).size(11.0),
            );
            if ui.button("Copy report path").clicked() {
                ui.ctx().copy_text(path.display().to_string());
            }
            ui.add_space(18.0);
        }
        let selected = self.photos.iter().filter(|p| p.selected).count();
        ui.label(RichText::new(format!("{selected} photos selected")).size(15.0));
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if ui.button("Select all").clicked() {
                for p in &mut self.photos {
                    p.selected = true;
                }
            }
            if ui.button("Select none").clicked() {
                for p in &mut self.photos {
                    p.selected = false;
                }
            }
        });
        ui.add_space(18.0);
        if ui
            .add_enabled(
                selected > 1,
                primary("Sync settings to selected").min_size(vec2(ui.available_width(), 40.0)),
            )
            .clicked()
        {
            self.sync();
        }
        ui.add_space(8.0);
        ui.label(
            RichText::new(
                "Copies this photo’s adjustments. Local masks stay with their own photos.",
            )
            .size(11.0)
            .color(MUTED),
        );
        ui.add_space(24.0);
        if ui
            .add_sized(
                [ui.available_width(), 34.0],
                secondary("Find sharpest picks"),
            )
            .clicked()
        {
            let mut scores: Vec<(usize, f32)> = self
                .photos
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    (
                        i,
                        p.photo.analysis.sharpness - (p.photo.analysis.mean - 0.5).abs() * 0.04,
                    )
                })
                .collect();
            scores.sort_by(|a, b| b.1.total_cmp(&a.1));
            for p in &mut self.photos {
                p.rating = 0;
            }
            for (i, _) in scores.into_iter().take(5) {
                self.photos[i].rating = 5;
            }
            self.notify("Rated up to 5 picks by sharpness and exposure. Check expressions and eyes manually.");
        }
        ui.add_space(20.0);
        section(ui, "SESSION", |ui| {
            for p in &self.photos {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(if p.rating == 5 { "★" } else { "·" }).color(YELLOW));
                    ui.label(RichText::new(&p.photo.name).size(11.0));
                });
                if let Some(note) = &p.note {
                    ui.label(RichText::new(note).size(10.0).color(YELLOW));
                }
                ui.add_space(7.0);
            }
        });
    }

    fn model_panel(&mut self, ui: &mut egui::Ui) {
        ui.label(
            RichText::new(if model::AVAILABLE {
                "ONNX support enabled"
            } else {
                "Local tools are ready"
            })
            .size(14.0),
        );
        ui.add_space(10.0);
        ui.label(
            RichText::new(
                "AI segmentation uses a model you supply. Your photos stay on this device.",
            )
            .size(12.0)
            .color(MUTED),
        );
        ui.add_space(18.0);
        if !model::AVAILABLE {
            ui.label(RichText::new("Build with the onnx feature to enable model loading. Setup instructions are in the README.").size(11.0).color(MUTED));
        }
        egui::ComboBox::from_id_salt("model-kind")
            .selected_text(if self.model_kind == Kind::FaceParsing {
                "Face parsing · BiSeNet"
            } else {
                "Foreground · RMBG"
            })
            .width(ui.available_width())
            .show_ui(ui, |ui| {
                ui.selectable_value(
                    &mut self.model_kind,
                    Kind::FaceParsing,
                    "Face parsing · 19 classes",
                );
                ui.selectable_value(
                    &mut self.model_kind,
                    Kind::Background,
                    "Foreground · probability mask",
                );
            });
        ui.add_space(10.0);
        egui::ComboBox::from_id_salt("provider")
            .selected_text(self.provider.label())
            .width(ui.available_width())
            .show_ui(ui, |ui| {
                for p in [
                    Provider::Cpu,
                    Provider::DirectMl,
                    Provider::Cuda,
                    Provider::CoreMl,
                ] {
                    ui.selectable_value(&mut self.provider, p, p.label());
                }
            });
        ui.add_space(16.0);
        if ui
            .add_enabled(
                model::AVAILABLE && !self.model_busy,
                primary(if self.model_busy {
                    "Processing…"
                } else {
                    "Load model & create mask…"
                })
                .min_size(vec2(ui.available_width(), 40.0)),
            )
            .clicked()
            && let Some(path) = rfd::FileDialog::new()
                .add_filter("ONNX model", &["onnx"])
                .pick_file()
        {
            let p = &self.photos[self.current];
            self.model_busy = true;
            let _ = self.tx.send(Job::Infer {
                id: p.id,
                path,
                kind: self.model_kind,
                provider: self.provider,
                image: p.photo.preview.clone(),
            });
        }
        ui.add_space(26.0);
        section(ui, "CURRENT RELEASE", |ui| {
            for text in [
                "Texture-preserving local retouch",
                "Manual spot healing and masks",
                "Linear-light color and tone",
                "Optional face/background segmentation",
                "Versioned JPG / PNG export",
            ] {
                ui.label(RichText::new(format!("·  {text}")).size(11.0).color(MUTED));
                ui.add_space(6.0);
            }
            ui.add_space(10.0);
            ui.label(RichText::new("Face/body sculpting, makeup, automatic flyaway cleanup, LaMa inpainting, RAW development, and age/gender detection are not available in this release.").size(11.0).color(MUTED));
        });
    }

    fn brush_controls(&mut self, ui: &mut egui::Ui) {
        if let Some(target) = self.brush {
            ui.add_space(18.0);
            egui::Frame::new()
                .fill(RAISED)
                .corner_radius(10)
                .inner_margin(12)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!("{} brush", target.label()))
                                .size(12.0)
                                .color(YELLOW),
                        );
                        if ui.button("Done").clicked() {
                            self.brush = None;
                        }
                    });
                    let mut radius = self.brush_radius * 1000.0;
                    slider(
                        ui,
                        "Brush size",
                        &mut radius,
                        3.0..=150.0,
                        "Radius relative to the image’s short edge.",
                    );
                    self.brush_radius = radius / 1000.0;
                    if target != Target::Heal {
                        ui.checkbox(&mut self.erase, "Erase mask");
                    }
                    ui.label(
                        RichText::new("Paint on the photo. Space + drag to pan.")
                            .size(10.0)
                            .color(MUTED),
                    );
                });
        }
    }

    fn filmstrip(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("filmstrip")
            .exact_height(154.0)
            .frame(
                egui::Frame::new()
                    .fill(PANEL)
                    .inner_margin(egui::Margin::symmetric(20, 12))
                    .stroke(Line::new(1.0_f32, BORDER)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("YOUR SESSION").size(9.0).color(MUTED));
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new(format!("{} photos", self.photos.len()))
                            .size(10.0)
                            .color(MUTED),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if small_chip(ui, "Sync settings", false)
                            .on_hover_text("Copy current adjustments to selected photos")
                            .clicked()
                        {
                            self.sync();
                        }
                        ui.label(
                            RichText::new("Ctrl + click to select")
                                .size(10.0)
                                .color(MUTED),
                        );
                    });
                });
                ui.add_space(8.0);
                egui::ScrollArea::horizontal()
                    .id_salt("filmstrip-scroll")
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let mut select = None;
                            for (index, photo) in self.photos.iter_mut().enumerate() {
                                let (rect, response) =
                                    ui.allocate_exact_size(vec2(72.0, 90.0), Sense::click());
                                let image_rect = Rect::from_min_size(
                                    rect.min + vec2(3.0, 3.0),
                                    vec2(66.0, 72.0),
                                );
                                ui.painter().rect_filled(rect, 8.0, RAISED);
                                ui.painter().image(
                                    photo.edited_texture.id(),
                                    image_rect,
                                    Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                                    Color32::WHITE,
                                );
                                if index == self.current {
                                    ui.painter().rect_stroke(
                                        rect,
                                        8.0,
                                        Line::new(1.5_f32, YELLOW),
                                        egui::StrokeKind::Inside,
                                    );
                                }
                                if photo.selected {
                                    ui.painter().circle_filled(
                                        image_rect.right_top() + vec2(-8.0, 8.0),
                                        4.0,
                                        YELLOW,
                                    );
                                }
                                if photo.edit != Edit::default() {
                                    ui.painter().text(
                                        image_rect.left_bottom() + vec2(4.0, -7.0),
                                        Align2::LEFT_CENTER,
                                        "✓",
                                        FontId::proportional(12.0),
                                        YELLOW,
                                    );
                                }
                                ui.painter().text(
                                    rect.center_bottom() + vec2(0.0, -7.0),
                                    Align2::CENTER_CENTER,
                                    format!(
                                        "{:02}{}",
                                        index + 1,
                                        if photo.rating > 0 { "  ★" } else { "" }
                                    ),
                                    FontId::proportional(9.0),
                                    MUTED,
                                );
                                if response.clicked() {
                                    if ctx.input(|i| i.modifiers.ctrl || i.modifiers.command) {
                                        photo.selected = !photo.selected;
                                    } else {
                                        select = Some(index);
                                    }
                                }
                                response.on_hover_text(format!(
                                    "{}\n{} × {}\n{}",
                                    photo.photo.name,
                                    photo.photo.original.width(),
                                    photo.photo.original.height(),
                                    photo.note.as_deref().unwrap_or("Original preserved")
                                ));
                                ui.add_space(5.0);
                            }
                            if let Some(index) = select {
                                self.current = index;
                                self.zoom = 1.0;
                                self.pan = Pan::ZERO;
                                self.gesture = false;
                            }
                            let (r, response) =
                                ui.allocate_exact_size(vec2(64.0, 90.0), Sense::click());
                            ui.painter().rect_stroke(
                                r,
                                8.0,
                                Line::new(1.0_f32, BORDER),
                                egui::StrokeKind::Inside,
                            );
                            ui.painter().text(
                                r.center() + vec2(0.0, -6.0),
                                Align2::CENTER_CENTER,
                                "+",
                                FontId::proportional(24.0),
                                MUTED,
                            );
                            ui.painter().text(
                                r.center() + vec2(0.0, 20.0),
                                Align2::CENTER_CENTER,
                                "Import",
                                FontId::proportional(9.0),
                                MUTED,
                            );
                            if response.clicked() {
                                self.import();
                            }
                        });
                    });
            });
    }

    fn canvas(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(BG)
                    .inner_margin(egui::Margin::symmetric(26, 20)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("PORTRAIT WORKSPACE").size(10.0).color(MUTED));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if small_chip(ui, "Before / After", self.compare)
                            .on_hover_text("Toggle split comparison · B")
                            .clicked()
                        {
                            self.compare = !self.compare;
                        }
                        if self
                            .photos
                            .get(self.current)
                            .is_some_and(|p| p.photo.path.is_none())
                        {
                            ui.label(RichText::new("GENERATED DEMO").size(9.0).color(MUTED));
                        }
                    });
                });
                ui.add_space(14.0);
                let (viewport, _) = ui.allocate_exact_size(
                    vec2(
                        ui.available_width(),
                        (ui.available_height() - 52.0).max(100.0),
                    ),
                    Sense::hover(),
                );
                if self.photos.is_empty() {
                    star(
                        ui.painter(),
                        viewport.center() + vec2(0.0, -60.0),
                        24.0,
                        YELLOW,
                    );
                    ui.painter().text(
                        viewport.center(),
                        Align2::CENTER_CENTER,
                        if self.importing {
                            "Preparing your studio…"
                        } else {
                            "A great portrait starts here."
                        },
                        FontId::proportional(24.0),
                        TEXT,
                    );
                    ui.painter().text(
                        viewport.center() + vec2(0.0, 37.0),
                        Align2::CENTER_CENTER,
                        "Drop photos here or use Import",
                        FontId::proportional(13.0),
                        MUTED,
                    );
                    return;
                }
                let p = &self.photos[self.current];
                let dimensions = vec2(
                    p.photo.preview.width() as f32,
                    p.photo.preview.height() as f32,
                );
                let fit =
                    (viewport.width() / dimensions.x).min(viewport.height() / dimensions.y) * 0.96;
                let size = dimensions * fit * self.zoom;
                let center = viewport.center() + vec2(self.pan.x, self.pan.y);
                let image_rect = Rect::from_center_size(center, size);
                let response = ui.interact(
                    viewport,
                    ui.id().with("photo-canvas"),
                    Sense::click_and_drag(),
                );
                let painter = ui.painter().with_clip_rect(viewport);
                painter.rect_filled(image_rect.expand(1.0), 0.0, BORDER);
                let original_held = ctx.input(|i| i.key_down(egui::Key::Backslash));
                if self.gpu {
                    painter.add(egui_wgpu::Callback::new_paint_callback(
                        image_rect,
                        gpu::Canvas {
                            key: (p.id, p.rendered_revision),
                            original: p.photo.preview.clone(),
                            edited: p.edited.clone(),
                            split: if self.compare { self.split } else { 0.0 },
                            show_original: original_held,
                        },
                    ));
                } else {
                    let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
                    painter.image(
                        if original_held {
                            p.original_texture.id()
                        } else {
                            p.edited_texture.id()
                        },
                        image_rect,
                        uv,
                        Color32::WHITE,
                    );
                    if self.compare {
                        let before = Rect::from_min_max(
                            image_rect.min,
                            pos2(
                                image_rect.left() + image_rect.width() * self.split,
                                image_rect.bottom(),
                            ),
                        );
                        painter.with_clip_rect(before.intersect(viewport)).image(
                            p.original_texture.id(),
                            image_rect,
                            uv,
                            Color32::WHITE,
                        );
                    }
                }
                let mut handle_hovered = false;
                if self.compare && !original_held {
                    let x = image_rect.left() + image_rect.width() * self.split;
                    let line_rect = Rect::from_min_max(
                        pos2(x - 12.0, image_rect.top()),
                        pos2(x + 12.0, image_rect.bottom()),
                    )
                    .intersect(viewport);
                    let handle =
                        ui.interact(line_rect, ui.id().with("split-handle"), Sense::drag());
                    handle_hovered = handle.hovered() || handle.dragged();
                    if handle.dragged()
                        && let Some(pos) = handle.interact_pointer_pos()
                    {
                        self.split =
                            ((pos.x - image_rect.left()) / image_rect.width()).clamp(0.01, 0.99);
                    }
                    painter.line_segment(
                        [pos2(x, image_rect.top()), pos2(x, image_rect.bottom())],
                        Line::new(1.5_f32, YELLOW),
                    );
                    painter.circle_filled(pos2(x, image_rect.center().y), 14.0, YELLOW);
                    painter.text(
                        pos2(x, image_rect.center().y),
                        Align2::CENTER_CENTER,
                        "‹ ›",
                        FontId::proportional(14.0),
                        BG,
                    );
                    pill(&painter, image_rect.left_top() + vec2(14.0, 14.0), "BEFORE");
                    pill(
                        &painter,
                        image_rect.right_top() + vec2(-76.0, 14.0),
                        "AFTER",
                    );
                }
                let space = ctx.input(|i| i.key_down(egui::Key::Space));
                if self.brush.is_none() || space {
                    if response.dragged() && !handle_hovered {
                        let delta = response.drag_delta();
                        self.pan += Pan::new(delta.x, delta.y);
                    }
                } else if !handle_hovered {
                    let target = self.brush.unwrap();
                    if let Some(pos) = response.interact_pointer_pos().or(response.hover_pos())
                        && image_rect.contains(pos)
                    {
                        let uv = [
                            (pos.x - image_rect.left()) / image_rect.width(),
                            (pos.y - image_rect.top()) / image_rect.height(),
                        ];
                        let radius = self.brush_radius * size.x.min(size.y);
                        painter.circle_stroke(pos, radius, Line::new(1.0_f32, YELLOW));
                        painter.circle_stroke(
                            pos,
                            radius + 1.0,
                            Line::new(1.0_f32, Color32::from_black_alpha(150)),
                        );
                        let stamp = if target == Target::Heal {
                            response.clicked()
                        } else {
                            response.dragged() || response.clicked()
                        };
                        let enough_distance = self.last_stroke.is_none_or(|last| {
                            ((last[0] - uv[0]).powi(2) + (last[1] - uv[1]).powi(2)).sqrt()
                                > self.brush_radius * 0.3
                        });
                        if stamp && enough_distance {
                            if !self.gesture {
                                let before = self.photos[self.current].edit.clone();
                                self.photos[self.current].history.record(before);
                                self.gesture = true;
                            }
                            self.photos[self.current].edit.strokes.push(Stroke {
                                target,
                                center: uv,
                                radius: self.brush_radius,
                                erase: self.erase && target != Target::Heal,
                            });
                            self.last_stroke = Some(uv);
                            self.edited();
                        }
                    }
                    for stroke in self.photos[self.current]
                        .edit
                        .strokes
                        .iter()
                        .filter(|s| s.target == target && !s.erase)
                        .rev()
                        .take(400)
                    {
                        let center = image_rect.min
                            + vec2(stroke.center[0] * size.x, stroke.center[1] * size.y);
                        painter.circle_filled(
                            center,
                            stroke.radius * size.x.min(size.y),
                            Color32::from_rgba_unmultiplied(255, 214, 10, 28),
                        );
                    }
                }
                if response.hovered() {
                    let scroll = ctx.input(|i| i.smooth_scroll_delta.y);
                    if scroll.abs() > 0.1 {
                        let old = self.zoom;
                        self.zoom = (self.zoom * (scroll * 0.003).exp()).clamp(0.5, 8.0);
                        if let Some(pos) = response.hover_pos() {
                            let anchor =
                                Pan::new(pos.x - viewport.center().x, pos.y - viewport.center().y);
                            self.pan = (self.pan - anchor) * (self.zoom / old) + anchor;
                        }
                    }
                }
                if self.render_busy || self.model_busy {
                    let time = ctx.input(|i| i.time) as f32;
                    let x = image_rect.left() + ((time * 0.7).fract()) * image_rect.width();
                    painter.rect_filled(
                        Rect::from_center_size(
                            pos2(x, image_rect.center().y),
                            vec2(2.0, image_rect.height()),
                        )
                        .intersect(viewport),
                        0.0,
                        Color32::from_rgba_unmultiplied(255, 214, 10, 65),
                    );
                }
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    let p = &self.photos[self.current];
                    ui.label(
                        RichText::new(format!(
                            "{} × {}  ·  sRGB",
                            p.photo.original.width(),
                            p.photo.original.height()
                        ))
                        .size(10.0)
                        .color(MUTED),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if small_chip(ui, "Fit", self.zoom == 1.0).clicked() {
                            self.zoom = 1.0;
                            self.pan = Pan::ZERO;
                        }
                        if small_chip(ui, "100%", false)
                            .on_hover_text(
                                "100% of the preview. Exports use full original resolution.",
                            )
                            .clicked()
                        {
                            self.zoom = 1.0 / fit;
                            self.pan = Pan::ZERO;
                        }
                        if small_chip(ui, "+", false).clicked() {
                            self.zoom = (self.zoom * 1.2).min(8.0);
                        }
                        ui.label(
                            RichText::new(format!("{:.0}%", fit * self.zoom * 100.0))
                                .size(11.0)
                                .color(TEXT),
                        );
                        if small_chip(ui, "−", false).clicked() {
                            self.zoom = (self.zoom / 1.2).max(0.5);
                        }
                    });
                });
            });
    }

    fn export_dialog(&mut self, ctx: &egui::Context) {
        if !self.export_open {
            return;
        }
        let mut open = self.export_open;
        egui::Window::new("Export your portraits")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .default_width(370.0)
            .frame(
                egui::Frame::window(&ctx.style())
                    .fill(PANEL)
                    .corner_radius(16)
                    .inner_margin(24),
            )
            .show(ctx, |ui| {
                ui.label(RichText::new("The finishing touch.").size(21.0).strong());
                ui.add_space(8.0);
                ui.label(
                    RichText::new(
                        "Originals stay untouched. Every export gets a new version number.",
                    )
                    .size(12.0)
                    .color(MUTED),
                );
                ui.add_space(22.0);
                ui.label(RichText::new("SIZE").size(10.0).color(MUTED));
                ui.add_space(8.0);
                egui::ComboBox::from_id_salt("export-size")
                    .selected_text(self.export_size.label())
                    .width(ui.available_width())
                    .show_ui(ui, |ui| {
                        for size in [
                            ExportSize::Original,
                            ExportSize::Web,
                            ExportSize::Instagram,
                            ExportSize::Story,
                            ExportSize::LinkedIn,
                        ] {
                            ui.selectable_value(&mut self.export_size, size, size.label());
                        }
                    });
                if matches!(
                    self.export_size,
                    ExportSize::Instagram | ExportSize::Story | ExportSize::LinkedIn
                ) {
                    ui.add_space(5.0);
                    ui.label(
                        RichText::new("Social sizes use a centered crop. Check the framing.")
                            .size(11.0)
                            .color(YELLOW),
                    );
                }
                ui.add_space(20.0);
                ui.label(RichText::new("FORMAT").size(10.0).color(MUTED));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.export_png, false, "JPG");
                    ui.selectable_value(&mut self.export_png, true, "PNG");
                });
                if !self.export_png {
                    let mut quality = self.quality as f32;
                    slider(
                        ui,
                        "JPEG quality",
                        &mut quality,
                        60.0..=100.0,
                        "95 is suitable for print. JPG composites transparency on white.",
                    );
                    self.quality = quality as u8;
                }
                if self
                    .photos
                    .get(self.current)
                    .is_some_and(|p| p.edit.settings.background == Background::Transparent)
                    && !self.export_png
                {
                    ui.label(
                        RichText::new("Choose PNG to keep transparency. JPG uses white.")
                            .size(11.0)
                            .color(YELLOW),
                    );
                }
                ui.add_space(16.0);
                ui.checkbox(&mut self.export_all, "Export selected photos in the batch");
                ui.add_space(24.0);
                if ui
                    .add_sized(
                        [ui.available_width(), 42.0],
                        primary("Choose folder & export  ↗"),
                    )
                    .clicked()
                    && let Some(directory) = rfd::FileDialog::new()
                        .set_title("Choose export folder")
                        .pick_folder()
                {
                    let photos: Vec<_> = self
                        .photos
                        .iter()
                        .enumerate()
                        .filter(|(i, p)| {
                            if self.export_all {
                                p.selected
                            } else {
                                *i == self.current
                            }
                        })
                        .map(|(_, p)| (p.photo.clone(), p.edit.clone(), p.seg.clone()))
                        .collect();
                    if photos.is_empty() {
                        self.notify("Select at least one photo to export.");
                    } else {
                        self.exporting = Some((0, photos.len()));
                        self.export_errors = 0;
                        self.exported_paths.clear();
                        let _ = self.tx.send(Job::Export {
                            photos,
                            directory,
                            size: self.export_size,
                            png: self.export_png,
                            quality: self.quality,
                        });
                        self.export_open = false;
                    }
                }
            });
        self.export_open = self.export_open && open;
    }

    fn status(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("status")
            .exact_height(30.0)
            .frame(
                egui::Frame::new()
                    .fill(BG)
                    .inner_margin(egui::Margin::symmetric(20, 6)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let message = if let Some((finished, total)) = self.exporting {
                        format!("Exporting · {finished}/{total}")
                    } else if self.importing {
                        "Importing photos…".into()
                    } else if self.model_busy {
                        "Creating mask…".into()
                    } else if self.render_busy {
                        "Updating preview…".into()
                    } else {
                        "Original preserved  ·  All processing stays on your device".into()
                    };
                    ui.label(RichText::new(message).size(10.0).color(MUTED));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(RichText::new("ASTRA  /  0.1").size(9.0).color(MUTED));
                    });
                });
            });
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.wants_keyboard_input() {
            return;
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::O)) {
            self.import();
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::S)) {
            self.save_session();
        }
        if ctx.input_mut(|i| {
            i.consume_key(
                egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                egui::Key::Z,
            )
        }) {
            self.redo();
        } else if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Z)) {
            self.undo();
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::E))
            && !self.photos.is_empty()
        {
            self.export_open = true;
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::B)) {
            self.compare = !self.compare;
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            self.brush = None;
            self.export_open = false;
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Num0)) {
            self.zoom = 1.0;
            self.pan = Pan::ZERO;
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::A)) {
            self.apply_preset(0);
        }
    }
}

impl eframe::App for AstraApp {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        self.frames += 1;
        self.poll(ctx);
        self.shortcuts(ctx);
        let dropped = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.clone())
                .collect::<Vec<_>>()
        });
        if !dropped.is_empty() && self.photos.len() + dropped.len() <= 64 {
            self.importing = true;
            let _ = self.tx.send(Job::Import(dropped));
        }
        self.header(ctx);
        self.status(ctx);
        self.rail(ctx);
        self.tools(ctx);
        self.filmstrip(ctx);
        self.canvas(ctx);
        self.export_dialog(ctx);
        if !ctx.input(|i| i.pointer.any_down()) {
            self.gesture = false;
            self.last_stroke = None;
        }
        if let Some((message, instant)) = &self.toast
            && instant.elapsed() < Duration::from_secs(9)
        {
            egui::Area::new("toast".into())
                .order(egui::Order::Foreground)
                .anchor(Align2::CENTER_BOTTOM, vec2(0.0, -180.0))
                .show(ctx, |ui| {
                    egui::Frame::new()
                        .fill(RAISED)
                        .stroke(Line::new(1.0_f32, BORDER))
                        .corner_radius(12)
                        .inner_margin(14)
                        .show(ui, |ui| {
                            ui.set_max_width(650.0);
                            ui.label(RichText::new(message).size(12.0));
                        });
                });
            ctx.request_repaint_after(Duration::from_secs(9).saturating_sub(instant.elapsed()));
        }
        if self.help {
            egui::Window::new("Astra shortcuts")
                .open(&mut self.help)
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    for (key, action) in [
                        ("Ctrl + O", "Import photos"),
                        ("Ctrl + S", "Save edit session"),
                        ("Ctrl + Z", "Undo"),
                        ("Ctrl + Shift + Z", "Redo"),
                        ("Ctrl + E", "Export"),
                        ("A", "Auto retouch"),
                        ("B", "Before / after"),
                        ("Hold backslash", "View original"),
                        ("0", "Fit photo"),
                        ("Space + drag", "Pan"),
                        ("Scroll", "Zoom around pointer"),
                        ("Escape", "Finish brush / close export"),
                    ] {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(key).color(YELLOW));
                            ui.label(action);
                        });
                    }
                    ui.add_space(15.0);
                    ui.label("Import photos you own or have permission to edit.");
                });
        }
        if let Some(path) = &self.screenshot {
            ctx.request_repaint_after(Duration::from_millis(30));
            if self.frames > 30
                && !self.importing
                && !self.render_busy
                && self
                    .photos
                    .get(self.current)
                    .is_none_or(|p| p.revision == p.rendered_revision)
                && !self.screenshot_requested
            {
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
                self.screenshot_requested = true;
            }
            let screenshots = ctx.input(|i| {
                i.events
                    .iter()
                    .filter_map(|e| {
                        if let egui::Event::Screenshot { image, .. } = e {
                            Some(image.clone())
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
            });
            for screenshot in screenshots {
                if let Some(parent) = path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let bytes: Vec<u8> = screenshot
                    .pixels
                    .iter()
                    .flat_map(|p| p.to_array())
                    .collect();
                let result = image::save_buffer(
                    path,
                    &bytes,
                    screenshot.size[0] as u32,
                    screenshot.size[1] as u32,
                    image::ColorType::Rgba8,
                );
                if let Err(e) = result {
                    eprintln!("Screenshot failed: {e}");
                }
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }
}

fn theme(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = PANEL;
    visuals.window_fill = PANEL;
    visuals.override_text_color = Some(TEXT);
    visuals.selection.bg_fill = Color32::from_rgb(58, 51, 17);
    visuals.selection.stroke = Line::new(1.0_f32, YELLOW);
    visuals.widgets.inactive.bg_fill = RAISED;
    visuals.widgets.inactive.weak_bg_fill = RAISED;
    visuals.widgets.inactive.bg_stroke = Line::new(1.0_f32, BORDER);
    visuals.widgets.inactive.corner_radius = egui::CornerRadius::same(8);
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(40, 40, 44);
    visuals.widgets.hovered.bg_stroke = Line::new(1.0_f32, Color32::from_rgb(66, 62, 34));
    visuals.widgets.hovered.corner_radius = egui::CornerRadius::same(8);
    visuals.widgets.active.bg_fill = Color32::from_rgb(54, 48, 20);
    visuals.widgets.active.bg_stroke = Line::new(1.0_f32, YELLOW);
    visuals.widgets.active.corner_radius = egui::CornerRadius::same(8);
    visuals.window_corner_radius = egui::CornerRadius::same(14);
    ctx.set_visuals(visuals);
    ctx.style_mut(|style| {
        style.spacing.item_spacing = vec2(8.0, 8.0);
        style.spacing.button_padding = vec2(12.0, 7.0);
        style.animation_time = 0.18;
        style
            .text_styles
            .insert(egui::TextStyle::Body, FontId::proportional(12.0));
        style
            .text_styles
            .insert(egui::TextStyle::Button, FontId::proportional(12.0));
    });
    if let Ok(font) = std::fs::read("C:/Windows/Fonts/segoeui.ttf") {
        let mut fonts = egui::FontDefinitions::default();
        fonts
            .font_data
            .insert("studio".into(), egui::FontData::from_owned(font).into());
        fonts
            .families
            .get_mut(&egui::FontFamily::Proportional)
            .unwrap()
            .insert(0, "studio".into());
        if let Ok(symbols) = std::fs::read("C:/Windows/Fonts/seguisym.ttf") {
            fonts
                .font_data
                .insert("symbols".into(), egui::FontData::from_owned(symbols).into());
            fonts
                .families
                .get_mut(&egui::FontFamily::Proportional)
                .unwrap()
                .insert(1, "symbols".into());
        }
        ctx.set_fonts(fonts);
    }
}
fn primary(text: &str) -> egui::Button<'_> {
    egui::Button::new(RichText::new(text).color(BG).strong())
        .fill(YELLOW)
        .stroke(Line::NONE)
        .corner_radius(9)
}
fn secondary(text: &str) -> egui::Button<'_> {
    egui::Button::new(RichText::new(text).color(TEXT))
        .fill(RAISED)
        .stroke(Line::new(1.0_f32, BORDER))
        .corner_radius(9)
}
fn small_chip(ui: &mut egui::Ui, text: &str, active: bool) -> egui::Response {
    ui.add(
        egui::Button::new(RichText::new(text).size(10.0).color(if active {
            YELLOW
        } else {
            MUTED
        }))
        .fill(if active {
            Color32::from_rgb(45, 40, 19)
        } else {
            RAISED
        })
        .stroke(Line::new(
            1.0_f32,
            if active {
                Color32::from_rgb(86, 73, 17)
            } else {
                BORDER
            },
        ))
        .corner_radius(6),
    )
}
fn section(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui)) {
    egui::CollapsingHeader::new(RichText::new(title).size(10.0).color(MUTED))
        .default_open(true)
        .show_unindented(ui, |ui| {
            ui.add_space(10.0);
            add(ui);
        });
}

fn slider(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    tooltip: &str,
) {
    ui.push_id(label, |ui| {
        let min = *range.start();
        let max = *range.end();
        ui.horizontal(|ui| {
            ui.label(RichText::new(label).size(12.0).color(MUTED));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add(
                    egui::DragValue::new(value)
                        .range(range.clone())
                        .speed(if max <= 2.0 { 0.01 } else { 1.0 })
                        .max_decimals(if max <= 2.0 { 2 } else { 0 }),
                );
            });
        });
        let (rect, mut response) =
            ui.allocate_exact_size(vec2(ui.available_width(), 18.0), Sense::click_and_drag());
        let track = Rect::from_min_max(
            pos2(rect.left() + 5.0, rect.center().y - 1.5),
            pos2(rect.right() - 5.0, rect.center().y + 1.5),
        );
        if (response.clicked() || response.dragged())
            && let Some(pos) = response.interact_pointer_pos()
        {
            *value = (min + ((pos.x - track.left()) / track.width()).clamp(0.0, 1.0) * (max - min))
                .clamp(min, max);
            if max > 2.0 {
                *value = value.round();
            }
            response.mark_changed();
            response.request_focus();
        }
        if response.has_focus() {
            let direction = ui.input(|i| {
                i.key_pressed(egui::Key::ArrowRight) as i32
                    - i.key_pressed(egui::Key::ArrowLeft) as i32
            });
            if direction != 0 {
                *value = (*value + direction as f32 * if max <= 2.0 { 0.01 } else { 1.0 })
                    .clamp(min, max);
            }
        }
        let t = ((*value - min) / (max - min)).clamp(0.0, 1.0);
        let x = track.left() + track.width() * t;
        ui.painter()
            .rect_filled(track, 2.0, Color32::from_rgb(62, 62, 65));
        if min < 0.0 {
            let zero = track.left() + track.width() * (-min / (max - min));
            ui.painter().rect_filled(
                Rect::from_min_max(
                    pos2(zero.min(x), track.top()),
                    pos2(zero.max(x), track.bottom()),
                ),
                2.0,
                YELLOW,
            );
        } else {
            ui.painter().rect_filled(
                Rect::from_min_max(track.min, pos2(x, track.bottom())),
                2.0,
                YELLOW,
            );
        }
        ui.painter().circle_filled(
            pos2(x, track.center().y),
            8.0,
            Color32::from_rgba_unmultiplied(255, 214, 10, 18),
        );
        ui.painter()
            .circle_filled(pos2(x, track.center().y), 4.5, YELLOW);
        response.on_hover_text(tooltip);
        ui.add_space(10.0);
    });
}

fn star(painter: &egui::Painter, center: Pos2, radius: f32, color: Color32) {
    let points: Vec<Pos2> = (0..8)
        .map(|i| {
            let angle = i as f32 * std::f32::consts::TAU / 8.0 - std::f32::consts::FRAC_PI_2;
            let r = if i % 2 == 0 { radius } else { radius * 0.26 };
            center + vec2(angle.cos() * r, angle.sin() * r)
        })
        .collect();
    painter.add(egui::Shape::convex_polygon(points, color, Line::NONE));
}
fn pill(painter: &egui::Painter, min: Pos2, text: &str) {
    let r = Rect::from_min_size(min, vec2(62.0, 23.0));
    painter.rect_filled(r, 5.0, Color32::from_black_alpha(170));
    painter.text(
        r.center(),
        Align2::CENTER_CENTER,
        text,
        FontId::proportional(8.0),
        TEXT,
    );
}

fn rail_button(
    ui: &mut egui::Ui,
    tool: Option<Tool>,
    active: bool,
    tooltip: &str,
) -> egui::Response {
    let (r, response) = ui.allocate_exact_size(vec2(50.0, 46.0), Sense::click());
    let painter = ui.painter();
    if active {
        painter.rect_filled(r, 12.0, Color32::from_rgb(48, 42, 18));
    } else if response.hovered() {
        painter.rect_filled(r, 12.0, RAISED);
    }
    let color = if active { YELLOW } else { MUTED };
    let line = Line::new(1.5_f32, color);
    let c = r.center();
    match tool {
        None => {
            painter.rect_stroke(
                Rect::from_center_size(c + vec2(0.0, 3.0), vec2(21.0, 17.0)),
                3.0,
                line,
                egui::StrokeKind::Inside,
            );
            painter.line_segment([c + vec2(0.0, 1.0), c + vec2(0.0, -10.0)], line);
            painter.line_segment([c + vec2(-4.0, -6.0), c + vec2(0.0, -10.0)], line);
            painter.line_segment([c + vec2(4.0, -6.0), c + vec2(0.0, -10.0)], line);
        }
        Some(Tool::Portrait) => {
            painter.circle_stroke(c + vec2(0.0, -4.0), 6.0, line);
            painter.add(egui::Shape::line(
                vec![
                    c + vec2(-10.0, 11.0),
                    c + vec2(-8.0, 5.0),
                    c + vec2(0.0, 2.0),
                    c + vec2(8.0, 5.0),
                    c + vec2(10.0, 11.0),
                ],
                line,
            ));
        }
        Some(Tool::Skin) => {
            painter.add(egui::Shape::closed_line(
                vec![
                    c + vec2(0.0, -12.0),
                    c + vec2(-8.0, 1.0),
                    c + vec2(-7.0, 7.0),
                    c + vec2(0.0, 11.0),
                    c + vec2(7.0, 7.0),
                    c + vec2(8.0, 1.0),
                ],
                line,
            ));
        }
        Some(Tool::Color) => {
            painter.circle_stroke(c, 6.0, line);
            for i in 0..8 {
                let angle = i as f32 * std::f32::consts::TAU / 8.0;
                let d = vec2(angle.cos(), angle.sin());
                painter.line_segment([c + d * 9.0, c + d * 12.0], line);
            }
        }
        Some(Tool::Background) => {
            painter.rect_stroke(
                Rect::from_center_size(c, vec2(22.0, 21.0)),
                3.0,
                line,
                egui::StrokeKind::Inside,
            );
            painter.add(egui::Shape::line(
                vec![
                    c + vec2(-8.0, 6.0),
                    c + vec2(-1.0, -2.0),
                    c + vec2(3.0, 2.0),
                    c + vec2(7.0, -3.0),
                ],
                line,
            ));
            painter.circle_filled(c + vec2(-5.0, -5.0), 2.0, color);
        }
        Some(Tool::Presets) => star(painter, c, 12.0, color),
        Some(Tool::Batch) => {
            for (x, y) in [(-6.0, -6.0), (6.0, -6.0), (-6.0, 6.0), (6.0, 6.0)] {
                painter.rect_stroke(
                    Rect::from_center_size(c + vec2(x, y), vec2(9.0, 9.0)),
                    2.0,
                    line,
                    egui::StrokeKind::Inside,
                );
            }
        }
        Some(Tool::Models) => {
            painter.circle_stroke(c, 7.0, line);
            for i in 0..8 {
                let angle = i as f32 * std::f32::consts::TAU / 8.0;
                let d = vec2(angle.cos(), angle.sin());
                painter.line_segment([c + d * 7.0, c + d * 11.0], line);
            }
            painter.circle_stroke(c, 2.0, line);
        }
    }
    response.on_hover_text(tooltip)
}
