#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui;
use hastur_retouch::{branding, ui::HasturApp};

fn main() -> eframe::Result {
    let arguments: Vec<_> = std::env::args().collect();
    let size = arguments
        .iter()
        .position(|a| a == "--size")
        .and_then(|i| arguments.get(i + 1))
        .and_then(|s| s.split_once('x'))
        .and_then(|(w, h)| Some([w.parse::<f32>().ok()?, h.parse::<f32>().ok()?]))
        .unwrap_or([1440.0, 940.0]);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Hastur Retouch")
            .with_icon(branding::window_icon())
            .with_inner_size(size)
            .with_min_inner_size([600.0, 560.0]),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    eframe::run_native(
        "Hastur Retouch",
        options,
        Box::new(|cc| Ok(Box::new(HasturApp::new(cc)))),
    )
}
