#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod analysis;
mod animation;
mod app;
mod cad_selection;
mod constraint_dialog;
mod contact_search;
mod contacts;
mod exploded;
mod features;
mod field_output_dialog;
mod gizmo;
mod history_output_dialog;
mod history_table;
mod hot_spots;
mod icons;
mod keywords;
mod material_library;
mod meshing;
mod model;
mod model_properties;
mod numeric;
mod overlay;
mod properties;
mod results;
mod screenshot;
mod section;
mod selection;
mod settings;
mod setup;
mod solver_check;
mod sound;
mod style;
mod symbols;
mod transformation_dialog;
mod tree;
mod tree_icons;
mod viewport;

fn main() -> eframe::Result {
    env_logger::init();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("prepolix")
            .with_app_id("prepolix")
            .with_inner_size([1400.0, 900.0])
            .with_min_inner_size([640.0, 400.0])
            .with_icon(window_icon()),
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

/// The program icon for the window and the task bar, drawn by `scripts/make_icon.py`.
fn window_icon() -> egui::IconData {
    let decoder = png::Decoder::new(std::io::Cursor::new(include_bytes!(
        "../assets/icon/prepolix.png"
    )));
    let mut reader = decoder.read_info().expect("Icon ist ein gültiges PNG");
    let mut rgba = vec![
        0;
        reader
            .output_buffer_size()
            .expect("Icon passt in den Speicher")
    ];
    let info = reader
        .next_frame(&mut rgba)
        .expect("Icon ist ein gültiges PNG");
    assert_eq!(
        info.color_type,
        png::ColorType::Rgba,
        "Icon braucht einen Alphakanal"
    );
    rgba.truncate(info.buffer_size());
    egui::IconData {
        rgba,
        width: info.width,
        height: info.height,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn window_icon_decodes() {
        let icon = super::window_icon();
        assert_eq!((icon.width, icon.height), (256, 256));
        assert_eq!(icon.rgba.len(), 256 * 256 * 4);
    }

    /// The bundled Noto Sans subset and egui's fallback fonts lack emoji and many symbols,
    /// which then show up as empty boxes. The GUI sources stick to Latin-1 plus a few checked
    /// typographic signs.
    #[test]
    fn gui_sources_only_use_glyphs_of_the_default_font() {
        let sources = [
            ("analysis.rs", include_str!("analysis.rs")),
            ("animation.rs", include_str!("animation.rs")),
            ("app.rs", include_str!("app.rs")),
            ("contact_search.rs", include_str!("contact_search.rs")),
            ("contacts.rs", include_str!("contacts.rs")),
            (
                "field_output_dialog.rs",
                include_str!("field_output_dialog.rs"),
            ),
            ("gizmo.rs", include_str!("gizmo.rs")),
            (
                "history_output_dialog.rs",
                include_str!("history_output_dialog.rs"),
            ),
            ("history_table.rs", include_str!("history_table.rs")),
            ("icons.rs", include_str!("icons.rs")),
            ("keywords.rs", include_str!("keywords.rs")),
            ("material_library.rs", include_str!("material_library.rs")),
            ("main.rs", include_str!("main.rs")),
            ("model.rs", include_str!("model.rs")),
            ("model_properties.rs", include_str!("model_properties.rs")),
            ("overlay.rs", include_str!("overlay.rs")),
            ("properties.rs", include_str!("properties.rs")),
            ("results.rs", include_str!("results.rs")),
            ("selection.rs", include_str!("selection.rs")),
            ("settings.rs", include_str!("settings.rs")),
            ("setup.rs", include_str!("setup.rs")),
            ("solver_check.rs", include_str!("solver_check.rs")),
            ("style.rs", include_str!("style.rs")),
            ("tree.rs", include_str!("tree.rs")),
            ("tree_icons.rs", include_str!("tree_icons.rs")),
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
