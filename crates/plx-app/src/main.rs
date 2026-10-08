#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod viewport;

fn main() -> eframe::Result {
    env_logger::init();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("prepolix")
            .with_inner_size([1400.0, 900.0])
            .with_min_inner_size([640.0, 400.0]),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    eframe::run_native(
        "prepolix",
        options,
        Box::new(|cc| Ok(Box::new(app::PrepolixApp::new(cc)?))),
    )
}
