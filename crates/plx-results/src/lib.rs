//! Ergebnis-Datenmodell und abgeleitete Größen.
//!
//! Results are stored per increment as nodal fields. Every component holds one value per node
//! of the result mesh, in the order of [`plx_mesh::FeMesh::coords`]; nodes without a value are
//! `NaN`.

mod derived;
pub mod equation;
pub mod field_output;
pub mod history_output;
pub mod hot_spot;
pub mod path;
pub mod transformation;

pub use derived::{add_derived_components, principal_values};
pub use equation::Equation;
pub use field_output::{FieldOutput, FieldOutputKind, LimitBasis};
pub use transformation::{Transformation, TransformationKind};

/// Kind of analysis an increment belongs to, as CalculiX reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnalysisKind {
    Static,
    Frequency,
    Buckling,
    Dynamic,
    Other(i32),
}

impl AnalysisKind {
    /// Label of the increment value (time, eigenfrequency, buckling factor).
    pub fn value_label(self) -> &'static str {
        match self {
            AnalysisKind::Frequency => "Frequenz",
            AnalysisKind::Buckling => "Lastfaktor",
            _ => "Zeit",
        }
    }
}

/// One component of a field, e.g. `D1` of `DISP` or `MISES` of `STRESS`.
#[derive(Clone, Debug, PartialEq)]
pub struct Component {
    pub name: String,
    pub values: Vec<f32>,
    /// Computed by prepolix rather than read from the file.
    pub derived: bool,
}

impl Component {
    /// A magnitude or equivalent value that never turns negative (`ALL`, `MISES`, `TRESCA`,
    /// `EQUIVALENT`): it keeps its sign when the result is scaled by a negative factor.
    pub fn is_invariant(&self) -> bool {
        self.derived
            && matches!(
                self.name.as_str(),
                "ALL" | "MISES" | "TRESCA" | "EQUIVALENT"
            )
    }

    /// Smallest and largest finite value, if any.
    pub fn range(&self) -> Option<(f32, f32)> {
        self.values
            .iter()
            .copied()
            .filter(|v| v.is_finite())
            .fold(None, |range, v| match range {
                None => Some((v, v)),
                Some((min, max)) => Some((min.min(v), max.max(v))),
            })
    }
}

/// A nodal result field such as `DISP` or `STRESS`.
#[derive(Clone, Debug, PartialEq)]
pub struct Field {
    pub name: String,
    pub components: Vec<Component>,
}

impl Field {
    pub fn component(&self, name: &str) -> Option<&Component> {
        self.components.iter().find(|c| c.name == name)
    }
}

/// All fields written for one increment of one step.
#[derive(Clone, Debug, PartialEq)]
pub struct Increment {
    pub step: u32,
    pub increment: u32,
    pub kind: AnalysisKind,
    /// Time, eigenfrequency or buckling factor, depending on [`Increment::kind`].
    pub value: f64,
    pub fields: Vec<Field>,
}

impl Increment {
    pub fn field(&self, name: &str) -> Option<&Field> {
        self.fields.iter().find(|f| f.name == name)
    }

    /// Nodal displacements, if the increment has a `DISP` field with three components.
    pub fn displacements(&self) -> Option<Vec<[f32; 3]>> {
        let field = self.field("DISP")?;
        let [x, y, z] = ["U1", "U2", "U3"].map(|n| field.component(n));
        let (x, y, z) = (x?, y?, z?);
        Some(
            x.values
                .iter()
                .zip(&y.values)
                .zip(&z.values)
                .map(|((&x, &y), &z)| [x, y, z].map(|v| if v.is_finite() { v } else { 0.0 }))
                .collect(),
        )
    }
}
