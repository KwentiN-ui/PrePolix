//! Hot spot stresses in the GUI: the paths in the 3D view, the evaluation of a results file
//! with the hot spots of the FE model, the output file and the table of values.

use std::path::{Path, PathBuf};

use glam::Vec3;
use plx_mesh::SkinFace;
use plx_model::{HotSpot, Region};
use plx_results::hot_spot::{self, HotSpotReport};

use crate::model::Model;

/// Hot spot values of one results file.
pub struct Evaluation {
    pub reports: Vec<HotSpotReport>,
    /// The output file, once written.
    pub file: Option<PathBuf>,
}

fn surface_faces(model: &Model) -> Vec<&SkinFace> {
    (0..model.parts.len())
        .flat_map(|part| &model.skin(part).faces)
        .collect()
}

/// The paths of a definition on the model in render coordinates, each starting at its toe,
/// for showing them while the definition is edited or selected.
pub fn preview(model: &Model, hot_spot: &HotSpot) -> Vec<Vec<Vec3>> {
    let (paths, _) = hot_spot::hot_spot_paths(&model.mesh, surface_faces(model), hot_spot);
    paths
        .iter()
        .map(|path| {
            // The path moves with its part in an exploded view.
            let offset = model.explosion_offset(path.index).as_vec3();
            std::iter::once(path.position)
                .chain(path.points.iter().map(|p| p.position))
                .map(|p| model.to_render(p) + offset)
                .collect()
        })
        .collect()
}

/// The paths of the evaluated hot spots on the shown results, deformed like the mesh.
pub fn result_paths(model: &Model, evaluation: &Evaluation) -> Vec<Vec<Vec3>> {
    let position = |weights: &[(usize, f64)]| {
        weights.iter().try_fold(Vec3::ZERO, |sum, &(node, weight)| {
            Some(sum + model.node_position(node)? * weight as f32)
        })
    };
    (evaluation.reports.iter())
        .flat_map(|r| &r.paths)
        .filter_map(|path| {
            std::iter::once(model.node_position(path.index))
                .chain(path.points.iter().map(|p| position(&p.weights)))
                .collect()
        })
        .collect()
}

/// Evaluates the hot spots of the FE model `fe` on `results`, a results file of it.
pub fn evaluate(fe: &Model, results: &Model) -> Result<Vec<HotSpotReport>, String> {
    let view = results
        .results
        .as_ref()
        .ok_or("Keine Ergebnisse geladen.")?;
    // The results file has no sets, so toe regions become their nodes; they must be the
    // same nodes in the same places.
    let size = fe.mesh.bounds().map_or(1.0, |(min, max)| {
        (0..3).map(|k| max[k] - min[k]).fold(0.0, f64::max)
    });
    let mut definitions = Vec::with_capacity(fe.fe.hot_spots.len());
    for hot_spot in &fe.fe.hot_spots {
        let nodes = hot_spot.toe.nodes(&fe.mesh);
        for &id in &nodes {
            let here = fe.mesh.node(id);
            let there = results.mesh.node(id);
            let moved = match (here, there) {
                (Some(a), Some(b)) => (0..3).any(|k| (a[k] - b[k]).abs() > 1e-6 * size),
                _ => true,
            };
            if moved {
                return Err(format!(
                    "{} passt nicht zum Modell: Knoten {id} von {} fehlt oder liegt anders.",
                    results.file_name(),
                    hot_spot.name
                ));
            }
        }
        definitions.push(HotSpot {
            toe: Region::Nodes(nodes),
            ..hot_spot.clone()
        });
    }
    let faces = surface_faces(results);
    Ok(hot_spot::evaluate(
        &results.mesh,
        &faces,
        &definitions,
        &view.increments,
    ))
}

/// Where the values of a results file are written: next to it, e.g. `Analysis-1.frd` gives
/// `Analysis-1_hot_spots.csv`.
pub fn output_path(results: &Path) -> PathBuf {
    let stem = results
        .file_stem()
        .map_or_else(|| "results".into(), |s| s.to_string_lossy());
    results.with_file_name(format!("{stem}_hot_spots.csv"))
}

pub fn write(results: &Path, reports: &[HotSpotReport]) -> Result<PathBuf, String> {
    let path = output_path(results);
    std::fs::write(&path, hot_spot::to_csv(reports))
        .map_err(|e| format!("{} nicht geschrieben: {e}", path.display()))?;
    Ok(path)
}

/// One line per definition with its largest value over all increments.
pub fn summary(reports: &[HotSpotReport]) -> Vec<String> {
    reports
        .iter()
        .map(|report| {
            let maximum = (report.increments.iter())
                .filter_map(|i| Some((i, i.maximum()?)))
                .max_by(|a, b| a.1.hot_spot.total_cmp(&b.1.hot_spot));
            match maximum {
                Some((increment, value)) => format!(
                    "{}: max. {} = {:.2} an Knoten {} (Step {}, Inkrement {})",
                    report.name,
                    report.component.short(),
                    value.hot_spot,
                    value.node,
                    increment.step,
                    increment.increment
                ),
                None => format!("{}: keine Werte", report.name),
            }
        })
        .collect()
}

/// The table of the values in the shown increment. Returns false when it was closed.
pub fn window(ctx: &egui::Context, evaluation: &Evaluation, step: Option<(u32, u32)>) -> bool {
    let mut open = true;
    egui::Window::new("Hot-Spot-Spannungen")
        .open(&mut open)
        .collapsible(false)
        .default_size([520.0, 320.0])
        .pivot(egui::Align2::RIGHT_TOP)
        .default_pos(ctx.content_rect().right_top() + egui::vec2(-20.0, 90.0))
        .show(ctx, |ui| {
            if let Some(file) = &evaluation.file {
                ui.label(format!("Alle Inkremente in {}", file.display()));
            }
            match step {
                Some((step, increment)) => {
                    ui.label(format!("Gezeigt: Step {step}, Inkrement {increment}"))
                }
                None => ui.label("Kein Inkrement gewählt"),
            };
            ui.separator();
            egui::ScrollArea::both()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for (r, report) in evaluation.reports.iter().enumerate() {
                        ui.strong(&report.name);
                        ui.label(format!("{}, {}", report.method, report.component.short()));
                        for warning in &report.warnings {
                            ui.colored_label(egui::Color32::from_rgb(170, 90, 0), warning);
                        }
                        let values = (report.increments.iter())
                            .find(|i| Some((i.step, i.increment)) == step);
                        let Some(values) = values else {
                            ui.weak("Keine Spannungen in diesem Inkrement.");
                            ui.add_space(8.0);
                            continue;
                        };
                        if let Some(maximum) = values.maximum() {
                            ui.label(format!(
                                "Maximum: S_hs = {:.2} an Knoten {}",
                                maximum.hot_spot, maximum.node
                            ));
                        }
                        let maximum = values.maximum().map(|m| m.node);
                        egui::Grid::new(("hot spot table", r))
                            .striped(true)
                            .num_columns(report.distances.len() + 2)
                            .show(ui, |ui| {
                                ui.strong("Knoten");
                                for d in &report.distances {
                                    ui.strong(format!("S({})", short(*d)));
                                }
                                ui.strong("S_hs");
                                ui.end_row();
                                for value in &values.values {
                                    ui.label(value.node.to_string());
                                    for s in &value.readings {
                                        ui.label(format!("{s:.2}"));
                                    }
                                    let text = format!("{:.2}", value.hot_spot);
                                    if Some(value.node) == maximum {
                                        ui.strong(text);
                                    } else {
                                        ui.label(text);
                                    }
                                    ui.end_row();
                                }
                            });
                        ui.add_space(8.0);
                    }
                });
        });
    open
}

/// A distance without float noise, e.g. 4.8 instead of 4.800000000000001.
pub fn short(value: f64) -> String {
    let text = format!("{value:.4}");
    text.trim_end_matches('0').trim_end_matches('.').to_string()
}

#[cfg(test)]
mod tests {
    use plx_model::{Extrapolation, HotSpotComponent};

    use super::*;
    use crate::model::load;

    fn testdata(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../testdata")
            .join(name)
    }

    /// Node of the beam mesh at a point.
    fn node_at(model: &Model, point: [f64; 3]) -> u32 {
        let index = (model.mesh.coords().iter())
            .position(|p| (0..3).all(|k| (p[k] - point[k]).abs() < 1e-9))
            .unwrap();
        model.mesh.node_ids()[index]
    }

    #[test]
    fn cantilever_hot_spot_matches_beam_theory() {
        // Cantilever 100 x 10 x 10, 100 N across at its end, solved by CalculiX 2.21 with
        // C3D20R. On top the bending stress rises linearly towards the support:
        // S11 = F (100 - x) (h / 2) / I = 0.6 (100 - x), so 30 at x = 50.
        let mut fe = load(&testdata("kragbalken_c3d20r.inp")).unwrap().model;
        let toe = [40.0, 50.0, 60.0].map(|x| node_at(&fe, [x, 5.0, 10.0]));
        for (i, extrapolation) in Extrapolation::IIW[..3].iter().enumerate() {
            fe.fe.hot_spots.push(HotSpot {
                toe: Region::Nodes(toe.to_vec()),
                direction: [1.0, 0.0, 0.0],
                thickness: 10.0,
                extrapolation: extrapolation.clone(),
                component: HotSpotComponent::Perpendicular,
                ..HotSpot::new(format!("Hot_Spot-{}", i + 1))
            });
        }
        let results = load(&testdata("kragbalken_c3d20r.frd")).unwrap().model;
        let reports = evaluate(&fe, &results).unwrap();
        assert_eq!(reports.len(), 3);
        for report in &reports {
            assert!(report.warnings.is_empty(), "{:?}", report.warnings);
            let values = &report.increments[0].values;
            assert_eq!(values.len(), 3);
            for (value, x) in values.iter().zip([40.0, 50.0, 60.0]) {
                let beam = 0.6 * (100.0 - x);
                let error = (value.hot_spot - beam).abs() / beam;
                assert!(error < 0.01, "{}: {value:?} vs {beam}", report.name);
            }
        }
        let csv = hot_spot::to_csv(&reports);
        assert_eq!(csv.lines().count(), 1 + 3 * 3);
    }

    #[test]
    fn results_of_another_mesh_are_refused() {
        let mut fe = load(&testdata("kragbalken_c3d8.inp")).unwrap().model;
        fe.fe.hot_spots.push(HotSpot {
            toe: Region::Nodes(vec![1, 3]),
            ..HotSpot::new("Hot_Spot-1")
        });
        // Node 1 is at the origin in both meshes, node 3 is not.
        let results = load(&testdata("block_c3d20r.frd")).unwrap().model;
        let error = evaluate(&fe, &results).unwrap_err();
        assert!(error.contains("Knoten 3"), "{error}");
    }

    #[test]
    fn output_file_sits_next_to_the_results() {
        let path = output_path(Path::new("/tmp/work/Analysis-1.frd"));
        assert_eq!(path, Path::new("/tmp/work/Analysis-1_hot_spots.csv"));
    }
}
