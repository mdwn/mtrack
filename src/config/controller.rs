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
pub use super::controller_data::*;
use std::error::Error;

use midly::live::LiveEvent;

use super::midi::{self, ToMidiEvent};

pub const DEFAULT_GRPC_PORT: u16 = 43234;

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
            ambient_on: None,
            ambient_off: None,
            ambient_auto: None,
            cancel_auto_play: None,
            morningstar: None,
        }
    }

    /// Sets the ambient on, off and auto events.
    #[cfg(test)]
    pub fn with_ambient(
        mut self,
        on: midi::Event,
        off: midi::Event,
        auto: midi::Event,
    ) -> MidiController {
        self.ambient_on = Some(on);
        self.ambient_off = Some(off);
        self.ambient_auto = Some(auto);
        self
    }

    /// Sets the cancel auto play event.
    #[cfg(test)]
    pub fn with_cancel_auto_play(mut self, event: midi::Event) -> MidiController {
        self.cancel_auto_play = Some(event);
        self
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

    /// Gets the ambient on event, if configured.
    pub fn ambient_on(&self) -> Result<Option<LiveEvent<'static>>, Box<dyn Error>> {
        optional_event(&self.ambient_on)
    }

    /// Gets the ambient off event, if configured.
    pub fn ambient_off(&self) -> Result<Option<LiveEvent<'static>>, Box<dyn Error>> {
        optional_event(&self.ambient_off)
    }

    /// Gets the ambient auto event, if configured.
    pub fn ambient_auto(&self) -> Result<Option<LiveEvent<'static>>, Box<dyn Error>> {
        optional_event(&self.ambient_auto)
    }

    /// Gets the cancel auto play event, if configured.
    pub fn cancel_auto_play(&self) -> Result<Option<LiveEvent<'static>>, Box<dyn Error>> {
        optional_event(&self.cancel_auto_play)
    }

    /// Gets the optional Morningstar configuration.
    pub fn morningstar(&self) -> Option<&MorningstarConfig> {
        self.morningstar.as_ref()
    }
}

fn optional_event(
    event: &Option<midi::Event>,
) -> Result<Option<LiveEvent<'static>>, Box<dyn Error>> {
    event.as_ref().map(|e| e.to_midi_event()).transpose()
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

impl Default for McpController {
    fn default() -> Self {
        Self {
            port: None,
            bind_address: default_mcp_bind_address(),
            bearer_token: None,
            idle_session_timeout_secs: default_mcp_idle_timeout_secs(),
            allowed_hosts: None,
        }
    }
}

impl McpController {
    #[cfg(test)]
    pub fn new(port: u16) -> McpController {
        McpController {
            port: Some(port),
            bind_address: default_mcp_bind_address(),
            bearer_token: None,
            idle_session_timeout_secs: default_mcp_idle_timeout_secs(),
            allowed_hosts: None,
        }
    }

    #[cfg(test)]
    pub fn with_bearer_token(port: u16, token: &str) -> McpController {
        McpController {
            port: Some(port),
            bind_address: default_mcp_bind_address(),
            bearer_token: Some(token.to_string()),
            idle_session_timeout_secs: default_mcp_idle_timeout_secs(),
            allowed_hosts: None,
        }
    }

    #[cfg(test)]
    pub fn with_allowed_hosts(port: u16, hosts: Option<Vec<String>>) -> McpController {
        McpController {
            port: Some(port),
            bind_address: default_mcp_bind_address(),
            bearer_token: None,
            idle_session_timeout_secs: default_mcp_idle_timeout_secs(),
            allowed_hosts: hosts,
        }
    }

    #[cfg(test)]
    pub fn with_idle_timeout(port: u16, idle_secs: Option<u64>) -> McpController {
        McpController {
            port: Some(port),
            bind_address: default_mcp_bind_address(),
            bearer_token: None,
            idle_session_timeout_secs: idle_secs,
            allowed_hosts: None,
        }
    }

    /// Gets the port to listen on.
    pub fn port(&self) -> u16 {
        self.port.unwrap_or(DEFAULT_MCP_PORT)
    }

    /// Gets the bind address.
    pub fn bind_address(&self) -> &str {
        &self.bind_address
    }

    /// Returns the configured bearer token, if any.
    pub fn bearer_token(&self) -> Option<&str> {
        self.bearer_token.as_deref()
    }

    /// The configured `Host` allowlist, if the config set one.
    pub fn allowed_hosts(&self) -> Option<&[String]> {
        self.allowed_hosts.as_deref()
    }

    /// Idle session timeout, if eviction is enabled.
    pub fn idle_session_timeout(&self) -> Option<std::time::Duration> {
        self.idle_session_timeout_secs
            .map(std::time::Duration::from_secs)
    }
}

impl Default for OscController {
    fn default() -> Self {
        OscController {
            port: DEFAULT_OSC_PORT,
            broadcast_addresses: Vec::new(),
            play: default_osc_play(),
            pause: default_osc_pause(),
            prev: default_osc_prev(),
            next: default_osc_next(),
            stop: default_osc_stop(),
            all_songs: default_osc_all_songs(),
            playlist: default_osc_playlist(),
            stop_samples: default_osc_stop_samples(),
            section_ack: default_osc_section_ack(),
            stop_section_loop: default_osc_stop_section_loop(),
            loop_section: default_osc_loop_section(),
            seek: default_osc_seek(),
            seek_section: default_osc_seek_section(),
            ambient_on: default_osc_ambient_on(),
            ambient_off: default_osc_ambient_off(),
            ambient_auto: default_osc_ambient_auto(),
            cancel_auto_play: default_osc_cancel_auto_play(),
            track_gain: default_osc_track_gain(),
            status: default_osc_status(),
            audio_health: default_osc_audio_health(),
            playlist_current: default_osc_playlist_current(),
            playlist_current_song: default_osc_playlist_current_song(),
            playlist_current_song_elapsed: default_osc_playlist_current_song_elapsed(),
            timeline: default_osc_timeline(),
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

    /// Gets the pause OSC address.
    pub fn pause(&self) -> &str {
        &self.pause
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

    /// Gets the seek OSC address.
    pub fn seek(&self) -> &str {
        &self.seek
    }

    /// Gets the seek-to-section OSC address.
    pub fn seek_section(&self) -> &str {
        &self.seek_section
    }

    /// Gets the ambient on OSC address.
    pub fn ambient_on(&self) -> &str {
        &self.ambient_on
    }

    /// Gets the ambient off OSC address.
    pub fn ambient_off(&self) -> &str {
        &self.ambient_off
    }

    /// Gets the ambient auto OSC address.
    pub fn ambient_auto(&self) -> &str {
        &self.ambient_auto
    }

    /// Gets the cancel auto play OSC address.
    pub fn cancel_auto_play(&self) -> &str {
        &self.cancel_auto_play
    }

    /// Gets the OSC address pattern for setting an output track's gain.
    pub fn track_gain(&self) -> &str {
        &self.track_gain
    }

    /// Gets the player status.
    pub fn status(&self) -> &str {
        &self.status
    }

    /// The address the audio health verdict is broadcast to.
    pub fn audio_health(&self) -> &str {
        &self.audio_health
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

    /// Gets the numeric playback timeline and section-boundary OSC address.
    pub fn timeline(&self) -> &str {
        &self.timeline
    }
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
        assert_eq!(osc.pause(), "/mtrack/pause");
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
        assert_eq!(osc.timeline(), "/mtrack/timeline");
        assert_eq!(osc.ambient_on(), "/mtrack/ambient/on");
        assert_eq!(osc.ambient_off(), "/mtrack/ambient/off");
        assert_eq!(osc.ambient_auto(), "/mtrack/ambient/auto");
        assert_eq!(osc.cancel_auto_play(), "/mtrack/auto_play/cancel");
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
        // The ambient and auto play events are optional.
        assert!(mc.ambient_on()?.is_none());
        assert!(mc.cancel_auto_play()?.is_none());
        Ok(())
    }

    #[test]
    fn midi_controller_ambient_and_auto_play_events_deserialize() -> Result<(), Box<dyn Error>> {
        let note = |key: u8| format!("{{ type: note_on, channel: 16, key: {key}, velocity: 127 }}");
        let yaml = format!(
            "play: {}\nprev: {}\nnext: {}\nstop: {}\nall_songs: {}\nplaylist: {}\n\
             ambient_on: {}\nambient_off: {}\nambient_auto: {}\ncancel_auto_play: {}\n",
            note(0),
            note(1),
            note(2),
            note(3),
            note(4),
            note(5),
            note(10),
            note(11),
            note(12),
            note(13),
        );
        let mc: MidiController = Config::builder()
            .add_source(File::from_str(&yaml, FileFormat::Yaml))
            .build()?
            .try_deserialize()?;
        assert_eq!(
            mc.ambient_on()?,
            Some(midi::note_on(16, 10, 127).to_midi_event()?)
        );
        assert_eq!(
            mc.ambient_off()?,
            Some(midi::note_on(16, 11, 127).to_midi_event()?)
        );
        assert_eq!(
            mc.ambient_auto()?,
            Some(midi::note_on(16, 12, 127).to_midi_event()?)
        );
        assert_eq!(
            mc.cancel_auto_play()?,
            Some(midi::note_on(16, 13, 127).to_midi_event()?)
        );
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
    fn mcp_default_port() {
        let mcp = McpController::default();
        assert_eq!(mcp.port(), DEFAULT_MCP_PORT);
        assert_eq!(mcp.bind_address(), DEFAULT_MCP_BIND_ADDRESS);
    }

    #[test]
    fn mcp_serde_default() -> Result<(), Box<dyn Error>> {
        let mcp: McpController = Config::builder()
            .add_source(File::from_str("{}", FileFormat::Yaml))
            .build()?
            .try_deserialize()?;
        assert_eq!(mcp.port(), DEFAULT_MCP_PORT);
        assert_eq!(mcp.bind_address(), DEFAULT_MCP_BIND_ADDRESS);
        Ok(())
    }

    #[test]
    fn mcp_serde_custom() -> Result<(), Box<dyn Error>> {
        let mcp: McpController = Config::builder()
            .add_source(File::from_str(
                r#"
                port: 9000
                bind_address: "0.0.0.0"
                bearer_token: "shhh"
                "#,
                FileFormat::Yaml,
            ))
            .build()?
            .try_deserialize()?;
        assert_eq!(mcp.port(), 9000);
        assert_eq!(mcp.bind_address(), "0.0.0.0");
        assert_eq!(mcp.bearer_token(), Some("shhh"));
        Ok(())
    }

    #[test]
    fn mcp_serde_no_token() -> Result<(), Box<dyn Error>> {
        let mcp: McpController = Config::builder()
            .add_source(File::from_str("{}", FileFormat::Yaml))
            .build()?
            .try_deserialize()?;
        assert!(mcp.bearer_token().is_none());
        Ok(())
    }

    #[test]
    fn controller_enum_mcp_serde() -> Result<(), Box<dyn Error>> {
        let controller: Controller = Config::builder()
            .add_source(File::from_str(
                r#"
                kind: mcp
                port: 7777
                "#,
                FileFormat::Yaml,
            ))
            .build()?
            .try_deserialize()?;
        match controller {
            Controller::Mcp(mcp) => assert_eq!(mcp.port(), 7777),
            _ => panic!("expected Mcp variant"),
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
