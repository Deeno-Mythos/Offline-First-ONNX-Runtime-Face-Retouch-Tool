#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use astra_retouch::ui::AstraApp;
use eframe::egui;

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
            .with_title("Astra Retouch")
            .with_inner_size(size)
            .with_min_inner_size([600.0, 560.0]),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    eframe::run_native(
        "Astra Retouch",
        options,
        Box::new(|cc| Ok(Box::new(AstraApp::new(cc)))),
    )
}
