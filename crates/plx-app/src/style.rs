//! Look of the application: a light theme after PrePoMax's classic Windows controls.
//!
//! PrePoMax uses Segoe UI, which may not be redistributed. Noto Sans (SIL Open Font License,
//! `assets/fonts/OFL.txt`) is a free face of the same humanist style; the bundled file is
//! subset to Latin, Greek and common symbols.

use std::sync::Arc;

use egui::{Color32, CornerRadius, Stroke, Theme, Visuals};

const NOTO_SANS: &[u8] = include_bytes!("../assets/fonts/NotoSans-Regular.ttf");

/// Windows "Control" colour: panels, menu and tool bars.
pub const CONTROL: Color32 = Color32::from_rgb(240, 240, 240);
/// Windows "Window" colour: tree, lists and text fields.
pub const WINDOW: Color32 = Color32::WHITE;
/// Windows highlight for selected items.
pub const HIGHLIGHT: Color32 = Color32::from_rgb(0, 120, 215);
pub const BORDER: Color32 = Color32::from_rgb(173, 173, 173);
const TEXT: Color32 = Color32::from_rgb(0, 0, 0);
pub const HOVER_FILL: Color32 = Color32::from_rgb(229, 241, 251);
pub const PRESSED_FILL: Color32 = Color32::from_rgb(204, 228, 247);

/// Minimum height of buttons, selectable labels and combo entries: text row plus padding and
/// frame stroke, rounded up (checked by a test).
const INTERACT_HEIGHT: f32 = 22.0;

pub fn apply(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "Noto Sans".into(),
        Arc::new(egui::FontData::from_static(NOTO_SANS)),
    );
    fonts
        .families
        .entry(egui::FontFamily::Proportional)
        .or_default()
        .insert(0, "Noto Sans".into());
    ctx.set_fonts(fonts);

    ctx.set_theme(Theme::Light);
    ctx.set_visuals_of(Theme::Light, visuals());
    ctx.style_mut_of(Theme::Light, |style| {
        style.spacing.item_spacing = egui::vec2(6.0, 3.0);
        style.spacing.button_padding = egui::vec2(6.0, 2.0);
        // egui lays out a selectable label without frame (unselected, not hovered) with its
        // padding reduced by the frame stroke, so hovering or selecting it grows the row by two
        // pixels and moves everything below. A minimum height that covers the framed size keeps
        // all states of buttons, selectables and combo entries equally tall.
        style.spacing.interact_size.y = INTERACT_HEIGHT;
        style.spacing.indent = 16.0;
        for (text_style, size) in [
            (egui::TextStyle::Body, 13.0),
            (egui::TextStyle::Button, 13.0),
            (egui::TextStyle::Small, 11.0),
            (egui::TextStyle::Heading, 16.0),
            (egui::TextStyle::Monospace, 12.0),
        ] {
            if let Some(font) = style.text_styles.get_mut(&text_style) {
                font.size = size;
            }
        }
    });
}

fn visuals() -> Visuals {
    let mut v = Visuals::light();
    v.override_text_color = Some(TEXT);
    v.panel_fill = CONTROL;
    v.window_fill = CONTROL;
    v.window_stroke = Stroke::new(1.0, BORDER);
    v.window_corner_radius = CornerRadius::ZERO;
    v.menu_corner_radius = CornerRadius::ZERO;
    v.extreme_bg_color = WINDOW;
    v.faint_bg_color = Color32::from_rgb(246, 246, 246);
    v.selection.bg_fill = HIGHLIGHT;
    v.selection.stroke = Stroke::new(1.0, Color32::WHITE);
    v.indent_has_left_vline = true;

    let w = &mut v.widgets;
    w.noninteractive.bg_fill = CONTROL;
    w.noninteractive.weak_bg_fill = CONTROL;
    w.noninteractive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(210, 210, 210));
    w.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
    w.inactive.bg_fill = Color32::from_rgb(225, 225, 225);
    w.inactive.weak_bg_fill = Color32::from_rgb(225, 225, 225);
    w.inactive.bg_stroke = Stroke::new(1.0, BORDER);
    w.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    w.hovered.bg_fill = HOVER_FILL;
    w.hovered.weak_bg_fill = HOVER_FILL;
    w.hovered.bg_stroke = Stroke::new(1.0, HIGHLIGHT);
    w.hovered.fg_stroke = Stroke::new(1.0, TEXT);
    w.hovered.expansion = 0.0;
    w.active.bg_fill = PRESSED_FILL;
    w.active.weak_bg_fill = PRESSED_FILL;
    w.active.bg_stroke = Stroke::new(1.0, Color32::from_rgb(0, 84, 153));
    w.active.fg_stroke = Stroke::new(1.0, TEXT);
    w.active.expansion = 0.0;
    w.open.bg_fill = PRESSED_FILL;
    w.open.weak_bg_fill = PRESSED_FILL;
    w.open.bg_stroke = Stroke::new(1.0, HIGHLIGHT);
    w.open.fg_stroke = Stroke::new(1.0, TEXT);
    for state in [
        &mut w.noninteractive,
        &mut w.inactive,
        &mut w.hovered,
        &mut w.active,
        &mut w.open,
    ] {
        state.corner_radius = CornerRadius::ZERO;
    }
    v
}

#[cfg(test)]
mod tests {
    /// Heights of an unselected and a selected label; the selected one is framed like a hovered
    /// one.
    fn selectable_heights(ctx: &egui::Context) -> (f32, f32, f32) {
        let mut heights = (0.0, 0.0, 0.0);
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            heights.0 = ui.selectable_label(false, "1, 4").rect.height();
            heights.1 = ui.selectable_label(true, "1, 4").rect.height();
            heights.2 = ui.button("1, 4").rect.height();
        });
        output.textures_delta.clear();
        heights
    }

    #[test]
    fn hover_frame_keeps_row_height() {
        let ctx = egui::Context::default();
        super::apply(&ctx);
        // Fonts are loaded during the first pass.
        selectable_heights(&ctx);
        let (plain, framed, button) = selectable_heights(&ctx);
        assert_eq!(plain, framed, "selectable label grows when framed");
        assert_eq!(plain, button);
        assert_eq!(
            plain,
            super::INTERACT_HEIGHT,
            "INTERACT_HEIGHT no longer covers the text"
        );
    }
}
