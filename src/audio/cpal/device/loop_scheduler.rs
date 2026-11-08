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

//! The audio's loop scheduling: building each section's (or pass's) sources
//! ahead of its jump, and scheduling the jump on the mixer's samples as it
//! comes, for the monitor loop in [`Device::play_from`] to poll.

use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    thread,
    time::Duration,
};

use tracing::{debug, error, info, warn};

use super::{
    build_active_source, build_pass_sources, Device, GroupedSources, SongLoopCrossfade,
    PASS_PREBUILD_AHEAD,
};
use crate::audio::song_loop::{PassCrossover, PassGroup};
use crate::playsync::{CancelHandle, SectionJumpLog};
use crate::section_loop::{EffectiveSection, SectionJump, SectionLoopTrigger};
use crate::songs::Song;

/// What committing to a jump came to (see [`commit_jump`]).
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Commit<T> {
    /// Scheduled, and pushed to the log for the followers.
    Made(SectionJump, T),
    /// Nothing was ready to play at it: taken back, to go round as soon as
    /// it is, from where playback is then.
    NotReady,
    /// The playback was cancelled, or its loop broken, first: taken back.
    Stopped,
}

/// Commits to `jump`, which `trigger` has just given, the one way every jump
/// is: under the playback's jump hold (a loop break fades and breaks between
/// jumps, never between scheduling one and committing it), and only if the
/// playback is neither cancelled nor broken by then, `schedule` schedules
/// it; if it does, the jump goes to `log` (when the audio commits the
/// playback's jumps), and otherwise the trigger takes it back. Every path
/// that does not commit takes the jump back.
pub(super) fn commit_jump<T>(
    trigger: &mut SectionLoopTrigger,
    cancel_handle: &CancelHandle,
    loop_break: &AtomicBool,
    log: Option<&SectionJumpLog>,
    jump: SectionJump,
    schedule: impl FnOnce(&SectionJump) -> Option<T>,
) -> Commit<T> {
    let _jumps = cancel_handle.hold_jumps();
    if cancel_handle.is_cancelled() || loop_break.load(Ordering::Relaxed) {
        trigger.retract(&jump);
        return Commit::Stopped;
    }
    match schedule(&jump) {
        Some(made) => {
            // The position counts the jump from the log as it falls due, so
            // it does not wrap before the listener crosses the boundary.
            if let Some(log) = log {
                log.push(jump.clone());
            }
            Commit::Made(jump, made)
        }
        None => {
            trigger.retract(&jump);
            Commit::NotReady
        }
    }
}

/// A jump the audio has scheduled and committed to.
pub(super) struct ScheduledJump {
    pub jump: SectionJump,
    /// The mixer sample the sources it jumps to start at.
    pub start_sample: u64,
}

/// What a poll of the [`LoopScheduler`] came to.
pub(super) enum Poll {
    /// A jump was scheduled: poll again at once.
    Scheduled(ScheduledJump),
    /// Nothing to do yet: poll again a moment on.
    Idle,
    /// The playback was cancelled or its loop broken while a jump was being
    /// committed to: stop polling.
    Stop,
}

/// What the loop scheduling needs from the playback, fixed for its length.
pub(super) struct LoopContext<'a> {
    pub device: &'a Device,
    pub song: &'a Arc<Song>,
    pub playback_context: &'a crate::audio::PlaybackContext,
    pub mappings: &'a HashMap<String, Vec<u16>>,
    pub cancel_handle: &'a CancelHandle,
    pub loop_break: &'a AtomicBool,
    /// The log the jumps go to, if the audio commits the playback's jumps.
    pub section_jumps: Option<&'a SectionJumpLog>,
    /// Look-ahead for the trigger. Must exceed the 10ms poll so the trigger
    /// is always detected before the ideal loop boundary; the jump is then
    /// scheduled at its exact sample, which keeps polling jitter out of the
    /// audible loop point.
    pub lookahead: Duration,
    /// The startup skew between the clock and the audio (see
    /// `audio_clock_skew_samples`).
    pub skew_samples: u64,
    /// The short crossfade a section's jump (and a cut track) switches over.
    pub crossfade_samples: u64,
    /// The tracks a pass switches over at its crossfade's midpoint: every
    /// build of the song's sources keeps them apart.
    pub cut_tracks: Vec<String>,
}

/// The sources the loop scheduling keeps: those ahead of the next jump, and
/// those playing.
struct LoopSources {
    /// Sources pre-built for the next loop iteration, keyed by section name
    /// so a section change invalidates them. Ready to swap in with zero I/O.
    /// Built in the pass groups (see `cut_tracks`).
    pending: Option<(String, GroupedSources)>,
    /// In-flight background build (section name + result channel). The build
    /// runs on a detached worker thread rather than inline because
    /// `create_channel_mapped_sources_from` blocks on BufferedSampleSource
    /// warmup (file open/seek/decode). Doing that on the monitor thread
    /// could delay the *first* loop boundary -- the loop can be armed close
    /// to the section end and the first build hits a cold file cache --
    /// which showed up as a one-off stutter on the first iteration.
    build_rx: Option<(String, mpsc::Receiver<Result<GroupedSources, String>>)>,
    /// A crossfading song's passes (`loop_crossfade`).
    song_loop: Option<SongLoopCrossfade>,
    /// The finish flags of the sources playing now.
    finish_flags: Vec<Arc<AtomicBool>>,
    /// DIAGNOSTIC: per-swap counter to compare the first loop against later
    /// ones.
    loop_iteration: u64,
}

/// A playback's loop scheduling: the trigger the jumps come from, the
/// sources built ahead of them, a crossfading song's passes, and where the
/// pass playing ends. [`LoopScheduler::poll`] builds ahead and schedules the
/// next jump as it comes, committing to it with [`commit_jump`].
pub(super) struct LoopScheduler<'a> {
    ctx: LoopContext<'a>,
    trigger: SectionLoopTrigger,
    sources: LoopSources,
    /// Where the pass playing ends on the playback timeline (the song's end
    /// plus what the jumps have put back): where a loop break's fade would
    /// need carrying on into another pass.
    pass_end: Duration,
}

impl<'a> LoopScheduler<'a> {
    /// Scheduling for a playback whose first sources (their finish flags
    /// `finish_flags`, a crossfading song's grouped in `song_loop`) are
    /// playing.
    pub fn new(
        ctx: LoopContext<'a>,
        song_loop: Option<SongLoopCrossfade>,
        finish_flags: Vec<Arc<AtomicBool>>,
    ) -> Self {
        let pass_end = ctx.song.duration();
        LoopScheduler {
            ctx,
            trigger: SectionLoopTrigger::new(),
            sources: LoopSources {
                pending: None,
                build_rx: None,
                song_loop,
                finish_flags,
                loop_iteration: 0,
            },
            pass_end,
        }
    }

    /// Whether every source playing has finished (EOF).
    pub fn all_finished(&self) -> bool {
        self.sources
            .finish_flags
            .iter()
            .all(|flag| flag.load(Ordering::Relaxed))
    }

    /// What the playback ends with: a crossfading song's passes, and where
    /// the pass playing ends on the playback timeline.
    pub fn finish(self) -> (Option<SongLoopCrossfade>, Duration) {
        (self.sources.song_loop, self.pass_end)
    }

    /// Builds ahead of the next jump of `effective`, the one section looping
    /// now, and schedules it once it is within the lookahead of `elapsed`
    /// (where playback is on its timeline).
    pub fn poll(&mut self, effective: Option<&EffectiveSection>, elapsed: Duration) -> Poll {
        let looping_whole = effective.is_some_and(EffectiveSection::is_whole_song);
        // Arming (or disarming) the trigger gives when its next jump falls,
        // to build ahead of it.
        let next_jump = self.trigger.arm(effective, elapsed);
        self.prewarm(effective, elapsed, next_jump, looping_whole);

        // A looping song with a crossfade starts each pass its overlap
        // before the last one ends, as the trigger times it: the passes
        // cross over, each group of tracks its own way.
        if let (true, Some(section), Some(target)) =
            (self.sources.song_loop.is_some(), effective, next_jump)
        {
            if section.is_whole_song() {
                return self.poll_pass(section, effective, elapsed, target);
            }
        }

        // Under a section of a crossfading song that runs into the
        // crossfade, the song's next pass is kept built: broken there, the
        // section hands over to it at once.
        if let (Some(song_loop), Some(section)) = (self.sources.song_loop.as_mut(), effective) {
            if !section.is_whole_song()
                && section.bounds.end_time + PASS_PREBUILD_AHEAD + song_loop.overlap
                    >= self.ctx.song.duration()
            {
                Device::prebuild_pass(
                    self.ctx.song,
                    song_loop,
                    self.ctx.playback_context,
                    self.ctx.mappings,
                    Duration::ZERO,
                );
            }
        }

        // A section loop, or a song going round as a whole without a
        // crossfade, from its top.
        let Some(section) = effective else {
            return Poll::Idle;
        };
        let Some(jump) = self
            .trigger
            .jump_effective(Some(section), elapsed, self.ctx.lookahead)
        else {
            return Poll::Idle;
        };
        let (ctx, sources) = (&self.ctx, &mut self.sources);
        let commit = commit_jump(
            &mut self.trigger,
            ctx.cancel_handle,
            ctx.loop_break,
            ctx.section_jumps,
            jump,
            |jump| sources.schedule_section(ctx, section, jump, elapsed),
        );
        self.committed(commit)
    }

    /// The jump of a crossfading song's next pass, at `target` on the
    /// playback timeline: its sources built ahead (from part way in, for a
    /// pass begun late), and scheduled to cross over as it comes.
    fn poll_pass(
        &mut self,
        section: &EffectiveSection,
        effective: Option<&EffectiveSection>,
        elapsed: Duration,
        target: Duration,
    ) -> Poll {
        let lookahead = self.ctx.lookahead;
        let song_loop = self
            .sources
            .song_loop
            .as_mut()
            .expect("a crossfading song's pass");
        let target = Device::plan_late_pass(
            song_loop,
            section,
            &mut self.trigger,
            effective,
            elapsed,
            lookahead,
        )
        .unwrap_or(target);
        // Begun late, a pass joins the grid part way in.
        let from = self.trigger.past_end(target);
        let Some(jump) = self.trigger.jump_effective(effective, elapsed, lookahead) else {
            self.sources.prepare_pass(&self.ctx, elapsed, target, from);
            return Poll::Idle;
        };
        let (ctx, sources) = (&self.ctx, &mut self.sources);
        let commit = commit_jump(
            &mut self.trigger,
            ctx.cancel_handle,
            ctx.loop_break,
            ctx.section_jumps,
            jump,
            |jump| {
                sources.prepare_pass(ctx, elapsed, target, from);
                sources.schedule_pass(ctx, jump, elapsed)
            },
        );
        self.committed(commit)
    }

    /// What a commit comes to for the monitor loop.
    fn committed(&mut self, commit: Commit<u64>) -> Poll {
        match commit {
            Commit::Made(jump, start_sample) => {
                self.pass_end = self.ctx.song.duration() + self.trigger.consumed();
                Poll::Scheduled(ScheduledJump { jump, start_sample })
            }
            Commit::NotReady => Poll::Idle,
            Commit::Stopped => Poll::Stop,
        }
    }

    /// Keeps warm loop-back sources for the section the next jump goes to,
    /// building them in the background.
    fn prewarm(
        &mut self,
        effective: Option<&EffectiveSection>,
        elapsed: Duration,
        next_jump: Option<Duration>,
        looping_whole: bool,
    ) {
        let sources = &mut self.sources;
        // Which section we want warm loop-back sources for. Once armed, the
        // active section is authoritative. Before arming, speculatively
        // target whichever section the playhead is currently inside -- it is
        // the loop-back candidate, so the sources are warm the instant the
        // performer arms, even on the section's final beat. (During looping
        // the raw clock runs past the section end, so we rely on `armed`
        // rather than the playhead lookup.)
        //
        // After a break too: reactive looping arms the next section as
        // playback reaches it, and with nothing warm a late arm loops late.
        //
        // A song going round as a whole without a crossfade loops through
        // this too, from its top: warmed as its pass nears its end. (With
        // a crossfade its passes are built their own way.)
        let pass_due_soon = looping_whole
            && sources.song_loop.is_none()
            && next_jump.is_some_and(|at| elapsed + PASS_PREBUILD_AHEAD >= at);
        let prewarm_target = match effective {
            Some(section) if !section.is_whole_song() || pass_due_soon => {
                Some(section.bounds.clone())
            }
            // The section the playhead sits in is the loop-back candidate:
            // whichever section the performer is hearing could be armed at
            // any moment, and arming loops back to its start, so its
            // sources are prewarmed speculatively rather than opened at the
            // first boundary. The clock is stream-relative; section bounds
            // are song-absolute. Offset by the playback start position, and
            // take off what loops have put back to have the song's.
            _ => crate::player::SectionBounds::at(
                self.ctx.song,
                elapsed.saturating_sub(self.trigger.consumed()),
            ),
        };

        // Collect the result of a finished background build. Take the
        // channel out and only restore it if the build is still running.
        let wanted = |name: &str| prewarm_target.as_ref().is_some_and(|t| t.name == name);
        if let Some((name, rx)) = sources.build_rx.take() {
            match rx.try_recv() {
                Ok(Ok(srcs)) => {
                    // Keep only if we still want this section's sources.
                    if wanted(&name) {
                        sources.pending = Some((name, srcs));
                    }
                }
                Ok(Err(e)) => {
                    error!(
                        err = %e,
                        section = name,
                        "Failed to pre-build section loop sources; will retry"
                    );
                }
                // Still building -- put the channel back.
                Err(mpsc::TryRecvError::Empty) => sources.build_rx = Some((name, rx)),
                // Worker vanished without sending (e.g. spawn failed);
                // leave build_rx cleared so we retry next iteration.
                Err(mpsc::TryRecvError::Disconnected) => {}
            }
        }

        // Drop warmed sources / abandon an in-flight build if the target
        // section changed (playhead moved on, loop armed elsewhere, section
        // cleared, or a break was requested) -- otherwise we'd swap stale
        // audio in on the next trigger.
        if sources
            .pending
            .as_ref()
            .is_some_and(|(name, _)| !wanted(name))
        {
            sources.pending = None;
        }
        if sources
            .build_rx
            .as_ref()
            .is_some_and(|(name, _)| !wanted(name))
        {
            // Dropping the receiver detaches the worker; its eventual send
            // is a harmless no-op.
            sources.build_rx = None;
        }

        // Kick off a background build for the target if we have neither
        // warmed sources nor a build already in flight. The worker thread
        // absorbs the warmup/I/O block so trigger detection here is never
        // delayed.
        if let Some(target) = &prewarm_target {
            if sources.pending.is_none() && sources.build_rx.is_none() {
                let (tx, rx) = mpsc::channel();
                let song = self.ctx.song.clone();
                let ctx = self.ctx.playback_context.clone();
                let mappings = self.ctx.mappings.clone();
                let start_time = target.start_time;
                let cut_tracks = self.ctx.cut_tracks.clone();
                let _ = thread::Builder::new()
                    .name("mtrack-section-prebuild".into())
                    .spawn(move || {
                        let result = build_pass_sources(
                            &song,
                            &ctx,
                            &mappings,
                            start_time,
                            start_time,
                            &cut_tracks,
                        );
                        // Receiver may be gone if the target changed.
                        let _ = tx.send(result);
                    });
                sources.build_rx = Some((target.name.clone(), rx));
            }
        }
    }
}

impl LoopSources {
    /// Builds a crossfading song's next pass ahead of its jump, at `target`
    /// on the playback timeline, from `from` in the song (the top, or part
    /// way in for one begun late).
    fn prepare_pass(
        &mut self,
        ctx: &LoopContext<'_>,
        now: Duration,
        target: Duration,
        from: Duration,
    ) {
        let song_loop = self.song_loop.as_mut().expect("a crossfading song's pass");
        if now + PASS_PREBUILD_AHEAD >= target {
            Device::prebuild_pass(
                ctx.song,
                song_loop,
                ctx.playback_context,
                ctx.mappings,
                from,
            );
        } else {
            Device::collect_pass(song_loop);
        }
    }

    /// Schedules a crossfading song's next pass at `jump`, if it is built:
    /// the fading tracks cross over the whole overlap on an equal-power
    /// curve, the cut tracks switch at its midpoint. Gives the sample the
    /// pass starts at, also kept in `song_loop.latest_start`.
    fn schedule_pass(
        &mut self,
        ctx: &LoopContext<'_>,
        jump: &SectionJump,
        now: Duration,
    ) -> Option<u64> {
        let song = ctx.song;
        let song_loop = self.song_loop.as_mut().expect("a crossfading song's pass");
        // A pass begun late, inside the crossfade (a start or a section loop
        // broken there, or a slow build), has less of it to cross. Every
        // track of it is built from the same point, so its cut tracks stay
        // in step whatever the overlap comes to.
        let overlap = jump.overlap.min(song_loop.overlap);
        if now + ctx.lookahead < jump.at {
            return None;
        }
        let Some((_, incoming)) = song_loop.prebuilt.take() else {
            if !song_loop.warned_late {
                warn!(
                    song = song.name(),
                    "Song loop crossfade: the next pass is not built at its boundary; \
                     starting it as soon as it is"
                );
                song_loop.warned_late = true;
            }
            return None;
        };
        song_loop.warned_late = false;

        let output_manager = &ctx.device.output_manager;
        let mixer = &output_manager.mixer;
        let rate = mixer.sample_rate();
        let now_sample = mixer.current_sample();
        // Late (a slow build, or a start inside the overlap): as soon as
        // possible rather than in the past.
        let start_sample = jump
            .boundary_sample(now_sample, now, rate)
            .saturating_add(ctx.skew_samples)
            .max(now_sample + crate::audio::duration_to_samples(ctx.lookahead / 2, rate));
        let crossover = PassCrossover {
            start_sample,
            overlap_samples: crate::audio::duration_to_samples(overlap, rate),
            crossfade_samples: ctx.crossfade_samples,
        };

        info!(
            song = song.name(),
            start_sample,
            overlap_ms = overlap.as_millis() as u64,
            "Song loop: scheduling the next pass's crossfade"
        );

        crossover.cross_out(mixer, &song_loop.fade_ids, &song_loop.cut_ids);
        song_loop.outgoing_ids = song_loop
            .fade_ids
            .iter()
            .chain(&song_loop.cut_ids)
            .copied()
            .collect();

        // The incoming pass, each source with its own envelope.
        song_loop.fade_ids.clear();
        song_loop.cut_ids.clear();
        self.finish_flags.clear();
        let mut incoming_ids = Vec::new();
        for (source, group) in incoming {
            let (envelope, starts_at) = crossover.cross_in(group);
            let (mut active_source, flag) = build_active_source(
                source,
                ctx.mappings,
                ctx.cancel_handle,
                Some(Arc::new(envelope)),
            );
            active_source.start_at_sample = Some(starts_at);
            match group {
                PassGroup::Fade => song_loop.fade_ids.push(active_source.id),
                PassGroup::Cut => song_loop.cut_ids.push(active_source.id),
            }
            incoming_ids.push(active_source.id);
            self.finish_flags.push(flag);
            output_manager.add_source_now(active_source);
        }
        song_loop.latest_start = Some((start_sample, incoming_ids));
        Some(start_sample)
    }

    /// Schedules a section's jump (or a song's going round as a whole
    /// without a crossfade), if its sources are built: the playback's
    /// sources cut at the jump's sample, the built ones starting there.
    /// Gives that sample.
    fn schedule_section(
        &mut self,
        ctx: &LoopContext<'_>,
        section: &EffectiveSection,
        jump: &SectionJump,
        elapsed: Duration,
    ) -> Option<u64> {
        let Some((_, pending)) = self.pending.take() else {
            warn!(
                section = section.bounds.name,
                "Audio section loop: trigger fired with no pre-built \
                 sources; looping once they are ready"
            );
            return None;
        };
        let output_manager = &ctx.device.output_manager;
        let crossfade_samples = ctx.crossfade_samples;

        // Map the ideal trigger time to an exact mixer sample. We use the
        // live sample counter plus the remaining delta so the result is
        // robust to any small skew between the playback clock and the mixer
        // counter.
        let now_sample = output_manager.mixer.current_sample();
        // Correct for the startup skew so the boundary tracks the audio
        // content, not the clock epoch (see where `audio_clock_skew_samples`
        // is captured). The shift is a constant, so inter-boundary spacing
        // is unchanged.
        let loop_sample = jump
            .boundary_sample(now_sample, elapsed, output_manager.mixer.sample_rate())
            .saturating_add(ctx.skew_samples);
        // At a section-loop boundary the loop-point content coincides on
        // both sources: the section's `end_time` on the outgoing source is
        // the same musical moment as its `start_time` on the incoming one --
        // for a one-measure loop, literally the same downbeat. With the
        // boundary aligned (seek-residual trim + startup-skew correction),
        // the two copies land on the same sample, so we overlap them and let
        // them reinforce into a single click rather than cutting one.
        //
        // The crossfade straddles the boundary on the outgoing side: the
        // outgoing source plays THROUGH loop_sample at full gain and fades
        // out over the following window, while the incoming source begins
        // exactly at loop_sample and fades in. This matters because the
        // outgoing source's resampler is in steady state, so its copy of the
        // downbeat is clean and carries the attack; the incoming source's
        // resampler is freshly created, so its first samples carry a sinc
        // pre-ring -- the fade-in attenuates that pre-ring while the clean
        // outgoing transient dominates the attack.
        let fade_end_sample = loop_sample.saturating_add(crossfade_samples);

        self.loop_iteration += 1;
        debug!(
            section = section.bounds.name,
            loop_iteration = self.loop_iteration,
            loop_sample,
            fade_end_sample,
            "Audio section loop: scheduling crossfade"
        );

        // Cut this playback's sources at loop_sample, each fading out from
        // where its own envelope has got to by then over the short
        // crossfade, and gone by fade_end_sample. A pass crossing out under
        // this one plays out its crossfade, as its notes do.
        let crossing_out = self
            .song_loop
            .as_ref()
            .map(|song_loop| song_loop.outgoing_ids.clone())
            .unwrap_or_default();
        output_manager.mixer.cut_playback_at(
            ctx.cancel_handle,
            loop_sample,
            fade_end_sample.saturating_sub(loop_sample),
            &crossing_out,
        );

        // Add the pre-built sources. Their own start_at_sample delays
        // playback until loop_sample; the fade-in envelope ramps from there
        // so the incoming source's resampler pre-ring is attenuated while
        // the clean outgoing transient carries the downbeat attack.
        self.finish_flags.clear();
        // A crossfading song's pass groups are these sources now: the next
        // pass crosses them over, not the ones cut here.
        if let Some(song_loop) = self.song_loop.as_mut() {
            song_loop.fade_ids.clear();
            song_loop.cut_ids.clear();
            song_loop.outgoing_ids.clear();
        }
        let mut incoming_ids = Vec::new();
        for (source, group) in pending {
            // Each source gets its own envelope instance -- a shared
            // GainEnvelope.position would advance once per source per batch
            // and the fade would complete N times too fast.
            let fade_in = Some(Arc::new(crate::audio::crossfade::GainEnvelope::fade_in(
                crossfade_samples,
                crate::audio::crossfade::CrossfadeCurve::Linear,
            )));
            let (mut active_source, flag) =
                build_active_source(source, ctx.mappings, ctx.cancel_handle, fade_in);
            active_source.start_at_sample = Some(loop_sample);
            self.finish_flags.push(flag);
            incoming_ids.push(active_source.id);
            if let Some(song_loop) = self.song_loop.as_mut() {
                match group {
                    PassGroup::Fade => song_loop.fade_ids.push(active_source.id),
                    PassGroup::Cut => song_loop.cut_ids.push(active_source.id),
                }
            }
            output_manager.add_source_now(active_source);
        }
        if let Some(song_loop) = self.song_loop.as_mut() {
            song_loop.latest_start = Some((loop_sample, incoming_ids));
        }
        Some(loop_sample)
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::player::SectionBounds;

    fn ms(ms: u64) -> Duration {
        Duration::from_millis(ms)
    }

    fn section() -> SectionBounds {
        SectionBounds {
            name: "a".into(),
            start_time: ms(10_000),
            end_time: ms(18_000),
        }
    }

    /// The trigger with its first jump given, as the scheduler has it when
    /// it commits.
    fn fired() -> (SectionLoopTrigger, SectionJump) {
        let mut trigger = SectionLoopTrigger::new();
        let jump = trigger.jump(&section(), ms(17_980), ms(30)).unwrap();
        (trigger, jump)
    }

    /// Scheduled, a jump is committed: pushed to the log, the time it puts
    /// back kept, and the next set a section's length on.
    #[test]
    fn a_jump_scheduled_is_committed() {
        let (mut trigger, jump) = fired();
        let log = SectionJumpLog::default();
        let commit = commit_jump(
            &mut trigger,
            &CancelHandle::new(),
            &AtomicBool::new(false),
            Some(&log),
            jump.clone(),
            |given| {
                assert_eq!(given, &jump);
                Some(7u64)
            },
        );
        assert_eq!(commit, Commit::Made(jump.clone(), 7));
        assert_eq!(log.committed(), 1);
        assert_eq!(trigger.consumed(), jump.amount());
        assert_eq!(trigger.check(&section(), ms(25_969), ms(30)), None);
        assert_eq!(
            trigger.check(&section(), ms(25_970), ms(30)),
            Some(ms(26_000))
        );
    }

    /// Without the log (the audio not committing the playback's jumps), a
    /// jump scheduled is made but goes nowhere.
    #[test]
    fn a_jump_is_logged_only_by_its_publisher() {
        let (mut trigger, jump) = fired();
        let commit = commit_jump(
            &mut trigger,
            &CancelHandle::new(),
            &AtomicBool::new(false),
            None,
            jump.clone(),
            |_| Some(()),
        );
        assert_eq!(commit, Commit::Made(jump.clone(), ()));
        assert_eq!(trigger.consumed(), jump.amount());
    }

    /// Nothing built to play at it, a jump is taken back: nothing logged,
    /// nothing put back, and it fires again at once, from where playback is.
    #[test]
    fn a_jump_not_scheduled_is_taken_back() {
        let (mut trigger, jump) = fired();
        let log = SectionJumpLog::default();
        let commit = commit_jump(
            &mut trigger,
            &CancelHandle::new(),
            &AtomicBool::new(false),
            Some(&log),
            jump,
            |_| None::<u64>,
        );
        assert_eq!(commit, Commit::NotReady);
        assert_eq!(log.committed(), 0);
        assert_eq!(trigger.consumed(), Duration::ZERO);
        let again = trigger.jump(&section(), ms(18_010), ms(30)).unwrap();
        assert_eq!((again.from, again.to), (ms(18_010), ms(10_000)));
    }

    /// Cancelled, or the loop broken, before it is scheduled, a jump is
    /// taken back without being scheduled at all.
    #[test]
    fn a_jump_after_a_stop_or_break_is_taken_back_unscheduled() {
        for (cancel, broken) in [(true, false), (false, true)] {
            let (mut trigger, jump) = fired();
            let log = SectionJumpLog::default();
            let cancel_handle = CancelHandle::new();
            if cancel {
                cancel_handle.cancel();
            }
            let commit = commit_jump(
                &mut trigger,
                &cancel_handle,
                &AtomicBool::new(broken),
                Some(&log),
                jump,
                |_| -> Option<()> { panic!("scheduled after a stop or break") },
            );
            assert_eq!(commit, Commit::Stopped);
            assert_eq!(log.committed(), 0);
            assert_eq!(trigger.consumed(), Duration::ZERO);
        }
    }

    /// A loop break holding the playback's jumps while it fades and breaks
    /// keeps a jump from being committed under it: once it lets go, the
    /// jump sees the break and is taken back.
    #[test]
    fn a_break_holding_the_jumps_keeps_a_jump_from_being_committed() {
        let cancel_handle = CancelHandle::new();
        let loop_break = Arc::new(AtomicBool::new(false));
        let log = Arc::new(SectionJumpLog::default());
        let hold = cancel_handle.hold_jumps();
        let committer = {
            let cancel_handle = cancel_handle.clone();
            let loop_break = loop_break.clone();
            let log = log.clone();
            thread::spawn(move || {
                let (mut trigger, jump) = fired();
                let commit = commit_jump(
                    &mut trigger,
                    &cancel_handle,
                    &loop_break,
                    Some(&log),
                    jump,
                    |_| Some(()),
                );
                (commit, trigger.consumed())
            })
        };
        thread::sleep(ms(50));
        assert_eq!(log.committed(), 0, "committed under the break's hold");
        loop_break.store(true, Ordering::Relaxed);
        drop(hold);
        let (commit, consumed) = committer.join().unwrap();
        assert_eq!(commit, Commit::Stopped);
        assert_eq!(consumed, Duration::ZERO);
        assert_eq!(log.committed(), 0);
    }
}
