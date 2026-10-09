#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod analysis;
mod animation;
mod app;
mod icons;
mod keywords;
mod model;
mod overlay;
mod properties;
mod results;
mod settings;
mod setup;
mod solver_check;
mod style;
mod tree;
mod viewport;

fn main() -> eframe::Result {
    env_logger::init();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("prepolix")
            .with_app_id("prepolix")
            .with_inner_size([1400.0, 900.0])
            .with_min_inner_size([640.0, 400.0]),
        renderer: eframe::Renderer::Wgpu,
        persist_window: true,
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
    /// The bundled Noto Sans subset and egui's fallback fonts lack emoji and many symbols,
    /// which then show up as empty boxes. The GUI sources stick to Latin-1 plus a few checked
    /// typographic signs.
    #[test]
    fn gui_sources_only_use_glyphs_of_the_default_font() {
        let sources = [
            ("analysis.rs", include_str!("analysis.rs")),
            ("animation.rs", include_str!("animation.rs")),
            ("app.rs", include_str!("app.rs")),
            ("icons.rs", include_str!("icons.rs")),
            ("keywords.rs", include_str!("keywords.rs")),
            ("main.rs", include_str!("main.rs")),
            ("model.rs", include_str!("model.rs")),
            ("overlay.rs", include_str!("overlay.rs")),
            ("properties.rs", include_str!("properties.rs")),
            ("results.rs", include_str!("results.rs")),
            ("settings.rs", include_str!("settings.rs")),
            ("setup.rs", include_str!("setup.rs")),
            ("solver_check.rs", include_str!("solver_check.rs")),
            ("style.rs", include_str!("style.rs")),
            ("tree.rs", include_str!("tree.rs")),
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
