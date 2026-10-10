//! Sections: what the elements of a region are made of. A solid section gives a material to
//! solid or plane elements, a beam or truss section turns line elements into CalculiX beams
//! (`*BEAM SECTION`) or trusses (`*SOLID SECTION` with an area).
//!
//! The mesh only knows the shape of a line element, two or three nodes; which CalculiX type
//! it becomes (B31, B32, B32R, T3D2) follows from its section when the input file is written.
//! PrePoMax has no beams, so these follow CalculiX's own keywords.

use glam::DVec3;
use plx_mesh::{Element, ElementFamily, ElementShape};
use serde::{Deserialize, Serialize};

use crate::Region;

/// Assigns a material to the elements of a region, PrePoMax's solid section, or a beam or
/// truss section of CalculiX.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Section {
    pub name: String,
    pub material: String,
    pub region: Region,
    /// Thickness of plane stress and plane strain elements; other models ignore it.
    #[serde(default = "unit_thickness")]
    pub thickness: f64,
    /// Projects saved before beams existed hold solid sections.
    #[serde(default)]
    pub kind: SectionKind,
}

/// PrePoMax's default thickness of 2D sections.
pub fn unit_thickness() -> f64 {
    1.0
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum SectionKind {
    /// Solid, shell or plane elements with a material.
    #[default]
    Solid,
    /// Line elements carrying axial force only (`T3D2`); `area` is the cross-section.
    Truss { area: f64 },
    /// Line elements with bending stiffness (`B31`, `B32`, `B32R`).
    Beam(BeamSection),
    /// Surface elements of a 3D model as shells (`*SHELL SECTION`), PrePoMax's shell
    /// section: the thickness and the offset of the reference surface from the mid-surface
    /// as a fraction of the thickness.
    Shell { thickness: f64, offset: f64 },
}

impl SectionKind {
    pub const ALL: [SectionKind; 4] = [
        SectionKind::Solid,
        SectionKind::Truss { area: 1.0 },
        SectionKind::Beam(BeamSection::DEFAULT),
        SectionKind::Shell {
            thickness: 1.0,
            offset: 0.0,
        },
    ];

    /// Name of the kind, also the prefix of new sections' names, like PrePoMax's
    /// `Solid_Section-1`.
    pub fn prefix(&self) -> &'static str {
        match self {
            SectionKind::Solid => "Solid_Section",
            SectionKind::Truss { .. } => "Truss_Section",
            SectionKind::Beam(_) => "Beam_Section",
            SectionKind::Shell { .. } => "Shell_Section",
        }
    }

    /// Name in the dialogs.
    pub fn label(&self) -> &'static str {
        match self {
            SectionKind::Solid => "Solid",
            SectionKind::Truss { .. } => "Stab (Truss)",
            SectionKind::Beam(_) => "Balken (Beam)",
            SectionKind::Shell { .. } => "Shell",
        }
    }

    pub fn is_line(&self) -> bool {
        matches!(self, SectionKind::Truss { .. } | SectionKind::Beam(_))
    }

    /// The CalculiX type the section gives a line element; `None` when the section does
    /// not change the element's type. Trusses are always `T3D2`: CalculiX 2.21 gets the
    /// stiffness of `T3D3` wrong (3.6 times too soft in a tension test), so a quadratic
    /// line is written with its end nodes only. Pipe and box profiles need `B32R`.
    pub fn element_type(&self, shape: ElementShape) -> Option<&'static str> {
        match (self, shape) {
            (SectionKind::Solid | SectionKind::Shell { .. }, _) => None,
            (SectionKind::Truss { .. }, ElementShape::Line2 | ElementShape::Line3) => Some("T3D2"),
            (SectionKind::Beam(_), ElementShape::Line2) => Some("B31"),
            (SectionKind::Beam(beam), ElementShape::Line3) => {
                Some(if beam.profile.needs_reduced_integration() {
                    "B32R"
                } else {
                    "B32"
                })
            }
            _ => None,
        }
    }

    /// Why the section cannot be written for an element of the region, if it cannot.
    pub fn rejects(&self, element: &Element) -> Option<String> {
        let line = element.shape.family() == ElementFamily::Line;
        match self {
            SectionKind::Solid if line => Some(format!(
                "Element {} ist ein Linienelement; Linien brauchen eine Beam oder Truss \
                 Section",
                element.id
            )),
            SectionKind::Solid => None,
            SectionKind::Shell { .. } if element.shape.family() != ElementFamily::Surface => {
                Some(format!(
                    "Element {} is not a surface element; a shell section needs triangles or \
                     quads",
                    element.id
                ))
            }
            SectionKind::Shell { .. } => None,
            _ if !line => Some(format!("Element {} ist kein Linienelement", element.id)),
            SectionKind::Beam(beam)
                if beam.profile.needs_reduced_integration()
                    && element.shape != ElementShape::Line3 =>
            {
                Some(format!(
                    "Element {} hat 2 Knoten; Rohr- und Kastenprofile brauchen Linien mit 3 \
                     Knoten (B32R)",
                    element.id
                ))
            }
            _ => None,
        }
    }
}

/// A CalculiX beam section: the profile, its orientation and the offset of the beam axis.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct BeamSection {
    pub profile: BeamProfile,
    #[serde(default)]
    pub orientation: BeamOrientation,
    /// Offset of the beam axis from the nodes in the 1- and 2-direction, in multiples of
    /// the profile's thickness in that direction (`OFFSET1`, `OFFSET2`).
    #[serde(default)]
    pub offset: [f64; 2],
}

impl BeamSection {
    pub const DEFAULT: BeamSection = BeamSection {
        profile: BeamProfile::Rect { a: 1.0, b: 1.0 },
        orientation: BeamOrientation::Automatic,
        offset: [0.0, 0.0],
    };
}

impl Default for BeamSection {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Cross-section of a beam, with CalculiX's dimensions: the 1-direction is the normal of
/// the section, the 2-direction the beam axis crossed with it.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum BeamProfile {
    /// Rectangle with thickness `a` in the 1-direction and `b` in the 2-direction.
    Rect { a: f64, b: f64 },
    /// Circle; CalculiX takes the diameter, see [`BeamProfile::data_line`].
    Circ { radius: f64 },
    /// Hollow circle with outer radius and wall thickness (`B32R` only).
    Pipe { radius: f64, thickness: f64 },
    /// Hollow rectangle `a` by `b` with four wall thicknesses, in the order CalculiX reads
    /// them: at +1, +2, -1 and -2 (`B32R` only).
    Box { a: f64, b: f64, t: [f64; 4] },
}

impl BeamProfile {
    pub const ALL: [BeamProfile; 4] = [
        BeamProfile::Rect { a: 1.0, b: 1.0 },
        BeamProfile::Circ { radius: 1.0 },
        BeamProfile::Pipe {
            radius: 1.0,
            thickness: 0.1,
        },
        BeamProfile::Box {
            a: 1.0,
            b: 1.0,
            t: [0.1; 4],
        },
    ];

    /// Value of `SECTION=` in the input file.
    pub fn keyword(&self) -> &'static str {
        match self {
            BeamProfile::Rect { .. } => "RECT",
            BeamProfile::Circ { .. } => "CIRC",
            BeamProfile::Pipe { .. } => "PIPE",
            BeamProfile::Box { .. } => "BOX",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            BeamProfile::Rect { .. } => "Rechteck",
            BeamProfile::Circ { .. } => "Kreis",
            BeamProfile::Pipe { .. } => "Rohr",
            BeamProfile::Box { .. } => "Kasten",
        }
    }

    /// Pipes and boxes exist for `B32R` only in CalculiX.
    pub fn needs_reduced_integration(&self) -> bool {
        matches!(self, BeamProfile::Pipe { .. } | BeamProfile::Box { .. })
    }

    /// The dimensions as CalculiX 2.21 reads them on the first data line of `*BEAM
    /// SECTION`. A circle is given by its thickness in both directions, the diameter: a
    /// value of 4 puts the expanded nodes at radius 2. A pipe takes the radius itself.
    pub fn data_line(&self) -> Vec<f64> {
        match *self {
            BeamProfile::Rect { a, b } => vec![a, b],
            BeamProfile::Circ { radius } => vec![2.0 * radius, 2.0 * radius],
            BeamProfile::Pipe { radius, thickness } => vec![radius, thickness],
            BeamProfile::Box { a, b, t } => vec![a, b, t[0], t[1], t[2], t[3]],
        }
    }

    /// Whether the dimensions make a section CalculiX accepts.
    pub fn is_valid(&self) -> bool {
        let positive = |v: f64| v.is_finite() && v > 0.0;
        match *self {
            BeamProfile::Rect { a, b } => positive(a) && positive(b),
            BeamProfile::Circ { radius } => positive(radius),
            BeamProfile::Pipe { radius, thickness } => {
                positive(radius) && positive(thickness) && thickness < radius
            }
            BeamProfile::Box { a, b, t } => {
                positive(a)
                    && positive(b)
                    && t.iter().all(|&t| positive(t))
                    && t[0] + t[2] < a
                    && t[1] + t[3] < b
            }
        }
    }
}

/// Direction 1 of a beam's section, the normal `n1` of `*BEAM SECTION`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum BeamOrientation {
    /// The global z-axis, or the x-axis for beams along z, chosen per element when the
    /// input file is written; CalculiX projects it onto the plane normal to the beam.
    #[default]
    Automatic,
    /// A direction given by the user; must not be parallel to any beam of the section.
    Direction([f64; 3]),
}

/// Angle in degrees below which a normal counts as parallel to the beam axis.
const PARALLEL_ANGLE_DEG: f64 = 1.0;

impl BeamOrientation {
    /// The normal written for an element with the given unit tangent, or `None` when the
    /// user's direction is parallel to it. Automatic orientation takes +z, or +x for beams
    /// within 1 degree of z.
    pub fn normal_for(&self, tangent: DVec3) -> Option<[f64; 3]> {
        let parallel = |n: DVec3| {
            let n = n.normalize_or_zero();
            n.cross(tangent).length() < PARALLEL_ANGLE_DEG.to_radians().sin()
        };
        match *self {
            BeamOrientation::Automatic => Some(if parallel(DVec3::Z) {
                [1.0, 0.0, 0.0]
            } else {
                [0.0, 0.0, 1.0]
            }),
            BeamOrientation::Direction(n) => {
                let n = DVec3::from(n);
                (n.length() > 0.0 && !parallel(n)).then_some(n.into())
            }
        }
    }
}

/// Unit tangent of a line element, from its first to its last corner node; zero for a
/// degenerate element or missing nodes.
pub fn line_tangent(mesh: &plx_mesh::FeMesh, element: &Element) -> DVec3 {
    let corners = match element.shape {
        ElementShape::Line2 => (0, 1),
        ElementShape::Line3 => (0, 2),
        _ => return DVec3::ZERO,
    };
    let point = |k: usize| element.nodes.get(k).and_then(|&id| mesh.node(id));
    match (point(corners.0), point(corners.1)) {
        (Some(a), Some(b)) => (DVec3::from(b) - DVec3::from(a)).normalize_or_zero(),
        _ => DVec3::ZERO,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_types_follow_the_section() {
        let beam = SectionKind::Beam(BeamSection::DEFAULT);
        assert_eq!(beam.element_type(ElementShape::Line2), Some("B31"));
        assert_eq!(beam.element_type(ElementShape::Line3), Some("B32"));
        assert_eq!(beam.element_type(ElementShape::Tet4), None);
        let pipe = SectionKind::Beam(BeamSection {
            profile: BeamProfile::ALL[2],
            ..BeamSection::DEFAULT
        });
        assert_eq!(pipe.element_type(ElementShape::Line3), Some("B32R"));
        let truss = SectionKind::Truss { area: 2.0 };
        assert_eq!(truss.element_type(ElementShape::Line3), Some("T3D2"));
        assert_eq!(SectionKind::Solid.element_type(ElementShape::Line2), None);
    }

    #[test]
    fn automatic_normals_avoid_the_beam_axis() {
        let auto = BeamOrientation::Automatic;
        assert_eq!(auto.normal_for(DVec3::X), Some([0.0, 0.0, 1.0]));
        assert_eq!(auto.normal_for(DVec3::Z), Some([1.0, 0.0, 0.0]));
        assert_eq!(auto.normal_for(-DVec3::Z), Some([1.0, 0.0, 0.0]));
        let given = BeamOrientation::Direction([0.0, 1.0, 0.0]);
        assert_eq!(given.normal_for(DVec3::X), Some([0.0, 1.0, 0.0]));
        assert_eq!(given.normal_for(DVec3::Y), None);
        assert_eq!(
            BeamOrientation::Direction([0.0; 3]).normal_for(DVec3::X),
            None
        );
    }

    #[test]
    fn profiles_write_calculix_dimensions() {
        assert_eq!(
            BeamProfile::Circ { radius: 2.0 }.data_line(),
            vec![4.0, 4.0]
        );
        assert_eq!(
            BeamProfile::Box {
                a: 10.0,
                b: 5.0,
                t: [1.0, 2.0, 3.0, 4.0]
            }
            .data_line(),
            vec![10.0, 5.0, 1.0, 2.0, 3.0, 4.0]
        );
        assert!(
            !BeamProfile::Pipe {
                radius: 1.0,
                thickness: 1.0
            }
            .is_valid()
        );
    }

    #[test]
    fn sections_of_old_projects_are_solid() {
        let text = r#"(name: "S", material: "M", region: Parts(["P"]))"#;
        let section: Section = ron::from_str(text).unwrap();
        assert_eq!(section.kind, SectionKind::Solid);
        assert_eq!(section.thickness, 1.0);
    }
}
