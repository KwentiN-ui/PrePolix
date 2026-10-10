//! Tables that lay out only their visible rows, for result lists with thousands of entries.

use egui::{Align2, Sense, TextStyle, Ui, vec2};

/// Gap between two columns.
const COLUMN_GAP: f32 = 16.0;
/// Space above and below the text of a row.
const ROW_PADDING: f32 = 2.0;

/// A striped table with a fixed header. The rows are painted only where the scroll area shows
/// them, so a table of 10000 rows costs as much per frame as one of 20. Every column is as wide
/// as its header or `sample`, the widest text a cell is expected to hold.
pub fn show(
    ui: &mut Ui,
    id: &str,
    headers: &[&str],
    sample: &str,
    rows: usize,
    max_height: f32,
    cell: impl Fn(usize, usize) -> String,
) {
    let font = TextStyle::Body.resolve(ui.style());
    let visuals = ui.visuals().clone();
    let width = |ui: &Ui, text: &str| {
        let galley =
            ui.fonts_mut(|f| f.layout_no_wrap(text.into(), font.clone(), visuals.text_color()));
        galley.size().x
    };
    let sample_width = width(ui, sample);
    let widths: Vec<f32> = (headers.iter())
        .map(|h| width(ui, h).max(sample_width) + COLUMN_GAP)
        .collect();
    let total = widths.iter().sum::<f32>();
    let row_height = ui.fonts_mut(|f| f.row_height(&font)) + 2.0 * ROW_PADDING;
    let paint_row = |ui: &mut Ui, texts: &mut dyn Iterator<Item = String>, color, stripe: bool| {
        let (rect, _) = ui.allocate_exact_size(vec2(total, row_height), Sense::hover());
        if !ui.is_rect_visible(rect) {
            return;
        }
        let painter = ui.painter();
        if stripe {
            painter.rect_filled(rect, 0.0, visuals.faint_bg_color);
        }
        let mut x = rect.left();
        for (text, w) in texts.zip(&widths) {
            painter.text(
                egui::pos2(x, rect.center().y),
                Align2::LEFT_CENTER,
                text,
                font.clone(),
                color,
            );
            x += w;
        }
    };
    ui.push_id(id, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        let strong = visuals.strong_text_color();
        paint_row(
            ui,
            &mut headers.iter().map(|h| h.to_string()),
            strong,
            false,
        );
        egui::ScrollArea::vertical()
            .max_height(max_height)
            .auto_shrink([false, true])
            .show_rows(ui, row_height, rows, |ui, range| {
                for row in range {
                    let mut texts = (0..headers.len()).map(|column| cell(row, column));
                    paint_row(ui, &mut texts, visuals.text_color(), row % 2 == 1);
                }
            });
    });
}

/// Monospace text lines in `area`, laid out only where they are visible. The solver output of a
/// long run has many thousands of lines.
pub fn lines(ui: &mut Ui, area: egui::ScrollArea, lines: &[String]) {
    let font = TextStyle::Monospace.resolve(ui.style());
    let row_height = ui.fonts_mut(|f| f.row_height(&font));
    ui.scope(|ui| {
        // show_rows takes the row pitch from this spacing.
        ui.spacing_mut().item_spacing.y = 0.0;
        area.show_rows(ui, row_height, lines.len(), |ui, rows| {
            for line in &lines[rows] {
                ui.add(egui::Label::new(egui::RichText::new(line).font(font.clone())).extend());
            }
        });
    });
}
