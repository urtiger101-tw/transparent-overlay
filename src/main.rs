#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod core;
mod ffmpeg;
mod project;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1360.0, 900.0])
            .with_min_inner_size([1080.0, 720.0]),
        ..Default::default()
    };

    eframe::run_native(
        "透明字卡工作室",
        options,
        Box::new(|context| Ok(Box::new(app::StudioApp::new(context)))),
    )
}
