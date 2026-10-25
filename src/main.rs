#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod action;
mod app;
mod config;
mod db;
mod devscript;
mod export;
mod sql;
mod storage;
mod tabs;
mod ui;
mod util;

/// Process start, for `TRUNQ_BENCH=startup` (time to first frame).
pub static STARTED: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();

fn main() -> eframe::Result {
    STARTED.get_or_init(std::time::Instant::now);
    // macOS expects a rounded-square app icon; other platforms use the plain mark.
    let icon_png: &[u8] = if cfg!(target_os = "macos") { include_bytes!("../assets/app_icon_256.png") } else { include_bytes!("../assets/logo_256.png") };
    let icon = eframe::icon_data::from_png_bytes(icon_png).ok();
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Trunq")
        .with_app_id("dev.trunq.Trunq")
        .with_inner_size([1440.0, 900.0])
        .with_min_inner_size([760.0, 480.0]);
    if cfg!(target_os = "macos") {
        // Draw our own top bar behind the transparent title bar; the traffic
        // light buttons sit inside it like in native Mac apps.
        // Trunq draws its own window controls inside the top bar so they line
        // up exactly with the rest of the toolbar.
        viewport = viewport.with_fullsize_content_view(true).with_titlebar_shown(false).with_title_shown(false).with_titlebar_buttons_shown(false);
    }
    if let Some(icon) = icon {
        viewport = viewport.with_icon(icon);
    }
    // Returning through AppKit's event loop can crash while unregistering
    // Touch Bar observers after window teardown (egui issue #2768). This is a
    // standalone application: let eframe save/destroy it and exit on macOS.
    let options = eframe::NativeOptions { viewport, run_and_return: !cfg!(target_os = "macos"), ..Default::default() };
    eframe::run_native("Trunq", options, Box::new(|cc| Ok(Box::new(app::App::new(cc)))))
}
