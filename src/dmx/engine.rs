// Copyright (C) 2025 Michael Wilson <mike@mdwn.dev>
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

use std::{
    collections::{HashMap, HashSet},
    error::Error,
    net::TcpStream,
    sync::{
        atomic::AtomicBool,
        mpsc::{self, Receiver},
        Arc, Barrier, Mutex,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use midly::num::u7;
use nodi::{Connection, Player};
use ola::{client::StreamingClientConfig, DmxBuffer, StreamingClient};
use tracing::{debug, error, info, span, Level};

use crate::{
    config,
    lighting::{system::LightingSystem, EffectEngine},
    midi,
    playsync::CancelHandle,
    songs::{MidiSheet, Song},
};

use super::Universe;

/// The DMX engine. This is meant to control the current state of the
/// universe(s) that should be sent to our DMX interface(s).
pub struct Engine {
    dimming_speed_modifier: f64,
    playback_delay: Duration,
    universes: HashMap<String, Universe>,
    cancel_handle: CancelHandle,
    client_handle: Option<JoinHandle<()>>,
    join_handles: Vec<JoinHandle<()>>,
    /// Effects engine for processing lighting effects
    effect_engine: Arc<Mutex<EffectEngine>>,
    /// Lighting system for fixture and group management
    lighting_system: Option<LightingSystem>,
}

/// DmxMessage is a message that can be passed around between senders and receivers.
pub(super) struct DmxMessage {
    pub universe: u32,
    pub buffer: DmxBuffer,
}

impl Engine {
    /// Creates a new DMX Engine.
    pub fn new(config: &config::Dmx) -> Result<Engine, Box<dyn Error>> {
        Self::new_with_lighting(config, None, None)
    }

    /// Creates a new DMX Engine with lighting system.
    pub fn new_with_lighting(
        config: &config::Dmx,
        lighting_config: Option<&config::Lighting>,
        base_path: Option<&std::path::Path>,
    ) -> Result<Engine, Box<dyn Error>> {
        let mut maybe_client = None;
        let ola_client_config = StreamingClientConfig {
            server_port: config.ola_port(),
            ..Default::default()
        };

        // Attempt to connect to OLA 10 times.
        for i in 0..10 {
            // Don't sleep on the first iteration.
            if i > 0 {
                thread::sleep(Duration::from_secs(5));
            }

            if let Ok(ola_client) = ola::connect_with_config(ola_client_config.clone()) {
                maybe_client = Some(ola_client);
                break;
            };

            debug!("Error connecting to OLA, waiting 5 seconds and trying again.");
        }
        let client = match maybe_client {
            Some(client) => client,
            None => return Err("unable to connect to OLA".into()),
        };
        let (sender, receiver) = mpsc::channel::<DmxMessage>();

        let client_handle = thread::spawn(move || {
            Self::ola_thread(client, receiver);
        });
        let cancel_handle = CancelHandle::new();
        let universes: HashMap<String, Universe> = config
            .universes()
            .into_iter()
            .map(|config| {
                (
                    config.name().to_string(),
                    Universe::new(config, cancel_handle.clone(), sender.clone()),
                )
            })
            .collect();
        let join_handles: Vec<JoinHandle<()>> = universes
            .values()
            .map(|universe| universe.start_thread())
            .collect();

        // Initialize lighting system if config is provided
        let lighting_system =
            if let (Some(lighting_config), Some(base_path)) = (lighting_config, base_path) {
                let mut system = LightingSystem::new();
                if let Err(e) = system.load(lighting_config, base_path) {
                    eprintln!("Warning: Failed to load lighting system: {}", e);
                    None
                } else {
                    Some(system)
                }
            } else {
                None
            };

        Ok(Engine {
            dimming_speed_modifier: config.dimming_speed_modifier(),
            playback_delay: config.playback_delay()?,
            universes: universes.into_iter().collect(),
            cancel_handle,
            client_handle: Some(client_handle),
            join_handles,
            effect_engine: Arc::new(Mutex::new(EffectEngine::new())),
            lighting_system,
        })
    }

    #[cfg(test)]
    pub(crate) fn get_universe(&self, universe_name: &str) -> Option<&Universe> {
        self.universes.get(universe_name)
    }

    /// Plays the given song through the DMX interface.
    pub fn play(
        dmx_engine: Arc<Engine>,
        song: Arc<Song>,
        cancel_handle: CancelHandle,
        play_barrier: Arc<Barrier>,
    ) -> Result<(), Box<dyn Error>> {
        let span = span!(Level::INFO, "play song (dmx)");
        let _enter = span.enter();

        // No light shows in this song, so return early.
        let light_shows = song.light_shows();
        if light_shows.is_empty() {
            return Ok(());
        }

        info!(
            song = song.name(),
            duration = song.duration_string(),
            "Playing song DMX."
        );

        // Register fixtures with the effects engine if lighting system is available
        if let Err(e) = dmx_engine.register_venue_fixtures_safe() {
            eprintln!("Warning: Failed to register venue fixtures: {}", e);
        }

        // Start the effects processing loop
        let effects_handle = Self::start_effects_loop(dmx_engine.clone(), cancel_handle.clone())?;

        let (universe_names, playback_delay): (HashSet<String>, Duration) = (
            dmx_engine.universes.keys().cloned().collect(),
            dmx_engine.playback_delay,
        );

        let mut dmx_midi_sheets: HashMap<String, (MidiSheet, Vec<u8>)> = HashMap::new();
        let mut empty_barrier_counter = 0;
        for light_show in song.light_shows().iter() {
            let universe_name = light_show.universe_name();
            if !universe_names.contains(&universe_name) {
                // Keep track of the number of threads that should just wait on the play barrier.
                empty_barrier_counter += 1;
                continue;
            }

            dmx_midi_sheets.insert(
                universe_name.clone(),
                (light_show.dmx_midi_sheet()?, light_show.midi_channels()),
            );
        }

        if dmx_midi_sheets.is_empty() {
            info!(song = song.name(), "Song has no matching light shows.");
            return Ok(());
        }

        let mut join_handles: Vec<JoinHandle<()>> = dmx_midi_sheets
            .into_iter()
            .map(|(universe_name, light_show_info)| {
                let dmx_midi_sheet = light_show_info.0;
                let midi_channels = HashSet::from_iter(light_show_info.1);
                let cancel_handle = cancel_handle.clone();
                let dmx_engine = dmx_engine.clone();
                let universe_name = universe_name.clone();
                let play_barrier = play_barrier.clone();

                thread::spawn(move || {
                    let connection = DMXConnection {
                        cancel_handle: cancel_handle.clone(),
                        universe_name,
                        midi_channels,
                        dmx_engine,
                    };
                    let mut player = Player::new(
                        midi::midir::CancelableTimer::new(
                            dmx_midi_sheet.ticker,
                            cancel_handle.clone(),
                        ),
                        connection,
                    );

                    let play_finished = Arc::new(AtomicBool::new(false));

                    play_barrier.wait();
                    spin_sleep::sleep(playback_delay);
                    player.play(&dmx_midi_sheet.sheet);
                    play_finished.store(true, std::sync::atomic::Ordering::Relaxed);
                })
            })
            .collect();

        // We need to make sure we wait on each available universe, even if it shouldn't
        // be played, to get to the appropriate barrier count, which is equal to the number
        // of universes available on the song.
        (0..empty_barrier_counter)
            .map(|_| {
                let play_barrier = play_barrier.clone();
                thread::spawn(move || {
                    play_barrier.wait();
                })
            })
            .for_each(|join_handle| {
                join_handle
                    .join()
                    .expect("Empty barrier join handle should join immediately");
            });

        if cancel_handle.is_cancelled() {
            info!("DMX playback has been cancelled.");
        }

        let results: Vec<Result<(), Box<dyn Error>>> = join_handles
            .drain(..)
            .map(|join_handle| {
                if join_handle.join().is_err() {
                    return Err("Error while joining thread!".into());
                }
                Ok(())
            })
            .collect();
        for result in results.into_iter() {
            result?;
        }

        // Wait for effects loop to finish
        if let Err(e) = effects_handle.join() {
            error!("Error waiting for effects loop to stop: {:?}", e);
        }

        info!("DMX playback stopped.");

        Ok(())
    }

    /// Starts the effects processing loop for continuous effect updates
    pub fn start_effects_loop(
        dmx_engine: Arc<Engine>,
        cancel_handle: CancelHandle,
    ) -> Result<JoinHandle<()>, Box<dyn Error>> {
        let effects_handle = thread::spawn(move || {
            let mut last_update = std::time::Instant::now();
            let target_frame_time = Duration::from_secs_f64(1.0 / 44.0); // 44Hz to match Universe TARGET_HZ

            while !cancel_handle.is_cancelled() {
                let now = std::time::Instant::now();
                let elapsed = now.duration_since(last_update);

                if elapsed >= target_frame_time {
                    // Update effects engine
                    if let Err(e) = dmx_engine.update_effects() {
                        error!("Error updating effects: {}", e);
                    }
                    last_update = now;
                }

                // Sleep for a short time to prevent busy waiting
                thread::sleep(Duration::from_millis(1));
            }
        });

        Ok(effects_handle)
    }

    /// Handles an incoming MIDI event.
    pub fn handle_midi_event(&self, universe_name: String, midi_message: midly::MidiMessage) {
        match midi_message {
            midly::MidiMessage::NoteOn { key, vel } => {
                self.handle_key_velocity(universe_name, key, vel);
            }
            midly::MidiMessage::NoteOff { key, vel } => {
                self.handle_key_velocity(universe_name, key, vel);
            }
            midly::MidiMessage::ProgramChange { program } => {
                self.update_dimming(
                    universe_name,
                    Duration::from_secs_f64(
                        f64::from(program.as_int()) * self.dimming_speed_modifier,
                    ),
                );
            }
            midly::MidiMessage::Controller { controller, value } => {
                self.update_universe(
                    universe_name,
                    controller.as_int().into(),
                    value.as_int() * 2,
                    false,
                );
            }
            _ => {
                debug!(
                    midi_event = format!("{:?}", midi_message),
                    "Unrecognized MIDI event"
                );
            }
        }
    }

    /// Handles MIDI events that use a key and velocity.
    fn handle_key_velocity(&self, universe_name: String, key: u7, velocity: u7) {
        self.update_universe(
            universe_name,
            key.as_int().into(),
            velocity.as_int() * 2,
            true,
        )
    }

    // Updates the current dimming speed.
    fn update_dimming(&self, universe_name: String, dimming_duration: Duration) {
        debug!(
            dimming = dimming_duration.as_secs_f64(),
            "Dimming speed updated"
        );
        if let Some(universe) = self.universes.get(&universe_name) {
            universe.update_dim_speed(dimming_duration)
        }
    }

    /// Updates the given universe.
    fn update_universe(&self, universe_name: String, channel: u16, value: u8, dim: bool) {
        if let Some(universe) = self.universes.get(&universe_name) {
            universe.update_channel_data(channel, value, dim)
        }
    }

    /// Updates the effects engine and applies any generated commands to universes
    pub fn update_effects(&self) -> Result<(), Box<dyn std::error::Error>> {
        // Update the effects engine with a 44Hz frame time (matching Universe TARGET_HZ)
        let dt = Duration::from_secs_f64(1.0 / 44.0);
        let mut effect_engine = self.effect_engine.lock().unwrap();
        let commands = effect_engine.update(dt)?;

        // Group commands by universe
        let mut universe_commands: std::collections::HashMap<u16, Vec<(u16, u8)>> =
            std::collections::HashMap::new();
        for command in commands {
            universe_commands
                .entry(command.universe)
                .or_default()
                .push((command.channel, command.value));
        }

        // Apply effect commands to universes
        for (universe_id, commands) in universe_commands {
            // Find universe by ID (assuming universe names match the ID)
            for (universe_name, universe) in &self.universes {
                if let Ok(config_universe) = universe_name.parse::<u16>() {
                    if config_universe == universe_id {
                        universe.update_effect_commands(commands);
                        break;
                    }
                }
            }
        }

        Ok(())
    }

    /// Starts a lighting effect
    #[allow(dead_code)]
    pub fn start_effect(
        &self,
        effect: crate::lighting::EffectInstance,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut effect_engine = self.effect_engine.lock().unwrap();
        effect_engine.start_effect(effect)?;
        Ok(())
    }

    /// Stops a lighting effect
    #[allow(dead_code)]
    pub fn stop_effect(&self, effect_id: &str) {
        let mut effect_engine = self.effect_engine.lock().unwrap();
        effect_engine.stop_effect(effect_id);
    }

    /// Starts a lighting chaser
    #[allow(dead_code)]
    pub fn start_chaser(
        &self,
        chaser: crate::lighting::Chaser,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut effect_engine = self.effect_engine.lock().unwrap();
        effect_engine.start_chaser(chaser)?;
        Ok(())
    }

    /// Stops a lighting chaser
    #[allow(dead_code)]
    pub fn stop_chaser(&self, chaser_id: &str) {
        let mut effect_engine = self.effect_engine.lock().unwrap();
        effect_engine.stop_chaser(chaser_id);
    }

    /// Registers all fixtures from the current venue with the effects engine
    #[allow(dead_code)]
    pub fn register_venue_fixtures(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(lighting_system) = &self.lighting_system {
            let fixture_infos = lighting_system.get_current_venue_fixtures()?;
            let mut effect_engine = self.effect_engine.lock().unwrap();

            for fixture_info in fixture_infos {
                effect_engine.register_fixture(fixture_info);
            }
        }
        Ok(())
    }

    /// Registers fixtures for a specific logical group with the effects engine
    #[allow(dead_code)]
    pub fn register_group_fixtures(
        &mut self,
        group_name: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(lighting_system) = &mut self.lighting_system {
            let fixture_infos = lighting_system.get_group_fixtures(group_name)?;
            let mut effect_engine = self.effect_engine.lock().unwrap();

            for fixture_info in fixture_infos {
                effect_engine.register_fixture(fixture_info);
            }
        }
        Ok(())
    }

    /// Gets the lighting system (for external access)
    #[allow(dead_code)]
    pub fn lighting_system(&self) -> Option<&LightingSystem> {
        self.lighting_system.as_ref()
    }

    /// Registers all fixtures from the current venue (thread-safe version)
    pub fn register_venue_fixtures_safe(&self) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(lighting_system) = &self.lighting_system {
            let fixture_infos = lighting_system.get_current_venue_fixtures()?;
            let mut effect_engine = self.effect_engine.lock().unwrap();

            for fixture_info in fixture_infos {
                effect_engine.register_fixture(fixture_info);
            }
        }
        Ok(())
    }

    /// Sends messages to OLA.
    fn ola_thread(mut client: StreamingClient<TcpStream>, receiver: Receiver<DmxMessage>) {
        loop {
            match receiver.recv() {
                Ok(message) => {
                    if let Err(err) = client.send_dmx(message.universe, &message.buffer) {
                        error!("error sending DMX to OLA: {}", err.to_string())
                    }
                }
                Err(_) => return,
            }
        }
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.cancel_handle.cancel();

        self.join_handles.drain(..).for_each(|join_handle| {
            if join_handle.join().is_err() {
                error!("Error joining handle");
            }
        });

        self.universes.drain();

        if self
            .client_handle
            .take()
            .expect("Expected client handle")
            .join()
            .is_err()
        {
            error!("Error joining handle");
        }
    }
}

/// DMXConnection is a nodi connection that can be cancelled and will poutput to a
/// DMX interface.
struct DMXConnection {
    cancel_handle: CancelHandle,
    universe_name: String,
    midi_channels: HashSet<u8>,
    dmx_engine: Arc<Engine>,
}

impl Connection for DMXConnection {
    fn play(&mut self, event: nodi::MidiEvent) -> bool {
        if self.cancel_handle.is_cancelled() {
            return false;
        };

        if self.midi_channels.is_empty() || self.midi_channels.contains(&event.channel.as_int()) {
            self.dmx_engine
                .handle_midi_event(self.universe_name.clone(), event.message);
        }

        true
    }
}

#[cfg(test)]
mod test {
    use std::{
        collections::HashSet,
        error::Error,
        net::{Ipv4Addr, SocketAddr, TcpListener},
        sync::Arc,
    };

    use midly::num::u7;
    use nodi::{Connection, MidiEvent};

    use crate::playsync::CancelHandle;

    use super::{config, DMXConnection, Engine};

    fn create_engine() -> Result<(Arc<Engine>, CancelHandle), Box<dyn Error>> {
        let listener = TcpListener::bind(SocketAddr::new(
            std::net::IpAddr::V4(Ipv4Addr::UNSPECIFIED),
            0,
        ))?;
        let engine = Engine::new(&config::Dmx::new(
            None,
            None,
            Some(listener.local_addr()?.port()),
            vec![config::Universe::new(5, "universe1".to_string())],
            None, // lighting configuration
        ))?;
        let cancel_handle = engine.cancel_handle.clone();
        Ok((Arc::new(engine), cancel_handle))
    }

    #[test]
    fn test_connection_cancel() -> Result<(), Box<dyn Error>> {
        let (engine, cancel_handle) = create_engine()?;

        let mut connection = DMXConnection {
            cancel_handle: cancel_handle.clone(),
            universe_name: "universe1".to_string(),
            midi_channels: HashSet::new(),
            dmx_engine: engine.clone(),
        };

        // Verify the default dim speed value.
        assert_eq!(
            engine.get_universe("universe1").unwrap().get_dim_speed(),
            1.0
        );

        // No cancellation.
        assert!(connection.play(MidiEvent {
            channel: 5.into(),
            message: midly::MidiMessage::ProgramChange {
                program: u7::new(1u8)
            }
        }));

        // Verify that the universe got our command.
        assert_eq!(
            engine.get_universe("universe1").unwrap().get_dim_speed(),
            44.0
        );

        cancel_handle.cancel();

        // Cancellation.
        assert!(!connection.play(MidiEvent {
            channel: 5.into(),
            message: midly::MidiMessage::NoteOn {
                key: 0.into(),
                vel: 0.into(),
            },
        }));

        Ok(())
    }

    #[test]
    fn test_effects_integration() -> Result<(), Box<dyn Error>> {
        let (engine, _cancel_handle) = create_engine()?;

        // Register a fixture with the effects engine
        let fixture_info = crate::lighting::effects::FixtureInfo {
            name: "test_fixture".to_string(),
            universe: 1,
            address: 1,
            fixture_type: "RGBW_Par".to_string(),
            channels: {
                let mut channels = std::collections::HashMap::new();
                channels.insert("dimmer".to_string(), 1);
                channels.insert("red".to_string(), 2);
                channels.insert("green".to_string(), 3);
                channels.insert("blue".to_string(), 4);
                channels
            },
        };

        {
            let mut effect_engine = engine.effect_engine.lock().unwrap();
            effect_engine.register_fixture(fixture_info);
        }

        // Test that we can start and stop effects
        let mut parameters = std::collections::HashMap::new();
        parameters.insert("dimmer".to_string(), 0.5);

        let effect = crate::lighting::EffectInstance::new(
            "test_effect".to_string(),
            crate::lighting::EffectType::Static {
                parameters: parameters.clone(),
                duration: None,
            },
            vec!["test_fixture".to_string()],
        );

        // This should not panic
        engine.start_effect(effect).unwrap();
        engine.stop_effect("test_effect");

        Ok(())
    }

    #[test]
    fn test_lighting_system_integration() -> Result<(), Box<dyn Error>> {
        // This test verifies that the lighting system can be initialized
        // without requiring OLA connection by testing the configuration parsing

        // Create a mock lighting config
        let lighting_config = config::Lighting::new(
            Some("test_venue".to_string()),
            Some({
                let mut fixtures = std::collections::HashMap::new();
                fixtures.insert("Wash1".to_string(), "RGBW_Par @ 1:1".to_string());
                fixtures
            }),
            Some({
                let mut groups = std::collections::HashMap::new();
                let front_wash_group = crate::config::lighting::LogicalGroup::new(
                    "front_wash".to_string(),
                    vec![crate::config::lighting::GroupConstraint::AllOf(vec![
                        "wash".to_string(),
                        "front".to_string(),
                    ])],
                );
                groups.insert("front_wash".to_string(), front_wash_group);
                groups
            }),
            None, // No directories for this test
        );

        // Test that the lighting config can be created and accessed
        assert!(lighting_config.current_venue().is_some());
        assert_eq!(lighting_config.current_venue().unwrap(), "test_venue");

        // fixtures() returns HashMap directly, not Option<HashMap>
        assert_eq!(lighting_config.fixtures().len(), 1);
        assert!(lighting_config.fixtures().contains_key("Wash1"));

        // groups() returns HashMap directly, not Option<HashMap>
        assert_eq!(lighting_config.groups().len(), 1);
        assert!(lighting_config.groups().contains_key("front_wash"));

        Ok(())
    }

    #[test]
    fn test_lighting_system_without_config() -> Result<(), Box<dyn Error>> {
        // This test verifies that DMX config can be created without lighting system
        let dmx_config = config::Dmx::new(
            None,
            None,
            Some(9090),
            vec![config::Universe::new(1, "universe1".to_string())],
            None,
        );

        // Verify that the DMX config has no lighting configuration
        assert!(dmx_config.lighting().is_none());

        Ok(())
    }

    #[test]
    fn test_register_venue_fixtures_without_lighting_system() -> Result<(), Box<dyn Error>> {
        let (engine, _cancel_handle) = create_engine()?;

        // Should not panic when no lighting system is available
        engine.register_venue_fixtures_safe()?;

        Ok(())
    }

    #[test]
    fn test_effects_update_without_fixtures() -> Result<(), Box<dyn Error>> {
        let (engine, _cancel_handle) = create_engine()?;

        // Update effects with no fixtures registered - should not panic
        engine.update_effects()?;

        Ok(())
    }

    #[test]
    fn test_effects_update_with_fixtures() -> Result<(), Box<dyn Error>> {
        let (engine, _cancel_handle) = create_engine()?;

        // Register a fixture
        let fixture_info = crate::lighting::effects::FixtureInfo {
            name: "test_fixture".to_string(),
            universe: 1,
            address: 1,
            fixture_type: "RGBW_Par".to_string(),
            channels: {
                let mut channels = std::collections::HashMap::new();
                channels.insert("dimmer".to_string(), 1);
                channels.insert("red".to_string(), 2);
                channels.insert("green".to_string(), 3);
                channels.insert("blue".to_string(), 4);
                channels
            },
        };

        {
            let mut effect_engine = engine.effect_engine.lock().unwrap();
            effect_engine.register_fixture(fixture_info);
        }

        // Start an effect
        let mut parameters = std::collections::HashMap::new();
        parameters.insert("dimmer".to_string(), 0.8);
        parameters.insert("red".to_string(), 1.0);

        let effect = crate::lighting::EffectInstance::new(
            "test_effect".to_string(),
            crate::lighting::EffectType::Static {
                parameters,
                duration: None,
            },
            vec!["test_fixture".to_string()],
        );

        engine.start_effect(effect)?;

        // Update effects - should generate commands
        engine.update_effects()?;

        Ok(())
    }

    #[test]
    fn test_chaser_integration() -> Result<(), Box<dyn Error>> {
        let (engine, _cancel_handle) = create_engine()?;

        // Register a fixture
        let fixture_info = crate::lighting::effects::FixtureInfo {
            name: "test_fixture".to_string(),
            universe: 1,
            address: 1,
            fixture_type: "RGBW_Par".to_string(),
            channels: {
                let mut channels = std::collections::HashMap::new();
                channels.insert("dimmer".to_string(), 1);
                channels.insert("red".to_string(), 2);
                channels
            },
        };

        {
            let mut effect_engine = engine.effect_engine.lock().unwrap();
            effect_engine.register_fixture(fixture_info);
        }

        // Create a chaser
        let mut parameters = std::collections::HashMap::new();
        parameters.insert("dimmer".to_string(), 1.0);
        parameters.insert("red".to_string(), 1.0);

        let step_effect = crate::lighting::EffectInstance::new(
            "step_effect".to_string(),
            crate::lighting::EffectType::Static {
                parameters,
                duration: None,
            },
            vec!["test_fixture".to_string()],
        );

        let step = crate::lighting::ChaserStep {
            effect: step_effect,
            hold_time: std::time::Duration::from_millis(100),
            transition_time: std::time::Duration::from_millis(50),
            transition_type: crate::lighting::effects::TransitionType::Fade,
        };

        let chaser =
            crate::lighting::Chaser::new("test_chaser".to_string(), "Test Chaser".to_string())
                .add_step(step);

        // Start the chaser
        engine.start_chaser(chaser)?;

        // Update effects - should process chaser
        engine.update_effects()?;

        // Stop the chaser
        engine.stop_chaser("test_chaser");

        Ok(())
    }

    #[test]
    fn test_connection_midi_inclusion() -> Result<(), Box<dyn Error>> {
        let (engine, cancel_handle) = create_engine()?;

        let mut midi_channels: HashSet<u8> = HashSet::new();
        midi_channels.insert(5);
        let mut connection = DMXConnection {
            cancel_handle: cancel_handle.clone(),
            universe_name: "universe1".to_string(),
            midi_channels,
            dmx_engine: engine.clone(),
        };

        assert_eq!(
            engine.get_universe("universe1").unwrap().get_dim_speed(),
            1.0
        );

        // Valid MIDI channel.
        assert!(connection.play(MidiEvent {
            channel: 5.into(),
            message: midly::MidiMessage::ProgramChange {
                program: u7::new(1u8)
            }
        }));

        assert_eq!(
            engine.get_universe("universe1").unwrap().get_dim_speed(),
            44.0
        );

        // This will be excluded.
        assert!(connection.play(MidiEvent {
            channel: 6.into(),
            message: midly::MidiMessage::ProgramChange {
                program: u7::new(0u8)
            }
        }));

        assert_eq!(
            engine.get_universe("universe1").unwrap().get_dim_speed(),
            44.0
        );

        Ok(())
    }
}
