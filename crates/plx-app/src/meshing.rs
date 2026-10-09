//! Meshing the CAD geometry with Gmsh: PrePoMax's meshing parameters dialog and the mesh
//! generation on a worker thread.

use std::sync::mpsc::Receiver;
use std::time::Instant;

use plx_mesher::GeneratedMesh;
use plx_model::{Geometry, MeshSetup};

use crate::numeric;

/// The open meshing parameters window with its unsaved draft.
pub struct MeshSetupWindow {
    draft: MeshSetup,
}

pub enum MeshSetupResult {
    Open,
    /// Take over the parameters and close.
    Ok(MeshSetup),
    /// Take over the parameters, close and mesh.
    Mesh(MeshSetup),
    Cancel,
}

impl MeshSetupWindow {
    pub fn new(setup: &MeshSetup) -> Self {
        Self { draft: *setup }
    }

    pub fn show(&mut self, ctx: &egui::Context) -> MeshSetupResult {
        let mut open = true;
        let mut result = MeshSetupResult::Open;
        egui::Window::new("Netzparameter")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.content_rect().center())
            .show(ctx, |ui| {
                let d = &mut self.draft;
                ui.strong("Netzgröße");
                egui::Grid::new("mesh size")
                    .num_columns(2)
                    .spacing([12.0, 6.0])
                    .show(ui, |ui| {
                        ui.label("Max. Elementgröße");
                        ui.add(numeric::drag_value(&mut d.max_size).range(0.0..=f64::MAX));
                        ui.end_row();
                        ui.label("Min. Elementgröße");
                        ui.add(numeric::drag_value(&mut d.min_size).range(0.0..=f64::MAX));
                        ui.end_row();
                        ui.label("Elemente pro Krümmungsradius")
                            .on_hover_text("0 schaltet die Verfeinerung an gekrümmten Flächen ab.");
                        ui.add(
                            numeric::drag_value(&mut d.elements_per_curvature).range(0.0..=100.0),
                        );
                        ui.end_row();
                    });
                ui.add_space(6.0);
                ui.strong("Netztyp");
                ui.checkbox(&mut d.second_order, "Zweite Ordnung (C3D10)");
                ui.add_enabled(
                    d.second_order,
                    egui::Checkbox::new(
                        &mut d.midside_nodes_on_geometry,
                        "Mittelknoten auf der Geometrie",
                    ),
                );
                ui.checkbox(&mut d.optimize, "Netz optimieren (Netgen)");
                if d.max_size <= 0.0 {
                    ui.colored_label(
                        egui::Color32::from_rgb(200, 0, 0),
                        "Die maximale Elementgröße muss größer als 0 sein.",
                    );
                }
                ui.separator();
                ui.horizontal(|ui| {
                    let valid = d.max_size > 0.0;
                    if ui
                        .add_enabled(valid, egui::Button::new("Netz erzeugen"))
                        .clicked()
                    {
                        result = MeshSetupResult::Mesh(*d);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Abbrechen").clicked() {
                            result = MeshSetupResult::Cancel;
                        }
                        if ui.add_enabled(valid, egui::Button::new("OK")).clicked() {
                            result = MeshSetupResult::Ok(*d);
                        }
                    });
                });
            });
        if !open {
            result = MeshSetupResult::Cancel;
        }
        result
    }
}

/// Mesh generation running on a worker thread.
pub struct MeshingJob {
    pub started: Instant,
    receiver: Receiver<Result<GeneratedMesh, String>>,
}

impl MeshingJob {
    pub fn start(geometry: Geometry, ctx: &egui::Context) -> Self {
        let (sender, receiver) = std::sync::mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = plx_mesher::generate_mesh(&geometry).map_err(|e| e.to_string());
            let _ = sender.send(result);
            ctx.request_repaint();
        });
        Self {
            started: Instant::now(),
            receiver,
        }
    }

    /// The result once the worker is done.
    pub fn poll(&self) -> Option<Result<GeneratedMesh, String>> {
        match self.receiver.try_recv() {
            Ok(result) => Some(result),
            Err(std::sync::mpsc::TryRecvError::Empty) => None,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                Some(Err("Vernetzung abgebrochen".into()))
            }
        }
    }
}
