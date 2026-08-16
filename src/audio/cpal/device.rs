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
use std::{
    collections::HashMap,
    error::Error,
    fmt,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc, Arc,
    },
    thread,
    time::Duration,
};

use cpal::traits::{DeviceTrait, HostTrait};
use tracing::{debug, error, info, span, warn, Level};

use crate::audio::format::{SampleFormat, TargetFormat};
use crate::audio::mixer::ActiveSource as MixerActiveSource;
use crate::audio::Device as AudioDevice;
use crate::config;
use crate::config::StreamBufferSize;
use crate::playsync::CancelHandle;
use crate::songs::Song;

use super::manager::OutputManager;
use super::stream::CpalOutputStreamFactory;

/// Maps a TargetFormat to the corresponding cpal::SampleFormat.
fn target_to_cpal_sample_format(format: SampleFormat, bits_per_sample: u16) -> cpal::SampleFormat {
    match (format, bits_per_sample) {
        (SampleFormat::Float, _) => cpal::SampleFormat::F32,
        (SampleFormat::Int, 16) => cpal::SampleFormat::I16,
        (SampleFormat::Int, 32) => cpal::SampleFormat::I32,
        _ => cpal::SampleFormat::I32,
    }
}

/// Returns the minimum supported output buffer size (frames) for the device and format, if known.
fn min_supported_buffer_size(
    device: &cpal::Device,
    target_format: &TargetFormat,
    channels: u16,
) -> Option<u32> {
    use cpal::SupportedBufferSize;
    let rate = target_format.sample_rate;
    let want_cpal_format =
        target_to_cpal_sample_format(target_format.sample_format, target_format.bits_per_sample);
    let configs = device.supported_output_configs().ok()?;
    let mut best_min = None::<u32>;
    for range in configs {
        if range.channels() != channels {
            continue;
        }
        if range.sample_format() != want_cpal_format {
            continue;
        }
        let (min_r, max_r) = (range.min_sample_rate(), range.max_sample_rate());
        if rate < min_r || rate > max_r {
            continue;
        }
        if let SupportedBufferSize::Range { min, max: _ } = range.buffer_size() {
            let m = *min;
            best_min = Some(best_min.map_or(m, |b| b.min(m)));
        }
    }
    best_min
}

/// Validates that the channel mappings don't exceed the device's max channel count.
fn validate_channel_count(
    mappings: &HashMap<String, Vec<u16>>,
    max_channels: u16,
    song_name: &str,
    device_name: &str,
) -> Result<(), Box<dyn Error>> {
    let num_channels = match mappings.iter().flat_map(|entry| entry.1).max() {
        Some(max) => *max,
        None => {
            return Err(format!(
                "no track mappings configured for audio device {} — map at least one track to an output channel before playing",
                device_name
            )
            .into())
        }
    };

    if max_channels < num_channels {
        return Err(format!(
            "{} channels requested for song {}, audio device {} only has {}",
            num_channels, song_name, device_name, max_channels
        )
        .into());
    }

    Ok(())
}

/// Resolves the output buffer size for the CPAL stream based on the config setting.
/// Returns `None` for default (let CPAL decide), or `Some(size)` for a fixed frame count.
fn resolve_buffer_size(
    stream_buffer_size: Option<StreamBufferSize>,
    fallback_buffer_size: u32,
    min_supported: Option<u32>,
) -> Option<u32> {
    match stream_buffer_size {
        None => Some(fallback_buffer_size),
        Some(StreamBufferSize::Default) => None,
        Some(StreamBufferSize::Min) => min_supported.or(Some(fallback_buffer_size)),
        Some(StreamBufferSize::Fixed(n)) => Some(n as u32),
    }
}

/// A supported sample format for an audio device.
#[derive(serde::Serialize, Clone, PartialEq, Eq, Hash)]
pub struct SupportedFormat {
    pub sample_format: String,
    pub bits_per_sample: u32,
}

/// Standard sample rates to check against device-reported ranges.
const STANDARD_SAMPLE_RATES: &[u32] = &[
    8000, 11025, 16000, 22050, 44100, 48000, 88200, 96000, 176400, 192000,
];

/// Serialises the device walks that silence ALSA's chatter.
///
/// `shh` redirects the process's stdout and stderr by swapping file descriptors
/// and restores them on drop, so two threads silencing at once each restore what
/// the other installed and the process loses its real stdout for good. This is
/// reachable in production -- a web UI device refresh can land while the player
/// is resolving its configured device -- and it surfaced first in the test
/// suite, where concurrent enumeration tests silenced the test harness itself
/// and it reported a failure with no output to explain it.
///
/// The window is wider than it looks: the output thread re-resolves a device
/// during stream recovery, so this is a lock rather than a hope.
static ENUMERATION_LOCK: parking_lot::Mutex<()> = parking_lot::Mutex::new(());

/// Silences ALSA's enumeration chatter for as long as the returned guard lives.
///
/// Holds [`ENUMERATION_LOCK`] alongside the redirect guards so the two cannot be
/// taken apart. The redirects are declared first so they are restored *before*
/// the lock is released.
///
/// Never call this while a guard is already alive on the same thread: the walks
/// that use it are siblings rather than nested, and `parking_lot::Mutex` is not
/// reentrant.
fn silence_enumeration() -> Result<Silenced, Box<dyn Error>> {
    let lock = ENUMERATION_LOCK.lock();
    Ok(Silenced {
        _stdout: shh::stdout()?,
        _stderr: shh::stderr()?,
        _lock: lock,
    })
}

/// Field order is drop order: redirects first, lock last.
struct Silenced {
    _stdout: shh::ShhStdout,
    _stderr: shh::ShhStderr,
    _lock: parking_lot::MutexGuard<'static, ()>,
}

/// The channel count cpal reports for a device whose real maximum it does not
/// know.
///
/// cpal clamps ALSA's reported maximum to this value -- `CHANNEL_ENUM_CAP` in
/// its `host/alsa/mod.rs` -- so a device advertising exactly this many channels
/// may have this many or any number more. ALSA plug nodes accept an unbounded
/// channel count and so always land here.
///
/// This must track cpal exactly, in both directions. Set too high and a real
/// interface's honest count is read as a floor; set too low and a plug node's
/// clamp is read as honest, which is worse -- `resolve_virtual_channel_counts`
/// would stop correcting plug nodes and mtrack would validate track mappings
/// against a number the hardware never promised. cpal 0.18 raised the cap from
/// 32 to 64 (AES10/MADI's maximum), so 32 is now a genuine capability.
const CPAL_CHANNEL_CLAMP: u16 = 64;

/// Whether a device's reported channel count is its real maximum.
///
/// Two separate reasons it might not be. Hardware reporting the clamp has that
/// many channels *or more*, so the figure is a floor. A virtual node's figure is
/// not about hardware at all -- it is whatever the plugin advertises, and ALSA's
/// software nodes advertise a number of their own choosing -- so it is never
/// known from the node's own report, whatever it happens to be. Only
/// [`resolve_virtual_channel_counts`], borrowing from a hardware sibling, can
/// make a plug node's count known.
///
/// Tying the virtual case to the clamp instead is what cpal 0.18 caught out:
/// `default` and `pulse` report 32, which *was* the clamp and so came back
/// unknown by coincidence. Once the clamp moved to 64 the same 32 read as an
/// honest count and `mtrack devices` printed `alsa:default (Channels=32)`,
/// offering ALSA's software mixer as if it were a 32-channel interface.
fn channels_are_known(virtual_node: bool, max_channels: u16) -> bool {
    !virtual_node && max_channels < CPAL_CHANNEL_CLAMP
}

/// The PCM id inside a device name, with cpal's host prefix removed.
///
/// Device names here are `cpal::DeviceId` strings, which are `"{host}:{pcm_id}"`
/// -- `alsa:hw:CARD=MAT,DEV=0`, not `hw:CARD=MAT,DEV=0`. cpal splits that on the
/// *first* colon, so the host never contains one and neither does this. Getting
/// it wrong is quiet rather than loud: `default` stops looking like a plug node
/// and every card token comes back as `hw:CARD=MAT`, so nothing pairs.
fn pcm_id(device_name: &str) -> &str {
    device_name
        .split_once(':')
        .map_or(device_name, |(_, pcm_id)| pcm_id)
}

/// Whether a device name refers to an ALSA plug/virtual node rather than
/// hardware.
///
/// These reach real hardware through ALSA's conversion plugins. That makes them
/// sometimes the *only* way to reach an interface -- a Behringer WING rack needs
/// `plughw`, because cpal's ALSA format table has `S24_3LE` commented out and so
/// finds no usable format on the raw node -- and it also means the capabilities
/// they advertise describe the plugin rather than the hardware behind it.
///
/// The list is the plugin nodes we have actually seen on a rig, and it only
/// affects wording: a plugin node not named here still reports
/// [`CPAL_CHANNEL_CLAMP`] channels, so its channel count is still marked
/// unknown.
fn is_virtual_node(device_name: &str) -> bool {
    const VIRTUAL_PREFIXES: &[&str] = &[
        "plughw",
        "plug:",
        "default",
        "sysdefault",
        "pulse",
        "dmix",
        "dsnoop",
        "null",
    ];
    let pcm_id = pcm_id(device_name);
    VIRTUAL_PREFIXES
        .iter()
        .any(|prefix| pcm_id.starts_with(prefix))
}

/// The card identity inside a device name, used to pair a plug node with the
/// hardware node behind it.
///
/// `alsa:plughw:CARD=MAT` and `alsa:hw:CARD=MAT,DEV=0` both yield `MAT`;
/// `alsa:plughw:0,0` and `alsa:hw:0,0` both yield `0`. Nodes naming no card
/// (`alsa:default`, `alsa:pulse`) yield `None`, which is correct -- there is no
/// one piece of hardware to ask.
fn card_token(device_name: &str) -> Option<String> {
    let (_, rest) = pcm_id(device_name).split_once(':')?;
    let first_field = rest.split(',').next()?;
    let token = first_field.strip_prefix("CARD=").unwrap_or(first_field);
    (!token.is_empty()).then(|| token.to_string())
}

/// The `DEV=` index inside a device name, or `None` if it names no device.
///
/// A plug node without one (`alsa:plughw:CARD=MAT`) routes to device 0, so that
/// is the subdevice whose channel count describes it.
fn dev_index(device_name: &str) -> Option<u16> {
    let (_, rest) = pcm_id(device_name).split_once(':')?;
    let second_field = rest.split(',').nth(1)?;
    second_field
        .strip_prefix("DEV=")
        .unwrap_or(second_field)
        .parse()
        .ok()
}

/// Serializable info about an audio device for the web UI.
#[derive(serde::Serialize)]
pub struct AudioDeviceInfo {
    pub name: String,
    pub max_channels: u16,
    pub host_name: String,
    pub supported_sample_rates: Vec<u32>,
    pub supported_formats: Vec<SupportedFormat>,
    /// Whether this is an ALSA plug/virtual node. Such a node is a legitimate
    /// and sometimes mandatory choice, so this labels it rather than hiding it.
    pub virtual_node: bool,
    /// Whether `max_channels` is the device's real maximum. False when the
    /// count is cpal's clamp and no hardware node was available to resolve it,
    /// in which case the true figure is `max_channels` or more.
    pub channels_known: bool,
}

impl AudioDeviceInfo {
    /// How to describe the channel count without overstating what is known.
    pub fn channels_display(&self) -> String {
        if self.channels_known {
            self.max_channels.to_string()
        } else if self.virtual_node {
            "virtual".to_string()
        } else {
            format!("{}+", self.max_channels)
        }
    }
}

impl fmt::Display for AudioDeviceInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} (Channels={}) ({})",
            self.name,
            self.channels_display(),
            self.host_name
        )
    }
}

/// Maps a cpal SampleFormat to mtrack's (sample_format, bits_per_sample) representation.
fn map_cpal_format(fmt: cpal::SampleFormat) -> SupportedFormat {
    let (sample_format, bits_per_sample) = if fmt.is_float() {
        ("float", fmt.bits_per_sample())
    } else {
        ("int", fmt.bits_per_sample())
    };
    SupportedFormat {
        sample_format: sample_format.to_string(),
        bits_per_sample,
    }
}

/// The output configurations a device reports, or `None` if it reports none
/// usable for output.
///
/// The single point where "does this device count as an output device?" is
/// decided, because it used to be decided twice. Device enumeration happens on
/// two paths -- [`list_device_info`] for the web UI's picker and
/// [`Device::list_cpal_devices`] for actually opening one -- and they disagreed:
/// the listing path called `supported_output_configs()` once and iterated that,
/// while the opening path called it, threw the result away, and called it a
/// second time to iterate. On an exclusive ALSA device the second call can come
/// back empty, leaving `max_channels` at zero so the device was dropped from the
/// openable list while remaining in the picker.
///
/// The result: the web UI offered 19 devices where only 8 could be opened, and
/// choosing one of the other 11 failed with "no device found with name ..."
/// (#357). Both paths now go through here, so they cannot drift apart again.
fn output_configs(device: &cpal::Device) -> Option<Vec<cpal::SupportedStreamConfigRange>> {
    let configs: Vec<_> = device.supported_output_configs().ok()?.collect();
    let max_channels = configs.iter().map(|c| c.channels()).max().unwrap_or(0);
    // Zero channels means there is nothing to play out of, whatever else the
    // device claims to support.
    (max_channels > 0).then_some(configs)
}

/// The highest channel count among a device's output configurations.
fn max_output_channels(configs: &[cpal::SupportedStreamConfigRange]) -> u16 {
    configs.iter().map(|c| c.channels()).max().unwrap_or(0)
}

/// One output device, resolved by name.
pub(super) struct ResolvedOutputDevice {
    /// The device id it was matched on, untrimmed.
    pub(super) name: String,
    pub(super) host_id: cpal::HostId,
    pub(super) device: cpal::Device,
    pub(super) configs: Vec<cpal::SupportedStreamConfigRange>,
}

/// Scans every host for one output device by name, holding no others open.
///
/// Deliberately not `list_cpal_devices().find(...)`. Every `Device` in that list
/// holds an open ALSA handle, and ALSA will not describe devices while its
/// siblings are held: building the full list makes the list *shrink*. Measured on
/// the test rig, in one process, alternating the two enumeration paths: 19
/// devices, then 8, then 7, then 3. So a search over that list could fail to find
/// a device the web UI had legitimately advertised, and the operator saw "no
/// device found with name ..." for a device that was right there in the dropdown
/// (#357).
///
/// Scanning and keeping only the match holds exactly one handle, which is the
/// same shape as [`crate::audio::find_input_device`].
pub(super) fn resolve_output_device(
    name: &str,
) -> Result<Option<ResolvedOutputDevice>, Box<dyn Error>> {
    // Suppress noisy output here.
    let _silenced = silence_enumeration()?;

    for host_id in cpal::available_hosts() {
        let host_devices = match cpal::host_from_id(host_id)?.devices() {
            Ok(host_devices) => host_devices,
            Err(e) => {
                error!(
                    err = e.to_string(),
                    host = host_id.name(),
                    "Unable to list devices for host"
                );
                continue;
            }
        };

        for device in host_devices {
            // Name first: a non-match is dropped here, before its configurations
            // are queried and before anything is built from it, so its handle is
            // released immediately.
            let device_id = match device.id() {
                Ok(id) => id.to_string(),
                Err(_) => continue,
            };
            if device_id.trim() != name.trim() {
                continue;
            }

            let Some(configs) = output_configs(&device) else {
                debug!(
                    device_name = %device_id,
                    "Device matched by name but reports no output configurations"
                );
                continue;
            };

            return Ok(Some(ResolvedOutputDevice {
                name: device_id,
                host_id,
                device,
                configs,
            }));
        }
    }

    Ok(None)
}

/// Lists audio devices as simple info structs (no trait objects).
pub fn list_device_info() -> Result<Vec<AudioDeviceInfo>, Box<dyn Error>> {
    // Suppress noisy output here.
    let _silenced = silence_enumeration()?;

    let mut infos: Vec<AudioDeviceInfo> = Vec::new();
    for host_id in cpal::available_hosts() {
        let host_devices = match cpal::host_from_id(host_id)?.devices() {
            Ok(d) => d,
            Err(_) => continue,
        };
        for device in host_devices {
            let Some(output_configs) = output_configs(&device) else {
                continue;
            };
            let max_channels = max_output_channels(&output_configs);

            let mut sample_rates = std::collections::BTreeSet::new();
            let mut formats = std::collections::BTreeSet::new();

            for cfg in output_configs {
                let min_rate = cfg.min_sample_rate();
                let max_rate = cfg.max_sample_rate();
                for &rate in STANDARD_SAMPLE_RATES {
                    if rate >= min_rate && rate <= max_rate {
                        sample_rates.insert(rate);
                    }
                }

                let mapped = map_cpal_format(cfg.sample_format());
                formats.insert((mapped.sample_format.clone(), mapped.bits_per_sample));
            }
            if max_channels > 0 {
                if let Ok(id) = device.id() {
                    let name = id.to_string();
                    let virtual_node = is_virtual_node(&name);
                    infos.push(AudioDeviceInfo {
                        name,
                        max_channels,
                        host_name: host_id.name().to_string(),
                        supported_sample_rates: sample_rates.into_iter().collect(),
                        supported_formats: formats
                            .into_iter()
                            .map(|(sample_format, bits_per_sample)| SupportedFormat {
                                sample_format,
                                bits_per_sample,
                            })
                            .collect(),
                        virtual_node,
                        channels_known: channels_are_known(virtual_node, max_channels),
                    });
                }
            }
        }
    }
    resolve_virtual_channel_counts(&mut infos);
    infos.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(infos)
}

/// Fills in plug nodes' channel counts from the hardware behind them.
///
/// Asking a plug node directly only ever yields [`CPAL_CHANNEL_CLAMP`], because
/// it accepts any channel count -- but `plughw:CARD=MAT` plays out of the same
/// eight outputs as `hw:CARD=MAT,DEV=0`, which reports honestly. So take the
/// count from the hardware sibling wherever there is one.
///
/// This costs no extra enumeration and holds no extra handles: every count used
/// here was already collected by the walk in [`list_device_info`]. Where there
/// is no sibling to consult -- `default` and `pulse` name no card, and a WING's
/// raw node reports no cpal-known format and so never appears at all -- the
/// count stays marked unknown rather than being guessed at. cpal exposes channel
/// counts only inside `SupportedStreamConfigRange`, so a device it cannot
/// format-match yields no channel information to borrow.
fn resolve_virtual_channel_counts(infos: &mut [AudioDeviceInfo]) {
    // (card, subdevice) -> real channel count, because subdevices on one card
    // are not interchangeable. An Intel HDA card exposes `hw:CARD=PCH,DEV=0`
    // (analog, 2ch) beside `hw:CARD=PCH,DEV=3` (HDMI, 8ch), and every
    // `plughw:CARD=PCH,DEV=n` in front of them reports the same clamped maximum.
    let mut by_subdevice: HashMap<(String, u16), u16> = HashMap::new();
    // Card identity -> (subdevice index, channels) for the lowest subdevice,
    // which is where a plug node naming no `DEV=` routes.
    let mut lowest_subdevice: HashMap<String, (u16, u16)> = HashMap::new();

    for info in infos.iter() {
        if info.virtual_node || !info.channels_known {
            continue;
        }
        let Some(card) = card_token(&info.name) else {
            continue;
        };
        let dev = dev_index(&info.name).unwrap_or(0);
        by_subdevice.insert((card.clone(), dev), info.max_channels);
        lowest_subdevice
            .entry(card)
            .and_modify(|existing| {
                if dev < existing.0 {
                    *existing = (dev, info.max_channels);
                }
            })
            .or_insert((dev, info.max_channels));
    }

    for info in infos.iter_mut() {
        if info.channels_known || !info.virtual_node {
            continue;
        }
        let Some(card) = card_token(&info.name) else {
            continue;
        };
        // A plug node that names a subdevice routes to *that* one. Looking up by
        // card alone handed `plughw:CARD=PCH,DEV=3` the analog node's 2 channels
        // and marked it known — a confidently wrong number, which is the one
        // outcome this whole labelling exists to avoid.
        let resolved = match dev_index(&info.name) {
            Some(dev) => by_subdevice.get(&(card, dev)).copied(),
            None => lowest_subdevice.get(&card).map(|(_, channels)| *channels),
        };
        // No sibling for that subdevice means no honest answer, so it stays
        // unknown rather than falling back to a different subdevice's count.
        if let Some(channels) = resolved {
            info.max_channels = channels;
            info.channels_known = true;
        }
    }
}

/// How long a built hint is reused before being rebuilt.
///
/// `Player::retry_until_ready` retries a missing device twice a second, forever,
/// so an uncached hint would walk every ALSA device at that rate -- about 10ms
/// for the three devices on a dev box, and the test rig has nineteen -- at
/// exactly the moment a rig is trying to come up. The hint is advisory text and
/// nothing decides anything from it, so a few seconds stale costs nothing: what
/// actually resolves a device is [`Device::find_cpal_device`], which is not
/// cached and still sees a device the instant it appears.
const AVAILABLE_HINT_TTL: Duration = Duration::from_secs(10);

/// The last built hint and when it was built.
static AVAILABLE_HINT_CACHE: parking_lot::Mutex<Option<(std::time::Instant, String)>> =
    parking_lot::Mutex::new(None);

/// The names an operator could have used instead, appended to the error raised
/// when the configured device is not one of them.
///
/// Cached for [`AVAILABLE_HINT_TTL`]; holding the cache lock across the rebuild
/// also collapses a herd of simultaneous failures into one walk.
fn available_hint() -> String {
    let mut cache = AVAILABLE_HINT_CACHE.lock();
    if let Some((built_at, hint)) = cache.as_ref() {
        if built_at.elapsed() < AVAILABLE_HINT_TTL {
            return hint.clone();
        }
    }

    let hint = build_available_hint();
    *cache = Some((std::time::Instant::now(), hint.clone()));
    hint
}

/// Builds the hint, unconditionally.
///
/// Built from [`list_device_info`], which holds no handles. Listing through
/// [`Device::list`] here would be self-defeating: building those devices is what
/// makes the list shrink, so the suggestion could omit a device that is present
/// and would have worked.
fn build_available_hint() -> String {
    match list_device_info() {
        Ok(infos) if infos.is_empty() => "; no audio output devices were found".to_string(),
        Ok(infos) => format!(
            "; available devices: {}",
            infos
                .iter()
                .map(|info| info.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Err(e) => format!(
            "; additionally, available devices could not be listed: {}",
            e
        ),
    }
}

/// A small wrapper around a cpal::Device. Used for storing some extra
/// data that makes multitrack playing more convenient.
pub struct Device {
    /// The name of the device.
    name: String,
    /// Controls how long to wait before playback of an audio file starts.
    playback_delay: Duration,
    /// The maximum number of channels the device supports.
    max_channels: u16,
    /// The host ID of the device.
    host_id: cpal::HostId,
    /// The underlying cpal device.
    device: cpal::Device,
    /// The target format for this device.
    target_format: TargetFormat,
    /// The output stream manager for continuous playback.
    output_manager: Arc<OutputManager>,
    /// Audio configuration for buffering and performance tuning.
    audio_config: config::Audio,
    /// The output buffer size actually resolved for the stream, in frames.
    /// `None` when the backend was left to choose and did not say, which is what
    /// `BufferSize::Default` gives. Used only to size the liveness window.
    output_buffer_size: Option<u32>,
    /// Player-wide metronome defaults, set at hardware init.
    metronome_defaults: parking_lot::Mutex<Option<config::MetronomeDefaults>>,
}

impl fmt::Display for Device {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} (Channels={}) ({})",
            self.name,
            self.max_channels,
            self.host_id.name()
        )
    }
}

impl Device {
    /// Lists cpal devices and produces the Device trait.
    pub fn list() -> Result<Vec<Box<dyn AudioDevice>>, Box<dyn Error>> {
        Ok(Device::list_cpal_devices()?
            .into_iter()
            .map(|device| {
                let device: Box<dyn AudioDevice> = Box::new(device);
                device
            })
            .collect())
    }

    /// Lists cpal devices.
    fn list_cpal_devices() -> Result<Vec<Device>, Box<dyn Error>> {
        // Suppress noisy output here.
        let _silenced = silence_enumeration()?;

        // Enumerate first, and build nothing while doing it.
        //
        // Constructing the placeholder `OutputManager` inside this walk reserves
        // audio resources partway through it, and ALSA then refuses to describe
        // the devices not yet visited: `supported_output_configs()` comes back
        // empty for them, they are dropped, and the list ends up a subset of
        // what the web UI's picker (which only queries) advertises. On the test
        // rig that was 8 devices against 19, and choosing one of the missing 11
        // failed with "no device found with name ..." (#357).
        let mut found: Vec<(cpal::HostId, cpal::Device, u16)> = Vec::new();
        for host_id in cpal::available_hosts() {
            let host_devices = match cpal::host_from_id(host_id)?.devices() {
                Ok(host_devices) => host_devices,
                Err(e) => {
                    error!(
                        err = e.to_string(),
                        host = host_id.name(),
                        "Unable to list devices for host"
                    );
                    continue;
                }
            };

            for device in host_devices {
                let Some(configs) = output_configs(&device) else {
                    continue;
                };
                found.push((host_id, device, max_output_channels(&configs)));
            }
        }

        // Now build. The manager here is a placeholder that `get` replaces, so
        // it holds nothing anyone depends on -- it just must not exist while the
        // enumeration above is still running.
        let mut devices: Vec<Device> = Vec::new();
        for (host_id, device, max_channels) in found {
            let default_format = TargetFormat::new(44100, SampleFormat::Int, 32)?;
            let temp_output_manager = Arc::new(OutputManager::new(
                max_channels,
                default_format.sample_rate,
            )?);

            devices.push(Device {
                name: device.id()?.to_string(),
                playback_delay: Duration::ZERO,
                max_channels,
                host_id,
                device,
                target_format: default_format,
                output_manager: temp_output_manager,
                audio_config: config::Audio::new("default"), // Default config for listing
                metronome_defaults: parking_lot::Mutex::new(None),
                output_buffer_size: None,
            })
        }

        devices.sort_by_key(|device| device.name.to_string());
        Ok(devices)
    }

    /// Whether a device of this name can be located for opening.
    ///
    /// The cheap half of [`Device::get`]: it resolves the device the same way
    /// but starts no output thread, so it can be asked about every advertised
    /// device without disturbing a rig.
    pub fn is_findable(name: &str) -> bool {
        matches!(Device::find_cpal_device(name), Ok(Some(_)))
    }

    /// Finds one output device by name and wraps it in a `Device`.
    ///
    /// See [`resolve_output_device`] for why this scans rather than searching
    /// `list_cpal_devices()`.
    fn find_cpal_device(name: &str) -> Result<Option<Device>, Box<dyn Error>> {
        let Some(resolved) = resolve_output_device(name)? else {
            return Ok(None);
        };

        let max_channels = max_output_channels(&resolved.configs);
        let default_format = TargetFormat::new(44100, SampleFormat::Int, 32)?;
        let output_manager = Arc::new(OutputManager::new(
            max_channels,
            default_format.sample_rate,
        )?);

        Ok(Some(Device {
            name: resolved.name,
            playback_delay: Duration::ZERO,
            max_channels,
            host_id: resolved.host_id,
            device: resolved.device,
            target_format: default_format,
            output_manager,
            audio_config: config::Audio::new("default"),
            metronome_defaults: parking_lot::Mutex::new(None),
            output_buffer_size: None,
        }))
    }

    /// Gets the given cpal device.
    ///
    /// Note the `available_hint()` on the failure path: this fires while the
    /// player is coming up, and a bare "not found" leaves the operator nothing
    /// to act on at the worst possible moment.
    pub fn get(config: config::Audio) -> Result<Device, crate::audio::AudioError> {
        let name = config.device().to_string();
        debug!(
            device_name = %name,
            device_name_len = name.len(),
            device_name_trimmed = %name.trim(),
            "Searching for audio device"
        );
        // Enumeration failing is neither of the two cases below: nothing is
        // known about the device yet, so it stays retryable.
        let found = Device::find_cpal_device(&name)
            .map_err(|e| crate::audio::AudioError::Playback(e.to_string()))?;
        match found {
            Some(device) => {
                let resolved = device.name.clone();
                // Say which of the two situations this is. `retry_until_ready`
                // re-runs this twice a second forever and logs the same line each
                // time, so without the distinction an operator cannot tell a
                // device that is still enumerating — which the retry picks up on
                // its own, and which is why the retry is perpetual — from one
                // that is sitting right there refusing to open, and will still be
                // refusing in an hour. The latter is usually a sample rate,
                // format or buffer size the interface won't accept, or another
                // process holding it exclusively; either way it needs a person.
                Device::open(device, config).map_err(|e| {
                    // No "audio device" prefix: the retry loop and the AudioError
                    // wrapper both add one already, and a line that says it three
                    // times before saying anything is harder to read at 2Hz.
                    crate::audio::AudioError::DeviceUnopenable(format!(
                        "'{resolved}' was found but could not be opened: {e}"
                    ))
                })
            }
            None => Err(crate::audio::AudioError::DeviceNotFound(format!(
                "no device found with name {}{}",
                name,
                available_hint()
            ))),
        }
    }

    /// Configures a resolved device and starts its output stream.
    ///
    /// Split from [`Device::get`] so every failure past the point where the
    /// device was located shares one "found it, couldn't open it" message.
    fn open(mut device: Device, config: config::Audio) -> Result<Device, Box<dyn Error>> {
        device.playback_delay = config.playback_delay()?;

        device.target_format = TargetFormat::new(
            config.sample_rate(),
            config.sample_format()?,
            config.bits_per_sample(),
        )?;

        // Initialize the output manager
        let mut output_manager =
            OutputManager::new(device.max_channels, device.target_format.sample_rate)?;

        // Resolve stream buffer size for CPAL (default / min / fixed)
        let min_size =
            min_supported_buffer_size(&device.device, &device.target_format, device.max_channels);
        let output_buffer_size = resolve_buffer_size(
            config.stream_buffer_size(),
            config.buffer_size() as u32,
            min_size,
        );
        if let (Some(StreamBufferSize::Min), Some(s)) =
            (config.stream_buffer_size(), output_buffer_size)
        {
            if min_size.is_some() {
                // Report the period in milliseconds as well as frames. "12
                // frames" is what the MAT's minimum resolves to on the test rig
                // and it means nothing on its own; "0.2ms" is immediately legible
                // as the callback deadline it actually is. `min` reads as a
                // recommendation, so the number behind it should not need
                // arithmetic to interpret.
                let period_ms = crate::audio::health::callback_period(
                    Some(s),
                    device.target_format.sample_rate,
                )
                .map(|p| p.as_secs_f64() * 1000.0)
                .unwrap_or(0.0);
                info!(
                    stream_buffer_size = s,
                    period_ms = format!("{period_ms:.2}"),
                    "Using minimum supported stream buffer size — lowest latency, \
                     and the least tolerance for scheduling jitter"
                );
            }
        }

        // Start the output thread with resolved buffer size
        let factory = Box::new(CpalOutputStreamFactory::new(
            device.device.clone(),
            device.name.clone(),
            device.target_format.clone(),
            output_buffer_size,
        ));
        output_manager.start_output_thread(factory)?;

        device.output_manager = Arc::new(output_manager);
        device.audio_config = config;
        device.output_buffer_size = output_buffer_size;

        Ok(device)
    }
}

/// Constructs a `MixerActiveSource` and its finish flag.
///
/// Every call site in `play_from` follows the same pattern: allocate a source ID,
/// create an `is_finished` flag, build the struct, and hand back the flag for
/// monitoring. This helper captures that boilerplate.
/// Finds the section the playhead currently sits inside, if any, resolved to
/// absolute time bounds via the song's beat grid.
///
/// This is the loop-back candidate: whichever section the performer is hearing
/// could be armed at any moment, and arming loops back to its `start_time`. The
/// audio monitor uses this to prewarm the loop-back sources speculatively, so
/// they are ready the instant the loop is armed — even on the last beat of the
/// section — rather than incurring a file-open/seek/warmup stall at the first
/// boundary.
fn current_playhead_section(
    song: &Song,
    elapsed: Duration,
) -> Option<crate::player::SectionBounds> {
    song.sections().iter().find_map(|s| {
        let (start_time, end_time) = song.resolve_section(&s.name)?;
        (elapsed >= start_time && elapsed < end_time).then(|| crate::player::SectionBounds {
            name: s.name.clone(),
            start_time,
            end_time,
        })
    })
}

fn build_active_source(
    source: Box<dyn crate::audio::sample_source::ChannelMappedSampleSource + Send + Sync>,
    track_mappings: &HashMap<String, Vec<u16>>,
    cancel_handle: &CancelHandle,
    gain_envelope: Option<Arc<crate::audio::crossfade::GainEnvelope>>,
) -> (MixerActiveSource, Arc<AtomicBool>) {
    let id = crate::audio::next_source_id();
    let cached_source_channel_count = source.source_channel_count();
    let is_finished = Arc::new(AtomicBool::new(false));
    let flag = is_finished.clone();
    // Pre-allocate the cancel slot so the section-loop monitor can schedule a
    // sample-accurate cut on an already-playing source via
    // `AudioMixer::set_cancel_at_sample`. The mixer treats 0 as "no cancel".
    let active = MixerActiveSource {
        id,
        source,
        track_mappings: track_mappings.clone(),
        channel_mappings: Vec::new(),
        cached_source_channel_count,
        cancel_handle: cancel_handle.clone(),
        is_finished,
        start_at_sample: None,
        cancel_at_sample: Some(Arc::new(std::sync::atomic::AtomicU64::new(0))),
        gain: Default::default(),
        gain_envelope,
    };
    (active, flag)
}

impl AudioDevice for Device {
    fn output_health(&self) -> Option<crate::audio::health::OutputHealthSnapshot> {
        Some(self.output_manager.health())
    }

    /// Sized from the resolved buffer, so a rig configured with an enormous one
    /// isn't judged against a window shorter than a single callback.
    fn liveness_window(&self) -> Duration {
        crate::audio::health::liveness_window(crate::audio::health::callback_period(
            self.output_buffer_size,
            self.target_format.sample_rate,
        ))
    }

    /// Matches on either the name this device was configured with or the name
    /// it resolved to.
    ///
    /// The two are routinely different — `default` resolves to something with a
    /// card in its name — and the caller may hold either. The config editor
    /// shows the configured string, so that is the usual hit; the resolved name
    /// is what someone gets if they paste from the device list instead.
    fn matches_name(&self, name: &str) -> bool {
        let wanted = name.trim();
        !wanted.is_empty()
            && (wanted.eq_ignore_ascii_case(self.audio_config.device().trim())
                || wanted.eq_ignore_ascii_case(self.name.trim()))
    }

    fn set_metronome_defaults(&self, defaults: Option<config::MetronomeDefaults>) {
        *self.metronome_defaults.lock() = defaults;
    }

    /// Play the given song through the audio device, starting from a specific time.
    fn play_from(
        &self,
        song: Arc<Song>,
        mappings: &HashMap<String, Vec<u16>>,
        sync: crate::playsync::PlaybackSync,
    ) -> Result<(), crate::audio::AudioError> {
        let crate::playsync::PlaybackSync {
            cancel_handle,
            mut ready_tx,
            clock,
            start_time,
            loop_control,
        } = sync;
        let crate::playsync::LoopControl {
            loop_break,
            active_section,
            section_loop_break,
            loop_time_consumed,
        } = loop_control;
        let span = span!(Level::INFO, "play song (cpal)");
        let _enter = span.enter();

        let is_transcoded = song.needs_transcoding(&self.target_format);
        info!(
            format = if song.sample_format() == SampleFormat::Float {
                "float"
            } else {
                "int"
            },
            device = self.name,
            song = song.name(),
            duration = song.duration_string(),
            transcoded = is_transcoded,
            "Playing song."
        );

        validate_channel_count(mappings, self.max_channels, song.name(), &self.name)
            .map_err(|e| crate::audio::AudioError::Playback(e.to_string()))?;

        ready_tx.send();

        clock.wait_for_start_or_cancel(&cancel_handle);
        if cancel_handle.is_cancelled() {
            return Ok(());
        }

        spin_sleep::sleep(self.playback_delay);

        // Build playback context (format, buffer size, shared pool) for source creation.
        let buffer_threads = self.audio_config.buffer_threads();
        let buffer_fill_pool =
            match crate::audio::sample_source::BufferFillPool::new(buffer_threads) {
                Ok(pool) => Some(Arc::new(pool)),
                Err(e) => {
                    error!(
                        error = %e,
                        threads = buffer_threads,
                        "Failed to create BufferFillPool, falling back to unbuffered song sources"
                    );
                    None
                }
            };

        let playback_context = crate::audio::PlaybackContext::new(
            self.target_format.clone(),
            self.audio_config.buffer_size(),
            buffer_fill_pool,
            self.audio_config.resampler(),
        )
        .with_metronome_defaults(self.metronome_defaults.lock().clone());

        // Create channel mapped sources for each track in the song, starting from start_time.
        let channel_mapped_sources = song
            .create_channel_mapped_sources_from(&playback_context, start_time, mappings)
            .map_err(|e| crate::audio::AudioError::Playback(e.to_string()))?;

        // Add all sources to the output manager
        if channel_mapped_sources.is_empty() {
            return Err(crate::audio::AudioError::Playback(
                "No sources found in song".to_string(),
            ));
        }

        // If there are already sources in the mixer (fading out from a previous
        // song), apply a fade-in envelope to the new sources for a smooth crossfade.
        let has_existing_sources = {
            let sources = self.output_manager.mixer.get_active_sources();
            let guard = sources.read();
            !guard.is_empty()
        };
        // Create sources and track their finish flags (no locks needed for monitoring)
        let mut source_finish_flags = Vec::new();

        let song_crossfade_envelope = if has_existing_sources {
            let cs = crate::audio::crossfade::default_crossfade_samples(
                self.output_manager.mixer.sample_rate(),
            );
            // Linear is fine for ≤5ms crossfades — perceptual difference
            // from EqualPower is inaudible at this duration, and Linear
            // is cheaper (no trig).
            Some(Arc::new(crate::audio::crossfade::GainEnvelope::fade_in(
                cs,
                crate::audio::crossfade::CrossfadeCurve::Linear,
            )))
        } else {
            None
        };

        for source in channel_mapped_sources.into_iter() {
            let (active_source, flag) = build_active_source(
                source,
                mappings,
                &cancel_handle,
                song_crossfade_envelope.clone(),
            );
            source_finish_flags.push(flag);
            self.output_manager
                .add_source(active_source)
                .map_err(|e| crate::audio::AudioError::Playback(e.to_string()))?;
        }

        // Startup skew between the clock epoch and the moment the original
        // source actually goes live in the mixer. `clock.start()` is called by
        // the player *before* this function builds and warms its sources, so the
        // free-running mixer counter has already advanced by the time the audio
        // is added. The clock's elapsed reading here is exactly that skew: the
        // original source plays this many samples behind what the clock reports,
        // for the whole song.
        //
        // Section-loop boundaries are computed from the clock
        // (`loop_boundary_sample`), and the original source is the *outgoing*
        // side of the first loop handoff while every loop source is pinned to
        // the sample counter. So without correction the first handoff lands
        // `skew` samples early (the section is cut short) while later
        // counter-pinned `N -> N` handoffs are seamless. Adding the skew to
        // every computed boundary realigns it to the audio content rather than
        // the clock epoch; loop sources started at the corrected boundaries
        // inherit the same alignment, so all handoffs — first included — stay
        // seamless. (The resampler group delay is symmetric between the outgoing
        // and incoming sources and cancels, so it needs no correction.)
        let audio_clock_skew_samples = (clock.elapsed().as_secs_f64()
            * self.output_manager.mixer.sample_rate() as f64)
            .round() as u64;
        debug!(
            audio_clock_skew_samples,
            skew_us = clock.elapsed().as_micros() as u64,
            "Audio section loop: original sources live; loop boundaries corrected for startup skew"
        );

        // Monitor loop: polls for source completion, cancellation, and section boundaries.
        let crossfade_samples = crate::audio::crossfade::default_crossfade_samples(
            self.output_manager.mixer.sample_rate(),
        );
        let mixer_sample_rate = self.output_manager.mixer.sample_rate();

        // Look-ahead for the section-loop trigger. Must exceed the 10ms poll
        // sleep below so the trigger is always detected *before* the ideal
        // loop boundary; we then schedule the actual crossfade at the exact
        // sample via `start_at_sample`/`cancel_at_sample`, eliminating
        // polling jitter from the audible loop point. The cost is a small
        // race window for break-after-trigger, which we accept.
        let loop_trigger_lookahead = Duration::from_millis(30);

        let mut section_trigger = crate::section_loop::SectionLoopTrigger::new();

        type PrebuiltSources = Vec<Box<dyn crate::audio::sample_source::ChannelMappedSampleSource>>;

        // Sources pre-built for the next loop iteration, keyed by section name
        // so a section change invalidates them. Ready to swap in with zero I/O.
        let mut pending_sources: Option<(String, PrebuiltSources)> = None;

        // In-flight background build (section name + result channel). The build
        // runs on a detached worker thread rather than inline because
        // `create_channel_mapped_sources_from` blocks on BufferedSampleSource
        // warmup (file open/seek/decode). Doing that on the monitor thread
        // could delay the *first* loop boundary — the loop can be armed close
        // to the section end and the first build hits a cold file cache — which
        // showed up as a one-off stutter on the first iteration.
        let mut build_rx: Option<(String, mpsc::Receiver<Result<PrebuiltSources, String>>)> = None;

        // Deferred `loop_time_consumed` bumps. Each entry is the sample at
        // which the audio actually loops (so the reported elapsed only jumps
        // when the listener crosses the boundary, not when the monitor
        // detected it).
        let mut pending_consumption: Vec<(u64, Duration)> = Vec::new();

        // DIAGNOSTIC: per-swap counter to compare the first loop against later ones.
        let mut loop_iteration: u64 = 0;

        'monitor: loop {
            if cancel_handle.is_cancelled() || loop_break.load(Ordering::Relaxed) {
                break;
            }

            // The callback has stopped while a song is playing. End the song.
            //
            // Not a recoverable condition, and deliberately not treated as one.
            // The transport is driven by the callback's sample counter, so an
            // outage freezes it rather than advancing it: audio, MIDI and cues
            // all stop together and, when the device returns, would resume from
            // the bar they froze on rather than from where the room now is.
            //
            // On stage that is worse than stopping. The band has just played
            // through the outage with no click and no tracks, so they are not
            // where the frozen transport thinks they are, and audio reappearing
            // mid-song gives them a second, confidently wrong reference to
            // follow. Nothing host-side can recover the one piece of state that
            // matters, which is where the *band* is, so resuming cannot be made
            // correct — only quieter or louder about being wrong.
            //
            // Fast-forwarding to a wall clock has the same flaw: it assumes the
            // band held tempo through the moment they had nothing to hold it to.
            //
            // So the device layer keeps rebuilding, and this ends the song. The
            // ERROR is for the operator afterwards; in the moment there is
            // nothing to read and nothing to do.
            //
            // Note the limit: a device that accepts every buffer and produces no
            // sound still looks alive from here, and nothing host-side can tell
            // the difference. What this catches is the callback going away.
            // Two atomic loads, not a snapshot: this runs at 100Hz for the whole
            // song, and a snapshot clones the last error string.
            if !self
                .output_manager
                .is_callback_alive(self.liveness_window())
            {
                // Only now is the full picture worth assembling.
                let health = self.output_manager.health();
                error!(
                    song = song.name(),
                    // 0 means the callback never ran at all; `callbacks`
                    // separates that from a callback that ran and then stopped.
                    since_last_callback_ms = health
                        .since_last_callback
                        .map(|d| d.as_millis() as u64)
                        .unwrap_or(0),
                    callbacks = health.callbacks,
                    "Audio output callback stopped during playback; ending the song. \
                     The transport is frozen, so resuming would re-enter the song at \
                     the point it stopped rather than where the performance now is"
                );
                // Cancel before returning: the sources, MIDI and DMX for this
                // song all share this handle, and without it they would sit on a
                // frozen clock waiting to be joined, then resume together when
                // the device came back.
                //
                // Marked as a *failure* cancellation, not a deliberate one. The
                // player's cleanup skips its work when a playback was cancelled
                // on the grounds that whoever asked for it — stop(), a seek —
                // has already done that work and may have started a new playback
                // to protect. Nothing asked for this one.
                cancel_handle.cancel_due_to_failure();
                return Err(crate::audio::AudioError::Stream(format!(
                    "audio output callback stopped after {} callbacks; song '{}' ended",
                    health.callbacks,
                    song.name()
                )));
            }

            // Apply any deferred loop_time_consumed bumps whose sample has
            // been reached by the audio callback.
            if !pending_consumption.is_empty() {
                let now_sample = self.output_manager.mixer.current_sample();
                pending_consumption.retain(|&(apply_at, amount)| {
                    if now_sample >= apply_at {
                        *loop_time_consumed.lock() += amount;
                        false
                    } else {
                        true
                    }
                });
            }

            // Check if all sources have finished (EOF).
            let all_finished = source_finish_flags
                .iter()
                .all(|flag| flag.load(Ordering::Relaxed));
            if all_finished {
                break;
            }

            let armed = active_section.read().clone();
            let break_requested = section_loop_break.load(Ordering::Relaxed);

            // Which section we want warm loop-back sources for. Once armed, the
            // active section is authoritative. Before arming, speculatively
            // target whichever section the playhead is currently inside — it is
            // the loop-back candidate, so the sources are warm the instant the
            // performer arms, even on the section's final beat. (During looping
            // the raw clock runs past the section end, so we rely on `armed`
            // rather than the playhead lookup.)
            let prewarm_target = if break_requested {
                None
            } else if armed.is_some() {
                armed.clone()
            } else {
                // The clock is stream-relative; section bounds are
                // song-absolute. Offset by the playback start position.
                current_playhead_section(&song, start_time + clock.elapsed())
            };

            // Collect the result of a finished background build. Take the
            // channel out and only restore it if the build is still running.
            let wanted = |name: &str| prewarm_target.as_ref().is_some_and(|t| t.name == name);
            if let Some((name, rx)) = build_rx.take() {
                match rx.try_recv() {
                    Ok(Ok(srcs)) => {
                        // Keep only if we still want this section's sources.
                        if wanted(&name) {
                            pending_sources = Some((name, srcs));
                        }
                    }
                    Ok(Err(e)) => {
                        error!(
                            err = %e,
                            section = name,
                            "Failed to pre-build section loop sources; will retry"
                        );
                    }
                    // Still building — put the channel back.
                    Err(mpsc::TryRecvError::Empty) => build_rx = Some((name, rx)),
                    // Worker vanished without sending (e.g. spawn failed);
                    // leave build_rx cleared so we retry next iteration.
                    Err(mpsc::TryRecvError::Disconnected) => {}
                }
            }

            // Drop warmed sources / abandon an in-flight build if the target
            // section changed (playhead moved on, loop armed elsewhere, section
            // cleared, or a break was requested) — otherwise we'd swap stale
            // audio in on the next trigger.
            if pending_sources
                .as_ref()
                .is_some_and(|(name, _)| !wanted(name))
            {
                pending_sources = None;
            }
            if build_rx.as_ref().is_some_and(|(name, _)| !wanted(name)) {
                // Dropping the receiver detaches the worker; its eventual send
                // is a harmless no-op.
                build_rx = None;
            }

            // Kick off a background build for the target if we have neither
            // warmed sources nor a build already in flight. The worker thread
            // absorbs the warmup/I/O block so trigger detection here is never
            // delayed.
            if let Some(target) = &prewarm_target {
                if pending_sources.is_none() && build_rx.is_none() {
                    let (tx, rx) = mpsc::channel();
                    let song = song.clone();
                    let ctx = playback_context.clone();
                    let mappings = mappings.clone();
                    let start_time = target.start_time;
                    let _ = thread::Builder::new()
                        .name("mtrack-section-prebuild".into())
                        .spawn(move || {
                            let result = song
                                .create_channel_mapped_sources_from(&ctx, start_time, &mappings)
                                .map_err(|e| e.to_string());
                            // Receiver may be gone if the target changed.
                            let _ = tx.send(result);
                        });
                    build_rx = Some((target.name.clone(), rx));
                }
            }

            // Only loop when the section is actually armed.
            if let Some(section) = armed {
                if !break_requested {
                    // Section bounds are song-absolute; the clock counts from
                    // this playback's start. Without the offset, a loop armed
                    // after starting mid-song (seek, section chip) fires every
                    // boundary `start_time` too late — the audio runs past the
                    // section end while the UI wraps on the correct grid.
                    let elapsed = start_time + clock.elapsed();
                    if let Some(trigger_time) =
                        section_trigger.check(&section, elapsed, loop_trigger_lookahead)
                    {
                        let pending = match pending_sources.take() {
                            Some((_, srcs)) => srcs,
                            None => {
                                warn!(
                                    section = section.name,
                                    "Audio section loop: trigger fired with no pre-built \
                                     sources; skipping this boundary"
                                );
                                thread::sleep(Duration::from_millis(10));
                                continue 'monitor;
                            }
                        };

                        // Map the ideal trigger time to an exact mixer sample.
                        // We use the live sample counter plus the remaining
                        // delta so the result is robust to any small skew
                        // between the playback clock and the mixer counter.
                        let now_sample = self.output_manager.mixer.current_sample();
                        // Correct for the startup skew so the boundary tracks the
                        // audio content, not the clock epoch (see where
                        // `audio_clock_skew_samples` is captured). The shift is a
                        // constant, so inter-boundary spacing is unchanged.
                        let loop_sample = crate::section_loop::loop_boundary_sample(
                            now_sample,
                            elapsed,
                            trigger_time,
                            mixer_sample_rate,
                        )
                        .saturating_add(audio_clock_skew_samples);
                        // At a section-loop boundary the loop-point content
                        // coincides on both sources: the section's `end_time` on
                        // the outgoing source is the same musical moment as its
                        // `start_time` on the incoming one — for a one-measure
                        // loop, literally the same downbeat. With the boundary
                        // aligned (seek-residual trim + startup-skew correction),
                        // the two copies land on the same sample, so we overlap
                        // them and let them reinforce into a single click rather
                        // than cutting one.
                        //
                        // The crossfade straddles the boundary on the outgoing
                        // side: the outgoing source plays THROUGH loop_sample at
                        // full gain and fades out over the following window, while
                        // the incoming source begins exactly at loop_sample and
                        // fades in. This matters because the outgoing source's
                        // resampler is in steady state, so its copy of the downbeat
                        // is clean and carries the attack; the incoming source's
                        // resampler is freshly created, so its first samples carry
                        // a sinc pre-ring — the fade-in attenuates that pre-ring
                        // while the clean outgoing transient dominates the attack.
                        let fade_end_sample = loop_sample.saturating_add(crossfade_samples);

                        loop_iteration += 1;
                        debug!(
                            section = section.name,
                            loop_iteration,
                            loop_sample,
                            fade_end_sample,
                            "Audio section loop: scheduling crossfade"
                        );

                        // Schedule fade-out + cut on currently-playing sources.
                        // The envelope holds at 1.0 until loop_sample, then ramps
                        // to 0 by fade_end_sample; cancel_at_sample removes them
                        // once the fade completes.
                        let current_ids: Vec<u64> = {
                            let sources = self.output_manager.mixer.get_active_sources();
                            let guard = sources.read();
                            guard.iter().map(|s| s.lock().id).collect()
                        };
                        if !current_ids.is_empty() {
                            let fade_out = Arc::new(
                                crate::audio::crossfade::GainEnvelope::fade_out(
                                    crossfade_samples,
                                    crate::audio::crossfade::CrossfadeCurve::Linear,
                                )
                                .with_start_sample(loop_sample),
                            );
                            self.output_manager
                                .mixer
                                .set_gain_envelope(&current_ids, fade_out);
                            self.output_manager
                                .mixer
                                .set_cancel_at_sample(&current_ids, fade_end_sample);
                        }

                        // Add the pre-built sources. Their own start_at_sample
                        // delays playback until loop_sample; the fade-in envelope
                        // ramps from there so the incoming source's resampler
                        // pre-ring is attenuated while the clean outgoing transient
                        // carries the downbeat attack.
                        source_finish_flags.clear();
                        for source in pending {
                            // Each source gets its own envelope instance — a shared
                            // GainEnvelope.position would advance once per source
                            // per batch and the fade would complete N× too fast.
                            let fade_in =
                                Some(Arc::new(crate::audio::crossfade::GainEnvelope::fade_in(
                                    crossfade_samples,
                                    crate::audio::crossfade::CrossfadeCurve::Linear,
                                )));
                            let (mut active_source, flag) =
                                build_active_source(source, mappings, &cancel_handle, fade_in);
                            active_source.start_at_sample = Some(loop_sample);
                            source_finish_flags.push(flag);
                            if let Err(e) = self.output_manager.add_source(active_source) {
                                error!(err = %e, "Failed to add section loop source");
                            }
                        }

                        // Defer the consumed-time bump until the audio thread
                        // actually crosses the boundary, so the user-visible
                        // elapsed doesn't briefly jump backwards.
                        let section_duration = section.end_time.saturating_sub(section.start_time);
                        pending_consumption.push((loop_sample, section_duration));

                        continue 'monitor;
                    }
                }
            }

            thread::sleep(Duration::from_millis(10));
        }

        // When section_loop_break is set, clear active_section so the song
        // continues normally past the section end.
        if section_loop_break.load(Ordering::Relaxed) {
            let mut section = active_section.write();
            *section = None;
        }

        // Loop if the song has loop_playback and we haven't been told to stop.
        while song.loop_playback()
            && !cancel_handle.is_cancelled()
            && !loop_break.load(Ordering::Relaxed)
        {
            info!(song = song.name(), "Audio loop: creating crossfade sources");

            let crossfade_samples = crate::audio::crossfade::default_crossfade_samples(
                self.output_manager.mixer.sample_rate(),
            );

            // Fade out any remaining sources from previous iteration.
            let current_ids: Vec<u64> = {
                let sources = self.output_manager.mixer.get_active_sources();
                let guard = sources.read();
                guard.iter().map(|s| s.lock().id).collect()
            };
            if !current_ids.is_empty() {
                let fade_out = Arc::new(crate::audio::crossfade::GainEnvelope::fade_out(
                    crossfade_samples,
                    crate::audio::crossfade::CrossfadeCurve::Linear,
                ));
                self.output_manager
                    .mixer
                    .set_gain_envelope(&current_ids, fade_out);
            }

            // Create new sources at t=0 with fade-in.
            let new_sources = match song.create_channel_mapped_sources_from(
                &playback_context,
                Duration::ZERO,
                mappings,
            ) {
                Ok(s) => s,
                Err(e) => {
                    error!(err = e.as_ref(), "Failed to create loop audio sources");
                    break;
                }
            };

            let mut new_finish_flags = Vec::new();
            let fade_in_envelope = Some(Arc::new(crate::audio::crossfade::GainEnvelope::fade_in(
                crossfade_samples,
                crate::audio::crossfade::CrossfadeCurve::Linear,
            )));
            for source in new_sources {
                let (active_source, flag) =
                    build_active_source(source, mappings, &cancel_handle, fade_in_envelope.clone());
                new_finish_flags.push(flag);
                if let Err(e) = self.output_manager.add_source(active_source) {
                    error!(err = %e, "Failed to add loop source to mixer");
                    break;
                }
            }

            if new_finish_flags.is_empty() {
                break;
            }

            // Wait for new sources to finish or cancel/loop_break.
            let loop_finished = Arc::new(AtomicBool::new(false));
            let loop_finished_monitor = loop_finished.clone();
            let cancel_for_monitor = cancel_handle.clone();
            let loop_break_for_monitor = loop_break.clone();
            let monitor_flags = new_finish_flags;

            thread::spawn(move || loop {
                if cancel_for_monitor.is_cancelled()
                    || loop_break_for_monitor.load(Ordering::Relaxed)
                {
                    break;
                }
                if monitor_flags.iter().all(|f| f.load(Ordering::Relaxed)) {
                    loop_finished_monitor.store(true, Ordering::Relaxed);
                    cancel_for_monitor.notify();
                    break;
                }
                thread::sleep(Duration::from_millis(10));
            });

            cancel_handle.wait(loop_finished);
        }

        Ok(())
    }

    fn mixer(&self) -> Option<Arc<super::super::mixer::AudioMixer>> {
        Some(Arc::new(self.output_manager.mixer.clone()))
    }

    fn source_sender(&self) -> Option<super::super::SourceSender> {
        Some(self.output_manager.source_tx.clone())
    }

    fn sample_counter(&self) -> Option<Arc<AtomicU64>> {
        Some(self.output_manager.mixer.sample_counter())
    }

    fn sample_rate(&self) -> Option<u32> {
        Some(self.output_manager.mixer.sample_rate())
    }

    #[cfg(test)]
    fn to_mock(&self) -> Result<Arc<super::super::mock::Device>, crate::audio::AudioError> {
        Err(crate::audio::AudioError::Other("not a mock".into()))
    }
}

#[cfg(test)]
mod test {
    use super::*;

    /// The device the web UI offers must be one the player can open.
    ///
    /// These are two functions walking the same hosts, and when they drifted
    /// the picker advertised 19 devices against 8 openable ones, so choosing
    /// the wrong alias failed with "no device found with name ..." (#357).
    ///
    /// Vacuous where there is no audio hardware, which is most CI containers --
    /// both lists are empty and agree. The load-bearing version is the hardware
    /// suite's `advertised_devices_are_openable`, which runs on real rigs. This
    /// exists so the two functions cannot be edited apart without something
    /// local objecting.
    #[test]
    fn advertised_and_openable_device_lists_agree() {
        let advertised: std::collections::BTreeSet<String> = match list_device_info() {
            Ok(infos) => infos.into_iter().map(|d| d.name).collect(),
            // No host at all is a container without ALSA, not a defect.
            Err(_) => return,
        };
        let openable: std::collections::BTreeSet<String> = match Device::list_cpal_devices() {
            Ok(devices) => devices.into_iter().map(|d| d.name).collect(),
            Err(_) => return,
        };

        let advertised_only: Vec<&String> = advertised.difference(&openable).collect();
        assert!(
            advertised_only.is_empty(),
            "the picker offers devices the player cannot open: {advertised_only:?}"
        );
    }

    mod virtual_node_tests {
        use super::*;

        fn info(name: &str, max_channels: u16) -> AudioDeviceInfo {
            AudioDeviceInfo {
                name: name.to_string(),
                max_channels,
                host_name: "Alsa".to_string(),
                supported_sample_rates: vec![48000],
                supported_formats: vec![],
                virtual_node: is_virtual_node(name),
                channels_known: channels_are_known(is_virtual_node(name), max_channels),
            }
        }

        #[test]
        fn plug_nodes_are_recognized() {
            for name in &[
                "alsa:plughw:CARD=WING",
                "alsa:plug:CARD=MAT",
                "alsa:default",
                "alsa:default:CARD=MAT",
                "alsa:sysdefault:CARD=MAT",
                "alsa:pulse",
                "alsa:dmix:CARD=MAT,DEV=0",
                "alsa:dsnoop:CARD=MAT",
            ] {
                assert!(is_virtual_node(name), "{name} should be virtual");
            }
        }

        #[test]
        fn hardware_nodes_are_not_virtual() {
            for name in &["alsa:hw:CARD=MAT,DEV=0", "alsa:hw:0,0", "alsa:hw:CARD=WING"] {
                assert!(!is_virtual_node(name), "{name} should not be virtual");
            }
        }

        #[test]
        fn card_token_pairs_plug_node_with_hardware() {
            assert_eq!(card_token("alsa:plughw:CARD=MAT").as_deref(), Some("MAT"));
            assert_eq!(card_token("alsa:hw:CARD=MAT,DEV=0").as_deref(), Some("MAT"));
            assert_eq!(card_token("alsa:plughw:0,0").as_deref(), Some("0"));
            assert_eq!(card_token("alsa:hw:0,0").as_deref(), Some("0"));
            assert_eq!(
                card_token("alsa:sysdefault:CARD=WING").as_deref(),
                Some("WING")
            );
        }

        #[test]
        fn card_token_absent_for_cardless_nodes() {
            assert_eq!(card_token("alsa:default"), None);
            assert_eq!(card_token("alsa:pulse"), None);
            assert_eq!(card_token("alsa:hw:"), None);
        }

        #[test]
        fn dev_index_parses_both_namings() {
            assert_eq!(dev_index("alsa:hw:CARD=MAT,DEV=2"), Some(2));
            assert_eq!(dev_index("alsa:hw:0,1"), Some(1));
            assert_eq!(dev_index("alsa:plughw:CARD=MAT"), None);
            assert_eq!(dev_index("alsa:default"), None);
        }

        /// The case that motivated this: `plughw:CARD=MAT` claims cpal's clamped
        /// 32, while the hardware it routes to reports its real 8.
        #[test]
        fn plug_node_inherits_channel_count_from_hardware() {
            let mut infos = vec![
                info("alsa:hw:CARD=MAT,DEV=0", 8),
                info("alsa:plughw:CARD=MAT", CPAL_CHANNEL_CLAMP),
            ];
            resolve_virtual_channel_counts(&mut infos);

            let plug = &infos[1];
            assert_eq!(plug.max_channels, 8);
            assert!(plug.channels_known);
            assert_eq!(plug.channels_display(), "8");
        }

        /// The bug this keying exists to prevent, on the most ordinary hardware
        /// there is: an Intel HDA card exposes analog on `DEV=0` and HDMI on
        /// `DEV=3`, with different channel counts, and every plug node in front
        /// of them reports the same clamped maximum.
        ///
        /// Looking the sibling up by card alone handed `plughw:CARD=PCH,DEV=3`
        /// the analog node's 2 channels and marked it `channels_known` — a
        /// confidently wrong number, which is the single outcome this labelling
        /// exists to avoid.
        #[test]
        fn plug_node_takes_the_subdevice_it_names_not_the_lowest() {
            let mut infos = vec![
                info("alsa:hw:CARD=PCH,DEV=0", 2),
                info("alsa:hw:CARD=PCH,DEV=3", 8),
                info("alsa:plughw:CARD=PCH,DEV=3", CPAL_CHANNEL_CLAMP),
                info("alsa:plughw:CARD=PCH,DEV=0", CPAL_CHANNEL_CLAMP),
            ];
            resolve_virtual_channel_counts(&mut infos);

            assert_eq!(infos[2].max_channels, 8, "DEV=3 plug node routes to HDMI");
            assert!(infos[2].channels_known);
            assert_eq!(infos[3].max_channels, 2, "DEV=0 plug node routes to analog");
            assert!(infos[3].channels_known);
        }

        /// A plug node naming a subdevice with no hardware sibling stays
        /// unknown. Falling back to the lowest subdevice would reintroduce the
        /// bug above by another route, and "unknown" is the honest answer — the
        /// same stance taken for `default` and `pulse`.
        #[test]
        fn plug_node_for_an_unlisted_subdevice_stays_unknown() {
            let mut infos = vec![
                info("alsa:hw:CARD=PCH,DEV=0", 2),
                info("alsa:plughw:CARD=PCH,DEV=7", CPAL_CHANNEL_CLAMP),
            ];
            resolve_virtual_channel_counts(&mut infos);

            assert!(
                !infos[1].channels_known,
                "no DEV=7 sibling means no honest count to borrow"
            );
        }

        #[test]
        fn plug_node_prefers_the_lowest_subdevice() {
            let mut infos = vec![
                info("alsa:hw:CARD=MAT,DEV=3", 2),
                info("alsa:hw:CARD=MAT,DEV=0", 8),
                info("alsa:plughw:CARD=MAT", CPAL_CHANNEL_CLAMP),
            ];
            resolve_virtual_channel_counts(&mut infos);

            assert_eq!(infos[2].max_channels, 8);
        }

        /// The WING: its raw node reports no cpal-known format, so it never
        /// appears and there is nothing to borrow a count from. The plug node
        /// must then say "virtual" rather than assert a fictional 32.
        #[test]
        fn plug_node_without_hardware_sibling_stays_unknown() {
            let mut infos = vec![info("alsa:plughw:CARD=WING", CPAL_CHANNEL_CLAMP)];
            resolve_virtual_channel_counts(&mut infos);

            let plug = &infos[0];
            assert!(!plug.channels_known);
            assert_eq!(plug.channels_display(), "virtual");
            assert!(plug.to_string().contains("Channels=virtual"));
        }

        #[test]
        fn cardless_plug_nodes_stay_unknown() {
            let mut infos = vec![
                info("alsa:hw:CARD=MAT,DEV=0", 8),
                info("alsa:default", CPAL_CHANNEL_CLAMP),
                info("alsa:pulse", CPAL_CHANNEL_CLAMP),
            ];
            resolve_virtual_channel_counts(&mut infos);

            assert_eq!(infos[1].channels_display(), "virtual");
            assert_eq!(infos[2].channels_display(), "virtual");
        }

        /// An interface at or above the clamp reports the clamp too, so its
        /// count is a floor -- but it is hardware, not a plugin, and must not be
        /// labelled "virtual".
        #[test]
        fn clamped_hardware_reports_a_floor_not_virtual() {
            let device = info("alsa:hw:CARD=BIG,DEV=0", CPAL_CHANNEL_CLAMP);
            assert!(!device.virtual_node);
            assert!(!device.channels_known);
            assert_eq!(device.channels_display(), format!("{CPAL_CHANNEL_CLAMP}+"));
        }

        /// Below the clamp is an honest count, and the boundary is the whole
        /// point of the constant: cpal 0.18 raised the cap to 64, so a 32-channel
        /// interface now reports its real width instead of a floor. Pinned so a
        /// future cpal bump that moves the cap again fails here rather than
        /// silently re-labelling real hardware as unknown.
        #[test]
        fn a_count_below_the_clamp_is_honest() {
            let device = info("alsa:hw:CARD=BIG,DEV=0", 32);
            assert!(device.channels_known);
            assert_eq!(device.channels_display(), "32");
        }

        /// ...but only for hardware. A software node's count is the plugin's,
        /// not the rig's, at any value.
        ///
        /// Observed on the rig under cpal 0.18: `default` and `pulse` report 32,
        /// which is below the new clamp, and `mtrack devices` offered them as
        /// `(Channels=32)`. Under 0.17 the same 32 *was* the clamp, so they read
        /// as unknown for a reason that had nothing to do with being virtual.
        #[test]
        fn a_software_node_never_reports_an_honest_count() {
            for name in &["alsa:default", "alsa:pulse"] {
                let device = info(name, 32);
                assert!(device.virtual_node, "{name} should be a virtual node");
                assert!(!device.channels_known, "{name} reported 32 as a capability");
                assert_eq!(device.channels_display(), "virtual");
            }
        }

        #[test]
        fn hardware_never_borrows_from_a_plug_node() {
            let mut infos = vec![
                info("alsa:hw:CARD=WING,DEV=0", CPAL_CHANNEL_CLAMP),
                info("alsa:plughw:CARD=WING", 8),
            ];
            resolve_virtual_channel_counts(&mut infos);

            // The hardware entry is left alone: only plug nodes are resolved.
            assert_eq!(infos[0].max_channels, CPAL_CHANNEL_CLAMP);
            assert!(!infos[0].channels_known);
        }

        #[test]
        fn known_hardware_count_is_displayed_plainly() {
            let device = info("alsa:hw:CARD=MAT,DEV=0", 8);
            assert_eq!(device.channels_display(), "8");
            assert_eq!(
                device.to_string(),
                "alsa:hw:CARD=MAT,DEV=0 (Channels=8) (Alsa)"
            );
        }

        /// Device names carry cpal's host prefix, and reading it as part of the
        /// PCM id fails silently: `alsa:default` stops looking like a plug node
        /// and every card token comes back as `hw:CARD=MAT`, so nothing pairs.
        /// The first cut of this code did exactly that, and `mtrack devices`
        /// reported `alsa:default (Channels=32+)` instead of `virtual`.
        #[test]
        fn host_prefix_is_not_read_as_part_of_the_pcm_id() {
            assert_eq!(pcm_id("alsa:hw:CARD=MAT,DEV=0"), "hw:CARD=MAT,DEV=0");
            assert_eq!(pcm_id("alsa:default"), "default");
            // A bare PCM id with no host prefix is left alone.
            assert_eq!(pcm_id("default"), "default");

            assert!(is_virtual_node("alsa:default"));
            assert_eq!(card_token("alsa:hw:CARD=MAT,DEV=0").as_deref(), Some("MAT"));
        }
    }

    /// Concurrent enumeration must not cost the process its stdout.
    ///
    /// Every walk silences ALSA with `shh`, which swaps fd 1 and fd 2 and puts
    /// them back on drop. Unserialised, two threads restore each other's pipe
    /// and the process is left writing into a closed descriptor -- which is how
    /// this was found: the test binary reported a failure and printed nothing at
    /// all to explain it.
    ///
    /// Linux-only because it reads the descriptor through `/proc`; the ALSA path
    /// under test is Linux-only too.
    ///
    /// Observing the descriptors has to take the lock as well, or the baseline
    /// can be read *during* another test's legitimate silence window and record
    /// its pipe as the original.
    #[test]
    #[cfg(target_os = "linux")]
    fn concurrent_enumeration_preserves_process_stdout() {
        fn std_stream_targets() -> (Option<std::path::PathBuf>, Option<std::path::PathBuf>) {
            let _lock = ENUMERATION_LOCK.lock();
            (
                std::fs::read_link("/proc/self/fd/1").ok(),
                std::fs::read_link("/proc/self/fd/2").ok(),
            )
        }

        let (stdout_before, stderr_before) = std_stream_targets();

        let handles: Vec<_> = (0..8)
            .map(|i| {
                thread::spawn(move || {
                    // Mix the paths that silence: two enumerations and a
                    // resolution that misses and then lists alternatives.
                    match i % 3 {
                        0 => drop(list_device_info()),
                        1 => drop(Device::list_cpal_devices()),
                        _ => drop(Device::get(config::Audio::new("no-such-device"))),
                    }
                })
            })
            .collect();
        for handle in handles {
            handle.join().expect("enumeration thread panicked");
        }

        let (stdout_after, stderr_after) = std_stream_targets();
        assert_eq!(
            stdout_after, stdout_before,
            "stdout was left pointing somewhere else"
        );
        assert_eq!(
            stderr_after, stderr_before,
            "stderr was left pointing somewhere else"
        );
    }

    mod available_hint_tests {
        use super::*;

        /// The hint must name devices when there are any, and say so plainly
        /// when there are none -- either way the operator learns something.
        ///
        /// Asserts the hint's shape rather than cross-checking its contents
        /// against a second enumeration. The two calls cannot be made to agree:
        /// the hint may be up to `AVAILABLE_HINT_TTL` old, and enumeration is
        /// self-truncating under concurrency -- ALSA will not describe a device
        /// while its siblings are held, so a listing taken while another test
        /// holds handles sees fewer devices than one taken alone. Comparing them
        /// tests the scheduler, not the hint.
        #[test]
        fn hint_describes_the_environment_it_runs_in() {
            let hint = available_hint();
            assert!(
                hint.contains("available devices:")
                    || hint.contains("no audio output devices were found")
                    || hint.contains("could not be listed"),
                "the hint should tell the operator something actionable: {hint}"
            );
        }

        /// The hint is cached, so a device that fails to resolve twice a second
        /// does not walk every ALSA device twice a second with it.
        #[test]
        fn repeated_hints_are_served_from_cache() {
            // Observe the cache rather than the clock. Timing the second call
            // looked like the obvious test and is not one: `available_hint`
            // holds the cache lock across a rebuild on purpose, to collapse a
            // herd of simultaneous failures into a single walk, so a "cached"
            // call racing another test's rebuild blocks behind it. On a rig with
            // nineteen devices that walk is not fast, and the assertion passed
            // here only because a dev box has nothing to enumerate.
            let first = available_hint();
            let built_at = AVAILABLE_HINT_CACHE
                .lock()
                .as_ref()
                .map(|(at, _)| *at)
                .expect("the first call populates the cache");

            let second = available_hint();
            let still = AVAILABLE_HINT_CACHE
                .lock()
                .as_ref()
                .map(|(at, _)| *at)
                .expect("the cache is still populated");

            assert_eq!(first, second);
            assert_eq!(
                built_at, still,
                "the second call rebuilt the hint instead of serving the cache"
            );
        }

        /// The whole point of item 2: the failure names alternatives.
        #[test]
        fn missing_device_error_lists_alternatives() {
            let config = config::Audio::new("definitely-not-a-real-device");
            let message = match Device::get(config) {
                Ok(_) => panic!("bogus device must not resolve"),
                Err(e) => e.to_string(),
            };

            assert!(
                message.contains("no device found with name definitely-not-a-real-device"),
                "{message}"
            );
            assert!(
                message.contains("available devices:")
                    || message.contains("no audio output devices were found")
                    || message.contains("could not be listed"),
                "{message}"
            );
        }
    }

    mod current_playhead_section_tests {
        use super::*;

        // Two 4-beat measures at 120 BPM: beats every 0.5s.
        // verse = measures 1..2 → [0.0, 2.0); chorus = measures 2..3 → [2.0, 3.5).
        fn song_with_sections() -> Song {
            Song::new_for_test_with_sections(
                "test",
                &["click"],
                crate::audio::click_analysis::BeatGrid {
                    beats: vec![0.0, 0.5, 1.0, 1.5, 2.0, 2.5, 3.0, 3.5],
                    measure_starts: vec![0, 4],
                },
                vec![
                    crate::config::Section {
                        name: "verse".to_string(),
                        start_measure: 1,
                        end_measure: 2,
                        start_beat: None,
                        end_beat: None,
                        color: None,
                    },
                    crate::config::Section {
                        name: "chorus".to_string(),
                        start_measure: 2,
                        end_measure: 3,
                        start_beat: None,
                        end_beat: None,
                        color: None,
                    },
                ],
            )
        }

        #[test]
        fn finds_section_containing_playhead() {
            let song = song_with_sections();
            assert_eq!(
                current_playhead_section(&song, Duration::from_millis(500)).map(|s| s.name),
                Some("verse".to_string())
            );
            assert_eq!(
                current_playhead_section(&song, Duration::from_millis(2500)).map(|s| s.name),
                Some("chorus".to_string())
            );
        }

        #[test]
        fn boundary_is_half_open() {
            let song = song_with_sections();
            // Exactly at the verse/chorus boundary belongs to chorus: a
            // section owns [start, end), so the measure boundary is the start
            // of the next section, not the tail of the previous one.
            let at_boundary = current_playhead_section(&song, Duration::from_secs(2)).unwrap();
            assert_eq!(at_boundary.name, "chorus");
            assert_eq!(at_boundary.start_time, Duration::from_secs(2));
        }

        #[test]
        fn none_when_past_all_sections() {
            let song = song_with_sections();
            assert!(current_playhead_section(&song, Duration::from_secs(10)).is_none());
        }

        #[test]
        fn none_without_beat_grid() {
            // A plain test song has no beat grid, so nothing resolves.
            let song = Song::new_for_test("test", &["click"]);
            assert!(current_playhead_section(&song, Duration::from_secs(1)).is_none());
        }
    }

    mod target_to_cpal_sample_format_tests {
        use super::*;

        #[test]
        fn float_any_bits() {
            assert_eq!(
                target_to_cpal_sample_format(SampleFormat::Float, 32),
                cpal::SampleFormat::F32
            );
            assert_eq!(
                target_to_cpal_sample_format(SampleFormat::Float, 64),
                cpal::SampleFormat::F32
            );
        }

        #[test]
        fn int_16_bit() {
            assert_eq!(
                target_to_cpal_sample_format(SampleFormat::Int, 16),
                cpal::SampleFormat::I16
            );
        }

        #[test]
        fn int_32_bit() {
            assert_eq!(
                target_to_cpal_sample_format(SampleFormat::Int, 32),
                cpal::SampleFormat::I32
            );
        }

        #[test]
        fn int_other_defaults_to_i32() {
            assert_eq!(
                target_to_cpal_sample_format(SampleFormat::Int, 24),
                cpal::SampleFormat::I32
            );
            assert_eq!(
                target_to_cpal_sample_format(SampleFormat::Int, 8),
                cpal::SampleFormat::I32
            );
        }
    }

    mod resolve_buffer_size_tests {
        use super::*;

        #[test]
        fn none_returns_fallback() {
            assert_eq!(resolve_buffer_size(None, 256, None), Some(256));
            assert_eq!(resolve_buffer_size(None, 512, Some(64)), Some(512));
        }

        #[test]
        fn default_returns_none() {
            assert_eq!(
                resolve_buffer_size(Some(StreamBufferSize::Default), 256, None),
                None
            );
        }

        #[test]
        fn min_returns_min_supported_when_available() {
            assert_eq!(
                resolve_buffer_size(Some(StreamBufferSize::Min), 256, Some(64)),
                Some(64)
            );
        }

        #[test]
        fn min_falls_back_when_no_min_supported() {
            assert_eq!(
                resolve_buffer_size(Some(StreamBufferSize::Min), 256, None),
                Some(256)
            );
        }

        #[test]
        fn fixed_returns_specified_value() {
            assert_eq!(
                resolve_buffer_size(Some(StreamBufferSize::Fixed(128)), 256, Some(64)),
                Some(128)
            );
        }
    }

    mod validate_channel_count_tests {
        use super::*;

        #[test]
        fn passes_when_channels_within_limit() {
            let mut mappings = HashMap::new();
            mappings.insert("track1".to_string(), vec![1, 2]);
            mappings.insert("track2".to_string(), vec![3, 4]);

            let result = validate_channel_count(&mappings, 4, "test_song", "test_device");
            assert!(result.is_ok());
        }

        #[test]
        fn fails_when_channels_exceed_limit() {
            let mut mappings = HashMap::new();
            mappings.insert("track1".to_string(), vec![1, 2, 3, 4]);

            let result = validate_channel_count(&mappings, 2, "test_song", "test_device");
            assert!(result.is_err());
            let err = result.unwrap_err().to_string();
            assert!(err.contains("4"), "error should mention requested channels");
            assert!(err.contains("2"), "error should mention available channels");
            assert!(err.contains("test_song"), "error should mention song name");
            assert!(
                err.contains("test_device"),
                "error should mention device name"
            );
        }

        #[test]
        fn fails_on_empty_mappings_with_descriptive_error() {
            let mappings: HashMap<String, Vec<u16>> = HashMap::new();
            let result = validate_channel_count(&mappings, 8, "song", "device");
            let err = result.unwrap_err().to_string();
            assert!(
                err.contains("no track mappings configured"),
                "error should explain that mappings are missing, got: {err}"
            );
        }

        #[test]
        fn uses_max_channel_across_all_tracks() {
            let mut mappings = HashMap::new();
            mappings.insert("track1".to_string(), vec![1]);
            mappings.insert("track2".to_string(), vec![8]);

            // Max channel is 8, device has 8 — should pass.
            assert!(validate_channel_count(&mappings, 8, "s", "d").is_ok());
            // Max channel is 8, device has 7 — should fail.
            assert!(validate_channel_count(&mappings, 7, "s", "d").is_err());
        }
    }
}
