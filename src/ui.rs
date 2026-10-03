use crate::session::{self, RecoveryStore, SavedPhoto, Session};
use crate::{
    branding,
    engine::{
        self, Adjustment, Background, Edit, ExportSize, History, Photo, Preset, Segmentation,
        Settings, Stroke, Target,
    },
    gpu,
    interaction::{self, Gesture, View},
    layer_stack::{BlendMode, LayerType},
    layers::LayerKind,
    model::{self, Kind, Provider},
    nullstate::PortraitDemand,
    preferences::{self, Preferences},
    worker::{self, Job, Message, RenderKind},
};
use crossbeam_channel::{Receiver, Sender};
use eframe::egui::{
    self, Align2, Color32, FontId, Pos2, Rect, RichText, Sense, Stroke as Line, Vec2, pos2, vec2,
};
use image::RgbaImage;
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{
        Arc, Weak,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

const BG: Color32 = Color32::from_rgb(11, 11, 12);
const PANEL: Color32 = Color32::from_rgb(20, 20, 22);
const RAISED: Color32 = Color32::from_rgb(29, 29, 31);
const YELLOW: Color32 = Color32::from_rgb(255, 214, 10);
const TEXT: Color32 = Color32::from_rgb(239, 239, 241);
const MUTED: Color32 = Color32::from_rgb(146, 146, 154);
const BORDER: Color32 = Color32::from_rgb(39, 39, 43);

#[path = "asset_workflow.rs"]
mod asset_workflow;
#[path = "layer_panel.rs"]
mod layer_panel;
use layer_panel::{LayerAction, LayerFilter};
#[path = "app_chrome.rs"]
mod app_chrome;
#[path = "manual_tools.rs"]
mod manual_tools;
use app_chrome::PreferencesTab;

#[cfg(test)]
#[path = "ui_tests.rs"]
mod tests;

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

#[derive(Clone, Copy, PartialEq, Eq)]
enum Inspector {
    Tools,
    History,
    Layers,
}
impl Inspector {
    fn label(self) -> &'static str {
        match self {
            Self::Tools => "Tools",
            Self::History => "History",
            Self::Layers => "Layers",
        }
    }
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
    ai_cancel: Option<Arc<AtomicBool>>,
    asset_cache_paths: Vec<PathBuf>,
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
    view: View,
    crop_center: [f32; 2],
    processing: Option<u64>,
    ai_processing: bool,
    ai_status: Option<model::AiStatus>,
    ai_request: u64,
    ai_provider: Option<Provider>,
    ai_seg_provider: Option<Provider>,
    ai_requested: PortraitDemand,
    ai_failed: PortraitDemand,
}

struct PreviewSnapshot {
    photo_id: u64,
    edit: Edit,
    segmentation: Option<Weak<Segmentation>>,
    image: Arc<RgbaImage>,
}

impl PreviewSnapshot {
    fn from_photo(photo: &PhotoState) -> Self {
        Self {
            photo_id: photo.id,
            edit: photo.edit.clone(),
            segmentation: photo.seg.as_ref().map(Arc::downgrade),
            image: photo.edited.clone(),
        }
    }

    fn same_state(&self, other: &Self) -> bool {
        self.photo_id == other.photo_id
            && self.edit == other.edit
            && match (&self.segmentation, &other.segmentation) {
                (None, None) => true,
                (Some(a), Some(b)) => match (a.upgrade(), b.upgrade()) {
                    (Some(a), Some(b)) => Arc::ptr_eq(&a, &b),
                    _ => false,
                },
                _ => false,
            }
    }
}

const PREVIEW_HISTORY_BYTES: usize = 32 * 1024 * 1024;

fn center_crop() -> [f32; 2] {
    [0.5; 2]
}

struct DetailTile {
    key: (u64, u64, Option<usize>, engine::Crop),
    source_generation: u64,
    original: Arc<RgbaImage>,
    edited: Arc<RgbaImage>,
    original_texture: egui::TextureHandle,
    edited_texture: egui::TextureHandle,
    serial: u64,
}

#[derive(Default)]
struct PatchDraft {
    boundary: Vec<[f32; 2]>,
    drawing: bool,
    source_drag: Option<([f32; 2], [f32; 2])>,
}

type ReferenceResult = (u64, u64, anyhow::Result<crate::color::ReferenceProfile>);
type PhotoFingerprint = (
    u64,
    u64,
    u8,
    bool,
    f32,
    bool,
    [f32; 2],
    [f32; 2],
    Option<usize>,
);

#[derive(PartialEq)]
struct WorkspaceFingerprint {
    current: usize,
    compare: bool,
    split: f32,
    photos: Vec<PhotoFingerprint>,
}

type MaskTexture = (
    u64,
    Edit,
    Option<Arc<Segmentation>>,
    Target,
    engine::Crop,
    egui::TextureHandle,
);

pub struct HasturApp {
    photos: Vec<PhotoState>,
    preview_history: VecDeque<PreviewSnapshot>,
    preview_history_bytes: usize,
    current: usize,
    next_id: u64,
    tool: Tool,
    tx: Sender<Job>,
    rx: Receiver<Message>,
    importing: bool,
    import_batches: usize,
    render_busy: bool,
    preview_cancel: Option<(u64, u64, Arc<AtomicBool>)>,
    detail_cancel: Option<Arc<AtomicBool>>,
    hover_cancel: Option<Arc<AtomicBool>>,
    last_edit: Instant,
    last_preview_request: Instant,
    brush_preview: Option<(u64, u64)>,
    last_detail_request: Instant,
    gesture: bool,
    compare: bool,
    split: f32,
    canvas_gesture: Gesture,
    selection_anchor: usize,
    detail_region: Option<(u64, u64, engine::Crop, Arc<RgbaImage>)>,
    detail_region_generation: u64,
    detail_region_pending: Option<(u64, u64, engine::Crop)>,
    detail_tile: Option<DetailTile>,
    tile_serial: u64,
    hover_preset: Option<usize>,
    hover_image: Option<(u64, u64, usize, Arc<RgbaImage>, egui::TextureHandle)>,
    hover_pending: Option<(u64, u64, usize)>,
    hover_region: Option<(u64, u64, usize, engine::Crop, Arc<RgbaImage>)>,
    hover_region_generation: u64,
    hover_region_pending: Option<(u64, u64, usize, engine::Crop)>,
    preset_key: Option<(u64, u64, usize)>,
    preset_thumbs: Vec<Option<egui::TextureHandle>>,
    mask_texture: Option<MaskTexture>,
    brush_softness: f32,
    show_mask: bool,
    export_cancel: Option<Arc<AtomicBool>>,
    crop_index: usize,
    launch_native: bool,
    brush: Option<Target>,
    brush_radius: f32,
    erase: bool,
    last_stroke: Option<[f32; 2]>,
    brush_strength: f32,
    clone_anchor: Option<(u64, [f32; 2])>,
    clone_origin: Option<[f32; 2]>,
    clone_aligned: bool,
    clone_offset: Option<(u64, [f32; 2])>,
    clone_pick_source: bool,
    patch_draft: PatchDraft,
    patch_feedback: manual_tools::PatchFeedback,
    auto_ai: bool,
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
    pending_session_cursor: usize,
    restored_current_id: Option<u64>,
    recovery_store: Option<RecoveryStore>,
    recovery_candidates: VecDeque<PathBuf>,
    automatic_recovery: bool,
    recovery_source: Option<(PathBuf, u64)>,
    autosave_observed: Option<WorkspaceFingerprint>,
    autosave_dirty_since: Option<Instant>,
    autosave_last_change: Instant,
    autosave_generation: u64,
    autosave_pending: Option<u64>,
    autosave_saved_at: Option<Instant>,
    gpu: bool,
    gpu_adapter: String,
    startup: bool,
    splash_preview: bool,
    splash_texture: Option<egui::TextureHandle>,
    screenshot: Option<PathBuf>,
    screenshot_requested: bool,
    screenshot_ready_since: Option<Instant>,
    frames: u64,
    compact_open: bool,
    inspector: Option<Inspector>,
    last_inspector: Inspector,
    layer_thumbs: Vec<(u64, u64, u64, egui::TextureHandle)>,
    layer_thumbs_pending: Option<(u64, u64)>,
    layer_drag: Option<u64>,
    layer_rename: Option<(u64, u64, String)>,
    layer_query: String,
    layer_filter: LayerFilter,
    layer_clipboard: Option<Settings>,
    history_focus: bool,
    export_report: Option<PathBuf>,
    reference_results: Receiver<ReferenceResult>,
    reference_sender: Sender<ReferenceResult>,
    pending_reference: Option<(u64, u64)>,
    reference_serial: u64,
    hsl_band: usize,
    curve_channel: usize,
    curve_drag_knot: Option<usize>,
    preferences: Preferences,
    preferences_path: Option<PathBuf>,
    preferences_open: bool,
    preferences_draft: Preferences,
    preferences_tab: PreferencesTab,
    preferences_error: Option<String>,
    shortcut_capture: Option<String>,
    command_open: bool,
    command_query: String,
    command_index: usize,
    command_focus: bool,
    filmstrip_visible: bool,
    focus_restore: Option<(Option<Inspector>, bool, bool)>,
    photo_focus: bool,
    canvas_viewport: Option<Rect>,
    active_asset: Option<u64>,
    asset_loading: Option<(u64, u64, Arc<AtomicBool>)>,
    asset_serial: u64,
    asset_error: Option<u64>,
    gpu_release: bool,
    export_serial: u64,
    export_queue_open: bool,
    export_jobs: Vec<asset_workflow::ExportQueueItem>,
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

fn cached_preview(
    history: &mut VecDeque<PreviewSnapshot>,
    bytes: &mut usize,
    photo_id: u64,
    edit: &Edit,
    segmentation: Option<&Arc<Segmentation>>,
) -> Option<Arc<RgbaImage>> {
    let index = history.iter().position(|entry| {
        entry.photo_id == photo_id
            && entry.edit == *edit
            && match (&entry.segmentation, segmentation) {
                (None, None) => true,
                (Some(old), Some(current)) => {
                    old.upgrade().is_some_and(|old| Arc::ptr_eq(&old, current))
                }
                _ => false,
            }
    })?;
    let entry = history.remove(index)?;
    *bytes -= entry.image.as_raw().len();
    let image = entry.image.clone();
    *bytes += image.as_raw().len();
    history.push_back(entry);
    Some(image)
}

fn apply_preview_texture(
    ctx: &egui::Context,
    photo: &mut PhotoState,
    image: &RgbaImage,
    gpu: bool,
) {
    if photo.edited_texture.id() == photo.original_texture.id() {
        photo.edited_texture = photo_texture(ctx, &format!("edited-{}", photo.id), image, gpu);
    } else {
        let display = display_image(image, gpu);
        photo.edited_texture.set(
            egui::ColorImage::from_rgba_unmultiplied(
                [display.width() as usize, display.height() as usize],
                display.as_raw(),
            ),
            egui::TextureOptions::LINEAR,
        );
    }
}

impl HasturApp {
    fn toggle_brush(&mut self, target: Target) {
        self.patch_feedback.clear();
        self.end_canvas_gesture();
        self.brush = (self.brush != Some(target)).then_some(target);
        if self.brush.is_some_and(manual_tools::is_manual) {
            self.compare = false;
        }
        self.clone_pick_source = false;
        self.last_stroke = None;
        self.clone_origin = None;
        self.patch_draft = PatchDraft::default();
    }

    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        cc.egui_ctx.set_zoom_factor(1.1);
        theme(&cc.egui_ctx);
        let gpu = gpu::install(cc);
        let (tx, rx) = worker::start(cc.egui_ctx.clone());
        let arguments: Vec<String> = std::env::args().collect();
        let screenshot = arguments
            .iter()
            .position(|a| a == "--screenshot")
            .and_then(|i| arguments.get(i + 1))
            .map(PathBuf::from);
        let saved_session = arguments
            .iter()
            .position(|a| a == "--session")
            .and_then(|i| arguments.get(i + 1))
            .map(PathBuf::from);
        let paths: Vec<PathBuf> = arguments
            .iter()
            .skip(1)
            .filter(|a| !a.starts_with('-'))
            .map(PathBuf::from)
            .filter(|p| {
                p.is_file() && Some(p) != screenshot.as_ref() && Some(p) != saved_session.as_ref()
            })
            .collect();
        let restore_workspace = paths.is_empty() && screenshot.is_none() && saved_session.is_none();
        let focus_workspace = arguments.iter().any(|a| a == "--focus-workspace");
        let hide_filmstrip = arguments.iter().any(|a| a == "--hide-filmstrip");
        let initial = if let Some(path) = saved_session {
            Job::ReadSession(path)
        } else if paths.is_empty() {
            Job::Demo
        } else {
            Job::Import(paths)
        };
        let mut app = Self::from_channels(tx, rx, gpu, arguments, screenshot);
        if app.screenshot.is_none() {
            let path = preferences::path();
            match preferences::load(&path) {
                Ok(preferences) => app.preferences = preferences,
                Err(error) => app.notify(format!("Using default preferences: {error}")),
            }
            app.preferences_path = Some(path);
        }
        app.preferences_draft = app.preferences.clone();
        app.apply_preferences(&cc.egui_ctx);
        if app.export_open {
            app.export_size = ExportSize::Story;
        }
        if hide_filmstrip {
            app.filmstrip_visible = false;
        }
        if focus_workspace {
            app.toggle_focus();
        }
        app.auto_ai = model::AVAILABLE;
        app.startup = true;
        app.gpu_adapter = cc
            .wgpu_render_state
            .as_ref()
            .map(|state| {
                let info = state.adapter.get_info();
                format!("{} · {:?}", info.name, info.backend)
            })
            .unwrap_or_else(|| "Software canvas".into());
        if app.screenshot.is_none() {
            match RecoveryStore::new() {
                Ok(store) => app.recovery_store = Some(store),
                Err(error) => app.notify(format!("Autosave is unavailable: {error}")),
            }
        }
        let restoring = restore_workspace
            && app.preferences.restore_workspace
            && app.resume_latest_workspace(
                session::recovery_candidates()
                    .into_iter()
                    .map(|candidate| candidate.path)
                    .collect(),
            );
        if !restoring {
            let _ = app.tx.send(initial);
        }
        app
    }
    fn from_channels(
        tx: Sender<Job>,
        rx: Receiver<Message>,
        gpu: bool,
        arguments: Vec<String>,
        screenshot: Option<PathBuf>,
    ) -> Self {
        let (reference_sender, reference_results) = crossbeam_channel::unbounded();
        let patch_preview =
            screenshot.is_some() && arguments.iter().any(|a| a == "--patch-preview");
        let manual_tool = arguments
            .iter()
            .position(|a| a == "--manual-tool")
            .and_then(|i| arguments.get(i + 1))
            .and_then(|name| match name.as_str() {
                "liquify" => Some(Target::Liquify),
                "heal" => Some(Target::Heal),
                "clone" => Some(Target::Clone),
                "patch" => Some(Target::Patch),
                _ => None,
            });
        Self {
            photos: vec![],
            active_asset: None,
            asset_loading: None,
            asset_serial: 0,
            asset_error: None,
            gpu_release: false,
            export_serial: 0,
            export_queue_open: false,
            export_jobs: Vec::new(),
            preview_history: VecDeque::new(),
            preview_history_bytes: 0,
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
            import_batches: 1,
            render_busy: false,
            preview_cancel: None,
            detail_cancel: None,
            hover_cancel: None,
            last_edit: Instant::now(),
            last_preview_request: Instant::now() - Duration::from_millis(70),
            brush_preview: None,
            last_detail_request: Instant::now() - Duration::from_millis(70),
            gesture: false,
            compare: arguments.iter().any(|a| a == "--compare"),
            split: 0.5,
            canvas_gesture: Gesture::None,
            selection_anchor: 0,
            detail_region: None,
            detail_region_generation: 0,
            detail_region_pending: None,
            detail_tile: None,
            tile_serial: 0,
            hover_preset: None,
            hover_image: None,
            hover_pending: None,
            hover_region: None,
            hover_region_generation: 0,
            hover_region_pending: None,
            preset_key: None,
            preset_thumbs: vec![],
            mask_texture: None,
            brush_softness: 0.75,
            show_mask: true,
            export_cancel: None,
            crop_index: 0,
            launch_native: arguments.iter().any(|a| a == "--100"),
            brush: if patch_preview {
                Some(Target::Patch)
            } else {
                manual_tool
            },
            brush_radius: 0.018,
            erase: false,
            last_stroke: None,
            brush_strength: 100.0,
            clone_anchor: None,
            clone_origin: None,
            clone_aligned: true,
            clone_offset: None,
            clone_pick_source: false,
            patch_draft: if patch_preview {
                PatchDraft {
                    boundary: vec![
                        [0.608, 0.418],
                        [0.632, 0.418],
                        [0.632, 0.442],
                        [0.608, 0.442],
                    ],
                    drawing: false,
                    source_drag: Some(([0.62, 0.43], [0.65, 0.44])),
                }
            } else {
                PatchDraft::default()
            },
            patch_feedback: manual_tools::PatchFeedback::default(),
            auto_ai: false,
            export_open: arguments.iter().any(|a| a == "--export-preview"),
            export_size: if arguments.iter().any(|a| a == "--export-preview") {
                ExportSize::Story
            } else {
                ExportSize::Original
            },
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
            provider: Provider::Auto,
            model_busy: false,
            pending_session: None,
            pending_session_cursor: 0,
            restored_current_id: None,
            recovery_store: None,
            recovery_candidates: VecDeque::new(),
            automatic_recovery: false,
            recovery_source: None,
            autosave_observed: None,
            autosave_dirty_since: None,
            autosave_last_change: Instant::now(),
            autosave_generation: 0,
            autosave_pending: None,
            autosave_saved_at: None,
            gpu,
            gpu_adapter: String::new(),
            startup: false,
            splash_preview: arguments.iter().any(|a| a == "--splash-preview"),
            splash_texture: None,
            screenshot,
            screenshot_requested: false,
            screenshot_ready_since: None,
            frames: 0,
            compact_open: arguments
                .iter()
                .any(|a| a == "--history" || a == "--layers"),
            inspector: if arguments.iter().any(|a| a == "--history") {
                Some(Inspector::History)
            } else if arguments.iter().any(|a| a == "--layers") {
                Some(Inspector::Layers)
            } else if arguments.iter().any(|a| a == "--hide-panels") {
                None
            } else {
                Some(Inspector::Tools)
            },
            layer_thumbs: vec![],
            layer_thumbs_pending: None,
            layer_drag: None,
            layer_rename: None,
            layer_query: String::new(),
            layer_filter: LayerFilter::All,
            layer_clipboard: None,
            last_inspector: if arguments.iter().any(|a| a == "--history") {
                Inspector::History
            } else if arguments.iter().any(|a| a == "--layers") {
                Inspector::Layers
            } else {
                Inspector::Tools
            },
            history_focus: true,
            export_report: None,
            reference_results,
            reference_sender,
            pending_reference: None,
            reference_serial: 0,
            hsl_band: 1,
            curve_channel: 0,
            curve_drag_knot: None,
            preferences: Preferences::default(),
            preferences_path: None,
            preferences_open: arguments.iter().any(|a| a == "--preferences-preview"),
            preferences_draft: Preferences::default(),
            preferences_tab: if arguments.iter().any(|a| a == "--shortcuts-preferences") {
                PreferencesTab::Shortcuts
            } else if arguments.iter().any(|a| a == "--editing-preferences") {
                PreferencesTab::Editing
            } else if arguments.iter().any(|a| a == "--workflow-preferences") {
                PreferencesTab::Workflow
            } else if arguments.iter().any(|a| a == "--export-preferences") {
                PreferencesTab::Export
            } else {
                PreferencesTab::Workspace
            },
            preferences_error: None,
            shortcut_capture: None,
            command_open: arguments.iter().any(|a| a == "--commands-preview"),
            command_query: String::new(),
            command_index: 0,
            command_focus: true,
            filmstrip_visible: !arguments.iter().any(|a| a == "--hide-filmstrip"),
            focus_restore: None,
            photo_focus: false,
            canvas_viewport: None,
        }
    }

    fn notify(&mut self, text: impl Into<String>) {
        self.toast = Some((text.into(), Instant::now()));
    }
    fn edited(&mut self) {
        if let Some(photo) = self.photos.get_mut(self.current) {
            if self.canvas_gesture == Gesture::Brush {
                if self.brush_preview.is_none_or(|(id, _)| id != photo.id) {
                    self.brush_preview = Some((photo.id, photo.revision));
                }
            } else {
                self.brush_preview = None;
            }
            photo.revision += 1;
            photo.edit.sync_active_layer();
        }
        self.last_edit = Instant::now();
    }

    /// NullState: importing or painting does not implicitly call portrait models.
    /// An enabled adjustment requests only its dependencies, once per photo.
    fn prepare_requested_ai(&mut self) {
        if !self.auto_ai || self.canvas_gesture == Gesture::Brush {
            return;
        }
        if let Some(photo) = self.photos.get(self.current) {
            let demand = photo.edit.portrait_demand();
            self.request_portrait(self.current, demand, false);
        }
    }

    fn request_portrait(&mut self, index: usize, demand: PortraitDemand, force: bool) {
        let demand = demand.normalized();
        if demand.is_empty() {
            return;
        }
        let Some(photo) = self.photos.get_mut(index) else {
            return;
        };
        if !photo.photo.resident {
            return;
        }
        let same_provider = photo.ai_provider == Some(self.provider);
        let same_maps = photo.ai_seg_provider == Some(self.provider);
        let prepared = if same_maps && !force {
            photo
                .seg
                .as_ref()
                .map_or(PortraitDemand::NONE, |seg| seg.prepared)
        } else {
            PortraitDemand::NONE
        };
        let missing = demand.without(prepared);
        if !force
            && (missing.is_empty()
                || (same_provider && photo.ai_processing)
                || (same_provider && photo.ai_failed.contains(missing)))
        {
            return;
        }
        let base = if same_maps && !force {
            photo.seg.clone()
        } else {
            None
        };
        if force || !same_provider {
            photo.ai_failed = PortraitDemand::NONE;
        }
        photo.ai_request += 1;
        photo.ai_provider = Some(self.provider);
        photo.ai_requested = missing;
        photo.ai_status = None;
        photo.note = Some("Preparing requested portrait tools…".into());
        if let Some(cancel) = photo.ai_cancel.take() {
            cancel.store(true, Ordering::Relaxed);
        }
        let cancel = Arc::new(AtomicBool::new(false));
        photo.ai_cancel = Some(cancel.clone());
        photo.ai_processing = self
            .tx
            .send(Job::Portrait {
                cancel,
                id: photo.id,
                request: photo.ai_request,
                force,
                provider: self.provider,
                demand,
                base,
                image: photo.photo.preview.clone(),
            })
            .is_ok();
        if !photo.ai_processing {
            photo.ai_failed = photo.ai_failed.union(missing);
            photo.note = Some("Portrait AI worker is unavailable; manual tools are ready".into());
        }
    }

    fn end_canvas_gesture(&mut self) {
        self.gesture = false;
        self.last_stroke = None;
        self.clone_origin = None;
        self.canvas_gesture = Gesture::None;
    }

    fn remember_preview(&mut self, snapshot: PreviewSnapshot) {
        let bytes = snapshot.image.as_raw().len();
        if bytes > PREVIEW_HISTORY_BYTES {
            return;
        }
        self.preview_history.retain(|entry| {
            !entry.same_state(&snapshot)
                && entry
                    .segmentation
                    .as_ref()
                    .is_none_or(|segmentation| segmentation.strong_count() > 0)
        });
        self.preview_history_bytes = self
            .preview_history
            .iter()
            .map(|entry| entry.image.as_raw().len())
            .sum();
        self.preview_history.push_back(snapshot);
        self.preview_history_bytes += bytes;
        while self.preview_history_bytes > PREVIEW_HISTORY_BYTES {
            if let Some(oldest) = self.preview_history.pop_front() {
                self.preview_history_bytes -= oldest.image.as_raw().len();
            } else {
                self.preview_history_bytes = 0;
                break;
            }
        }
    }

    fn history_step(&mut self, ctx: &egui::Context, redo: bool) {
        let Some(photo) = self.photos.get(self.current) else {
            return;
        };
        let (past, future) = photo.history.depths();
        let target = if redo && future > 0 {
            past + 1
        } else if !redo && past > 0 {
            past - 1
        } else {
            return;
        };
        self.history_jump(ctx, target);
    }

    /// Navigate snapshots once, restoring only the final preview and edit revision.
    fn history_jump(&mut self, ctx: &egui::Context, target: usize) {
        let Some(photo) = self.photos.get(self.current) else {
            return;
        };
        let (past, future) = photo.history.depths();
        if target == past || target > past + future {
            return;
        }
        self.brush_preview = None;
        let snapshot = self
            .photos
            .get(self.current)
            .filter(|photo| photo.revision == photo.rendered_revision)
            .map(PreviewSnapshot::from_photo);
        if let Some(snapshot) = snapshot {
            self.remember_preview(snapshot);
        }
        let Some(photo) = self.photos.get_mut(self.current) else {
            return;
        };
        for _ in target..past {
            photo.history.undo(&mut photo.edit);
        }
        for _ in past..target {
            photo.history.redo(&mut photo.edit);
        }
        self.patch_draft = PatchDraft::default();
        photo.revision += 1;
        photo.processing = None;
        let (id, revision, edit, segmentation) = (
            photo.id,
            photo.revision,
            photo.edit.clone(),
            photo.seg.clone(),
        );
        self.end_canvas_gesture();
        if let Some((preview_id, preview_revision, cancel)) = &self.preview_cancel
            && (*preview_id != id || *preview_revision != revision)
        {
            cancel.store(true, Ordering::Relaxed);
        }
        let cached = cached_preview(
            &mut self.preview_history,
            &mut self.preview_history_bytes,
            id,
            &edit,
            segmentation.as_ref(),
        );
        if let Some(image) = cached {
            if let Some(photo) = self.photos.get_mut(self.current) {
                apply_preview_texture(ctx, photo, &image, self.gpu);
                photo.edited = image;
                photo.rendered_revision = revision;
            }
            self.last_edit = Instant::now();
        } else {
            self.last_edit = Instant::now() - Duration::from_millis(66);
        }
        ctx.request_repaint();
    }

    fn poll(&mut self, ctx: &egui::Context) {
        self.poll_color_reference(ctx);
        let live_brush = self.canvas_gesture == Gesture::Brush
            && self.gesture
            && ctx.input(|i| i.pointer.primary_down());
        let current_photo_id = self.photos.get(self.current).map(|p| p.id);
        if let Some((id, rev, cancel)) = &self.preview_cancel
            && (!self.photos.iter().any(|p| {
                p.id == *id
                    && (p.revision == *rev || (live_brush && Some(p.id) == current_photo_id))
            }) || self
                .photos
                .get(self.current)
                .is_some_and(|p| p.id != *id && p.revision != p.rendered_revision))
        {
            cancel.store(true, Ordering::Relaxed);
        }
        if let Some((id, rev, _)) = self.detail_region_pending
            && let Some(cancel) = &self.detail_cancel
            && !self.photos.get(self.current).is_some_and(|p| {
                p.id == id
                    && (p.revision == rev || live_brush)
                    && needs_detail_resolution(&p.photo, p.view.scale)
            })
        {
            cancel.store(true, Ordering::Relaxed);
        }
        if let Some((id, rev, index)) = self.hover_pending
            && let Some(cancel) = &self.hover_cancel
            && !self.photos.get(self.current).is_some_and(|p| {
                p.id == id && p.revision == rev && self.hover_preset == Some(index)
            })
        {
            cancel.store(true, Ordering::Relaxed);
        }
        while let Ok(message) = self.rx.try_recv() {
            match message {
                Message::AssetCaches { id, paths } => {
                    if let Some(photo) = self.photos.iter_mut().find(|p| p.id == id) {
                        photo.asset_cache_paths.extend(paths);
                    } else {
                        for path in paths {
                            let _ = std::fs::remove_file(path);
                        }
                    }
                }
                Message::AssetLoaded {
                    id,
                    request,
                    result,
                } => self.accept_asset(ctx, id, request, result),
                Message::PatchPreview {
                    id,
                    revision,
                    request,
                    crop,
                    result,
                } => self.accept_patch_preview(ctx, id, revision, request, crop, result),
                Message::LayerThumbnails {
                    id,
                    revision,
                    images,
                } => {
                    if self.layer_thumbs_pending == Some((id, revision)) {
                        self.layer_thumbs_pending = None;
                    }
                    if self
                        .photos
                        .iter()
                        .any(|p| p.id == id && p.revision == revision)
                    {
                        self.layer_thumbs.retain(|(photo, _, _, _)| *photo != id);
                        for (layer, image) in images {
                            self.layer_thumbs.push((
                                id,
                                layer,
                                revision,
                                texture(ctx, &format!("layer-{id}-{layer}"), &image),
                            ));
                        }
                        while self.layer_thumbs.len() > 128 {
                            let _ = self.layer_thumbs.remove(0);
                        }
                    }
                }
                Message::HoverRegionRendered {
                    id,
                    revision,
                    index,
                    crop,
                    image,
                } => {
                    if self.hover_region_pending == Some((id, revision, index, crop)) {
                        self.hover_region_pending = None;
                        self.hover_pending = None;
                        self.hover_cancel = None;
                        if self
                            .photos
                            .get(self.current)
                            .is_some_and(|p| p.id == id && p.revision == revision)
                            && self.hover_preset == Some(index)
                            && image.dimensions() == (crop.width, crop.height)
                        {
                            self.hover_region = Some((id, revision, index, crop, Arc::new(image)));
                            self.hover_region_generation += 1;
                        }
                    }
                }
                Message::HoverRegionCancelled {
                    id,
                    revision,
                    index,
                    crop,
                } => {
                    if self.hover_region_pending == Some((id, revision, index, crop)) {
                        self.hover_region_pending = None;
                        self.hover_pending = None;
                        self.hover_cancel = None;
                    }
                }
                Message::DetailRegionRendered {
                    id,
                    revision,
                    crop,
                    image,
                } => {
                    if self.detail_region_pending == Some((id, revision, crop)) {
                        self.detail_region_pending = None;
                        self.detail_cancel = None;
                        if self.photos.get(self.current).is_some_and(|p| {
                            p.id == id
                                && (p.revision == revision
                                    || (live_brush
                                        && revision <= p.revision
                                        && self.detail_region.as_ref().is_none_or(
                                            |(old_id, old_rev, _, _)| {
                                                *old_id != id || revision >= *old_rev
                                            },
                                        )))
                        }) && image.dimensions() == (crop.width, crop.height)
                        {
                            self.detail_region = Some((id, revision, crop, Arc::new(image)));
                            self.detail_region_generation += 1;
                        }
                    }
                }
                Message::DetailRegionCancelled { id, revision, crop } => {
                    if self.detail_region_pending == Some((id, revision, crop)) {
                        self.detail_region_pending = None;
                        self.detail_cancel = None;
                    }
                }
                Message::SessionSaved(result) => match result {
                    Ok(()) => self.notify("Session saved with undo and redo history"),
                    Err(e) => self.notify(format!("Could not save session: {e}")),
                },
                Message::RecoverySaved { generation, result } => {
                    if self.autosave_pending == Some(generation) {
                        self.autosave_pending = None;
                        match result {
                            Ok(()) => {
                                self.autosave_saved_at = Some(Instant::now());
                                if self
                                    .recovery_source
                                    .as_ref()
                                    .is_some_and(|(_, minimum)| generation >= *minimum)
                                    && let Some((source, _)) = self.recovery_source.take()
                                {
                                    let _ = session::discard_recovery(&source);
                                }
                            }
                            Err(e) => {
                                self.autosave_dirty_since = Some(Instant::now());
                                self.autosave_last_change = Instant::now();
                                self.notify(format!("Autosave failed; retrying: {e}"));
                            }
                        }
                    }
                }
                Message::SessionRead(result) => match result {
                    Ok(session) => {
                        if self.automatic_recovery
                            && !session.photos.is_empty()
                            && !session
                                .photos
                                .iter()
                                .any(|photo| photo.path.as_ref().is_none_or(|path| path.is_file()))
                        {
                            self.recovery_failed();
                            continue;
                        }
                        self.photos.clear();
                        self.current = 0;
                        self.importing = true;
                        self.import_batches = 1;
                        self.pending_session_cursor = 0;
                        self.restored_current_id = None;
                        self.compare = session.compare;
                        self.split = session.split.clamp(0.02, 0.98);
                        let photos = session.photos.iter().map(|p| p.path.clone()).collect();
                        self.pending_session = Some(session);
                        let _ = self.tx.send(Job::OpenSession { photos });
                    }
                    Err(e) => {
                        if self.automatic_recovery {
                            self.recovery_failed();
                            continue;
                        }
                        self.importing = false;
                        self.recovery_source = None;
                        self.notify(format!("Could not open session: {e}"));
                    }
                },
                Message::RenderCancelled { id, revision, kind } => match kind {
                    RenderKind::Preview => {
                        if self
                            .preview_cancel
                            .as_ref()
                            .is_some_and(|(old, rev, _)| *old != id || *rev != revision)
                        {
                            continue;
                        }
                        self.render_busy = false;
                        self.preview_cancel = None;
                        if let Some(p) = self.photos.iter_mut().find(|p| p.id == id)
                            && p.processing == Some(revision)
                        {
                            p.processing = None;
                        }
                    }
                    RenderKind::Hover(index) => {
                        if self.hover_region_pending.is_none()
                            && self.hover_pending == Some((id, revision, index))
                        {
                            self.hover_pending = None;
                            self.hover_cancel = None;
                        }
                    }
                },
                Message::ExportReport(result) => match result {
                    Ok(path) => self.export_report = Some(path),
                    Err(e) => self.notify(format!(
                        "Images exported, but report could not be saved: {e}"
                    )),
                },
                Message::Imported(result) => {
                    match result {
                        Ok(photo) => {
                            if self.photos.len() >= 64 {
                                self.notify("Project limit reached · remaining imports skipped.");
                                continue;
                            }
                            let id = self.next_id;
                            self.next_id += 1;
                            let tex = photo_texture(
                                ctx,
                                &format!("original-{id}"),
                                &photo.preview,
                                self.gpu,
                            );
                            let mut edit = Edit::default();
                            let mut history = History::default();
                            let mut selected = true;
                            let mut rating = 0;
                            let mut segmentation = None;
                            let mut view = View {
                                fit: !self.launch_native,
                                ..View::default()
                            };
                            let mut crop_center = center_crop();
                            if let Some(session) = &self.pending_session
                                && let Some(saved) = session.photos.get(self.pending_session_cursor)
                            {
                                view = saved.view;
                                crop_center = saved.crop_center;
                                edit = saved.edit.clone();
                                history = saved.history.clone();
                                selected = saved.selected;
                                rating = saved.rating;
                                segmentation = saved.segmentation.clone().map(Arc::new);
                                if session.current == self.pending_session_cursor {
                                    self.restored_current_id = Some(id);
                                }
                            }
                            self.photos.push(PhotoState {
                                ai_cancel: None,
                                asset_cache_paths: Vec::new(),
                                id,
                                edited: photo.preview.clone(),
                                photo,
                                edit,
                                history,
                                original_texture: tex.clone(),
                                edited_texture: tex,
                                revision: 1,
                                rendered_revision: 0,
                                selected,
                                rating,
                                seg: segmentation,
                                note: None,
                                view,
                                crop_center,
                                processing: None,
                                ai_processing: false,
                                ai_status: None,
                                ai_request: 0,
                                ai_provider: None,
                                ai_seg_provider: None,
                                ai_requested: PortraitDemand::NONE,
                                ai_failed: PortraitDemand::NONE,
                            });
                            if self.photos.len() == 1 || self.restored_current_id == Some(id) {
                                self.current = self.photos.len() - 1;
                            }
                        }
                        Err(e) => self.notify(format!("Import failed: {e:#}")),
                    }
                    self.pending_session_cursor += 1;
                }
                Message::ImportFinished => {
                    self.import_batches = self.import_batches.saturating_sub(1);
                    self.importing = self.import_batches > 0;
                    let restored_empty = self
                        .pending_session
                        .as_ref()
                        .is_some_and(|s| s.photos.is_empty());
                    if let Some(saved) = &self.pending_session {
                        if self.photos.len() != saved.photos.len() {
                            // Keep the original recovery when source files were moved or unavailable.
                            self.recovery_source = None;
                            self.notify("Some session originals are missing. The saved session remains available for recovery.");
                        }
                        if let Some(id) = self.restored_current_id
                            && let Some(index) = self.photos.iter().position(|p| p.id == id)
                        {
                            self.current = index;
                        }
                    }
                    self.pending_session = None;
                    self.selection_anchor = self.current;
                    if self.automatic_recovery {
                        if self.photos.is_empty() && !restored_empty {
                            self.recovery_failed();
                        } else {
                            self.automatic_recovery = false;
                            self.recovery_candidates.clear();
                        }
                    }
                }
                Message::RenderStarted { id, revision, kind } => {
                    if kind == RenderKind::Preview
                        && Some(id) == current_photo_id
                        && let Some(p) = self.photos.iter_mut().find(|p| p.id == id)
                    {
                        p.processing = Some(revision);
                    }
                }
                Message::PresetThumb {
                    id,
                    revision,
                    index,
                    image,
                } => {
                    if self.preset_key == Some((id, revision, self.presets.len()))
                        && index < self.preset_thumbs.len()
                    {
                        self.preset_thumbs[index] =
                            Some(texture(ctx, &format!("preset-{id}-{index}"), &image));
                    }
                }
                Message::ExportFinished {
                    queue,
                    finished,
                    total,
                    cancelled,
                } => {
                    if let Some(job) = self.export_jobs.iter_mut().find(|j| j.id == queue) {
                        job.finished = finished;
                        job.state = if cancelled { "Cancelled" } else { "Done" }.into();
                    }
                    self.exporting = self
                        .export_jobs
                        .iter()
                        .find(|j| !j.done())
                        .map(|j| (j.finished, j.total));
                    self.export_cancel = self
                        .export_jobs
                        .iter()
                        .find(|j| !j.done())
                        .map(|j| j.cancel.clone());
                    self.notify(format!(
                        "{} {finished}/{total} · {} saved · {} failures",
                        if cancelled {
                            "Cancelled after"
                        } else {
                            "Completed"
                        },
                        finished.saturating_sub(
                            self.export_jobs
                                .iter()
                                .find(|j| j.id == queue)
                                .map_or(self.export_errors, |j| j.errors)
                        ),
                        self.export_jobs
                            .iter()
                            .find(|j| j.id == queue)
                            .map_or(self.export_errors, |j| j.errors)
                    ));
                }
                Message::Rendered {
                    kind,
                    id,
                    revision,
                    image,
                } => match kind {
                    RenderKind::Preview => {
                        if self
                            .preview_cancel
                            .as_ref()
                            .is_some_and(|(old, rev, _)| *old != id || *rev != revision)
                        {
                            continue;
                        }
                        self.render_busy = false;
                        self.preview_cancel = None;
                        let mut snapshot = None;
                        if let Some(p) = self.photos.iter_mut().find(|p| p.id == id) {
                            p.processing = None;
                            if p.photo.resident
                                && Some(p.id) == current_photo_id
                                && (p.revision == revision
                                    || (live_brush
                                        && Some(p.id) == current_photo_id
                                        && revision > p.rendered_revision
                                        && revision < p.revision))
                            {
                                let image = Arc::new(image);
                                apply_preview_texture(ctx, p, &image, self.gpu);
                                p.edited = image;
                                p.rendered_revision = revision;
                                if p.revision == revision {
                                    snapshot = Some(PreviewSnapshot::from_photo(p));
                                }
                            }
                        }
                        if let Some(snapshot) = snapshot {
                            self.remember_preview(snapshot);
                        }
                    }
                    RenderKind::Hover(index) => {
                        if self
                            .photos
                            .get(self.current)
                            .is_some_and(|p| p.id == id && p.revision == revision)
                        {
                            self.hover_pending = None;
                            self.hover_region_pending = None;
                            self.hover_cancel = None;
                            let tex = photo_texture(
                                ctx,
                                "preset-hover",
                                &image::imageops::thumbnail(&image, 1440, 1440),
                                self.gpu,
                            );
                            self.hover_image = Some((id, revision, index, Arc::new(image), tex));
                        }
                    }
                },
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
                                    seg.custom_skin = true;
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
                Message::AiStatus {
                    id,
                    request,
                    mut status,
                } => {
                    if let Some(photo) = self
                        .photos
                        .iter_mut()
                        .find(|p| p.id == id && p.ai_request == request && p.ai_processing)
                    {
                        if let Some(old) = &photo.ai_status
                            && old.progress < 1.0
                        {
                            status.progress = status.progress.max(old.progress);
                        }
                        status.progress = status.progress.clamp(0.0, 1.0);
                        photo.ai_status = Some(status);
                    }
                }
                Message::PortraitReady {
                    id,
                    request,
                    complete,
                    result,
                } => {
                    let failed = result.is_err();
                    if let Some(photo) = self
                        .photos
                        .iter_mut()
                        .find(|p| p.id == id && p.ai_request == request)
                    {
                        photo.ai_processing = !complete;
                        match result {
                            Ok(mut seg) => {
                                if let Some(old) = &photo.seg {
                                    let old = old.session_copy().resample(seg.width, seg.height);
                                    seg.background = old.background;
                                    if old.custom_skin {
                                        seg.skin = old.skin;
                                        seg.teeth = old.teeth;
                                        seg.eyes = old.eyes;
                                        seg.custom_skin = true;
                                    }
                                }
                                photo.note = Some(seg.status.clone());
                                photo.ai_seg_provider = photo.ai_provider;
                                photo.seg = Some(Arc::new(seg));
                                photo.revision += 1;
                            }
                            Err(error) => {
                                let prepared = if photo.ai_seg_provider == photo.ai_provider {
                                    photo
                                        .seg
                                        .as_ref()
                                        .map_or(PortraitDemand::NONE, |seg| seg.prepared)
                                } else {
                                    PortraitDemand::NONE
                                };
                                photo.ai_failed =
                                    photo.ai_failed.union(photo.ai_requested.without(prepared));
                                photo.note = Some(format!("Portrait AI unavailable: {error:#}"))
                            }
                        }
                        if complete {
                            let cached = photo
                                .ai_status
                                .as_ref()
                                .is_some_and(|s| s.stage == "Cached portrait ready");
                            photo.ai_status = Some(model::AiStatus::new(
                                if failed {
                                    "Portrait AI unavailable"
                                } else if cached {
                                    "Cached portrait ready"
                                } else {
                                    "Portrait analysis complete"
                                },
                                photo.note.clone().unwrap_or_default(),
                                None,
                                1.0,
                            ));
                        }
                    }
                }
                Message::Exported {
                    queue,
                    finished,
                    total,
                    result,
                } => {
                    if let Some(job) = self.export_jobs.iter_mut().find(|j| j.id == queue) {
                        job.finished = finished;
                        job.state = "Exporting".into();
                        if result.is_err() {
                            job.errors += 1;
                        }
                    }
                    if let Ok(path) = result {
                        self.exported_paths.push(path);
                    } else if let Err(e) = result {
                        self.export_errors += 1;
                        self.notify(format!("Export failed: {e:#}"));
                    }
                    self.exporting = Some((finished, total));
                }
            }
        }
        self.prepare_requested_ai();
        self.schedule_preview(ctx, live_brush);
        if self.render_busy
            || self.importing
            || self.model_busy
            || self.photos.iter().any(|p| p.ai_processing)
            || self.exporting.is_some()
            || self
                .photos
                .get(self.current)
                .is_some_and(|p| p.revision != p.rendered_revision)
        {
            ctx.request_repaint_after(Duration::from_millis(16));
        }
    }

    fn schedule_preview(&mut self, ctx: &egui::Context, live_brush: bool) {
        if live_brush
            && self.photos.get(self.current).is_some_and(|photo| {
                needs_detail_resolution(&photo.photo, photo.view.scale)
                    && photo.photo.dimensions() != photo.photo.preview.dimensions()
            })
        {
            ctx.request_repaint_after(Duration::from_millis(16));
            return;
        }
        let brush_dirty = self.photos.get(self.current).is_some_and(|photo| {
            self.brush_preview.is_some_and(|(id, _)| id == photo.id)
                && photo.revision != photo.rendered_revision
        });
        let first_brush_preview = brush_dirty
            && self.photos.get(self.current).is_some_and(|photo| {
                self.brush_preview
                    .is_some_and(|(_, start)| photo.rendered_revision <= start)
            });
        if !self.render_busy
            && (self.last_edit.elapsed() > Duration::from_millis(65)
                || first_brush_preview
                || ((live_brush || brush_dirty)
                    && self.last_preview_request.elapsed() >= Duration::from_millis(16)))
        {
            let pending = self
                .photos
                .get(self.current)
                .filter(|p| p.photo.resident && p.rendered_revision != p.revision);
            if let Some(p) = pending {
                let cancel = Arc::new(AtomicBool::new(false));
                self.preview_cancel = Some((p.id, p.revision, cancel.clone()));
                self.last_preview_request = Instant::now();
                self.render_busy = self
                    .tx
                    .send(Job::Render {
                        kind: RenderKind::Preview,
                        id: p.id,
                        revision: p.revision,
                        image: p.photo.preview.clone(),
                        edit: p.edit.clone(),
                        seg: p.seg.clone(),
                        cancel,
                    })
                    .is_ok();
            }
        }
        if self.render_busy || brush_dirty {
            ctx.request_repaint_after(Duration::from_millis(16));
        }
    }

    fn import(&mut self) {
        if self.pending_session.is_some() || self.automatic_recovery {
            self.notify("Workspace recovery is still loading metadata.");
            return;
        }
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
            self.import_batches += 1;
            let _ = self.tx.send(Job::Import(paths));
        }
    }

    fn undo(&mut self, ctx: &egui::Context) {
        self.history_step(ctx, false);
    }
    fn redo(&mut self, ctx: &egui::Context) {
        self.history_step(ctx, true);
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

    fn session_snapshot(&self, clean_shutdown: bool) -> Session {
        Session {
            version: session::VERSION,
            current: self.current,
            compare: self.compare,
            split: self.split,
            clean_shutdown,
            photos: self
                .photos
                .iter()
                .map(|p| SavedPhoto {
                    view: p.view,
                    crop_center: p.crop_center,
                    path: p.photo.path.clone(),
                    edit: p.edit.clone(),
                    history: p.history.clone(),
                    selected: p.selected,
                    rating: p.rating,
                    segmentation: p.seg.as_deref().map(Segmentation::session_copy),
                })
                .collect(),
        }
    }

    fn workspace_fingerprint(&self) -> WorkspaceFingerprint {
        WorkspaceFingerprint {
            current: self.current,
            compare: self.compare,
            split: self.split,
            photos: self
                .photos
                .iter()
                .map(|p| {
                    (
                        p.id,
                        p.revision,
                        p.rating,
                        p.selected,
                        p.view.scale,
                        p.view.fit,
                        p.view.pan,
                        p.crop_center,
                        p.seg.as_ref().map(|s| Arc::as_ptr(s) as usize),
                    )
                })
                .collect(),
        }
    }

    fn autosave_tick(&mut self, ctx: &egui::Context) {
        if !self.preferences.autosave
            || self.recovery_store.is_none()
            || (self.photos.is_empty() && self.autosave_observed.is_none())
            || (self.importing
                && (self.pending_session.is_some() || self.recovery_source.is_some()))
        {
            return;
        }
        let state = self.workspace_fingerprint();
        let now = Instant::now();
        if self.autosave_observed.as_ref() != Some(&state) {
            self.autosave_observed = Some(state);
            self.autosave_dirty_since.get_or_insert(now);
            self.autosave_last_change = now;
        }
        if let Some(dirty_since) = self.autosave_dirty_since {
            let interval = self.preferences.autosave_seconds.clamp(1, 10);
            let quiet =
                now.duration_since(self.autosave_last_change) >= Duration::from_secs(interval);
            let overdue =
                now.duration_since(dirty_since) >= Duration::from_secs((interval * 5).max(10));
            if self.autosave_pending.is_none() && (quiet || overdue) {
                self.autosave_generation += 1;
                let generation = self.autosave_generation;
                let path = self.recovery_store.as_ref().unwrap().path.clone();
                let session = self.session_snapshot(false);
                if self
                    .tx
                    .send(Job::SaveRecovery {
                        path,
                        session,
                        generation,
                    })
                    .is_ok()
                {
                    self.autosave_pending = Some(generation);
                    self.autosave_dirty_since = None;
                }
            }
            ctx.request_repaint_after(Duration::from_millis(500));
        }
    }

    fn flush_recovery(&mut self) -> anyhow::Result<()> {
        if !self.preferences.autosave {
            return Ok(());
        }
        let Some(store) = &self.recovery_store else {
            return Ok(());
        };
        if self.photos.is_empty() && self.autosave_observed.is_none() {
            return Ok(());
        }
        self.autosave_generation += 1;
        session::save_recovery(
            &store.path,
            &self.session_snapshot(true),
            self.autosave_generation,
        )?;
        if !self.importing
            && let Some((source, _)) = self.recovery_source.take()
        {
            let _ = session::discard_recovery(&source);
        }
        Ok(())
    }

    fn resume_latest_workspace(&mut self, candidates: Vec<PathBuf>) -> bool {
        self.recovery_candidates.clear();
        for path in candidates {
            self.recovery_candidates.push_back(path.clone());
            let previous = path.with_extension("previous");
            if previous.is_file() {
                self.recovery_candidates.push_back(previous);
            }
        }
        self.read_next_recovery()
    }

    fn read_next_recovery(&mut self) -> bool {
        let Some(path) = self.recovery_candidates.pop_front() else {
            self.automatic_recovery = false;
            self.recovery_source = None;
            return false;
        };
        self.importing = true;
        self.import_batches = 1;
        self.automatic_recovery = true;
        self.recovery_source = Some((path.with_extension("ron"), self.autosave_generation + 1));
        self.tx.send(Job::ReadRecovery(path)).is_ok()
    }

    fn recovery_failed(&mut self) {
        if !self.read_next_recovery() {
            // No demo import can race saved-photo responses: it starts only after
            // every recovery generation failed to provide a usable workspace.
            self.importing = self.tx.send(Job::Demo).is_ok();
            self.import_batches = usize::from(self.importing);
        }
    }

    fn save_session(&mut self) {
        if self.importing {
            self.notify("Finish importing before saving a manual session. Autosave preserves the loaded photos.");
            return;
        }
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Hastur session", &["ron"])
            .set_file_name("hastur-session.ron")
            .save_file()
        {
            let session = self.session_snapshot(false);
            let _ = self.tx.send(Job::SaveSession { path, session });
        }
    }

    fn open_session(&mut self) {
        if self.importing {
            self.notify("Finish the current import before opening another session.");
            return;
        }
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Hastur session", &["ron"])
            .pick_file()
        {
            if let Err(e) = self.flush_recovery() {
                self.notify(format!("Could not preserve current workspace: {e}"));
                return;
            }
            self.recovery_source = None;
            self.importing = true;
            let _ = self.tx.send(Job::ReadSession(path));
        }
    }

    fn header(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("topbar")
            .exact_height(52.0)
            .frame(
                egui::Frame::new()
                    .fill(PANEL)
                    .inner_margin(egui::Margin::symmetric(12, 6))
                    .stroke(Line::new(1.0_f32, BORDER)),
            )
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(26.0, 28.0), Sense::hover());
                    branding::paint_logo(ui.painter(), r, branding::IVORY);
                    ui.add_space(6.0);
                    ui.label(RichText::new("hastur").size(23.0).strong().color(TEXT));
                    if ctx.content_rect().width() >= 720.0 {
                        ui.label(RichText::new("RETOUCH").size(10.0).color(MUTED));
                    }
                    ui.add_space(6.0);
                    ui.menu_button(RichText::new("File").color(MUTED), |ui| {
                        if ui
                            .button(
                                self.command_label(
                                    app_chrome::AppCommand::Import,
                                    "Import photos…",
                                ),
                            )
                            .clicked()
                        {
                            self.import();
                            ui.close();
                        }
                        if ui
                            .button(self.command_label(
                                app_chrome::AppCommand::ImportFolder,
                                "Import folder…",
                            ))
                            .clicked()
                        {
                            self.import_folder();
                            ui.close();
                        }
                        if ui
                            .button(self.command_label(
                                app_chrome::AppCommand::OpenSession,
                                "Open session…",
                            ))
                            .clicked()
                        {
                            self.open_session();
                            ui.close();
                        }
                        if ui
                            .button(self.command_label(
                                app_chrome::AppCommand::SaveSession,
                                "Save session…",
                            ))
                            .clicked()
                        {
                            self.save_session();
                            ui.close();
                        }
                        ui.label(
                            RichText::new(if !self.preferences.autosave {
                                "Autosave paused · use Save session".to_owned()
                            } else if self.autosave_pending.is_some() {
                                "Saving recovery…".to_owned()
                            } else if self.autosave_dirty_since.is_some() {
                                "Autosave pending…".to_owned()
                            } else if self.autosave_saved_at.is_some() {
                                "Autosave is up to date".to_owned()
                            } else {
                                format!(
                                    "Autosave after {} quiet seconds",
                                    self.preferences.autosave_seconds
                                )
                            })
                            .size(11.0)
                            .color(MUTED),
                        );
                        ui.separator();
                        let preferences = ui.button(
                            self.command_label(app_chrome::AppCommand::Preferences, "Preferences…"),
                        );
                        hit(ui, "file-preferences", preferences.rect);
                        if preferences.clicked() {
                            self.open_preferences();
                            ui.close();
                        }
                        if ui
                            .button(self.command_label(
                                app_chrome::AppCommand::ExportQueue,
                                "Export queue…",
                            ))
                            .clicked()
                        {
                            self.export_queue_open = true;
                            ui.close();
                        }
                        if ui
                            .button(self.command_label(
                                app_chrome::AppCommand::Shortcuts,
                                "Keyboard shortcuts",
                            ))
                            .clicked()
                        {
                            self.execute_command(ctx, app_chrome::AppCommand::Shortcuts);
                            ui.close();
                        }
                    });
                    let view_menu = ui.menu_button(RichText::new("View").color(MUTED), |ui| {
                        for panel in [Inspector::Tools, Inspector::History, Inspector::Layers] {
                            let response = ui.add(
                                egui::Button::new(panel.label())
                                    .selected(self.inspector == Some(panel))
                                    .min_size(vec2(150.0, 34.0)),
                            );
                            hit(ui, &format!("view-{}", panel.label()), response.rect);
                            if response.clicked() {
                                self.toggle_inspector(panel, ctx.content_rect().width() < 1050.0);
                                ui.close();
                            }
                        }
                        ui.separator();
                        let commands = ui.button(
                            self.command_label(app_chrome::AppCommand::Commands, "Find a command…"),
                        );
                        hit(ui, "view-commands", commands.rect);
                        if commands.clicked() {
                            self.open_commands();
                            ui.close();
                        }
                        let focus =
                            ui.add(
                                egui::Button::new(self.command_label(
                                    app_chrome::AppCommand::Focus,
                                    "Focus workspace",
                                ))
                                .selected(self.focus_restore.is_some()),
                            );
                        hit(ui, "view-focus", focus.rect);
                        if focus.clicked() {
                            self.toggle_focus();
                            ui.close();
                        }
                        let filmstrip = ui.add(
                            egui::Button::new(
                                self.command_label(app_chrome::AppCommand::Filmstrip, "Filmstrip"),
                            )
                            .selected(self.filmstrip_visible),
                        );
                        hit(ui, "view-filmstrip", filmstrip.rect);
                        if filmstrip.clicked() {
                            self.filmstrip_visible = !self.filmstrip_visible;
                            ui.close();
                        }
                        ui.separator();
                        if ui.button("Hide panels  ·  Tab").clicked() {
                            self.inspector = None;
                            self.compact_open = false;
                            ui.close();
                        }
                    });
                    hit(ui, "view-menu", view_menu.response.rect);
                    ui.add_space(4.0);
                    let undo = self
                        .photos
                        .get(self.current)
                        .is_some_and(|p| p.history.can_undo());
                    let redo = self
                        .photos
                        .get(self.current)
                        .is_some_and(|p| p.history.can_redo());
                    if layer_panel::icon_button(
                        ui,
                        layer_panel::Icon::Undo,
                        false,
                        undo,
                        "toolbar-undo",
                        "Undo",
                    )
                    .clicked()
                    {
                        self.undo(ui.ctx());
                    }
                    if layer_panel::icon_button(
                        ui,
                        layer_panel::Icon::Redo,
                        false,
                        redo,
                        "toolbar-redo",
                        "Redo",
                    )
                    .clicked()
                    {
                        self.redo(ui.ctx());
                    }
                    let name = self
                        .photos
                        .get(self.current)
                        .map(|p| p.photo.name.as_str())
                        .unwrap_or("Your next great portrait");
                    if ctx.content_rect().width() >= 1360.0 {
                        ui.add_space(12.0);
                        ui.label(RichText::new(name).size(12.0).color(MUTED));
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if layer_panel::icon_button(
                            ui,
                            layer_panel::Icon::Export,
                            true,
                            !self.photos.is_empty(),
                            "toolbar-export",
                            "Export photos",
                        )
                        .clicked()
                        {
                            self.export_open = true;
                        }
                        if ctx.content_rect().width() >= 1050.0 {
                            for panel in [Inspector::Layers, Inspector::History] {
                                let icon = if panel == Inspector::Layers {
                                    layer_panel::Icon::Stack
                                } else {
                                    layer_panel::Icon::Undo
                                };
                                let response = layer_panel::icon_button(
                                    ui,
                                    icon,
                                    self.inspector == Some(panel),
                                    true,
                                    &format!("toggle-{}", panel.label()),
                                    panel.label(),
                                );
                                if response.clicked() {
                                    self.toggle_inspector(panel, false);
                                }
                            }
                        }
                        if ctx.content_rect().width() >= 850.0
                            && layer_panel::icon_button(
                                ui,
                                layer_panel::Icon::Command,
                                false,
                                true,
                                "open-commands",
                                "Find a command · Ctrl+K",
                            )
                            .clicked()
                        {
                            self.open_commands();
                        }
                        if ctx.content_rect().width() >= 720.0
                            && layer_panel::icon_button(
                                ui,
                                layer_panel::Icon::Gear,
                                false,
                                true,
                                "open-preferences",
                                "Preferences · Ctrl+,",
                            )
                            .clicked()
                        {
                            self.open_preferences();
                        }
                    });
                });
            });
    }

    fn rail(&mut self, ctx: &egui::Context) {
        if ctx.content_rect().width() < 1050.0 {
            egui::TopBottomPanel::bottom("compact-tabs")
                .exact_height(54.0)
                .frame(
                    egui::Frame::new()
                        .fill(PANEL)
                        .inner_margin(egui::Margin::symmetric(12, 4)),
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
                                self.inspector = Some(Inspector::Tools);
                                self.last_inspector = Inspector::Tools;
                                self.brush = None;
                            }
                        }
                    });
                });
            return;
        }
        egui::SidePanel::left("rail")
            .exact_width(64.0)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(PANEL)
                    .inner_margin(egui::Margin::symmetric(6, 10))
                    .stroke(Line::new(1.0_f32, BORDER)),
            )
            .show(ctx, |ui| {
                if rail_button(ui, None, false, "Import photos · Ctrl+O").clicked() {
                    self.import();
                }
                ui.add_space(12.0);
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
                        self.inspector = Some(Inspector::Tools);
                        self.last_inspector = Inspector::Tools;
                        self.brush = None;
                    }
                    ui.add_space(4.0);
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
                        self.inspector = Some(Inspector::Tools);
                        self.last_inspector = Inspector::Tools;
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
        if self.inspector.is_none() {
            return;
        }
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
                            .inner_margin(12),
                    )
                    .show(ctx, |ui| {
                        if ui.button("Done").clicked() {
                            self.compact_open = false;
                        }
                        self.inspector_content(ui);
                    });
            }
            return;
        }
        egui::SidePanel::right("controls")
            .exact_width(300.0)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(PANEL)
                    .inner_margin(egui::Margin::symmetric(14, 12))
                    .stroke(Line::new(1.0_f32, BORDER)),
            )
            .show(ctx, |ui| {
                hit(ui, "inspector", ui.max_rect());
                self.inspector_content(ui);
            });
    }

    fn toggle_inspector(&mut self, panel: Inspector, compact: bool) {
        self.last_inspector = panel;
        if self.inspector == Some(panel) && (!compact || self.compact_open) {
            self.inspector = None;
            self.compact_open = false;
        } else {
            self.inspector = Some(panel);
            self.compact_open = true;
            self.history_focus = true;
        }
    }

    fn inspector_content(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            for panel in [Inspector::Tools, Inspector::History, Inspector::Layers] {
                let response = ui.add(
                    egui::Button::new(panel.label())
                        .selected(self.inspector == Some(panel))
                        .min_size(vec2(68.0, 34.0)),
                );
                hit(ui, &format!("inspector-{}", panel.label()), response.rect);
                if response.clicked() {
                    self.inspector = Some(panel);
                    self.last_inspector = panel;
                    self.history_focus = true;
                }
            }
            let close = ui.add(egui::Button::new("×").min_size(vec2(32.0, 34.0)));
            hit(ui, "inspector-close", close.rect);
            if close.on_hover_text("Hide panels · Tab").clicked() {
                self.inspector = None;
                self.compact_open = false;
            }
        });
        ui.add_space(8.0);
        match self.inspector {
            Some(Inspector::Tools) => self.controls_content(ui),
            Some(Inspector::History) => self.history_panel(ui),
            Some(Inspector::Layers) => self.layers_panel(ui),
            None => {}
        }
    }

    fn history_panel(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Edit history").size(18.0).strong());
        ui.label(
            RichText::new("Click a step to return to it.")
                .size(11.0)
                .color(MUTED),
        );
        let Some(photo) = self.photos.get(self.current) else {
            ui.label("Import a photo to begin.");
            return;
        };
        let (past, future) = photo.history.snapshots();
        let mut states: Vec<&Edit> = past.collect();
        let current = states.len();
        states.push(&photo.edit);
        let mut redo: Vec<&Edit> = future.collect();
        redo.reverse();
        states.extend(redo);
        let mut selected = None;
        ui.add_space(8.0);
        egui::ScrollArea::vertical()
            .id_salt(("history-scroll", photo.id))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for (index, state) in states.iter().enumerate() {
                    let label = if index == 0 {
                        "Starting state".into()
                    } else {
                        history_label(states[index - 1], state)
                    };
                    let text = format!(
                        "{:02}  {}{}",
                        index,
                        label,
                        if index == current { "  •" } else { "" }
                    );
                    let response = ui.add(
                        egui::Button::new(
                            RichText::new(text).size(12.0).color(if index > current {
                                MUTED
                            } else {
                                TEXT
                            }),
                        )
                        .selected(index == current)
                        .min_size(vec2(ui.available_width(), 36.0)),
                    );
                    hit(ui, &format!("history-{index}"), response.rect);
                    if index == current && self.history_focus {
                        response.scroll_to_me(Some(egui::Align::Center));
                    }
                    if response.clicked() {
                        selected = Some(index);
                    }
                }
                ui.add_space(6.0);
                ui.label(
                    RichText::new("Up to 100 undo steps. A new edit replaces the redo branch.")
                        .size(10.0)
                        .color(MUTED),
                );
            });
        self.history_focus = false;
        if let Some(index) = selected {
            self.history_jump(ui.ctx(), index);
        }
    }

    fn end_layer_interaction(&mut self) {
        self.patch_feedback.clear();
        self.patch_draft = PatchDraft::default();
        self.last_stroke = None;
        self.clone_origin = None;
        self.clone_anchor = None;
        self.clone_offset = None;
        self.clone_pick_source = false;
    }

    fn rename_layer_dialog(&mut self, ctx: &egui::Context) {
        let Some((photo, id, mut name)) = self.layer_rename.clone() else {
            return;
        };
        let mut commit = false;
        let mut cancel = false;
        egui::Window::new("Rename layer")
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                let text = ui.add(egui::TextEdit::singleline(&mut name).desired_width(240.0));
                hit(ui, "stack-rename-text", text.rect);
                if !ctx.wants_keyboard_input() {
                    text.request_focus();
                }
                commit |= text.lost_focus() && ctx.input(|i| i.key_pressed(egui::Key::Enter));
                ui.horizontal(|ui| {
                    let save = ui.button("Rename");
                    hit(ui, "stack-rename-save", save.rect);
                    commit |= save.clicked();
                    let response = ui.button("Cancel");
                    hit(ui, "stack-rename-cancel", response.rect);
                    cancel |= response.clicked();
                });
            });
        cancel |= ctx.input(|i| i.key_pressed(egui::Key::Escape));
        if commit {
            if let Some(index) = self.photos.iter().position(|p| p.id == photo) {
                let before = self.photos[index].edit.clone();
                if let Some(layer) = self.photos[index]
                    .edit
                    .stack
                    .layers
                    .iter_mut()
                    .find(|l| l.id == id)
                {
                    layer.name = name.trim().chars().take(80).collect();
                    if layer.name.is_empty() {
                        layer.name = "Layer".into();
                    }
                }
                if before != self.photos[index].edit {
                    self.photos[index].history.record(before);
                    self.photos[index].revision += 1;
                }
            }
            self.layer_rename = None;
        } else if cancel {
            self.layer_rename = None;
        } else {
            self.layer_rename = Some((photo, id, name));
        }
    }

    fn controls_content(&mut self, ui: &mut egui::Ui) {
        let tool = self.tool;
        let ctx = ui.ctx().clone();
        ui.label(RichText::new(tool.title()).size(20.0).strong());
        if let Some(layer) = self
            .photos
            .get(self.current)
            .and_then(|p| p.edit.active_layer())
        {
            ui.label(
                RichText::new(format!("Editing: {}", layer.name))
                    .size(11.0)
                    .color(YELLOW),
            );
        }
        ui.add_space(5.0);
        ui.label(RichText::new(tool.description()).size(11.0).color(MUTED));
        ui.add_space(10.0);
        ui.spacing_mut().scroll.floating = false;
        egui::ScrollArea::vertical()
            .id_salt("tool-scroll")
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if self.photos.is_empty() {
                    ui.label(RichText::new("Import a photo to start editing.").color(MUTED));
                    return;
                }
                let before = self.photos[self.current].edit.clone();
                let locked = self.photos[self.current].edit.layer_locked();
                if locked {
                    ui.label(RichText::new("Selected layer is locked.").color(YELLOW));
                }
                ui.add_enabled_ui(!locked, |ui| match tool {
                    Tool::Portrait => self.portrait_panel(ui),
                    Tool::Skin => self.skin_panel(ui),
                    Tool::Color => self.color_panel(ui),
                    Tool::Background => self.background_panel(ui),
                    Tool::Presets => self.preset_panel(ui),
                    Tool::Batch => self.batch_panel(ui),
                    Tool::Models => self.model_panel(ui),
                });
                if before != self.photos[self.current].edit {
                    if !self.gesture {
                        self.photos[self.current].history.record(before);
                    }
                    self.gesture = ctx.input(|i| i.pointer.any_down());
                    self.edited();
                }
                ui.add_space(12.0);
                ui.separator();
                ui.add_space(8.0);
                ui.label(RichText::new("NATURAL BY DESIGN").size(9.0).color(MUTED));
                ui.label(
                    RichText::new("Your features. Your texture. Your story.")
                        .size(11.0)
                        .color(MUTED),
                );
            });
    }

    fn portrait_panel(&mut self, ui: &mut egui::Ui) {
        let p = &self.photos[self.current];
        ui.label(
            RichText::new(p.note.as_deref().unwrap_or(if p.ai_processing {
                "Detecting faces…"
            } else {
                "Manual tools ready · portrait AI has not run"
            }))
            .size(10.0)
            .color(MUTED),
        );
        ui.add_space(10.0);
        let auto = ui
            .add_sized([ui.available_width(), 42.0], primary("✧  Auto Retouch"))
            .on_hover_text("Clean up facial skin and wrinkles while keeping your color, contrast and shape settings.");
        hit(ui, "auto-retouch", auto.rect);
        if auto.clicked() {
            self.photos[self.current].edit.settings.apply_auto_retouch();
            self.photos[self.current].edit.preset = None;
        }
        ui.add_space(10.0);
        ui.label(
            RichText::new("Local processing · no photo uploads")
                .size(10.0)
                .color(MUTED),
        );
        ui.add_space(24.0);
        section(ui, "MANUAL CLEANUP", |ui| {
            ui.horizontal_wrapped(|ui| {
                for target in [Target::Liquify, Target::Heal, Target::Clone, Target::Patch] {
                    let icon = match target {
                        Target::Liquify => layer_panel::Icon::Liquify,
                        Target::Heal => layer_panel::Icon::Heal,
                        Target::Clone => layer_panel::Icon::Clone,
                        _ => layer_panel::Icon::Patch,
                    };
                    let response = layer_panel::icon_button(
                        ui,
                        icon,
                        self.brush == Some(target),
                        true,
                        &format!("tool-{}", target.label()),
                        target.label(),
                    );
                    if response.clicked() {
                        self.toggle_brush(target);
                    }
                }
            });
        });
        if matches!(
            self.brush,
            Some(Target::Heal | Target::Liquify | Target::Clone | Target::Patch)
        ) {
            self.brush_controls(ui);
        }
        ui.add_space(20.0);
        section(ui, "SKIN", |ui| {
            let settings = &mut self.photos[self.current].edit.settings;
            adjustment(
                ui,
                "Neural skin smoothing",
                settings,
                Adjustment::Smoothing,
                0.0..=100.0,
                "Learned retouching plus texture-aware smoothing. Detected eyes, eyebrows and lips are protected.",
            );
            adjustment(
                ui,
                "Blemish removal",
                settings,
                Adjustment::Blemishes,
                0.0..=100.0,
                "Automatically detect and repair spots, scars, freckles, moles and fine hairs on facial skin. Full strength removes detected marks; reduce it to retain them.",
            );
            adjustment(
                ui,
                "Redness reduction",
                settings,
                Adjustment::Redness,
                0.0..=100.0,
                "Gently reduce excess red in the skin mask.",
            );
            ui.add_space(5.0);
            ui.horizontal(|ui| {
                if small_chip(ui, "Spot heal", self.brush == Some(Target::Heal)).clicked() {
                    self.toggle_brush(Target::Heal);
                }
                if small_chip(ui, "Refine mask", self.brush == Some(Target::Skin)).clicked() {
                    self.toggle_brush(Target::Skin);
                }
            });
        });
        ui.add_space(20.0);
        section(ui, "FLYAWAY HAIR", |ui| {
            adjustment(
                ui,
                "Flyaway cleanup",
                &mut self.photos[self.current].edit.settings,
                Adjustment::FlyawayHairs,
                0.0..=100.0,
                "Repair fine isolated strands outside the detected face. Broad hair locks and facial edges are protected. Textured backgrounds may need Spot heal or Patch.",
            );
            if self.photos[self.current]
                .seg
                .as_ref()
                .is_none_or(|s| s.faces.is_empty())
            {
                ui.label(
                    RichText::new("Detect a face first to locate the hair region.").color(MUTED),
                );
            }
        });
        ui.add_space(20.0);
        section(ui, "FACE DETAILS", |ui| {
            let settings = &mut self.photos[self.current].edit.settings;
            adjustment(
                ui,
                "Under-eye bags",
                settings,
                Adjustment::UnderEyes,
                0.0..=100.0,
                "Automatically target under-eye bags and soften dark creases while retaining texture.",
            );
            adjustment(
                ui,
                "Forehead wrinkles",
                settings,
                Adjustment::Forehead,
                0.0..=100.0,
                "Reduce detected forehead creases while retaining skin texture.",
            );
            adjustment(
                ui,
                "Laugh lines",
                settings,
                Adjustment::LaughLines,
                0.0..=100.0,
                "Reduce nasolabial creases beside the nose and mouth.",
            );
            adjustment(
                ui,
                "Teeth whitening",
                settings,
                Adjustment::Teeth,
                0.0..=100.0,
                "Automatically targets bright, low-chroma pixels inside the detected mouth; refine with the teeth brush.",
            );
            adjustment(
                ui,
                "Eye clarity",
                settings,
                Adjustment::Eyes,
                0.0..=100.0,
                "Automatically target the detected eyes; refine with the eye brush.",
            );
            ui.horizontal_wrapped(|ui| {
                for target in [Target::UnderEyes, Target::Teeth, Target::Eyes] {
                    if small_chip(ui, target.label(), self.brush == Some(target)).clicked() {
                        self.toggle_brush(target);
                    }
                }
            });
            ui.add_space(7.0);
            ui.label(
                RichText::new(
                    "Detected areas are ready automatically. Brushes refine each target.",
                )
                .size(10.0)
                .color(MUTED),
            );
        });
        ui.add_space(20.0);
        egui::CollapsingHeader::new("FACE SHAPE & SCULPTING")
            .default_open(false)
            .show(ui, |ui| {
                let settings = &mut self.photos[self.current].edit.settings;
                adjustment(
                    ui,
                    "Eye size",
                    settings,
                    Adjustment::EyeSize,
                    -100.0..=100.0,
                    "Shrink or enlarge both eyes using the detected face mesh.",
                );
                adjustment(
                    ui,
                    "Nose width",
                    settings,
                    Adjustment::NoseWidth,
                    -100.0..=100.0,
                    "Narrow or widen the nose with a soft mesh-guided deformation.",
                );
                adjustment(
                    ui,
                    "Lip plumpness",
                    settings,
                    Adjustment::LipPlumpness,
                    0.0..=100.0,
                    "Add volume to the detected lips.",
                );
                adjustment(
                    ui,
                    "Jawline",
                    settings,
                    Adjustment::Jawline,
                    0.0..=100.0,
                    "Gently draw the lower cheeks inward for a defined jaw.",
                );
                adjustment(
                    ui,
                    "AI contour",
                    settings,
                    Adjustment::Contour,
                    0.0..=100.0,
                    "Deepen facial structure shadows beneath cheekbones and along the jaw.",
                );
                adjustment(
                    ui,
                    "AI highlight",
                    settings,
                    Adjustment::FaceHighlight,
                    0.0..=100.0,
                    "Brighten cheekbones, the nose bridge and forehead using landmarks.",
                );
            });
        ui.add_space(20.0);
        ui.add_space(20.0);
        section(ui, "FINISH", |ui| {
            adjustment(
                ui,
                "Detail sharpening",
                &mut self.photos[self.current].edit.settings,
                Adjustment::Sharpening,
                0.0..=100.0,
                "Subtle high-pass sharpening in linear light.",
            );
        });
        if !matches!(
            self.brush,
            Some(Target::Heal | Target::Liquify | Target::Clone | Target::Patch)
        ) {
            self.brush_controls(ui);
        }
        ui.add_space(18.0);
        if ui
            .add_sized([ui.available_width(), 32.0], secondary("Reset adjustments"))
            .clicked()
        {
            self.photos[self.current].edit = Edit::default();
            self.brush = None;
            self.patch_draft = PatchDraft::default();
        }
    }

    fn skin_panel(&mut self, ui: &mut egui::Ui) {
        section(ui, "TONE & TEXTURE", |ui| {
            let settings = &mut self.photos[self.current].edit.settings;
            adjustment(
                ui,
                "Tone evenness",
                settings,
                Adjustment::ToneEvenness,
                0.0..=100.0,
                "Balance patchy skin tones without flattening fine texture.",
            );
            adjustment(
                ui,
                "Redness reduction",
                settings,
                Adjustment::Redness,
                0.0..=100.0,
                "Correct only the skin mask.",
            );
            adjustment(
                ui,
                "Neural skin smoothing",
                settings,
                Adjustment::Smoothing,
                0.0..=100.0,
                "Preserve natural pores.",
            );
        });
        ui.add_space(20.0);
        if ui
            .add_sized([ui.available_width(), 36.0], secondary("Refine skin mask"))
            .clicked()
        {
            self.toggle_brush(Target::Skin);
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
            adjustment(
                ui,
                "Exposure · EV",
                s,
                Adjustment::Exposure,
                -2.0..=2.0,
                "Linear-light exposure adjustment in stops.",
            );
            adjustment(
                ui,
                "Contrast",
                s,
                Adjustment::Contrast,
                -100.0..=100.0,
                "Contrast around middle gray.",
            );
            adjustment(
                ui,
                "Highlights",
                s,
                Adjustment::Highlights,
                -100.0..=100.0,
                "Recover or lift bright tones.",
            );
            adjustment(
                ui,
                "Shadows",
                s,
                Adjustment::Shadows,
                -100.0..=100.0,
                "Lift or deepen dark tones.",
            );
        });
        ui.add_space(20.0);
        section(ui, "WHITE BALANCE", |ui| {
            adjustment(
                ui,
                "Temperature",
                s,
                Adjustment::Warmth,
                -100.0..=100.0,
                "Cooler to warmer; relative correction, not Kelvin.",
            );
            adjustment(
                ui,
                "Tint",
                s,
                Adjustment::Tint,
                -100.0..=100.0,
                "Green to magenta balance.",
            );
        });
        ui.add_space(20.0);
        section(ui, "COLOR & FINISH", |ui| {
            adjustment(
                ui,
                "Saturation",
                s,
                Adjustment::Saturation,
                -100.0..=100.0,
                "Linear luminance-preserving saturation.",
            );
            adjustment(
                ui,
                "Vignette",
                s,
                Adjustment::Vignette,
                0.0..=100.0,
                "A gentle edge falloff.",
            );
            adjustment(
                ui,
                "Sharpening",
                s,
                Adjustment::Sharpening,
                0.0..=100.0,
                "Keep the smallest details clear.",
            );
        });
        ui.add_space(20.0);
        self.hsl_panel(ui);
        ui.add_space(20.0);
        self.curves_panel(ui);
        ui.add_space(20.0);
        self.reference_panel(ui);
    }

    fn hsl_panel(&mut self, ui: &mut egui::Ui) {
        section(ui, "SELECTIVE HSL", |ui| {
            egui::ComboBox::from_id_salt("hsl-sector")
                .selected_text(crate::color::HUE_NAMES[self.hsl_band])
                .width(ui.available_width())
                .show_ui(ui, |ui| {
                    for (i, name) in crate::color::HUE_NAMES.iter().enumerate() {
                        ui.selectable_value(&mut self.hsl_band, i, *name);
                    }
                });
            ui.add_space(10.0);
            let s = &mut self.photos[self.current].edit.settings;
            adjustment(
                ui,
                "HSL amount",
                s,
                Adjustment::Hsl,
                0.0..=100.0,
                "Enable or blend the selective hue, saturation and luminance edits.",
            );
            let b = &mut s.color.hsl[self.hsl_band];
            slider(
                ui,
                "Hue shift · degrees",
                &mut b.hue,
                -60.0..=60.0,
                "Shift only the selected color range; neighboring ranges blend smoothly.",
            );
            slider(
                ui,
                "Range saturation",
                &mut b.saturation,
                -100.0..=100.0,
                "Color intensity within the selected hue range.",
            );
            slider(
                ui,
                "Range luminance",
                &mut b.luminance,
                -100.0..=100.0,
                "Lighten or darken the selected hue range.",
            );
            if ui
                .add_sized(
                    [ui.available_width(), 34.0],
                    secondary("Reset selected range"),
                )
                .clicked()
            {
                *b = Default::default();
            }
        });
    }

    fn curves_panel(&mut self, ui: &mut egui::Ui) {
        section(ui, "COLOR CURVES", |ui| {
            ui.horizontal(|ui| {
                for (i, name) in ["Luma", "Red", "Green", "Blue"].iter().enumerate() {
                    let response = ui.selectable_label(self.curve_channel == i, *name);
                    hit(ui, &format!("curve-channel-{i}"), response.rect);
                    if response.clicked() {
                        self.curve_channel = i;
                        self.curve_drag_knot = None;
                    }
                }
            });
            ui.add_space(8.0);
            let s = &mut self.photos[self.current].edit.settings;
            let points = &mut s.color.curves[self.curve_channel];
            curve_editor(ui, points, self.curve_channel, &mut self.curve_drag_knot);
            ui.label(
                RichText::new("Drag a point up or down. Double-click the graph to reset.")
                    .size(11.0)
                    .color(MUTED),
            );
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.add_sized([104.0, 32.0], secondary("Gentle S")).clicked() {
                    *points = [0.0, 0.09, 0.19, 0.33, 0.5, 0.67, 0.81, 0.91, 1.0];
                }
                if ui
                    .add_sized([104.0, 32.0], secondary("Reset curve"))
                    .clicked()
                {
                    *points = crate::color::IDENTITY_CURVE;
                }
            });
            ui.add_space(8.0);
            adjustment(
                ui,
                "Curves amount",
                s,
                Adjustment::Curves,
                0.0..=100.0,
                "Blend luminance and RGB curves; disabling keeps the control points.",
            );
        });
    }

    fn reference_panel(&mut self, ui: &mut egui::Ui) {
        section(ui, "REFERENCE COLOR MATCH", |ui| {
            let photo_id = self.photos[self.current].id;
            let busy = self.pending_reference.is_some_and(|(id, _)| id == photo_id);
            let name = self.photos[self.current]
                .edit
                .settings
                .color
                .reference
                .as_ref()
                .map(|p| p.name.clone());
            if let Some(name) = name {
                ui.label(RichText::new(name).size(12.0).color(TEXT));
            } else {
                ui.label(
                    RichText::new("Choose a photo with the color and light you want.")
                        .size(11.0)
                        .color(MUTED),
                );
            }
            if ui
                .add_enabled(
                    !busy,
                    secondary(if busy {
                        "Reading reference…"
                    } else {
                        "Choose reference photo…"
                    })
                    .min_size(vec2(ui.available_width(), 36.0)),
                )
                .clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .add_filter(
                        "Reference photo",
                        &["jpg", "jpeg", "png", "webp", "tif", "tiff", "bmp"],
                    )
                    .pick_file()
            {
                self.reference_serial += 1;
                let serial = self.reference_serial;
                self.pending_reference = Some((photo_id, serial));
                let out = self.reference_sender.clone();
                let ctx = ui.ctx().clone();
                std::thread::spawn(move || {
                    let result = crate::color::load_reference(&path);
                    let _ = out.send((photo_id, serial, result));
                    ctx.request_repaint();
                });
            }
            ui.add_space(10.0);
            let s = &mut self.photos[self.current].edit.settings;
            adjustment(
                ui,
                "Reference match amount",
                s,
                Adjustment::ReferenceMatch,
                0.0..=100.0,
                "Match lightness distribution and color balance to the reference. Each photo is measured independently.",
            );
            if ui
                .add_sized([ui.available_width(), 34.0], secondary("Clear reference"))
                .clicked()
            {
                s.color.reference = None;
                s.color.reference_strength = 0.0;
                self.pending_reference = None;
            }
            ui.add_space(8.0);
            let selected = self
                .photos
                .iter()
                .enumerate()
                .any(|(i, p)| i != self.current && p.selected);
            let has_reference = self.photos[self.current]
                .edit
                .settings
                .color
                .reference
                .is_some();
            if ui
                .add_enabled(
                    has_reference && selected,
                    primary("Match selected to reference")
                        .min_size(vec2(ui.available_width(), 36.0)),
                )
                .clicked()
            {
                self.sync_reference();
            }
            ui.add_space(8.0);
            let preview_ready =
                self.photos[self.current].revision == self.photos[self.current].rendered_revision;
            if ui
                .add_enabled(
                    selected && preview_ready,
                    secondary("Use current look for selected")
                        .min_size(vec2(ui.available_width(), 36.0)),
                )
                .clicked()
            {
                let p = &self.photos[self.current];
                match crate::color::ReferenceProfile::from_image(p.photo.name.clone(), &p.edited) {
                    Ok(profile) => self.match_selected_reference(profile, 65.0, true),
                    Err(e) => self.notify(format!("Could not use reference: {e}")),
                }
            }
            ui.label(RichText::new("Matching changes color and light. Use similar scenes for close matches. The reference travels with your presets and session.")
                .size(11.0).color(MUTED));
        });
    }

    fn poll_color_reference(&mut self, ctx: &egui::Context) {
        while let Ok((id, serial, result)) = self.reference_results.try_recv() {
            if self.pending_reference != Some((id, serial)) {
                continue;
            }
            self.pending_reference = None;
            match result {
                Ok(profile) => {
                    if let Some(photo) = self.photos.iter_mut().find(|p| p.id == id) {
                        photo.history.record(photo.edit.clone());
                        photo.edit.settings.color.reference = Some(profile);
                        photo.edit.settings.color.reference_strength = 65.0;
                        photo
                            .edit
                            .settings
                            .disabled
                            .retain(|a| *a != Adjustment::ReferenceMatch);
                        photo.revision += 1;
                        self.last_edit = Instant::now();
                        self.notify("Reference ready · adjust its amount or match selected photos");
                        ctx.request_repaint();
                    }
                }
                Err(e) => self.notify(format!("Could not read reference: {e}")),
            }
        }
    }

    fn sync_reference(&mut self) {
        let s = &self.photos[self.current].edit.settings;
        if let Some(profile) = s.color.reference.clone() {
            let disabled = s.disabled.contains(&Adjustment::ReferenceMatch);
            self.match_selected_reference(profile, s.color.reference_strength, !disabled);
        }
    }

    fn match_selected_reference(
        &mut self,
        reference: crate::color::ReferenceProfile,
        strength: f32,
        enabled: bool,
    ) {
        let mut matched = 0;
        for (i, photo) in self.photos.iter_mut().enumerate() {
            if i == self.current || !photo.selected {
                continue;
            }
            let before = photo.edit.clone();
            photo.edit.settings.color.reference = Some(reference.clone());
            photo.edit.settings.color.reference_strength = strength;
            photo
                .edit
                .settings
                .disabled
                .retain(|a| *a != Adjustment::ReferenceMatch);
            if !enabled {
                photo
                    .edit
                    .settings
                    .disabled
                    .push(Adjustment::ReferenceMatch);
            }
            if photo.edit != before {
                photo.history.record(before);
                photo.revision += 1;
                matched += 1;
            }
        }
        self.last_edit = Instant::now();
        self.notify(format!(
            "Reference color matched to {matched} selected photos"
        ));
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
            adjustment(
                ui,
                "Blur strength",
                s,
                Adjustment::BackgroundBlur,
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
            self.patch_draft = PatchDraft::default();
        }
        self.brush_controls(ui);
    }

    fn preset_panel(&mut self, ui: &mut egui::Ui) {
        let p = &self.photos[self.current];
        let key = (p.id, p.revision, self.presets.len());
        if self.preset_key != Some(key) {
            self.preset_key = Some(key);
            self.preset_thumbs = vec![None; self.presets.len()];
            let _ = self.tx.send(Job::Presets {
                id: p.id,
                revision: p.revision,
                image: p.photo.preview.clone(),
                edit: p.edit.clone(),
                seg: p.seg.clone(),
                presets: self.presets.clone(),
            });
        }
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
            if let Some(Some(tex)) = self.preset_thumbs.get(index) {
                ui.painter().image(
                    tex.id(),
                    thumb,
                    Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            } else {
                ui.painter().text(
                    thumb.center(),
                    Align2::CENTER_CENTER,
                    "…",
                    FontId::proportional(18.0),
                    MUTED,
                );
            }
            hit(ui, &format!("preset-{index}"), rect);
            if response.hovered() {
                self.hover_preset = Some(index);
            }
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
                .add_filter("Hastur preset", &["ron"])
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
                .add_filter("Hastur preset", &["ron"])
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
                self.photos
                    .iter()
                    .enumerate()
                    .any(|(i, p)| i != self.current && p.selected),
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
                    ui.label(
                        RichText::new(if p.revision == p.rendered_revision {
                            "Done"
                        } else if p.processing == Some(p.revision) {
                            "Processing"
                        } else {
                            "Queued"
                        })
                        .size(9.0)
                        .color(YELLOW),
                    );
                });
                if let Some(note) = &p.note {
                    ui.label(RichText::new(note).size(10.0).color(YELLOW));
                }
                ui.add_space(7.0);
            }
        });
    }

    fn model_panel(&mut self, ui: &mut egui::Ui) {
        let p = &self.photos[self.current];
        ui.label(
            RichText::new(
                p.note
                    .as_deref()
                    .unwrap_or("Portrait AI is dormant until a tool needs it"),
            )
            .size(11.0)
            .color(MUTED),
        );
        ui.add_space(10.0);
        if ui
            .add_enabled(
                model::AVAILABLE && !p.ai_processing,
                primary("Reanalyze portrait"),
            )
            .clicked()
        {
            self.request_portrait(self.current, PortraitDemand::ALL, true);
        }
        ui.add_space(16.0);
        ui.label(
            RichText::new(if model::AVAILABLE {
                "ONNX support enabled"
            } else {
                "ONNX support disabled"
            })
            .size(14.0),
        );
        ui.add_space(10.0);
        ui.label(
            RichText::new(
                "NullState · AI runs when an enabled tool needs it. Manual and color tools are ready without portrait analysis. Everything runs on your device.",
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
                #[allow(unused_mut)]
                let mut providers = vec![
                    Provider::Auto,
                    Provider::Cpu,
                    Provider::DirectMl,
                    Provider::Cuda,
                    Provider::CoreMl,
                ];
                #[cfg(astra_burn_models)]
                providers.insert(2, Provider::Burn);
                for p in providers {
                    ui.selectable_value(&mut self.provider, p, p.label());
                }
            });
        if let Some(status) = self
            .photos
            .get(self.current)
            .and_then(|p| p.ai_status.as_ref())
        {
            ui.add_space(10.0);
            ui.label(RichText::new(&status.stage).color(TEXT));
            ui.label(RichText::new(&status.detail).size(11.0).color(MUTED));
        }
        if self.provider == Provider::Burn {
            ui.label(
                RichText::new(
                    "Burn runs bundled portrait models only. Its CPU backend can be much slower and use more memory than ONNX Runtime.",
                )
                .size(11.0)
                .color(MUTED),
            );
        } else if self.provider == Provider::Auto {
            ui.label(RichText::new("Auto qualifies GPU execution for bundled portrait models. Additional imported mask models use CPU; choose an explicit provider to run them on a GPU.").size(11.0).color(MUTED));
        }
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
                "Pretrained neural skin retouching",
                "478-point mesh and face reshaping",
                "Automatic blemish detection and repair",
                "Targeted wrinkle, bag and sculpting controls",
                "Manual liquify, texture healing, clone and patch",
                "Linear-light color and tone",
                "Optional additional segmentation models",
                "Versioned JPG / PNG export",
            ] {
                ui.label(RichText::new(format!("·  {text}")).size(11.0).color(MUTED));
                ui.add_space(6.0);
            }
            ui.add_space(10.0);
            ui.label(RichText::new("Automatic cleanup targets marks and hairs on facial skin. Use healing or clone for hair and clothing outside the detected face.").size(11.0).color(MUTED));
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
                            self.patch_draft = PatchDraft::default();
                        }
                    });
                    if target == Target::Patch {
                        let mut softness = self.brush_softness * 100.0;
                        slider(
                            ui,
                            "Selection feather",
                            &mut softness,
                            0.0..=100.0,
                            "Softens the patch boundary to blend it into the surrounding texture.",
                        );
                        self.brush_softness = softness / 100.0;
                        slider(
                            ui,
                            "Patch strength",
                            &mut self.brush_strength,
                            0.0..=100.0,
                            "How strongly the sampled clean texture replaces the selected area.",
                        );
                    } else {
                        let mut radius = self.brush_radius * 1000.0;
                        slider(
                            ui,
                            "Brush size",
                            &mut radius,
                            3.0..=150.0,
                            "Radius relative to the image’s short edge.",
                        );
                        self.brush_radius = radius / 1000.0;
                        let mut softness = self.brush_softness * 100.0;
                        slider(
                            ui,
                            "Brush softness",
                            &mut softness,
                            0.0..=100.0,
                            "0 has a hard edge; 100 feathers across the whole radius.",
                        );
                        self.brush_softness = softness / 100.0;
                    }
                    if matches!(target, Target::Heal | Target::Liquify | Target::Clone | Target::LayerMask) {
                        slider(
                            ui,
                            "Brush strength",
                            &mut self.brush_strength,
                            0.0..=100.0,
                            "Strength of new strokes. Existing strokes keep their saved strength.",
                        );
                    }
                    if target != Target::Patch {
                        ui.horizontal(|ui| {
                            ui.label("Diameter");
                            let short = self.photos[self.current].photo.dimensions().0.min(self.photos[self.current].photo.dimensions().1) as f32;
                            let mut pixels = self.brush_radius * short * 2.;
                            let value = ui.add(egui::DragValue::new(&mut pixels).range(2.0..=short*0.3).speed(1.).suffix(" px"));
                            hit(ui,"brush-diameter",value.rect);
                            if value.changed() { self.brush_radius = pixels / (short*2.); }
                        });
                    }
                    if target == Target::Clone {
                        let aligned = ui.checkbox(&mut self.clone_aligned, "Aligned source across strokes");
                        hit(ui,"clone-aligned",aligned.rect);
                        if aligned.changed() { self.clone_offset = None; self.end_canvas_gesture(); }
                    }
                    if target == Target::Liquify && !self.photos[self.current].edit.warps.is_empty() {
                        let reset = ui.button("Reset liquify"); hit(ui,"liquify-reset",reset.rect);
                        if reset.clicked() { self.photos[self.current].edit.warps.clear(); }
                    }
                    if target == Target::LayerMask {
                        ui.checkbox(&mut self.show_mask, "Show hidden area");
                        ui.checkbox(&mut self.erase, "Reveal instead of hide");
                    } else if !matches!(
                        target,
                        Target::Heal | Target::Liquify | Target::Clone | Target::Patch
                    ) {
                        ui.checkbox(&mut self.show_mask, "Show mask overlay");
                        ui.checkbox(&mut self.erase, "Erase mask");
                    }
                    ui.label(
                        RichText::new(if target == Target::Patch {
                            if self.patch_draft.drawing {
                                "Trace around the area to repair. Release, then drag the selection onto clean texture."
                            } else if !self.patch_draft.boundary.is_empty() {
                                "Drag inside the selection onto clean texture. Draw a new outline to replace it."
                            } else {
                                "Draw around an area to repair, then drag the selection onto clean texture."
                            }
                        } else if target == Target::Clone {
                            if self
                                .clone_anchor
                                .is_some_and(|(id, _)| id == self.photos[self.current].id)
                            {
                                "Source selected. Paint to clone. Alt + click chooses a new source."
                            } else {
                                "Alt + click to select a source, then paint to clone."
                            }
                        } else if target == Target::LayerMask {
                            "Paint black to hide this layer, or enable Reveal to restore it. Space + drag to pan."
                        } else if target == Target::Liquify {
                            "Drag to push or pull. Space + drag to pan."
                        } else {
                            "Paint on the photo. Space + drag to pan."
                        })
                        .size(10.0)
                        .color(MUTED),
                    );
                });
        }
    }

    fn filmstrip(&mut self, ctx: &egui::Context) {
        if !self.filmstrip_visible {
            return;
        }
        let photo_focus = self.photo_focus;
        self.photo_focus = false;
        egui::TopBottomPanel::bottom("filmstrip")
            .exact_height(128.0)
            .frame(
                egui::Frame::new()
                    .fill(PANEL)
                    .inner_margin(egui::Margin::symmetric(12, 6))
                    .stroke(Line::new(1.0_f32, BORDER)),
            )
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
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
                            RichText::new("Ctrl / Shift + click to select")
                                .size(10.0)
                                .color(MUTED),
                        );
                    });
                });
                ui.add_space(4.0);
                egui::ScrollArea::horizontal()
                    .id_salt("filmstrip-scroll")
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let mut select = None;
                            let mut delete = None;
                            for (index, photo) in self.photos.iter_mut().enumerate() {
                                let (rect, response) =
                                    ui.allocate_exact_size(vec2(64.0, 68.0), Sense::click());
                                let image_bounds = Rect::from_min_size(
                                    rect.min + vec2(3.0, 3.0),
                                    vec2(58.0, 50.0),
                                );
                                let dimensions = photo.edited_texture.size_vec2();
                                let fit = (image_bounds.width() / dimensions.x)
                                    .min(image_bounds.height() / dimensions.y);
                                let image_rect =
                                    Rect::from_center_size(image_bounds.center(), dimensions * fit);
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
                                if photo.rendered_revision == photo.revision {
                                    ui.painter().text(
                                        image_rect.left_bottom() + vec2(4.0, -7.0),
                                        Align2::LEFT_CENTER,
                                        "✓",
                                        FontId::proportional(12.0),
                                        YELLOW,
                                    );
                                }
                                if photo.photo.resident && photo.rendered_revision != photo.revision
                                {
                                    ui.painter().text(
                                        image_rect.left_bottom() + vec2(4.0, -7.0),
                                        Align2::LEFT_CENTER,
                                        if photo.processing == Some(photo.revision) {
                                            "Working"
                                        } else {
                                            "Queued"
                                        },
                                        FontId::proportional(9.0),
                                        YELLOW,
                                    );
                                }
                                ui.painter().text(
                                    rect.center_bottom() + vec2(0.0, -7.0),
                                    Align2::CENTER_CENTER,
                                    format!(
                                        "{:02}{}",
                                        index + 1,
                                        if photo.rating > 0 {
                                            format!("  ★ {}", photo.rating)
                                        } else {
                                            String::new()
                                        }
                                    ),
                                    FontId::proportional(9.0),
                                    MUTED,
                                );
                                hit(ui, &format!("photo-{index}"), rect);
                                if index == self.current && photo_focus {
                                    response.scroll_to_me(Some(egui::Align::Center));
                                }
                                if response.clicked() {
                                    select = Some(index);
                                }
                                response.context_menu(|ui| {
                                    if ui.button("Delete from Project").clicked() {
                                        delete = Some(photo.id);
                                        ui.close();
                                    }
                                    ui.label(
                                        RichText::new("Source file stays on disk")
                                            .size(11.)
                                            .color(MUTED),
                                    );
                                });
                                response.on_hover_text(format!(
                                    "{}\n{} × {}\nRating: {} / 5 · press 1–5, Shift+0 to clear\n{}",
                                    photo.photo.name,
                                    photo.photo.dimensions().0,
                                    photo.photo.dimensions().1,
                                    photo.rating,
                                    photo.note.as_deref().unwrap_or("Original preserved")
                                ));
                                ui.add_space(5.0);
                            }
                            if let Some(index) = select {
                                let (shift, control) = ctx.input(|i| {
                                    (i.modifiers.shift, i.modifiers.ctrl || i.modifiers.command)
                                });
                                self.select_photo(index, shift, control);
                            }
                            if let Some(id) = delete {
                                self.delete_from_project(id);
                            }
                            let (r, response) =
                                ui.allocate_exact_size(vec2(64.0, 68.0), Sense::click());
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
                    .inner_margin(egui::Margin::symmetric(12, 10)),
            )
            .show(ctx, |ui| {
                hit(ui, "canvas-content", ui.max_rect());
                ui.horizontal(|ui| {
                    if self.brush.is_some_and(manual_tools::is_manual) && !self.photos.is_empty() {
                        ui.allocate_ui_with_layout(vec2((ui.available_width()-130.).max(160.),34.), egui::Layout::left_to_right(egui::Align::Center), |ui| self.manual_workspace_bar(ui));
                    } else {
                        ui.label(RichText::new("PORTRAIT WORKSPACE").size(10.0).color(MUTED));
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if small_chip(ui, "Before / After", self.compare).clicked() {
                            self.compare = !self.compare;
                        }
                        if !self.brush.is_some_and(manual_tools::is_manual) && self
                            .photos
                            .get(self.current)
                            .is_some_and(|p| p.photo.path.is_none())
                        {
                            ui.label(RichText::new("GENERATED DEMO").size(9.0).color(MUTED));
                        }
                    });
                });
                ui.add_space(6.0);
                let (viewport, _) = ui.allocate_exact_size(
                    vec2(
                        ui.available_width(),
                        (ui.available_height() - 46.0).max(1.0),
                    ),
                    Sense::hover(),
                );
                hit(ui, "canvas", viewport);
                self.canvas_viewport = Some(viewport);
                if self.photos.is_empty() {
                    ui.painter().text(
                        viewport.center(),
                        Align2::CENTER_CENTER,
                        if self.importing {
                            "Preparing your studio…"
                        } else {
                            "Drop photos here or use Import"
                        },
                        FontId::proportional(24.0),
                        TEXT,
                    );
                    return;
                }
                let dpi = ctx.pixels_per_point();
                let dimensions = vec2(
                    self.photos[self.current].photo.dimensions().0 as f32,
                    self.photos[self.current].photo.dimensions().1 as f32,
                );
                let mut rect = self.photos[self.current]
                    .view
                    .layout(viewport, dimensions, dpi);
                if !self.photos[self.current].photo.resident {
                    let photo = &self.photos[self.current];
                    let (w,h)=photo.photo.dimensions();
                    let rect=Rect::from_center_size(viewport.center(),vec2(w as f32,h as f32)*
                        (viewport.width()/w as f32).min(viewport.height()/h as f32));
                    ui.painter().image(photo.edited_texture.id(),rect,Rect::from_min_max(Pos2::ZERO,pos2(1.,1.)),Color32::WHITE);
                    if self.asset_error==Some(photo.id) {
                        ui.scope_builder(egui::UiBuilder::new().max_rect(Rect::from_center_size(viewport.center(),vec2(230.,40.))),|ui|{
                            if ui.button("Retry opening original").clicked() {self.asset_error=None;}
                        });
                    } else {
                        ui.painter().text(viewport.center_bottom()-vec2(0.,24.),Align2::CENTER_BOTTOM,"Opening original for editing…",FontId::proportional(14.),YELLOW);
                    }
                    return;
                }
                let response = ui.interact(
                    viewport,
                    ui.id().with("photo-canvas"),
                    Sense::click_and_drag(),
                );
                let held = ctx.input(|i| i.key_down(egui::Key::Backslash));
                let space = ctx.input(|i| i.key_down(egui::Key::Space));
                let pointer = ctx.input(|i| i.pointer.interact_pos());
                // egui clears press_origin when a complete click arrives in one
                // frame. Preserve the press event itself for exact registration.
                let press_origin = ctx.input(|i| i.pointer.press_origin().or_else(||
                    i.events.iter().rev().find_map(|event| match event {
                        egui::Event::PointerButton {pos,button:egui::PointerButton::Primary,pressed:true,..} => Some(*pos),
                        _ => None,
                    })));
                if space
                    && response.dragged()
                    && ctx.input(|i| i.pointer.primary_down())
                    && self.canvas_gesture == Gesture::Brush
                {
                    self.canvas_gesture = Gesture::Pan;
                    self.last_stroke = None;
                    self.clone_origin = None;
                    if self.brush == Some(Target::Patch) {
                        self.patch_draft.drawing = false;
                        self.patch_draft.source_drag = None;
                    }
                }
                if response.hovered() || response.dragged() {
                    if space && self.canvas_gesture != Gesture::Divider {
                        ctx.set_cursor_icon(if ctx.input(|i| i.pointer.primary_down()) {
                            egui::CursorIcon::Grabbing
                        } else {
                            egui::CursorIcon::Grab
                        });
                    } else if !space
                        && self.brush.is_some()
                        && pointer.is_some_and(|pos| rect.contains(pos) && viewport.contains(pos))
                    {
                        ctx.set_cursor_icon(if self.manual_blocked() { egui::CursorIcon::NotAllowed } else { egui::CursorIcon::Crosshair });
                    }
                }
                if response.contains_pointer()
                    && ctx.input(|i| i.pointer.primary_pressed())
                    && let Some(pos) = press_origin.or(pointer)
                {
                    self.canvas_gesture = interaction::gesture_at(
                        pos,
                        rect,
                        (self.compare && !held).then_some(self.split),
                        self.brush.is_some(),
                        space,
                    );
                    if self.canvas_gesture == Gesture::Brush && self.brush == Some(Target::Clone)
                        && (self.clone_pick_source || ctx.input(|i| i.modifiers.alt)) {
                        self.canvas_gesture = Gesture::Source;
                    }
                    if self.canvas_gesture == Gesture::Brush && (held || (self.compare && pos.x < rect.left()+rect.width()*self.split)) {
                        self.notify("You are viewing the original. Paint on the edited side, or press B to leave comparison.");
                        self.end_canvas_gesture();
                    }
                    if self.canvas_gesture == Gesture::Brush && self.brush == Some(Target::Liquify)
                        && let Some(start) = press_origin.filter(|p| rect.contains(*p) && viewport.contains(*p)) {
                        self.last_stroke = Some([(start.x-rect.left())/rect.width(),(start.y-rect.top())/rect.height()]);
                    }
                    if self.brush == Some(Target::Patch) && self.canvas_gesture == Gesture::Brush {
                        let uv = [
                            (pos.x - rect.left()) / rect.width(),
                            (pos.y - rect.top()) / rect.height(),
                        ];
                        if !self.patch_draft.drawing
                            && self.patch_draft.boundary.len() >= 3
                            && point_in_polygon_uv(uv, &self.patch_draft.boundary)
                        {
                            self.patch_draft.source_drag = Some((uv, uv));
                        } else {
                            self.patch_draft.boundary.clear();
                            self.patch_draft.boundary.push(uv);
                            self.patch_draft.drawing = true;
                            self.patch_draft.source_drag = None;
                        }
                    }
                }
                if response.double_clicked()
                    && (self.brush.is_none() || space)
                    && self.canvas_gesture != Gesture::Divider
                {
                    let view = &mut self.photos[self.current].view;
                    if view.fit {
                        view.zoom_at(1.0, pointer.unwrap_or(viewport.center()), viewport);
                    } else {
                        *view = View::default();
                    }
                }
                if response.hovered() && self.canvas_gesture == Gesture::None {
                    let scroll = ctx.input(|i| i.smooth_scroll_delta.y);
                    if scroll.abs() > 0.01 {
                        let view = &mut self.photos[self.current].view;
                        let scroll = scroll
                            * if self.preferences.invert_scroll {
                                -1.0
                            } else {
                                1.0
                            };
                        if self.preferences.wheel_zoom {
                            view.zoom_at(
                                view.scale * (scroll * 0.003 * self.preferences.zoom_speed).exp(),
                                pointer.unwrap_or(viewport.center()),
                                viewport,
                            );
                        } else {
                            view.fit = false;
                            view.pan[1] += scroll;
                        }
                    }
                }
                if matches!(self.canvas_gesture, Gesture::Brush | Gesture::Source)
                    && (self.brush.is_none() || self.photos[self.current].edit.layer_locked() || self.manual_blocked())
                {
                    self.reject_manual_stroke();
                }
                if ctx.input(|i| i.pointer.primary_down() || i.pointer.primary_pressed()) {
                    match self.canvas_gesture {
                        Gesture::Source => {
                            if ctx.input(|i| i.pointer.primary_pressed()) && let Some(pos) = press_origin.or(pointer) {
                                let uv = [(pos.x-rect.left())/rect.width(), (pos.y-rect.top())/rect.height()];
                                let p = &self.photos[self.current];
                                let source = if held || (self.compare && pos.x<rect.left()+rect.width()*self.split) {uv} else {crate::geometry::source_uv(uv, &p.edit, p.seg.as_deref(), p.photo.dimensions())};
                                self.clone_anchor = Some((p.id,source)); self.clone_offset = None;
                                self.clone_origin = None; self.last_stroke = None; self.clone_pick_source = false;
                                self.notify("Clone source selected. Paint to copy texture.");
                            }
                        }
                        Gesture::Pan => {
                            let delta = ctx.input(|i| i.pointer.delta());
                            let view = &mut self.photos[self.current].view;
                            if response.dragged() {
                                view.fit = false;
                                view.pan[0] += delta.x;
                                view.pan[1] += delta.y;
                            }
                        }
                        Gesture::Divider => {
                            if let Some(pos) = pointer {
                                self.split =
                                    ((pos.x - rect.left()) / rect.width()).clamp(0.01, 0.99);
                            }
                        }
                        Gesture::Brush => {
                            if let Some(pos) = pointer {
                                if rect.contains(pos) && viewport.contains(pos) {
                                    let uv = [
                                        (pos.x - rect.left()) / rect.width(),
                                        (pos.y - rect.top()) / rect.height(),
                                    ];
                                    let target = self.brush.unwrap();
                                    let dims = (dimensions.x as u32, dimensions.y as u32);
                                    let alt = ctx.input(|i| i.modifiers.alt);
                                    if target == Target::LayerMask {
                                        let first = self.last_stroke.is_none() && ctx.input(|i| i.pointer.primary_pressed());
                                        let start = press_origin.filter(|point| rect.contains(*point) && viewport.contains(*point))
                                            .map(|point| [(point.x - rect.left()) / rect.width(), (point.y - rect.top()) / rect.height()]).unwrap_or(uv);
                                        let stamps = if first {
                                            let mut points = vec![start];
                                            points.extend(interaction::stamps(Some(start), uv, [dims.0, dims.1], self.brush_radius));
                                            points
                                        } else {
                                            interaction::stamps(self.last_stroke, uv, [dims.0, dims.1], self.brush_radius)
                                        };
                                        if !stamps.is_empty() {
                                            let p = &mut self.photos[self.current];
                                            if !self.gesture {
                                                p.history.record(p.edit.clone());
                                                self.gesture = true;
                                            }
                                            if let Some(mask) = p.edit.active_layer_mut().and_then(|layer| layer.mask.as_mut()) {
                                                for center in stamps {
                                                    mask.strokes.push(crate::layer_mask::MaskStroke {
                                                        center,
                                                        radius: self.brush_radius,
                                                        softness: self.brush_softness,
                                                        strength: self.brush_strength,
                                                        reveal: self.erase,
                                                    });
                                                }
                                                self.last_stroke = Some(uv);
                                                self.edited();
                                            }
                                        }
                                    } else if target == Target::Patch {
                                        if self.patch_draft.drawing {
                                            if let Some(last) =
                                                self.patch_draft.boundary.last().copied()
                                            {
                                                let distance = ((uv[0] - last[0]) * rect.width())
                                                    .hypot((uv[1] - last[1]) * rect.height());
                                                if distance >= 4.0
                                                    && self.patch_draft.boundary.len() < 256
                                                {
                                                    self.patch_draft.boundary.push(uv);
                                                }
                                            }
                                        } else if let Some((start, _)) =
                                            self.patch_draft.source_drag
                                        {
                                            self.patch_draft.source_drag = Some((start, uv));
                                        }
                                    } else if target == Target::Clone && alt {
                                        if ctx.input(|i| i.pointer.primary_pressed()) {
                                            let p = &self.photos[self.current];
                                            let source = crate::geometry::source_uv(
                                                uv,
                                                &p.edit,
                                                p.seg.as_deref(),
                                                dims,
                                            );
                                            self.clone_anchor = Some((p.id, source));
                                            self.clone_origin = None;
                                            self.last_stroke = None;
                                        }
                                    } else if target == Target::Liquify {
                                        if let Some(previous) = self.last_stroke {
                                            let stamps = interaction::stamps(
                                                Some(previous),
                                                uv,
                                                [dims.0, dims.1],
                                                self.brush_radius,
                                            );
                                            let delta = [uv[0] - previous[0], uv[1] - previous[1]];
                                            if delta[0].abs() + delta[1].abs() > 0.000001
                                                && !stamps.is_empty()
                                            {
                                                let p = &mut self.photos[self.current];
                                                if !self.gesture {
                                                    p.history.record(p.edit.clone());
                                                    self.gesture = true;
                                                }
                                                let count = stamps.len() as f32;
                                                for center in stamps {
                                                    p.edit.warps.push(
                                                        crate::geometry::WarpStroke {
                                                            center,
                                                            delta: [
                                                                delta[0] / count,
                                                                delta[1] / count,
                                                            ],
                                                            radius: self.brush_radius,
                                                            softness: self.brush_softness,
                                                            strength: self.brush_strength,
                                                        },
                                                    );
                                                }
                                                self.edited();
                                                self.last_stroke = Some(uv);
                                            }
                                        } else {
                                            self.last_stroke = Some(uv);
                                        }
                                    } else if target != Target::Clone
                                        || self.clone_anchor.is_some_and(|(id, _)| {
                                            id == self.photos[self.current].id
                                        })
                                    {
                                        let first = self.last_stroke.is_none() && ctx.input(|i| i.pointer.primary_pressed());
                                        let start = press_origin.filter(|p| rect.contains(*p) && viewport.contains(*p))
                                            .map(|p|[(p.x-rect.left())/rect.width(),(p.y-rect.top())/rect.height()]).unwrap_or(uv);
                                        let stamps = if first {
                                            let mut points = vec![start];
                                            points.extend(interaction::stamps(Some(start),uv,[dims.0,dims.1],self.brush_radius));
                                            points
                                        } else {interaction::stamps(self.last_stroke,uv,[dims.0,dims.1],self.brush_radius)};
                                        if !stamps.is_empty() {
                                            let p = &mut self.photos[self.current];
                                            if !self.gesture {
                                                p.history.record(p.edit.clone());
                                                self.gesture = true;
                                            }
                                            let mapping = crate::geometry::Mapping::new(
                                                &p.edit,
                                                p.seg.as_deref(),
                                                dims,
                                            );
                                            let origin = mapping.source(if first {start} else {uv});
                                            let centers: Vec<_> = stamps
                                                .into_iter()
                                                .map(|center| mapping.source(center))
                                                .collect();
                                            drop(mapping);
                                            if target == Target::Clone
                                                && self.clone_origin.is_none()
                                            {
                                                self.clone_origin = Some(origin);
                                                if !self.clone_aligned || self.clone_offset.is_none_or(|(id,_)| id != p.id) {
                                                    let anchor = self.clone_anchor.unwrap().1;
                                                    self.clone_offset = Some((p.id,[anchor[0]-origin[0], anchor[1]-origin[1]]));
                                                }
                                            }
                                            for center in centers {
                                                if target == Target::Clone {
                                                    let offset = self.clone_offset.unwrap().1;
                                                    p.edit.clones.push(
                                                        crate::cleanup::CloneStamp {
                                                            center,
                                                            source: [
                                                                center[0] + offset[0],
                                                                center[1] + offset[1],
                                                            ],
                                                            radius: self.brush_radius,
                                                            softness: self.brush_softness,
                                                            strength: self.brush_strength,
                                                        },
                                                    );
                                                } else {
                                                    p.edit.strokes.push(Stroke {
                                                        target,
                                                        center,
                                                        radius: self.brush_radius,
                                                        softness: self.brush_softness,
                                                        erase: self.erase && target != Target::Heal,
                                                        strength: if target == Target::Heal { self.brush_strength } else { 100.0 },
                                                    });
                                                }
                                            }
                                            self.last_stroke = Some(uv);
                                            self.edited();
                                        }
                                    } else if ctx.input(|i| i.pointer.primary_pressed()) {
                                        self.notify("Choose a clone source first: click Choose source, or Alt+click a clean area.");
                                        self.end_canvas_gesture();
                                    }
                                } else {
                                    self.last_stroke = None;
                                    self.clone_origin = None;
                                }
                            }
                        }
                        Gesture::None => {}
                    }
                }
                if self.brush == Some(Target::Patch) && self.canvas_gesture == Gesture::Brush && ctx.input(|i| i.pointer.primary_released())
                {
                    let release_uv = pointer
                        .filter(|pos| rect.contains(*pos) && viewport.contains(*pos))
                        .map(|pos| {
                            [
                                (pos.x - rect.left()) / rect.width(),
                                (pos.y - rect.top()) / rect.height(),
                            ]
                        });
                    if self.patch_draft.drawing {
                        if let Some(uv) = release_uv
                            && let Some(last) = self.patch_draft.boundary.last().copied()
                            && ((uv[0] - last[0]) * rect.width())
                                .hypot((uv[1] - last[1]) * rect.height())
                                >= 2.0
                        {
                            self.patch_draft.boundary.push(uv);
                        }
                        self.patch_draft.drawing = false;
                        if polygon_area_uv(&self.patch_draft.boundary) < 0.000005 {
                            self.patch_draft = PatchDraft::default();
                            self.notify("Draw a larger closed outline around the area to repair.");
                        }
                    } else if let Some((start, previous_end)) = self.patch_draft.source_drag.take()
                    {
                        let end = release_uv.unwrap_or(previous_end);
                        let distance = ((end[0] - start[0]) * rect.width())
                            .hypot((end[1] - start[1]) * rect.height());
                        if distance >= 3.0 && self.patch_draft.boundary.len() >= 3 {
                            let p = &mut self.photos[self.current];
                            let dims = (p.photo.dimensions().0, p.photo.dimensions().1);
                            let mapping =
                                crate::geometry::Mapping::new(&p.edit, p.seg.as_deref(), dims);
                            let boundary = self
                                .patch_draft
                                .boundary
                                .iter()
                                .copied()
                                .map(|point| mapping.source(point))
                                .collect();
                            let source_start = mapping.source(start);
                            let source_end = mapping.source(end);
                            p.history.record(p.edit.clone());
                            let patch = crate::cleanup::PatchStroke {
                                boundary,
                                offset: [
                                    source_end[0] - source_start[0],
                                    source_end[1] - source_start[1],
                                ],
                                softness: self.brush_softness,
                                strength: self.brush_strength,
                            };
                            p.edit.patches.push(patch.clone());
                            self.edited();
                            self.commit_patch_feedback(&patch);
                            self.patch_draft = PatchDraft::default();
                        }
                    }
                }
                rect = self.photos[self.current]
                    .view
                    .layout(viewport, dimensions, dpi);
                hit(ui, "image", rect);
                let p = &self.photos[self.current];
                let native = needs_detail_resolution(&p.photo, p.view.scale);
                let live_brush = self.canvas_gesture == Gesture::Brush
                    && self.gesture
                    && ctx.input(|i| i.pointer.primary_down());
                let visible_crop =
                    visible_source_crop(rect, viewport, p.photo.dimensions());
                let feedback_crop = native.then(|| padded_crop(visible_crop,p.photo.dimensions()));
                self.schedule_patch_preview(ctx,feedback_crop);
                let p = &self.photos[self.current];
                let detail_ready = self
                    .detail_region
                    .as_ref()
                    .is_some_and(|(id, rev, crop, _)| {
                        *id == p.id && *rev == p.revision && crop_contains(*crop, visible_crop)
                    });
                // A pan can supersede an in-flight request without changing the edit revision.
                // Keep the old sharp tile while requesting only the newly visible source region.
                if let Some((id, rev, crop)) = self.detail_region_pending
                    && (id != p.id
                        || (rev != p.revision && !live_brush)
                        || !crop_contains(crop, visible_crop))
                {
                    if let Some(cancel) = self.detail_cancel.take() {
                        cancel.store(true, Ordering::Relaxed);
                    }
                    self.detail_region_pending = None;
                }
                if native
                    && !detail_ready
                    && (live_brush || p.rendered_revision == p.revision)
                    && self.detail_region_pending.is_none()
                    && p.photo.dimensions() != p.photo.preview.dimensions()
                {
                    self.last_detail_request = Instant::now();
                    let crop = padded_crop(visible_crop, p.photo.dimensions());
                    self.detail_region_pending = Some((p.id, p.revision, crop));
                    let cancel = Arc::new(AtomicBool::new(false));
                    self.detail_cancel = Some(cancel.clone());
                    let _ = self.tx.send(Job::DetailRegion {
                        id: p.id,
                        revision: p.revision,
                        image: p.photo.original.clone(),
                        edit: p.edit.clone(),
                        seg: p.seg.clone(),
                        crop,
                        cancel,
                    });
                }
                if let Some(index) = self.hover_preset {
                    let key = (p.id, p.revision, index);
                    if native {
                        if let Some((id, rev, i, crop)) = self.hover_region_pending
                            && ((id, rev, i) != key || !crop_contains(crop, visible_crop))
                        {
                            if let Some(cancel) = self.hover_cancel.take() {
                                cancel.store(true, Ordering::Relaxed);
                            }
                            self.hover_region_pending = None;
                            self.hover_pending = None;
                        }
                        let ready =
                            self.hover_region
                                .as_ref()
                                .is_some_and(|(id, rev, i, crop, _)| {
                                    (*id, *rev, *i) == key && crop_contains(*crop, visible_crop)
                                });
                        if !ready && self.hover_pending.is_none() {
                            let crop = padded_crop(visible_crop, p.photo.dimensions());
                            self.hover_pending = Some(key);
                            self.hover_region_pending = Some((p.id, p.revision, index, crop));
                            let cancel = Arc::new(AtomicBool::new(false));
                            self.hover_cancel = Some(cancel.clone());
                            let mut edit = p.edit.clone();
                            edit.settings = self.presets[index].settings.clone();
                            let _ = self.tx.send(Job::HoverRegion {
                                id: p.id,
                                revision: p.revision,
                                index,
                                image: p.photo.original.clone(),
                                edit,
                                seg: p.seg.clone(),
                                crop,
                                cancel,
                            });
                        }
                    } else {
                        if !self.hover_image.as_ref().is_some_and(|(id, rev, i, _, _)| {
                            (*id, *rev, *i) == key
                                && (if native {
                                    p.photo.dimensions()
                                } else {
                                    p.photo.preview.dimensions()
                                }) == self.hover_image.as_ref().unwrap().3.dimensions()
                        }) && self.hover_pending.is_none()
                        {
                            self.hover_pending = Some(key);
                            let cancel = Arc::new(AtomicBool::new(false));
                            self.hover_cancel = Some(cancel.clone());
                            let mut edit = p.edit.clone();
                            edit.settings = self.presets[index].settings.clone();
                            let _ = self.tx.send(Job::Render {
                                kind: RenderKind::Hover(index),
                                id: p.id,
                                revision: p.revision,
                                image: p.photo.preview.clone(),
                                edit,
                                seg: p.seg.clone(),
                                cancel,
                            });
                        }
                    }
                }
                let hover = self.hover_image.as_ref().filter(|(id, rev, index, _, _)| {
                    *id == p.id
                        && *rev == p.revision
                        && Some(*index) == self.hover_preset
                        && self.hover_image.as_ref().unwrap().3.dimensions()
                            == if native {
                                p.photo.dimensions()
                            } else {
                                p.photo.preview.dimensions()
                            }
                });
                let full = if native {
                    hover
                        .map(|(_, _, _, img, _)| img.clone())
                        .filter(|i| i.dimensions() == p.photo.dimensions())
                        .or_else(|| {
                            (p.photo.dimensions() == p.photo.preview.dimensions())
                                .then(|| p.edited.clone())
                        })
                        .or_else(|| Some(p.photo.original.clone()))
                } else {
                    None
                };
                let painter = ui.painter().with_clip_rect(viewport);
                painter.rect_filled(rect.expand(1.0), 0.0, BORDER);
                let mut draw_rect = rect;
                let mut native_region = None;
                let mut original = p.photo.preview.clone();
                let mut edited =
                    hover.map_or_else(|| p.edited.clone(), |(_, _, _, i, _)| i.clone());
                let mut original_tex = p.original_texture.clone();
                let mut edited_tex =
                    hover.map_or_else(|| p.edited_texture.clone(), |(_, _, _, _, t)| t.clone());
                let mut key = (p.id, p.rendered_revision);
                if hover.is_some() {
                    key.1 = key.1.wrapping_add(
                        0x4000_0000 + self.hover_preset.unwrap() as u64 * 0x0100_0000,
                    );
                }
                if let Some(full) = full {
                    let hover_region =
                        self.hover_region
                            .as_ref()
                            .filter(|(id, rev, index, crop, _)| {
                                *id == p.id
                                    && *rev == p.revision
                                    && Some(*index) == self.hover_preset
                                    && crop_contains(*crop, visible_crop)
                            });
                    let region = hover_region
                        .map(|(id, rev, _, crop, image)| (*id, *rev, *crop, image))
                        .or_else(|| {
                            sharp_region_for_photo(&self.detail_region, p.id)
                                .map(|(id, rev, crop, image)| (*id, *rev, *crop, image))
                        });
                    let crop = if let Some((_, rev, crop, _)) = region
                        && rev == p.revision
                        && crop_contains(crop, visible_crop)
                    {
                        crop
                    } else if let Some(tile) = &self.detail_tile
                        && tile.key.0 == p.id
                        && crop_contains(tile.key.3, visible_crop)
                    {
                        tile.key.3
                    } else {
                        padded_crop(visible_crop, p.photo.dimensions())
                    };
                    let (x, y) = (crop.x, crop.y);
                    native_region = Some(crop);
                    let tile_key = (
                        p.id,
                        if hover.is_some() {
                            p.revision
                        } else if let Some((_, rev, _, _)) = region {
                            rev
                        } else if detail_ready {
                            p.revision
                        } else {
                            // Revision zero identifies a sharp original fallback while the edited region renders.
                            0
                        },
                        hover_region
                            .map(|(_, _, i, _, _)| *i)
                            .or_else(|| hover.map(|(_, _, i, _, _)| *i)),
                        crop,
                    );
                    // The same edit/crop key can first hold a partial pan fallback and later
                    // receive its completed render. Track accepted source content separately.
                    let source_generation = if hover_region.is_some() {
                        self.hover_region_generation
                    } else if region.is_some() {
                        self.detail_region_generation
                    } else {
                        0
                    };
                    if self.detail_tile.as_ref().is_none_or(|t| {
                        t.key != tile_key || t.source_generation != source_generation
                    }) {
                        let before = if let Some(tile) = &self.detail_tile
                            && tile.key.0 == p.id
                            && tile.key.3 == crop
                        {
                            tile.original.clone()
                        } else {
                            Arc::new(
                                image::imageops::crop_imm(
                                    &*p.photo.original,
                                    x,
                                    y,
                                    crop.width,
                                    crop.height,
                                )
                                .to_image(),
                            )
                        };
                        let after = if hover.is_none()
                            && let Some((_, _, source_crop, source)) = region
                        {
                            if source_crop == crop {
                                source.clone()
                            } else {
                                let mut after = before.as_ref().clone();
                                // Reuse the overlapping sharp region during a pan; uncovered pixels
                                // use the sharp original until the new edited tile arrives.
                                let overlap = intersect_crop(crop, source_crop);
                                if overlap.width != 0 && overlap.height != 0 {
                                    let part = image::imageops::crop_imm(
                                        source.as_ref(),
                                        overlap.x - source_crop.x,
                                        overlap.y - source_crop.y,
                                        overlap.width,
                                        overlap.height,
                                    )
                                    .to_image();
                                    image::imageops::replace(
                                        &mut after,
                                        &part,
                                        (overlap.x - crop.x) as i64,
                                        (overlap.y - crop.y) as i64,
                                    );
                                }
                                Arc::new(after)
                            }
                        } else {
                            Arc::new(
                                image::imageops::crop_imm(&*full, x, y, crop.width, crop.height)
                                    .to_image(),
                            )
                        };
                        self.tile_serial += 1;
                        let original_texture = if let Some(tile) = &self.detail_tile
                            && Arc::ptr_eq(&tile.original, &before)
                        {
                            tile.original_texture.clone()
                        } else {
                            canvas_texture(ctx, "native-before", &before, self.gpu)
                        };
                        let edited_texture = if let Some(tile) = &self.detail_tile {
                            let mut tex = tile.edited_texture.clone();
                            if !self.gpu {
                                let display = display_image(&after, false);
                                tex.set(
                                    egui::ColorImage::from_rgba_unmultiplied(
                                        [display.width() as usize, display.height() as usize],
                                        display.as_raw(),
                                    ),
                                    egui::TextureOptions::LINEAR,
                                );
                            }
                            tex
                        } else {
                            canvas_texture(ctx, "native-after", &after, self.gpu)
                        };
                        self.detail_tile = Some(DetailTile {
                            key: tile_key,
                            source_generation,
                            original_texture,
                            edited_texture,
                            original: before,
                            edited: after,
                            serial: self.tile_serial,
                        });
                    }
                    let tile = self.detail_tile.as_ref().unwrap();
                    draw_rect = Rect::from_min_size(
                        rect.min + vec2(x as f32, y as f32) * p.view.scale / dpi,
                        vec2(crop.width as f32, crop.height as f32) * p.view.scale / dpi,
                    );
                    original = tile.original.clone();
                    edited = tile.edited.clone();
                    original_tex = tile.original_texture.clone();
                    edited_tex = tile.edited_texture.clone();
                    key = (p.id, 0x8000_0000 + tile.serial);
                }
                // A patch preview replaces the canvas pair. The GPU canvas has one
                // image/uniform binding, so a second callback would overwrite it.
                if self.brush == Some(Target::Patch) && let Some(frame) = &self.patch_feedback.frame
                    && (self.patch_draft.source_drag.is_some() || frame.committed)
                    && frame.id == p.id && frame.revision == p.revision
                    && match frame.crop { Some(crop) => native && crop_contains(crop, visible_crop), None => !native } {
                    draw_rect = if let Some(crop) = frame.crop {
                        Rect::from_min_size(rect.min+vec2(crop.x as f32,crop.y as f32)*p.view.scale/dpi,vec2(crop.width as f32,crop.height as f32)*p.view.scale/dpi)
                    } else {rect};
                    native_region = frame.crop;
                    original = frame.original.clone();
                    edited = frame.image.clone();
                    original_tex = frame.original_texture.clone();
                    edited_tex = frame.texture.clone();
                    key = (p.id, 0x6000_0000 + frame.request);
                }
                let split_x = rect.left() + rect.width() * self.split;
                if self.gpu {
                    let paint_rect = draw_rect.intersect(viewport);
                    hit(ui, "canvas-paint", paint_rect);
                    painter.add(egui_wgpu::Callback::new_paint_callback(
                        paint_rect,
                        gpu::Canvas {
                            key,
                            image_rect: draw_rect,
                            paint_rect,
                            original,
                            edited,
                            split: if self.compare {
                                (split_x - draw_rect.left()) / draw_rect.width()
                            } else {
                                0.0
                            },
                            show_original: held,
                        },
                    ));
                } else {
                    let uv = Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0));
                    painter.image(
                        if held {
                            original_tex.id()
                        } else {
                            edited_tex.id()
                        },
                        draw_rect,
                        uv,
                        Color32::WHITE,
                    );
                    if self.compare && !held {
                        painter
                            .with_clip_rect(
                                Rect::from_min_max(viewport.min, pos2(split_x, viewport.bottom()))
                                    .intersect(viewport),
                            )
                            .image(original_tex.id(), draw_rect, uv, Color32::WHITE);
                    }
                }
                if self.compare && !held {
                    painter.line_segment(
                        [pos2(split_x, rect.top()), pos2(split_x, rect.bottom())],
                        Line::new(1.5_f32, YELLOW),
                    );
                    painter.circle_filled(pos2(split_x, viewport.center().y), 14.0, YELLOW);
                    painter.text(
                        pos2(split_x, viewport.center().y),
                        Align2::CENTER_CENTER,
                        "‹ ›",
                        FontId::proportional(14.0),
                        BG,
                    );
                    hit(
                        ui,
                        "divider",
                        Rect::from_center_size(
                            pos2(split_x, viewport.center().y),
                            vec2(28.0, 28.0),
                        ),
                    );
                }
                if let Some(target) = self.brush {
                    if self.show_mask
                        && self.canvas_gesture != Gesture::Brush
                        && !matches!(
                            target,
                            Target::Heal | Target::Liquify | Target::Clone | Target::Patch
                        )
                    {
                        let source = if native_region.is_some() {
                            &p.photo.original
                        } else {
                            &p.photo.preview
                        };
                        let region = native_region.unwrap_or(engine::Crop {
                            x: 0,
                            y: 0,
                            width: source.width(),
                            height: source.height(),
                        });
                        if self
                            .mask_texture
                            .as_ref()
                            .is_none_or(|(id, edit, seg, t, crop, _)| {
                                *id != p.id
                                    || *t != target
                                    || *crop != region
                                    || !edit.strokes.same_storage(&p.edit.strokes)
                                    || !edit.warps.same_storage(&p.edit.warps)
                                    || edit.layers != p.edit.layers
                                    || (target == Target::LayerMask && edit.stack != p.edit.stack)
                                    || shape_values(&edit.settings)
                                        != shape_values(&p.edit.settings)
                                    || !same_seg(seg.as_ref(), p.seg.as_ref())
                            })
                        {
                            let overlay = if target == Target::LayerMask {
                                let coverage = p.edit.active_layer().and_then(|layer| layer.mask.as_ref())
                                    .map(|mask| mask.coverage(source.dimensions(), region));
                                image::RgbaImage::from_fn(region.width, region.height, |x, y| {
                                    let hidden = coverage.as_ref().map_or(0, |pixels| 255 - pixels[(y * region.width + x) as usize]);
                                    image::Rgba([220, 55, 40, (hidden as f32 * 0.45) as u8])
                                })
                            } else {
                                engine::mask_overlay_region(
                                    source,
                                    &p.edit,
                                    p.seg.as_deref(),
                                    target,
                                    region,
                                )
                            };
                            self.mask_texture = Some((
                                p.id,
                                p.edit.clone(),
                                p.seg.clone(),
                                target,
                                region,
                                texture(ctx, "mask-overlay", &overlay),
                            ));
                        }
                        painter.image(
                            self.mask_texture.as_ref().unwrap().5.id(),
                            if native_region.is_some() {
                                draw_rect
                            } else {
                                rect
                            },
                            Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)),
                            Color32::WHITE,
                        );
                    }
                    if self.preferences.brush_outline
                        && target != Target::Patch
                        && !(space && self.canvas_gesture == Gesture::Pan)
                        && let Some(pos) = pointer
                        && rect.contains(pos)
                        && viewport.contains(pos)
                    {
                        painter.circle_stroke(
                            pos,
                            self.brush_radius * rect.size().x.min(rect.size().y),
                            Line::new(1.0_f32, YELLOW),
                        );
                    }
                    if target == Target::Clone
                        && let Some((id, anchor)) = self.clone_anchor
                        && id == p.id
                    {
                        let source = if self.clone_aligned && let Some((offset_id, offset)) = self.clone_offset
                            && offset_id == p.id && let Some(pointer) = pointer.filter(|p| viewport.contains(*p) && rect.contains(*p)) {
                            let uv = [(pointer.x-rect.left())/rect.width(), (pointer.y-rect.top())/rect.height()];
                            let mapped = crate::geometry::source_uv(uv,&p.edit,p.seg.as_deref(),p.photo.dimensions());
                            [mapped[0]+offset[0],mapped[1]+offset[1]]
                        } else if let (Some(origin), Some(last)) = (self.clone_origin, self.last_stroke)
                        {
                            let last = crate::geometry::source_uv(
                                last,
                                &p.edit,
                                p.seg.as_deref(),
                                p.photo.dimensions(),
                            );
                            [
                                anchor[0] + last[0] - origin[0],
                                anchor[1] + last[1] - origin[1],
                            ]
                        } else {
                            anchor
                        };
                        let uv = crate::geometry::display_uv(
                            source,
                            &p.edit,
                            p.seg.as_deref(),
                            p.photo.dimensions(),
                        );
                        let pos = rect.min + vec2(uv[0] * rect.width(), uv[1] * rect.height());
                        if viewport.contains(pos) && rect.contains(pos) {
                            let source_color = Color32::from_rgb(80, 225, 220);
                            let radius = self.brush_radius * rect.size().x.min(rect.size().y);
                            painter.circle_stroke(pos, radius, Line::new(1.0_f32, source_color));
                            painter.line_segment(
                                [pos - vec2(7.0, 0.0), pos + vec2(7.0, 0.0)],
                                Line::new(1.0_f32, source_color),
                            );
                            painter.line_segment(
                                [pos - vec2(0.0, 7.0), pos + vec2(0.0, 7.0)],
                                Line::new(1.0_f32, source_color),
                            );
                        }
                    }
                }
                if self.brush == Some(Target::Patch) && self.patch_draft.boundary.len() >= 2 {
                    let to_pos = |uv: [f32; 2]| {
                        pos2(
                            rect.left() + uv[0] * rect.width(),
                            rect.top() + uv[1] * rect.height(),
                        )
                    };
                    for pair in self.patch_draft.boundary.windows(2) {
                        painter.line_segment(
                            [to_pos(pair[0]), to_pos(pair[1])],
                            Line::new(1.5_f32, YELLOW),
                        );
                    }
                    if !self.patch_draft.drawing {
                        painter.line_segment(
                            [
                                to_pos(*self.patch_draft.boundary.last().unwrap()),
                                to_pos(self.patch_draft.boundary[0]),
                            ],
                            Line::new(1.5_f32, YELLOW),
                        );
                    }
                    if let Some((start, end)) = self.patch_draft.source_drag {
                        let delta = [end[0] - start[0], end[1] - start[1]];
                        for index in 0..self.patch_draft.boundary.len() {
                            let a = self.patch_draft.boundary[index];
                            let b = self.patch_draft.boundary
                                [(index + 1) % self.patch_draft.boundary.len()];
                            painter.line_segment(
                                [
                                    to_pos([a[0] + delta[0], a[1] + delta[1]]),
                                    to_pos([b[0] + delta[0], b[1] + delta[1]]),
                                ],
                                Line::new(1.25_f32, Color32::from_rgb(80, 225, 220)),
                            );
                        }
                    }
                }
                if native
                    && !detail_ready
                    && p.photo.dimensions() != p.photo.preview.dimensions()
                {
                    pill(&painter, viewport.left_top() + vec2(8.0, 8.0), "DETAIL…");
                }
                if let Some(index) = self.hover_preset {
                    ui.painter().text(
                        viewport.center_top() + vec2(0.0, 16.0),
                        Align2::CENTER_TOP,
                        format!("Preview · {}", self.presets[index].name),
                        FontId::proportional(12.0),
                        YELLOW,
                    );
                }
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!(
                            "{} × {}  ·  sRGB",
                            dimensions.x as u32, dimensions.y as u32
                        ))
                        .size(10.0)
                        .color(MUTED),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let view = &mut self.photos[self.current].view;
                        let fit = small_chip(ui, "Fit", view.fit);
                        hit(ui, "fit", fit.rect);
                        if fit.clicked() {
                            *view = View::default();
                        }
                        let native =
                            small_chip(ui, "100%", !view.fit && (view.scale - 1.0).abs() < 0.001);
                        hit(ui, "native", native.rect);
                        if native
                            .on_hover_text("One original pixel per screen pixel")
                            .clicked()
                        {
                            view.scale = 1.0;
                            view.fit = false;
                            view.pan = [0.0; 2];
                        }
                        let plus = small_chip(ui, "+", false);
                        hit(ui, "zoom-plus", plus.rect);
                        if plus.clicked() {
                            view.zoom_at(view.scale * 1.2, viewport.center(), viewport);
                        }
                        ui.label(
                            RichText::new(format!("{:.1}%", view.scale * 100.0))
                                .size(11.0)
                                .color(TEXT),
                        );
                        let minus = small_chip(ui, "−", false);
                        hit(ui, "zoom-minus", minus.rect);
                        if minus.clicked() {
                            view.zoom_at(view.scale / 1.2, viewport.center(), viewport);
                        }
                    });
                });
            });
    }

    fn crop_preview(&mut self, ui: &mut egui::Ui) {
        if self.photos.is_empty() {
            return;
        }
        if self.crop_index >= self.photos.len() {
            self.crop_index = self.current;
        }
        if self.export_all && !self.photos[self.crop_index].selected {
            self.crop_index = self
                .photos
                .iter()
                .position(|p| p.selected)
                .unwrap_or(self.current);
        }
        let index = if self.export_all {
            self.crop_index
        } else {
            self.current
        };
        if self.export_all {
            egui::ComboBox::from_id_salt("crop-photo")
                .selected_text(&self.photos[index].photo.name)
                .show_ui(ui, |ui| {
                    for (i, p) in self.photos.iter().enumerate().filter(|(_, p)| p.selected) {
                        ui.selectable_value(&mut self.crop_index, i, &p.photo.name);
                    }
                });
        }
        let p = &mut self.photos[index];
        let (w, h) = p.photo.dimensions();
        let (ow, oh) = engine::output_dimensions(w, h, self.export_size);
        ui.label(
            RichText::new(format!("Output · {ow} × {oh} pixels"))
                .size(12.0)
                .color(YELLOW),
        );
        let (area, _) = ui.allocate_exact_size(vec2(ui.available_width(), 180.0), Sense::hover());
        let scale = (area.width() / w as f32).min(area.height() / h as f32);
        let image_rect = Rect::from_center_size(area.center(), vec2(w as f32, h as f32) * scale);
        let response = ui.interact(
            image_rect,
            ui.id().with("crop-preview"),
            Sense::click_and_drag(),
        );
        hit(ui, "crop-preview", image_rect);
        let social = !matches!(self.export_size, ExportSize::Original | ExportSize::Web);
        if social {
            if response.dragged() {
                let delta = response.drag_delta();
                p.crop_center[0] += delta.x / image_rect.width();
                p.crop_center[1] += delta.y / image_rect.height();
            } else if response.clicked()
                && let Some(pos) = response.interact_pointer_pos()
            {
                p.crop_center = [
                    (pos.x - image_rect.left()) / image_rect.width(),
                    (pos.y - image_rect.top()) / image_rect.height(),
                ];
            }
        }
        let crop = engine::crop_rect(w, h, self.export_size, p.crop_center);
        if social {
            p.crop_center = [
                (crop.x as f32 + crop.width as f32 / 2.0) / w as f32,
                (crop.y as f32 + crop.height as f32 / 2.0) / h as f32,
            ];
        }
        let frame = Rect::from_min_size(
            image_rect.min + vec2(crop.x as f32, crop.y as f32) * scale,
            vec2(crop.width as f32, crop.height as f32) * scale,
        );
        hit(ui, "crop-frame", frame);
        ui.painter().image(
            p.edited_texture.id(),
            image_rect,
            Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)),
            Color32::WHITE,
        );
        for outside in [
            Rect::from_min_max(image_rect.min, pos2(frame.left(), image_rect.bottom())),
            Rect::from_min_max(pos2(frame.right(), image_rect.top()), image_rect.max),
            Rect::from_min_max(pos2(frame.left(), image_rect.top()), frame.right_top()),
            Rect::from_min_max(
                frame.left_bottom(),
                pos2(frame.right(), image_rect.bottom()),
            ),
        ] {
            ui.painter()
                .rect_filled(outside, 0.0, Color32::from_black_alpha(160));
        }
        ui.painter().rect_stroke(
            frame,
            0.0,
            Line::new(1.5_f32, YELLOW),
            egui::StrokeKind::Inside,
        );
        if social {
            ui.label(
                RichText::new("Drag the frame or click to reposition the crop.")
                    .size(10.0)
                    .color(MUTED),
            );
        }
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
                egui::ScrollArea::vertical()
                    .id_salt("export-scroll")
                    .max_height((ctx.content_rect().height() - 100.0).max(150.0))
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
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
                        ui.horizontal_wrapped(|ui| {
                            for (label, size, png, quality) in [
                                ("Print", ExportSize::Original, false, 95),
                                ("Web", ExportSize::Web, false, 85),
                                ("Cutout", ExportSize::Original, true, 95),
                            ] {
                                if ui.button(label).clicked() {
                                    self.export_size = size;
                                    self.export_png = png;
                                    self.quality = quality;
                                }
                            }
                        });
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
                        ui.add_space(12.0);
                        self.crop_preview(ui);
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
                        if self.photos.get(self.current).is_some_and(|p| {
                            p.edit.effective_settings().background == Background::Transparent
                        }) && !self.export_png
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
                        ui.label(
                            RichText::new(
                                "Portrait AI and original pixels are prepared by the export queue.",
                            )
                            .size(11.)
                            .color(MUTED),
                        );
                        let export_button = ui.add(
                            primary("Choose folder & add to queue")
                                .min_size(vec2(ui.available_width(), 42.)),
                        );
                        hit(ui, "export-start", export_button.rect);
                        if export_button.clicked()
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
                                .map(|(_, p)| {
                                    (
                                        {
                                            let mut photo = p.photo.clone();
                                            photo.release_pixels();
                                            photo
                                        },
                                        p.edit.clone(),
                                        p.seg.as_ref().map(|s| Arc::new(s.session_copy())),
                                        p.crop_center,
                                    )
                                })
                                .collect();
                            if photos.is_empty() {
                                self.notify("Select at least one photo to export.");
                            } else {
                                self.exporting = Some((0, photos.len()));
                                self.export_errors = 0;
                                self.exported_paths.clear();
                                let cancel = Arc::new(AtomicBool::new(false));
                                self.export_cancel = Some(cancel.clone());
                                self.export_serial += 1;
                                self.export_jobs.push(asset_workflow::ExportQueueItem {
                                    id: self.export_serial,
                                    directory: directory.clone(),
                                    finished: 0,
                                    total: photos.len(),
                                    errors: 0,
                                    state: "Queued".into(),
                                    cancel: cancel.clone(),
                                });
                                let _ = self.tx.send(Job::Export {
                                    concurrency: self.preferences.export_concurrency,
                                    queue: self.export_serial,
                                    provider: self.provider,
                                    cancel,
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
            });
        self.export_open = self.export_open && open;
    }

    fn status(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("status")
            .exact_height(26.0)
            .frame(
                egui::Frame::new()
                    .fill(BG)
                    .inner_margin(egui::Margin::symmetric(12, 4)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let message = if let Some((finished, total)) = self.exporting {
                        format!("Exporting · {finished}/{total}")
                    } else if self.importing {
                        "Importing photos…".into()
                    } else if self.photos.iter().any(|p| p.ai_processing) {
                        "Analyzing faces & neural retouch…".into()
                    } else if self.model_busy {
                        "Creating mask…".into()
                    } else if self.render_busy {
                        "Updating preview…".into()
                    } else {
                        "Original preserved  ·  All processing stays on your device".into()
                    };
                    ui.label(RichText::new(message).size(10.0).color(MUTED));
                    if !self.export_jobs.is_empty() && ui.small_button("Queue").clicked() {
                        self.export_queue_open = true;
                    }
                    if let Some(cancel) = &self.export_cancel {
                        let cancelling = cancel.load(Ordering::Relaxed);
                        let button = ui.add_enabled(
                            !cancelling,
                            egui::Button::new(if cancelling {
                                "Cancelling…"
                            } else {
                                "Cancel export"
                            })
                            .small(),
                        );
                        hit(ui, "cancel-export", button.rect);
                        if button.clicked() {
                            cancel.store(true, Ordering::Relaxed);
                        }
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(concat!("HASTUR  /  ", env!("CARGO_PKG_VERSION")))
                                .size(9.0)
                                .color(MUTED),
                        );
                        if ctx.content_rect().width() >= 1100.0 && self.recovery_store.is_some() {
                            let label = if !self.preferences.autosave {
                                "Autosave paused"
                            } else if self.autosave_pending.is_some() {
                                "Saving…"
                            } else if self.autosave_dirty_since.is_some() {
                                "Unsaved changes"
                            } else if self.autosave_saved_at.is_some() {
                                "Edits saved"
                            } else {
                                "Autosave ready"
                            };
                            ui.label(RichText::new(label).size(10.0).color(
                                if self.autosave_dirty_since.is_some() {
                                    YELLOW
                                } else {
                                    MUTED
                                },
                            ));
                        }
                    });
                });
            });
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        if self.preferences_open || self.command_open {
            return;
        }
        if self.layer_rename.is_some() {
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
                self.layer_rename = None;
            }
            return;
        }
        let typing = ctx
            .memory(|memory| memory.focused())
            .is_some_and(|id| egui::TextEdit::load_state(ctx, id).is_some());
        if !typing
            && !egui::Popup::is_any_open(ctx)
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Tab))
        {
            // egui queues Tab focus navigation before application shortcuts run.
            ctx.memory_mut(|memory| memory.move_focus(egui::FocusDirection::None));
            if self.inspector.is_some()
                && (ctx.content_rect().width() >= 1050.0 || self.compact_open)
            {
                self.last_inspector = self.inspector.unwrap();
                self.inspector = None;
                self.compact_open = false;
            } else {
                self.inspector = Some(self.last_inspector);
                self.compact_open = true;
                self.history_focus = true;
            }
        }
        if ctx.wants_keyboard_input() {
            return;
        }
        self.command_shortcuts(ctx);
        self.micro_shortcuts(ctx);
        if self.preferences_open || self.command_open {
            return;
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            self.brush = None;
            self.end_canvas_gesture();
            self.patch_draft = PatchDraft::default();
            self.export_open = false;
            self.export_queue_open = false;
        }
    }
}

impl eframe::App for HasturApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        if self.gpu_release {
            if let Some(state) = frame.wgpu_render_state() {
                gpu::release_images(state);
            }
            self.gpu_release = false;
        }
        self.draw(ctx);
    }
    fn on_exit(&mut self) {
        if let Err(error) = self.flush_recovery() {
            eprintln!("Could not save final workspace recovery: {error:#}");
        }
    }
}
impl HasturApp {
    fn startup_ready(&self) -> bool {
        (!self.importing || (self.pending_session.is_none() && !self.photos.is_empty()))
            && self.photos.get(self.current).is_none_or(|p| {
                self.asset_error == Some(p.id)
                    || (p.photo.resident && !p.ai_processing && p.revision == p.rendered_revision)
            })
    }

    fn startup_splash(&mut self, ctx: &egui::Context) {
        let portrait = self.splash_texture.get_or_insert_with(|| {
            ctx.load_texture(
                "hastur-splash",
                branding::splash_portrait(),
                egui::TextureOptions::LINEAR,
            )
        });
        let image_id = portrait.id();
        let image_size = portrait.size_vec2();
        let ready = self.startup_ready();
        let (stage, detail, progress) = if self.importing {
            (
                if self.automatic_recovery {
                    "Restoring saved workspace"
                } else {
                    "Opening photographs"
                }
                .to_owned(),
                if self.automatic_recovery {
                    "Restoring your photographs, adjustments and history."
                } else {
                    "Preparing your workspace and original images."
                }
                .to_owned(),
                0.02,
            )
        } else if let Some(status) = self
            .photos
            .get(self.current)
            .filter(|p| p.ai_processing)
            .and_then(|p| p.ai_status.as_ref())
        {
            (status.stage.clone(), status.detail.clone(), status.progress)
        } else if self.photos.iter().any(|p| p.ai_processing) {
            (
                "Preparing portrait AI".to_owned(),
                "Checking local models and available acceleration.".to_owned(),
                0.05,
            )
        } else if !ready {
            (
                "Preparing the photo canvas".to_owned(),
                "Rendering the first preview in linear color.".to_owned(),
                0.97,
            )
        } else {
            (
                "Workspace ready".to_owned(),
                self.photos
                    .get(self.current)
                    .and_then(|p| p.note.clone())
                    .unwrap_or_else(|| "Your original photographs are preserved.".into()),
                1.0,
            )
        };
        let mut open_workspace = false;
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(BG))
            .show(ctx, |ui| {
                let area = ui.max_rect();
                let compact = area.width() < 780.0;
                let base_height = if compact { 600.0 } else { 500.0 };
                let scale = ((area.width() - 40.0) / 900.0)
                    .min((area.height() - 40.0) / base_height)
                    .clamp(0.35, 1.0);
                let card = Rect::from_center_size(area.center(), vec2(900.0, base_height) * scale);
                let painter = ui.painter();
                painter.rect_filled(card, 18.0 * scale, PANEL);
                painter.rect_stroke(
                    card,
                    18.0 * scale,
                    Line::new(1.0_f32, BORDER),
                    egui::StrokeKind::Inside,
                );
                let portrait_rect = Rect::from_min_max(
                    pos2(card.left() + 400.0 * scale, card.top() + 16.0 * scale),
                    card.max - vec2(16.0, 16.0) * scale,
                );
                // Uniform fit keeps the portrait's proportions at every window size.
                let fit = (portrait_rect.width() / image_size.x)
                    .min(portrait_rect.height() / image_size.y);
                let fitted = Rect::from_center_size(portrait_rect.center(), image_size * fit);
                painter.image(
                    image_id,
                    fitted,
                    Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
                let origin = card.min + vec2(34.0, 34.0) * scale;
                branding::paint_logo(
                    painter,
                    Rect::from_min_size(origin, vec2(64.0, 64.0) * scale),
                    branding::IVORY,
                );
                painter.text(
                    origin + vec2(0.0, 84.0) * scale,
                    Align2::LEFT_TOP,
                    "HASTUR",
                    FontId::proportional((34.0 * scale).max(26.0)),
                    TEXT,
                );
                painter.text(
                    origin + vec2(scale, (128.0 * scale).max(84.0 * scale + 40.0)),
                    Align2::LEFT_TOP,
                    "R E T O U C H",
                    FontId::proportional((14.0 * scale).max(11.0)),
                    MUTED,
                );
                if !compact {
                    painter.text(
                        origin + vec2(0.0, 178.0) * scale,
                        Align2::LEFT_TOP,
                        "A little polish. Still entirely you.",
                        FontId::proportional(13.0 * scale),
                        MUTED,
                    );
                }
                let status_rect = Rect::from_min_size(
                    origin + vec2(0.0, 250.0) * scale,
                    vec2(328.0, 145.0) * scale,
                );
                ui.scope_builder(egui::UiBuilder::new().max_rect(status_rect), |ui| {
                    ui.set_max_width(status_rect.width());
                    ui.label(
                        RichText::new(&stage)
                            .size((15.0 * scale).max(12.0))
                            .strong()
                            .color(TEXT),
                    );
                    ui.add(
                        egui::ProgressBar::new(progress)
                            .fill(YELLOW)
                            .desired_height(3.0 * scale)
                            .desired_width(status_rect.width())
                            .animate(!ready && !self.preferences.reduced_motion),
                    );
                    ui.add(
                        egui::Label::new(
                            RichText::new(&detail)
                                .size((11.0 * scale).max(11.0))
                                .color(MUTED),
                        )
                        .truncate(),
                    )
                    .on_hover_text(&detail);
                    ui.add_space(4.0 * scale);
                    ui.add(
                        egui::Label::new(
                            RichText::new(&self.gpu_adapter)
                                .size((10.0 * scale).max(10.0))
                                .color(MUTED),
                        )
                        .truncate(),
                    )
                    .on_hover_text(&self.gpu_adapter);
                    if !self.photos.is_empty() {
                        ui.add_space(5.0 * scale);
                        let response = ui.add(
                            egui::Button::new(RichText::new("Open workspace").size(12.0))
                                .min_size(vec2(150.0, 32.0)),
                        );
                        hit(ui, "startup-open", response.rect);
                        if response.clicked() {
                            open_workspace = true;
                        }
                    }
                });
                ui.painter().text(
                    card.min + vec2(34.0, base_height - 30.0) * scale,
                    Align2::LEFT_TOP,
                    format!(
                        "Version {}  ·  Local photo studio",
                        env!("CARGO_PKG_VERSION")
                    ),
                    FontId::proportional((10.0 * scale).max(9.0)),
                    MUTED,
                );
            });
        if open_workspace {
            self.startup = false;
        }
        ctx.request_repaint_after(Duration::from_millis(33));
    }

    fn draw(&mut self, ctx: &egui::Context) {
        self.frames += 1;
        self.poll(ctx);
        self.ensure_active_asset(ctx);
        if self.startup && !self.splash_preview && self.startup_ready() {
            self.startup = false;
        }
        if self.startup {
            self.startup_splash(ctx);
            self.capture_screenshot(ctx, self.splash_preview && self.startup_ready());
            return;
        }
        self.hover_preset = None;
        self.shortcuts(ctx);
        self.preferences_dialog(ctx);
        self.command_dialog(ctx);
        let dropped = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.clone())
                .collect::<Vec<_>>()
        });
        if !dropped.is_empty()
            && self.pending_session.is_none()
            && self.photos.len() + dropped.len() <= 64
        {
            self.importing = true;
            for path in &dropped {
                if path.is_dir() {
                    self.import_batches += 1;
                    let _ = self.tx.send(Job::ImportFolder(path.clone()));
                }
            }
            let files: Vec<_> = dropped.into_iter().filter(|p| p.is_file()).collect();
            if !files.is_empty() {
                self.import_batches += 1;
                let _ = self.tx.send(Job::Import(files));
            }
        }
        self.header(ctx);
        self.status(ctx);
        self.rail(ctx);
        self.tools(ctx);
        self.rename_layer_dialog(ctx);
        self.filmstrip(ctx);
        self.ensure_active_asset(ctx);
        self.canvas(ctx);
        let live_brush = self.canvas_gesture == Gesture::Brush && self.gesture;
        self.schedule_preview(ctx, live_brush);
        self.prepare_requested_ai();
        self.export_dialog(ctx);
        self.export_queue_dialog(ctx);
        self.autosave_tick(ctx);
        if !ctx.input(|i| i.pointer.any_down()) {
            self.end_canvas_gesture();
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
            egui::Window::new("Hastur shortcuts")
                .open(&mut self.help)
                .collapsible(false)
                .resizable(false)
                .default_width(460.0)
                .max_height((ctx.content_rect().height() - 80.0).max(120.0))
                .scroll([false, true])
                .show(ctx, |ui| {
                    for (key, action) in [
                        ("Ctrl + O", "Import photos"),
                        ("Ctrl + S", "Save edit session"),
                        ("Ctrl + Z", "Undo"),
                        ("Ctrl + Shift + Z", "Redo"),
                        ("Ctrl + E", "Export"),
                        ("Ctrl + ,", "Preferences"),
                        ("Ctrl + K", "Find a command"),
                        ("F / F6", "Focus workspace / toggle filmstrip"),
                        (
                            "Left / Right",
                            "Previous / next photo (Shift: extend selection)",
                        ),
                        ("1–5 / Shift+0", "Rate photo / clear rating"),
                        ("Ctrl + A / I", "Select all photos / invert selection"),
                        ("[ / ]", "Smaller / larger brush (Shift: strength)"),
                        ("M / X", "Mask overlay / add or erase masks"),
                        ("Tab", "Show / hide the active panel"),
                        ("Ctrl + Shift + N", "New retouch layer (Layers panel)"),
                        ("Ctrl + J", "Duplicate layer (Layers panel)"),
                        ("Ctrl + [ / ]", "Move layer down / up (Layers panel)"),
                        ("Delete / F2", "Delete / rename layer (Layers panel)"),
                        ("Alt + layer eye", "Solo layer / restore visibility"),
                        ("A", "Auto retouch"),
                        ("B", "Before / after"),
                        ("Hold backslash", "View original"),
                        ("0", "Fit photo"),
                        ("Space + drag", "Pan"),
                        ("Scroll", "Zoom around pointer"),
                        ("Double-click photo", "Fit / 100%"),
                        ("Double-click slider", "Reset adjustment"),
                        ("Ctrl / Shift + click", "Toggle selection / select range"),
                        ("Alt + click", "Select clone source"),
                        ("Patch", "Draw an outline, then drag it to clean texture"),
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
        self.capture_screenshot(ctx, true);
    }

    fn capture_screenshot(&mut self, ctx: &egui::Context, ready: bool) {
        if let Some(path) = &self.screenshot {
            ctx.request_repaint_after(Duration::from_millis(30));
            let settled = ready
                && !self.importing
                && !self.render_busy
                && self.detail_region_pending.is_none()
                && !self.patch_feedback.busy()
                && self.hover_pending.is_none()
                && self.layer_thumbs_pending.is_none()
                && self.preset_thumbs.iter().all(Option::is_some)
                && !self.photos.iter().any(|p| p.ai_processing)
                && self
                    .photos
                    .get(self.current)
                    .is_none_or(|p| p.revision == p.rendered_revision);
            if !settled {
                self.screenshot_ready_since = None;
            }
            let animations_finished = settled
                && self
                    .screenshot_ready_since
                    .get_or_insert_with(Instant::now)
                    .elapsed()
                    >= Duration::from_millis(350);
            if self.frames > 30 && animations_finished && !self.screenshot_requested {
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

fn history_label(before: &Edit, after: &Edit) -> String {
    if before.stack.solo != after.stack.solo {
        return if after.stack.solo.is_some() {
            "Solo layer"
        } else {
            "Restore layer visibility"
        }
        .into();
    }
    if before.stack.layers.len() != after.stack.layers.len() {
        return if after.stack.layers.len() > before.stack.layers.len() {
            "Add layer".into()
        } else {
            "Delete layer".into()
        };
    }
    if before
        .stack
        .layers
        .iter()
        .map(|l| l.id)
        .ne(after.stack.layers.iter().map(|l| l.id))
    {
        return "Reorder layers".into();
    }
    for (a, b) in before.stack.layers.iter().zip(after.stack.layers.iter()) {
        if a.name != b.name {
            return format!("Rename · {}", b.name);
        }
        if a.color != b.color {
            return format!("{} label · {}", b.name, b.color.label());
        }
        if a.visible != b.visible {
            return format!("{} · {}", if b.visible { "Show" } else { "Hide" }, b.name);
        }
        if a.opacity != b.opacity {
            return format!("{} opacity · {:.0}%", b.name, b.opacity);
        }
        if a.locked != b.locked {
            return format!("{} · {}", if b.locked { "Lock" } else { "Unlock" }, b.name);
        }
        if a.blend != b.blend {
            return format!("{} · {}", b.name, b.blend.label());
        }
    }
    for kind in LayerKind::ALL {
        let previous = before.layers.get(kind);
        let next = after.layers.get(kind);
        if previous.visible != next.visible {
            return format!(
                "{} {} layer",
                if next.visible { "Show" } else { "Hide" },
                kind.label()
            );
        }
        if previous.opacity != next.opacity {
            return format!("{} opacity · {:.0}%", kind.label(), next.opacity);
        }
    }
    if before.preset != after.preset
        && let Some(name) = &after.preset
    {
        return format!("Preset · {name}");
    }
    for (name, changed) in [
        (
            "Liquify",
            !before.warps.same_storage(&after.warps) && before.warps != after.warps,
        ),
        (
            "Clone stamp",
            !before.clones.same_storage(&after.clones) && before.clones != after.clones,
        ),
        (
            "Patch",
            !before.patches.same_storage(&after.patches) && before.patches != after.patches,
        ),
    ] {
        if changed {
            return name.into();
        }
    }
    if !before.strokes.same_storage(&after.strokes) && before.strokes != after.strokes {
        return after.strokes.last().map_or_else(
            || "Clear local strokes".into(),
            |stroke| {
                format!(
                    "{}{}",
                    stroke.target.label(),
                    if stroke.target == Target::Heal {
                        ""
                    } else {
                        " mask"
                    }
                )
            },
        );
    }
    let a = &before.settings;
    let b = &after.settings;
    if a.disabled != b.disabled {
        return "Toggle adjustment".into();
    }
    let differences: Vec<_> = [
        ("Skin smoothing", a.smoothing, b.smoothing),
        ("Blemish removal", a.blemishes, b.blemishes),
        ("Skin tone", a.tone_evenness, b.tone_evenness),
        ("Redness", a.redness, b.redness),
        ("Teeth", a.teeth, b.teeth),
        ("Eye clarity", a.eyes, b.eyes),
        ("Under-eye bags", a.under_eyes, b.under_eyes),
        ("Forehead", a.forehead, b.forehead),
        ("Laugh lines", a.laugh_lines, b.laugh_lines),
        ("Contour", a.contour, b.contour),
        ("Face highlight", a.face_highlight, b.face_highlight),
        ("Eye size", a.eye_size, b.eye_size),
        ("Nose width", a.nose_width, b.nose_width),
        ("Lip plumpness", a.lip_plumpness, b.lip_plumpness),
        ("Jawline", a.jawline, b.jawline),
        ("Spot heal strength", a.healing, b.healing),
        ("Flyaway hairs", a.flyaway_hairs, b.flyaway_hairs),
        ("Exposure", a.exposure, b.exposure),
        ("Contrast", a.contrast, b.contrast),
        ("Shadows", a.shadows, b.shadows),
        ("Highlights", a.highlights, b.highlights),
        ("Warmth", a.warmth, b.warmth),
        ("Tint", a.tint, b.tint),
        ("Saturation", a.saturation, b.saturation),
        ("Sharpening", a.sharpening, b.sharpening),
        ("Vignette", a.vignette, b.vignette),
        ("Background blur", a.background_blur, b.background_blur),
    ]
    .into_iter()
    .filter(|(_, old, new)| old != new)
    .collect();
    if a.background != b.background || a.background_color != b.background_color {
        "Background".into()
    } else if a.color != b.color {
        "Selective color / curves".into()
    } else if differences.len() == 1 {
        if differences[0].0 == "Exposure" {
            format!("Exposure · {:+.2} EV", differences[0].2)
        } else {
            format!("{} · {:.0}", differences[0].0, differences[0].2)
        }
    } else if b == &Settings::default() {
        "Reset adjustments".into()
    } else {
        "Adjustments".into()
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
        style.spacing.item_spacing = vec2(6.0, 6.0);
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

fn hit(ui: &egui::Ui, key: &str, rect: Rect) {
    #[cfg(test)]
    ui.ctx()
        .data_mut(|d| d.insert_temp(egui::Id::new(("hit", key)), rect));
    #[cfg(not(test))]
    let _ = (ui, key, rect);
}
fn adjustment(
    ui: &mut egui::Ui,
    label: &str,
    settings: &mut Settings,
    kind: Adjustment,
    range: std::ops::RangeInclusive<f32>,
    tooltip: &str,
) {
    let mut enabled = !settings.disabled.contains(&kind);
    let previous = enabled;
    slider_control(
        ui,
        label,
        settings.value_mut(kind),
        range,
        tooltip,
        Some(&mut enabled),
    );
    if enabled != previous {
        settings.disabled.retain(|a| *a != kind);
        if !enabled {
            settings.disabled.push(kind);
        }
    }
    let mask = match kind {
        Adjustment::Blemishes => Some("Full strength can remove freckles and moles"),
        Adjustment::BackgroundBlur => Some("Paint the background or run segmentation"),
        _ => None,
    };
    if let Some(text) = mask {
        ui.label(RichText::new(text).size(10.0).color(MUTED));
    }
}

fn slider(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    tooltip: &str,
) {
    slider_control(ui, label, value, range, tooltip, None);
}
fn slider_control(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    tooltip: &str,
    mut enabled: Option<&mut bool>,
) {
    ui.push_id(label, |ui| {
        let min = *range.start();
        let max = *range.end();
        ui.horizontal(|ui| {
            if let Some(active) = enabled.as_deref_mut() {
                let check = ui.checkbox(active, "");
                hit(ui, &format!("enable-{label}"), check.rect);
                check.on_hover_text("Enable this adjustment. Its value is kept while disabled.");
            }
            ui.label(RichText::new(label).size(12.0).color(MUTED));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let numeric = ui.add_enabled(
                    enabled.as_deref().copied().unwrap_or(true),
                    egui::DragValue::new(value)
                        .range(range.clone())
                        .speed(if max <= 2.0 { 0.01 } else { 1.0 })
                        .max_decimals(if max <= 2.0 { 2 } else { 0 }),
                );
                hit(ui, &format!("value-{label}"), numeric.rect);
            });
        });
        let active = enabled.as_deref().copied().unwrap_or(true);
        let (rect, mut response) = ui
            .add_enabled_ui(active, |ui| {
                ui.allocate_exact_size(vec2(ui.available_width(), 18.0), Sense::click_and_drag())
            })
            .inner;
        let track = Rect::from_min_max(
            pos2(rect.left() + 5.0, rect.center().y - 1.5),
            pos2(rect.right() - 5.0, rect.center().y + 1.5),
        );
        hit(ui, &format!("slider-{label}"), rect);
        if response.double_clicked() {
            *value = match label {
                "Blemish removal" => 0.0,
                "Healing strength" | "Brush strength" | "HSL amount" | "Curves amount" => 100.0,
                "Blur strength" => 35.0,
                "JPEG quality" => 95.0,
                "Brush size" => 18.0,
                "Brush softness" => 75.0,
                _ => 0.0,
            };
            response.mark_changed();
        } else if (response.clicked() || response.dragged())
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
        if active && response.has_focus() {
            let direction = ui.input(|i| {
                i.key_pressed(egui::Key::ArrowRight) as i32
                    - i.key_pressed(egui::Key::ArrowLeft) as i32
            });
            if direction != 0 {
                *value = (*value + direction as f32 * if max <= 2.0 { 0.01 } else { 1.0 })
                    .clamp(min, max);
            }
        }
        let accent = if active { YELLOW } else { MUTED };
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
                accent,
            );
        } else {
            ui.painter().rect_filled(
                Rect::from_min_max(track.min, pos2(x, track.bottom())),
                2.0,
                accent,
            );
        }
        ui.painter().circle_filled(
            pos2(x, track.center().y),
            8.0,
            Color32::from_rgba_unmultiplied(255, 214, 10, 18),
        );
        ui.painter()
            .circle_filled(pos2(x, track.center().y), 4.5, accent);
        response.on_hover_text(format!("{tooltip}\nDouble-click the track to reset."));
        ui.add_space(10.0);
    });
}

fn curve_editor(
    ui: &mut egui::Ui,
    points: &mut [f32; 9],
    channel: usize,
    drag_knot: &mut Option<usize>,
) {
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), 206.0), Sense::click_and_drag());
    hit(ui, "curve-graph", rect);
    let graph = rect.shrink(12.0);
    let pos = |x: f32, y: f32| {
        pos2(
            graph.left() + x * graph.width(),
            graph.bottom() - y * graph.height(),
        )
    };
    if response.double_clicked() {
        *points = crate::color::IDENTITY_CURVE;
        *drag_knot = None;
    } else {
        if (response.drag_started() || response.clicked())
            && let Some(pointer) = response.interact_pointer_pos()
        {
            // Fixed horizontal anchors make selecting and dragging a point predictable.
            *drag_knot = Some(
                (((pointer.x - graph.left()) / graph.width()).clamp(0.0, 1.0) * 8.0).round()
                    as usize,
            );
        }
        if (response.dragged() || response.clicked())
            && let Some(pointer) = response.interact_pointer_pos()
            && let Some(index) = *drag_knot
        {
            points[index] = ((graph.bottom() - pointer.y) / graph.height()).clamp(0.0, 1.0);
        }
        if response.drag_stopped() {
            *drag_knot = None;
        }
    }
    let painter = ui.painter();
    painter.rect_filled(rect, 8.0, BG);
    for i in 0..=4 {
        let t = i as f32 / 4.0;
        painter.line_segment([pos(t, 0.0), pos(t, 1.0)], Line::new(1.0_f32, BORDER));
        painter.line_segment([pos(0.0, t), pos(1.0, t)], Line::new(1.0_f32, BORDER));
    }
    painter.line_segment(
        [pos(0.0, 0.0), pos(1.0, 1.0)],
        Line::new(1.0_f32, MUTED.gamma_multiply(0.5)),
    );
    let color = [
        YELLOW,
        Color32::from_rgb(255, 115, 115),
        Color32::from_rgb(120, 230, 140),
        Color32::from_rgb(120, 165, 255),
    ][channel];
    let path: Vec<Pos2> = (0..=128)
        .map(|i| {
            let x = i as f32 / 128.0;
            pos(x, crate::color::curve_value(points, x))
        })
        .collect();
    painter.add(egui::Shape::line(path, Line::new(2.0_f32, color)));
    for (i, y) in points.iter().enumerate() {
        let knot = pos(i as f32 / 8.0, *y);
        painter.circle_filled(knot, 6.0, color);
        painter.circle_stroke(knot, 6.0, Line::new(1.5_f32, BG));
    }
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

fn needs_detail_resolution(photo: &Photo, scale: f32) -> bool {
    let original = photo.original.dimensions();
    let preview = photo.preview.dimensions();
    if original == preview {
        return false;
    }

    // `View::scale` is screen pixels per source pixel. Switch to full-resolution
    // rendering as soon as the preview would need to be enlarged on screen.
    let preview_scale =
        (preview.0 as f32 / original.0 as f32).min(preview.1 as f32 / original.1 as f32);
    scale > preview_scale
}

fn sharp_region_for_photo(
    detail: &Option<(u64, u64, engine::Crop, Arc<RgbaImage>)>,
    photo_id: u64,
) -> Option<&(u64, u64, engine::Crop, Arc<RgbaImage>)> {
    detail.as_ref().filter(|(id, _, _, _)| *id == photo_id)
}

fn point_in_polygon_uv(point: [f32; 2], boundary: &[[f32; 2]]) -> bool {
    if boundary.len() < 3 {
        return false;
    }
    let mut inside = false;
    let mut previous = boundary.len() - 1;
    for current in 0..boundary.len() {
        let a = boundary[current];
        let b = boundary[previous];
        if (a[1] > point[1]) != (b[1] > point[1])
            && point[0] < (b[0] - a[0]) * (point[1] - a[1]) / (b[1] - a[1]) + a[0]
        {
            inside = !inside;
        }
        previous = current;
    }
    inside
}

fn polygon_area_uv(boundary: &[[f32; 2]]) -> f32 {
    if boundary.len() < 3 {
        return 0.0;
    }
    let twice_area: f32 = (0..boundary.len())
        .map(|i| {
            let a = boundary[i];
            let b = boundary[(i + 1) % boundary.len()];
            a[0] * b[1] - b[0] * a[1]
        })
        .sum();
    twice_area.abs() * 0.5
}

fn crop_contains(outer: engine::Crop, inner: engine::Crop) -> bool {
    inner.x >= outer.x
        && inner.y >= outer.y
        && inner.x + inner.width <= outer.x + outer.width
        && inner.y + inner.height <= outer.y + outer.height
}

fn intersect_crop(a: engine::Crop, b: engine::Crop) -> engine::Crop {
    let x = a.x.max(b.x);
    let y = a.y.max(b.y);
    engine::Crop {
        x,
        y,
        width: (a.x + a.width).min(b.x + b.width).saturating_sub(x),
        height: (a.y + a.height).min(b.y + b.height).saturating_sub(y),
    }
}

fn visible_source_crop(image: Rect, viewport: Rect, dimensions: (u32, u32)) -> engine::Crop {
    let visible = image.intersect(viewport);
    let x = (((visible.left() - image.left()) / image.width()) * dimensions.0 as f32)
        .floor()
        .clamp(0.0, dimensions.0.saturating_sub(1) as f32) as u32;
    let y = (((visible.top() - image.top()) / image.height()) * dimensions.1 as f32)
        .floor()
        .clamp(0.0, dimensions.1.saturating_sub(1) as f32) as u32;
    let right = (((visible.right() - image.left()) / image.width()) * dimensions.0 as f32)
        .ceil()
        .clamp(x as f32 + 1.0, dimensions.0 as f32) as u32;
    let bottom = (((visible.bottom() - image.top()) / image.height()) * dimensions.1 as f32)
        .ceil()
        .clamp(y as f32 + 1.0, dimensions.1 as f32) as u32;
    engine::Crop {
        x,
        y,
        width: right - x,
        height: bottom - y,
    }
}
fn padded_crop(crop: engine::Crop, dims: (u32, u32)) -> engine::Crop {
    let x = crop.x.saturating_sub(128);
    let y = crop.y.saturating_sub(128);
    let right = (crop.x + crop.width + 128).min(dims.0);
    let bottom = (crop.y + crop.height + 128).min(dims.1);
    engine::Crop {
        x,
        y,
        width: right - x,
        height: bottom - y,
    }
}
fn shape_values(s: &Settings) -> [f32; 4] {
    let s = s.effective();
    [s.eye_size, s.nose_width, s.lip_plumpness, s.jawline]
}
fn same_seg(a: Option<&Arc<Segmentation>>, b: Option<&Arc<Segmentation>>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => Arc::ptr_eq(a, b),
        _ => false,
    }
}
fn display_image(image: &RgbaImage, gpu: bool) -> std::borrow::Cow<'_, RgbaImage> {
    if gpu {
        std::borrow::Cow::Owned(image::imageops::thumbnail(image, 320, 320))
    } else {
        std::borrow::Cow::Borrowed(image)
    }
}
fn photo_texture(
    ctx: &egui::Context,
    name: &str,
    image: &RgbaImage,
    gpu: bool,
) -> egui::TextureHandle {
    texture(ctx, name, &display_image(image, gpu))
}

// Native/Patch canvas textures are used only by the non-GPU painter. The
// custom GPU callback consumes the sharp Arc<RgbaImage> directly. A one-pixel
// handle keeps the UI bookkeeping without scanning a native frame for an
// unused thumbnail. Filmstrip and ordinary overview thumbnails stay unchanged.
fn canvas_texture(
    ctx: &egui::Context,
    name: &str,
    image: &RgbaImage,
    gpu: bool,
) -> egui::TextureHandle {
    if gpu {
        texture(
            ctx,
            name,
            &RgbaImage::from_pixel(1, 1, image::Rgba([0, 0, 0, 0])),
        )
    } else {
        texture(ctx, name, image)
    }
}
