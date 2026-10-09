//! Field outputs the user derives from the results, PrePoMax's "Create Field Output": a
//! limit (utilisation and safety factor), the envelope over all increments, an equation of
//! other components, or a field in another coordinate system.

use std::collections::HashSet;

use plx_mesh::FeMesh;

use crate::equation::Equation;
use plx_model::CoordinateSystem;

use crate::transformation::{ComponentKind, component_kind};
use crate::{Component, Field, Increment};

/// Component names PrePoMax gives the computed fields.
pub const RATIO: &str = "RATIO";
pub const SAFETY_FACTOR: &str = "SAFETY_FACTOR";
pub const MAX: &str = "MAX";
pub const MIN: &str = "MIN";
pub const AVERAGE: &str = "AVERAGE";
pub const VALUE: &str = "VALUE";

/// What the limit values of a [`FieldOutputKind::Limit`] are given for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LimitBasis {
    Parts,
    ElementSets,
    AllElements,
}

impl LimitBasis {
    pub const ALL: [LimitBasis; 3] = [
        LimitBasis::Parts,
        LimitBasis::ElementSets,
        LimitBasis::AllElements,
    ];

    /// PrePoMax's name, also the item name of the single "All elements" limit.
    pub fn name(self) -> &'static str {
        match self {
            LimitBasis::Parts => "Parts",
            LimitBasis::ElementSets => "Element sets",
            LimitBasis::AllElements => "All elements",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum FieldOutputKind {
    /// RATIO = value / limit and SAFETY_FACTOR = limit / value; a node in several items takes
    /// the smallest limit.
    Limit {
        field: String,
        component: String,
        basis: LimitBasis,
        /// Limit per part or element set name; for all elements one entry.
        limits: Vec<(String, f64)>,
    },
    /// MAX, MIN and AVERAGE of a component over all increments, the same in every increment.
    Envelope { field: String, component: String },
    /// VALUE of an equation of other components.
    Equation { equation: String, unit: String },
    /// The field's components in a user coordinate system.
    CoordinateSystemTransform {
        field: String,
        coordinate_system: String,
    },
}

/// A derived field output: its name becomes the name of the computed field.
#[derive(Clone, Debug, PartialEq)]
pub struct FieldOutput {
    pub name: String,
    pub kind: FieldOutputKind,
}

impl FieldOutput {
    /// Fields this output is computed from.
    pub fn parents(&self) -> Vec<String> {
        match &self.kind {
            FieldOutputKind::Limit { field, .. }
            | FieldOutputKind::Envelope { field, .. }
            | FieldOutputKind::CoordinateSystemTransform { field, .. } => vec![field.clone()],
            FieldOutputKind::Equation { equation, .. } => Equation::parse(equation)
                .map(|e| {
                    let mut fields: Vec<String> = Vec::new();
                    for v in e.variables() {
                        let field = v.split_once('.').map_or(v.as_str(), |(f, _)| f);
                        if !fields.iter().any(|f| f == field) {
                            fields.push(field.to_string());
                        }
                    }
                    fields
                })
                .unwrap_or_default(),
        }
    }

    /// Unit the user entered, for equations.
    pub fn unit(&self) -> Option<&str> {
        match &self.kind {
            FieldOutputKind::Equation { unit, .. } => Some(unit.as_str()),
            _ => None,
        }
    }
}

/// Indices of the mesh nodes used by the given elements.
fn element_nodes<'a>(
    mesh: &FeMesh,
    elements: impl IntoIterator<Item = &'a plx_mesh::ElementId>,
) -> Vec<usize> {
    let mut nodes = HashSet::new();
    for element in elements.into_iter().filter_map(|&id| mesh.element(id)) {
        nodes.extend(element.nodes.iter().filter_map(|&n| mesh.node_index(n)));
    }
    nodes.into_iter().collect()
}

/// Limit of every node, NaN where none is given.
fn node_limits(
    mesh: &FeMesh,
    basis: LimitBasis,
    limits: &[(String, f64)],
) -> Result<Vec<f64>, String> {
    let mut node_limit = vec![f64::NAN; mesh.node_count()];
    for (item, limit) in limits {
        if *limit == 0.0 || !limit.is_finite() {
            return Err(format!("The limit for {item} must not be 0."));
        }
        let nodes = match basis {
            LimitBasis::Parts => match mesh.parts.iter().find(|p| p.name == *item) {
                Some(part) => element_nodes(mesh, &part.elements),
                None => continue,
            },
            LimitBasis::ElementSets => match mesh.element_sets.get(item) {
                Some(set) => element_nodes(mesh, set),
                None => continue,
            },
            LimitBasis::AllElements => (0..mesh.node_count()).collect(),
        };
        for node in nodes {
            let slot = &mut node_limit[node];
            *slot = if slot.is_nan() {
                *limit
            } else {
                slot.min(*limit)
            };
        }
    }
    Ok(node_limit)
}

fn component<'a>(increment: &'a Increment, field: &str, component: &str) -> Option<&'a [f32]> {
    Some(&increment.field(field)?.component(component)?.values)
}

/// The raw components of a vector or tensor field in local directions given per node; `None`
/// for a field that is neither.
fn transformed_field(source: &Field, name: &str, rotations: &[[[f64; 3]; 3]]) -> Option<Field> {
    let raw: Vec<&Component> = source.components.iter().filter(|c| !c.derived).collect();
    let kinds: Vec<ComponentKind> = (0..source.components.len())
        .filter(|&i| !source.components[i].derived)
        .map(|i| component_kind(source, i))
        .collect();
    if kinds.iter().all(|k| *k == ComponentKind::Scalar) {
        return None;
    }
    let count = raw
        .iter()
        .map(|c| c.values.len())
        .min()?
        .min(rotations.len());
    const SLOT: [[usize; 3]; 3] = [[0, 3, 5], [3, 1, 4], [5, 4, 2]];
    let components = raw
        .iter()
        .zip(&kinds)
        .map(|(component, kind)| {
            let values = (0..count)
                .map(|n| {
                    let q = &rotations[n];
                    let value = |k: usize| raw[k].values[n] as f64;
                    let v = match *kind {
                        ComponentKind::Scalar => component.values[n] as f64,
                        ComponentKind::Vector(row) => (0..3).map(|k| q[row][k] * value(k)).sum(),
                        ComponentKind::Tensor(row, column) => {
                            let mut sum = 0.0;
                            for i in 0..3 {
                                for j in 0..3 {
                                    sum += q[row][i] * value(SLOT[i][j]) * q[column][j];
                                }
                            }
                            sum
                        }
                    };
                    v as f32
                })
                .collect();
            Component {
                name: component.name.clone(),
                values,
                derived: false,
            }
        })
        .collect();
    // The invariants and magnitudes follow from the raw components, as for read fields;
    // which ones depends on the source field's name.
    let mut field = Field {
        name: source.name.clone(),
        components,
    };
    crate::add_derived_components(&mut field);
    field.name = name.to_string();
    Some(field)
}

fn new_field(name: &str, components: Vec<(&str, Vec<f32>)>) -> Field {
    Field {
        name: name.to_string(),
        components: components
            .into_iter()
            .map(|(n, values)| Component {
                name: n.to_string(),
                values,
                derived: true,
            })
            .collect(),
    }
}

/// Computes the output for every increment that has its source data and stores it there as a
/// field named after the output, replacing an earlier one. Fails without changing anything if
/// no increment has the source data.
pub fn compute(
    output: &FieldOutput,
    increments: &mut [Increment],
    mesh: &FeMesh,
    systems: &[CoordinateSystem],
) -> Result<(), String> {
    let missing = |field: &str, component: &str| {
        format!("The result {field}.{component} exists in no increment.")
    };
    let fields: Vec<Option<Field>> = match &output.kind {
        FieldOutputKind::Limit {
            field,
            component: name,
            basis,
            limits,
        } => {
            let node_limit = node_limits(mesh, *basis, limits)?;
            increments
                .iter()
                .map(|inc| {
                    let values = component(inc, field, name)?;
                    let (ratio, safety) = values
                        .iter()
                        .zip(&node_limit)
                        .map(|(&v, &limit)| {
                            let v = v as f64;
                            let safety = if v == 0.0 { f64::NAN } else { limit / v };
                            ((v / limit) as f32, safety as f32)
                        })
                        .unzip();
                    Some(new_field(
                        &output.name,
                        vec![(RATIO, ratio), (SAFETY_FACTOR, safety)],
                    ))
                })
                .collect()
        }
        FieldOutputKind::Envelope {
            field,
            component: name,
        } => {
            let sources: Vec<&[f32]> = increments
                .iter()
                .filter_map(|inc| component(inc, field, name))
                .collect();
            let Some(first) = sources.first() else {
                return Err(missing(field, name));
            };
            let mut max = vec![f32::NAN; first.len()];
            let mut min = max.clone();
            let mut sum = vec![0.0f64; first.len()];
            for values in &sources {
                for (i, &v) in values.iter().enumerate().take(first.len()) {
                    // f32::max and min ignore NaN, so nodes without a value stay NaN.
                    max[i] = max[i].max(v);
                    min[i] = min[i].min(v);
                    sum[i] += v as f64;
                }
            }
            let average: Vec<f32> = sum
                .iter()
                .map(|s| (s / sources.len() as f64) as f32)
                .collect();
            let envelope = new_field(
                &output.name,
                vec![(MAX, max), (MIN, min), (AVERAGE, average)],
            );
            increments
                .iter()
                .map(|inc| inc.field(field).map(|_| envelope.clone()))
                .collect()
        }
        FieldOutputKind::Equation { equation, .. } => {
            let parsed = Equation::parse(equation)?;
            let names: Vec<(&str, &str)> = parsed
                .variables()
                .iter()
                .map(|v| {
                    v.split_once('.').ok_or_else(|| {
                        format!("{v}: Results are given as FieldName.Component, e.g. STRESS.MISES.")
                    })
                })
                .collect::<Result<_, _>>()?;
            if names.is_empty() {
                return Err(
                    "The equation must contain at least one component, e.g. =STRESS.MISES.".into(),
                );
            }
            for (field, name) in &names {
                if field == &output.name {
                    return Err(format!("{} cannot use itself.", output.name));
                }
                if !increments
                    .iter()
                    .any(|inc| component(inc, field, name).is_some())
                {
                    return Err(missing(field, name));
                }
            }
            increments
                .iter()
                .map(|inc| {
                    let sources: Vec<&[f32]> = names
                        .iter()
                        .map(|(f, c)| component(inc, f, c))
                        .collect::<Option<_>>()?;
                    let count = sources.iter().map(|s| s.len()).min()?;
                    let mut args = vec![0.0; sources.len()];
                    let values = (0..count)
                        .map(|i| {
                            for (arg, source) in args.iter_mut().zip(&sources) {
                                *arg = source[i] as f64;
                            }
                            parsed.evaluate(&args) as f32
                        })
                        .collect();
                    Some(new_field(&output.name, vec![(VALUE, values)]))
                })
                .collect()
        }
        FieldOutputKind::CoordinateSystemTransform {
            field,
            coordinate_system,
        } => {
            let system = (systems.iter())
                .find(|s| s.name == *coordinate_system)
                .ok_or_else(|| {
                    format!("The coordinate system {coordinate_system} does not exist.")
                })?;
            // The local directions at every node, as rows of a rotation.
            let rotations = (mesh.coords().iter())
                .map(|&p| system.directions_at(p))
                .collect::<Result<Vec<_>, _>>()?;
            increments
                .iter()
                .map(|inc| transformed_field(inc.field(field)?, &output.name, &rotations))
                .collect()
        }
    };
    if fields.iter().all(Option::is_none) {
        return Err(match &output.kind {
            FieldOutputKind::Limit {
                field, component, ..
            } => missing(field, component),
            _ => "The source data is available for no increment.".into(),
        });
    }
    remove(&output.name, increments);
    for (increment, field) in increments.iter_mut().zip(fields) {
        if let Some(field) = field {
            increment.fields.push(field);
        }
    }
    Ok(())
}

/// Removes the computed field of an output from all increments.
pub fn remove(name: &str, increments: &mut [Increment]) {
    for increment in increments {
        increment.fields.retain(|f| f.name != name);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AnalysisKind;
    use plx_mesh::{Element, ElementShape, Part};

    /// Two linear bars, one per part, sharing node 2.
    fn mesh() -> FeMesh {
        let mut mesh = FeMesh::default();
        for (id, x) in [(1, 0.0), (2, 1.0), (3, 2.0)] {
            mesh.set_node(id, [x, 0.0, 0.0]);
        }
        for (id, nodes) in [(1, vec![1, 2]), (2, vec![2, 3])] {
            mesh.add_element(Element {
                id,
                type_name: "T3D2".into(),
                shape: ElementShape::Line2,
                nodes,
            })
            .unwrap();
        }
        mesh.parts = vec![
            Part {
                name: "A".into(),
                elements: vec![1],
            },
            Part {
                name: "B".into(),
                elements: vec![2],
            },
        ];
        mesh
    }

    fn increment(step: u32, mises: [f32; 3]) -> Increment {
        Increment {
            step,
            increment: 1,
            kind: AnalysisKind::Static,
            value: step as f64,
            fields: vec![new_field("STRESS", vec![("MISES", mises.to_vec())])],
        }
    }

    fn values<'a>(inc: &'a Increment, field: &str, component: &str) -> &'a [f32] {
        &inc.field(field)
            .unwrap()
            .component(component)
            .unwrap()
            .values
    }

    #[test]
    fn limit_per_part_takes_the_smaller_limit_on_shared_nodes() {
        let mut incs = vec![increment(1, [100.0, 200.0, 0.0])];
        let output = FieldOutput {
            name: "Limit-1".into(),
            kind: FieldOutputKind::Limit {
                field: "STRESS".into(),
                component: "MISES".into(),
                basis: LimitBasis::Parts,
                limits: vec![("A".into(), 200.0), ("B".into(), 400.0)],
            },
        };
        compute(&output, &mut incs, &mesh(), &[]).unwrap();
        assert_eq!(values(&incs[0], "Limit-1", RATIO), [0.5, 1.0, 0.0]);
        let safety = values(&incs[0], "Limit-1", SAFETY_FACTOR);
        assert_eq!(&safety[..2], [2.0, 1.0]);
        assert!(safety[2].is_nan(), "no safety factor without stress");
        // Computing again replaces the field.
        compute(&output, &mut incs, &mesh(), &[]).unwrap();
        assert_eq!(incs[0].fields.len(), 2);
    }

    #[test]
    fn zero_limit_is_rejected() {
        let mut incs = vec![increment(1, [1.0; 3])];
        let output = FieldOutput {
            name: "Limit-1".into(),
            kind: FieldOutputKind::Limit {
                field: "STRESS".into(),
                component: "MISES".into(),
                basis: LimitBasis::AllElements,
                limits: vec![(LimitBasis::AllElements.name().into(), 0.0)],
            },
        };
        assert!(compute(&output, &mut incs, &mesh(), &[]).is_err());
        assert_eq!(incs[0].fields.len(), 1);
    }

    #[test]
    fn envelope_spans_all_increments() {
        let mut incs = vec![
            increment(1, [1.0, 5.0, -2.0]),
            increment(2, [3.0, 1.0, -4.0]),
        ];
        let output = FieldOutput {
            name: "Envelope-1".into(),
            kind: FieldOutputKind::Envelope {
                field: "STRESS".into(),
                component: "MISES".into(),
            },
        };
        compute(&output, &mut incs, &mesh(), &[]).unwrap();
        for inc in &incs {
            assert_eq!(values(inc, "Envelope-1", MAX), [3.0, 5.0, -2.0]);
            assert_eq!(values(inc, "Envelope-1", MIN), [1.0, 1.0, -4.0]);
            assert_eq!(values(inc, "Envelope-1", AVERAGE), [2.0, 3.0, -3.0]);
        }
    }

    #[test]
    fn equation_combines_components_and_earlier_outputs() {
        let mut incs = vec![increment(1, [100.0, 200.0, 300.0])];
        let first = FieldOutput {
            name: "Equation-1".into(),
            kind: FieldOutputKind::Equation {
                equation: "=STRESS.MISES / 100".into(),
                unit: "/".into(),
            },
        };
        compute(&first, &mut incs, &mesh(), &[]).unwrap();
        let second = FieldOutput {
            name: "Equation-2".into(),
            kind: FieldOutputKind::Equation {
                equation: "=[Equation-1.VALUE] * 2 + STRESS.MISES".into(),
                unit: "/".into(),
            },
        };
        compute(&second, &mut incs, &mesh(), &[]).unwrap();
        assert_eq!(values(&incs[0], "Equation-2", VALUE), [102.0, 204.0, 306.0]);
        assert_eq!(second.parents(), ["Equation-1", "STRESS"]);
    }

    #[test]
    fn equation_with_unknown_component_fails() {
        let mut incs = vec![increment(1, [1.0; 3])];
        for equation in ["=STRESS.S11", "=MISES", "=1 + 2"] {
            let output = FieldOutput {
                name: "Equation-1".into(),
                kind: FieldOutputKind::Equation {
                    equation: equation.into(),
                    unit: "/".into(),
                },
            };
            assert!(
                compute(&output, &mut incs, &mesh(), &[]).is_err(),
                "{equation}"
            );
        }
    }

    fn raw_field(name: &str, components: &[(&str, [f32; 3])]) -> Field {
        let mut field = Field {
            name: name.into(),
            components: (components.iter())
                .map(|(n, v)| Component {
                    name: n.to_string(),
                    values: v.to_vec(),
                    derived: false,
                })
                .collect(),
        };
        crate::add_derived_components(&mut field);
        field
    }

    #[test]
    fn fields_turn_into_a_coordinate_system() {
        let stress = [("S11", [10.0; 3]), ("S22", [0.0; 3]), ("S33", [0.0; 3])]
            .into_iter()
            .chain([("S12", [0.0; 3]), ("S23", [0.0; 3]), ("S13", [0.0; 3])]);
        let stress: Vec<(&str, [f32; 3])> = stress.collect();
        let disp = [("U1", [1.0; 3]), ("U2", [2.0; 3]), ("U3", [3.0; 3])];
        let mut incs = vec![Increment {
            step: 1,
            increment: 1,
            kind: AnalysisKind::Static,
            value: 1.0,
            fields: vec![raw_field("STRESS", &stress), raw_field("DISP", &disp)],
        }];
        // x along global y, y along global -x.
        let mut system = CoordinateSystem::new("CS-1");
        system.point_x = [0.0, 1.0, 0.0];
        system.point_xy = [-1.0, 0.0, 0.0];
        let output = |field: &str, name: &str| FieldOutput {
            name: name.into(),
            kind: FieldOutputKind::CoordinateSystemTransform {
                field: field.into(),
                coordinate_system: "CS-1".into(),
            },
        };
        let systems = [system];
        compute(&output("STRESS", "S_CS"), &mut incs, &mesh(), &systems).unwrap();
        compute(&output("DISP", "U_CS"), &mut incs, &mesh(), &systems).unwrap();
        assert_eq!(values(&incs[0], "S_CS", "S11"), [0.0; 3]);
        assert_eq!(values(&incs[0], "S_CS", "S22"), [10.0; 3]);
        // The invariants do not change.
        assert_eq!(
            values(&incs[0], "S_CS", "MISES"),
            values(&incs[0], "STRESS", "MISES")
        );
        assert_eq!(values(&incs[0], "U_CS", "U1"), [2.0; 3]);
        assert_eq!(values(&incs[0], "U_CS", "U2"), [-1.0; 3]);
        assert_eq!(values(&incs[0], "U_CS", "U3"), [3.0; 3]);
        let missing = output("DISP", "X");
        assert!(compute(&missing, &mut incs, &mesh(), &[]).is_err());
    }
}
