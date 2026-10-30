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
use parking_lot::{Condvar, Mutex};
use std::sync::{atomic::AtomicBool, atomic::Ordering, Arc};
use std::time::Duration;

/// Drop guard for the readiness channel.
///
/// Wraps a `Sender<()>` and guarantees that exactly one message is sent — either
/// explicitly via [`send`] or automatically on [`Drop`]. This prevents deadlocks
/// in the `play_files` ready-wait loop when a subsystem returns early without
/// signaling readiness.
///
/// [`send`]: ReadyGuard::send
pub struct ReadyGuard {
    tx: Option<std::sync::mpsc::Sender<()>>,
}

impl ReadyGuard {
    /// Creates a new guard that will send on drop if not already sent.
    pub fn new(tx: std::sync::mpsc::Sender<()>) -> Self {
        Self { tx: Some(tx) }
    }

    /// Explicitly send the ready signal, consuming the inner sender.
    ///
    /// Subsequent calls and the `Drop` impl are no-ops.
    pub fn send(&mut self) {
        if let Some(tx) = self.tx.take() {
            let _ = tx.send(());
        }
    }
}

impl Drop for ReadyGuard {
    fn drop(&mut self) {
        self.send();
    }
}

/// Bundles the playback synchronization state passed to `play_from` / `play`.
/// Includes the cancel handle, clock, ready signal, start time, and loop control.
pub struct PlaybackSync {
    pub cancel_handle: CancelHandle,
    pub ready_tx: ReadyGuard,
    pub clock: crate::clock::PlaybackClock,
    pub start_time: Duration,
    pub loop_control: LoopControl,
}

/// How the song a loop break moves to fades in under the one fading out.
#[derive(Clone, Debug)]
pub struct StartFade {
    /// The song it is for: another song started instead does not fade in.
    pub song: String,
    pub fade: crate::songs::ResolvedFade,
}

/// A loop break's fade-out in progress (a playlist `fade_out`).
#[derive(Clone, Debug)]
pub struct BreakFade {
    /// When the break came.
    pub started: std::time::Instant,
    /// How long the audio fades.
    pub length: Duration,
    pub curve: crate::audio::crossfade::CrossfadeCurve,
    /// Tracks that cut at once instead.
    pub exclude: Vec<String>,
}

/// Bundles the shared loop-control state threaded through audio, MIDI, and DMX
/// `play_from` / `play` methods.
#[derive(Clone)]
pub struct LoopControl {
    /// Shared flag to break out of the whole-song loop gracefully.
    pub loop_break: Arc<AtomicBool>,
    /// Active section loop bounds (shared with player).
    pub active_section: Arc<parking_lot::RwLock<Option<crate::player::SectionBounds>>>,
    /// Shared flag to break out of a section loop.
    pub section_loop_break: Arc<AtomicBool>,
    /// A loop break's fade-out (a playlist `fade_out`), set by the player as it
    /// breaks the loop.
    pub break_fade: Arc<parking_lot::Mutex<Option<BreakFade>>>,
    /// The sources fading out after a loop break, which a stop cuts.
    pub break_fade_ids: Arc<parking_lot::Mutex<Vec<u64>>>,
    /// How the song fades in under a song a loop break is fading out (a
    /// playlist `fade_in`), set by the player as it breaks the loop and taken
    /// by the audio device as the song starts.
    pub start_fade: Arc<parking_lot::Mutex<Option<StartFade>>>,
    /// This playback's section loop jumps, as they are committed to: by the
    /// audio when it plays any, by a thread of their own otherwise. The
    /// notes, lights and beat clock follow them, so none can decide a
    /// moment before or after the others.
    pub section_jumps: Arc<SectionJumpLog>,
}

/// A playback's section loop jumps, in order, as they are committed to. Each
/// follower reads it with a [`JumpCursor`] of its own.
#[derive(Default)]
pub struct SectionJumpLog {
    jumps: parking_lot::Mutex<Vec<crate::section_loop::SectionJump>>,
    /// Who commits the jumps: nobody yet, the audio, or the playback's own
    /// publisher. One only, whatever happens to the audio at the start.
    publisher: std::sync::atomic::AtomicU8,
    /// The playback's clock and start position: the timeline the jumps are
    /// timed on, read to know which have fallen due.
    timeline: parking_lot::Mutex<Option<(crate::clock::PlaybackClock, Duration)>>,
    /// Set once whoever commits the jumps has stopped playing: no more will
    /// come. Until then the followers follow, even past the end of their own
    /// notes or cues, since a section armed late, or one looping past them,
    /// takes them back.
    closed: AtomicBool,
}

/// Who commits a playback's section loop jumps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JumpPublisherRole {
    Audio = 1,
    Fallback = 2,
}

impl SectionJumpLog {
    /// Takes on committing the jumps, unless another has already: true if
    /// `role` has it now.
    pub fn claim(&self, role: JumpPublisherRole) -> bool {
        use std::sync::atomic::Ordering;
        match self
            .publisher
            .compare_exchange(0, role as u8, Ordering::AcqRel, Ordering::Acquire)
        {
            Ok(_) => true,
            Err(current) => current == role as u8,
        }
    }

    /// Sets the playback's clock and start position, which the jumps are
    /// timed on.
    pub fn set_timeline(&self, clock: crate::clock::PlaybackClock, start_time: Duration) {
        *self.timeline.lock() = Some((clock, start_time));
    }

    /// The time the jumps fallen due by now put back, and how many they are:
    /// read on the playback's own clock, the jumps' timeline. None due before
    /// it has started.
    pub fn put_back_now(&self) -> (Duration, usize) {
        self.now()
            .map_or((Duration::ZERO, 0), |now| self.put_back_by(now))
    }

    /// Where the playback is on its timeline (the start position plus its
    /// clock), once its clock has started.
    pub fn now(&self) -> Option<Duration> {
        self.timeline
            .lock()
            .as_ref()
            .filter(|(clock, _)| clock.has_started())
            .map(|(clock, start_time)| *start_time + clock.elapsed())
    }

    /// The time the jumps falling due by `at` (on the playback timeline) put
    /// back, and how many they are.
    pub fn put_back_by(&self, at: Duration) -> (Duration, usize) {
        self.jumps
            .lock()
            .iter()
            .take_while(|jump| jump.at <= at)
            .fold((Duration::ZERO, 0), |(time, count), jump| {
                (time + jump.amount(), count + 1)
            })
    }

    /// Whether the audio has taken on committing the jumps.
    pub fn claimed_by_audio(&self) -> bool {
        self.publisher.load(std::sync::atomic::Ordering::Acquire) == JumpPublisherRole::Audio as u8
    }

    /// Records a jump committed to.
    pub fn push(&self, jump: crate::section_loop::SectionJump) {
        self.jumps.lock().push(jump);
    }

    /// Says no more jumps will come: the audio, or the publisher standing in
    /// for it, has stopped playing.
    pub fn close(&self) {
        self.closed.store(true, Ordering::Release);
    }

    /// Whether no more jumps will come.
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }

    /// The jump at `index`, if it has been committed to.
    fn get(&self, index: usize) -> Option<crate::section_loop::SectionJump> {
        self.jumps.lock().get(index).cloned()
    }
}

/// A follower's place in a [`SectionJumpLog`]: the jumps it has passed, and
/// the time they have put back.
pub struct JumpCursor {
    log: Arc<SectionJumpLog>,
    next: usize,
    consumed: Duration,
}

impl JumpCursor {
    pub fn new(log: Arc<SectionJumpLog>) -> Self {
        JumpCursor {
            log,
            next: 0,
            consumed: Duration::ZERO,
        }
    }

    /// The next jump committed to, not yet passed.
    pub fn peek(&self) -> Option<crate::section_loop::SectionJump> {
        self.log.get(self.next)
    }

    /// Passes the next jump if it falls due by `at` on the playback timeline
    /// (the start position plus the clock), giving it.
    pub fn due(&mut self, at: Duration) -> Option<crate::section_loop::SectionJump> {
        let jump = self.peek().filter(|jump| jump.at <= at)?;
        self.next += 1;
        self.consumed += jump.amount();
        Some(jump)
    }

    /// The time the jumps passed have put back: playback time less this is
    /// the position in the song.
    pub fn consumed(&self) -> Duration {
        self.consumed
    }

    /// Whether another jump may yet come: one committed and not passed, or
    /// the log still open.
    pub fn may_jump(&self) -> bool {
        self.peek().is_some() || !self.log.is_closed()
    }

    /// The log followed.
    pub fn log(&self) -> &Arc<SectionJumpLog> {
        &self.log
    }
}

impl LoopControl {
    /// Creates a new `LoopControl` with all flags in their default (inactive) state.
    pub fn new() -> Self {
        Self {
            loop_break: Arc::new(AtomicBool::new(false)),
            active_section: Arc::new(parking_lot::RwLock::new(None)),
            section_loop_break: Arc::new(AtomicBool::new(false)),
            break_fade: Arc::new(parking_lot::Mutex::new(None)),
            break_fade_ids: Arc::new(parking_lot::Mutex::new(Vec::new())),
            start_fade: Arc::new(parking_lot::Mutex::new(None)),
            section_jumps: Arc::new(SectionJumpLog::default()),
        }
    }
}

impl LoopControl {
    /// A `LoopControl` for a playback nothing commits loop jumps for: its
    /// log closed from the start, so the lights and notes end with their
    /// own content, as a song without loops does.
    #[cfg(test)]
    pub fn without_jumps() -> Self {
        let loop_control = Self::new();
        loop_control.section_jumps.close();
        loop_control
    }
}

impl Default for LoopControl {
    fn default() -> Self {
        Self::new()
    }
}

/// Represents the current cancel state.
#[derive(PartialEq)]
enum CancelState {
    Untouched,
    /// Cancelled deliberately, by `stop()`, a seek, or a loop break. Whoever
    /// asked for it is responsible for the player-side cleanup.
    Cancelled,
    /// Cancelled by playback itself because it could not continue — the audio
    /// device disappearing mid-song. Nobody is waiting to clean up after this
    /// one, so the cleanup task has to do it.
    CancelledByFailure,
}

/// A cancel handle is passed to the device during a play operation. It's the player's responsibility
/// to respect a cancel request.
#[derive(Clone)]
pub struct CancelHandle {
    /// A boolean that should be set to true if the underlying operation should be cancelled.
    cancelled: Arc<Mutex<CancelState>>,
    /// The condvar will handle notification of cancelling.
    condvar: Arc<Condvar>,
}

impl CancelHandle {
    /// Creates a new cancel handle.
    pub fn new() -> CancelHandle {
        CancelHandle {
            cancelled: Arc::new(Mutex::new(CancelState::Untouched)),
            condvar: Arc::new(Condvar::new()),
        }
    }
}

impl Default for CancelHandle {
    fn default() -> Self {
        Self::new()
    }
}

impl CancelHandle {
    /// Returns true if the device process has been cancelled, for either reason.
    ///
    /// Every subsystem loop polls this to know it should wind up, and that is
    /// true whichever way the cancellation came about — only the player-side
    /// cleanup cares about the difference.
    pub fn is_cancelled(&self) -> bool {
        *self.cancelled.lock() != CancelState::Untouched
    }

    /// Whether `other` is a clone of this handle: the same playback.
    pub fn same_as(&self, other: &CancelHandle) -> bool {
        Arc::ptr_eq(&self.cancelled, &other.cancelled)
    }

    /// Waits for the cancel handle to be cancelled or for finished to be set to true.
    pub fn wait(&self, finished: Arc<AtomicBool>) {
        let mut guard = self.cancelled.lock();
        self.condvar.wait_while(&mut guard, |cancelled| {
            *cancelled == CancelState::Untouched && !finished.load(Ordering::Relaxed)
        });
    }

    /// Waits for the cancel handle to be cancelled or for finished to be set to true,
    /// with a timeout. Returns `true` if the condition was met, `false` if timed out.
    pub fn wait_with_timeout(
        &self,
        finished: Arc<AtomicBool>,
        timeout: std::time::Duration,
    ) -> bool {
        let mut guard = self.cancelled.lock();
        let result = self.condvar.wait_while_for(
            &mut guard,
            |cancelled| *cancelled == CancelState::Untouched && !finished.load(Ordering::Relaxed),
            timeout,
        );
        !result.timed_out()
    }

    /// Notifies the cancel handle to see if this the song has been cancelled or if the
    /// particular element has finished.
    ///
    /// Acquires the mutex before signaling so the notification cannot be lost
    /// between a waiter's predicate check and its condvar sleep.
    pub fn notify(&self) {
        let _guard = self.cancelled.lock();
        self.condvar.notify_all();
    }

    /// Cancel the device process.
    pub fn cancel(&self) {
        let mut cancel_state = self.cancelled.lock();
        if *cancel_state == CancelState::Untouched {
            *cancel_state = CancelState::Cancelled;
            // Signal directly — we already hold the mutex.
            self.condvar.notify_all();
        }
    }

    /// Cancels because playback cannot continue, rather than because anyone
    /// asked it to.
    ///
    /// Distinct from [`CancelHandle::cancel`] because the two want opposite
    /// cleanup. A deliberate cancel is issued by `stop()`, which clears the
    /// player's `join` and `play_start_time` itself and may already have started
    /// a *new* playback — so the cleanup task must not touch that state. A
    /// failure cancel has nobody behind it, so the cleanup task is the only
    /// thing that will ever clear it.
    ///
    /// Telling them apart by "was there also an error?" is not enough: a
    /// deliberately cancelled playback can report an error on its way out, and
    /// treating that as a failure lets the cleanup clobber the playback a seek
    /// has just started.
    pub fn cancel_due_to_failure(&self) {
        let mut cancel_state = self.cancelled.lock();
        if *cancel_state == CancelState::Untouched {
            *cancel_state = CancelState::CancelledByFailure;
            self.condvar.notify_all();
        }
    }

    /// Whether cancellation came from playback failing rather than from a
    /// deliberate stop, seek, or loop break.
    pub fn cancelled_due_to_failure(&self) -> bool {
        *self.cancelled.lock() == CancelState::CancelledByFailure
    }
}

#[cfg(test)]
mod test {
    use std::thread;

    use super::*;

    #[test]
    fn test_cancel_handle_cancelled() {
        let cancel_handle = CancelHandle::new();
        assert!(!cancel_handle.is_cancelled());

        let join = {
            let cancel_handle = cancel_handle.clone();
            thread::spawn(move || cancel_handle.wait(Arc::new(AtomicBool::new(false))))
        };

        cancel_handle.cancel();
        assert!(join.join().is_ok());
        assert!(cancel_handle.is_cancelled());
    }

    #[test]
    fn test_cancel_handle_finished() {
        let cancel_handle = CancelHandle::new();
        assert!(!cancel_handle.is_cancelled());

        let join = {
            let cancel_handle = cancel_handle.clone();
            thread::spawn(move || cancel_handle.wait(Arc::new(AtomicBool::new(true))))
        };

        assert!(join.join().is_ok());
        assert!(!cancel_handle.is_cancelled());
    }

    #[test]
    fn test_wait_with_timeout_returns_true_when_finished() {
        let cancel_handle = CancelHandle::new();
        let finished = Arc::new(AtomicBool::new(true));
        assert!(cancel_handle.wait_with_timeout(finished, std::time::Duration::from_secs(1)));
    }

    #[test]
    fn test_wait_with_timeout_returns_false_on_timeout() {
        let cancel_handle = CancelHandle::new();
        let finished = Arc::new(AtomicBool::new(false));
        assert!(!cancel_handle.wait_with_timeout(finished, std::time::Duration::from_millis(50)));
    }

    #[test]
    fn test_wait_with_timeout_returns_true_when_cancelled() {
        let cancel_handle = CancelHandle::new();
        let finished = Arc::new(AtomicBool::new(false));

        let join = {
            let cancel_handle = cancel_handle.clone();
            thread::spawn(move || {
                cancel_handle.wait_with_timeout(finished, std::time::Duration::from_secs(10))
            })
        };

        cancel_handle.cancel();
        assert!(join.join().unwrap());
    }

    #[test]
    fn test_cancel_idempotent() {
        let cancel_handle = CancelHandle::new();
        assert!(!cancel_handle.is_cancelled());

        cancel_handle.cancel();
        assert!(cancel_handle.is_cancelled());

        // Second cancel should be a no-op
        cancel_handle.cancel();
        assert!(cancel_handle.is_cancelled());
    }

    #[test]
    fn test_default_impl() {
        let cancel_handle = CancelHandle::default();
        assert!(!cancel_handle.is_cancelled());
    }

    #[test]
    fn test_clone_shares_state() {
        let handle1 = CancelHandle::new();
        let handle2 = handle1.clone();

        assert!(!handle2.is_cancelled());
        handle1.cancel();
        assert!(handle2.is_cancelled());
    }

    #[test]
    fn test_notify_wakes_waiter_when_finished() {
        let cancel_handle = CancelHandle::new();
        let finished = Arc::new(AtomicBool::new(false));

        let join = {
            let cancel_handle = cancel_handle.clone();
            let finished = finished.clone();
            thread::spawn(move || cancel_handle.wait(finished))
        };

        // Set finished and notify
        finished.store(true, Ordering::Relaxed);
        cancel_handle.notify();

        assert!(join.join().is_ok());
        // Should NOT be cancelled — just finished
        assert!(!cancel_handle.is_cancelled());
    }

    #[test]
    fn test_wait_returns_immediately_when_already_cancelled() {
        let cancel_handle = CancelHandle::new();
        cancel_handle.cancel();

        // Wait should return immediately since already cancelled
        let finished = Arc::new(AtomicBool::new(false));
        cancel_handle.wait(finished);
        assert!(cancel_handle.is_cancelled());
    }

    #[test]
    fn test_wait_with_timeout_returns_immediately_when_already_finished() {
        let cancel_handle = CancelHandle::new();
        let finished = Arc::new(AtomicBool::new(true));

        let result = cancel_handle.wait_with_timeout(finished, std::time::Duration::from_millis(1));
        assert!(result);
    }

    #[test]
    fn test_ready_guard_explicit_send() {
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        let mut guard = ReadyGuard::new(tx);
        guard.send();
        assert!(rx.try_recv().is_ok(), "explicit send should deliver");
        // Second send is a no-op.
        guard.send();
        assert!(
            rx.try_recv().is_err(),
            "second send should not deliver again"
        );
    }

    #[test]
    fn test_ready_guard_sends_on_drop() {
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        {
            let _guard = ReadyGuard::new(tx);
            // guard dropped here without explicit send
        }
        assert!(
            rx.try_recv().is_ok(),
            "drop should send the ready signal automatically"
        );
    }

    #[test]
    fn test_ready_guard_no_double_send_on_drop() {
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        {
            let mut guard = ReadyGuard::new(tx);
            guard.send();
            // guard dropped here; should not send again
        }
        assert!(rx.try_recv().is_ok(), "explicit send should arrive");
        assert!(
            rx.try_recv().is_err(),
            "drop after explicit send should not send again"
        );
    }
}
