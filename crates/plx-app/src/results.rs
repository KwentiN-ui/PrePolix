use plx_render::contour::DEFAULT_LEVELS;

use crate::animation::{Animation, AnimationKind, ColorLimits};
use crate::sound::ModeSound;
use plx_mesh::FeMesh;
use plx_results::field_output::{self, FieldOutput};
use plx_results::history_output::{self, HistoryOutput, HistorySet};
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
    /// Running animation, if the animation window is open.
    pub animation: Option<Animation>,
    /// Settings of the sound window, if it is open.
    pub sound: Option<ModeSound>,
    /// Field outputs the user derived from the results, in the order they are computed.
    pub field_outputs: Vec<FieldOutput>,
    /// History outputs the user derived, in the order they are computed.
    pub history_outputs: Vec<HistoryOutput>,
    /// Data of the history outputs that could be computed, by name.
    pub history: Vec<HistorySet>,
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
            animation: None,
            sound: None,
            field_outputs: Vec::new(),
            history_outputs: Vec::new(),
            history: Vec::new(),
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

    /// Index of the derived field output that computes the given field.
    pub fn field_output_index(&self, field: usize) -> Option<usize> {
        let name = &self.current_increment()?.fields.get(field)?.name;
        self.field_outputs.iter().position(|o| o.name == *name)
    }

    /// Creates (`index` is `None`) or replaces a derived field output and computes it. The
    /// outputs after it are computed again, as they may use it; their failures are returned.
    pub fn set_field_output(
        &mut self,
        index: Option<usize>,
        output: FieldOutput,
        mesh: &FeMesh,
    ) -> Result<Vec<String>, String> {
        let shown = self.shown_names();
        field_output::compute(&output, &mut self.increments, mesh)?;
        let at = match index.filter(|&i| i < self.field_outputs.len()) {
            Some(i) => {
                let old = std::mem::replace(&mut self.field_outputs[i], output);
                if old.name != self.field_outputs[i].name {
                    field_output::remove(&old.name, &mut self.increments);
                }
                i
            }
            None => {
                self.field_outputs.push(output);
                self.field_outputs.len() - 1
            }
        };
        let mut warnings = self.recompute_from(at + 1, mesh);
        warnings.extend(self.recompute_history(mesh));
        self.restore_selection(shown);
        Ok(warnings)
    }

    /// Deletes a derived field output and its field.
    pub fn remove_field_output(&mut self, index: usize, mesh: &FeMesh) -> Vec<String> {
        if index >= self.field_outputs.len() {
            return Vec::new();
        }
        let shown = self.shown_names();
        let output = self.field_outputs.remove(index);
        field_output::remove(&output.name, &mut self.increments);
        let mut warnings = self.recompute_from(index, mesh);
        warnings.extend(self.recompute_history(mesh));
        self.restore_selection(shown);
        warnings
    }

    /// Computes the outputs from `start` on again; one that fails loses its field.
    fn recompute_from(&mut self, start: usize, mesh: &FeMesh) -> Vec<String> {
        let mut warnings = Vec::new();
        for output in self.field_outputs.iter().skip(start) {
            if let Err(error) = field_output::compute(output, &mut self.increments, mesh) {
                field_output::remove(&output.name, &mut self.increments);
                warnings.push(format!("{}: {error}", output.name));
            }
        }
        warnings
    }

    fn shown_names(&self) -> Option<(String, String)> {
        self.current()
            .map(|(f, c)| (f.name.clone(), c.name.clone()))
    }

    /// Shows the field and component by name again after fields were added or removed.
    fn restore_selection(&mut self, names: Option<(String, String)>) {
        let found = names.and_then(|(field, component)| {
            let fields = &self.current_increment()?.fields;
            let f = fields.iter().position(|f| f.name == field)?;
            let c = fields[f]
                .components
                .iter()
                .position(|c| c.name == component);
            Some((f, c.unwrap_or(0)))
        });
        (self.field, self.component) = found.unwrap_or((0, 0));
    }

    /// Creates (`index` is `None`) or replaces a history output and computes it; the history
    /// outputs after it are computed again. Their failures are returned.
    pub fn set_history_output(
        &mut self,
        index: Option<usize>,
        output: HistoryOutput,
        mesh: &FeMesh,
    ) -> Result<Vec<String>, String> {
        let at = index.filter(|&i| i < self.history_outputs.len());
        // An equation sees the outputs before it.
        let before = at.unwrap_or(self.history_outputs.len());
        let earlier: Vec<HistorySet> = (self.history.iter())
            .filter(|set| (self.history_outputs[..before].iter()).any(|o| o.name == set.name))
            .cloned()
            .collect();
        history_output::compute(&output, &self.increments, mesh, &earlier)?;
        match at {
            Some(i) => self.history_outputs[i] = output,
            None => self.history_outputs.push(output),
        }
        Ok(self.recompute_history(mesh))
    }

    pub fn remove_history_output(&mut self, index: usize, mesh: &FeMesh) -> Vec<String> {
        if index >= self.history_outputs.len() {
            return Vec::new();
        }
        self.history_outputs.remove(index);
        self.recompute_history(mesh)
    }

    /// Computes all history outputs again, in order; failures leave an output without data.
    pub fn recompute_history(&mut self, mesh: &FeMesh) -> Vec<String> {
        let mut warnings = Vec::new();
        let mut sets = Vec::new();
        for output in &self.history_outputs {
            match history_output::compute(output, &self.increments, mesh, &sets) {
                Ok(set) => sets.push(set),
                Err(error) => warnings.push(format!("{}: {error}", output.name)),
            }
        }
        self.history = sets;
        warnings
    }

    /// Index of the history output of a computed set.
    pub fn history_output_index(&self, set: usize) -> Option<usize> {
        let name = &self.history.get(set)?.name;
        self.history_outputs.iter().position(|o| o.name == *name)
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

    /// Node index and value of the largest value on screen, animation frame included.
    pub fn maximum(&self) -> Option<(usize, f32)> {
        self.extreme(|a, b| b > a)
    }

    /// Displacement scale factor for the current increment and deformation setting.
    pub fn scale(&self) -> f32 {
        match self.deformation {
            Deformation::Undeformed => 0.0,
            Deformation::TrueScale => 1.0,
            Deformation::UserDefined => self.user_scale,
            Deformation::Automatic(factor) => {
                // An increment animation keeps one scale for all frames: that of the largest
                // displacement among them.
                let animated = self
                    .animation
                    .as_ref()
                    .filter(|a| a.kind == AnimationKind::Increments)
                    .map(|a| a.increments.as_slice());
                let max = match animated.filter(|i| !i.is_empty()) {
                    Some(indices) => indices
                        .iter()
                        .filter_map(|&i| self.increments.get(i).and_then(max_deformation))
                        .reduce(f32::max),
                    None => self.current_increment().and_then(max_deformation),
                };
                let Some(max) = max else {
                    return 0.0;
                };
                if max <= 0.0 {
                    return 1.0;
                }
                round_significant(factor as f64 * 0.25 * self.model_size / max as f64, 2) as f32
            }
        }
    }

    /// Factor on the deformation of the shown animation frame; 1 without animation.
    pub fn amplitude(&self) -> f32 {
        self.animation.as_ref().map_or(1.0, Animation::amplitude)
    }

    /// Factor on the shown values: the deformation factor, without its sign for magnitudes and
    /// equivalent values, which stay positive while a mode shape swings (as in PrePoMax).
    pub fn value_amplitude(&self) -> f32 {
        let amplitude = self.amplitude();
        match self.current() {
            Some((_, component)) if component.is_invariant() => amplitude.abs(),
            _ => amplitude,
        }
    }

    /// Opens an animation of the given kind over the step of the shown increment.
    pub fn start_animation(&mut self, kind: AnimationKind) {
        let start = self
            .animation
            .take()
            .map_or(self.increment, |a| a.start_increment);
        self.select_increment(start);
        let step = self.current_increment().map(|i| i.step);
        let modal = self
            .current_increment()
            .is_some_and(|i| matches!(i.kind, AnalysisKind::Frequency | AnalysisKind::Buckling));
        let increments = (0..self.increments.len())
            .filter(|&i| Some(self.increments[i].step) == step)
            .collect();
        self.animation = Some(Animation::new(kind, increments, start, modal));
        self.show_animation_frame();
    }

    /// Ends the animation and shows the increment from before it again.
    pub fn stop_animation(&mut self) {
        if let Some(animation) = self.animation.take() {
            self.select_increment(animation.start_increment);
        }
    }

    /// Selects the increment of the current frame of an increment animation.
    pub fn show_animation_frame(&mut self) {
        if let Some(increment) = self.animation.as_ref().and_then(Animation::increment)
            && increment != self.increment
        {
            self.select_increment(increment);
        }
    }

    /// Value range of the legend: that of the shown values, or over all animation frames.
    fn value_range(&self) -> Option<(f32, f32)> {
        let (field, component) = self.current()?;
        let (min, max) = component.range()?;
        let Some(animation) = &self.animation else {
            return Some((min, max));
        };
        match (animation.kind, animation.limits) {
            (AnimationKind::ScaleFactor, ColorLimits::CurrentFrame) => {
                // A negative factor swaps the ends.
                let (a, b) = (self.value_amplitude() * min, self.value_amplitude() * max);
                Some((a.min(b), a.max(b)))
            }
            // A mode shape swings every value between minus and plus its full size.
            (AnimationKind::ScaleFactor, ColorLimits::AllFrames)
                if animation.modal && !component.is_invariant() =>
            {
                Some((min.min(-max), max.max(-min)))
            }
            // Scaling runs every value from zero to its full size.
            (AnimationKind::ScaleFactor, ColorLimits::AllFrames) => {
                Some((min.min(0.0), max.max(0.0)))
            }
            (AnimationKind::Increments, ColorLimits::CurrentFrame) => Some((min, max)),
            (AnimationKind::Increments, ColorLimits::AllFrames) => animation
                .increments
                .iter()
                .filter_map(|&i| {
                    self.increments
                        .get(i)?
                        .field(&field.name)?
                        .component(&component.name)?
                        .range()
                })
                .reduce(|(a, b), (c, d)| (a.min(c), b.max(d))),
        }
    }

    /// Node index and value of the smallest value on screen, animation frame included.
    pub fn minimum(&self) -> Option<(usize, f32)> {
        self.extreme(|a, b| b < a)
    }

    fn extreme(&self, better: impl Fn(f32, f32) -> bool) -> Option<(usize, f32)> {
        let (_, component) = self.current()?;
        let amplitude = self.value_amplitude();
        component
            .values
            .iter()
            .map(|v| v * amplitude)
            .enumerate()
            .filter(|(_, v)| v.is_finite())
            .reduce(|a, b| if better(a.1, b.1) { b } else { a })
    }

    pub fn legend(&self) -> Option<Legend> {
        let (field, component) = self.current()?;
        let (min, max) = self.value_range()?;
        // PrePoMax writes names with blanks instead of underscores and dashes.
        let name = |n: &str| n.replace(['_', '-'], " ");
        let unit = (self.field_outputs.iter())
            .find(|o| o.name == field.name)
            .and_then(FieldOutput::unit)
            .filter(|u| !u.trim().is_empty() && u.trim() != "/")
            .map(|u| format!("\nUnit: {}", u.trim()))
            .unwrap_or_default();
        Some(Legend {
            title: format!(
                "{}: {}{unit}\nAutomatic",
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
    fn animation_scales_values_and_sets_legend_limits() {
        let still = [[0.0; 3]];
        let mut view = ResultsView::new(
            vec![
                increment(1, 1, AnalysisKind::Static, &[[0.0, 0.0, 1.0]]),
                increment(1, 2, AnalysisKind::Static, &[[0.0, 0.0, 3.0]]),
                increment(2, 1, AnalysisKind::Static, &still),
            ],
            Some(([0.0; 3], [10.0; 3])),
        );
        view.select_increment(0);
        view.field = 0;
        view.component = 3; // U3
        view.start_animation(AnimationKind::Increments);
        let animation = view.animation.as_mut().unwrap();
        assert_eq!(animation.increments, [0, 1]);
        animation.limits = ColorLimits::AllFrames;
        assert_eq!(view.legend().map(|l| (l.min, l.max)), Some((1.0, 3.0)));

        view.start_animation(AnimationKind::ScaleFactor);
        let animation = view.animation.as_mut().unwrap();
        animation.frames = 3;
        animation.go_to(1);
        assert_eq!(view.amplitude(), 0.5);
        assert_eq!(view.legend().map(|l| (l.min, l.max)), Some((0.5, 0.5)));
        view.animation.as_mut().unwrap().limits = ColorLimits::AllFrames;
        assert_eq!(view.legend().map(|l| (l.min, l.max)), Some((0.0, 1.0)));

        view.stop_animation();
        assert_eq!(view.increment, 0);
        assert_eq!(view.amplitude(), 1.0);
    }

    #[test]
    fn mode_shape_animation_swings_signed_values_but_not_magnitudes() {
        let mode = [[0.0, 0.0, 1.0], [0.0, 0.0, 3.0]];
        let mut view = ResultsView::new(
            vec![increment(1, 1, AnalysisKind::Frequency, &mode)],
            Some(([0.0; 3], [10.0; 3])),
        );
        view.component = 3; // U3
        view.start_animation(AnimationKind::ScaleFactor);
        let animation = view.animation.as_mut().unwrap();
        assert!(animation.modal);
        animation.go_to(0);
        assert_eq!(view.amplitude(), -1.0);
        assert_eq!(view.legend().map(|l| (l.min, l.max)), Some((-3.0, -1.0)));
        assert_eq!(view.maximum(), Some((0, -1.0)));
        assert_eq!(view.minimum(), Some((1, -3.0)));
        view.animation.as_mut().unwrap().limits = ColorLimits::AllFrames;
        assert_eq!(view.legend().map(|l| (l.min, l.max)), Some((-3.0, 3.0)));

        view.component = 0; // ALL stays positive
        assert_eq!(view.value_amplitude(), 1.0);
        assert_eq!(view.legend().map(|l| (l.min, l.max)), Some((0.0, 3.0)));
        view.animation.as_mut().unwrap().limits = ColorLimits::CurrentFrame;
        assert_eq!(view.legend().map(|l| (l.min, l.max)), Some((1.0, 3.0)));
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
