// Hide the console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod auth;
mod codegen;
mod collection;
mod crypto;
mod data;
mod export;
mod features;
mod graphql;
mod http;
mod import;
mod lens;
mod mock;
mod preview;
mod rtl;
mod script;
mod shot;
mod tests;
mod theme;
mod ui_dialogs;
mod ui_request;
mod ui_sidebar;
mod ws;
mod yaml;

use std::sync::Arc;

use eframe::egui;

/// The window / taskbar icon, rendered from `assets/icon.svg`.
fn window_icon() -> egui::IconData {
    eframe::icon_data::from_png_bytes(include_bytes!("../assets/window-icon.png"))
        .unwrap_or_default()
}

fn main() -> eframe::Result {
    let preload = std::env::args().nth(1).map(std::path::PathBuf::from);
    let screenshot = std::env::var_os("GAZA_SHOT").is_some();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Gaza Man")
            .with_inner_size(if screenshot {
                [1440.0, 900.0]
            } else {
                [1360.0, 860.0]
            })
            .with_min_inner_size([900.0, 560.0])
            .with_drag_and_drop(true)
            .with_icon(Arc::new(window_icon())),
        // Screenshot runs use a throwaway workspace; keep them from saving window state too.
        persist_window: !screenshot,
        ..Default::default()
    };

    eframe::run_native(
        "Gaza Man",
        options,
        Box::new(move |cc| Ok(Box::new(app::GazaMan::new(cc, preload)))),
    )
}
