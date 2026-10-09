//! Number input shared by all numeric fields of the GUI.

use egui::emath::Numeric;

/// Parses a number typed by the user.
///
/// Both `.` and `,` are accepted as decimal separator, as is e-notation
/// (`2,1e5`, `2.1E-3`). There are no thousands separators, so `1.000,5` and
/// `1 000` are rejected. Non-finite values are rejected as well.
pub fn parse_number(text: &str) -> Option<f64> {
    let text: String = text
        .trim()
        .chars()
        .map(|c| match c {
            ',' => '.',
            // Typographic minus, as egui's own parser accepts it.
            '\u{2212}' => '-',
            c => c,
        })
        .collect();
    if !text
        .chars()
        .all(|c| c.is_ascii_digit() || "+-.eE".contains(c))
    {
        return None;
    }
    text.parse::<f64>().ok().filter(|v| v.is_finite())
}

/// A [`egui::DragValue`] that parses typed input with [`parse_number`].
pub fn drag_value<Num: Numeric>(value: &mut Num) -> egui::DragValue<'_> {
    egui::DragValue::new(value).custom_parser(parse_number)
}

#[cfg(test)]
mod tests {
    use super::parse_number;

    #[test]
    fn accepts_point_and_comma() {
        assert_eq!(parse_number("1.5"), Some(1.5));
        assert_eq!(parse_number("1,5"), Some(1.5));
        assert_eq!(parse_number(" -0,25 "), Some(-0.25));
        assert_eq!(parse_number(",5"), Some(0.5));
        assert_eq!(parse_number("42"), Some(42.0));
        assert_eq!(parse_number("\u{2212}3"), Some(-3.0));
    }

    #[test]
    fn accepts_e_notation() {
        assert_eq!(parse_number("2,1e5"), Some(2.1e5));
        assert_eq!(parse_number("2.1E-3"), Some(2.1e-3));
        assert_eq!(parse_number("7,85e-9"), Some(7.85e-9));
        assert_eq!(parse_number("1e+3"), Some(1e3));
    }

    #[test]
    fn rejects_thousands_separators_and_junk() {
        assert_eq!(parse_number("1.000,5"), None);
        assert_eq!(parse_number("1,000.5"), None);
        assert_eq!(parse_number("1 000"), None);
        assert_eq!(parse_number(""), None);
        assert_eq!(parse_number("abc"), None);
        assert_eq!(parse_number("inf"), None);
        assert_eq!(parse_number("NaN"), None);
        assert_eq!(parse_number("1e999"), None);
    }
}
