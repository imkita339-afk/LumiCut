#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
// egui принимает толщину линий как `impl Into<f32>`, поэтому литералы вида `1.0` — это f32.
#![allow(float_literal_f32_fallback)]

mod app;
mod audio;
mod effects;
mod export;
mod ffmpeg;
mod icons;
mod keys;
mod logo;
mod look;
mod model;
mod motion;
mod playback;
mod srt;
mod text;
mod theme;
mod transition;
mod ui;
mod vfx;

use eframe::egui;

/// Сторона значка окна; до нужных системе размеров его уменьшает сама библиотека окна.
const ICON_SIDE: u32 = 256;

fn main() -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let icon = egui::IconData { rgba: logo::rgba(ICON_SIDE), width: ICON_SIDE, height: ICON_SIDE };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("LumiCut")
            .with_icon(icon)
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([1100.0, 680.0])
            .with_maximized(true)
            .with_drag_and_drop(true),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    eframe::run_native(
        "LumiCut",
        options,
        Box::new(move |cc| Ok(Box::new(app::App::new(&cc.egui_ctx, args)))),
    )
}
