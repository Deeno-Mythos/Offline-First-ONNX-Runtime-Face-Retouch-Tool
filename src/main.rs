#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui;
use hastur_retouch::{branding, ui::HasturApp};

fn main() -> eframe::Result {
    let arguments: Vec<_> = std::env::args().collect();
    if let Some(index) = arguments.iter().position(|a| a == "--verify-installation") {
        let Some(output) = arguments.get(index + 1) else {
            std::process::exit(2)
        };
        let provider = arguments
            .iter()
            .position(|a| a == "--verify-provider")
            .and_then(|i| arguments.get(i + 1))
            .map(String::as_str)
            .unwrap_or("cpu");
        let provider = match provider {
            "cpu" => hastur_retouch::model::Provider::Cpu,
            "auto" => hastur_retouch::model::Provider::Auto,
            "burn" => hastur_retouch::model::Provider::Burn,
            _ => std::process::exit(2),
        };
        let result =
            hastur_retouch::installation_check::run(std::path::Path::new(output), provider);
        std::process::exit(if result.is_ok() { 0 } else { 1 });
    }
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
