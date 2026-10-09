//! Hot spot stress (Strukturspannung) at weld toes and notches, evaluated from the results.
//!
//! A definition says where the hot spots are and how their stress is read: paths start at
//! the picked toe nodes, run along the plate surface away from the weld, and the surface
//! stress at fixed distances on them is extrapolated back to the toe, as in the IIW
//! recommendations for fatigue design of welded joints (Hobbacher, 2016, section 2.2.3).
//! Nothing of it goes into the input file.

use serde::{Deserialize, Serialize};

use crate::Region;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HotSpot {
    pub name: String,
    /// Nodes on the weld toe or at the notch; a path starts at each of them.
    pub toe: Region,
    /// Direction of the paths, away from the weld along the plate surface. It is turned
    /// into the plate surface and, where several toe nodes form a line, perpendicular to it.
    pub direction: [f64; 3],
    /// Plate thickness t, the unit of the read-out distances of type a hot spots.
    pub thickness: f64,
    pub extrapolation: Extrapolation,
    pub component: HotSpotComponent,
}

impl HotSpot {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            toe: Region::Nodes(Vec::new()),
            direction: [1.0, 0.0, 0.0],
            thickness: 10.0,
            extrapolation: Extrapolation::IiwFineLinear,
            component: HotSpotComponent::Perpendicular,
        }
    }

    /// Distances of the read-out points from the toe, in model length units.
    pub fn distances(&self) -> Vec<f64> {
        self.extrapolation.distances(self.thickness)
    }
}

/// Read-out points and extrapolation, IIW's surface stress extrapolation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Extrapolation {
    /// Type a, fine mesh: linear through 0.4 t and 1.0 t.
    IiwFineLinear,
    /// Type a, fine mesh with a strongly non-linear stress rise: quadratic through 0.4 t,
    /// 0.9 t and 1.4 t.
    IiwFineQuadratic,
    /// Type a, coarse mesh with elements of length t: linear through 0.5 t and 1.5 t.
    IiwCoarse,
    /// Type b (toe on a plate edge), fine mesh: quadratic through 4, 8 and 12 mm.
    IiwTypeBFine,
    /// Type b, coarse mesh: linear through 5 and 15 mm.
    IiwTypeBCoarse,
    /// Own distances in model length units; a polynomial through all of them is
    /// extrapolated, a straight line for two.
    Custom(Vec<f64>),
}

impl Extrapolation {
    pub const IIW: [Extrapolation; 5] = [
        Extrapolation::IiwFineLinear,
        Extrapolation::IiwFineQuadratic,
        Extrapolation::IiwCoarse,
        Extrapolation::IiwTypeBFine,
        Extrapolation::IiwTypeBCoarse,
    ];

    /// Distances of the read-out points from the toe for plate thickness `t`.
    pub fn distances(&self, t: f64) -> Vec<f64> {
        match self {
            Extrapolation::IiwFineLinear => vec![0.4 * t, 1.0 * t],
            Extrapolation::IiwFineQuadratic => vec![0.4 * t, 0.9 * t, 1.4 * t],
            Extrapolation::IiwCoarse => vec![0.5 * t, 1.5 * t],
            Extrapolation::IiwTypeBFine => vec![4.0, 8.0, 12.0],
            Extrapolation::IiwTypeBCoarse => vec![5.0, 15.0],
            Extrapolation::Custom(distances) => distances.clone(),
        }
    }

    /// Whether the distances scale with the plate thickness.
    pub fn uses_thickness(&self) -> bool {
        matches!(
            self,
            Extrapolation::IiwFineLinear
                | Extrapolation::IiwFineQuadratic
                | Extrapolation::IiwCoarse
        )
    }

    pub fn label(&self) -> &'static str {
        match self {
            Extrapolation::IiwFineLinear => "IIW a, fein, linear: 0,4t / 1,0t",
            Extrapolation::IiwFineQuadratic => "IIW a, fein, quadr.: 0,4t / 0,9t / 1,4t",
            Extrapolation::IiwCoarse => "IIW a, grob, linear: 0,5t / 1,5t",
            Extrapolation::IiwTypeBFine => "IIW b, fein, quadr.: 4 / 8 / 12 mm",
            Extrapolation::IiwTypeBCoarse => "IIW b, grob, linear: 5 / 15 mm",
            Extrapolation::Custom(_) => "Eigene Lesepunkte",
        }
    }
}

/// Weights that extrapolate values at `distances` to distance 0 with the polynomial through
/// all points (Lagrange). For IIW's distances they are IIW's factors, e.g. 1.67 and -0.67.
pub fn extrapolation_weights(distances: &[f64]) -> Vec<f64> {
    distances
        .iter()
        .enumerate()
        .map(|(i, &di)| {
            distances
                .iter()
                .enumerate()
                .filter(|&(j, _)| j != i)
                .map(|(_, &dj)| dj / (dj - di))
                .product()
        })
        .collect()
}

/// Which stress is extrapolated.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum HotSpotComponent {
    /// Normal stress perpendicular to the weld toe, along the path (IIW's default).
    #[default]
    Perpendicular,
    /// Largest principal stress.
    MaxPrincipal,
    /// The principal stress of largest magnitude, with its sign.
    SignedMaxAbsPrincipal,
}

impl HotSpotComponent {
    pub const ALL: [HotSpotComponent; 3] = [
        HotSpotComponent::Perpendicular,
        HotSpotComponent::MaxPrincipal,
        HotSpotComponent::SignedMaxAbsPrincipal,
    ];

    pub fn label(self) -> &'static str {
        match self {
            HotSpotComponent::Perpendicular => "Senkrecht zum Nahtübergang",
            HotSpotComponent::MaxPrincipal => "Größte Hauptspannung",
            HotSpotComponent::SignedMaxAbsPrincipal => "Betragsgrößte Hauptspannung",
        }
    }

    /// Short name for tables and the output file.
    pub fn short(self) -> &'static str {
        match self {
            HotSpotComponent::Perpendicular => "S_PERP",
            HotSpotComponent::MaxPrincipal => "PRINCIPAL_MAX",
            HotSpotComponent::SignedMaxAbsPrincipal => "SGN_MAX_ABS_PRI",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: &[f64], b: &[f64]) -> bool {
        a.len() == b.len() && a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-9)
    }

    #[test]
    fn weights_are_the_iiw_factors() {
        let t = 12.0;
        let weights = |e: Extrapolation| extrapolation_weights(&e.distances(t));
        // IIW recommendations, equations (2.2-1) to (2.2-5).
        assert!(close(
            &weights(Extrapolation::IiwFineLinear),
            &[5.0 / 3.0, -2.0 / 3.0]
        ));
        assert!(close(
            &weights(Extrapolation::IiwFineQuadratic),
            &[2.52, -2.24, 0.72]
        ));
        assert!(close(&weights(Extrapolation::IiwCoarse), &[1.5, -0.5]));
        assert!(close(
            &weights(Extrapolation::IiwTypeBFine),
            &[3.0, -3.0, 1.0]
        ));
        assert!(close(&weights(Extrapolation::IiwTypeBCoarse), &[1.5, -0.5]));
    }

    #[test]
    fn extrapolation_is_exact_for_polynomials_of_its_order() {
        let distances = [2.0, 5.0, 7.0];
        let weights = extrapolation_weights(&distances);
        let f = |x: f64| 3.0 - 2.0 * x + 0.5 * x * x;
        let at_toe: f64 = distances.iter().zip(&weights).map(|(&d, w)| w * f(d)).sum();
        assert!((at_toe - 3.0).abs() < 1e-9);
    }
}
