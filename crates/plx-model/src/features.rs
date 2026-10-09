//! PrePoMax's features: reference points and coordinate systems. They describe places and
//! directions in the model that other items refer to by name, e.g. a section plane spanned by
//! two axes of a coordinate system or a result path between two reference points.
//!
//! A coordinate system is defined like PrePoMax's and CalculiX's `*ORIENTATION`: by its
//! origin, a point on its x axis and a point in its xy plane. Cylindrical systems use the same
//! points; their z axis is the cylinder axis and r, θ, z are the local directions.

use serde::{Deserialize, Serialize};

use crate::FeModel;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReferencePoint {
    pub name: String,
    pub position: [f64; 3],
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum CoordinateSystemKind {
    #[default]
    Rectangular,
    Cylindrical,
}

impl CoordinateSystemKind {
    pub const ALL: [CoordinateSystemKind; 2] = [Self::Rectangular, Self::Cylindrical];

    pub fn label(self) -> &'static str {
        match self {
            Self::Rectangular => "Kartesisch",
            Self::Cylindrical => "Zylindrisch",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CoordinateSystem {
    pub name: String,
    pub kind: CoordinateSystemKind,
    pub origin: [f64; 3],
    /// A point on the positive x axis.
    pub point_x: [f64; 3],
    /// A point in the xy plane on the side of the positive y axis.
    pub point_xy: [f64; 3],
}

/// The three planes of a coordinate system.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CoordinatePlane {
    Xy,
    Yz,
    Xz,
}

impl CoordinatePlane {
    pub const ALL: [CoordinatePlane; 3] = [Self::Xy, Self::Yz, Self::Xz];

    pub fn label(self) -> &'static str {
        match self {
            Self::Xy => "XY",
            Self::Yz => "YZ",
            Self::Xz => "XZ",
        }
    }

    /// Index of the axis normal to the plane.
    pub fn normal_axis(self) -> usize {
        match self {
            Self::Yz => 0,
            Self::Xz => 1,
            Self::Xy => 2,
        }
    }
}

impl CoordinateSystem {
    /// PrePoMax's default: the global system.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            kind: CoordinateSystemKind::Rectangular,
            origin: [0.0; 3],
            point_x: [1.0, 0.0, 0.0],
            point_xy: [0.0, 1.0, 0.0],
        }
    }

    /// The unit axes x, y and z; fails when the points do not span a plane.
    pub fn axes(&self) -> Result<[[f64; 3]; 3], String> {
        let x = sub(self.point_x, self.origin);
        let xy = sub(self.point_xy, self.origin);
        let scale = norm(x).max(norm(xy));
        let Some(x) = normalize(x) else {
            return Err(format!(
                "{}: Der Punkt auf der x-Achse liegt im Ursprung.",
                self.name
            ));
        };
        let z = cross(x, xy);
        if norm(z) <= 1e-9 * scale * scale {
            return Err(format!(
                "{}: Der Punkt in der xy-Ebene liegt auf der x-Achse.",
                self.name
            ));
        }
        let z = normalize(z).unwrap_or([0.0, 0.0, 1.0]);
        Ok([x, cross(z, x), z])
    }

    /// The local unit directions at a point: x, y, z of a rectangular system, or r, θ, z of a
    /// cylindrical one. On the cylinder axis, r is the system's x axis.
    pub fn directions_at(&self, point: [f64; 3]) -> Result<[[f64; 3]; 3], String> {
        let [x, y, z] = self.axes()?;
        if self.kind == CoordinateSystemKind::Rectangular {
            return Ok([x, y, z]);
        }
        let d = sub(point, self.origin);
        let radial = sub(d, scale(z, dot(d, z)));
        let r = normalize(radial)
            .filter(|_| norm(radial) > 1e-12 * norm(d).max(1e-300))
            .unwrap_or(x);
        Ok([r, cross(z, r), z])
    }

    /// A plane of the system as a point on it and its unit normal, shifted by `offset` along
    /// the normal.
    pub fn plane(
        &self,
        plane: CoordinatePlane,
        offset: f64,
    ) -> Result<([f64; 3], [f64; 3]), String> {
        let axes = self.axes()?;
        let normal = axes[plane.normal_axis()];
        Ok((add(self.origin, scale(normal, offset)), normal))
    }

    /// Coordinates of a global point in the system: x, y, z, or r, θ (radians), z.
    pub fn local(&self, point: [f64; 3]) -> Result<[f64; 3], String> {
        let [x, y, z] = self.axes()?;
        let d = sub(point, self.origin);
        let (a, b, c) = (dot(d, x), dot(d, y), dot(d, z));
        Ok(match self.kind {
            CoordinateSystemKind::Rectangular => [a, b, c],
            CoordinateSystemKind::Cylindrical => [a.hypot(b), b.atan2(a), c],
        })
    }
}

/// A point given by coordinates or by a reference point's name.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum PointRef {
    Coordinates([f64; 3]),
    ReferencePoint(String),
}

impl PointRef {
    pub fn resolve(&self, model: &FeModel) -> Result<[f64; 3], String> {
        match self {
            PointRef::Coordinates(p) => Ok(*p),
            PointRef::ReferencePoint(name) => model
                .reference_point(name)
                .map(|r| r.position)
                .ok_or_else(|| format!("Reference Point {name} existiert nicht")),
        }
    }
}

/// A straight path through the model on which results are read, e.g. across a wall: values
/// at `points` evenly spaced points from start to end, interpolated inside the elements.
/// Nothing of it goes into the input file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResultPath {
    pub name: String,
    pub start: PointRef,
    pub end: PointRef,
    pub points: u32,
}

impl ResultPath {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            start: PointRef::Coordinates([0.0; 3]),
            end: PointRef::Coordinates([1.0, 0.0, 0.0]),
            points: 100,
        }
    }

    /// Start and end in model coordinates.
    pub fn ends(&self, model: &FeModel) -> Result<([f64; 3], [f64; 3]), String> {
        Ok((self.start.resolve(model)?, self.end.resolve(model)?))
    }

    /// Positions of the read-out points with their distance from the start.
    pub fn samples(&self, model: &FeModel) -> Result<Vec<(f64, [f64; 3])>, String> {
        let (start, end) = self.ends(model)?;
        let length = norm(sub(end, start));
        if length == 0.0 {
            return Err(format!("{}: Anfang und Ende fallen zusammen.", self.name));
        }
        let n = self.points.max(2);
        Ok((0..n)
            .map(|i| {
                let t = i as f64 / (n - 1) as f64;
                (t * length, add(start, scale(sub(end, start), t)))
            })
            .collect())
    }
}

impl FeModel {
    pub fn reference_point(&self, name: &str) -> Option<&ReferencePoint> {
        self.reference_points.iter().find(|r| r.name == name)
    }

    pub fn coordinate_system(&self, name: &str) -> Option<&CoordinateSystem> {
        self.coordinate_systems.iter().find(|c| c.name == name)
    }

    /// Copies the features of another model that this one lacks, e.g. those of the FE model
    /// into its results.
    pub fn add_missing_features(&mut self, other: &FeModel) {
        for point in &other.reference_points {
            if self.reference_point(&point.name).is_none() {
                self.reference_points.push(point.clone());
            }
        }
        for system in &other.coordinate_systems {
            if self.coordinate_system(&system.name).is_none() {
                self.coordinate_systems.push(system.clone());
            }
        }
    }

    /// Points referring to a renamed reference point follow it.
    pub fn rename_reference_point(&mut self, old: &str, new: &str) {
        for path in &mut self.result_paths {
            for end in [&mut path.start, &mut path.end] {
                if matches!(end, PointRef::ReferencePoint(n) if n == old) {
                    *end = PointRef::ReferencePoint(new.to_string());
                }
            }
        }
    }
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn add(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn scale(a: [f64; 3], s: f64) -> [f64; 3] {
    a.map(|v| v * s)
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn norm(a: [f64; 3]) -> f64 {
    dot(a, a).sqrt()
}

fn normalize(a: [f64; 3]) -> Option<[f64; 3]> {
    let n = norm(a);
    (n > 0.0 && n.is_finite()).then(|| scale(a, 1.0 / n))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: [f64; 3], b: [f64; 3]) -> bool {
        (0..3).all(|k| (a[k] - b[k]).abs() < 1e-12)
    }

    #[test]
    fn axes_follow_the_three_points() {
        let mut cs = CoordinateSystem::new("CS");
        cs.origin = [1.0, 1.0, 0.0];
        cs.point_x = [1.0, 3.0, 0.0];
        cs.point_xy = [-2.0, 4.0, 0.0];
        let [x, y, z] = cs.axes().unwrap();
        assert!(close(x, [0.0, 1.0, 0.0]));
        // The xy point need not be on the y axis, only on its side.
        assert!(close(y, [-1.0, 0.0, 0.0]));
        assert!(close(z, [0.0, 0.0, 1.0]));
        let (point, normal) = cs.plane(CoordinatePlane::Yz, 2.0).unwrap();
        assert!(close(point, [1.0, 3.0, 0.0]) && close(normal, x));
    }

    #[test]
    fn degenerate_systems_are_reported() {
        let mut cs = CoordinateSystem::new("CS");
        cs.point_xy = [2.0, 0.0, 0.0];
        assert!(cs.axes().is_err());
        cs.point_x = cs.origin;
        assert!(cs.axes().is_err());
    }

    #[test]
    fn cylindrical_directions_turn_with_the_point() {
        let mut cs = CoordinateSystem::new("CS");
        cs.kind = CoordinateSystemKind::Cylindrical;
        let [r, t, z] = cs.directions_at([0.0, 2.0, 7.0]).unwrap();
        assert!(close(r, [0.0, 1.0, 0.0]) && close(t, [-1.0, 0.0, 0.0]));
        assert!(close(z, [0.0, 0.0, 1.0]));
        let local = cs.local([0.0, 2.0, 7.0]).unwrap();
        assert!(close(local, [2.0, std::f64::consts::FRAC_PI_2, 7.0]));
        // On the axis r falls back to x.
        assert!(close(
            cs.directions_at([0.0, 0.0, 3.0]).unwrap()[0],
            [1.0, 0.0, 0.0]
        ));
    }

    #[test]
    fn paths_resolve_reference_points_and_follow_renames() {
        let mut model = FeModel::default();
        model.reference_points.push(ReferencePoint {
            name: "RP-1".into(),
            position: [0.0, 0.0, 4.0],
        });
        let mut path = ResultPath::new("Path-1");
        path.start = PointRef::ReferencePoint("RP-1".into());
        path.end = PointRef::Coordinates([0.0, 0.0, 0.0]);
        path.points = 5;
        let samples = path.samples(&model).unwrap();
        assert_eq!(samples.len(), 5);
        assert_eq!(samples[1], (1.0, [0.0, 0.0, 3.0]));
        model.result_paths.push(path);
        model.rename_reference_point("RP-1", "Top");
        assert_eq!(
            model.result_paths[0].start,
            PointRef::ReferencePoint("Top".into())
        );
        assert!(model.result_paths[0].samples(&model).is_err());
    }
}
