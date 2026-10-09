//! Number input shared by all numeric fields of the GUI.

use egui::emath::Numeric;
use plx_model::{Quantity, UnitSystem};

pub use plx_model::units::parse_number;

/// A [`egui::DragValue`] that parses typed input with [`parse_number`].
pub fn drag_value<Num: Numeric>(value: &mut Num) -> egui::DragValue<'_> {
    egui::DragValue::new(value).custom_parser(parse_number)
}

/// A field for a value of a physical quantity in the model's units, PrePoMax's typed
/// converters: the unit stands after the number, and a value typed with another unit is
/// converted, so "3 cm" becomes "30 mm" in a millimetre model.
pub fn quantity(value: &mut f64, units: UnitSystem, quantity: Quantity) -> egui::DragValue<'_> {
    let unit = units.unit(quantity);
    let field = without_unit(value, units, quantity);
    if unit.is_empty() {
        field
    } else {
        field.suffix(format!(" {unit}"))
    }
}

/// A [`quantity`] field that does not show the unit, e.g. because a column title does.
pub fn without_unit(value: &mut f64, units: UnitSystem, quantity: Quantity) -> egui::DragValue<'_> {
    egui::DragValue::new(value).custom_parser(move |text| units.parse_value(text, quantity).ok())
}

/// A [`quantity`] field for values that may be very small or large, such as a density of
/// 7.85e-9: shown in e-notation then, and not changed by dragging.
pub fn physical(value: &mut f64, units: UnitSystem, of: Quantity) -> egui::DragValue<'_> {
    quantity(value, units, of)
        .speed(0.0)
        .custom_formatter(|v, _| format_physical(v))
}

/// Very small or large numbers in e-notation, others as they are.
pub fn format_physical(value: f64) -> String {
    if value != 0.0 && !(1e-3..1e7).contains(&value.abs()) {
        format!("{value:e}")
    } else {
        format!("{value}")
    }
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
