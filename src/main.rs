#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod core;
mod ffmpeg;
mod project;

fn main() -> eframe::Result<()> {
    let icon = image::load_from_memory(include_bytes!("../assets/app_icon.png"))
        .expect("內附 App 圖示無法載入")
        .into_rgba8();
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1360.0, 900.0])
            .with_min_inner_size([1080.0, 720.0])
            .with_icon(eframe::egui::IconData {
                width: icon.width(),
                height: icon.height(),
                rgba: icon.into_raw(),
            }),
        ..Default::default()
    };

    eframe::run_native(
        "透明字卡工作室",
        options,
        Box::new(|context| Ok(Box::new(app::StudioApp::new(context)))),
    )
}
