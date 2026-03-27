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
use std::{collections::HashMap, error::Error};

use midly::live::LiveEvent;
use serde::{Deserialize, Serialize};

use super::midi::{self, ToMidiEvent};

pub const DEFAULT_GRPC_PORT: u16 = 43234;
pub const DEFAULT_OSC_PORT: u16 = 43235;

fn default_osc_play() -> String {
    "/mtrack/play".to_string()
}
fn default_osc_prev() -> String {
    "/mtrack/prev".to_string()
}
fn default_osc_next() -> String {
    "/mtrack/next".to_string()
}
fn default_osc_stop() -> String {
    "/mtrack/stop".to_string()
}
fn default_osc_all_songs() -> String {
    "/mtrack/all_songs".to_string()
}
fn default_osc_playlist() -> String {
    "/mtrack/playlist".to_string()
}
fn default_osc_stop_samples() -> String {
    "/mtrack/samples/stop".to_string()
}
fn default_osc_section_ack() -> String {
    "/mtrack/section_ack".to_string()
}
fn default_osc_stop_section_loop() -> String {
    "/mtrack/stop_section_loop".to_string()
}
fn default_osc_loop_section() -> String {
    "/mtrack/loop_section".to_string()
}
fn default_osc_status() -> String {
    "/mtrack/status".to_string()
}
fn default_osc_playlist_current() -> String {
    "/mtrack/playlist/current".to_string()
}
fn default_osc_playlist_current_song() -> String {
    "/mtrack/playlist/current_song".to_string()
}
fn default_osc_playlist_current_song_elapsed() -> String {
    "/mtrack/playlist/current_song/elapsed".to_string()
}

/// Allows users to specify various controllers.
#[derive(Deserialize, Serialize, Clone)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Controller {
    Grpc(GrpcController),
    Midi(MidiController),
    Multi(HashMap<String, Controller>),
    Osc(Box<OscController>),
}

/// The configuration that maps MIDI events to controller messages.
#[derive(Deserialize, Serialize, Clone)]
pub struct MidiController {
    /// The MIDI event to look for to play the current song in the playlist.
    play: midi::Event,
    /// The MIDI event to look for to move the playlist to the previous item.
    prev: midi::Event,
    /// The MIDI event to look for to move the playlist to the next item.
    next: midi::Event,
    /// The MIDI event to look for to stop playback.
    stop: midi::Event,
    /// The MIDI event to look for to switch from the current playlist to an all songs playlist.
    all_songs: midi::Event,
    /// The MIDI event to look for to switch back to the current playlist.
    playlist: midi::Event,
    /// The MIDI event to acknowledge the current section (arm section loop).
    #[serde(default)]
    section_ack: Option<midi::Event>,
    /// The MIDI event to break out of the current section loop.
    #[serde(default)]
    stop_section_loop: Option<midi::Event>,
    /// Optional Morningstar controller integration for automatic preset naming.
    #[serde(default)]
    morningstar: Option<MorningstarConfig>,
}

impl MidiController {
    #[cfg(test)]
    pub fn new(
        play: midi::Event,
        prev: midi::Event,
        next: midi::Event,
        stop: midi::Event,
        all_songs: midi::Event,
        playlist: midi::Event,
    ) -> MidiController {
        MidiController {
            play,
            prev,
            next,
            stop,
            all_songs,
            playlist,
            section_ack: None,
            stop_section_loop: None,
            morningstar: None,
        }
    }
    /// Gets the play event.
    pub fn play(&self) -> Result<LiveEvent<'static>, Box<dyn Error>> {
        self.play.to_midi_event()
    }

    /// Gets the prev event.
    pub fn prev(&self) -> Result<LiveEvent<'static>, Box<dyn Error>> {
        self.prev.to_midi_event()
    }

    /// Gets the next event.
    pub fn next(&self) -> Result<LiveEvent<'static>, Box<dyn Error>> {
        self.next.to_midi_event()
    }

    /// Gets the stop event.
    pub fn stop(&self) -> Result<LiveEvent<'static>, Box<dyn Error>> {
        self.stop.to_midi_event()
    }

    /// Gets the all songs event.
    pub fn all_songs(&self) -> Result<LiveEvent<'static>, Box<dyn Error>> {
        self.all_songs.to_midi_event()
    }

    /// Gets the playlist event.
    pub fn playlist(&self) -> Result<LiveEvent<'static>, Box<dyn Error>> {
        self.playlist.to_midi_event()
    }

    /// Gets the section ack event, if configured.
    pub fn section_ack(&self) -> Result<Option<LiveEvent<'static>>, Box<dyn Error>> {
        self.section_ack
            .as_ref()
            .map(|e| e.to_midi_event())
            .transpose()
    }

    /// Gets the stop section loop event, if configured.
    pub fn stop_section_loop(&self) -> Result<Option<LiveEvent<'static>>, Box<dyn Error>> {
        self.stop_section_loop
            .as_ref()
            .map(|e| e.to_midi_event())
            .transpose()
    }

    /// Gets the optional Morningstar configuration.
    pub fn morningstar(&self) -> Option<&MorningstarConfig> {
        self.morningstar.as_ref()
    }
}

/// The configuration for the multitrack player gRPC server.
#[derive(Clone, Default, Deserialize, Serialize)]
pub struct GrpcController {
    /// The port to listen on.
    port: Option<u16>,
}

impl GrpcController {
    #[cfg(test)]
    pub fn new(port: u16) -> GrpcController {
        GrpcController { port: Some(port) }
    }

    /// Gets the port to listen on.
    pub fn port(&self) -> u16 {
        self.port.unwrap_or(DEFAULT_GRPC_PORT)
    }
}

/// The configuration for the multitrack player OSC server.
#[derive(Clone, Deserialize, Serialize)]
pub struct OscController {
    /// The port to listen on.
    #[serde(default = "default_osc_port")]
    port: u16,
    /// The broadcast addresses including the port.
    #[serde(default)]
    broadcast_addresses: Vec<String>,
    /// The OSC address to look for to play the current song in the playlist.
    #[serde(default = "default_osc_play")]
    play: String,
    /// The OSC address to look for to move the playlist to the previous item.
    #[serde(default = "default_osc_prev")]
    prev: String,
    /// The OSC address to look for to move the playlist to the next item.
    #[serde(default = "default_osc_next")]
    next: String,
    /// The OSC address to look for to stop playback.
    #[serde(default = "default_osc_stop")]
    stop: String,
    /// The OSC address to look for to switch from the current playlist to an all songs playlist.
    #[serde(default = "default_osc_all_songs")]
    all_songs: String,
    /// The OSC address to look for to switch back to the current playlist.
    #[serde(default = "default_osc_playlist")]
    playlist: String,
    /// The OSC address to look for to stop all triggered samples.
    #[serde(default = "default_osc_stop_samples")]
    stop_samples: String,
    /// The OSC address to acknowledge the current section (arm section loop).
    #[serde(default = "default_osc_section_ack")]
    section_ack: String,
    /// The OSC address to break out of the current section loop.
    #[serde(default = "default_osc_stop_section_loop")]
    stop_section_loop: String,
    /// The OSC address to loop a specific section by name (takes a string arg).
    #[serde(default = "default_osc_loop_section")]
    loop_section: String,
    /// The OSC address to broadcast to display the current player status.
    #[serde(default = "default_osc_status")]
    status: String,
    /// The OSC address to broadcast the current playlist songs.
    #[serde(default = "default_osc_playlist_current")]
    playlist_current: String,
    /// The OSC address to broadcast to display the current song on the playlist.
    #[serde(default = "default_osc_playlist_current_song")]
    playlist_current_song: String,
    /// The OSC address to broadcast to display the current song elapsed duration.
    #[serde(default = "default_osc_playlist_current_song_elapsed")]
    playlist_current_song_elapsed: String,
}

fn default_osc_port() -> u16 {
    DEFAULT_OSC_PORT
}

impl Default for OscController {
    fn default() -> Self {
        OscController {
            port: DEFAULT_OSC_PORT,
            broadcast_addresses: Vec::new(),
            play: default_osc_play(),
            prev: default_osc_prev(),
            next: default_osc_next(),
            stop: default_osc_stop(),
            all_songs: default_osc_all_songs(),
            playlist: default_osc_playlist(),
            stop_samples: default_osc_stop_samples(),
            section_ack: default_osc_section_ack(),
            stop_section_loop: default_osc_stop_section_loop(),
            loop_section: default_osc_loop_section(),
            status: default_osc_status(),
            playlist_current: default_osc_playlist_current(),
            playlist_current_song: default_osc_playlist_current_song(),
            playlist_current_song_elapsed: default_osc_playlist_current_song_elapsed(),
        }
    }
}

impl OscController {
    #[cfg(test)]
    pub fn new() -> OscController {
        OscController::default()
    }

    /// Gets the port to listen on.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Gets the broadcast addresses to broadcast OSC status messages to.
    pub fn broadcast_addresses(&self) -> &[String] {
        &self.broadcast_addresses
    }

    /// Gets the play OSC address.
    pub fn play(&self) -> &str {
        &self.play
    }

    /// Gets the prev OSC address.
    pub fn prev(&self) -> &str {
        &self.prev
    }

    /// Gets the next OSC address.
    pub fn next(&self) -> &str {
        &self.next
    }

    /// Gets the stop OSC address.
    pub fn stop(&self) -> &str {
        &self.stop
    }

    /// Gets the all songs OSC address.
    pub fn all_songs(&self) -> &str {
        &self.all_songs
    }

    /// Gets the playlist OSC address.
    pub fn playlist(&self) -> &str {
        &self.playlist
    }

    /// Gets the stop samples OSC address.
    pub fn stop_samples(&self) -> &str {
        &self.stop_samples
    }

    /// Gets the section ack OSC address.
    pub fn section_ack(&self) -> &str {
        &self.section_ack
    }

    /// Gets the stop section loop OSC address.
    pub fn stop_section_loop(&self) -> &str {
        &self.stop_section_loop
    }

    /// Gets the loop section OSC address.
    pub fn loop_section(&self) -> &str {
        &self.loop_section
    }

    /// Gets the player status.
    pub fn status(&self) -> &str {
        &self.status
    }

    /// Gets the playlist current OSC address.
    pub fn playlist_current(&self) -> &str {
        &self.playlist_current
    }

    /// Gets the playlist current song OSC address.
    pub fn playlist_current_song(&self) -> &str {
        &self.playlist_current_song
    }

    /// Gets the playlist current song elapsed OSC address.
    pub fn playlist_current_song_elapsed(&self) -> &str {
        &self.playlist_current_song_elapsed
    }
}

/// Configuration for Morningstar MIDI controller integration.
///
/// When present, mtrack will send SysEx messages to update the current bank
/// name on the controller whenever the current song changes.
#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct MorningstarConfig {
    /// The Morningstar controller model.
    model: MorningstarModel,
    /// Whether to save the name to flash (true) or keep it temporary (false).
    #[serde(default)]
    save: bool,
}

impl MorningstarConfig {
    #[cfg(test)]
    pub fn new(model: MorningstarModel, save: bool) -> MorningstarConfig {
        MorningstarConfig { model, save }
    }

    /// Returns the device model ID byte for the SysEx message.
    pub fn model_id(&self) -> u8 {
        self.model.device_id()
    }

    /// Returns whether to save to flash.
    pub fn save(&self) -> bool {
        self.save
    }

    /// Returns the required bank name length for this model.
    /// Names must be padded with spaces to exactly this length.
    pub fn name_length(&self) -> usize {
        self.model.name_length()
    }
}

/// Morningstar controller model.
#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(rename_all = "lowercase")]
pub enum MorningstarModel {
    MC3,
    MC6,
    MC8,
    #[serde(rename = "mc6pro")]
    MC6Pro,
    #[serde(rename = "mc8pro")]
    MC8Pro,
    #[serde(rename = "mc4pro")]
    MC4Pro,
    Custom(CustomModel),
}

impl MorningstarModel {
    /// Returns the SysEx device ID byte for this model.
    fn device_id(&self) -> u8 {
        match self {
            MorningstarModel::MC3 => 0x05,
            MorningstarModel::MC6 => 0x03,
            MorningstarModel::MC8 => 0x04,
            MorningstarModel::MC6Pro => 0x06,
            MorningstarModel::MC8Pro => 0x08,
            MorningstarModel::MC4Pro => 0x09,
            MorningstarModel::Custom(c) => c.model_id,
        }
    }

    /// Returns the required bank name length for this model.
    fn name_length(&self) -> usize {
        match self {
            MorningstarModel::MC3 => 16,
            MorningstarModel::MC6 | MorningstarModel::MC8 => 24,
            MorningstarModel::MC6Pro | MorningstarModel::MC8Pro | MorningstarModel::MC4Pro => 32,
            MorningstarModel::Custom(_) => 32,
        }
    }
}

/// Custom Morningstar model with explicit device ID.
#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct CustomModel {
    pub model_id: u8,
}

#[cfg(test)]
mod test {
    use std::error::Error;

    use config::{Config, File, FileFormat};

    use super::*;

    #[test]
    fn grpc_default_port() {
        let grpc = GrpcController::default();
        assert_eq!(grpc.port(), DEFAULT_GRPC_PORT);
    }

    #[test]
    fn grpc_custom_port() {
        let grpc = GrpcController::new(9999);
        assert_eq!(grpc.port(), 9999);
    }

    #[test]
    fn grpc_serde_default() -> Result<(), Box<dyn Error>> {
        let grpc: GrpcController = Config::builder()
            .add_source(File::from_str("{}", FileFormat::Yaml))
            .build()?
            .try_deserialize()?;
        assert_eq!(grpc.port(), DEFAULT_GRPC_PORT);
        Ok(())
    }

    #[test]
    fn grpc_serde_custom_port() -> Result<(), Box<dyn Error>> {
        let grpc: GrpcController = Config::builder()
            .add_source(File::from_str("port: 5000", FileFormat::Yaml))
            .build()?
            .try_deserialize()?;
        assert_eq!(grpc.port(), 5000);
        Ok(())
    }

    #[test]
    fn osc_defaults() {
        let osc = OscController::default();
        assert_eq!(osc.port(), DEFAULT_OSC_PORT);
        assert!(osc.broadcast_addresses().is_empty());
        assert_eq!(osc.play(), "/mtrack/play");
        assert_eq!(osc.prev(), "/mtrack/prev");
        assert_eq!(osc.next(), "/mtrack/next");
        assert_eq!(osc.stop(), "/mtrack/stop");
        assert_eq!(osc.all_songs(), "/mtrack/all_songs");
        assert_eq!(osc.playlist(), "/mtrack/playlist");
        assert_eq!(osc.stop_samples(), "/mtrack/samples/stop");
        assert_eq!(osc.status(), "/mtrack/status");
        assert_eq!(osc.playlist_current(), "/mtrack/playlist/current");
        assert_eq!(osc.playlist_current_song(), "/mtrack/playlist/current_song");
        assert_eq!(
            osc.playlist_current_song_elapsed(),
            "/mtrack/playlist/current_song/elapsed"
        );
    }

    #[test]
    fn osc_serde_custom_addresses() -> Result<(), Box<dyn Error>> {
        let osc: OscController = Config::builder()
            .add_source(File::from_str(
                r#"
                port: 9000
                play: /custom/play
                stop: /custom/stop
                broadcast_addresses:
                  - "192.168.1.100:9000"
                "#,
                FileFormat::Yaml,
            ))
            .build()?
            .try_deserialize()?;
        assert_eq!(osc.port(), 9000);
        assert_eq!(osc.play(), "/custom/play");
        assert_eq!(osc.stop(), "/custom/stop");
        // Non-overridden fields keep defaults.
        assert_eq!(osc.next(), "/mtrack/next");
        assert_eq!(osc.prev(), "/mtrack/prev");
        assert_eq!(
            osc.broadcast_addresses(),
            &["192.168.1.100:9000".to_string()]
        );
        Ok(())
    }

    #[test]
    fn midi_controller_events() -> Result<(), Box<dyn Error>> {
        let mc = MidiController::new(
            midi::note_on(1, 60, 127),
            midi::note_on(1, 61, 127),
            midi::note_on(1, 62, 127),
            midi::note_on(1, 63, 127),
            midi::note_on(1, 64, 127),
            midi::note_on(1, 65, 127),
        );
        // Each accessor should produce a valid LiveEvent.
        mc.play()?;
        mc.prev()?;
        mc.next()?;
        mc.stop()?;
        mc.all_songs()?;
        mc.playlist()?;
        Ok(())
    }

    #[test]
    fn controller_enum_grpc_serde() -> Result<(), Box<dyn Error>> {
        let controller: Controller = Config::builder()
            .add_source(File::from_str(
                r#"
                kind: grpc
                port: 1234
                "#,
                FileFormat::Yaml,
            ))
            .build()?
            .try_deserialize()?;
        match controller {
            Controller::Grpc(grpc) => assert_eq!(grpc.port(), 1234),
            _ => panic!("expected Grpc variant"),
        }
        Ok(())
    }

    #[test]
    fn controller_enum_osc_serde() -> Result<(), Box<dyn Error>> {
        let controller: Controller = Config::builder()
            .add_source(File::from_str(
                r#"
                kind: osc
                port: 5555
                "#,
                FileFormat::Yaml,
            ))
            .build()?
            .try_deserialize()?;
        match controller {
            Controller::Osc(osc) => assert_eq!(osc.port(), 5555),
            _ => panic!("expected Osc variant"),
        }
        Ok(())
    }
}
