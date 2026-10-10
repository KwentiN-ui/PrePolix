//! Hot spot stresses in the Results workspace: the definitions of a results file, their
//! paths in the 3D view, the evaluation, the output file and the table of values.

use std::path::{Path, PathBuf};

use glam::Vec3;
use plx_mesh::SkinFace;
use plx_model::UnitSystem;
use plx_results::hot_spot::{self, HotSpot, HotSpotPath, HotSpotReport};

use crate::model::Model;

/// The hot spots of one results file: defined on it, like its derived outputs, and
/// evaluated whenever they change.
#[derive(Default)]
pub struct HotSpots {
    pub definitions: Vec<HotSpot>,
    /// Values of the definitions, in their order.
    pub reports: Vec<HotSpotReport>,
    /// The output file, once written.
    pub file: Option<PathBuf>,
}

fn surface_faces(model: &Model) -> Vec<&SkinFace> {
    (0..model.parts.len())
        .flat_map(|part| &model.skin(part).faces)
        .collect()
}

/// The paths of a definition on the results, for showing them while it is edited or
/// selected.
pub fn paths(model: &Model, hot_spot: &HotSpot) -> Vec<HotSpotPath> {
    let hot_spot = hot_spot.in_units(units(model));
    hot_spot::hot_spot_paths(&model.mesh, surface_faces(model), &hot_spot).0
}

/// The unit system of the results, which IIW's millimetre distances are converted to.
pub fn units(model: &Model) -> UnitSystem {
    (model.results.as_ref()).map_or(model.fe.properties.units, |view| view.units)
}

/// Paths in render coordinates, deformed like the shown mesh, each starting at its toe.
pub fn render_paths<'a>(
    model: &Model,
    paths: impl IntoIterator<Item = &'a HotSpotPath>,
) -> Vec<Vec<Vec3>> {
    let position = |weights: &[(usize, f64)]| {
        weights.iter().try_fold(Vec3::ZERO, |sum, &(node, weight)| {
            Some(sum + model.node_position(node)? * weight as f32)
        })
    };
    paths
        .into_iter()
        .filter_map(|path| {
            std::iter::once(model.node_position(path.index))
                .chain(path.points.iter().map(|p| position(&p.weights)))
                .collect()
        })
        .collect()
}

/// Evaluates the hot spot definitions of the results file over all its increments.
pub fn evaluate(model: &Model) -> Vec<HotSpotReport> {
    let Some(view) = &model.results else {
        return Vec::new();
    };
    let definitions: Vec<HotSpot> = (model.hot_spots.definitions.iter())
        .map(|h| h.in_units(units(model)))
        .collect();
    hot_spot::evaluate(
        &model.mesh,
        &surface_faces(model),
        &definitions,
        &view.increments,
    )
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
        .map_err(|e| format!("{} not written: {e}", path.display()))?;
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
                    "{}: max. {} = {:.2} at node {} (Step {}, Increment {})",
                    report.name,
                    report.component.short(),
                    value.hot_spot,
                    value.node,
                    increment.step,
                    increment.increment
                ),
                None => format!("{}: no values", report.name),
            }
        })
        .collect()
}

/// The table of the values in the shown increment. Returns false when it was closed.
pub fn window(ctx: &egui::Context, evaluation: &HotSpots, step: Option<(u32, u32)>) -> bool {
    let mut open = true;
    egui::Window::new("Hot Spot Stresses")
        .open(&mut open)
        .collapsible(false)
        .default_size([520.0, 320.0])
        .pivot(egui::Align2::RIGHT_TOP)
        .default_pos(ctx.content_rect().right_top() + egui::vec2(-20.0, 90.0))
        .show(ctx, |ui| {
            if let Some(file) = &evaluation.file {
                ui.label(format!("All increments in {}", file.display()));
            }
            match step {
                Some((step, increment)) => {
                    ui.label(format!("Shown: Step {step}, Increment {increment}"))
                }
                None => ui.label("No increment selected"),
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
                            ui.weak("No stresses in this increment.");
                            ui.add_space(8.0);
                            continue;
                        };
                        if let Some(maximum) = values.maximum() {
                            ui.label(format!(
                                "Maximum: S_hs = {:.2} at node {}",
                                maximum.hot_spot, maximum.node
                            ));
                        }
                        let maximum = values.maximum().map(|m| m.node);
                        egui::Grid::new(("hot spot table", r))
                            .striped(true)
                            .num_columns(report.distances.len() + 2)
                            .show(ui, |ui| {
                                ui.strong("Node");
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
    use plx_model::Region;
    use plx_results::hot_spot::{Extrapolation, HotSpotComponent};

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
        let mut results = load(&testdata("kragbalken_c3d20r.frd"), UnitSystem::MmTonSC)
            .unwrap()
            .model;
        let toe = [40.0, 50.0, 60.0].map(|x| node_at(&results, [x, 5.0, 10.0]));
        for (i, extrapolation) in Extrapolation::IIW[..3].iter().enumerate() {
            results.hot_spots.definitions.push(HotSpot {
                toe: Region::Nodes(toe.to_vec()),
                direction: [1.0, 0.0, 0.0],
                thickness: 10.0,
                extrapolation: extrapolation.clone(),
                component: HotSpotComponent::Perpendicular,
                ..HotSpot::new(format!("Hot_Spot-{}", i + 1))
            });
        }
        let reports = evaluate(&results);
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
        // The paths start at the toe and run along the top in +x.
        let paths = paths(&results, &results.hot_spots.definitions[0]);
        assert_eq!(paths.len(), 3);
        assert_eq!(render_paths(&results, &paths)[0].len(), 3);
    }

    #[test]
    fn output_file_sits_next_to_the_results() {
        let path = output_path(Path::new("/tmp/work/Analysis-1.frd"));
        assert_eq!(path, Path::new("/tmp/work/Analysis-1_hot_spots.csv"));
    }
}
