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

//! Passes crossing out: a crossfading song loop's outgoing pass, and loop
//! breaks' fades ([`BreakFades`]).

use std::{
    collections::HashSet,
    time::{Duration, Instant},
};

use tracing::error;

use super::timeline::Fired;
use crate::lighting::{
    effects::{DmxCommand, EffectError},
    timeline::LightingTimeline,
    EffectEngine,
};

/// The wall clock loop breaks' fades run on: [`Instant::now`], except on a
/// test's thread that has set its own ([`TestClock`]) and moves it on by
/// hand, so a fade's every frame is at a time the test chose (#654).
pub(super) fn wall_now() -> Instant {
    #[cfg(test)]
    if let Some(now) = TEST_CLOCK.with(std::cell::Cell::get) {
        return now;
    }
    Instant::now()
}

#[cfg(test)]
thread_local! {
    static TEST_CLOCK: std::cell::Cell<Option<Instant>> = const { std::cell::Cell::new(None) };
}

/// A test's own wall clock for loop breaks' fades, on this thread, until it
/// is dropped: it stands still between [`TestClock::advance`]s.
#[cfg(test)]
pub(crate) struct TestClock;

#[cfg(test)]
impl TestClock {
    pub(crate) fn start() -> TestClock {
        TEST_CLOCK.with(|clock| clock.set(Some(Instant::now())));
        TestClock
    }

    pub(crate) fn advance(&self, by: Duration) {
        TEST_CLOCK.with(|clock| clock.set(clock.get().map(|now| now + by)));
    }
}

#[cfg(test)]
impl Drop for TestClock {
    fn drop(&mut self) {
        TEST_CLOCK.with(|clock| clock.set(None));
    }
}

/// The pass of a crossfading song loop that the lights are fading out of: the
/// effects engine and timeline the last pass ran in, moved aside whole when
/// the next pass began, so it plays on exactly as it was -- its remaining
/// cues included -- while the two frames cross. A loop break's fade is one
/// too, crossing out over whatever comes after it.
pub(crate) struct OutgoingPass {
    pub(super) effect_engine: EffectEngine,
    /// The frame this pass last rendered.
    pub(super) last: Vec<DmxCommand>,
    state: PassState,
}

/// Where an [`OutgoingPass`] is.
enum PassState {
    /// Crossing out, its engine and timeline playing on.
    Crossing(Box<Crossing>),
    /// Over and held: the pass renders its last frame and nothing more --
    /// its engine and timeline stopped -- for what it holds that nothing
    /// under it drives (a mover's position), until the fades older than it
    /// go. All the way out.
    Held,
}

/// A pass crossing out.
struct Crossing {
    timeline: Option<LightingTimeline>,
    /// What the pass's time and the crossfade's progress run on.
    clock: PassClock,
    /// How long the crossfade lasts.
    overlap: Duration,
    /// A loop break's fade cut short by a stop (#572).
    hurry: Option<Hurry>,
    /// A loop break's fade begun during a crossfade between passes: that
    /// crossfade's older pass, still crossing into this one's engine on this
    /// one's time, as it was before the break (#560). It goes when its own
    /// crossfade does.
    inner: Option<Box<OutgoingPass>>,
    /// Channels the rig holds lit that no engine writes any more, from
    /// effects that ended: rendered where this pass's engine does not drive
    /// them, so they fade out with it rather than staying on (#573).
    floor: Vec<DmxCommand>,
}

/// A fade cut short: from `from` of the way through it, at `at`, it goes
/// the rest of the way over `over`.
#[derive(Clone, Copy)]
struct Hurry {
    at: Instant,
    from: f64,
    over: Duration,
}

/// What an [`OutgoingPass`] keeps time by.
pub(crate) enum PassClock {
    /// The incoming pass's song time: a crossfade between passes of one
    /// looping song, the outgoing pass `lead` (a pass's length less the
    /// overlap) ahead. The crossfade begins at the incoming pass's `start`:
    /// 0, or part way in for a pass begun late, which joins the song's grid
    /// where one begun on time would be by then.
    Song { lead: Duration, start: Duration },
    /// The wall clock from a loop break (a playlist `fade_out`): the looping song
    /// fades out over the next song's start, whose song time is not running
    /// yet and has nothing to do with it. The outgoing pass plays on from
    /// `from`, its song time at the break.
    Wall { started: Instant, from: Duration },
}

/// The crossfade `t` of the way (0.0 to 1.0) from `older`'s frame, shown
/// as `older_engine` shows it, to `newer`'s just rendered, `frame`: the
/// frame that goes out, channel by channel as a console crosses cues (see
/// [`EffectEngine::crossfade_commands`]). What goes out is the blend, so
/// that is what the rig shows too -- the web UI and `get_fixture_state`
/// (#555) -- left on `newer`. The one blend every layer of light crosses
/// by: a loop pass into the next, the next into a break's fade, ambient
/// over all of it.
pub(super) fn cross(
    older: &[DmxCommand],
    older_engine: &EffectEngine,
    newer: &mut EffectEngine,
    frame: &[DmxCommand],
    t: f64,
) -> Vec<DmxCommand> {
    let blended = newer.crossfade_commands(older, frame, t);
    let shown = newer.crossfade_states_onto(
        older_engine,
        (newer.shown_fixture_states(), newer.shown_poses()),
        t,
    );
    newer.set_shown(Some(shown));
    blended
}

impl Crossing {
    /// How far through the crossfade it is (0.0 to 1.0 and beyond), and the
    /// pass's own song time, given the incoming pass's.
    fn timing(&self, song_time: Duration) -> (f64, Duration) {
        let (into, time) = match self.clock {
            PassClock::Song { lead, start } => (song_time.saturating_sub(start), song_time + lead),
            PassClock::Wall { started, from } => {
                let elapsed = wall_now().saturating_duration_since(started);
                (elapsed, from + elapsed)
            }
        };
        let t = match self.hurry {
            Some(Hurry { at, from, over }) => {
                let progress = crate::section_loop::crossfade_progress(
                    wall_now().saturating_duration_since(at),
                    over,
                );
                crate::lighting::channels::lerp(from, 1.0, progress)
            }
            None => crate::section_loop::crossfade_progress(into, self.overlap),
        };
        (t, time)
    }
}

impl OutgoingPass {
    /// A pass crossing out of `effect_engine` and `timeline` (its hold let
    /// go: a start held it while its state went in, and whatever would have
    /// let it go now finds the next song's timeline in its place), over
    /// `overlap` on `clock`, from the frame the engine last rendered.
    pub(super) fn crossing(
        effect_engine: EffectEngine,
        timeline: Option<LightingTimeline>,
        clock: PassClock,
        overlap: Duration,
        inner: Option<OutgoingPass>,
        floor: Vec<DmxCommand>,
    ) -> OutgoingPass {
        let last = effect_engine.last_commands().to_vec();
        let timeline = timeline.map(|mut timeline| {
            timeline.hold(false);
            timeline
        });
        OutgoingPass {
            effect_engine,
            last,
            state: PassState::Crossing(Box::new(Crossing {
                timeline,
                clock,
                overlap,
                hurry: None,
                inner: inner.map(Box::new),
                floor,
            })),
        }
    }

    /// Whether this pass is over and held.
    pub(super) fn is_held(&self) -> bool {
        matches!(self.state, PassState::Held)
    }

    /// The pass crossing into this one, if one is.
    #[cfg(test)]
    pub(super) fn inner(&self) -> Option<&OutgoingPass> {
        match &self.state {
            PassState::Crossing(crossing) => crossing.inner.as_deref(),
            PassState::Held => None,
        }
    }

    /// How far through its crossfade this pass is, given the incoming
    /// pass's song time: 0.0 to 1.0 and beyond, a held pass all the way.
    fn progress(&self, song_time: Duration) -> f64 {
        match &self.state {
            PassState::Crossing(crossing) => crossing.timing(song_time).0,
            PassState::Held => 1.0,
        }
    }

    /// Holds this pass if its crossfade, `t` of the way, is over: it fires
    /// no more cues -- one due in the very frame it ends cannot move what it
    /// holds (a mover's position) -- and renders its last frame from now on.
    fn hold_if_over(&mut self, t: f64) {
        if t >= 1.0 {
            self.hold();
        }
    }

    /// Stops this pass where it is: it renders its last frame from now on.
    fn hold(&mut self) {
        self.state = PassState::Held;
    }

    /// Renders this pass's frame, the incoming pass at `song_time`: its
    /// engine's, at its own song time, crossed with an inner pass still
    /// crossing into it, as before a break that interrupted their
    /// crossfade. A held pass renders its last frame. Returns how far
    /// through its crossfade it is, and the frame.
    pub(super) fn render(
        &mut self,
        dt: Duration,
        song_time: Duration,
    ) -> Result<(f64, Vec<DmxCommand>), EffectError> {
        let PassState::Crossing(crossing) = &mut self.state else {
            return Ok((1.0, self.last.clone()));
        };
        let (t, time) = crossing.timing(song_time);
        let mut own = self.effect_engine.update(dt, Some(time))?.to_vec();
        if !crossing.floor.is_empty() {
            let driven: HashSet<(u16, u16)> = own.iter().map(|c| (c.universe, c.channel)).collect();
            own.extend(
                crossing
                    .floor
                    .iter()
                    .filter(|c| !driven.contains(&(c.universe, c.channel)))
                    .cloned(),
            );
        }
        let frame = match crossing.inner.as_mut() {
            None => own,
            Some(inner) => {
                let (inner_t, old) = inner.render(dt, time)?;
                let blended = cross(
                    &old,
                    &inner.effect_engine,
                    &mut self.effect_engine,
                    &own,
                    inner_t,
                );
                // An inner pass whose crossfade is over stays, held, until
                // this one goes: at its end it holds only what this one does
                // not drive -- a mover's position -- which would jump as it
                // went.
                inner.hold_if_over(inner_t);
                blended
            }
        };
        self.last.clone_from(&frame);
        Ok((t, frame))
    }

    /// Advances this pass's timeline to its own song time, the incoming
    /// pass at `song_time`, and each inner pass's to its own, collecting
    /// what they fire.
    pub(super) fn advance_timelines(&mut self, song_time: Duration, fired: &mut Vec<Fired>) {
        let PassState::Crossing(crossing) = &mut self.state else {
            return;
        };
        let (_, time) = crossing.timing(song_time);
        if let Some(timeline) = crossing.timeline.as_mut() {
            fired.push(Fired::new(timeline.epoch(), timeline.update(time)));
        }
        if let Some(inner) = crossing.inner.as_mut() {
            inner.advance_timelines(time, fired);
        }
    }

    /// The engine of the pass, this one or an inner one, whose timeline is
    /// armed as `epoch`.
    pub(super) fn engine_of(&mut self, epoch: u64) -> Option<&mut EffectEngine> {
        let PassState::Crossing(crossing) = &mut self.state else {
            return None;
        };
        if crossing.timeline.as_ref().map(|t| t.epoch()) == Some(epoch) {
            return Some(&mut self.effect_engine);
        }
        crossing
            .inner
            .as_mut()
            .and_then(|inner| inner.engine_of(epoch))
    }

    /// Moves this pass off the incoming pass's song time onto the wall clock,
    /// from where it is at the incoming `song_time`: the incoming pass is
    /// about to go back (a section's jump) and would take this one with it.
    pub(super) fn keep_own_time(&mut self, song_time: Duration) {
        let PassState::Crossing(crossing) = &mut self.state else {
            return;
        };
        if let PassClock::Song { lead, start } = crossing.clock {
            let into = song_time.saturating_sub(start);
            let now = wall_now();
            let started = now.checked_sub(into).unwrap_or(now);
            crossing.clock = PassClock::Wall {
                started,
                from: (song_time + lead).saturating_sub(now - started),
            };
        }
    }

    /// Takes `newer`'s held frame into this one's: where both drive a
    /// channel, the newer, as crossing the two in turn would give.
    fn merge_held(&mut self, newer: OutgoingPass) {
        let theirs: HashSet<(u16, u16)> =
            newer.last.iter().map(|c| (c.universe, c.channel)).collect();
        self.last
            .retain(|c| !theirs.contains(&(c.universe, c.channel)));
        self.last.extend(newer.last);
        // What the rig shows likewise: the newer pass's over this one's.
        let shown = self.effect_engine.crossfade_states_onto(
            &self.effect_engine,
            (
                newer.effect_engine.shown_fixture_states(),
                newer.effect_engine.shown_poses(),
            ),
            1.0,
        );
        self.effect_engine.set_shown(Some(shown));
    }

    /// Hurries this pass to the end of its crossfade over `fade` from now,
    /// if that is sooner: from where it is, so nothing jumps.
    fn hurry_over(&mut self, fade: Duration) {
        let PassState::Crossing(crossing) = &mut self.state else {
            return;
        };
        let (t, _) = crossing.timing(Duration::ZERO);
        if t >= 1.0 {
            return;
        }
        let left = match crossing.hurry {
            Some(Hurry { at, over, .. }) => {
                over.saturating_sub(wall_now().saturating_duration_since(at))
            }
            None => crossing.overlap.mul_f64(1.0 - t),
        };
        if fade < left {
            crossing.hurry = Some(Hurry {
                at: wall_now(),
                from: t,
                over: fade,
            });
        }
    }
}

/// Loop breaks' fades still going out, oldest first: each crosses out over
/// everything after it at its own length, so nothing that comes later --
/// another pass's crossfade, another break, a stop's dark -- cuts it short
/// or drops it.
#[derive(Default)]
pub(crate) struct BreakFades(Vec<OutgoingPass>);

impl std::ops::Deref for BreakFades {
    type Target = [OutgoingPass];

    fn deref(&self) -> &[OutgoingPass] {
        &self.0
    }
}

impl BreakFades {
    /// Ends every fade at once.
    #[cfg(test)]
    pub(super) fn clear(&mut self) {
        self.0.clear();
    }

    /// Begins a loop break's fade, over all those already going.
    pub(super) fn begin(&mut self, fade: OutgoingPass) {
        self.0.push(fade);
    }

    /// The fades, oldest first.
    pub(super) fn iter_mut(&mut self) -> impl Iterator<Item = &mut OutgoingPass> {
        self.0.iter_mut()
    }

    /// Hurries every fade out, the rest of the way over `fade` from now --
    /// sooner, never later, than it would have (see
    /// [`super::Engine::hurry_break_fade`]).
    pub(super) fn hurry(&mut self, fade: Duration) {
        for pass in &mut self.0 {
            pass.hurry_over(fade);
        }
    }

    /// A crossfade pass going -- its crossfade over, the next pass's begun,
    /// or its playback stopped -- while these fades cross over it: it joins
    /// them, held at its last frame, so what it holds that nothing under it
    /// drives (a mover's position) stays until every older fade has gone,
    /// rather than jumping under them. Its engine and timeline stop. One
    /// held right after another merges into it, so a song looping under a
    /// long fade adds one, not one a pass. With no fade over it, it just
    /// goes.
    pub(super) fn retire(&mut self, mut pass: OutgoingPass) {
        if self.0.is_empty() {
            return;
        }
        pass.hold();
        match self.0.last_mut() {
            Some(newest) if newest.is_held() => newest.merge_held(pass),
            _ => self.0.push(pass),
        }
    }

    /// Holds each fade that is over, before its cues are looked at or once
    /// it is kept for an older one: its song's show has faded out, and must
    /// not go on firing cues under it.
    pub(super) fn sweep(&mut self, song_time: Duration) {
        for fade in &mut self.0 {
            let t = fade.progress(song_time);
            fade.hold_if_over(t);
        }
    }

    /// Renders the fades over `under`, the frame below them, each at its
    /// own length: the newest first, so the oldest crosses outermost, with
    /// what the rig shows left on `main`. One that is over goes only once
    /// every older one has: what it holds of a mover's position is what the
    /// older ones cross onto. One that fails to render goes at once, rather
    /// than stopping every frame after it.
    pub(super) fn render_over(
        &mut self,
        main: &mut EffectEngine,
        dt: Duration,
        song_time: Duration,
        under: Vec<DmxCommand>,
    ) -> Vec<DmxCommand> {
        let mut blended = under;
        // Newest first: (over, failed) for each.
        let mut ends = Vec::with_capacity(self.0.len());
        for fade in self.0.iter_mut().rev() {
            let (t, old) = match fade.render(dt, song_time) {
                Ok(rendered) => rendered,
                Err(e) => {
                    error!("A loop break's fade failed to render, and goes: {e}");
                    ends.push((true, true));
                    continue;
                }
            };
            blended = cross(&old, &fade.effect_engine, main, &blended, t);
            ends.push((t >= 1.0, false));
        }
        // Oldest first: the over ones until the first still going, and any
        // that failed.
        let mut still_older = false;
        let keep: Vec<bool> = ends
            .into_iter()
            .rev()
            .map(|(over, failed)| {
                let keep = !failed && (still_older || !over);
                still_older |= keep;
                keep
            })
            .collect();
        let mut keep = keep.into_iter();
        self.0.retain(|_| keep.next().unwrap_or(true));
        self.sweep(song_time);
        blended
    }
}
