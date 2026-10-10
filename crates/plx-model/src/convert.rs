//! Converting the values of a model into another unit system, so that the model stays the
//! same physically when its unit system changes.

use crate::units::{Quantity, UnitSystem};
use crate::{
    BoundaryKind, Constraint, FeModel, GapConductance, Geometry, InitialConditionKind,
    InteractionProperty, LoadKind, Material, MeshSetupKind, MeshingParameters, StaticStep,
    StepKind, SurfaceBehavior,
};

/// Converts values from one unit system into another.
#[derive(Clone, Copy, Debug)]
pub struct Conversion {
    pub from: UnitSystem,
    pub to: UnitSystem,
}

impl Conversion {
    pub fn new(from: UnitSystem, to: UnitSystem) -> Self {
        Self { from, to }
    }

    pub fn value(&self, value: &mut f64, quantity: Quantity) {
        *value = self.from.convert(*value, quantity, self.to);
    }

    pub fn option(&self, value: &mut Option<f64>, quantity: Quantity) {
        if let Some(value) = value {
            self.value(value, quantity);
        }
    }

    pub fn all(&self, values: &mut [f64], quantity: Quantity) {
        for value in values {
            self.value(value, quantity);
        }
    }

    /// Factor of lengths, by which the mesh and the geometry are scaled.
    pub fn length_factor(&self) -> f64 {
        self.from.factor_to(Quantity::Length, self.to)
    }
}

impl FeModel {
    /// Converts every value of the model into the unit system `to` and makes it the model's.
    /// The mesh and the geometry are not part of the FE model; they scale by
    /// [`Conversion::length_factor`]. Returns what could not be converted.
    pub fn convert_units(&mut self, to: UnitSystem) -> Vec<String> {
        let c = Conversion::new(self.properties.units, to);
        self.properties.units = to;
        let mut notes = Vec::new();
        if !c.from.has_units() || !c.to.has_units() {
            return notes;
        }
        for material in &mut self.materials {
            material.convert_units(&c);
        }
        for section in &mut self.sections {
            c.value(&mut section.thickness, Quantity::Length);
        }
        for constraint in &mut self.constraints {
            constraint.convert_units(&c);
        }
        for interaction in &mut self.surface_interactions {
            for property in &mut interaction.properties {
                property.convert_units(&c);
            }
        }
        for pair in &mut self.contact_pairs {
            c.option(&mut pair.adjustment_size, Quantity::Length);
        }
        c.option(&mut self.properties.absolute_zero, Quantity::Temperature);
        c.option(
            &mut self.properties.stefan_boltzmann,
            Quantity::StefanBoltzmann,
        );
        for condition in &mut self.initial_conditions {
            match &mut condition.kind {
                InitialConditionKind::Temperature(t) => c.value(t, Quantity::Temperature),
            }
        }
        for step in &mut self.steps {
            match &mut step.kind {
                StepKind::Static(s) => s.convert_units(&c),
                StepKind::HeatTransfer(h) | StepKind::CoupledTempDisp(h) => {
                    h.increments.convert_units(&c);
                    c.option(&mut h.deltmx, Quantity::TemperatureDifference);
                }
                StepKind::Frequency(f) => {
                    c.option(&mut f.lower_frequency, Quantity::Frequency);
                    c.option(&mut f.upper_frequency, Quantity::Frequency);
                }
                StepKind::ComplexFrequency(_) => {}
                // Buckling factors and the accuracy have no unit.
                StepKind::Buckle(_) => {}
            }
            for bc in &mut step.boundary_conditions {
                match &mut bc.kind {
                    BoundaryKind::Fixed | BoundaryKind::Submodel { .. } => {}
                    BoundaryKind::Displacement(values) => {
                        for (i, value) in values.iter_mut().enumerate() {
                            let quantity = if i < 3 {
                                Quantity::Length
                            } else {
                                Quantity::Angle
                            };
                            c.option(value, quantity);
                        }
                    }
                    BoundaryKind::Temperature(t) => c.value(t, Quantity::Temperature),
                }
            }
            for load in &mut step.loads {
                match &mut load.kind {
                    LoadKind::ConcentratedForce(force) | LoadKind::SurfaceTraction(force) => {
                        c.all(force, Quantity::Force)
                    }
                    LoadKind::Pressure(pressure) => c.value(pressure, Quantity::Pressure),
                    LoadKind::ConcentratedFlux(flux) => c.value(flux, Quantity::Power),
                    LoadKind::SurfaceFlux(flux) => c.value(flux, Quantity::HeatFlux),
                    LoadKind::BodyFlux(flux) => c.value(flux, Quantity::PowerPerVolume),
                    LoadKind::Film { sink, coefficient } => {
                        c.value(sink, Quantity::Temperature);
                        c.value(coefficient, Quantity::HeatTransferCoefficient);
                    }
                    LoadKind::Radiation { sink, .. } => c.value(sink, Quantity::Temperature),
                    LoadKind::Gravity(acceleration) => c.all(acceleration, Quantity::Acceleration),
                    LoadKind::Centrifugal { point, speed, .. } => {
                        c.all(point, Quantity::Length);
                        c.value(speed, Quantity::RotationalSpeed);
                    }
                }
            }
        }
        for amplitude in &mut self.amplitudes {
            c.value(&mut amplitude.shift_time, Quantity::Time);
            for point in &mut amplitude.points {
                c.value(&mut point[0], Quantity::Time);
            }
        }
        // An amplitude scales a temperature from the zero of its unit, which moves with
        // another temperature unit such as Kelvin instead of degrees Celsius.
        let zero_moves = c.from.convert(0.0, Quantity::Temperature, c.to) != 0.0;
        let scaled_temperature = self.steps.iter().any(|step| {
            (step.boundary_conditions.iter()).any(|b| b.amplitude.is_some() && b.kind.is_thermal())
                || (step.loads.iter()).any(|l| {
                    l.amplitude.is_some()
                        && matches!(l.kind, LoadKind::Film { .. } | LoadKind::Radiation { .. })
                })
        });
        if zero_moves && scaled_temperature {
            notes.push(
                "Temperaturen mit Amplitude beziehen sich jetzt auf einen anderen Nullpunkt; \
                 bitte die Amplituden prüfen."
                    .into(),
            );
        }
        if !self.user_keywords.is_empty() {
            notes.push(
                "Eigene Keywords des Keyword-Editors wurden nicht umgerechnet; bitte prüfen."
                    .into(),
            );
        }
        notes
    }
}

impl StaticStep {
    fn convert_units(&mut self, c: &Conversion) {
        for time in [
            &mut self.initial_increment,
            &mut self.time_period,
            &mut self.min_increment,
            &mut self.max_increment,
        ] {
            c.value(time, Quantity::Time);
        }
    }
}

impl Material {
    pub fn convert_units(&mut self, c: &Conversion) {
        c.option(&mut self.density, Quantity::Density);
        if let Some(elastic) = &mut self.elastic {
            c.value(&mut elastic.young, Quantity::Pressure);
        }
        c.option(&mut self.conductivity, Quantity::ThermalConductivity);
        c.option(&mut self.specific_heat, Quantity::SpecificHeat);
        if let Some(expansion) = &mut self.expansion {
            c.value(&mut expansion.coefficient, Quantity::ThermalExpansion);
            c.value(&mut expansion.zero_temperature, Quantity::Temperature);
        }
    }
}

impl Constraint {
    fn convert_units(&mut self, c: &Conversion) {
        let stiffness = |per_area: bool| {
            if per_area {
                Quantity::ForcePerVolume
            } else {
                Quantity::ForcePerLength
            }
        };
        match self {
            Constraint::PointSpring(s) => c.all(&mut s.stiffness, Quantity::ForcePerLength),
            Constraint::SurfaceSpring(s) => c.all(&mut s.stiffness, stiffness(s.per_area)),
            Constraint::SurfaceToSurfaceSpring(s) => c.all(&mut s.stiffness, stiffness(s.per_area)),
            Constraint::CompressionOnly(s) => {
                c.value(&mut s.clearance, Quantity::Length);
                c.value(&mut s.offset, Quantity::Length);
                c.option(&mut s.spring_stiffness, Quantity::ForcePerLength);
                c.option(&mut s.tensile_force, Quantity::Force);
            }
            Constraint::Tie(t) => c.option(&mut t.position_tolerance, Quantity::Length),
            Constraint::NodeTie(_) => {}
        }
    }
}

impl InteractionProperty {
    pub fn convert_units(&mut self, c: &Conversion) {
        match self {
            InteractionProperty::SurfaceBehavior(behavior) => match behavior {
                SurfaceBehavior::Hard => {}
                SurfaceBehavior::Linear { k, sigma_inf, c0 } => {
                    c.value(k, Quantity::ForcePerVolume);
                    c.value(sigma_inf, Quantity::Pressure);
                    c.option(c0, Quantity::Length);
                }
                SurfaceBehavior::Exponential { c0, p0 } => {
                    c.value(c0, Quantity::Length);
                    c.value(p0, Quantity::Pressure);
                }
                SurfaceBehavior::Tabular(rows) => {
                    for [pressure, overclosure] in rows {
                        c.value(pressure, Quantity::Pressure);
                        c.value(overclosure, Quantity::Length);
                    }
                }
                SurfaceBehavior::Tied { k } => c.value(k, Quantity::ForcePerVolume),
            },
            InteractionProperty::Friction(friction) => {
                c.option(&mut friction.stick_slope, Quantity::ForcePerVolume)
            }
            InteractionProperty::GapConductance(conductance) => match conductance {
                GapConductance::Constant(value) => {
                    c.value(value, Quantity::HeatTransferCoefficient)
                }
                GapConductance::Tabular(rows) => {
                    for [value, pressure, temperature] in rows {
                        c.value(value, Quantity::HeatTransferCoefficient);
                        c.value(pressure, Quantity::Pressure);
                        c.value(temperature, Quantity::Temperature);
                    }
                }
            },
        }
    }
}

impl Geometry {
    /// Converts the mesh sizes; the shapes themselves are scaled by the mesher, which can
    /// read them.
    pub fn convert_sizes(&mut self, c: &Conversion) {
        self.meshing.convert_units(c);
        for item in &mut self.mesh_items {
            match &mut item.kind {
                MeshSetupKind::MeshingParameters { parameters, .. } => parameters.convert_units(c),
                MeshSetupKind::LocalMeshSize { size, .. } => c.value(size, Quantity::Length),
                MeshSetupKind::TetrahedralGmsh { .. } => {}
            }
        }
    }
}

impl MeshingParameters {
    fn convert_units(&mut self, c: &Conversion) {
        c.value(&mut self.max_size, Quantity::Length);
        c.value(&mut self.min_size, Quantity::Length);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Elastic, Load, Region, Section, SectionKind, Step};

    #[test]
    fn a_converted_model_stays_the_same_physically() {
        let mut model = FeModel::default();
        model.materials.push(Material {
            name: "Steel".into(),
            density: Some(7.85e-9),
            elastic: Some(Elastic {
                young: 210_000.0,
                poisson: 0.3,
            }),
            conductivity: Some(50.0),
            ..Material::default()
        });
        model.sections.push(Section {
            name: "Section-1".into(),
            material: "Steel".into(),
            region: Region::Parts(Vec::new()),
            thickness: 5.0,
            kind: SectionKind::Solid,
        });
        let mut step = Step::new_static("Step-1");
        step.loads.push(Load {
            name: "Pressure-1".into(),
            active: true,
            region: Region::Surface("A".into()),
            kind: LoadKind::Pressure(2.0),
            amplitude: None,
            factor_amplitude: None,
        });
        step.loads.push(Load {
            name: "Force-1".into(),
            active: true,
            region: Region::Nodes(Vec::new()),
            kind: LoadKind::ConcentratedForce([1000.0, 0.0, 0.0]),
            amplitude: None,
            factor_amplitude: None,
        });
        model.steps.push(step);
        assert!(model.convert_units(UnitSystem::MTonSC).is_empty());
        assert_eq!(model.properties.units, UnitSystem::MTonSC);
        let close = |a: f64, b: f64| (a - b).abs() <= 1e-12 * b.abs();
        let material = &model.materials[0];
        assert!(close(material.density.unwrap(), 7.85));
        assert!(close(material.elastic.unwrap().young, 2.1e8));
        // 50 W/(m·K) is 50 mW/(mm·K) and 0.05 kW/(m·K).
        assert!(close(material.conductivity.unwrap(), 0.05));
        assert!(close(model.sections[0].thickness, 0.005));
        let loads = &model.steps[0].loads;
        assert!(matches!(loads[0].kind, LoadKind::Pressure(p) if close(p, 2000.0)));
        assert!(matches!(loads[1].kind, LoadKind::ConcentratedForce([f, _, _]) if close(f, 1.0)));
        // Without units, the numbers are only reinterpreted.
        model.convert_units(UnitSystem::Unitless);
        assert!(close(model.sections[0].thickness, 0.005));
    }
}
