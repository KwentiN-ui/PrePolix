//! Colour spectrum for result contours, following PrePoMax's defaults: a discrete rainbow
//! from blue (minimum) to red (maximum), lightened by 20 %.

/// Number of colour bands PrePoMax uses by default.
pub const DEFAULT_LEVELS: u32 = 9;
/// Most bands the renderer supports.
pub const MAX_LEVELS: u32 = 24;
/// Normalized value written for nodes without a result; drawn grey.
pub const NO_VALUE: f32 = -1.0;

const BRIGHTNESS: f32 = 0.2;

/// sRGB colour of band `index` out of `levels`, as shown in the legend.
pub fn band_color(index: u32, levels: u32) -> [f32; 3] {
    let levels = levels.clamp(2, MAX_LEVELS);
    let t = index.min(levels - 1) as f32 / (levels - 1) as f32;
    // Control points blue, cyan, green, yellow, red share saturation and value after
    // lightening, so interpolating in HSV is a plain hue sweep from 240° to 0°.
    hsv_to_rgb(240.0 * (1.0 - t), 1.0 - BRIGHTNESS, 1.0)
}

/// Linear RGB colours of all bands, for the shader.
pub fn band_colors_linear(levels: u32) -> Vec<[f32; 3]> {
    (0..levels.clamp(2, MAX_LEVELS))
        .map(|i| band_color(i, levels).map(srgb_to_linear))
        .collect()
}

/// Maps result values to the normalized range used for colouring: 0 at `min`, 1 at `max`,
/// [`NO_VALUE`] where there is no finite value.
pub fn normalize(values: &[f32], min: f32, max: f32) -> Vec<f32> {
    let span = max - min;
    values
        .iter()
        .map(|&v| {
            if !v.is_finite() {
                NO_VALUE
            } else if span > 0.0 {
                ((v - min) / span).clamp(0.0, 1.0)
            } else {
                0.5
            }
        })
        .collect()
}

fn hsv_to_rgb(hue_deg: f32, saturation: f32, value: f32) -> [f32; 3] {
    let h = (hue_deg.rem_euclid(360.0)) / 60.0;
    let c = value * saturation;
    let x = c * (1.0 - (h % 2.0 - 1.0).abs());
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = value - c;
    [r + m, g + m, b + m]
}

pub(crate) fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-5)
    }

    #[test]
    fn rainbow_runs_from_light_blue_to_light_red() {
        assert!(close(band_color(0, 9), [0.2, 0.2, 1.0]));
        assert!(close(band_color(4, 9), [0.2, 1.0, 0.2]));
        assert!(close(band_color(8, 9), [1.0, 0.2, 0.2]));
        assert!(close(band_color(2, 5), [0.2, 1.0, 0.2]));
    }

    #[test]
    fn normalization_marks_missing_values() {
        assert_eq!(
            normalize(&[0.0, 5.0, 10.0, f32::NAN], 0.0, 10.0),
            [0.0, 0.5, 1.0, NO_VALUE]
        );
        assert_eq!(normalize(&[3.0], 3.0, 3.0), [0.5]);
    }
}
