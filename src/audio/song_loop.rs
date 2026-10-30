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
    /// sample it starts at. Every source of the pass starts with it, from
    /// the top, so the cut tracks keep step with the fading ones however
    /// long the overlap turns out to be; a cut source stays silent until
    /// the midpoint.
    pub(crate) fn cross_in(&self, group: PassGroup) -> (GainEnvelope, u64) {
        match group {
            PassGroup::Fade => (
                GainEnvelope::fade_in(self.overlap_samples, CrossfadeCurve::EqualPower),
                self.start_sample,
            ),
            PassGroup::Cut => (
                GainEnvelope::fade_in(self.crossfade_samples, CrossfadeCurve::Linear)
                    .with_start_sample(self.cut_sample()),
                self.start_sample,
            ),
        }
    }
}

/// Whether a source playing `labels` (its channels' track names) plays only
/// tracks in `names`, and at least one: a source a fade's `exclude` leaves
/// out.
pub(crate) fn only_tracks_in(labels: &[Vec<String>], names: &[String]) -> bool {
    let mut tracks = labels.iter().flatten().peekable();
    tracks.peek().is_some() && tracks.all(|t| names.contains(t))
}

/// Gain knots for [`GainEnvelope::piecewise`]: (samples from the envelope's
/// start, gain).
pub(crate) type Knots = Vec<(u64, f32)>;

/// Where a loop break's fade-out (a playlist `fade_out`) stands, in mixer
/// samples, when the song's playback ends: for [`plan_break_carry`].
#[derive(Clone, Copy, Debug)]
pub(crate) struct BreakCarryTiming {
    pub now: u64,
    /// When the break came, and how long the fade lasts from there, on what
    /// curve. The fade runs from each source's gain at the break.
    pub break_sample: u64,
    pub fade: u64,
    pub curve: CrossfadeCurve,
    /// When the pass playing out ends, how long a pass is, and how long
    /// passes overlap (`loop_crossfade`; zero when they do not).
    pub pass_end: u64,
    pub pass_len: u64,
    pub overlap: u64,
    pub sample_rate: u32,
}

/// How a loop break's fade-out that outlasts the pass playing out is seen
/// out: by as many more passes as it takes, crossing pass to pass as the
/// song's own loop does. Gains are for a source at full level at the break;
/// scale them by where it was.
#[derive(Debug)]
pub(crate) struct BreakCarry {
    /// For the pass playing out, from `now`: the fade going on and, with an
    /// overlap, crossing out over its last. `None` when it just plays to its
    /// end.
    pub outgoing: Option<Knots>,
    /// The passes that carry the fade on: each one's start sample, and its
    /// knots from there.
    pub passes: Vec<(u64, Knots)>,
}

/// Ends `knots` at exactly zero, as a fade-out the mixer lets outlive its
/// playback must: at its last knot if it is all but there, or just after
/// it -- past the end of a pass that stops where the next takes over, so
/// the splice does not dip.
fn end_silent(knots: &mut Knots) {
    let Some(last) = knots.last_mut() else {
        return;
    };
    if last.1.abs() < 1e-4 {
        last.1 = 0.0;
    } else {
        let after = last.0 + 1;
        knots.push((after, 0.0));
    }
}

/// How a loop break's fade-out is carried past the pass playing out, or
/// `None` when that pass outlasts it (with a little to spare: a pass ending
/// in the fade's last moments is not worth a new one).
pub(crate) fn plan_break_carry(t: BreakCarryTiming) -> Option<BreakCarry> {
    /// A pass the fade would hardly be heard in is not started; nor are more
    /// than this many, however short the song.
    const SPARE_MS: u64 = 50;
    const MAX_PASSES: usize = 32;
    let spare = t.sample_rate as u64 * SPARE_MS / 1000;
    let step = (t.sample_rate as u64 / 50).max(1);
    let fade_end = t.break_sample + t.fade;
    if t.pass_end.max(t.now) + spare >= fade_end || t.pass_len == 0 {
        return None;
    }
    let fade = |at: u64| {
        let into = at.saturating_sub(t.break_sample) as f32 / t.fade.max(1) as f32;
        t.curve.gains(into).0
    };
    // Equal power across an overlap, as the loop's own crossfade.
    let ramp = |at: u64, from: u64, to: u64, up: bool| {
        if to <= from {
            return 1.0;
        }
        let (down, rise) =
            CrossfadeCurve::EqualPower.gains(at.saturating_sub(from) as f32 / (to - from) as f32);
        if up {
            rise
        } else {
            down
        }
    };
    let knots = |from: u64, to: u64, gain: &dyn Fn(u64) -> f32| -> Knots {
        let mut knots: Knots = (from..to)
            .step_by(step as usize)
            .map(|at| (at - from, gain(at)))
            .collect();
        knots.push((to - from, gain(to)));
        knots
    };

    let outgoing = (t.overlap > 0).then(|| {
        let cross = t.pass_end.saturating_sub(t.overlap).max(t.now);
        let mut knots = knots(t.now, t.pass_end, &|at| {
            fade(at) * ramp(at, cross, t.pass_end, false)
        });
        end_silent(&mut knots);
        knots
    });

    let mut passes = Vec::new();
    let mut prev_end = t.pass_end;
    while passes.len() < MAX_PASSES {
        let start = prev_end.saturating_sub(t.overlap).max(t.now);
        if start + spare >= fade_end {
            break;
        }
        let end = start + t.pass_len;
        let rise = (start + t.overlap).min(end);
        let to = end.min(fade_end);
        let ends_first = end < fade_end;
        let fall = if ends_first {
            end.saturating_sub(t.overlap).max(rise)
        } else {
            end
        };
        let mut knots = knots(start, to, &|at| {
            fade(at) * ramp(at, start, rise, true) * ramp(at, fall, end, false)
        });
        end_silent(&mut knots);
        passes.push((start, knots));
        prev_end = end;
    }
    Some(BreakCarry { outgoing, passes })
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
            break_envelope: None,
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
    fn a_shortened_overlap_keeps_the_cut_tracks_in_step() {
        // A pass begun inside the crossfade crosses less of it. Its cut
        // tracks still start with the pass, from the top, as its fading
        // tracks do, and only sound from the shorter overlap's midpoint.
        for overlap_samples in [8_192, 2_000, 0] {
            let crossover = PassCrossover {
                start_sample: 2_048,
                overlap_samples,
                crossfade_samples: 480,
            };
            let (fade_env, fade_start) = crossover.cross_in(PassGroup::Fade);
            let (cut_env, cut_start) = crossover.cross_in(PassGroup::Cut);
            assert_eq!(cut_start, fade_start, "overlap {overlap_samples}");
            assert_eq!(cut_start, crossover.start_sample);
            let _ = fade_env;

            let mixer = AudioMixer::new(2, 48_000);
            let mut incoming = source(1, "click", 1.0, 20_000);
            incoming.gain_envelope = Some(Arc::new(cut_env));
            incoming.start_at_sample = Some(cut_start);
            mixer.add_source(incoming);
            let out = render(&mixer, 14_000);
            let cut = crossover.cut_sample() as usize;
            if cut > crossover.start_sample as usize + 300 {
                assert_eq!(out[cut - 300].1, 0.0, "silent before the midpoint");
            }
            assert_eq!(out[cut + 1_000].1, 1.0, "sounding after it");
        }
    }

    /// The gain `knots` give `at` samples from their start.
    fn gain(knots: &Knots, at: u64) -> f32 {
        GainEnvelope::piecewise(knots.clone().into()).gain_at(at)
    }

    fn timing(
        break_sample: u64,
        fade: u64,
        pass_end: u64,
        pass_len: u64,
        overlap: u64,
    ) -> BreakCarryTiming {
        BreakCarryTiming {
            now: break_sample,
            break_sample,
            fade,
            curve: CrossfadeCurve::Linear,
            pass_end,
            pass_len,
            overlap,
            sample_rate: 1_000,
        }
    }

    #[test]
    fn a_break_fade_outlasting_its_pass_carries_on_where_it_got_to() {
        // A linear 4 s fade from a break 1 s before the pass ends, with no
        // overlap: the pass plays out, and the next starts where it ends, a
        // quarter of the way down, and fades the last 3 s.
        let carry = plan_break_carry(timing(10_000, 4_000, 11_000, 10_000, 0)).unwrap();
        assert!(carry.outgoing.is_none());
        assert_eq!(carry.passes.len(), 1);
        let (start, knots) = &carry.passes[0];
        assert_eq!(*start, 11_000);
        assert!((gain(knots, 0) - 0.75).abs() < 1e-3, "{}", gain(knots, 0));
        assert!((gain(knots, 1_500) - 0.375).abs() < 1e-3);
        assert_eq!(knots.last().unwrap(), &(3_000, 0.0));

        // A pass that outlasts the fade, or ends in its last moments: none.
        assert!(plan_break_carry(timing(10_000, 4_000, 20_000, 10_000, 0)).is_none());
        assert!(plan_break_carry(timing(10_000, 4_000, 13_980, 10_000, 0)).is_none());
    }

    #[test]
    fn a_long_fade_on_a_short_loop_takes_as_many_passes_as_it_needs() {
        // A 2 s loop, an 8 s fade, the break 1 s before the pass ends: passes
        // at 1, 3, 5 and 7 s in, each picking up where the last left off.
        let carry = plan_break_carry(timing(0, 8_000, 1_000, 2_000, 0)).unwrap();
        let starts: Vec<u64> = carry.passes.iter().map(|(s, _)| *s).collect();
        assert_eq!(starts, [1_000, 3_000, 5_000, 7_000]);
        for (start, knots) in &carry.passes {
            let expected = 1.0 - *start as f32 / 8_000.0;
            assert!((gain(knots, 0) - expected).abs() < 1e-3, "{start}");
        }
    }

    #[test]
    fn carried_passes_cross_over_the_loops_overlap() {
        // A 10 s song crossing passes over 2 s; a linear 10 s fade from a break
        // 5 s before the pass ends. The pass playing out crosses out over its
        // last 2 s while the next crosses in, holding the level as the loop
        // does, then the next fades the rest on its own.
        let carry = plan_break_carry(timing(0, 10_000, 5_000, 10_000, 2_000)).unwrap();
        let outgoing = carry.outgoing.unwrap();
        assert!(
            (gain(&outgoing, 2_000) - 0.8).abs() < 1e-3,
            "no cross until the overlap"
        );
        assert_eq!(outgoing.last().unwrap(), &(5_000, 0.0));
        assert_eq!(carry.passes.len(), 1);
        let (start, incoming) = &carry.passes[0];
        assert_eq!(*start, 3_000);
        assert!(gain(incoming, 0) < 1e-3, "crosses in from silence");
        for at in [3_000u64, 3_500, 4_000, 4_500] {
            let (out, inc) = (gain(&outgoing, at), gain(incoming, at - 3_000));
            let level = 1.0 - at as f32 / 10_000.0;
            assert!(
                ((out * out + inc * inc).sqrt() - level).abs() < 0.02,
                "power held at {at}: {out} {inc}"
            );
        }
        assert!((gain(incoming, 4_000) - 0.3).abs() < 1e-3);
    }
}
