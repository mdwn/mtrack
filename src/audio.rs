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
use std::any::Any;
use std::sync::atomic::{AtomicU64, Ordering};
use std::{error::Error, fmt, sync::Arc};

use crate::config;
use crate::playsync::PlaybackSync;
use crate::songs::Song;
use std::collections::HashMap;

pub mod click_analysis;
pub mod confirmation;
pub mod context;
pub mod cpal;
pub mod crossfade;
pub mod format;
pub mod health;
pub mod metronome;
pub mod midi_tempo;
pub mod mixer;
pub mod mock;
pub mod monoclip;
pub mod pilot;
pub mod sample_source;
pub mod tempo_guess;
pub mod track_gains;

// Re-export the format types for backward compatibility
pub use context::PlaybackContext;
pub use cpal::AudioDeviceInfo;
pub use format::{SampleFormat, TargetFormat};

/// Global source ID counter shared by song playback and sample triggers so IDs are unique.
static SOURCE_ID_COUNTER: AtomicU64 = AtomicU64::new(1);

/// Returns the next unique source ID for the mixer. Used by both song play_from and sample engine.
pub fn next_source_id() -> u64 {
    SOURCE_ID_COUNTER.fetch_add(1, Ordering::Relaxed)
}

/// Type alias for the channel sender used to add sources to the mixer.
pub type SourceSender = crossbeam_channel::Sender<mixer::ActiveSource>;

/// Typed errors for the audio subsystem.
#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    #[error("Audio device not found: {0}")]
    DeviceNotFound(String),

    /// The device is there and will not open.
    ///
    /// Distinct from [`Self::DeviceNotFound`] because the two call for opposite
    /// responses: a device that is not there yet may be mid-enumeration or not
    /// plugged in, and waiting is exactly right; one that is sitting there
    /// refusing to open will still be refusing in an hour, and waiting is
    /// exactly wrong.
    #[error("Audio device found but could not be opened: {0}")]
    DeviceUnopenable(String),

    /// The audio config cannot describe a working device, whatever hardware
    /// turns up.
    #[error("Audio configuration error: {0}")]
    Misconfigured(String),

    #[error("Audio format mismatch: {0}")]
    FormatMismatch(String),

    #[error("Audio playback error: {0}")]
    Playback(String),

    #[error("Audio stream error: {0}")]
    Stream(String),

    #[error(transparent)]
    Other(Box<dyn Error + Send + Sync>),
}

impl From<Box<dyn Error + Send + Sync>> for AudioError {
    fn from(e: Box<dyn Error + Send + Sync>) -> Self {
        AudioError::Other(e)
    }
}

impl AudioError {
    /// Whether this failure will still be a failure however long we wait.
    ///
    /// Hardware init retries perpetually, which is right for a device that has
    /// not appeared yet and wrong for one that has appeared and refuses its
    /// configuration — the second spins at 2Hz forever and, because subsystems
    /// are joined, takes the rest of the rig down with it.
    ///
    /// Unclassified errors count as **retryable**. That is the pre-existing
    /// behaviour, so a variant nobody has thought about keeps waiting rather
    /// than newly abandoning a device that would have arrived. The cost of
    /// guessing wrong that way is a spin; the other way it is a rig that
    /// silently comes up without its audio.
    pub fn is_terminal(&self) -> bool {
        match self {
            AudioError::DeviceUnopenable(_)
            | AudioError::Misconfigured(_)
            | AudioError::FormatMismatch(_) => true,
            AudioError::DeviceNotFound(_)
            | AudioError::Playback(_)
            | AudioError::Stream(_)
            | AudioError::Other(_) => false,
        }
    }
}

pub trait Device: Any + fmt::Display + std::marker::Send + std::marker::Sync {
    /// Plays the given song through the audio interface, starting from a specific time.
    /// The `ready_tx` sender signals that setup is complete. The implementation should
    /// then wait for `clock.elapsed() > Duration::ZERO` as the "go" signal before
    /// starting playback.
    fn play_from(
        &self,
        song: Arc<Song>,
        mappings: &HashMap<String, Vec<u16>>,
        sync: PlaybackSync,
    ) -> Result<(), AudioError>;

    /// Gets the mixer for adding triggered samples.
    /// Returns None if the device doesn't support triggered samples.
    fn mixer(&self) -> Option<Arc<mixer::AudioMixer>> {
        None
    }

    /// Gets the source sender for adding triggered samples without lock contention.
    /// Returns None if the device doesn't support triggered samples.
    fn source_sender(&self) -> Option<SourceSender> {
        None
    }

    /// Returns the device's hardware sample counter, if available.
    /// Used by `PlaybackClock` to derive timing from the audio interface's oscillator.
    fn sample_counter(&self) -> Option<Arc<AtomicU64>> {
        self.mixer().map(|m| m.sample_counter())
    }

    /// Returns the device's sample rate in Hz, if available.
    fn sample_rate(&self) -> Option<u32> {
        self.mixer().map(|m| m.sample_rate())
    }

    /// Sets the player-wide metronome defaults (master volume and click
    /// sounds), applied to songs that don't override them. Devices that don't
    /// play the metronome ignore it.
    fn set_metronome_defaults(&self, _defaults: Option<config::MetronomeDefaults>) {}

    /// Liveness facts from the output callback, for devices that have one.
    /// `None` means the device cannot report — not that it is unhealthy.
    ///
    /// A device reporting a live callback and recent signal means mtrack is handing
    /// non-silent audio to the driver. It does not mean sound reached the room: a
    /// device can accept every buffer and produce nothing, and that is not
    /// detectable from here. The value of this is ruling mtrack out.
    fn output_health(&self) -> Option<health::OutputHealthSnapshot> {
        None
    }

    /// Whether this open device is the one that `name` refers to.
    ///
    /// Asked by the web UI before probing: a probe opens the device, and the
    /// device this player is holding will refuse a second open. Probing it
    /// would report "could not be opened" for the one device with minutes of
    /// positive evidence behind it, which is the worst answer available.
    ///
    /// The device answers rather than the caller comparing strings, because
    /// only the backend knows that a configured name and the name it resolved
    /// to are the same device.
    fn matches_name(&self, _name: &str) -> bool {
        false
    }

    /// How long the output callback may be silent before it counts as stopped.
    ///
    /// Devices that know their callback period size this from it; everything
    /// else gets the floor. Exposed on the trait so the status snapshot judges
    /// liveness by the same window the playback monitor does, rather than the
    /// two disagreeing about whether a device is alive.
    fn liveness_window(&self) -> std::time::Duration {
        health::LIVENESS_WINDOW
    }

    #[cfg(test)]
    fn to_mock(&self) -> Result<Arc<mock::Device>, AudioError>;
}

/// Finds a cpal input device by name, searching all available hosts.
pub(crate) fn find_input_device(name: &str) -> Result<::cpal::Device, AudioError> {
    use ::cpal::traits::{DeviceTrait, HostTrait};

    for host_id in ::cpal::available_hosts() {
        let host =
            ::cpal::host_from_id(host_id).map_err(|e| AudioError::Playback(e.to_string()))?;
        let devices = match host.input_devices() {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!(
                    host = host_id.name(),
                    error = %e,
                    "Failed to list input devices for host"
                );
                continue;
            }
        };

        for device in devices {
            let device_id = match device.id() {
                Ok(id) => id.to_string(),
                Err(_) => continue,
            };
            if device_id.trim() == name.trim() {
                return Ok(device);
            }
        }
    }

    Err(AudioError::DeviceNotFound(name.to_string()))
}

/// Lists audio devices as simple info structs for the web UI.
pub fn list_device_info() -> Result<Vec<AudioDeviceInfo>, AudioError> {
    cpal::list_device_info().map_err(|e| AudioError::Playback(e.to_string()))
}

/// Lists devices known to cpal.
///
/// Each returned device holds an open handle, and ALSA will not describe a
/// device while its siblings are held, so this list is a *subset* of what
/// actually exists and shrinks the more of it you build. It is fine for a
/// display, and wrong as a way to answer "does this device exist?" -- use
/// [`can_open_device`] for that.
pub fn list_devices() -> Result<Vec<Box<dyn Device>>, AudioError> {
    cpal::Device::list().map_err(|e| AudioError::Playback(e.to_string()))
}

/// Whether a device of this name can be resolved for opening.
///
/// Resolves it exactly as playback does but without starting an output thread,
/// so it answers the question the web UI's device picker implicitly asks --
/// "will choosing this work?" -- for one device at a time and without holding
/// any others open.
pub fn can_open_device(name: &str) -> bool {
    if name.trim().is_empty() {
        return false;
    }
    if name.starts_with("mock") {
        return true;
    }
    cpal::Device::is_findable(name)
}

/// How long to wait for the output callback to run before calling a device
/// silent. Callbacks start within milliseconds of a stream going live, so this
/// is generous rather than tuned.
const PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

/// What a device did when asked to actually stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeOutcome {
    /// The stream opened and the callback ran. As close to "this device will
    /// play" as anything host-side can get.
    Streaming { callbacks: u64 },
    /// The stream opened but the callback never ran. The device accepted the
    /// format and then produced nothing — the failure that is invisible to
    /// every other check.
    OpenedButSilent,
    /// The device could not be opened, with the reason.
    CouldNotOpen(String),
    /// The device opened but cannot report liveness, so streaming could not be
    /// confirmed. Mock devices only.
    OpenedCannotReport,
}

impl fmt::Display for ProbeOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProbeOutcome::Streaming { callbacks } => {
                write!(f, "streaming ({callbacks} callbacks served)")
            }
            ProbeOutcome::OpenedButSilent => {
                write!(f, "opened, but the output callback never ran")
            }
            ProbeOutcome::CouldNotOpen(why) => write!(f, "could not be opened: {why}"),
            ProbeOutcome::OpenedCannotReport => {
                write!(f, "opened (this device cannot report liveness)")
            }
        }
    }
}

impl ProbeOutcome {
    /// Stable wire name for this outcome.
    ///
    /// The single source for the strings the web UI switches on, so a renamed
    /// variant cannot quietly change the API. `Display` is prose for a human
    /// and is free to be reworded.
    pub fn kind(&self) -> &'static str {
        match self {
            ProbeOutcome::Streaming { .. } => "streaming",
            ProbeOutcome::OpenedButSilent => "opened_but_silent",
            ProbeOutcome::CouldNotOpen(_) => "could_not_open",
            ProbeOutcome::OpenedCannotReport => "opened_cannot_report",
        }
    }

    /// Whether the device proved it can stream.
    pub fn is_ok(&self) -> bool {
        matches!(
            self,
            ProbeOutcome::Streaming { .. } | ProbeOutcome::OpenedCannotReport
        )
    }
}

/// Opens a device, confirms its output callback runs, and closes it again.
///
/// The check `can_open_device` cannot make. Resolving a device proves its name
/// exists; it does not prove the format will be accepted or that the callback
/// will ever fire. A WING reports itself happily and then refuses to open,
/// because cpal has S24_3LE commented out of its ALSA format table — and
/// nothing short of building a stream finds that out.
///
/// Deliberately silent. No sources are added, so the mixer hands the device
/// zeroes: this confirms the path works without making a sound in a room that
/// might have an audience in it. It is the whole reason this can be run before
/// a set rather than only after one goes wrong.
///
/// Goes through `get_device`, so it exercises the same code playback does —
/// format negotiation, buffer resolution, the output thread — rather than a
/// simplified imitation that could succeed where the real thing fails.
pub fn probe_device(config: config::Audio) -> ProbeOutcome {
    let device = match get_device(Some(config)) {
        Ok(device) => device,
        Err(e) => return ProbeOutcome::CouldNotOpen(e.to_string()),
    };

    let started = std::time::Instant::now();
    loop {
        match device.output_health() {
            Some(health) if health.callbacks > 0 => {
                return ProbeOutcome::Streaming {
                    callbacks: health.callbacks,
                }
            }
            // Nothing to poll — a mock. Opening is all that can be confirmed.
            None => return ProbeOutcome::OpenedCannotReport,
            Some(_) => {}
        }
        if started.elapsed() >= PROBE_TIMEOUT {
            return ProbeOutcome::OpenedButSilent;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

/// Gets a device with the given name.
pub fn get_device(config: Option<config::Audio>) -> Result<Arc<dyn Device>, AudioError> {
    let config = match config {
        Some(config) => config,
        None => {
            // Config problems, not missing hardware: no device will ever turn
            // up to satisfy a config that names none.
            return Err(AudioError::Misconfigured(
                "there must be an audio device specified".to_string(),
            ));
        }
    };

    let device = config.device();
    if device.trim().is_empty() {
        return Err(AudioError::Misconfigured(
            "audio device name is empty; set 'device' in the audio config".to_string(),
        ));
    }
    if device.starts_with("mock") {
        return Ok(Arc::new(mock::Device::get(device)));
    };

    // Passed through rather than restated: `Device::get` is what knows whether
    // the device was absent or present-and-refusing, and flattening that here
    // is what left init unable to tell them apart.
    Ok(Arc::new(cpal::Device::get(config)?))
}

#[cfg(test)]
mod test {
    use super::*;

    /// A device that has not turned up may still turn up, so init must keep
    /// waiting for it. This is the case the perpetual retry exists for.
    #[test]
    fn a_missing_device_is_not_terminal() {
        let Err(err) = get_device(Some(config::Audio::new("definitely-not-a-real-device"))) else {
            panic!("a nonexistent device must not resolve");
        };
        assert!(
            !err.is_terminal(),
            "a device that is merely absent must stay retryable: {err}"
        );
    }

    /// Config that names no device cannot be satisfied by any hardware, so
    /// waiting for hardware is waiting for something that cannot happen.
    #[test]
    fn a_config_naming_no_device_is_terminal() {
        for config in [None, Some(config::Audio::new("   "))] {
            let Err(err) = get_device(config) else {
                panic!("an empty device name must not resolve");
            };
            assert!(
                err.is_terminal(),
                "a config error must not be retried forever: {err}"
            );
        }
    }

    /// The classification is the whole point of the split, so it is asserted
    /// per variant rather than inferred from the two paths above.
    #[test]
    fn terminal_variants_are_the_ones_a_person_must_fix() {
        assert!(AudioError::DeviceUnopenable("x".into()).is_terminal());
        assert!(AudioError::Misconfigured("x".into()).is_terminal());
        assert!(AudioError::FormatMismatch("x".into()).is_terminal());

        // Unclassified failures keep waiting: the pre-existing behaviour, so a
        // variant nobody has considered cannot newly abandon a device.
        assert!(!AudioError::DeviceNotFound("x".into()).is_terminal());
        assert!(!AudioError::Playback("x".into()).is_terminal());
        assert!(!AudioError::Stream("x".into()).is_terminal());
        assert!(!AudioError::Other("x".into()).is_terminal());
    }

    /// A device that cannot be opened must say so rather than reporting silence
    /// — the two call for different responses, and the reason is the whole
    /// value of the probe.
    #[test]
    fn probe_reports_why_a_missing_device_could_not_open() {
        let outcome = probe_device(config::Audio::new("definitely-not-a-real-device"));
        match &outcome {
            ProbeOutcome::CouldNotOpen(why) => {
                assert!(
                    why.contains("definitely-not-a-real-device"),
                    "the reason should name the device: {why}"
                );
            }
            other => panic!("expected CouldNotOpen, got {other:?}"),
        }
        assert!(!outcome.is_ok());
    }

    #[test]
    fn probe_rejects_an_empty_device_name() {
        assert!(!probe_device(config::Audio::new("")).is_ok());
    }

    /// Mock devices open but have no callback to observe, so the probe says
    /// exactly that instead of claiming a stream it never saw.
    #[test]
    fn probe_admits_when_a_device_cannot_report() {
        let outcome = probe_device(config::Audio::new("mock-probe"));
        assert_eq!(outcome, ProbeOutcome::OpenedCannotReport);
        assert!(outcome.is_ok(), "opening is still a pass");
    }

    #[test]
    fn probe_outcomes_read_clearly() {
        assert!(ProbeOutcome::Streaming { callbacks: 12 }
            .to_string()
            .contains("12 callbacks"));
        assert!(ProbeOutcome::OpenedButSilent
            .to_string()
            .contains("never ran"));
        assert!(!ProbeOutcome::OpenedButSilent.is_ok());
    }

    #[test]
    fn get_device_none_returns_error() {
        let result = get_device(None);
        // `Misconfigured`, not `DeviceNotFound`: nothing is missing from the
        // machine, the config names nothing. The distinction decides whether
        // hardware init waits for a device to appear, and no device can satisfy
        // a config that asks for none.
        match result {
            Err(AudioError::Misconfigured(msg)) => {
                assert!(msg.contains("audio device specified"))
            }
            Err(e) => panic!("expected Misconfigured, got: {}", e),
            Ok(_) => panic!("expected error for None config"),
        }
    }

    #[test]
    fn get_device_mock_returns_ok() {
        let config = config::Audio::new("mock-device");
        let result = get_device(Some(config));
        assert!(result.is_ok());
    }

    #[test]
    fn default_mixer_returns_none() {
        struct Dummy;
        impl fmt::Display for Dummy {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "dummy")
            }
        }
        impl Device for Dummy {
            fn play_from(
                &self,
                _song: Arc<Song>,
                _mappings: &HashMap<String, Vec<u16>>,
                _sync: PlaybackSync,
            ) -> Result<(), AudioError> {
                Ok(())
            }
            fn to_mock(&self) -> Result<Arc<mock::Device>, AudioError> {
                Err(AudioError::Other("not a mock".into()))
            }
        }
        let d = Dummy;
        assert!(d.mixer().is_none());
        assert!(d.source_sender().is_none());
        assert!(d.sample_counter().is_none());
        assert!(d.sample_rate().is_none());
    }

    #[test]
    fn next_source_id_increments() {
        let id1 = next_source_id();
        let id2 = next_source_id();
        assert!(id2 > id1);
    }

    #[test]
    fn get_device_mock_prefix_variants() {
        // Any device starting with "mock" should return a mock device
        for name in &["mock", "mock-test", "mock_custom", "mockDevice"] {
            let config = config::Audio::new(name);
            let result = get_device(Some(config));
            assert!(result.is_ok(), "mock device '{}' should succeed", name);
            let device = result.unwrap();
            let display = format!("{}", device);
            assert!(
                display.contains("Mock"),
                "device '{}' display should contain Mock: {}",
                name,
                display
            );
        }
    }

    #[test]
    fn get_device_display_shows_name() {
        let config = config::Audio::new("mock-hello");
        let device = get_device(Some(config)).unwrap();
        let display = format!("{}", device);
        assert!(display.contains("mock-hello"));
    }

    #[test]
    fn mock_device_clock_methods_return_none() {
        let device = mock::Device::get("mock-test");
        let d: &dyn Device = &device;
        assert!(d.sample_counter().is_none());
        assert!(d.sample_rate().is_none());
    }

    #[test]
    fn mock_device_to_mock() {
        let config = config::Audio::new("mock-test");
        let device = get_device(Some(config)).unwrap();
        let mock = device
            .to_mock()
            .expect("to_mock should work on mock devices");
        assert_eq!(format!("{}", mock), "mock-test (Mock)");
    }

    #[test]
    fn source_id_is_unique_across_calls() {
        let ids: Vec<u64> = (0..100).map(|_| next_source_id()).collect();
        for i in 1..ids.len() {
            assert!(
                ids[i] > ids[i - 1],
                "IDs should be monotonically increasing"
            );
        }
    }

    #[test]
    fn audio_error_device_not_found() {
        let e = AudioError::DeviceNotFound("test-device".to_string());
        assert!(e.to_string().contains("test-device"));
    }

    #[test]
    fn audio_error_from_boxed() {
        let boxed: Box<dyn Error + Send + Sync> = "something failed".into();
        let e: AudioError = boxed.into();
        assert!(e.to_string().contains("something failed"));
    }
}
