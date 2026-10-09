//! Number input shared by all numeric fields of the GUI.

use egui::emath::Numeric;
use plx_model::{Quantity, UnitSystem};

pub use plx_model::units::parse_number;

/// A [`egui::DragValue`] that parses typed input with [`parse_number`] and shows the value
/// with all its digits ([`format_physical`]). egui would show only as many decimals as its
/// drag speed resolves, so 0.005 would read "0.01" and be stored so on the next edit.
pub fn drag_value<Num: Numeric>(value: &mut Num) -> egui::DragValue<'_> {
    egui::DragValue::new(value)
        .custom_parser(parse_number)
        .custom_formatter(|v, _| format_physical(v))
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
    egui::DragValue::new(value)
        .custom_parser(move |text| units.parse_value(text, quantity).ok())
        .custom_formatter(|v, _| format_physical(v))
}

/// A [`quantity`] field for values that may be very small or large, such as a density of
/// 7.85e-9: shown in e-notation then, and not changed by dragging.
pub fn physical(value: &mut f64, units: UnitSystem, of: Quantity) -> egui::DragValue<'_> {
    quantity(value, units, of).speed(0.0)
}

/// A value with all its digits, never rounded to fewer: very small or large numbers in
/// e-notation, others as they are. Only the noise of a unit conversion beyond 15
/// significant digits, such as 30.000000000000004, is dropped.
pub fn format_physical(value: f64) -> String {
    let value = format!("{value:.14e}").parse().unwrap_or(value);
    if value != 0.0 && !(1e-3..1e7).contains(&value.abs()) {
        format!("{value:e}")
    } else {
        format!("{value}")
    }
}

#[cfg(test)]
mod tests {
    use super::{format_physical, parse_number};
    use plx_model::{Quantity, UnitSystem};

    #[test]
    fn small_values_keep_their_digits() {
        // 5 mm in a metre model is 0.005 m, not the 0.01 m egui's decimals would show.
        let value = UnitSystem::MKgSC
            .parse_value("5mm", Quantity::Length)
            .unwrap();
        assert_eq!(value, 0.005);
        assert_eq!(format_physical(value), "0.005");
        assert_eq!(parse_number(&format_physical(value)), Some(value));
        assert_eq!(format_physical(0.000_25), "2.5e-4");
        assert_eq!(format_physical(1.234_567_891_234), "1.234567891234");
        assert_eq!(format_physical(2.1e11), "2.1e11");
        assert_eq!(format_physical(0.0), "0");
        assert_eq!(format_physical(-12.5), "-12.5");
        // Conversion noise does not show, the typed value does.
        let value = UnitSystem::MmTonSC
            .parse_value("3cm", Quantity::Length)
            .unwrap();
        assert_eq!(format_physical(value), "30");
    }

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
