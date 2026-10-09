//! Properties of a tree item as a two-column grid. The grid does not care where it is shown:
//! today in PrePoMax's double-click dialog, it fits a side panel just as well.

use plx_mesh::SurfaceDefinition;

use crate::model::Model;
use crate::results::format_value;
use crate::tree::TreeItem;

/// Title of the properties window of an item.
pub fn title(model: Option<&Model>, item: &TreeItem) -> String {
    let name = match (item, model) {
        (TreeItem::Model, Some(model)) => model.file_name(),
        (TreeItem::Model, None) => "Model".into(),
        (TreeItem::Part(index), Some(model)) => model
            .parts
            .get(*index)
            .map_or_else(String::new, |p| p.name.clone()),
        (TreeItem::NodeSet(n) | TreeItem::ElementSet(n) | TreeItem::Surface(n), _) => n.clone(),
        (TreeItem::Field(f) | TreeItem::ResultFieldOutput(f), Some(model)) => field_name(model, *f),
        (TreeItem::Component(f, c), Some(model)) => {
            let field = model
                .results
                .as_ref()
                .and_then(|v| v.current_increment())
                .and_then(|i| i.fields.get(*f));
            field
                .and_then(|field| {
                    Some(format!(
                        "{}: {}",
                        field.name,
                        field.components.get(*c)?.name
                    ))
                })
                .unwrap_or_default()
        }
        _ => String::new(),
    };
    format!("Eigenschaften: {name}")
}

fn field_name(model: &Model, field: usize) -> String {
    model
        .results
        .as_ref()
        .and_then(|v| v.current_increment())
        .and_then(|i| i.fields.get(field))
        .map_or_else(String::new, |f| f.name.clone())
}

/// Shows the properties of `item` in a grid.
pub fn show(ui: &mut egui::Ui, model: Option<&Model>, item: &TreeItem) {
    let mut rows: Vec<(&'static str, String)> = Vec::new();
    match model {
        Some(model) => rows_of(model, item, &mut rows),
        None => rows.push(("", "Kein Modell geladen".into())),
    }
    if rows.is_empty() {
        rows.push(("", "Noch nicht implementiert".into()));
    }
    egui::Grid::new("properties")
        .num_columns(2)
        .striped(true)
        .spacing([16.0, 4.0])
        .show(ui, |ui| {
            for (key, value) in rows {
                ui.label(key);
                ui.label(value);
                ui.end_row();
            }
        });
}

fn rows_of(model: &Model, item: &TreeItem, rows: &mut Vec<(&'static str, String)>) {
    let mesh = &model.mesh;
    match item {
        TreeItem::Group(_)
        | TreeItem::Mesh
        | TreeItem::FieldOutputs
        | TreeItem::Material(_)
        | TreeItem::Section(_)
        | TreeItem::Step(_)
        | TreeItem::StepGroup(..)
        | TreeItem::BoundaryCondition(..)
        | TreeItem::Load(..)
        | TreeItem::FieldOutput(..)
        | TreeItem::HistorySet(_)
        | TreeItem::HistoryField(..)
        | TreeItem::HistoryComponent(..)
        | TreeItem::Analysis => {}
        TreeItem::Model => {
            rows.push(("Datei", model.path.display().to_string()));
            rows.push(("Ladezeit", format!("{} ms", model.load_time.as_millis())));
            rows.push(("Knoten", mesh.node_count().to_string()));
            rows.push(("Elemente", mesh.element_count().to_string()));
            rows.push(("Parts", model.parts.len().to_string()));
            let mut types = std::collections::BTreeMap::<&str, usize>::new();
            for part in &model.parts {
                for (name, count) in &part.element_types {
                    *types.entry(name).or_default() += count;
                }
            }
            for (name, count) in types {
                rows.push(("Elementtyp", format!("{name} ({count})")));
            }
            if let Some((min, max)) = mesh.bounds() {
                let size = [0, 1, 2].map(|k| format_value((max[k] - min[k]) as f32));
                rows.push(("Abmessungen", size.join(" × ")));
            }
            rows.push(("Knotensets", mesh.node_sets.len().to_string()));
            rows.push(("Elementsets", mesh.element_sets.len().to_string()));
            rows.push(("Surfaces", mesh.surfaces.len().to_string()));
        }
        TreeItem::Part(index) => {
            let Some(part) = model.parts.get(*index) else {
                return;
            };
            rows.push(("Name", part.name.clone()));
            rows.push(("Elemente", part.element_count.to_string()));
            rows.push(("Knoten", part.node_count.to_string()));
            for (type_name, count) in &part.element_types {
                rows.push(("Elementtyp", format!("{type_name} ({count})")));
            }
            let [r, g, b] = part.color.map(|c| (c * 255.0).round() as u8);
            rows.push(("Farbe", format!("RGB {r}, {g}, {b}")));
            rows.push(("Sichtbar", if part.visible { "ja" } else { "nein" }.into()));
        }
        TreeItem::NodeSet(name) => {
            rows.push(("Name", name.clone()));
            let count = mesh.node_sets.get(name).map_or(0, Vec::len);
            rows.push(("Knoten", count.to_string()));
        }
        TreeItem::ElementSet(name) => {
            rows.push(("Name", name.clone()));
            let count = mesh.element_sets.get(name).map_or(0, Vec::len);
            rows.push(("Elemente", count.to_string()));
        }
        TreeItem::Surface(name) => {
            rows.push(("Name", name.clone()));
            match mesh.surfaces.get(name) {
                Some(SurfaceDefinition::ElementFaces(faces)) => {
                    rows.push(("Typ", "Elementflächen".into()));
                    rows.push(("Flächen", faces.len().to_string()));
                }
                Some(SurfaceDefinition::Nodes(nodes)) => {
                    rows.push(("Typ", "Knoten".into()));
                    rows.push(("Knoten", nodes.len().to_string()));
                }
                None => {}
            }
        }
        TreeItem::Field(f) | TreeItem::ResultFieldOutput(f) => {
            let Some(view) = &model.results else { return };
            let Some(field) = view.current_increment().and_then(|i| i.fields.get(*f)) else {
                return;
            };
            rows.push(("Name", field.name.clone()));
            let names: Vec<&str> = field.components.iter().map(|c| c.name.as_str()).collect();
            rows.push(("Komponenten", names.join(", ")));
        }
        TreeItem::Component(f, c) => {
            let Some(view) = &model.results else { return };
            let Some(inc) = view.current_increment() else {
                return;
            };
            let Some(component) = inc
                .fields
                .get(*f)
                .and_then(|field| field.components.get(*c))
            else {
                return;
            };
            rows.push((
                "Feld",
                format!("{}: {}", inc.fields[*f].name, component.name),
            ));
            rows.push((
                "Schritt, Inkrement",
                format!("{}, {}", inc.step, inc.increment),
            ));
            let extreme = |pick_max: bool| {
                component
                    .values
                    .iter()
                    .enumerate()
                    .filter(|(_, v)| v.is_finite())
                    .reduce(|a, b| {
                        let better = if pick_max { b.1 > a.1 } else { b.1 < a.1 };
                        if better { b } else { a }
                    })
            };
            for (label, pick_max) in [("Maximum", true), ("Minimum", false)] {
                if let Some((index, value)) = extreme(pick_max) {
                    let node = mesh.node_ids()[index];
                    rows.push((label, format!("{} (Knoten {node})", format_value(*value))));
                }
            }
            rows.push((
                "Berechnet",
                if component.derived {
                    "ja, von prepolix"
                } else {
                    "nein, aus der Datei"
                }
                .into(),
            ));
        }
    }
}
