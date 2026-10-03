//! Embedded Hastur Retouch brand assets shared by the window and startup UI.

use eframe::egui::{self, Color32, ColorImage, IconData, Painter, Rect, Shape, pos2};

pub const GOLD: Color32 = Color32::from_rgb(242, 197, 38);
pub const IVORY: Color32 = Color32::from_rgb(244, 242, 233);

// Same 512-unit geometry as assets/branding/hastur-mark.svg. The concave crown
// is triangulated explicitly; convex_polygon would incorrectly fill its gaps.
const CROWN: [[f32; 2]; 11] = [
    [42.0, 131.0],
    [152.0, 311.0],
    [144.0, 202.0],
    [213.0, 325.0],
    [256.0, 36.0],
    [299.0, 325.0],
    [368.0, 202.0],
    [360.0, 311.0],
    [470.0, 131.0],
    [384.0, 420.0],
    [128.0, 420.0],
];
const TRIANGLES: [u32; 27] = [
    10, 0, 1, 1, 2, 3, 10, 1, 3, 3, 4, 5, 10, 3, 5, 5, 6, 7, 10, 5, 7, 7, 8, 9, 7, 9, 10,
];
const BASE: [[f32; 2]; 4] = [
    [128.0, 438.0],
    [384.0, 438.0],
    [392.0, 462.0],
    [120.0, 462.0],
];

/// Paint the five-spire crown, centered in `rect` without stretching it.
/// The detached base uses Hastur gold; `crown_color` supports light/dark surfaces.
pub fn paint_logo(painter: &Painter, rect: Rect, crown_color: Color32) {
    let side = rect.width().min(rect.height()).max(0.0);
    let origin = rect.center() - egui::vec2(side, side) * 0.5;
    let project = |point: [f32; 2]| {
        pos2(
            origin.x + point[0] * side / 512.0,
            origin.y + point[1] * side / 512.0,
        )
    };
    let mut mesh = egui::Mesh::default();
    for point in CROWN {
        mesh.colored_vertex(project(point), crown_color);
    }
    mesh.indices.extend_from_slice(&TRIANGLES);
    painter.add(Shape::mesh(mesh));
    painter.add(Shape::convex_polygon(
        BASE.into_iter().map(project).collect(),
        GOLD,
        egui::Stroke::NONE,
    ));
}

/// The application icon, with an opaque ink backing for taskbar contrast.
pub fn window_icon() -> IconData {
    let rgba = image::load_from_memory(include_bytes!("../assets/branding/hastur-icon.png"))
        .expect("embedded Hastur icon must be a valid PNG")
        .into_rgba8();
    let (width, height) = rgba.dimensions();
    IconData {
        rgba: rgba.into_raw(),
        width,
        height,
    }
}

/// The user's chosen full demo portrait, embedded at its original dimensions.
/// Load once into a texture rather than decoding this PNG on every frame.
pub fn splash_portrait() -> ColorImage {
    let rgba = image::load_from_memory(include_bytes!("../assets/demo-portrait.png"))
        .expect("embedded splash portrait must be a valid PNG")
        .into_rgba8();
    ColorImage::from_rgba_unmultiplied(
        [rgba.width() as usize, rgba.height() as usize],
        rgba.as_raw(),
    )
}
