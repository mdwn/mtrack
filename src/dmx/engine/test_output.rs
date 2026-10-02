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

//! A fixture test's output: one fixture's channels laid over a universe's
//! frame, last, as it goes out (see `Universe::compose`), so it wins over
//! effects and MIDI-DMX while it is set and the show's own values return
//! the moment it is released.
//!
//! It expires: every update carries it [`TEST_OUTPUT_EXPIRY`] forward, and
//! the effects loop releases it once that passes with no update — a closed
//! tab cannot leave a light on or strobing. Playback and the player lock
//! release it too.

use std::time::{Duration, Instant};

use super::Engine;

/// How long a test holds after its last update.
pub const TEST_OUTPUT_EXPIRY: Duration = Duration::from_secs(5);

/// The environment variable that shortens the expiry, in milliseconds — for
/// tests of the expiry itself against a running server.
pub const EXPIRY_ENV: &str = "MTRACK_TEST_OUTPUT_EXPIRY_MS";

/// The expiry this process uses.
pub(super) fn expiry_from_env() -> Duration {
    std::env::var(EXPIRY_ENV)
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|ms| *ms > 0)
        .map(Duration::from_millis)
        .unwrap_or(TEST_OUTPUT_EXPIRY)
}

/// One fixture's test output.
#[derive(Clone, Debug, PartialEq)]
pub struct TestOutput {
    pub universe: u16,
    pub address: u16,
    pub footprint: u16,
    pub fixture_type: String,
    pub mode: Option<String>,
    /// 1-based DMX channel and value, for the whole span.
    pub channels: Vec<(u16, u8)>,
}

/// What `/api/status` says about a running test.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct TestOutputStatus {
    pub universe: u16,
    pub address: u16,
    pub footprint: u16,
    pub fixture_type: String,
    pub mode: Option<String>,
    pub expires_in_secs: f64,
}

/// The test in force, with when it lapses.
pub(super) struct ActiveTest {
    output: TestOutput,
    expires: Instant,
}

impl Engine {
    /// Sets (or replaces) the test output and carries its expiry forward;
    /// answers how long it holds. A universe this profile has no output for
    /// is refused.
    pub fn set_test_output(&self, output: TestOutput) -> Result<Duration, String> {
        let Some(universe) = self.universes.get(&output.universe) else {
            return Err(format!(
                "universe {} is not one of this profile's DMX universes",
                output.universe
            ));
        };
        let mut active = self.test_output.lock();
        if let Some(previous) = active.as_ref() {
            if previous.output.universe != output.universe {
                if let Some(old) = self.universes.get(&previous.output.universe) {
                    old.set_test_override(None);
                }
            }
        }
        if active.as_ref().map(|a| &a.output.channels) != Some(&output.channels)
            || active.as_ref().map(|a| a.output.universe) != Some(output.universe)
        {
            universe.set_test_override(Some(&output.channels));
            *self.test_overlay.write() = Some(crate::state::TestOverlay {
                universe: output.universe,
                channels: output.channels.clone(),
            });
        }
        *active = Some(ActiveTest {
            output,
            expires: Instant::now() + self.test_expiry,
        });
        Ok(self.test_expiry)
    }

    /// Releases the test output now, if there is one; whether there was.
    pub fn release_test_output(&self) -> bool {
        let Some(active) = self.test_output.lock().take() else {
            return false;
        };
        *self.test_overlay.write() = None;
        if let Some(universe) = self.universes.get(&active.output.universe) {
            universe.set_test_override(None);
        }
        tracing::info!(
            universe = active.output.universe,
            address = active.output.address,
            "Fixture test output released"
        );
        true
    }

    /// The running test's bytes, shared with the state sampler so the
    /// stage views draw what leaves for olad.
    pub fn test_overlay(&self) -> crate::state::TestOverlayHandle {
        self.test_overlay.clone()
    }

    /// The running test, if any.
    pub fn test_output_status(&self) -> Option<TestOutputStatus> {
        self.test_output
            .lock()
            .as_ref()
            .map(|active| TestOutputStatus {
                universe: active.output.universe,
                address: active.output.address,
                footprint: active.output.footprint,
                fixture_type: active.output.fixture_type.clone(),
                mode: active.output.mode.clone(),
                expires_in_secs: active
                    .expires
                    .saturating_duration_since(Instant::now())
                    .as_secs_f64(),
            })
    }

    /// The effects loop's check: a test whose expiry has passed is released.
    pub(super) fn expire_test_output(&self) {
        let lapsed = self
            .test_output
            .lock()
            .as_ref()
            .is_some_and(|active| Instant::now() >= active.expires);
        if lapsed {
            self.release_test_output();
        }
    }

    #[cfg(test)]
    pub(crate) fn set_test_expiry(&mut self, expiry: Duration) {
        self.test_expiry = expiry;
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::sync::Arc;

    use super::*;
    use crate::config;
    use crate::dmx::ola_client::MockOlaClient;
    use crate::lighting::effects::{EffectType, FixtureInfo};
    use crate::lighting::EffectInstance;

    type Sent = Arc<parking_lot::Mutex<Vec<crate::dmx::ola_client::DmxMessage>>>;

    /// An engine with universe 1, a PAR (red, green, blue at 1-3) a show
    /// holds at full red, and the frames it sends.
    fn engine_with_red_par(expiry: Duration) -> Result<(Arc<Engine>, Sent), Box<dyn Error>> {
        let client = MockOlaClient::new();
        let sent = client.sent_messages.clone();
        let mut engine = Engine::new(
            &config::Dmx::new(
                None,
                None,
                Some(9090),
                vec![config::Universe::new(1, "main".to_string())],
                None,
            ),
            None,
            None,
            Box::new(client),
        )?;
        engine.set_test_expiry(expiry);
        let engine = Arc::new(engine);
        {
            let mut effects = engine.effect_engine.lock();
            effects.register_fixture(FixtureInfo::new(
                "par".into(),
                1,
                1,
                "Par".into(),
                [("red", 1), ("green", 2), ("blue", 3)]
                    .into_iter()
                    .map(|(n, o)| (n.to_string(), o))
                    .collect(),
                None,
            ));
            effects.start_effect(EffectInstance::new(
                "red".into(),
                EffectType::Static {
                    parameters: [("red".to_string(), 1.0)].into_iter().collect(),
                    duration: Duration::from_secs(60),
                },
                vec!["par".into()],
                None,
                None,
                None,
            ))?;
        }
        engine.update_effects()?;
        Ok((engine, sent))
    }

    /// Waits for universe 1's frame to start with `want`.
    fn frame_becomes(sent: &Sent, want: &[u8]) -> Result<(), Box<dyn Error>> {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            let last = sent
                .lock()
                .iter()
                .rev()
                .find(|m| m.universe == 1)
                .map(|m| m.buffer[0..want.len()].to_vec());
            if last.as_deref() == Some(want) {
                return Ok(());
            }
            if Instant::now() > deadline {
                return Err(format!("frame stayed {last:?}, wanted {want:?}").into());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn green_test() -> TestOutput {
        TestOutput {
            universe: 1,
            address: 1,
            footprint: 3,
            fixture_type: "Par".into(),
            mode: None,
            channels: vec![(1, 0), (2, 255), (3, 0)],
        }
    }

    #[test]
    fn a_test_wins_over_the_show_and_releases_back_to_it() -> Result<(), Box<dyn Error>> {
        let (engine, sent) = engine_with_red_par(TEST_OUTPUT_EXPIRY)?;
        frame_becomes(&sent, &[255, 0, 0])?;

        let holds = engine.set_test_output(green_test())?;
        assert_eq!(holds, TEST_OUTPUT_EXPIRY);
        frame_becomes(&sent, &[0, 255, 0])?;
        // The show keeps running underneath and does not get through.
        engine.update_effects()?;
        frame_becomes(&sent, &[0, 255, 0])?;
        let status = engine.test_output_status().expect("live");
        assert_eq!(
            (status.universe, status.address, status.footprint),
            (1, 1, 3)
        );
        assert!(status.expires_in_secs > 4.0);
        // The stage views draw the same bytes, over the show's state.
        let overlay = engine.test_overlay().read().clone().expect("drawn");
        assert_eq!(overlay.channels, green_test().channels);
        let drawn = crate::state::engine_snapshot(
            &engine.effect_engine.lock(),
            Some(&overlay),
            &Default::default(),
        );
        assert_eq!(drawn.under_test, vec!["par"]);
        let par = drawn.fixtures.iter().find(|f| f.name == "par").unwrap();
        assert_eq!(
            (
                par.channels["red"],
                par.channels["green"],
                par.channels["blue"]
            ),
            (0, 255, 0)
        );

        assert!(engine.release_test_output());
        frame_becomes(&sent, &[255, 0, 0])?;
        assert!(engine.test_overlay().read().is_none(), "nothing left drawn");
        assert!(engine.test_output_status().is_none());
        assert!(!engine.release_test_output(), "nothing left to release");
        engine.cancel_handle.cancel();
        Ok(())
    }

    #[test]
    fn a_test_nobody_keeps_alive_goes_dark_by_itself() -> Result<(), Box<dyn Error>> {
        let (engine, sent) = engine_with_red_par(Duration::from_millis(60))?;
        engine.set_test_output(green_test())?;
        frame_becomes(&sent, &[0, 255, 0])?;
        // Still inside its time: kept.
        engine.expire_test_output();
        assert!(engine.test_output_status().is_some());
        std::thread::sleep(Duration::from_millis(90));
        engine.expire_test_output();
        assert!(engine.test_output_status().is_none());
        assert!(engine.test_overlay().read().is_none());
        frame_becomes(&sent, &[255, 0, 0])?;
        engine.cancel_handle.cancel();
        Ok(())
    }

    #[test]
    fn an_update_carries_the_expiry_forward() -> Result<(), Box<dyn Error>> {
        let (engine, _sent) = engine_with_red_par(Duration::from_millis(80))?;
        engine.set_test_output(green_test())?;
        for _ in 0..4 {
            std::thread::sleep(Duration::from_millis(40));
            engine.set_test_output(green_test())?;
            engine.expire_test_output();
            assert!(engine.test_output_status().is_some());
        }
        engine.cancel_handle.cancel();
        Ok(())
    }

    #[test]
    fn a_universe_the_profile_lacks_is_refused() -> Result<(), Box<dyn Error>> {
        let (engine, _sent) = engine_with_red_par(TEST_OUTPUT_EXPIRY)?;
        let mut test = green_test();
        test.universe = 7;
        let err = engine.set_test_output(test).unwrap_err();
        assert!(err.contains("universe 7"), "{err}");
        assert!(engine.test_output_status().is_none());
        engine.cancel_handle.cancel();
        Ok(())
    }

    #[test]
    fn playing_a_song_releases_the_test() -> Result<(), Box<dyn Error>> {
        let (engine, sent) = engine_with_red_par(TEST_OUTPUT_EXPIRY)?;
        engine.set_test_output(green_test())?;
        frame_becomes(&sent, &[0, 255, 0])?;

        let dir = tempfile::tempdir()?;
        let song_config = crate::config::Song::new(
            "Quiet",
            None,
            None,
            None,
            None,
            None,
            vec![],
            std::collections::HashMap::new(),
            Vec::new(),
        );
        let song = Arc::new(crate::songs::Song::new(dir.path(), &song_config)?);
        let (ready_tx, _ready_rx) = std::sync::mpsc::channel::<()>();
        let clock = crate::clock::PlaybackClock::wall();
        clock.start();
        let cancel_handle = crate::playsync::CancelHandle::new();
        cancel_handle.cancel();
        Engine::play(
            engine.clone(),
            song,
            crate::playsync::PlaybackSync {
                cancel_handle,
                ready_tx: crate::playsync::ReadyGuard::new(ready_tx),
                clock,
                start_time: Duration::ZERO,
                loop_control: crate::playsync::LoopControl::new(),
            },
        )?;
        assert!(engine.test_output_status().is_none());
        frame_becomes(&sent, &[255, 0, 0])?;
        engine.cancel_handle.cancel();
        Ok(())
    }
}
