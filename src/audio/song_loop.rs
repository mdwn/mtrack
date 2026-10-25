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

//! The crossover between passes of a looping song with a `loop_crossfade`.
//!
//! The next pass starts `overlap` before the current one ends. Over the
//! overlap the two cross on an equal-power curve, so a long crossfade holds
//! its level; tracks named in `loop_crossfade_cut` instead switch over at the
//! midpoint with the short default crossfade, so a click is never heard
//! twice.

use std::sync::Arc;
use std::time::Duration;

use super::crossfade::{CrossfadeCurve, GainEnvelope};
use super::mixer::AudioMixer;

/// Which way a source crosses the overlap.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum PassGroup {
    /// Faded over the whole overlap.
    Fade,
    /// Switched over at the overlap's midpoint.
    Cut,
}

/// One crossover between passes, in mixer samples.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PassCrossover {
    /// The sample the incoming pass starts at.
    pub start_sample: u64,
    /// How long the passes overlap.
    pub overlap_samples: u64,
    /// The short crossfade a cut track switches over with.
    pub crossfade_samples: u64,
}

impl PassCrossover {
    /// Where cut tracks switch over: the overlap's midpoint.
    pub(crate) fn cut_sample(&self) -> u64 {
        self.start_sample + self.overlap_samples / 2
    }

    /// Schedules the outgoing pass's sources: those fading fade out over the
    /// overlap and end with it, those cut play to the midpoint and switch
    /// there.
    pub(crate) fn cross_out(&self, mixer: &AudioMixer, fade_ids: &[u64], cut_ids: &[u64]) {
        if !fade_ids.is_empty() {
            mixer.set_gain_envelope(
                fade_ids,
                Arc::new(
                    GainEnvelope::fade_out(self.overlap_samples, CrossfadeCurve::EqualPower)
                        .with_start_sample(self.start_sample),
                ),
            );
            mixer.set_cancel_at_sample(fade_ids, self.start_sample + self.overlap_samples);
        }
        if !cut_ids.is_empty() {
            mixer.set_gain_envelope(
                cut_ids,
                Arc::new(
                    GainEnvelope::fade_out(self.crossfade_samples, CrossfadeCurve::Linear)
                        .with_start_sample(self.cut_sample()),
                ),
            );
            mixer.set_cancel_at_sample(cut_ids, self.cut_sample() + self.crossfade_samples);
        }
    }

    /// The envelope an incoming source in `group` fades in with, and the
    /// sample it starts at. A cut source is built from the song time the
    /// midpoint falls on, so it starts there in step.
    pub(crate) fn cross_in(&self, group: PassGroup) -> (GainEnvelope, u64) {
        match group {
            PassGroup::Fade => (
                GainEnvelope::fade_in(self.overlap_samples, CrossfadeCurve::EqualPower),
                self.start_sample,
            ),
            PassGroup::Cut => (
                GainEnvelope::fade_in(self.crossfade_samples, CrossfadeCurve::Linear),
                self.cut_sample(),
            ),
        }
    }
}

/// A loop break's fade-out (`loop_break_fade`) that outlasts the pass it
/// began in: where the next pass has to carry it on. A loop break fades the
/// song out from full gain on `curve`'s out half; the pass it began in
/// ends at `pass_end` on the playback clock, `now` is the clock now,
/// `since_break` how long ago the break was, and `fade` the fade's length.
/// Returns the gain the fade has reached at `pass_end` and how long it still
/// has to run from there, or `None` when the pass outlasts the fade (with a
/// little to spare: a pass ending in the fade's last moments is not worth a
/// new one).
pub(crate) fn break_fade_continuation(
    pass_end: Duration,
    now: Duration,
    since_break: Duration,
    fade: Duration,
    curve: CrossfadeCurve,
) -> Option<(f32, Duration)> {
    const SPARE: Duration = Duration::from_millis(50);
    let fade_end = (now + fade).saturating_sub(since_break);
    let from = pass_end.max(now);
    if from + SPARE >= fade_end {
        return None;
    }
    let into = (from + since_break).saturating_sub(now);
    let t = (into.as_secs_f64() / fade.as_secs_f64()).clamp(0.0, 1.0) as f32;
    let (gain, _) = curve.gains(t);
    Some((gain, fade_end - from))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::mixer::ActiveSource;
    use crate::audio::sample_source::{ChannelMappedSource, MemorySampleSource};
    use crate::playsync::CancelHandle;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicBool, AtomicU64};

    /// A source playing `value` on track `track` for `frames` frames.
    fn source(id: u64, track: &str, value: f32, frames: usize) -> ActiveSource {
        let memory = MemorySampleSource::new(vec![value; frames], 1, 48_000);
        let mut mappings = HashMap::new();
        mappings.insert("pad".to_string(), vec![1]);
        mappings.insert("click".to_string(), vec![2]);
        ActiveSource {
            id,
            source: Box::new(ChannelMappedSource::new(
                Box::new(memory),
                vec![vec![track.to_string()]],
                1,
            )),
            track_mappings: mappings,
            channel_mappings: Vec::new(),
            cached_source_channel_count: 0,
            is_finished: Arc::new(AtomicBool::new(false)),
            cancel_handle: CancelHandle::new(),
            start_at_sample: None,
            cancel_at_sample: Some(Arc::new(AtomicU64::new(0))),
            gain: Default::default(),
            gain_envelope: None,
        }
    }

    /// Renders `frames` frames of a two-channel mixer in device-sized
    /// buffers, as (pad, click) pairs.
    fn render(mixer: &AudioMixer, frames: usize) -> Vec<(f32, f32)> {
        let mut out = vec![0.0f32; frames * 2];
        for chunk in out.chunks_mut(256 * 2) {
            let n = chunk.len() / 2;
            mixer.process_into_output(chunk, n);
        }
        out.chunks(2).map(|f| (f[0], f[1])).collect()
    }

    #[test]
    fn passes_cross_over_the_overlap_and_cut_tracks_switch_at_its_midpoint() {
        let mixer = AudioMixer::new(2, 48_000);
        let crossover = PassCrossover {
            start_sample: 2_048,
            overlap_samples: 8_192,
            crossfade_samples: 480,
        };

        // The outgoing pass: 1.0 on the pad, 1.0 on the click, both running
        // well past the overlap. The incoming: 0.5 on each.
        mixer.add_source(source(1, "pad", 1.0, 20_000));
        mixer.add_source(source(2, "click", 1.0, 20_000));
        crossover.cross_out(&mixer, &[1], &[2]);
        for (id, track, group) in [(3, "pad", PassGroup::Fade), (4, "click", PassGroup::Cut)] {
            let mut incoming = source(id, track, 0.5, 20_000);
            let (envelope, starts_at) = crossover.cross_in(group);
            incoming.gain_envelope = Some(Arc::new(envelope));
            incoming.start_at_sample = Some(starts_at);
            mixer.add_source(incoming);
        }

        let out = render(&mixer, 14_000);
        let start = crossover.start_sample as usize;
        let end = start + crossover.overlap_samples as usize;
        let cut = crossover.cut_sample() as usize;

        // Before the boundary only the outgoing pass plays; after the overlap
        // only the incoming one.
        assert_eq!(out[start - 10], (1.0, 1.0));
        assert_eq!(out[end + 300], (0.5, 0.5));

        // Through the overlap the pad's two passes hold their power: the
        // outgoing at cos, the incoming at sin, gains summing in quadrature
        // to one. (The mixer steps gain once per buffer, so allow a step.)
        for i in (start..end).step_by(97) {
            let t = (i - start) as f32 / crossover.overlap_samples as f32;
            let (out_gain, in_gain) = CrossfadeCurve::EqualPower.gains(t);
            let expected = out_gain * 1.0 + in_gain * 0.5;
            assert!(
                (out[i].0 - expected).abs() < 0.05,
                "pad at {i}: {} vs {expected}",
                out[i].0
            );
        }
        // Halfway, both are well up: no dip to silence in the middle.
        assert!(out[cut].0 > 0.9, "{}", out[cut].0);

        // The click is one pass or the other, never both: the old one to the
        // midpoint, the new one after the short switch.
        assert_eq!(out[cut - 10].1, 1.0, "old click to the midpoint");
        assert_eq!(out[cut + 600].1, 0.5, "new click after it");
        assert!(out[start + 10].1 == 1.0 && out[end - 10].1 == 0.5);
    }

    #[test]
    fn a_break_fade_outlasting_its_pass_carries_on_where_it_got_to() {
        let s = Duration::from_secs_f64;
        // A 4 s fade from a break 1 s ago; the pass ends 1 s from now, half
        // way through the fade: the next pass starts at cos(pi/4) and fades
        // the last 2 s.
        let (gain, rest) =
            break_fade_continuation(s(11.0), s(10.0), s(1.0), s(4.0), CrossfadeCurve::EqualPower)
                .unwrap();
        assert!(
            (gain - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6,
            "{gain}"
        );
        assert!((rest.as_secs_f64() - 2.0).abs() < 1e-9, "{rest:?}");

        // A pass already over (a short last pass): from now -- on a linear
        // curve, a quarter of the way down, 1 s of the 4 gone.
        let (gain, rest) =
            break_fade_continuation(s(9.0), s(10.0), s(1.0), s(4.0), CrossfadeCurve::Linear)
                .unwrap();
        assert!((rest.as_secs_f64() - 3.0).abs() < 1e-9, "{rest:?}");
        assert!((gain - 0.75).abs() < 1e-6, "{gain}");

        // A pass that outlasts the fade, or ends in its last moments: none.
        assert!(
            break_fade_continuation(s(20.0), s(10.0), s(1.0), s(4.0), CrossfadeCurve::Linear)
                .is_none()
        );
        assert!(
            break_fade_continuation(s(12.98), s(10.0), s(1.0), s(4.0), CrossfadeCurve::Linear)
                .is_none()
        );
    }
}
