//! Contact and tie definitions: PrePoMax's surface interactions, contact pairs and tie
//! constraints. Master and slave are regions the user picked; their surfaces are written to
//! the input file only when it is exported.

use serde::{Deserialize, Serialize};

use crate::{CompressionOnly, NodeTie, PointSpring, Region, SurfaceSpring, SurfaceToSurfaceSpring};

/// PrePoMax's default colour of contact and constraint surfaces, yellow.
pub const DEFAULT_SURFACE_COLOR: [u8; 3] = [255, 255, 0];

/// Mechanical and thermal behaviour of touching surfaces (`*SURFACE INTERACTION`), referred to
/// by contact pairs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SurfaceInteraction {
    pub name: String,
    /// The interaction models in the order the user added them; each kind at most once.
    pub properties: Vec<InteractionProperty>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum InteractionProperty {
    SurfaceBehavior(SurfaceBehavior),
    Friction(Friction),
    GapConductance(GapConductance),
}

impl InteractionProperty {
    /// PrePoMax's names of the interaction models, which are also the default models.
    pub fn all() -> [InteractionProperty; 3] {
        [
            InteractionProperty::SurfaceBehavior(SurfaceBehavior::default()),
            InteractionProperty::Friction(Friction::default()),
            InteractionProperty::GapConductance(GapConductance::default()),
        ]
    }

    pub fn name(&self) -> &'static str {
        match self {
            InteractionProperty::SurfaceBehavior(_) => "Surface Behavior",
            InteractionProperty::Friction(_) => "Friction",
            InteractionProperty::GapConductance(_) => "Gap Conductance",
        }
    }
}

/// Pressure-overclosure relation of the contact (`*SURFACE BEHAVIOR`), with PrePoMax's default
/// values for mm, N and MPa.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum SurfaceBehavior {
    /// No penetration at all.
    #[default]
    Hard,
    /// Linear spring: slope `k`, tension `sigma_inf` at large clearances and the clearance
    /// `c0` up to which the spring acts (`None` lets CalculiX choose it).
    Linear {
        k: f64,
        sigma_inf: f64,
        c0: Option<f64>,
    },
    /// Pressure falling off exponentially with the clearance: `c0` where it is 1 % of the
    /// pressure `p0` at zero clearance.
    Exponential { c0: f64, p0: f64 },
    /// Pressure over overclosure as a table of (pressure, overclosure).
    Tabular(Vec<[f64; 2]>),
    /// Tied contact with the stiffness `k`.
    Tied { k: f64 },
}

impl SurfaceBehavior {
    /// Kinds of the relation with PrePoMax's default values, as offered in the dialog.
    pub fn kinds() -> [SurfaceBehavior; 5] {
        // E = 200 GPa: K = 50 E, sigma_inf = E / 70000.
        let k = 1e7;
        [
            SurfaceBehavior::Hard,
            SurfaceBehavior::Linear {
                k,
                sigma_inf: 2.86,
                c0: None,
            },
            SurfaceBehavior::Exponential { c0: 1.0, p0: 0.1 },
            SurfaceBehavior::Tabular(vec![[0.0, 0.0], [1e5, 1.0]]),
            SurfaceBehavior::Tied { k },
        ]
    }

    /// The value of `PRESSURE-OVERCLOSURE=`.
    pub fn keyword(&self) -> &'static str {
        match self {
            SurfaceBehavior::Hard => "Hard",
            SurfaceBehavior::Linear { .. } => "Linear",
            SurfaceBehavior::Exponential { .. } => "Exponential",
            SurfaceBehavior::Tabular(_) => "Tabular",
            SurfaceBehavior::Tied { .. } => "Tied",
        }
    }
}

/// Coulomb friction (`*FRICTION`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Friction {
    pub coefficient: f64,
    /// Shear stress over tangential displacement while sticking; `None` lets CalculiX choose.
    pub stick_slope: Option<f64>,
}

impl Default for Friction {
    fn default() -> Self {
        Self {
            coefficient: 0.1,
            stick_slope: None,
        }
    }
}

/// Heat conductance across the contact (`*GAP CONDUCTANCE`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum GapConductance {
    Constant(f64),
    /// Rows of (conductance, contact pressure, temperature).
    Tabular(Vec<[f64; 3]>),
}

impl Default for GapConductance {
    fn default() -> Self {
        GapConductance::Constant(0.0)
    }
}

/// How CalculiX treats a contact pair (`TYPE=`). All pairs of a model must use the same.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContactMethod {
    NodeToSurface,
    #[default]
    SurfaceToSurface,
    Mortar,
    Massless,
}

impl ContactMethod {
    pub const ALL: [ContactMethod; 4] = [
        ContactMethod::NodeToSurface,
        ContactMethod::SurfaceToSurface,
        ContactMethod::Mortar,
        ContactMethod::Massless,
    ];

    /// PrePoMax's name, which is also the value of `TYPE=`.
    pub fn name(self) -> &'static str {
        match self {
            ContactMethod::NodeToSurface => "Node to surface",
            ContactMethod::SurfaceToSurface => "Surface to surface",
            ContactMethod::Mortar => "Mortar",
            ContactMethod::Massless => "Massless",
        }
    }
}

/// Two surfaces that may touch (`*CONTACT PAIR`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ContactPair {
    pub name: String,
    /// A deactivated contact pair is written to the input file as a comment only.
    #[serde(default = "crate::active")]
    pub active: bool,
    /// Name of the surface interaction.
    pub interaction: String,
    pub method: ContactMethod,
    /// Pairing only at the start of each increment; node to surface contact only.
    pub small_sliding: bool,
    /// Moves slave nodes onto the master surface at the start (`ADJUST=`).
    pub adjust: bool,
    /// Distance within which slave nodes are moved; `None` moves only penetrating nodes.
    pub adjustment_size: Option<f64>,
    pub master: Region,
    pub slave: Region,
    pub master_color: [u8; 3],
    pub slave_color: [u8; 3],
}

/// PrePoMax's name of a swapped pair: `<slave>_to_<master>` for `<master>_to_<slave>`, other
/// names as they are.
pub fn swapped_name(name: &str) -> String {
    match name.split("_to_").collect::<Vec<_>>()[..] {
        [master, slave] => format!("{slave}_to_{master}"),
        _ => name.to_owned(),
    }
}

impl ContactPair {
    /// Swaps master and slave, like PrePoMax also in a name `<master>_to_<slave>`.
    pub fn swap_master_slave(&mut self) {
        std::mem::swap(&mut self.master, &mut self.slave);
        self.name = swapped_name(&self.name);
    }

    pub fn new(name: impl Into<String>, interaction: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            active: true,
            interaction: interaction.into(),
            method: ContactMethod::default(),
            small_sliding: false,
            adjust: false,
            adjustment_size: None,
            master: Region::Faces(Vec::new()),
            slave: Region::Faces(Vec::new()),
            master_color: DEFAULT_SURFACE_COLOR,
            slave_color: DEFAULT_SURFACE_COLOR,
        }
    }
}

/// A constraint between parts of the model or to ground, PrePoMax's Constraints.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Constraint {
    PointSpring(PointSpring),
    SurfaceSpring(SurfaceSpring),
    CompressionOnly(CompressionOnly),
    Tie(Tie),
    SurfaceToSurfaceSpring(SurfaceToSurfaceSpring),
    /// Only in projects saved while node ties were constraints; [`crate::FeModel::migrate`]
    /// moves them to the model's node ties, where they belong with the contact pairs.
    NodeTie(NodeTie),
}

impl Constraint {
    pub fn name(&self) -> &str {
        match self {
            Constraint::PointSpring(c) => &c.name,
            Constraint::SurfaceSpring(c) => &c.name,
            Constraint::CompressionOnly(c) => &c.name,
            Constraint::Tie(tie) => &tie.name,
            Constraint::SurfaceToSurfaceSpring(c) => &c.name,
            Constraint::NodeTie(c) => &c.name,
        }
    }

    pub fn active(&self) -> bool {
        match self {
            Constraint::PointSpring(c) => c.active,
            Constraint::SurfaceSpring(c) => c.active,
            Constraint::CompressionOnly(c) => c.active,
            Constraint::Tie(tie) => tie.active,
            Constraint::SurfaceToSurfaceSpring(c) => c.active,
            Constraint::NodeTie(c) => c.active,
        }
    }

    pub fn active_mut(&mut self) -> &mut bool {
        match self {
            Constraint::PointSpring(c) => &mut c.active,
            Constraint::SurfaceSpring(c) => &mut c.active,
            Constraint::CompressionOnly(c) => &mut c.active,
            Constraint::Tie(tie) => &mut tie.active,
            Constraint::SurfaceToSurfaceSpring(c) => &mut c.active,
            Constraint::NodeTie(c) => &mut c.active,
        }
    }

    pub fn name_mut(&mut self) -> &mut String {
        match self {
            Constraint::PointSpring(c) => &mut c.name,
            Constraint::SurfaceSpring(c) => &mut c.name,
            Constraint::CompressionOnly(c) => &mut c.name,
            Constraint::Tie(tie) => &mut tie.name,
            Constraint::SurfaceToSurfaceSpring(c) => &mut c.name,
            Constraint::NodeTie(c) => &mut c.name,
        }
    }

    /// Swaps master and slave of a tie or spring connection, like PrePoMax also in a name
    /// `<master>_to_<slave>`; returns false for constraints without the two.
    pub fn swap_master_slave(&mut self) -> bool {
        let (name, master, slave) = match self {
            Constraint::Tie(tie) => (&mut tie.name, &mut tie.master, &mut tie.slave),
            Constraint::SurfaceToSurfaceSpring(c) => (&mut c.name, &mut c.master, &mut c.slave),
            _ => return false,
        };
        std::mem::swap(master, slave);
        *name = swapped_name(name);
        true
    }

    /// Master and slave region of a tie or spring connection.
    pub fn master_slave(&self) -> Option<[&Region; 2]> {
        match self {
            Constraint::Tie(tie) => Some([&tie.master, &tie.slave]),
            Constraint::SurfaceToSurfaceSpring(c) => Some([&c.master, &c.slave]),
            _ => None,
        }
    }

    /// The regions the constraint is defined on, master before slave.
    pub fn regions(&self) -> Vec<&Region> {
        match self {
            Constraint::PointSpring(c) => vec![&c.region],
            Constraint::SurfaceSpring(c) => vec![&c.region],
            Constraint::CompressionOnly(c) => vec![&c.region],
            Constraint::Tie(tie) => vec![&tie.master, &tie.slave],
            Constraint::SurfaceToSurfaceSpring(c) => vec![&c.master, &c.slave],
            Constraint::NodeTie(c) => vec![&c.region],
        }
    }

    pub fn regions_mut(&mut self) -> Vec<&mut Region> {
        match self {
            Constraint::PointSpring(c) => vec![&mut c.region],
            Constraint::SurfaceSpring(c) => vec![&mut c.region],
            Constraint::CompressionOnly(c) => vec![&mut c.region],
            Constraint::Tie(tie) => vec![&mut tie.master, &mut tie.slave],
            Constraint::SurfaceToSurfaceSpring(c) => vec![&mut c.master, &mut c.slave],
            Constraint::NodeTie(c) => vec![&mut c.region],
        }
    }
}

/// Slave surface glued to a master surface (`*TIE`), e.g. parts meshed separately.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Tie {
    pub name: String,
    #[serde(default = "crate::active")]
    pub active: bool,
    /// Distance within which slave nodes are tied; `None` lets CalculiX choose.
    pub position_tolerance: Option<f64>,
    /// Moves the tied slave nodes onto the master surface.
    pub adjust: bool,
    pub master: Region,
    pub slave: Region,
    pub master_color: [u8; 3],
    pub slave_color: [u8; 3],
}

impl Tie {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            active: true,
            position_tolerance: None,
            adjust: true,
            master: Region::Faces(Vec::new()),
            slave: Region::Faces(Vec::new()),
            master_color: DEFAULT_SURFACE_COLOR,
            slave_color: DEFAULT_SURFACE_COLOR,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swapping_turns_regions_and_a_pair_name_around() {
        let mut pair = ContactPair::new("BEAM_to_DISC", "Steel");
        pair.master = Region::Surface("TOP".into());
        pair.slave = Region::Surface("BOTTOM".into());
        pair.swap_master_slave();
        assert_eq!(pair.name, "DISC_to_BEAM");
        assert_eq!(pair.master, Region::Surface("BOTTOM".into()));
        assert_eq!(pair.slave, Region::Surface("TOP".into()));
        let mut tie = Constraint::Tie(Tie::new("Lager"));
        assert!(tie.swap_master_slave());
        assert_eq!(tie.name(), "Lager");
        assert_eq!(swapped_name("A_to_B_to_C"), "A_to_B_to_C");
    }
}
