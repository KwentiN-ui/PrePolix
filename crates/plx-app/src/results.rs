use plx_render::contour::DEFAULT_LEVELS;
use plx_results::{AnalysisKind, Component, Field, Increment};

/// How the deformed shape is scaled, as in PrePoMax's results toolbar.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Deformation {
    Undeformed,
    TrueScale,
    /// Largest displacement drawn as a quarter of the model size, times this factor.
    Automatic(f32),
    UserDefined,
}

impl Deformation {
    pub const CHOICES: [Deformation; 8] = [
        Deformation::Undeformed,
        Deformation::TrueScale,
        Deformation::Automatic(0.25),
        Deformation::Automatic(0.5),
        Deformation::Automatic(1.0),
        Deformation::Automatic(2.0),
        Deformation::Automatic(5.0),
        Deformation::UserDefined,
    ];

    pub fn label(self) -> String {
        match self {
            Deformation::Undeformed => "Unverformt".into(),
            Deformation::TrueScale => "Echter Maßstab".into(),
            Deformation::Automatic(1.0) => "Automatisch".into(),
            Deformation::Automatic(f) => format!("Automatisch × {f}"),
            Deformation::UserDefined => "Benutzerdefiniert".into(),
        }
    }
}

/// Selection and display settings for the results of one `.frd` file.
pub struct ResultsView {
    pub increments: Vec<Increment>,
    pub increment: usize,
    pub field: usize,
    pub component: usize,
    pub deformation: Deformation,
    pub user_scale: f32,
    pub levels: u32,
    /// Draw the undeformed outline behind the deformed shape.
    pub show_undeformed: bool,
    /// When CalculiX ran the analysis, as written into the file.
    pub date: Option<String>,
    pub time: Option<String>,
    /// Characteristic model size for the automatic scale (PrePoMax: cube root of the bounding
    /// box volume, square root of the area for flat models).
    model_size: f64,
}

/// What the viewport legend shows.
pub struct Legend {
    pub title: String,
    pub min: f32,
    pub max: f32,
    pub levels: u32,
}

impl ResultsView {
    pub fn new(increments: Vec<Increment>, bounds: Option<([f64; 3], [f64; 3])>) -> Self {
        let mut view = Self {
            increments,
            increment: 0,
            field: 0,
            component: 0,
            deformation: Deformation::Automatic(1.0),
            user_scale: 10.0,
            levels: DEFAULT_LEVELS,
            show_undeformed: true,
            date: None,
            time: None,
            model_size: bounds.map_or(1.0, model_size),
        };
        view.increment = view.default_increment();
        view
    }

    /// PrePoMax opens the last increment of the last step, or the first mode of a frequency step.
    fn default_increment(&self) -> usize {
        let Some(last) = self.increments.last() else {
            return 0;
        };
        let step: Vec<usize> = (0..self.increments.len())
            .filter(|&i| self.increments[i].step == last.step)
            .collect();
        if last.kind == AnalysisKind::Frequency {
            step[0]
        } else {
            *step.last().unwrap()
        }
    }

    pub fn current_increment(&self) -> Option<&Increment> {
        self.increments.get(self.increment)
    }

    pub fn current(&self) -> Option<(&Field, &Component)> {
        let field = self.current_increment()?.fields.get(self.field)?;
        Some((field, field.components.get(self.component)?))
    }

    /// Switches to another increment, keeping field and component by name where possible.
    pub fn select_increment(&mut self, index: usize) {
        let names = self
            .current()
            .map(|(f, c)| (f.name.clone(), c.name.clone()));
        self.increment = index;
        self.field = 0;
        self.component = 0;
        let found = names.and_then(|(field, component)| {
            let fields = &self.current_increment()?.fields;
            let f = fields.iter().position(|f| f.name == field)?;
            let c = fields[f]
                .components
                .iter()
                .position(|c| c.name == component);
            Some((f, c.unwrap_or(0)))
        });
        if let Some((field, component)) = found {
            self.field = field;
            self.component = component;
        }
    }

    /// Entry of the increment list, "step, increment" as in PrePoMax's results toolbar.
    pub fn increment_label(increment: &Increment) -> String {
        format!("{}, {}", increment.step, increment.increment)
    }

    /// PrePoMax's information block in the top right corner of the 3D view.
    pub fn status_lines(&self, file_name: &str) -> Vec<String> {
        let mut lines = vec![format!(
            "Name: {file_name}   Date: {}   Time: {}",
            self.date.as_deref().unwrap_or("-"),
            self.time.as_deref().unwrap_or("-")
        )];
        if let Some(inc) = self.current_increment() {
            let value = format_value(inc.value as f32);
            lines.push(match inc.kind {
                AnalysisKind::Frequency => format!(
                    "Step: #{}   Mode: #{}   Frequency: {value}",
                    inc.step, inc.increment
                ),
                AnalysisKind::Buckling => {
                    format!("Step: #{}   Buckling factor: {value}", inc.step)
                }
                _ => format!(
                    "Step: #{}   Increment: #{}   Analysis time: {value}",
                    inc.step, inc.increment
                ),
            });
        }
        lines.push(format!(
            "Deformation variable: Displacements   Deformation scale factor: {}",
            format_value(self.scale())
        ));
        lines
    }

    /// Node index and value of the largest value of the shown component.
    pub fn maximum(&self) -> Option<(usize, f32)> {
        let (_, component) = self.current()?;
        component
            .values
            .iter()
            .copied()
            .enumerate()
            .filter(|(_, v)| v.is_finite())
            .reduce(|a, b| if b.1 > a.1 { b } else { a })
    }

    /// Displacement scale factor for the current increment and deformation setting.
    pub fn scale(&self) -> f32 {
        match self.deformation {
            Deformation::Undeformed => 0.0,
            Deformation::TrueScale => 1.0,
            Deformation::UserDefined => self.user_scale,
            Deformation::Automatic(factor) => {
                let Some(max) = self.current_increment().and_then(max_deformation) else {
                    return 0.0;
                };
                if max <= 0.0 {
                    return 1.0;
                }
                round_significant(factor as f64 * 0.25 * self.model_size / max as f64, 2) as f32
            }
        }
    }

    pub fn legend(&self) -> Option<Legend> {
        let (field, component) = self.current()?;
        let (min, max) = component.range()?;
        // PrePoMax writes names with blanks instead of underscores and dashes.
        let name = |n: &str| n.replace(['_', '-'], " ");
        Some(Legend {
            title: format!(
                "{}: {}\nAutomatic",
                name(&field.name),
                name(&component.name)
            ),
            min,
            max,
            levels: self.levels,
        })
    }
}

/// PrePoMax's measure of the largest displacement: the root of the summed squares of the
/// largest absolute value of every `DISP` component, including `ALL`.
fn max_deformation(increment: &Increment) -> Option<f32> {
    let field = increment.field("DISP")?;
    let sum: f32 = field
        .components
        .iter()
        .filter_map(Component::range)
        .map(|(min, max)| min.abs().max(max.abs()).powi(2))
        .sum();
    Some(sum.sqrt())
}

fn model_size((min, max): ([f64; 3], [f64; 3])) -> f64 {
    let d = [0, 1, 2].map(|k| max[k] - min[k]);
    let diagonal = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
    if d[2].abs() < 1e-6 * diagonal {
        (d[0] * d[1]).abs().sqrt()
    } else {
        (d[0] * d[1] * d[2]).abs().cbrt()
    }
}

fn round_significant(value: f64, digits: i32) -> f64 {
    if value == 0.0 || !value.is_finite() {
        return value;
    }
    let magnitude = 10f64.powi(digits - 1 - value.abs().log10().floor() as i32);
    (value * magnitude).round() / magnitude
}

/// Number with four significant digits, like PrePoMax's default "G4" format.
pub fn format_value(value: f32) -> String {
    let abs = value.abs();
    if value == 0.0 {
        "0".into()
    } else if (1e-3..1e4).contains(&abs) {
        let decimals = (3 - abs.log10().floor() as i32).max(0) as usize;
        let text = format!("{value:.decimals$}");
        if text.contains('.') {
            text.trim_end_matches('0').trim_end_matches('.').to_string()
        } else {
            text
        }
    } else {
        format!("{value:.3E}")
    }
}

/// Legend label: like [`format_value`] with a sign on non-negative values.
pub fn format_legend_value(value: f32) -> String {
    let text = format_value(value);
    if value >= 0.0 {
        format!("+{text}")
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn increment(step: u32, number: u32, kind: AnalysisKind, disp: &[[f32; 3]]) -> Increment {
        let column = |k: usize, name: &str| Component {
            name: name.into(),
            values: disp.iter().map(|d| d[k]).collect(),
            derived: false,
        };
        let mut field = Field {
            name: "DISP".into(),
            components: vec![column(0, "U1"), column(1, "U2"), column(2, "U3")],
        };
        plx_results::add_derived_components(&mut field);
        Increment {
            step,
            increment: number,
            kind,
            value: number as f64,
            fields: vec![field],
        }
    }

    #[test]
    fn automatic_scale_follows_prepomax() {
        // Cube of size 10, largest displacement 0.1 in z: ALL and U3 both reach 0.1.
        let view = ResultsView::new(
            vec![increment(
                1,
                1,
                AnalysisKind::Static,
                &[[0.0, 0.0, 0.0], [0.0, 0.0, -0.1]],
            )],
            Some(([0.0; 3], [10.0; 3])),
        );
        let max = (2.0f32 * 0.01).sqrt();
        let expected = round_significant(0.25 * 10.0 / max as f64, 2) as f32;
        assert_eq!(view.scale(), expected);
        assert_eq!(view.scale(), 18.0);
    }

    #[test]
    fn opens_last_increment_but_first_mode() {
        let still = [[0.0; 3]];
        let static_view = ResultsView::new(
            vec![
                increment(1, 1, AnalysisKind::Static, &still),
                increment(1, 2, AnalysisKind::Static, &still),
            ],
            None,
        );
        assert_eq!(static_view.increment, 1);
        let modes = ResultsView::new(
            vec![
                increment(1, 1, AnalysisKind::Static, &still),
                increment(2, 1, AnalysisKind::Frequency, &still),
                increment(2, 2, AnalysisKind::Frequency, &still),
            ],
            None,
        );
        assert_eq!(modes.increment, 1);
        assert_eq!(modes.current().unwrap().1.name, "ALL");
    }

    #[test]
    fn values_are_formatted_with_four_significant_digits() {
        assert_eq!(format_value(123.456), "123.5");
        assert_eq!(format_value(0.012345), "0.01235");
        assert_eq!(format_value(-2.0), "-2");
        assert_eq!(format_value(123456.0), "1.235E5");
        assert_eq!(format_legend_value(1.5), "+1.5");
    }
}
