//! History outputs the user derives from the results, PrePoMax's "Create History Output":
//! values of field components at nodes over all increments, an equation of other history
//! outputs, or the size of elements or element faces on the deformed mesh.
//!
//! A history output gives a set of fields with components; every component has one entry
//! per node, element or face, with one value per increment.

use plx_mesh::{ElementId, FeMesh};
use plx_model::Region;

use crate::equation::Equation;
use crate::{Increment, field_output};

/// Field and component names PrePoMax gives computed history outputs.
pub const EQUATION_FIELD: &str = "EQUATION";
pub const SIZE_FIELD: &str = "ELEMENT_SIZE";
pub const VOLUME: &str = "VOLUME";
pub const AREA: &str = "AREA";

/// What the element size measures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SizeKind {
    /// Volume of solid elements.
    Volume,
    /// Area of element faces.
    Area,
}

#[derive(Clone, Debug, PartialEq)]
pub enum HistoryOutputKind {
    /// The components of a field at the nodes of a region.
    FromField {
        region: Region,
        field: String,
        components: Vec<String>,
    },
    /// An equation of components of other history outputs, entry by entry.
    FromEquation { equation: String, unit: String },
    /// Volume of the elements or area of the faces of a region, on the deformed mesh.
    ElementSize { region: Region, kind: SizeKind },
}

#[derive(Clone, Debug, PartialEq)]
pub struct HistoryOutput {
    pub name: String,
    pub kind: HistoryOutputKind,
}

impl HistoryOutput {
    pub fn unit(&self) -> Option<&str> {
        match &self.kind {
            HistoryOutputKind::FromEquation { unit, .. } => Some(unit),
            _ => None,
        }
    }
}

/// One column: a node, element or face, with a value per increment.
#[derive(Clone, Debug, PartialEq)]
pub struct HistoryEntry {
    pub name: String,
    pub values: Vec<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct HistoryComponent {
    pub name: String,
    pub entries: Vec<HistoryEntry>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct HistoryField {
    pub name: String,
    pub components: Vec<HistoryComponent>,
}

impl HistoryField {
    pub fn component(&self, name: &str) -> Option<&HistoryComponent> {
        self.components.iter().find(|c| c.name == name)
    }
}

/// The computed data of a history output.
#[derive(Clone, Debug, PartialEq)]
pub struct HistorySet {
    pub name: String,
    /// Step, increment and time (or frequency, buckling factor) of every row.
    pub rows: Vec<(u32, u32, f64)>,
    pub fields: Vec<HistoryField>,
}

impl HistorySet {
    pub fn field(&self, name: &str) -> Option<&HistoryField> {
        self.fields.iter().find(|f| f.name == name)
    }
}

fn rows(increments: &[Increment]) -> Vec<(u32, u32, f64)> {
    increments
        .iter()
        .map(|i| (i.step, i.increment, i.value))
        .collect()
}

/// Computes a history output. `sets` are the history outputs computed so far, which an
/// equation may use.
pub fn compute(
    output: &HistoryOutput,
    increments: &[Increment],
    mesh: &FeMesh,
    sets: &[HistorySet],
) -> Result<HistorySet, String> {
    let fields = match &output.kind {
        HistoryOutputKind::FromField {
            region,
            field,
            components,
        } => vec![from_field(region, field, components, increments, mesh)?],
        HistoryOutputKind::FromEquation { equation, .. } => {
            vec![from_equation(&output.name, equation, sets)?]
        }
        HistoryOutputKind::ElementSize { region, kind } => {
            vec![element_size(region, *kind, increments, mesh)?]
        }
    };
    Ok(HistorySet {
        name: output.name.clone(),
        rows: rows(increments),
        fields,
    })
}

fn from_field(
    region: &Region,
    field: &str,
    components: &[String],
    increments: &[Increment],
    mesh: &FeMesh,
) -> Result<HistoryField, String> {
    let nodes = region.nodes(mesh);
    if nodes.is_empty() {
        return Err("The region contains no nodes.".into());
    }
    if components.is_empty() {
        return Err("Please select at least one component.".into());
    }
    let indices: Vec<Option<usize>> = nodes.iter().map(|&n| mesh.node_index(n)).collect();
    let mut result = Vec::new();
    for name in components {
        if !increments
            .iter()
            .any(|i| i.field(field).and_then(|f| f.component(name)).is_some())
        {
            return Err(format!("The result {field}.{name} exists in no increment."));
        }
        let entries = nodes
            .iter()
            .zip(&indices)
            .map(|(id, index)| HistoryEntry {
                name: id.to_string(),
                values: increments
                    .iter()
                    .map(|inc| {
                        let values = inc.field(field).and_then(|f| f.component(name));
                        match (values, index) {
                            (Some(c), Some(i)) => c.values.get(*i).map_or(f64::NAN, |&v| v as f64),
                            _ => f64::NAN,
                        }
                    })
                    .collect(),
            })
            .collect();
        result.push(HistoryComponent {
            name: name.clone(),
            entries,
        });
    }
    Ok(HistoryField {
        name: field.to_string(),
        components: result,
    })
}

fn from_equation(own: &str, equation: &str, sets: &[HistorySet]) -> Result<HistoryField, String> {
    let parsed = Equation::parse(equation)?;
    let mut sources: Vec<&HistoryComponent> = Vec::new();
    for variable in parsed.variables() {
        let parts: Vec<&str> = variable.splitn(3, '.').collect();
        let [set, field, component] = parts[..] else {
            return Err(format!(
                "{variable}: History outputs are given as Name.Field.Component, \
                 e.g. History-1.STRESS.MISES."
            ));
        };
        if set == own {
            return Err(format!("{own} cannot use itself."));
        }
        let set = (sets.iter().find(|s| s.name == set))
            .ok_or_else(|| format!("The history output {set} does not exist."))?;
        let component = (set.field(field).and_then(|f| f.component(component)))
            .ok_or_else(|| format!("{variable} does not exist."))?;
        sources.push(component);
    }
    let (columns, rows) = match sources.first() {
        Some(first) => (
            first.entries.len(),
            first.entries.first().map_or(0, |e| e.values.len()),
        ),
        None => (1, 1),
    };
    for source in &sources {
        if source.entries.len() != columns {
            return Err(
                "All components of the equation must have the same number of entries (columns)."
                    .into(),
            );
        }
        if source.entries.iter().any(|e| e.values.len() != rows) {
            return Err(
                "All components of the equation must have the same number of increments (rows)."
                    .into(),
            );
        }
    }
    // PrePoMax keeps the entry names when all components share them, else numbers them.
    let names: Vec<String> = match sources.first() {
        Some(first)
            if sources.iter().all(|s| {
                s.entries
                    .iter()
                    .zip(&first.entries)
                    .all(|(a, b)| a.name == b.name)
            }) =>
        {
            first.entries.iter().map(|e| e.name.clone()).collect()
        }
        _ => (1..=columns).map(|i| i.to_string()).collect(),
    };
    let mut args = vec![0.0; sources.len()];
    let entries = names
        .into_iter()
        .enumerate()
        .map(|(column, name)| HistoryEntry {
            name,
            values: (0..rows)
                .map(|row| {
                    for (arg, source) in args.iter_mut().zip(&sources) {
                        *arg = source.entries[column].values[row];
                    }
                    parsed.evaluate(&args)
                })
                .collect(),
        })
        .collect();
    Ok(HistoryField {
        name: EQUATION_FIELD.into(),
        components: vec![HistoryComponent {
            name: field_output::VALUE.into(),
            entries,
        }],
    })
}

/// Area of a polygon given as a closed ring of points, fanned around its centroid; also the
/// signed volume of the cone from `apex` over it.
fn ring_area_and_volume(ring: &[glam::DVec3], apex: glam::DVec3) -> (f64, f64) {
    let center = ring.iter().copied().sum::<glam::DVec3>() / ring.len() as f64;
    let mut area = 0.0;
    let mut volume = 0.0;
    for (i, &a) in ring.iter().enumerate() {
        let b = ring[(i + 1) % ring.len()];
        let cross = (a - center).cross(b - center);
        area += cross.length() / 2.0;
        volume += (center - apex).dot((a - apex).cross(b - apex)) / 6.0;
    }
    (area, volume)
}

/// Corners and midside nodes of a face as a ring: c0, m0, c1, m1, ...
fn face_ring(
    element: &plx_mesh::Element,
    face: &plx_mesh::FaceTopology,
    position: &dyn Fn(u32) -> Option<glam::DVec3>,
) -> Option<Vec<glam::DVec3>> {
    let quadratic = element.shape.is_quadratic();
    let mut ring = Vec::new();
    for (i, &corner) in face.corners.iter().enumerate() {
        ring.push(position(*element.nodes.get(corner)?)?);
        if quadratic && let Some(&mid) = face.mids.get(i) {
            ring.push(position(*element.nodes.get(mid)?)?);
        }
    }
    Some(ring)
}

fn element_size(
    region: &Region,
    kind: SizeKind,
    increments: &[Increment],
    mesh: &FeMesh,
) -> Result<HistoryField, String> {
    // Entries: whole elements for volumes, faces for areas.
    let items: Vec<(ElementId, Option<u8>)> = match kind {
        SizeKind::Volume => region
            .elements(mesh)
            .into_iter()
            .filter(|&e| {
                mesh.element(e)
                    .is_some_and(|e| e.shape.family() == plx_mesh::ElementFamily::Solid)
            })
            .map(|e| (e, None))
            .collect(),
        SizeKind::Area => {
            let faces = region.faces(mesh);
            if faces.is_empty() {
                // Shell elements are their own face.
                region
                    .elements(mesh)
                    .into_iter()
                    .filter(|&e| {
                        mesh.element(e)
                            .is_some_and(|e| e.shape.family() == plx_mesh::ElementFamily::Surface)
                    })
                    .map(|e| (e, Some(1)))
                    .collect()
            } else {
                faces.into_iter().map(|(e, f)| (e, Some(f))).collect()
            }
        }
    };
    if items.is_empty() {
        return Err(match kind {
            SizeKind::Volume => "The region contains no solid elements.".into(),
            SizeKind::Area => "The region contains no element faces.".into(),
        });
    }
    let coords = mesh.coords();
    let mut entries: Vec<HistoryEntry> = items
        .iter()
        .map(|(element, face)| HistoryEntry {
            name: match face {
                Some(face) if kind == SizeKind::Area => format!("{element}-S{face}"),
                _ => element.to_string(),
            },
            values: Vec::with_capacity(increments.len()),
        })
        .collect();
    for increment in increments {
        let displacements = increment.displacements();
        let position = |id: u32| {
            let index = mesh.node_index(id)?;
            let mut p = glam::DVec3::from(coords[index]);
            if let Some(d) = displacements.as_ref().and_then(|d| d.get(index)) {
                p += glam::DVec3::new(d[0] as f64, d[1] as f64, d[2] as f64);
            }
            Some(p)
        };
        for ((element, face), entry) in items.iter().zip(&mut entries) {
            let value = mesh.element(*element).and_then(|element| {
                let faces = element.shape.faces();
                match face {
                    Some(face) => {
                        let topology = faces.get(usize::from(*face).checked_sub(1)?)?;
                        let ring = face_ring(element, topology, &position)?;
                        Some(ring_area_and_volume(&ring, glam::DVec3::ZERO).0)
                    }
                    None => {
                        let rings: Vec<Vec<glam::DVec3>> = (faces.iter())
                            .map(|f| face_ring(element, f, &position))
                            .collect::<Option<_>>()?;
                        let apex = rings.iter().flatten().copied().sum::<glam::DVec3>()
                            / rings.iter().map(Vec::len).sum::<usize>() as f64;
                        let volume: f64 = (rings.iter())
                            .map(|ring| ring_area_and_volume(ring, apex).1)
                            .sum();
                        Some(volume.abs())
                    }
                }
            });
            entry.values.push(value.unwrap_or(f64::NAN));
        }
    }
    Ok(HistoryField {
        name: SIZE_FIELD.into(),
        components: vec![HistoryComponent {
            name: match kind {
                SizeKind::Volume => VOLUME,
                SizeKind::Area => AREA,
            }
            .into(),
            entries,
        }],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AnalysisKind, Component, Field};
    use plx_mesh::{Element, ElementShape, Part};

    /// A unit cube C3D8 whose top moves up by `top` in the second increment.
    fn cube() -> (FeMesh, Vec<Increment>) {
        let mut mesh = FeMesh::default();
        let corners = [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
            [1.0, 0.0, 1.0],
            [1.0, 1.0, 1.0],
            [0.0, 1.0, 1.0],
        ];
        for (i, c) in corners.iter().enumerate() {
            mesh.set_node(i as u32 + 1, *c);
        }
        mesh.add_element(Element {
            id: 1,
            type_name: "C3D8".into(),
            shape: ElementShape::Hex8,
            nodes: (1..=8).collect(),
        })
        .unwrap();
        mesh.parts.push(Part {
            name: "CUBE".into(),
            elements: vec![1],
        });
        let disp = |top: f32| {
            let column = |k: usize, name: &str| Component {
                name: name.into(),
                values: corners
                    .iter()
                    .map(|c| if k == 2 && c[2] > 0.5 { top } else { 0.0 })
                    .collect(),
                derived: false,
            };
            Field {
                name: "DISP".into(),
                components: vec![column(0, "U1"), column(1, "U2"), column(2, "U3")],
            }
        };
        let increments = [0.0, 1.0]
            .iter()
            .enumerate()
            .map(|(i, &top)| Increment {
                step: 1,
                increment: i as u32 + 1,
                kind: AnalysisKind::Static,
                value: i as f64 + 1.0,
                fields: vec![disp(top)],
            })
            .collect();
        (mesh, increments)
    }

    #[test]
    fn field_values_at_nodes_over_increments() {
        let (mesh, increments) = cube();
        let output = HistoryOutput {
            name: "History-1".into(),
            kind: HistoryOutputKind::FromField {
                region: Region::Nodes(vec![7, 1]),
                field: "DISP".into(),
                components: vec!["U3".into()],
            },
        };
        let set = compute(&output, &increments, &mesh, &[]).unwrap();
        let u3 = set.field("DISP").unwrap().component("U3").unwrap();
        let names: Vec<&str> = u3.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["1", "7"]);
        assert_eq!(u3.entries[1].values, [0.0, 1.0]);
        assert_eq!(set.rows, [(1, 1, 1.0), (1, 2, 2.0)]);
    }

    #[test]
    fn element_volume_and_face_area_follow_the_deformation() {
        let (mesh, increments) = cube();
        let volume = HistoryOutput {
            name: "Size-1".into(),
            kind: HistoryOutputKind::ElementSize {
                region: Region::Parts(vec!["CUBE".into()]),
                kind: SizeKind::Volume,
            },
        };
        let set = compute(&volume, &increments, &mesh, &[]).unwrap();
        let values = &set.field(SIZE_FIELD).unwrap().components[0].entries[0].values;
        assert!((values[0] - 1.0).abs() < 1e-12 && (values[1] - 2.0).abs() < 1e-12);
        // Face S3 (nodes 1, 5, 6, 2) is the side y = 0, stretched to height 2.
        let area = HistoryOutput {
            name: "Size-2".into(),
            kind: HistoryOutputKind::ElementSize {
                region: Region::Faces(vec![(1, 3)]),
                kind: SizeKind::Area,
            },
        };
        let set = compute(&area, &increments, &mesh, &[]).unwrap();
        let entry = &set.field(SIZE_FIELD).unwrap().components[0].entries[0];
        assert_eq!(entry.name, "1-S3");
        assert!((entry.values[0] - 1.0).abs() < 1e-12 && (entry.values[1] - 2.0).abs() < 1e-12);
    }

    #[test]
    fn equation_combines_history_outputs_entry_by_entry() {
        let (mesh, increments) = cube();
        let first = HistoryOutput {
            name: "History-1".into(),
            kind: HistoryOutputKind::FromField {
                region: Region::Nodes(vec![5, 6]),
                field: "DISP".into(),
                components: vec!["U3".into()],
            },
        };
        let sets = vec![compute(&first, &increments, &mesh, &[]).unwrap()];
        let equation = HistoryOutput {
            name: "Equation-1".into(),
            kind: HistoryOutputKind::FromEquation {
                equation: "=[History-1.DISP.U3] * 10 + 1".into(),
                unit: "/".into(),
            },
        };
        let set = compute(&equation, &increments, &mesh, &sets).unwrap();
        let value = set
            .field(EQUATION_FIELD)
            .unwrap()
            .component("VALUE")
            .unwrap();
        assert_eq!(value.entries[0].name, "5");
        assert_eq!(value.entries[1].values, [1.0, 11.0]);
        for wrong in ["=History-2.DISP.U3", "=DISP.U3", "=[History-1.DISP.U1]"] {
            let output = HistoryOutput {
                name: "Equation-1".into(),
                kind: HistoryOutputKind::FromEquation {
                    equation: wrong.into(),
                    unit: "/".into(),
                },
            };
            assert!(
                compute(&output, &increments, &mesh, &sets).is_err(),
                "{wrong}"
            );
        }
    }
}
