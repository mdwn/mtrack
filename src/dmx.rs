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

pub mod engine;
pub mod midi_dmx_store;
pub mod ola_client;
pub mod patch_check;
pub mod universe;
pub mod watcher;

use crate::config;
use crate::dmx::ola_client::OlaClientFactory;
use engine::Engine;
#[cfg(not(test))]
use ola::client::StreamingClientConfig;
use std::thread;
use std::time::Duration;
use std::{error::Error, path::Path, sync::Arc};
use tracing::info;

/// Typed errors for the DMX subsystem.
#[derive(Debug, thiserror::Error)]
pub enum DmxError {
    #[error("DMX engine initialization error: {0}")]
    Init(String),

    #[error("DMX playback error: {0}")]
    Playback(String),

    #[error("DMX validation error: {0}")]
    Validation(String),

    #[error(transparent)]
    Other(Box<dyn Error + Send + Sync>),
}

impl From<Box<dyn Error + Send + Sync>> for DmxError {
    fn from(e: Box<dyn Error + Send + Sync>) -> Self {
        DmxError::Other(e)
    }
}

/// Creates a DMX engine, connecting to the OLA daemon for output.
/// Falls back to a no-op OLA client if the OLA daemon is unavailable, so the
/// lighting/effects engine can still run without physical hardware (web UI, etc.).
/// Calls `connect` up to `attempts` times, `pause` apart, until it succeeds;
/// the pause is taken in short steps, ending early once `stop` says so.
/// Gives back what it got: the value, or the last error.
fn retry_connect<T, E>(
    attempts: u32,
    pause: Duration,
    stop: &dyn Fn() -> bool,
    mut connect: impl FnMut() -> Result<T, E>,
) -> (Option<T>, Option<E>) {
    const STEP: Duration = Duration::from_millis(100);
    let mut last_err = None;
    for attempt in 0..attempts {
        if attempt > 0 {
            let resume = std::time::Instant::now() + pause;
            while std::time::Instant::now() < resume {
                if stop() {
                    return (None, last_err);
                }
                thread::sleep(
                    STEP.min(resume.saturating_duration_since(std::time::Instant::now())),
                );
            }
        }
        match connect() {
            Ok(value) => return (Some(value), None),
            Err(e) => last_err = Some(e),
        }
    }
    (None, last_err)
}

pub fn create_engine(
    config: Option<&config::Dmx>,
    base_path: Option<&Path>,
) -> Result<Option<Arc<Engine>>, Box<dyn Error>> {
    create_engine_inner(config, base_path, true, &|| false)
}

/// As [`create_engine`], giving up its wait for olad as soon as `stop` says
/// so: a hardware init round that has been cancelled. Its retries took up to
/// 45s in one blocking call, and a reload waits for the round it replaced to
/// finish before opening devices of its own.
pub fn create_engine_until(
    config: Option<&config::Dmx>,
    base_path: Option<&Path>,
    stop: &dyn Fn() -> bool,
) -> Result<Option<Arc<Engine>>, Box<dyn Error>> {
    create_engine_inner(config, base_path, true, stop)
}

fn create_engine_inner(
    config: Option<&config::Dmx>,
    base_path: Option<&Path>,
    allow_null_client: bool,
    #[cfg_attr(test, allow(unused_variables))] stop: &dyn Fn() -> bool,
) -> Result<Option<Arc<Engine>>, Box<dyn Error>> {
    let config = match config {
        Some(config) => config,
        None => return Ok(None),
    };

    // Use the lighting config from the DMX config if available
    let lighting_config = config.lighting();

    // Build a real OLA client and construct the engine
    // In test mode, use a mock client to avoid hanging on OLA connection
    #[cfg(test)]
    let ola_client = {
        let _ = allow_null_client; // Only used in non-test builds
        OlaClientFactory::create_mock_client_unconditional()
    };

    // Only a real connection is worth asking olad about its patch state: the
    // null client streams nowhere by design.
    #[cfg(not(test))]
    let mut ola_connected = false;

    #[cfg(not(test))]
    let ola_client = if config.null_client() {
        info!("null_client enabled, skipping OLA connection");
        Box::new(ola_client::NullOlaClient) as Box<dyn ola_client::OlaClient>
    } else {
        let ola_client_config = StreamingClientConfig {
            server_port: config.ola_port(),
            auto_start: false,
        };
        // Retry connecting to OLA a few times with backoff
        let (found, last_err) = retry_connect(10, Duration::from_secs(5), stop, || {
            OlaClientFactory::create_real_client(ola_client_config.clone())
        });
        if found.is_none() && stop() {
            return Err("DMX engine creation was cancelled".into());
        }
        match (found, last_err) {
            (Some(client), _) => {
                ola_connected = true;
                client
            }
            (None, Some(e)) => {
                if allow_null_client {
                    info!("OLA not available, using null DMX client");
                    Box::new(ola_client::NullOlaClient) as Box<dyn ola_client::OlaClient>
                } else {
                    return Err(e);
                }
            }
            (None, None) => unreachable!(),
        }
    };

    let engine = Arc::new(Engine::new(config, lighting_config, base_path, ola_client)?);

    info!(
        lighting = lighting_config.is_some(),
        "DMX engine initialized"
    );

    // Reload re-enters create_engine, so this covers startup and reload alike.
    #[cfg(not(test))]
    if ola_connected {
        patch_check::warn_unpatched_universes(
            config.ola_http_port(),
            config
                .universes()
                .iter()
                .map(|universe| universe.universe())
                .collect(),
        );
    }

    // Register the current venue now, not first at a song start, so a venue
    // that does not load says so at boot (status, banner, log) instead of
    // when the first song plays dark. A failure is recorded, not fatal.
    let _ = engine.register_venue_fixtures_safe();

    // Start the persistent effects loop
    Engine::start_persistent_effects_loop(engine.clone());

    Ok(Some(engine))
}

#[cfg(test)]
mod test {
    use super::*;

    /// A wait for olad gives up within a step of being told to stop: a
    /// cancelled init round is not held for the rest of its retries, which a
    /// reload waits out before opening devices of its own.
    #[test]
    fn retry_connect_stops_when_told() {
        let started = std::time::Instant::now();
        let stop = || started.elapsed() > Duration::from_millis(150);
        let mut calls = 0;
        let (found, last_err) = retry_connect(10, Duration::from_secs(5), &stop, || {
            calls += 1;
            Err::<(), _>("olad is down")
        });
        assert!(found.is_none());
        assert_eq!(last_err, Some("olad is down"));
        assert_eq!(calls, 1);
        // Within a step of the stop, not after the 5s pause.
        assert!(
            started.elapsed() < Duration::from_secs(4),
            "{:?}",
            started.elapsed()
        );
    }

    /// It retries until it connects.
    #[test]
    fn retry_connect_retries_until_it_connects() {
        let mut calls = 0;
        let (found, last_err) = retry_connect(10, Duration::from_millis(1), &|| false, || {
            calls += 1;
            if calls < 3 {
                Err("not yet")
            } else {
                Ok(calls)
            }
        });
        assert_eq!(found, Some(3));
        assert_eq!(last_err, None);
    }

    #[test]
    fn create_engine_none_config_returns_none() {
        let result = create_engine(None, None).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn create_engine_with_config_returns_some() {
        let dmx_config = config::Dmx::new(
            None,
            None,
            Some(9090),
            vec![config::Universe::new(1, "test".to_string())],
            None,
        );
        let result = create_engine(Some(&dmx_config), None).unwrap();
        assert!(result.is_some());
    }
}
