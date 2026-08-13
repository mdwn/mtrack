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
//! Liveness facts recorded by the audio output callback.
//!
//! A stream that opened successfully tells you the device accepted the format. It
//! does not tell you the callback is still running, and it certainly does not tell
//! you sound is coming out. This records the two things mtrack can actually know:
//!
//! - the callback is being invoked, and
//! - the samples being handed to the device are not silence.
//!
//! Neither proves audio reached the room — a device can accept every buffer and
//! produce nothing, which no amount of host-side instrumentation can detect. What
//! these facts buy is the other half of the diagnosis: they let mtrack be ruled out
//! in seconds instead of guessed at.
//!
//! This is deliberately *not* metering. Nothing here reports a level, because a
//! level sampled by a status poller is a bad meter (a poll sees one callback in
//! several hundred) and a good meter is a different feature with a different
//! reader. All that is asked of the audio here is a yes/no above the silence
//! floor, which is cheaper than a peak and answers the health question exactly.
//!
//! Everything is written from the realtime audio callback, so it holds no locks,
//! allocates nothing, and reads the clock at most once per callback.
//!
//! Nothing here is backend-specific: it lives beside the `Device` trait rather
//! than inside the cpal implementation so a second backend — or a second
//! interface, once multi-interface output lands — reports health the same way.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Absolute sample value at or below which a buffer counts as silence.
///
/// Chosen just above the quantisation floor of 16-bit audio (1/32768 ≈ 3.05e-5) so
/// dither or a stray LSB doesn't read as signal, while anything genuinely audible
/// does.
const SILENCE_FLOOR: f32 = 1.0e-4;

/// How long the callback may be silent before it counts as stopped.
///
/// Not a jitter margin — a consequence threshold. Once a full second has passed
/// with nothing handed to the device, the moment is gone whether or not the
/// device was really dead, so a "false positive" here is not meaningfully false:
/// the song it ends was already ruined by the second of silence that triggered
/// it. That is what makes one second a principled number rather than an
/// arbitrary one, and also why it should not be much larger — past this point
/// waiting only extends the silence without learning anything.
///
/// The error bars are deliberately lopsided. Concluding too late costs a little
/// more of an already-silent moment; concluding too early kills a healthy song
/// in front of an audience. Given that asymmetry, this sits at the generous end.
pub const LIVENESS_WINDOW: Duration = Duration::from_secs(1);

/// How many consecutive callback periods must be missed before the floor is
/// overridden. Only reachable when a single period approaches a second on its
/// own — see [`liveness_window`].
const MISSED_CALLBACKS: u32 = 4;

/// The liveness window for a device whose callback period is known.
///
/// [`LIVENESS_WINDOW`] on every configuration anyone actually runs: at the
/// default 1024 frames / 44.1kHz a period is 23ms, so four of them is 92ms and
/// the floor wins by an order of magnitude.
///
/// The derived term exists only for absurd buffers. `buffer_size` has no upper
/// bound, and at 65536 frames a period is ~1.5s — longer than the floor, so a
/// perfectly healthy callback would look stopped between every buffer and every
/// song would be ended a second after it started. Scaling to "four consecutive
/// missed buffers" keeps the check meaningful there instead of making playback
/// impossible.
///
/// `None` means the period is unknown, which is what `BufferSize::Default`
/// leaves us with — the backend picked, and did not say. The floor is the only
/// safe answer.
pub fn liveness_window(callback_period: Option<Duration>) -> Duration {
    match callback_period {
        Some(period) => LIVENESS_WINDOW.max(period * MISSED_CALLBACKS),
        None => LIVENESS_WINDOW,
    }
}

/// The time one callback covers, for a buffer size in frames and a sample rate.
///
/// `None` when either is unknown or zero, rather than guessing.
pub fn callback_period(buffer_frames: Option<u32>, sample_rate: u32) -> Option<Duration> {
    let frames = buffer_frames?;
    if frames == 0 || sample_rate == 0 {
        return None;
    }
    Some(Duration::from_secs_f64(
        f64::from(frames) / f64::from(sample_rate),
    ))
}

/// How recently non-silent audio must have been written to count as signalling.
///
/// Wider than [`LIVENESS_WINDOW`] because real music has quiet passages: a fade,
/// a break, or a soft intro should not read as "gone silent" the moment it dips
/// below the floor.
pub const SIGNAL_WINDOW: Duration = Duration::from_secs(2);

/// Lock-free liveness signals written by the output callback.
pub struct OutputHealth {
    /// Reference point for the nanosecond timestamps below.
    base: Instant,
    /// Nanos since `base` at the last callback invocation. 0 means "never called".
    last_callback_nanos: AtomicU64,
    /// Nanos since `base` at the last callback whose output exceeded the silence
    /// floor. 0 means "no signal has ever been written".
    last_signal_nanos: AtomicU64,
    /// Total callbacks served, for diagnostics.
    callbacks: AtomicU64,
}

impl Default for OutputHealth {
    fn default() -> Self {
        Self::new()
    }
}

impl OutputHealth {
    pub fn new() -> Self {
        Self {
            base: Instant::now(),
            last_callback_nanos: AtomicU64::new(0),
            last_signal_nanos: AtomicU64::new(0),
            callbacks: AtomicU64::new(0),
        }
    }

    /// Record that the callback ran, and whether it handed the device anything
    /// above the silence floor. Called from the realtime thread.
    pub fn record_callback(&self, has_signal: bool) {
        // Saturates after ~584 years of uptime, which is not a scenario worth code.
        let now = self.base.elapsed().as_nanos() as u64;
        // Never store 0 for a real callback — 0 is the "never" sentinel, and it is
        // reachable if a callback lands in the first nanosecond of the process.
        let now = now.max(1);

        self.callbacks.fetch_add(1, Ordering::Relaxed);
        self.last_callback_nanos.store(now, Ordering::Relaxed);
        if has_signal {
            self.last_signal_nanos.store(now, Ordering::Relaxed);
        }
    }

    /// Read the current facts.
    pub fn snapshot(&self) -> OutputHealthSnapshot {
        let now = self.base.elapsed().as_nanos() as u64;
        let age = |stamp: u64| {
            if stamp == 0 {
                None
            } else {
                Some(Duration::from_nanos(now.saturating_sub(stamp)))
            }
        };

        OutputHealthSnapshot {
            since_last_callback: age(self.last_callback_nanos.load(Ordering::Relaxed)),
            since_last_signal: age(self.last_signal_nanos.load(Ordering::Relaxed)),
            callbacks: self.callbacks.load(Ordering::Relaxed),
        }
    }
}

/// A point-in-time read of [`OutputHealth`].
///
/// Deliberately facts rather than a verdict. "Silent" is correct and expected when
/// nothing is playing, so whether silence is a problem can only be decided where
/// playback state is known — see the stall check in the playback monitor loop,
/// which is the one place that knows audio is supposed to be coming out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutputHealthSnapshot {
    /// Time since the callback last ran, or `None` if it never has.
    pub since_last_callback: Option<Duration>,
    /// Time since non-silent audio was last written, or `None` if it never was.
    pub since_last_signal: Option<Duration>,
    /// Total callbacks served since the device was opened.
    pub callbacks: u64,
}

impl OutputHealthSnapshot {
    /// Whether the callback has run within `threshold`.
    pub fn callback_alive(&self, threshold: Duration) -> bool {
        matches!(self.since_last_callback, Some(age) if age <= threshold)
    }

    /// Whether non-silent audio was written within `threshold`.
    ///
    /// False is normal whenever nothing is playing, and can also go false during
    /// a genuinely quiet passage or with output gains at zero. Callers that treat
    /// this as a fault must know playback state.
    pub fn writing_signal(&self, threshold: Duration) -> bool {
        matches!(self.since_last_signal, Some(age) if age <= threshold)
    }
}

/// Whether a buffer contains anything above the silence floor.
///
/// Runs in the audio callback. `any` short-circuits, so a buffer with audio in it
/// costs a few samples and only true silence costs a full pass — cheaper than
/// computing a peak, which is the other reason not to report a level from here.
///
/// NaN compares false against every threshold, so a corrupt sample can never latch
/// "signal" permanently on.
#[inline]
pub fn has_output_signal(data: &[f32]) -> bool {
    data.iter().any(|sample| sample.abs() > SILENCE_FLOOR)
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn default_buffer_uses_the_floor() {
        // 1024 frames at 44.1kHz is a 23ms period; four of those is 92ms, so the
        // floor wins. A 92ms window would read an ordinary scheduling hiccup on a
        // loaded machine as a dead device and end the song for it.
        let period = callback_period(Some(1024), 44_100).unwrap();
        assert!(period < Duration::from_millis(25));
        assert_eq!(liveness_window(Some(period)), LIVENESS_WINDOW);
    }

    #[test]
    fn an_absurd_buffer_scales_past_the_floor() {
        // 65536 frames at 44.1kHz is ~1.49s — longer than the floor on its own,
        // so without scaling a healthy callback would look stopped between every
        // buffer and no song could ever play.
        let period = callback_period(Some(65_536), 44_100).unwrap();
        assert!(period > LIVENESS_WINDOW);
        assert!(liveness_window(Some(period)) > period * 2);
    }

    #[test]
    fn unknown_period_uses_the_floor() {
        // BufferSize::Default: the backend chose and didn't say.
        assert_eq!(callback_period(None, 44_100), None);
        assert_eq!(liveness_window(None), LIVENESS_WINDOW);
    }

    #[test]
    fn nonsense_inputs_do_not_produce_a_period() {
        assert_eq!(callback_period(Some(0), 44_100), None);
        assert_eq!(callback_period(Some(1024), 0), None);
    }

    #[test]
    fn empty_buffer_has_no_signal() {
        assert!(!has_output_signal(&[]));
    }

    #[test]
    fn negative_extreme_counts_as_signal() {
        assert!(has_output_signal(&[0.0, -0.8, 0.0]));
    }

    #[test]
    fn nan_is_not_signal() {
        // A corrupt sample must not latch the signal timestamp on forever.
        assert!(!has_output_signal(&[f32::NAN, f32::NAN]));
        assert!(has_output_signal(&[f32::NAN, 0.5]));
    }

    #[test]
    fn dither_level_output_is_not_signal() {
        // One LSB of 16-bit audio, below the floor.
        assert!(!has_output_signal(&[1.0 / 32768.0; 8]));
    }

    #[test]
    fn fresh_health_reports_nothing_seen() {
        let snap = OutputHealth::new().snapshot();
        assert_eq!(snap.since_last_callback, None);
        assert_eq!(snap.since_last_signal, None);
        assert_eq!(snap.callbacks, 0);
        assert!(!snap.callback_alive(LIVENESS_WINDOW));
        assert!(!snap.writing_signal(SIGNAL_WINDOW));
    }

    #[test]
    fn silent_callback_is_alive_but_not_signalling() {
        let health = OutputHealth::new();
        health.record_callback(false);

        let snap = health.snapshot();
        assert_eq!(snap.callbacks, 1);
        assert!(
            snap.callback_alive(LIVENESS_WINDOW),
            "a callback that ran is alive even when it wrote silence"
        );
        assert!(!snap.writing_signal(SIGNAL_WINDOW));
    }

    #[test]
    fn signal_registers() {
        let health = OutputHealth::new();
        health.record_callback(true);

        assert!(health.snapshot().writing_signal(SIGNAL_WINDOW));
    }

    #[test]
    fn signal_age_persists_after_going_quiet() {
        let health = OutputHealth::new();
        health.record_callback(true);
        health.record_callback(false);

        let snap = health.snapshot();
        assert!(
            snap.writing_signal(SIGNAL_WINDOW),
            "signal age should remember the earlier non-silent buffer"
        );
        assert_eq!(snap.callbacks, 2);
    }

    #[test]
    fn a_stale_callback_reads_as_not_alive() {
        let health = OutputHealth::new();
        health.record_callback(true);
        std::thread::sleep(Duration::from_millis(20));

        let snap = health.snapshot();
        assert!(
            !snap.callback_alive(Duration::from_millis(1)),
            "a callback 20ms old must not pass a 1ms liveness threshold"
        );
        assert!(snap.callback_alive(LIVENESS_WINDOW));
    }
}
