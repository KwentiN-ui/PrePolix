#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod model;
mod results;
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

#[cfg(test)]
mod tests {
    /// egui's default proportional font lacks most symbols and emoji, which then show up as
    /// empty boxes. The GUI sources stick to Latin-1 plus a few checked typographic signs.
    #[test]
    fn gui_sources_only_use_glyphs_of_the_default_font() {
        let sources = [
            ("app.rs", include_str!("app.rs")),
            ("main.rs", include_str!("main.rs")),
            ("model.rs", include_str!("model.rs")),
            ("results.rs", include_str!("results.rs")),
            ("viewport.rs", include_str!("viewport.rs")),
        ];
        for (file, text) in sources {
            for (line, content) in text.lines().enumerate() {
                for c in content.chars() {
                    let ok = (c as u32) < 0x100 || "…–".contains(c);
                    assert!(ok, "{file}:{}: Zeichen {c:?} fehlt im egui-Font", line + 1);
                }
            }
        }
    }
}
