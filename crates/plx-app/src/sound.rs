//! Audio preview of eigenmodes: the eigenfrequencies of a frequency step played as sine tones,
//! alone or several together as one sound.
//!
//! Eigenfrequencies of real parts are often outside the audible range (a long beam hums at a
//! few hertz, a small bracket rings at tens of kilohertz). By default all modes are shifted by
//! the same number of octaves so the lowest chosen mode lands between 110 and 220 Hz; the
//! intervals between the modes, and with them the character of the sound, stay as they are.

use std::f64::consts::TAU;
use std::sync::{Arc, Mutex};

use plx_results::{AnalysisKind, Increment};

/// Lowest frequency a person hears.
pub const AUDIBLE_MIN: f64 = 20.0;
/// Highest frequency a person hears.
pub const AUDIBLE_MAX: f64 = 20_000.0;
/// Without shifting, modes are played as they are while all lie in this comfortable range.
const COMFORT_MIN: f64 = 40.0;
const COMFORT_MAX: f64 = 16_000.0;
/// An automatic shift puts the lowest mode just below this frequency (A3).
const AUTO_TARGET: f64 = 220.0;
/// Sample rate of exported WAV files.
pub const WAV_RATE: u32 = 44_100;

/// How the eigenfrequencies are made audible.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transpose {
    /// Played as computed; modes outside the audible range stay silent.
    Off,
    /// Shifted by whole octaves when a chosen mode is outside the comfortable range.
    Automatic,
    /// Shifted by the given number of octaves.
    Octaves,
}

/// How a tone develops over time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Envelope {
    /// Constant loudness until stopped.
    Sustained,
    /// Struck like a bell: every mode dies away, higher modes faster.
    Struck,
}

/// One mode of the frequency step in the sound window.
#[derive(Clone, Debug, PartialEq)]
pub struct Voice {
    /// Index into the increments of the results.
    pub increment: usize,
    pub mode: u32,
    /// Eigenfrequency in hertz.
    pub frequency: f64,
    pub enabled: bool,
    /// Relative loudness, 0 to 1.
    pub level: f32,
}

/// Settings of the sound window, kept with the results it belongs to.
pub struct ModeSound {
    pub step: u32,
    pub voices: Vec<Voice>,
    pub transpose: Transpose,
    /// Octaves of a manual shift.
    pub octaves: i32,
    pub envelope: Envelope,
    /// Time in seconds in which a struck lowest mode falls to about a third.
    pub decay: f32,
    pub volume: f32,
    /// Length of an exported sustained tone in seconds.
    pub duration: f32,
    /// Last problem with the audio device or the export, shown in the window.
    pub message: Option<String>,
}

/// A sine tone as the synthesizer plays it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tone {
    pub frequency: f64,
    pub amplitude: f32,
    /// Decay time constant in seconds; infinite for a sustained tone.
    pub decay: f32,
}

impl ModeSound {
    /// Sound window for the frequency step of the increment `shown`; the shown mode is the one
    /// enabled at the start. `None` when `shown` is not a mode.
    pub fn new(increments: &[Increment], shown: usize) -> Option<Self> {
        let current = increments.get(shown)?;
        if current.kind != AnalysisKind::Frequency {
            return None;
        }
        let voices = increments
            .iter()
            .enumerate()
            .filter(|(_, inc)| inc.step == current.step && inc.kind == AnalysisKind::Frequency)
            .map(|(index, inc)| Voice {
                increment: index,
                mode: inc.increment,
                frequency: inc.value,
                enabled: index == shown,
                level: 1.0,
            })
            .collect();
        Some(Self {
            step: current.step,
            voices,
            transpose: Transpose::Automatic,
            octaves: 0,
            envelope: Envelope::Sustained,
            decay: 1.5,
            volume: 0.5,
            duration: 3.0,
            message: None,
        })
    }

    /// Octaves every mode is shifted by.
    pub fn shift(&self) -> i32 {
        match self.transpose {
            Transpose::Off => 0,
            Transpose::Octaves => self.octaves,
            Transpose::Automatic => automatic_octaves(
                &self
                    .voices
                    .iter()
                    .filter(|v| v.enabled && v.level > 0.0)
                    .map(|v| v.frequency)
                    .collect::<Vec<_>>(),
            ),
        }
    }

    /// Frequency a voice is played at.
    pub fn played(&self, voice: &Voice) -> f64 {
        voice.frequency * 2f64.powi(self.shift())
    }

    /// The tones of the enabled, audible modes, scaled so their sum never clips.
    pub fn tones(&self) -> Vec<Tone> {
        let audible: Vec<(f64, f32)> = self
            .voices
            .iter()
            .filter(|v| v.enabled && v.level > 0.0)
            .map(|v| (self.played(v), v.level))
            .filter(|&(f, _)| is_audible(f))
            .collect();
        let total: f32 = audible.iter().map(|&(_, level)| level).sum();
        let lowest = audible
            .iter()
            .map(|&(f, _)| f)
            .fold(f64::INFINITY, f64::min);
        audible
            .iter()
            .map(|&(frequency, level)| Tone {
                frequency,
                amplitude: self.volume * level / total.max(1.0),
                decay: match self.envelope {
                    Envelope::Sustained => f32::INFINITY,
                    // Higher modes ring shorter, as in a struck bell or plate.
                    Envelope::Struck => self.decay * (lowest / frequency).sqrt() as f32,
                },
            })
            .collect()
    }

    /// Length of an exported file in seconds.
    pub fn export_seconds(&self) -> f32 {
        match self.envelope {
            Envelope::Sustained => self.duration,
            Envelope::Struck => (5.0 * self.decay).min(30.0),
        }
    }
}

pub fn is_audible(frequency: f64) -> bool {
    (AUDIBLE_MIN..=AUDIBLE_MAX).contains(&frequency)
}

/// Octave shift that makes the given frequencies audible: none while all lie in the
/// comfortable range, otherwise the lowest one is put between 110 and 220 Hz.
pub fn automatic_octaves(frequencies: &[f64]) -> i32 {
    let positive = frequencies.iter().copied().filter(|f| *f > 0.0);
    let (low, high) = positive.fold((f64::INFINITY, 0.0f64), |(lo, hi), f| {
        (lo.min(f), hi.max(f))
    });
    if !low.is_finite() || (low >= COMFORT_MIN && high <= COMFORT_MAX) {
        return 0;
    }
    (AUTO_TARGET / low).log2().floor() as i32
}

/// Sine synthesizer shared between the window and the audio callback.
#[derive(Default)]
pub struct Synth {
    tones: Vec<Tone>,
    phases: Vec<f64>,
    /// Seconds since the sound was started.
    time: f64,
    /// Loudness ramp against clicks: 0 silent, 1 full.
    gain: f32,
    gate: bool,
}

/// Time of the fade in and out in seconds.
const RAMP: f32 = 0.02;

impl Synth {
    /// Starts the tones from the beginning, e.g. strikes again.
    pub fn start(&mut self, tones: Vec<Tone>) {
        self.phases = vec![0.0; tones.len()];
        self.tones = tones;
        self.time = 0.0;
        self.gain = 0.0;
        self.gate = true;
    }

    /// Changes the tones of a running sound without restarting it.
    pub fn update(&mut self, tones: Vec<Tone>) {
        self.phases.resize(tones.len(), 0.0);
        self.tones = tones;
    }

    /// Fades out.
    pub fn stop(&mut self) {
        self.gate = false;
    }

    /// Whether anything can still be heard.
    pub fn sounding(&self) -> bool {
        (self.gate || self.gain > 0.0)
            && self
                .tones
                .iter()
                .any(|t| (self.time as f32) < 8.0 * t.decay)
    }

    pub fn next_sample(&mut self, rate: f64) -> f32 {
        let step = (1.0 / (RAMP as f64 * rate)) as f32;
        self.gain = if self.gate {
            (self.gain + step).min(1.0)
        } else {
            (self.gain - step).max(0.0)
        };
        if self.gain == 0.0 && !self.gate {
            return 0.0;
        }
        let time = self.time as f32;
        let mut sum = 0.0;
        for (tone, phase) in self.tones.iter().zip(&mut self.phases) {
            // Tones above the Nyquist frequency of the device would alias.
            if tone.frequency < rate / 2.0 {
                sum += tone.amplitude * (-time / tone.decay).exp() * (*phase * TAU).sin() as f32;
            }
            *phase = (*phase + tone.frequency / rate).fract();
        }
        self.time += 1.0 / rate;
        sum * self.gain
    }
}

/// Renders the tones into a mono 16 bit WAV file.
pub fn wav(tones: Vec<Tone>, seconds: f32) -> Vec<u8> {
    let rate = WAV_RATE as f64;
    let count = (seconds.max(0.1) as f64 * rate) as usize;
    let mut synth = Synth::default();
    synth.start(tones);
    let mut samples: Vec<f32> = (0..count).map(|_| synth.next_sample(rate)).collect();
    // Fade out at the end so the file stops without a click.
    let ramp = ((RAMP as f64 * rate) as usize).min(count);
    for (i, sample) in samples[count - ramp..].iter_mut().enumerate() {
        *sample *= 1.0 - (i + 1) as f32 / ramp as f32;
    }
    let data_len = (count * 2) as u32;
    let mut out = Vec::with_capacity(44 + count * 2);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&WAV_RATE.to_le_bytes());
    out.extend_from_slice(&(WAV_RATE * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for sample in samples {
        let value = (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        out.extend_from_slice(&value.to_le_bytes());
    }
    out
}

/// Output on the default audio device of the system.
pub struct Player {
    synth: Arc<Mutex<Synth>>,
    _stream: cpal::Stream,
}

impl Player {
    /// Opens the default output device; the error text is shown to the user.
    pub fn open() -> Result<Self, String> {
        use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
        use cpal::{FromSample, SampleFormat, SizedSample};

        fn build<T: SizedSample + FromSample<f32>>(
            device: &cpal::Device,
            config: cpal::StreamConfig,
            synth: Arc<Mutex<Synth>>,
        ) -> Result<cpal::Stream, cpal::Error> {
            let rate = config.sample_rate as f64;
            let channels = config.channels as usize;
            device.build_output_stream(
                config,
                move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
                    let mut synth = synth.lock().unwrap_or_else(|e| e.into_inner());
                    for frame in data.chunks_mut(channels) {
                        let value = T::from_sample(synth.next_sample(rate));
                        frame.fill(value);
                    }
                },
                |error| log::warn!("Audioausgabe: {error}"),
                None,
            )
        }

        let device = cpal::default_host()
            .default_output_device()
            .ok_or("Kein Audioausgabegerät gefunden")?;
        let config = device
            .default_output_config()
            .map_err(|e| format!("Audiogerät nicht nutzbar: {e}"))?;
        let synth = Arc::new(Mutex::new(Synth::default()));
        let shared = Arc::clone(&synth);
        let stream = match config.sample_format() {
            SampleFormat::F32 => build::<f32>(&device, config.into(), shared),
            SampleFormat::F64 => build::<f64>(&device, config.into(), shared),
            SampleFormat::I16 => build::<i16>(&device, config.into(), shared),
            SampleFormat::I32 => build::<i32>(&device, config.into(), shared),
            SampleFormat::U16 => build::<u16>(&device, config.into(), shared),
            SampleFormat::U8 => build::<u8>(&device, config.into(), shared),
            SampleFormat::I8 => build::<i8>(&device, config.into(), shared),
            other => return Err(format!("Nicht unterstütztes Audioformat {other}")),
        }
        .map_err(|e| format!("Audioausgabe konnte nicht geöffnet werden: {e}"))?;
        stream
            .play()
            .map_err(|e| format!("Audioausgabe konnte nicht gestartet werden: {e}"))?;
        Ok(Self {
            synth,
            _stream: stream,
        })
    }

    pub fn synth(&self) -> std::sync::MutexGuard<'_, Synth> {
        self.synth.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// What the user did in the sound window.
#[derive(Default)]
pub struct WindowActions {
    pub play: bool,
    pub stop: bool,
    /// The tones changed; a running sound takes them over.
    pub changed: bool,
    pub export: bool,
    pub close: bool,
    /// Increment of a mode the user clicked, to show it in the 3D view.
    pub show: Option<usize>,
}

fn format_hz(frequency: f64) -> String {
    match frequency.abs() {
        f if f >= 1000.0 => format!("{frequency:.1} Hz"),
        f if f >= 1.0 => format!("{frequency:.2} Hz"),
        _ => format!("{frequency:.4} Hz"),
    }
}

/// The sound window: modes of the step with their frequencies, how they are made audible,
/// and the player.
pub fn window(
    ctx: &egui::Context,
    sound: &mut ModeSound,
    shown: usize,
    playing: bool,
) -> WindowActions {
    use crate::icons::{self, Icon};
    use crate::numeric;

    let mut actions = WindowActions::default();
    let mut open = true;
    egui::Window::new("Klang der Eigenformen")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .pivot(egui::Align2::RIGHT_BOTTOM)
        .default_pos(ctx.content_rect().right_bottom() + egui::vec2(-130.0, -60.0))
        .show(ctx, |ui| {
            ui.label(format!(
                "Step {}: Moden ankreuzen, um sie zusammen zu hören",
                sound.step
            ));
            let shift = sound.shift();
            let factor = 2f64.powi(shift);
            egui::ScrollArea::vertical()
                .max_height(220.0)
                .show(ui, |ui| {
                    egui::Grid::new("sound modes")
                        .num_columns(5)
                        .spacing([12.0, 4.0])
                        .striped(true)
                        .show(ui, |ui| {
                            ui.label("");
                            ui.strong("Mode");
                            ui.strong("Frequenz");
                            ui.strong("Gespielt");
                            ui.strong("Pegel");
                            ui.end_row();
                            for voice in &mut sound.voices {
                                actions.changed |= ui.checkbox(&mut voice.enabled, "").changed();
                                let label = ui
                                    .selectable_label(
                                        voice.increment == shown,
                                        voice.mode.to_string(),
                                    )
                                    .on_hover_text("Diese Eigenform anzeigen");
                                if label.clicked() {
                                    actions.show = Some(voice.increment);
                                }
                                ui.label(format_hz(voice.frequency));
                                let played = voice.frequency * factor;
                                if is_audible(played) {
                                    ui.label(format_hz(played));
                                } else {
                                    ui.weak("unhörbar");
                                }
                                actions.changed |= ui
                                    .add(
                                        egui::Slider::new(&mut voice.level, 0.0..=1.0)
                                            .show_value(false),
                                    )
                                    .changed();
                                ui.end_row();
                            }
                        });
                });
            ui.horizontal(|ui| {
                for (label, enabled) in [("Alle", true), ("Keine", false)] {
                    if ui.button(label).clicked() {
                        sound.voices.iter_mut().for_each(|v| v.enabled = enabled);
                        actions.changed = true;
                    }
                }
            });
            ui.separator();
            egui::Grid::new("sound settings")
                .num_columns(2)
                .spacing([12.0, 6.0])
                .show(ui, |ui| {
                    ui.label("Hörbar machen");
                    ui.horizontal(|ui| {
                        for (transpose, label, tip) in [
                            (
                                Transpose::Automatic,
                                "Automatisch",
                                "Um ganze Oktaven verschieben, wenn eine Mode außerhalb von \
                                 40 Hz bis 16 kHz liegt; die Intervalle bleiben erhalten",
                            ),
                            (
                                Transpose::Octaves,
                                "Oktaven",
                                "Um feste Oktaven verschieben",
                            ),
                            (Transpose::Off, "Original", "Die berechneten Frequenzen"),
                        ] {
                            actions.changed |= ui
                                .radio_value(&mut sound.transpose, transpose, label)
                                .on_hover_text(tip)
                                .changed();
                        }
                    });
                    ui.end_row();
                    ui.label("Verschiebung");
                    ui.horizontal(|ui| {
                        if sound.transpose == Transpose::Octaves {
                            actions.changed |= ui
                                .add(numeric::drag_value(&mut sound.octaves).range(-16..=16))
                                .changed();
                            ui.label("Oktaven");
                        } else {
                            ui.label(format!("{shift:+} Oktaven"));
                        }
                        ui.weak(format!("(Faktor {})", format_factor(shift)));
                    });
                    ui.end_row();
                    ui.label("Klang");
                    ui.horizontal(|ui| {
                        for (envelope, label) in [
                            (Envelope::Sustained, "Dauerton"),
                            (Envelope::Struck, "Angeschlagen"),
                        ] {
                            actions.changed |= ui
                                .radio_value(&mut sound.envelope, envelope, label)
                                .changed();
                        }
                    });
                    ui.end_row();
                    if sound.envelope == Envelope::Struck {
                        ui.label("Abklingzeit");
                        ui.horizontal(|ui| {
                            actions.changed |= ui
                                .add(numeric::drag_value(&mut sound.decay).range(0.1..=10.0))
                                .changed();
                            ui.label("s");
                        });
                    } else {
                        ui.label("Dauer der WAV-Datei");
                        ui.horizontal(|ui| {
                            ui.add(numeric::drag_value(&mut sound.duration).range(0.5..=60.0));
                            ui.label("s");
                        });
                    }
                    ui.end_row();
                    ui.label("Lautstärke");
                    actions.changed |= ui
                        .add(egui::Slider::new(&mut sound.volume, 0.0..=1.0).show_value(false))
                        .changed();
                    ui.end_row();
                });
            ui.separator();
            ui.horizontal(|ui| {
                let audible = !sound.tones().is_empty();
                let (icon, tip) = if playing {
                    (Icon::Pause, "Anhalten")
                } else {
                    (Icon::Animate, "Abspielen")
                };
                if icons::button(ui, icon, tip, playing || audible, false).clicked() {
                    if playing {
                        actions.stop = true;
                    } else {
                        actions.play = true;
                    }
                }
                if sound.envelope == Envelope::Struck
                    && icons::button(ui, Icon::Sound, "Erneut anschlagen", audible, false).clicked()
                {
                    actions.play = true;
                }
                ui.add_space(8.0);
                if ui
                    .add_enabled(audible, egui::Button::new("Als WAV speichern..."))
                    .clicked()
                {
                    actions.export = true;
                }
                if !audible {
                    ui.weak("Keine hörbare Mode gewählt");
                }
            });
            if let Some(message) = &sound.message {
                ui.colored_label(ui.visuals().error_fg_color, message);
            }
        });
    actions.close = !open;
    actions
}

fn format_factor(octaves: i32) -> String {
    if octaves >= 0 {
        format!("{}", 1u64 << octaves.min(62))
    } else {
        format!("1/{}", 1u64 << (-octaves).min(62))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mode(step: u32, number: u32, kind: AnalysisKind, frequency: f64) -> Increment {
        Increment {
            step,
            increment: number,
            kind,
            value: frequency,
            fields: Vec::new(),
        }
    }

    fn modes(frequencies: &[f64]) -> Vec<Increment> {
        let mut increments = vec![mode(1, 1, AnalysisKind::Static, 1.0)];
        for (i, &f) in frequencies.iter().enumerate() {
            increments.push(mode(2, i as u32 + 1, AnalysisKind::Frequency, f));
        }
        increments
    }

    #[test]
    fn only_modes_open_the_window_with_the_shown_mode_enabled() {
        let increments = modes(&[100.0, 250.0, 400.0]);
        assert!(ModeSound::new(&increments, 0).is_none());
        let sound = ModeSound::new(&increments, 2).unwrap();
        assert_eq!(sound.step, 2);
        assert_eq!(sound.voices.len(), 3);
        let enabled: Vec<u32> = (sound.voices.iter())
            .filter(|v| v.enabled)
            .map(|v| v.mode)
            .collect();
        assert_eq!(enabled, [2]);
    }

    #[test]
    fn automatic_shift_keeps_audible_modes_and_moves_the_others_by_octaves() {
        assert_eq!(automatic_octaves(&[100.0, 3000.0]), 0);
        // 3 Hz is moved up six octaves to 192 Hz.
        assert_eq!(automatic_octaves(&[3.0, 7.5]), 6);
        // 50 kHz is moved down eight octaves to about 195 Hz.
        assert_eq!(automatic_octaves(&[50_000.0, 80_000.0]), -8);
        assert_eq!(automatic_octaves(&[]), 0);
        let low = 50_000.0 * 2f64.powi(-8);
        assert!((110.0..220.0).contains(&low));
    }

    #[test]
    fn combined_tones_keep_their_ratio_and_never_clip() {
        let increments = modes(&[3.0, 7.5, 1.0e6]);
        let mut sound = ModeSound::new(&increments, 1).unwrap();
        for voice in &mut sound.voices {
            voice.enabled = true;
        }
        sound.volume = 1.0;
        let tones = sound.tones();
        // The third mode is shifted far above hearing and left out.
        assert_eq!(tones.len(), 2);
        assert!((tones[1].frequency / tones[0].frequency - 2.5).abs() < 1e-9);
        let peak: f32 = tones.iter().map(|t| t.amplitude).sum();
        assert!(peak <= 1.0);
        sound.transpose = Transpose::Off;
        assert!(sound.tones().is_empty());
    }

    #[test]
    fn struck_sound_dies_away_and_higher_modes_faster() {
        let mut sound = ModeSound::new(&modes(&[200.0, 800.0]), 1).unwrap();
        sound.voices[1].enabled = true;
        sound.envelope = Envelope::Struck;
        let tones = sound.tones();
        assert!(tones[1].decay < tones[0].decay);
        let mut synth = Synth::default();
        synth.start(tones);
        let rate = 8000.0;
        let early: f32 = (0..800).map(|_| synth.next_sample(rate).abs()).sum();
        for _ in 0..40_000 {
            synth.next_sample(rate);
        }
        let late: f32 = (0..800).map(|_| synth.next_sample(rate).abs()).sum();
        assert!(late < early * 0.05);
    }

    #[test]
    fn synth_plays_a_sine_and_fades_out_after_stop() {
        let mut synth = Synth::default();
        let tone = Tone {
            frequency: 1000.0,
            amplitude: 0.5,
            decay: f32::INFINITY,
        };
        synth.start(vec![tone]);
        let rate = 48_000.0;
        let samples: Vec<f32> = (0..4800).map(|_| synth.next_sample(rate)).collect();
        let peak = samples[2400..].iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!((peak - 0.5).abs() < 0.01);
        assert!(synth.sounding());
        synth.stop();
        for _ in 0..2000 {
            synth.next_sample(rate);
        }
        assert!(!synth.sounding());
        assert_eq!(synth.next_sample(rate), 0.0);
    }

    #[test]
    fn wav_has_a_valid_header() {
        let tone = Tone {
            frequency: 440.0,
            amplitude: 0.5,
            decay: f32::INFINITY,
        };
        let data = wav(vec![tone], 0.5);
        let samples = (WAV_RATE / 2) as usize;
        assert_eq!(data.len(), 44 + samples * 2);
        assert_eq!(&data[0..4], b"RIFF");
        assert_eq!(&data[8..16], b"WAVEfmt ");
        assert_eq!(
            u32::from_le_bytes(data[40..44].try_into().unwrap()),
            samples as u32 * 2
        );
        // The last sample is faded to silence.
        assert_eq!(
            i16::from_le_bytes([data[data.len() - 2], data[data.len() - 1]]),
            0
        );
    }
}
