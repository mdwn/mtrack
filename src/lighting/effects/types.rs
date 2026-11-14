// Copyright (C) 2026 Michael Wilson <mike@mdwn.dev>
//
// This program is free software: you can redistribute it and/or modify it under
// the terms of the GNU General Public License as published by the Free Software
// Foundation, version 3.
//
// This program is distributed in the hope that it will be useful, but WITHOUT
// ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS
// FOR A PARTICULAR PURPOSE. See the GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License along with
// this program. If not, see <https://www.gnu.org/licenses/>.
//

use std::collections::HashMap;
use std::time::Duration;

use super::color::Color;
use super::tempo_aware::{TempoAwareFrequency, TempoAwareSpeed};

/// Core effect types for lighting.
/// All effects have an explicit, finite duration.
#[derive(Debug, Clone)]
pub enum EffectType {
    /// Static effect with fixed parameter values
    Static {
        parameters: HashMap<String, f64>,
        duration: Duration,
    },

    /// Color cycle effect
    ColorCycle {
        colors: Vec<Color>,
        speed: TempoAwareSpeed, // cycles per second (can be tempo-aware)
        direction: CycleDirection,
        transition: CycleTransition, // how to transition between colors
        duration: Duration,
    },

    /// Strobe effect
    Strobe {
        frequency: TempoAwareFrequency, // Hz (can be tempo-aware)
        duration: Duration,
    },

    /// Dimmer effect with smooth transitions
    Dimmer {
        start_level: f64,
        end_level: f64,
        duration: Duration,
        curve: DimmerCurve,
    },

    /// Chase effect that moves across fixtures
    Chase {
        pattern: ChasePattern,
        speed: TempoAwareSpeed, // cycles per second (can be tempo-aware)
        direction: ChaseDirection,
        transition: CycleTransition, // how to transition between fixtures (fade in/out)
        duration: Duration,
    },

    /// Rainbow effect
    Rainbow {
        speed: TempoAwareSpeed, // cycles per second (can be tempo-aware)
        saturation: f64,
        brightness: f64,
        duration: Duration,
    },

    /// Pulse effect
    Pulse {
        base_level: f64,
        pulse_amplitude: f64,
        frequency: TempoAwareFrequency, // Hz (can be tempo-aware)
        duration: Duration,
    },

    /// Movement: aim the group at a venue focus point or explicit angles,
    /// travelling there over `duration` in physical space. The pose holds
    /// after arrival (pose memory, design §15.4).
    Move {
        /// Where to go.
        to: MoveTarget,
        /// Where to start; the fixture's current pose when absent.
        from: Option<MoveTarget>,
        easing: Easing,
        duration: Duration,
    },
}

/// A place a `move` aims at.
#[derive(Debug, Clone, PartialEq)]
pub enum MoveTarget {
    /// A named focus point, bound by the venue.
    Focus(String),
    /// Explicit angles in degrees; a `None` leaves that axis where it is.
    Angles { pan: Option<f64>, tilt: Option<f64> },
}

impl MoveTarget {
    /// Whether the target is a set of angles with neither axis given.
    pub fn is_empty(&self) -> bool {
        matches!(
            self,
            MoveTarget::Angles {
                pan: None,
                tilt: None
            }
        )
    }
}

/// How a move's progress maps onto its travel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Easing {
    Linear,
    #[default]
    Smooth,
}

impl Easing {
    /// Progress 0..1 to travel fraction 0..1.
    pub fn apply(self, t: f64) -> f64 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Easing::Linear => t,
            // Ease-in-out: smoothstep.
            Easing::Smooth => t * t * (3.0 - 2.0 * t),
        }
    }
}

impl EffectType {
    /// Get the duration field from the effect type
    pub fn duration(&self) -> Duration {
        match self {
            EffectType::Static { duration, .. }
            | EffectType::Strobe { duration, .. }
            | EffectType::Pulse { duration, .. }
            | EffectType::Dimmer { duration, .. }
            | EffectType::ColorCycle { duration, .. }
            | EffectType::Chase { duration, .. }
            | EffectType::Rainbow { duration, .. }
            | EffectType::Move { duration, .. } => *duration,
        }
    }

    /// How long an effect of this type holds when no `hold_time` is written
    /// for it: its own duration. A dimmer has no hold: it runs its duration
    /// and ends.
    pub fn default_hold(&self) -> Option<Duration> {
        match self {
            EffectType::Dimmer { .. } => None,
            _ => Some(self.duration()),
        }
    }

    /// How long an effect of this type lasts under the envelope written for
    /// it: its fade in, its hold (see [`Self::default_hold`]) and its fade
    /// out; a dimmer its own duration. All effects have a finite duration.
    pub fn total_duration(
        &self,
        up_time: Option<Duration>,
        hold_time: Option<Duration>,
        down_time: Option<Duration>,
    ) -> Duration {
        if let EffectType::Dimmer { duration, .. } = self {
            return *duration;
        }
        up_time.unwrap_or(Duration::ZERO)
            + hold_time
                .or_else(|| self.default_hold())
                .unwrap_or(Duration::ZERO)
            + down_time.unwrap_or(Duration::ZERO)
    }

    /// Replace the duration field.
    pub fn set_duration(&mut self, new_duration: Duration) {
        match self {
            EffectType::Static { duration, .. }
            | EffectType::Strobe { duration, .. }
            | EffectType::Pulse { duration, .. }
            | EffectType::Dimmer { duration, .. }
            | EffectType::ColorCycle { duration, .. }
            | EffectType::Chase { duration, .. }
            | EffectType::Rainbow { duration, .. }
            | EffectType::Move { duration, .. } => *duration = new_duration,
        }
    }

    /// The effect's authored parameters, as displayable strings keyed by the
    /// name the DSL uses.
    ///
    /// `name()` alone does not identify an effect: two `static` cues differing
    /// only in colour or level are the same kind and a completely different
    /// look. Anything comparing effects — `diff_shows` above all — needs these
    /// as well, so they live here rather than being re-derived per caller.
    pub fn parameters(&self) -> std::collections::BTreeMap<String, String> {
        let mut out: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
        match self {
            EffectType::Static { parameters, .. } => {
                // Keyed by the DSL spelling the author used (`color`, `dimmer`,
                // `intensity`, individual channels).
                for (key, value) in parameters {
                    out.insert(key.clone(), format!("{value}"));
                }
            }
            EffectType::ColorCycle {
                colors,
                speed,
                direction,
                transition,
                ..
            } => {
                out.insert("colors".to_string(), format!("{colors:?}"));
                out.insert("speed".to_string(), format!("{speed:?}"));
                out.insert("direction".to_string(), format!("{direction:?}"));
                out.insert("transition".to_string(), format!("{transition:?}"));
            }
            EffectType::Strobe { frequency, .. } => {
                out.insert("frequency".to_string(), format!("{frequency:?}"));
            }
            EffectType::Dimmer {
                start_level,
                end_level,
                curve,
                ..
            } => {
                out.insert("start_level".to_string(), format!("{start_level}"));
                out.insert("end_level".to_string(), format!("{end_level}"));
                out.insert("curve".to_string(), format!("{curve:?}"));
            }
            EffectType::Chase {
                pattern,
                speed,
                direction,
                transition,
                ..
            } => {
                out.insert("pattern".to_string(), format!("{pattern:?}"));
                out.insert("speed".to_string(), format!("{speed:?}"));
                out.insert("direction".to_string(), format!("{direction:?}"));
                out.insert("transition".to_string(), format!("{transition:?}"));
            }
            EffectType::Move {
                to, from, easing, ..
            } => {
                let describe = |target: &MoveTarget| match target {
                    MoveTarget::Focus(name) => format!("\"{name}\""),
                    MoveTarget::Angles { pan, tilt } => format!(
                        "pan {} tilt {}",
                        pan.map(|p| format!("{p}deg"))
                            .unwrap_or_else(|| "-".to_string()),
                        tilt.map(|t| format!("{t}deg"))
                            .unwrap_or_else(|| "-".to_string())
                    ),
                };
                out.insert("to".to_string(), describe(to));
                if let Some(from) = from {
                    out.insert("from".to_string(), describe(from));
                }
                out.insert("easing".to_string(), format!("{easing:?}"));
            }
            EffectType::Rainbow {
                speed,
                saturation,
                brightness,
                ..
            } => {
                out.insert("speed".to_string(), format!("{speed:?}"));
                out.insert("saturation".to_string(), format!("{saturation}"));
                out.insert("brightness".to_string(), format!("{brightness}"));
            }
            EffectType::Pulse {
                base_level,
                pulse_amplitude,
                frequency,
                ..
            } => {
                out.insert("base_level".to_string(), format!("{base_level}"));
                out.insert("pulse_amplitude".to_string(), format!("{pulse_amplitude}"));
                out.insert("frequency".to_string(), format!("{frequency:?}"));
            }
        }
        out
    }

    /// The effect's name as reported to users — status output, MCP responses,
    /// and offline evaluation all use this spelling.
    pub fn name(&self) -> &'static str {
        match self {
            EffectType::Static { .. } => "Static",
            EffectType::ColorCycle { .. } => "ColorCycle",
            EffectType::Strobe { .. } => "Strobe",
            EffectType::Dimmer { .. } => "Dimmer",
            EffectType::Chase { .. } => "Chase",
            EffectType::Rainbow { .. } => "Rainbow",
            EffectType::Pulse { .. } => "Pulse",
            EffectType::Move { .. } => "Move",
        }
    }
}

/// Cycle direction for color cycling effects
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CycleDirection {
    Forward,
    Backward,
    PingPong,
}

/// Transition type for color cycling effects
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CycleTransition {
    /// Snap instantly between colors
    Snap,
    /// Fade smoothly between colors
    Fade,
    /// Fade around the colour wheel: the hue turns the short way while
    /// saturation and brightness blend, so yellow to blue passes through
    /// red and magenta rather than grey. Cycles only.
    Hue,
}

/// Chase pattern for spatial effects
#[derive(Debug, Clone, PartialEq)]
pub enum ChasePattern {
    Linear,
    Snake,
    Random,
}

/// Chase direction for spatial effects
#[derive(Debug, Clone, Copy)]
pub enum ChaseDirection {
    LeftToRight,
    RightToLeft,
    TopToBottom,
    BottomToTop,
    Clockwise,
    CounterClockwise,
}

/// Dimmer curve types
#[derive(Debug, Clone)]
pub enum DimmerCurve {
    Linear,
    Exponential,
    Logarithmic,
    Sine,
    Cosine,
}

/// Effect layer for layering system: an index into an ordered [`LayerSet`],
/// bottom (0) to top, ordered by index. The name lives in the set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EffectLayer(pub u8);

impl EffectLayer {
    /// Base layer of the default set (e.g., static colors).
    pub const BACKGROUND: EffectLayer = EffectLayer(0);
    /// Middle layer of the default set (e.g., dimmer effects).
    pub const MIDGROUND: EffectLayer = EffectLayer(1);
    /// Top layer of the default set (e.g., strobe effects).
    pub const FOREGROUND: EffectLayer = EffectLayer(2);

    /// The layer's position in its [`LayerSet`], bottom first.
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// The layer an effect runs on when the DSL names none: the bottom layer
/// of every set.
pub const DEFAULT_LAYER: EffectLayer = EffectLayer(0);

/// The most layers a [`LayerSet`] holds.
pub const MAX_LAYERS: usize = 16;

/// The names of the layers, bottom to top. The position of a name is its
/// [`EffectLayer`] index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayerSet {
    names: Vec<String>,
}

/// The default layers, bottom to top.
const DEFAULT_LAYER_NAMES: [&str; 3] = ["background", "midground", "foreground"];

impl LayerSet {
    /// A set of the given names, bottom to top. Names must be non-empty,
    /// unique, and between one and [`MAX_LAYERS`] of them.
    pub fn new(names: Vec<String>) -> Result<Self, String> {
        if names.is_empty() {
            return Err("at least one layer is required".to_string());
        }
        if names.len() > MAX_LAYERS {
            return Err(format!(
                "at most {} layers are allowed, got {}",
                MAX_LAYERS,
                names.len()
            ));
        }
        for (i, name) in names.iter().enumerate() {
            if name.is_empty() {
                return Err("layer names cannot be empty".to_string());
            }
            if names[..i].contains(name) {
                return Err(format!("duplicate layer name '{}'", name));
            }
        }
        Ok(Self { names })
    }

    /// The default set, shared process-wide: `background`, `midground`,
    /// `foreground`. L1 uses only this; callers that will take a
    /// configured set take a `&LayerSet` and default to this.
    pub fn standard() -> &'static LayerSet {
        static STANDARD: std::sync::OnceLock<LayerSet> = std::sync::OnceLock::new();
        STANDARD.get_or_init(LayerSet::default)
    }

    /// How many layers there are.
    pub fn len(&self) -> usize {
        self.names.len()
    }

    /// Always false: a set holds at least one layer.
    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    /// The layers' names, bottom to top.
    pub fn names(&self) -> &[String] {
        &self.names
    }

    /// The layers, bottom to top.
    pub fn layers(&self) -> impl Iterator<Item = EffectLayer> {
        (0..self.names.len() as u8).map(EffectLayer)
    }

    /// The bottom layer, where an effect runs when it names none.
    pub fn bottom(&self) -> EffectLayer {
        DEFAULT_LAYER
    }

    /// The top layer.
    pub fn top(&self) -> EffectLayer {
        EffectLayer((self.names.len() - 1) as u8)
    }

    /// The layer's name, as the DSL writes it.
    pub fn name(&self, layer: EffectLayer) -> Option<&str> {
        self.names.get(layer.index()).map(String::as_str)
    }

    /// The layer's name, or `layer<N>` for an index outside the set.
    pub fn display_name(&self, layer: EffectLayer) -> String {
        self.name(layer)
            .map(str::to_string)
            .unwrap_or_else(|| format!("layer{}", layer.0))
    }

    /// The layer a name denotes.
    pub fn parse(&self, name: &str) -> Option<EffectLayer> {
        self.names
            .iter()
            .position(|n| n == name)
            .map(|i| EffectLayer(i as u8))
    }
}

impl Default for LayerSet {
    fn default() -> Self {
        Self {
            names: DEFAULT_LAYER_NAMES.iter().map(|n| n.to_string()).collect(),
        }
    }
}

/// Blend mode for combining effects
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BlendMode {
    /// Replace - higher layer completely replaces lower layer
    Replace,
    /// Multiply - multiply values together (good for dimming)
    Multiply,
    /// Add - add values together (good for color mixing)
    Add,
    /// Overlay - overlay effect (good for strobes)
    Overlay,
    /// Screen - screen blend mode
    Screen,
}

/// The blend mode an effect runs with when the DSL names none.
pub const DEFAULT_BLEND: BlendMode = BlendMode::Replace;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effect_type_static_duration() {
        let effect = EffectType::Static {
            parameters: HashMap::new(),
            duration: Duration::from_secs(5),
        };
        assert_eq!(effect.duration(), Duration::from_secs(5));
    }

    #[test]
    fn effect_type_strobe_duration() {
        let effect = EffectType::Strobe {
            frequency: TempoAwareFrequency::Fixed(10.0),
            duration: Duration::from_millis(500),
        };
        assert_eq!(effect.duration(), Duration::from_millis(500));
    }

    #[test]
    fn effect_type_pulse_duration() {
        let effect = EffectType::Pulse {
            base_level: 0.0,
            pulse_amplitude: 1.0,
            frequency: TempoAwareFrequency::Fixed(2.0),
            duration: Duration::from_secs(3),
        };
        assert_eq!(effect.duration(), Duration::from_secs(3));
    }

    #[test]
    fn effect_type_dimmer_duration() {
        let effect = EffectType::Dimmer {
            start_level: 0.0,
            end_level: 1.0,
            duration: Duration::from_secs(2),
            curve: DimmerCurve::Linear,
        };
        assert_eq!(effect.duration(), Duration::from_secs(2));
    }

    #[test]
    fn effect_type_color_cycle_duration() {
        let effect = EffectType::ColorCycle {
            colors: vec![Color::new(255, 0, 0), Color::new(0, 0, 255)],
            speed: TempoAwareSpeed::Fixed(1.0),
            direction: CycleDirection::Forward,
            transition: CycleTransition::Fade,
            duration: Duration::from_secs(10),
        };
        assert_eq!(effect.duration(), Duration::from_secs(10));
    }

    #[test]
    fn effect_type_chase_duration() {
        let effect = EffectType::Chase {
            pattern: ChasePattern::Linear,
            speed: TempoAwareSpeed::Fixed(1.0),
            direction: ChaseDirection::LeftToRight,
            transition: CycleTransition::Snap,
            duration: Duration::from_secs(5),
        };
        assert_eq!(effect.duration(), Duration::from_secs(5));
    }

    #[test]
    fn effect_type_rainbow_duration() {
        let effect = EffectType::Rainbow {
            speed: TempoAwareSpeed::Fixed(0.5),
            saturation: 1.0,
            brightness: 1.0,
            duration: Duration::from_secs(8),
        };
        assert_eq!(effect.duration(), Duration::from_secs(8));
    }

    #[test]
    fn effect_layer_ordering() {
        assert!(EffectLayer::BACKGROUND < EffectLayer::MIDGROUND);
        assert!(EffectLayer::MIDGROUND < EffectLayer::FOREGROUND);
    }

    #[test]
    fn the_default_layer_set_names_the_three_layers() {
        let set = LayerSet::standard();
        assert_eq!(set.len(), 3);
        assert_eq!(set.bottom(), EffectLayer::BACKGROUND);
        assert_eq!(set.top(), EffectLayer::FOREGROUND);
        assert_eq!(set.name(EffectLayer::MIDGROUND), Some("midground"));
        assert_eq!(set.parse("foreground"), Some(EffectLayer::FOREGROUND));
        assert_eq!(set.parse("accents"), None);
        assert_eq!(set.name(EffectLayer(3)), None);
        assert_eq!(set.display_name(EffectLayer(3)), "layer3");
        assert_eq!(set.layers().count(), 3);
    }

    #[test]
    fn a_layer_set_refuses_empty_duplicate_and_too_many() {
        assert!(LayerSet::new(vec![]).is_err());
        assert!(LayerSet::new(vec!["a".into(), "a".into()]).is_err());
        assert!(LayerSet::new(vec!["".into()]).is_err());
        let many: Vec<String> = (0..=MAX_LAYERS).map(|i| format!("l{i}")).collect();
        assert!(LayerSet::new(many).is_err());
        let ok: Vec<String> = (0..MAX_LAYERS).map(|i| format!("l{i}")).collect();
        let set = LayerSet::new(ok).unwrap();
        assert_eq!(set.top(), EffectLayer(15));
    }

    #[test]
    fn cycle_direction_equality() {
        assert_eq!(CycleDirection::Forward, CycleDirection::Forward);
        assert_ne!(CycleDirection::Forward, CycleDirection::Backward);
        assert_ne!(CycleDirection::Backward, CycleDirection::PingPong);
    }

    #[test]
    fn cycle_transition_equality() {
        assert_eq!(CycleTransition::Snap, CycleTransition::Snap);
        assert_ne!(CycleTransition::Snap, CycleTransition::Fade);
    }

    #[test]
    fn blend_mode_equality() {
        assert_eq!(BlendMode::Replace, BlendMode::Replace);
        assert_ne!(BlendMode::Replace, BlendMode::Multiply);
        assert_ne!(BlendMode::Add, BlendMode::Screen);
    }

    // ── total_duration ─────────────────────────────────────────────

    #[test]
    fn total_duration_holds_for_the_effect_duration_unless_a_hold_is_written() {
        let effect = EffectType::Static {
            parameters: HashMap::new(),
            duration: Duration::from_secs(5),
        };
        let up = Some(Duration::from_secs(1));
        let down = Some(Duration::from_secs(2));
        assert_eq!(effect.default_hold(), Some(Duration::from_secs(5)));
        assert_eq!(
            effect.total_duration(None, None, None),
            Duration::from_secs(5)
        );
        assert_eq!(
            effect.total_duration(up, None, down),
            Duration::from_secs(8)
        );
        assert_eq!(
            effect.total_duration(up, Some(Duration::from_secs(3)), down),
            Duration::from_secs(6)
        );
    }

    #[test]
    fn total_duration_of_a_dimmer_is_its_own_whatever_the_envelope() {
        let effect = EffectType::Dimmer {
            start_level: 0.0,
            end_level: 1.0,
            duration: Duration::from_secs(4),
            curve: DimmerCurve::Linear,
        };
        assert_eq!(effect.default_hold(), None);
        assert_eq!(
            effect.total_duration(
                Some(Duration::from_secs(1)),
                Some(Duration::from_secs(9)),
                None
            ),
            Duration::from_secs(4)
        );
    }
}
