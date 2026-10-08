use crate::{Component, Field};

/// Adds the quantities PrePoMax shows besides the raw components: the magnitude of vector
/// fields and, for tensor fields, the von Mises and Tresca equivalents and the principal values.
pub fn add_derived_components(field: &mut Field) {
    let column = |names: &[&str]| -> Option<Vec<&[f32]>> {
        names
            .iter()
            .map(|n| field.component(n).map(|c| c.values.as_slice()))
            .collect()
    };
    let mut derived: Vec<(&str, Vec<f32>)> = Vec::new();
    if let Some(v) = vector_columns(field).and_then(|names| column(&names)) {
        let all = (0..v[0].len())
            .map(|i| (v[0][i].powi(2) + v[1][i].powi(2) + v[2][i].powi(2)).sqrt())
            .collect();
        derived.push(("ALL", all));
    } else if let Some(t) = tensor_columns(field).and_then(|names| column(&names)) {
        let count = t[0].len();
        let mut mises = Vec::with_capacity(count);
        let mut tresca = Vec::with_capacity(count);
        let mut principal = [
            Vec::with_capacity(count),
            Vec::with_capacity(count),
            Vec::with_capacity(count),
        ];
        let mut signed_max_abs = Vec::with_capacity(count);
        for i in 0..count {
            let [xx, yy, zz, xy, yz, zx] = [0, 1, 2, 3, 4, 5].map(|k| t[k][i] as f64);
            let m = (0.5
                * ((xx - yy).powi(2)
                    + (yy - zz).powi(2)
                    + (zz - xx).powi(2)
                    + 6.0 * (xy * xy + yz * yz + zx * zx)))
                .sqrt();
            let [p1, p2, p3] = principal_values([xx, yy, zz, xy, yz, zx]);
            mises.push(m as f32);
            tresca.push((p1 - p3) as f32);
            principal[0].push(p1 as f32);
            principal[1].push(p2 as f32);
            principal[2].push(p3 as f32);
            let signed = if p1.abs() >= p3.abs() { p1 } else { p3 };
            signed_max_abs.push(signed as f32);
        }
        let [p1, p2, p3] = principal;
        derived.extend([
            ("MISES", mises),
            ("TRESCA", tresca),
            ("SGN-MAX-ABS-PRI", signed_max_abs),
            ("MAX-PRI", p1),
            ("MID-PRI", p2),
            ("MIN-PRI", p3),
        ]);
    }
    for (name, values) in derived {
        field.components.retain(|c| c.name != name);
        field.components.push(Component {
            name: name.to_string(),
            values,
            derived: true,
        });
    }
}

/// Names of the x, y, z components of a vector field, if the field has them.
fn vector_columns(field: &Field) -> Option<[&'static str; 3]> {
    const VECTORS: [[&str; 3]; 4] = [
        ["D1", "D2", "D3"],
        ["V1", "V2", "V3"],
        ["F1", "F2", "F3"],
        ["RF1", "RF2", "RF3"],
    ];
    VECTORS
        .into_iter()
        .find(|names| names.iter().all(|n| field.component(n).is_some()))
}

/// Names of the six components of a symmetric tensor field (xx, yy, zz, xy, yz, zx).
fn tensor_columns(field: &Field) -> Option<[&'static str; 6]> {
    const TENSORS: [[&str; 6]; 2] = [
        ["SXX", "SYY", "SZZ", "SXY", "SYZ", "SZX"],
        ["EXX", "EYY", "EZZ", "EXY", "EYZ", "EZX"],
    ];
    TENSORS
        .into_iter()
        .find(|names| names.iter().all(|n| field.component(n).is_some()))
}

/// Eigenvalues of a symmetric 3 × 3 tensor given as (xx, yy, zz, xy, yz, zx), largest first.
pub fn principal_values([xx, yy, zz, xy, yz, zx]: [f64; 6]) -> [f64; 3] {
    let off = xy * xy + yz * yz + zx * zx;
    if !(xx.is_finite() && yy.is_finite() && zz.is_finite() && off.is_finite()) {
        return [f64::NAN; 3];
    }
    let mean = (xx + yy + zz) / 3.0;
    let (a, b, c) = (xx - mean, yy - mean, zz - mean);
    let p = ((a * a + b * b + c * c + 2.0 * off) / 6.0).sqrt();
    if p <= f64::EPSILON * mean.abs().max(1.0) {
        return [mean; 3];
    }
    // det((T - mean I) / p) / 2 = cos(3 phi)
    let det = a * (b * c - yz * yz) - xy * (xy * c - yz * zx) + zx * (xy * yz - b * zx);
    let r = (det / (2.0 * p * p * p)).clamp(-1.0, 1.0);
    let phi = r.acos() / 3.0;
    let third = 2.0 * std::f64::consts::PI / 3.0;
    let p1 = mean + 2.0 * p * phi.cos();
    let p3 = mean + 2.0 * p * (phi + third).cos();
    [p1, 3.0 * mean - p1 - p3, p3]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(name: &str, components: &[(&str, f32)]) -> Field {
        Field {
            name: name.into(),
            components: components
                .iter()
                .map(|&(n, v)| Component {
                    name: n.into(),
                    values: vec![v],
                    derived: false,
                })
                .collect(),
        }
    }

    fn value(field: &Field, name: &str) -> f32 {
        field.component(name).unwrap().values[0]
    }

    #[test]
    fn displacement_magnitude() {
        let mut disp = field("DISP", &[("D1", 3.0), ("D2", 0.0), ("D3", 4.0)]);
        add_derived_components(&mut disp);
        assert_eq!(value(&disp, "ALL"), 5.0);
    }

    #[test]
    fn uniaxial_stress_invariants() {
        let mut stress = field(
            "STRESS",
            &[
                ("SXX", 100.0),
                ("SYY", 0.0),
                ("SZZ", 0.0),
                ("SXY", 0.0),
                ("SYZ", 0.0),
                ("SZX", 0.0),
            ],
        );
        add_derived_components(&mut stress);
        assert!((value(&stress, "MISES") - 100.0).abs() < 1e-4);
        assert!((value(&stress, "TRESCA") - 100.0).abs() < 1e-4);
        assert!((value(&stress, "MAX-PRI") - 100.0).abs() < 1e-4);
        assert!(value(&stress, "MIN-PRI").abs() < 1e-4);
    }

    #[test]
    fn pure_shear_principal_values() {
        let [p1, p2, p3] = principal_values([0.0, 0.0, 0.0, 50.0, 0.0, 0.0]);
        assert!((p1 - 50.0).abs() < 1e-9 && p2.abs() < 1e-9 && (p3 + 50.0).abs() < 1e-9);
    }

    #[test]
    fn principal_values_of_a_general_tensor() {
        // Eigenvalues 3, 2, 1 rotated: T = R diag(3,2,1) R^T with a 45° turn about z.
        let [p1, p2, p3] = principal_values([2.5, 2.5, 1.0, 0.5, 0.0, 0.0]);
        assert!((p1 - 3.0).abs() < 1e-9 && (p2 - 2.0).abs() < 1e-9 && (p3 - 1.0).abs() < 1e-9);
    }

    #[test]
    fn hydrostatic_state_has_equal_principal_values() {
        assert_eq!(principal_values([7.0, 7.0, 7.0, 0.0, 0.0, 0.0]), [7.0; 3]);
    }
}
