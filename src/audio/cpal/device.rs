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
pub use crate::device_info::{AudioDeviceInfo, SupportedFormat};
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

use std::ops::ControlFlow;

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
use super::stream::{CpalOutputStreamFactory, PeriodSetting};

mod loop_scheduler;

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

/// Whether a device name is a raw ALSA hardware node (`hw:...`): the one node
/// whose channel count is the card's own.
fn is_hw_node(device_name: &str) -> bool {
    pcm_id(device_name).starts_with("hw:")
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
    let mut found = None;
    walk_output_devices(|host_id, device, device_id| {
        // Name first: a non-match is dropped here, before its configurations
        // are queried and before anything is built from it, so its handle is
        // released immediately.
        if device_id.trim() != name.trim() {
            return ControlFlow::Continue(());
        }
        let Some(configs) = output_configs(&device) else {
            debug!(
                device_name = %device_id,
                "Device matched by name but reports no output configurations"
            );
            return ControlFlow::Continue(());
        };
        found = Some(ResolvedOutputDevice {
            name: device_id,
            host_id,
            device,
            configs,
        });
        ControlFlow::Break(())
    })?;
    Ok(found)
}

/// Every host's devices, visited one at a time in host then device order:
/// the one enumeration behind the listing, the search and the open, with
/// each caller's filter in `visit`. A device `visit` declines is dropped
/// before anything else is asked of it, so its handle is held no longer
/// than its id takes to read; a host whose devices will not list is logged
/// and skipped. `visit` says whether to go on.
fn walk_output_devices(
    mut visit: impl FnMut(cpal::HostId, cpal::Device, String) -> ControlFlow<()>,
) -> Result<(), Box<dyn Error>> {
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
            let Ok(id) = device.id() else { continue };
            if visit(host_id, device, id.to_string()).is_break() {
                return Ok(());
            }
        }
    }
    Ok(())
}

/// Lists audio devices as simple info structs (no trait objects).
pub fn list_device_info() -> Result<Vec<AudioDeviceInfo>, Box<dyn Error>> {
    let mut infos: Vec<AudioDeviceInfo> = Vec::new();
    walk_output_devices(|host_id, device, name| {
        if let Some(output_configs) = output_configs(&device) {
            infos.push(device_info(host_id, name, &output_configs));
        }
        ControlFlow::Continue(())
    })?;
    resolve_virtual_channel_counts_on_system(&mut infos, None);
    infos.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(infos)
}

/// One device's entry, from the output configurations it reports, before
/// [`resolve_virtual_channel_counts`] has looked for a hardware sibling.
fn device_info(
    host_id: cpal::HostId,
    name: String,
    output_configs: &[cpal::SupportedStreamConfigRange],
) -> AudioDeviceInfo {
    let max_channels = max_output_channels(output_configs);

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
    let virtual_node = is_virtual_node(&name);
    AudioDeviceInfo {
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
    }
}

/// The channel count a device is opened with: the listing's, once
/// [`resolve_virtual_channel_counts`] has looked for a hardware sibling, where
/// that is known, and otherwise the device's own report.
///
/// A plug node's own report is cpal's clamp, so opening it at that width had
/// the mixer fill 64 channels on every callback for ALSA's plug layer to drop
/// all but the hardware's eight.
fn open_channel_count(listed: Option<&AudioDeviceInfo>, reported: u16) -> u16 {
    match listed {
        Some(info) if info.channels_known => info.max_channels,
        _ => reported,
    }
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
///
/// `hw_channels` is asked first, for `(card, subdevice)`: what the raw `hw`
/// node offers by ALSA's own hw_params ([`alsa_hw_channels`] on a real system).
/// That answers where cpal cannot -- the WING's raw node, whose formats cpal
/// does not know -- and a hardware node at cpal's clamp. `None` (busy, absent,
/// not ALSA) falls back to the listed sibling, then to unknown. Each
/// `(card, subdevice)` is asked at most once.
fn resolve_virtual_channel_counts(
    infos: &mut [AudioDeviceInfo],
    mut hw_channels: impl FnMut(&str, u16) -> Option<u16>,
) {
    // (card, subdevice) -> real channel count, because subdevices on one card
    // are not interchangeable. An Intel HDA card exposes `hw:CARD=PCH,DEV=0`
    // (analog, 2ch) beside `hw:CARD=PCH,DEV=3` (HDMI, 8ch), and every
    // `plughw:CARD=PCH,DEV=n` in front of them reports the same clamped maximum.
    let mut by_subdevice: HashMap<(String, u16), u16> = HashMap::new();
    // Card identity -> (subdevice index, channels) for the lowest subdevice,
    // which is where a plug node naming no `DEV=` routes.
    let mut lowest_subdevice: HashMap<String, (u16, u16)> = HashMap::new();

    for info in infos.iter() {
        // Only the raw `hw:` node is the hardware. `front:`, `surround71:` and
        // the like are not plugins by [`is_virtual_node`]'s list, but they name
        // the same card and subdevice with a width of their own -- `front` is
        // two channels on an eight-channel HDA card -- and whichever of them
        // came last in the walk used to win the subdevice.
        if info.virtual_node || !info.channels_known || !is_hw_node(&info.name) {
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

    let mut asked: HashMap<(String, u16), Option<u16>> = HashMap::new();
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
        let queried = if info.name.starts_with("alsa:") {
            let key = (card.clone(), dev_index(&info.name).unwrap_or(0));
            *asked
                .entry(key)
                .or_insert_with_key(|(card, dev)| hw_channels(card, *dev))
        } else {
            None
        };
        let resolved = queried.or_else(|| match dev_index(&info.name) {
            Some(dev) => by_subdevice.get(&(card, dev)).copied(),
            None => lowest_subdevice.get(&card).map(|(_, channels)| *channels),
        });
        // No sibling for that subdevice means no honest answer, so it stays
        // unknown rather than falling back to a different subdevice's count.
        //
        // The node's own report caps it. A plug node reports cpal's clamp, so
        // the hardware's count is the answer; a `dmix` node reports the width
        // its slave was configured at -- `dmix:CARD=Headphones,DEV=0` is two
        // channels in front of an eight-channel `hw` node -- and opens at that
        // and nothing wider. Taking the sibling's count outright opened it at
        // eight, which ALSA refused.
        if let Some(channels) = resolved {
            info.max_channels = info.max_channels.min(channels);
            info.channels_known = true;
        }
    }
}

/// [`resolve_virtual_channel_counts`] against this machine's ALSA, at
/// `sample_rate` where one is configured.
///
/// Must run while the virtual node is not held: ALSA will not hand out the
/// `hw` node's hw_params while a plug in front of it has it open.
fn resolve_virtual_channel_counts_on_system(
    infos: &mut [AudioDeviceInfo],
    sample_rate: Option<u32>,
) {
    resolve_virtual_channel_counts(infos, |card, dev| alsa_hw_channels(card, dev, sample_rate));
}

/// The most channels `hw:CARD=<card>,DEV=<dev>` offers for playback, from
/// ALSA's hw_params, at `sample_rate` where the node accepts it.
///
/// The one channel count that does not depend on cpal knowing the node's
/// formats: a Behringer WING's raw node is `S24_3LE` only, which cpal cannot
/// use, so it never lists and its plug node had nothing to borrow from. Opened
/// non-blocking and closed at once; a node that is busy (EBUSY: something is
/// playing through it), absent, or refuses is `None`, never a guess.
#[cfg(target_os = "linux")]
fn alsa_hw_channels(card: &str, dev: u16, sample_rate: Option<u32>) -> Option<u16> {
    let node = format!("hw:CARD={card},DEV={dev}");
    let query = || -> Result<u32, alsa::Error> {
        let pcm = alsa::PCM::new(&node, alsa::Direction::Playback, true)?;
        let params = alsa::pcm::HwParams::any(&pcm)?;
        if let Some(rate) = sample_rate {
            // Narrows the space where the rate is offered; where it is not,
            // the node's overall maximum is still the honest answer.
            let _ = params.set_rate(rate, alsa::ValueOr::Nearest);
        }
        params.get_channels_max()
    };
    let result = {
        // alsa-lib writes its own complaints to stderr on a failed open.
        let _silenced = silence_enumeration().ok();
        query()
    };
    match result {
        Ok(channels) => {
            debug!(node = %node, channels, "Read hardware channel count from ALSA");
            Some(u16::try_from(channels).unwrap_or(u16::MAX))
        }
        Err(e) => {
            debug!(node = %node, err = %e, "Could not read hardware channel count from ALSA");
            None
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn alsa_hw_channels(_card: &str, _dev: u16, _sample_rate: Option<u32>) -> Option<u16> {
    None
}

/// What finding a virtual node's width needs from the system, so that the
/// order and the cost of it can be tested.
trait NodeProbe {
    /// Whether a device of this name is listed at all. Name only: nothing is
    /// asked of any device, so a device still missing costs no probing.
    fn exists(&mut self, name: &str) -> Result<bool, Box<dyn Error>>;
    /// The listing entries of `name` and of the raw `hw` nodes on `card`,
    /// probed and released; no other device is asked anything.
    fn card_nodes(
        &mut self,
        name: &str,
        card: &str,
    ) -> Result<Vec<AudioDeviceInfo>, Box<dyn Error>>;
    /// See [`alsa_hw_channels`].
    fn hw_channels(&mut self, card: &str, dev: u16) -> Option<u16>;
}

/// [`NodeProbe`] on this machine.
struct SystemProbe {
    sample_rate: Option<u32>,
}

impl NodeProbe for SystemProbe {
    fn exists(&mut self, name: &str) -> Result<bool, Box<dyn Error>> {
        let mut exists = false;
        walk_output_devices(|_, _, device_id| {
            if device_id.trim() == name.trim() {
                exists = true;
                return ControlFlow::Break(());
            }
            ControlFlow::Continue(())
        })?;
        Ok(exists)
    }

    fn card_nodes(
        &mut self,
        name: &str,
        card: &str,
    ) -> Result<Vec<AudioDeviceInfo>, Box<dyn Error>> {
        let mut infos = Vec::new();
        walk_output_devices(|host_id, device, device_id| {
            let wanted = device_id.trim() == name.trim()
                || (is_hw_node(&device_id) && card_token(&device_id).as_deref() == Some(card));
            if wanted {
                if let Some(configs) = output_configs(&device) {
                    infos.push(device_info(host_id, device_id, &configs));
                }
            }
            ControlFlow::Continue(())
        })?;
        Ok(infos)
    }

    fn hw_channels(&mut self, card: &str, dev: u16) -> Option<u16> {
        alsa_hw_channels(card, dev, self.sample_rate)
    }
}

/// A virtual node's listing entry, its width resolved against only its own
/// card: `None` if no such device is listed.
///
/// Name first, and nothing probed until the name exists: `retry_until_ready`
/// asks for a missing device twice a second, at boot until the WING appears,
/// and a full listing calls `supported_output_configs()` on every device of
/// every host. Then only the node and its card's `hw` nodes are probed, and
/// ALSA asked for the `hw` node's width, all before the node is held.
fn listed_virtual_node(
    name: &str,
    card: &str,
    probe: &mut impl NodeProbe,
) -> Result<Option<AudioDeviceInfo>, Box<dyn Error>> {
    if !probe.exists(name)? {
        return Ok(None);
    }
    let mut infos = probe.card_nodes(name, card)?;
    resolve_virtual_channel_counts(&mut infos, |card, dev| probe.hw_channels(card, dev));
    Ok(infos
        .into_iter()
        .find(|info| info.name.trim() == name.trim()))
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
    /// Whether `max_channels` is the real width behind the device -- hardware
    /// reporting under cpal's clamp, or a virtual node whose hardware was
    /// read -- rather than a plugin's or cpal's figure.
    channels_known: bool,
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
    /// Takes in the next pass of a crossfading song loop if its build has
    /// finished.
    fn collect_pass(song_loop: &mut SongLoopCrossfade) {
        if let Some((from, rx)) = song_loop.build_rx.take() {
            match rx.try_recv() {
                Ok(Ok((sources, took))) => {
                    song_loop.prebuilt = Some((from, sources));
                    // A slow build is remembered for a few more: a fast
                    // one (its files cached) is no promise the next will be.
                    let remembered = song_loop
                        .build_took
                        .map_or(Duration::ZERO, |slowest| slowest.mul_f64(0.75));
                    song_loop.build_took = Some(took.max(remembered));
                }
                Ok(Err(e)) => {
                    error!(err = %e, "Failed to pre-build the next loop pass; will retry")
                }
                Err(mpsc::TryRecvError::Empty) => song_loop.build_rx = Some((from, rx)),
                Err(mpsc::TryRecvError::Disconnected) => {}
            }
        }
    }

    /// Where in the song the pass built, or building, starts.
    fn pass_from(song_loop: &SongLoopCrossfade) -> Option<Duration> {
        song_loop
            .prebuilt
            .as_ref()
            .map(|(from, _)| *from)
            .or(song_loop.build_rx.as_ref().map(|(from, _)| *from))
    }

    /// Keeps a pass begun late (inside the crossfade) on a build that can
    /// make it: one built, or building, from a point playback has yet to
    /// reach is joined there rather than thrown away for the plan of the
    /// moment; and with nothing building, a plan too near to build for, as
    /// long as builds have been taking, is made again further ahead. Gives
    /// the pass's target on the playback timeline, if it moved.
    #[allow(clippy::too_many_arguments)]
    fn plan_late_pass(
        song_loop: &mut SongLoopCrossfade,
        section: &crate::section_loop::EffectiveSection,
        trigger: &mut crate::section_loop::SectionLoopTrigger,
        effective: Option<&crate::section_loop::EffectiveSection>,
        elapsed: Duration,
        lookahead: Duration,
    ) -> Option<Duration> {
        Self::collect_pass(song_loop);
        let needs = song_loop
            .build_took
            .map(|took| took * 3 / 2 + lookahead)
            .unwrap_or_default();
        trigger.join_ahead_by(needs);
        let target = trigger.arm(effective, elapsed)?;
        let planned = trigger.past_end(target);
        match Self::pass_from(song_loop) {
            // Only a pass begun late: one on time builds from the top.
            Some(built)
                if !planned.is_zero()
                    && built != planned
                    && built <= section.overlap
                    && trigger
                        .join_time(built)
                        .is_some_and(|join| join >= elapsed + lookahead) =>
            {
                trigger.join_from(built);
            }
            None if !planned.is_zero() && target < elapsed + needs => {
                trigger.reset();
            }
            _ => {}
        }
        trigger.arm(effective, elapsed)
    }

    /// Has the next pass of a crossfading song loop built from `from` in
    /// the song (the top, or part way in for one begun late), if it is not
    /// built or building already. One built or building from elsewhere is
    /// dropped.
    fn prebuild_pass(
        song: &Arc<Song>,
        song_loop: &mut SongLoopCrossfade,
        playback_context: &crate::audio::PlaybackContext,
        mappings: &HashMap<String, Vec<u16>>,
        from: Duration,
    ) {
        Self::collect_pass(song_loop);
        if song_loop
            .prebuilt
            .as_ref()
            .is_some_and(|(at, _)| *at != from)
        {
            song_loop.prebuilt = None;
        }
        // One build at a time: one under way from elsewhere is seen out
        // (a build cannot be stopped part way) before another starts.
        if song_loop.prebuilt.is_some() || song_loop.build_rx.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        let song = song.clone();
        let context = playback_context.clone();
        let mappings = mappings.clone();
        let _ = thread::Builder::new()
            .name("mtrack-loop-pass-prebuild".into())
            .spawn(move || {
                let started = std::time::Instant::now();
                let built = build_pass_sources(
                    &song,
                    &context,
                    &mappings,
                    from,
                    from,
                    song.loop_crossfade_cut(),
                );
                let _ = tx.send(built.map(|sources| (sources, started.elapsed())));
            });
        song_loop.build_rx = Some((from, rx));
    }

    /// Sees out a loop break's fade that the pass playing out does not, as
    /// `carry` plans: the pass's sources (`pass_ids`, those of them fading)
    /// cross out over its last overlap, and as many more passes as it takes
    /// start from the top and carry the fade on. The fade's excluded tracks,
    /// cut at the break, stay out. The new sources join the ones a stop
    /// cuts.
    #[allow(clippy::too_many_arguments)]
    fn carry_break_fade(
        &self,
        song: &Arc<Song>,
        playback_context: &crate::audio::PlaybackContext,
        mappings: &HashMap<String, Vec<u16>>,
        cancel_handle: &CancelHandle,
        pass_ids: &[u64],
        carry: crate::audio::song_loop::BreakCarry,
        timing: &crate::audio::song_loop::BreakCarryTiming,
        fade: &crate::playsync::BreakFade,
        break_fade_ids: &parking_lot::Mutex<Vec<u64>>,
    ) {
        use crate::audio::crossfade::GainEnvelope;
        let mixer = &self.output_manager.mixer;
        // The pass playing out keeps its own envelope (a crossover under
        // way goes on); its break fade becomes the one that crosses it out.
        // The fade's excluded tracks, cut at the break, stay out.
        if let Some(outgoing) = &carry.outgoing {
            let fading = break_fade_ids.lock().clone();
            let ids: Vec<u64> = pass_ids
                .iter()
                .filter(|id| fading.contains(id))
                .copied()
                .collect();
            mixer.replace_break_envelopes(
                &ids,
                &GainEnvelope::piecewise(outgoing.clone().into()).with_start_sample(timing.now),
                |labels| !crate::audio::song_loop::only_tracks_in(labels, &fade.exclude),
            );
        }
        info!(
            song = song.name(),
            passes = carry.passes.len(),
            "Loop break: carrying the fade-out into more passes"
        );
        for (start, knots) in &carry.passes {
            let sources = match build_pass_sources(
                song,
                playback_context,
                mappings,
                Duration::ZERO,
                Duration::ZERO,
                &fade.exclude,
            ) {
                Ok(sources) => sources,
                Err(e) => {
                    error!(err = %e, "Failed to carry a loop break's fade into the next pass");
                    return;
                }
            };
            let knots: Arc<[(u64, f32)]> = knots.clone().into();
            let start = (*start).max(mixer.current_sample());
            for (source, group) in sources {
                if group == PassGroup::Cut {
                    continue;
                }
                let envelope = GainEnvelope::piecewise(knots.clone()).with_start_sample(start);
                let (mut active_source, _) =
                    build_active_source(source, mappings, cancel_handle, Some(Arc::new(envelope)));
                active_source.start_at_sample = Some(start);
                break_fade_ids.lock().push(active_source.id);
                self.output_manager.add_source_now(active_source);
            }
        }
    }

    /// Ends a crossfading song loop's playback mid-crossfade: a pass not yet
    /// started never starts, and, unless the player is already fading
    /// everything out for a loop break, the outgoing pass -- whose fade-out
    /// would otherwise keep it playing through a cancel for the rest of a
    /// long overlap -- fades over the short crossfade instead. Only this
    /// song's sources are touched.
    fn end_song_loop_crossfade(
        &self,
        song_loop: &SongLoopCrossfade,
        crossfade_samples: u64,
        loop_broken: bool,
    ) {
        use crate::audio::crossfade::{CrossfadeCurve, GainEnvelope};
        let mixer = &self.output_manager.mixer;
        let now_sample = mixer.current_sample();
        // A loop break lets a pass already scheduled start, under its fade:
        // the notes and lights make that jump too.
        if let Some((start_sample, ids)) = &song_loop.latest_start {
            if !loop_broken && now_sample < *start_sample {
                mixer.set_cancel_at_sample(ids, now_sample.max(1));
            }
        }
        if !loop_broken && !song_loop.outgoing_ids.is_empty() {
            mixer.set_gain_envelope(
                &song_loop.outgoing_ids,
                Arc::new(GainEnvelope::fade_out(
                    crossfade_samples,
                    CrossfadeCurve::Linear,
                )),
            );
        }
    }

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
        // Enumerate first, and build nothing while doing it.
        //
        // Constructing the placeholder `OutputManager` inside this walk reserves
        // audio resources partway through it, and ALSA then refuses to describe
        // the devices not yet visited: `supported_output_configs()` comes back
        // empty for them, they are dropped, and the list ends up a subset of
        // what the web UI's picker (which only queries) advertises. On the test
        // rig that was 8 devices against 19, and choosing one of the missing 11
        // failed with "no device found with name ..." (#357).
        let mut found: Vec<ResolvedOutputDevice> = Vec::new();
        walk_output_devices(|host_id, device, name| {
            if let Some(configs) = output_configs(&device) {
                found.push(ResolvedOutputDevice {
                    name,
                    host_id,
                    device,
                    configs,
                });
            }
            ControlFlow::Continue(())
        })?;

        // Now build. The manager here is a placeholder that `get` replaces, so
        // it holds nothing anyone depends on -- it just must not exist while the
        // enumeration above is still running.
        //
        // The listing's channel counts, resolved the same way, from the
        // configurations read above rather than another walk.
        let mut infos: Vec<AudioDeviceInfo> = found
            .iter()
            .map(|resolved| device_info(resolved.host_id, resolved.name.clone(), &resolved.configs))
            .collect();
        // The walk above holds every device, so ALSA's answer here is often
        // "busy" and the cpal siblings stand in; `get` opens through
        // `find_cpal_device`, which asks before holding anything.
        resolve_virtual_channel_counts_on_system(&mut infos, None);
        let mut devices = found
            .into_iter()
            .zip(&infos)
            .map(|(resolved, info)| {
                let reported = max_output_channels(&resolved.configs);
                let mut device =
                    Device::placeholder(resolved, open_channel_count(Some(info), reported))?;
                device.channels_known = info.channels_known;
                Ok::<_, Box<dyn Error>>(device)
            })
            .collect::<Result<Vec<Device>, _>>()?;
        devices.sort_by_key(|device| device.name.to_string());
        Ok(devices)
    }

    /// A `Device` for a resolved output device, with a placeholder format
    /// and output manager that `get` replaces: what a listing holds, and
    /// what a find hands to `get`.
    ///
    /// `max_channels` is the width it is opened at, from [`open_channel_count`].
    fn placeholder(
        resolved: ResolvedOutputDevice,
        max_channels: u16,
    ) -> Result<Device, Box<dyn Error>> {
        let default_format = TargetFormat::new(44100, SampleFormat::Int, 32)?;
        let output_manager = Arc::new(OutputManager::new(
            max_channels,
            default_format.sample_rate,
        )?);
        Ok(Device {
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
            channels_known: false,
        })
    }

    /// Whether a device of this name can be located for opening.
    ///
    /// The cheap half of [`Device::get`]: it starts no output thread, so it can
    /// be asked about every advertised device without disturbing a rig. It is
    /// not the whole of [`Device::find_cpal_device`]: it matches the name and
    /// asks only that device for output configurations, and does not resolve
    /// a virtual node's width (no hardware sibling, no ALSA query). A name it
    /// finds, `find_cpal_device` finds too.
    pub fn is_findable(name: &str) -> bool {
        matches!(resolve_output_device(name), Ok(Some(_)))
    }

    /// Finds one output device by name and wraps it in a `Device`, at the width
    /// it is opened with.
    ///
    /// See [`resolve_output_device`] for why this scans rather than searching
    /// `list_cpal_devices()`.
    ///
    /// A virtual node naming a card takes its width from
    /// [`listed_virtual_node`], which reads it from the hardware behind the
    /// node before the node is held -- ALSA will not describe the hardware
    /// while a plug in front of it is open. A node it does not find is not
    /// there to open either.
    fn find_cpal_device(
        name: &str,
        sample_rate: Option<u32>,
    ) -> Result<Option<Device>, Box<dyn Error>> {
        let virtual_node = is_virtual_node(name);
        let listed = match card_token(name).filter(|_| virtual_node) {
            Some(card) => {
                match listed_virtual_node(name, &card, &mut SystemProbe { sample_rate })? {
                    Some(info) => Some(info),
                    None => return Ok(None),
                }
            }
            None => None,
        };
        let Some(resolved) = resolve_output_device(name)? else {
            return Ok(None);
        };
        let reported = max_output_channels(&resolved.configs);
        let channels = open_channel_count(listed.as_ref(), reported);
        let channels_known = match &listed {
            Some(info) => info.channels_known,
            None => channels_are_known(virtual_node, reported),
        };
        if channels != reported {
            info!(
                device = %resolved.name,
                channels,
                reported,
                "Opening a virtual node at the width of the hardware behind it"
            );
        } else if virtual_node && !channels_known && reported >= CPAL_CHANNEL_CLAMP {
            warn!(
                device = %resolved.name,
                channels,
                "Could not read the channel count of the hardware behind this virtual \
                 node (busy, absent, or no card named); opening it at cpal's clamped \
                 width, and track mappings are not checked against the hardware"
            );
        }
        let mut device = Device::placeholder(resolved, channels)?;
        device.channels_known = channels_known;
        Ok(Some(device))
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
        let found = Device::find_cpal_device(&name, Some(config.sample_rate()))
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
                let period_ms = crate::audio::health::callback_period_ms(
                    Some(s),
                    device.target_format.sample_rate,
                )
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
            match config.stream_buffer_size() {
                None => PeriodSetting::BufferSize,
                Some(StreamBufferSize::Fixed(_)) => PeriodSetting::Fixed,
                Some(StreamBufferSize::Default) => PeriodSetting::BackendDefault,
                Some(StreamBufferSize::Min) if min_size.is_some() => PeriodSetting::Min,
                Some(StreamBufferSize::Min) => PeriodSetting::MinFallback,
            },
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
use crate::audio::song_loop::PassGroup;

type GroupedSources = Vec<(
    Box<dyn crate::audio::sample_source::ChannelMappedSampleSource>,
    PassGroup,
)>;

/// One pass of a looping song's sources, in its two groups: the fading tracks
/// from `fade_from`, the cut tracks from `cut_from`. A file holding tracks of
/// both groups is opened once for each, each copy carrying only its group's
/// tracks; a copy left carrying none is dropped.
fn build_pass_sources(
    song: &Song,
    context: &crate::audio::PlaybackContext,
    mappings: &HashMap<String, Vec<u16>>,
    fade_from: Duration,
    cut_from: Duration,
    cut: &[String],
) -> Result<GroupedSources, String> {
    let in_group = |group: PassGroup| -> HashMap<String, Vec<u16>> {
        mappings
            .iter()
            .filter(|(track, _)| cut.contains(track) == (group == PassGroup::Cut))
            .map(|(track, channels)| (track.clone(), channels.clone()))
            .collect()
    };
    let mut grouped = Vec::new();
    for (group, from) in [(PassGroup::Fade, fade_from), (PassGroup::Cut, cut_from)] {
        let group_mappings = in_group(group);
        if group_mappings.is_empty() {
            continue;
        }
        let sources = song
            .create_channel_mapped_sources_from(context, from, &group_mappings)
            .map_err(|e| e.to_string())?;
        grouped.extend(
            sources
                .into_iter()
                .filter(|s| s.channel_mappings().iter().any(|labels| !labels.is_empty()))
                .map(|s| (s, group)),
        );
    }
    Ok(grouped)
}

/// The state of a looping song's crossfade between passes (`loop_crossfade`):
/// the current pass's sources by group, and the next pass's, built ahead.
struct SongLoopCrossfade {
    /// How long passes overlap.
    overlap: Duration,
    /// The current pass's sources, by group.
    fade_ids: Vec<u64>,
    cut_ids: Vec<u64>,
    /// The next pass's sources, built and waiting for the boundary.
    prebuilt: Option<(Duration, GroupedSources)>,
    /// A build of the next pass's sources in flight.
    build_rx: Option<PassBuild>,
    /// How long builds have been taking, timed where they are built: the
    /// latest, or a slower one before it, fading by a quarter a build.
    build_took: Option<Duration>,
    /// The sample the latest pass starts at, and its sources' ids, to cancel
    /// them outright if playback ends before they have started.
    latest_start: Option<(u64, Vec<u64>)>,
    /// Whether a missing build at the boundary has been reported.
    warned_late: bool,
    /// The outgoing pass's sources, still fading while the next plays.
    outgoing_ids: Vec<u64>,
}

/// How far ahead of a pass boundary its sources are built: long enough for a
/// cold file open, seek and buffer warmup.
const PASS_PREBUILD_AHEAD: Duration = Duration::from_secs(5);

/// A pass being built: from where in the song, and where its sources come,
/// with how long they took to build.
type PassBuild = (
    Duration,
    mpsc::Receiver<Result<(GroupedSources, Duration), String>>,
);

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
        break_envelope: None,
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

    fn channels(&self) -> Option<u16> {
        self.channels_known.then_some(self.max_channels)
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
            break_fade,
            break_fade_ids,
            start_fade,
            section_jumps,
        } = loop_control;
        // Only for the song it was meant for, from the top.
        let start_fade = start_fade
            .lock()
            .take()
            .filter(|fade| fade.song == song.name() && start_time.is_zero())
            .map(|fade| fade.fade);
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
        // A song that loops with a crossfade keeps its cut tracks in sources of
        // their own, so each group can cross the pass boundary its own way.
        let mut song_loop = song.loop_crossfade().map(|overlap| SongLoopCrossfade {
            overlap,
            fade_ids: Vec::new(),
            cut_ids: Vec::new(),
            prebuilt: None,
            build_rx: None,
            build_took: None,
            latest_start: None,
            warned_late: false,
            outgoing_ids: Vec::new(),
        });
        // How the song goes round as a whole when no section loop is armed:
        // passes the trigger times as it times a section's jumps.
        let whole_song = crate::section_loop::WholeSongLoop::of(&song, song.duration());
        // The tracks a pass switches over at its crossfade's midpoint: every
        // build of the song's sources keeps them apart, a section loop's
        // too, so a pass after one crosses each group its own way.
        let cut_tracks: Vec<String> = if song_loop.is_some() {
            song.loop_crossfade_cut().to_vec()
        } else {
            Vec::new()
        };
        let channel_mapped_sources: GroupedSources = if song_loop.is_some() {
            build_pass_sources(
                &song,
                &playback_context,
                mappings,
                start_time,
                start_time,
                song.loop_crossfade_cut(),
            )
            .map_err(crate::audio::AudioError::Playback)?
        } else {
            song.create_channel_mapped_sources_from(&playback_context, start_time, mappings)
                .map_err(|e| crate::audio::AudioError::Playback(e.to_string()))?
                .into_iter()
                .map(|s| (s, PassGroup::Fade))
                .collect()
        };

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

        // Starting under a song a loop break is fading out, with a playlist
        // `fade_in`: everything but its excluded tracks fades in over it, each
        // source on an envelope of its own.
        let start_fade_samples = start_fade.as_ref().map(|fade| {
            crate::audio::duration_to_samples(fade.length, self.output_manager.mixer.sample_rate())
        });
        for (source, group) in channel_mapped_sources.into_iter() {
            let envelope = match (&start_fade, start_fade_samples) {
                (Some(fade), Some(samples))
                    if !crate::audio::song_loop::only_tracks_in(
                        source.channel_mappings(),
                        &fade.exclude,
                    ) =>
                {
                    Some(Arc::new(crate::audio::crossfade::GainEnvelope::fade_in(
                        samples, fade.curve,
                    )))
                }
                _ => song_crossfade_envelope
                    .as_ref()
                    .map(|e| Arc::new(e.fresh())),
            };
            let (active_source, flag) =
                build_active_source(source, mappings, &cancel_handle, envelope);
            if let Some(song_loop) = song_loop.as_mut() {
                match group {
                    PassGroup::Fade => song_loop.fade_ids.push(active_source.id),
                    PassGroup::Cut => song_loop.cut_ids.push(active_source.id),
                }
            }
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
        let audio_clock_skew_samples = crate::audio::duration_to_samples(
            clock.elapsed(),
            self.output_manager.mixer.sample_rate(),
        );
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

        // Playing now, the audio commits the playback's loop jumps, section
        // and whole-song alike, which the notes, lights and beat clock follow.
        let publishes_jumps = section_jumps.claim(crate::playsync::JumpPublisherRole::Audio);

        let mut scheduler = loop_scheduler::LoopScheduler::new(
            loop_scheduler::LoopContext {
                device: self,
                song: &song,
                playback_context: &playback_context,
                mappings,
                cancel_handle: &cancel_handle,
                loop_break: &loop_break,
                section_jumps: publishes_jumps.then_some(&*section_jumps),
                // Must exceed the 10ms poll sleep below so the trigger is
                // always detected *before* the ideal loop boundary; the
                // crossfade is then scheduled at the exact sample via
                // `start_at_sample`/`cancel_at_sample`, eliminating polling
                // jitter from the audible loop point. The cost is a small
                // race window for break-after-trigger, which we accept.
                lookahead: Duration::from_millis(30),
                skew_samples: audio_clock_skew_samples,
                crossfade_samples,
                cut_tracks,
            },
            song_loop,
            source_finish_flags,
        );

        loop {
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

            let armed = active_section.read().clone();
            let break_requested = section_loop_break.load(Ordering::Relaxed);
            // The one section looping now: the armed one, or else the whole
            // song for a song that loops (see `effective_section`).
            let effective = crate::section_loop::effective_section(
                armed,
                break_requested,
                whole_song,
                loop_break.load(Ordering::Relaxed),
            );

            // Check if all sources have finished (EOF). A song going round as
            // a whole, or a section looping, waits for its next time round
            // instead, should it not be built in time.
            let loops = effective
                .as_ref()
                .is_some_and(|section| section.bounds.end_time > section.bounds.start_time);
            if scheduler.all_finished() && !loops {
                break;
            }

            // Section bounds are song-absolute; the clock counts from this
            // playback's start. Without the offset, a loop armed after
            // starting mid-song (seek, section chip) fires every boundary
            // `start_time` too late -- the audio runs past the section end
            // while the UI wraps on the correct grid.
            let elapsed = start_time + clock.elapsed();
            match scheduler.poll(effective.as_ref(), elapsed) {
                loop_scheduler::Poll::Scheduled(scheduled) => {
                    debug!(
                        at = ?scheduled.jump.at,
                        start_sample = scheduled.start_sample,
                        "Audio loop: jump committed"
                    );
                }
                loop_scheduler::Poll::Idle => thread::sleep(Duration::from_millis(10)),
                loop_scheduler::Poll::Stop => break,
            }
        }
        let (song_loop, pass_end) = scheduler.finish();

        // Playback ending in the middle of a crossfade: the outgoing pass's
        // fade-out would keep it playing for the rest of a multi-second fade,
        // and the incoming pass could still be waiting to start. Cut both
        // short, as a loop break or stop does any other loop.
        if let Some(song_loop) = song_loop.as_ref() {
            let loop_broken = loop_break.load(Ordering::Relaxed);
            if cancel_handle.is_cancelled() || loop_broken {
                self.end_song_loop_crossfade(song_loop, crossfade_samples, loop_broken);
            }
        }

        // When section_loop_break is set, clear active_section so the song
        // continues normally past the section end.
        if section_loop_break.load(Ordering::Relaxed) {
            let mut section = active_section.write();
            *section = None;
        }

        // A loop break fading the song out (a playlist `fade_out`) over longer than
        // its audible pass has left: carry the fade on into the next pass, so
        // the song does not fall silent part way through.
        let fade = break_fade.lock().clone();
        if let (true, Some(fade)) = (loop_break.load(Ordering::Relaxed), fade) {
            let samples = |d: Duration| crate::audio::duration_to_samples(d, mixer_sample_rate);
            let now_sample = self.output_manager.mixer.current_sample();
            let break_sample = now_sample.saturating_sub(samples(fade.started.elapsed()));
            // The pass playing out, and its sources: the latest, a jump
            // scheduled before the break made under its fade.
            let pass_ids: Vec<u64> = match song_loop.as_ref() {
                Some(song_loop) => song_loop
                    .fade_ids
                    .iter()
                    .chain(&song_loop.cut_ids)
                    .copied()
                    .collect(),
                None => break_fade_ids.lock().clone(),
            };
            let timing = crate::audio::song_loop::BreakCarryTiming {
                now: now_sample,
                break_sample,
                fade: samples(fade.length),
                curve: fade.curve,
                pass_end: crate::section_loop::loop_boundary_sample(
                    now_sample,
                    start_time + clock.elapsed(),
                    pass_end,
                    mixer_sample_rate,
                )
                .saturating_add(audio_clock_skew_samples),
                pass_len: samples(song.duration()),
                overlap: song_loop.as_ref().map_or(0, |l| samples(l.overlap)),
                sample_rate: mixer_sample_rate,
            };
            if let Some(carry) = crate::audio::song_loop::plan_break_carry(timing) {
                self.carry_break_fade(
                    &song,
                    &playback_context,
                    mappings,
                    &cancel_handle,
                    &pass_ids,
                    carry,
                    &timing,
                    &fade,
                    &break_fade_ids,
                );
            }
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

        /// No ALSA to ask, as for a node cpal lists from another host.
        fn no_alsa(_card: &str, _dev: u16) -> Option<u16> {
            None
        }

        /// A [`NodeProbe`] over a fixed listing, counting what it is asked.
        struct FakeProbe {
            listing: Vec<AudioDeviceInfo>,
            hw: HashMap<(String, u16), u16>,
            card_walks: usize,
            hw_queries: usize,
        }

        impl NodeProbe for FakeProbe {
            fn exists(&mut self, name: &str) -> Result<bool, Box<dyn Error>> {
                Ok(self.listing.iter().any(|info| info.name == name))
            }
            fn card_nodes(
                &mut self,
                name: &str,
                card: &str,
            ) -> Result<Vec<AudioDeviceInfo>, Box<dyn Error>> {
                self.card_walks += 1;
                Ok(self
                    .listing
                    .iter()
                    .filter(|info| {
                        info.name == name
                            || (is_hw_node(&info.name)
                                && card_token(&info.name).as_deref() == Some(card))
                    })
                    .cloned()
                    .collect())
            }
            fn hw_channels(&mut self, card: &str, dev: u16) -> Option<u16> {
                self.hw_queries += 1;
                self.hw.get(&(card.to_string(), dev)).copied()
            }
        }

        fn fake(listing: Vec<AudioDeviceInfo>, hw: &[(&str, u16, u16)]) -> FakeProbe {
            FakeProbe {
                listing,
                hw: hw
                    .iter()
                    .map(|(card, dev, channels)| ((card.to_string(), *dev), *channels))
                    .collect(),
                card_walks: 0,
                hw_queries: 0,
            }
        }

        /// The band rig at boot: retried twice a second until the WING
        /// appears. A missing name costs the name check and nothing else.
        #[test]
        fn a_missing_device_is_not_probed() {
            let mut probe = fake(
                vec![info("alsa:hw:CARD=Headphones,DEV=0", 8)],
                &[("WING", 0, 48)],
            );
            let found =
                listed_virtual_node("alsa:plughw:CARD=WING,DEV=0", "WING", &mut probe).unwrap();
            assert!(found.is_none());
            assert_eq!(probe.card_walks, 0);
            assert_eq!(probe.hw_queries, 0);
        }

        /// The WING: its raw node never lists (cpal knows none of its
        /// formats), so cpal has nothing to borrow; ALSA's hw_params say 48.
        #[test]
        fn plug_node_takes_the_width_alsa_reads_from_its_hardware() {
            let mut probe = fake(
                vec![
                    info("alsa:plughw:CARD=WING,DEV=0", CPAL_CHANNEL_CLAMP),
                    info("alsa:hw:CARD=Headphones,DEV=0", 8),
                ],
                &[("WING", 0, 48)],
            );
            let plug = listed_virtual_node("alsa:plughw:CARD=WING,DEV=0", "WING", &mut probe)
                .unwrap()
                .expect("listed");
            assert_eq!(plug.max_channels, 48);
            assert!(plug.channels_known);
            assert_eq!(open_channel_count(Some(&plug), CPAL_CHANNEL_CLAMP), 48);
            assert_eq!(probe.card_walks, 1);
            assert_eq!(probe.hw_queries, 1);
        }

        /// ALSA busy or refusing: the cpal sibling stands in, and with none
        /// the node stays unknown at its own report.
        #[test]
        fn a_failed_alsa_query_falls_back_to_the_listed_sibling() {
            let mut probe = fake(
                vec![
                    info("alsa:hw:CARD=MAT,DEV=0", 8),
                    info("alsa:plughw:CARD=MAT,DEV=0", CPAL_CHANNEL_CLAMP),
                ],
                &[],
            );
            let plug = listed_virtual_node("alsa:plughw:CARD=MAT,DEV=0", "MAT", &mut probe)
                .unwrap()
                .expect("listed");
            assert_eq!(plug.max_channels, 8);

            let mut probe = fake(
                vec![info("alsa:plughw:CARD=WING,DEV=0", CPAL_CHANNEL_CLAMP)],
                &[],
            );
            let plug = listed_virtual_node("alsa:plughw:CARD=WING,DEV=0", "WING", &mut probe)
                .unwrap()
                .expect("listed");
            assert!(!plug.channels_known);
            assert_eq!(
                open_channel_count(Some(&plug), CPAL_CHANNEL_CLAMP),
                CPAL_CHANNEL_CLAMP
            );
        }

        /// The ALSA count caps a dmix node as the listed one does, and one
        /// query serves every node on the card.
        #[test]
        fn alsa_count_caps_dmix_and_is_asked_once_per_subdevice() {
            let mut infos = vec![
                info("alsa:dmix:CARD=Headphones,DEV=0", 2),
                info("alsa:plughw:CARD=Headphones,DEV=0", CPAL_CHANNEL_CLAMP),
                info("alsa:sysdefault:CARD=Headphones", CPAL_CHANNEL_CLAMP),
            ];
            let mut asked = 0;
            resolve_virtual_channel_counts(&mut infos, |_, _| {
                asked += 1;
                Some(8)
            });
            assert_eq!(asked, 1);
            assert_eq!(infos[0].max_channels, 2);
            assert_eq!(infos[1].max_channels, 8);
            assert_eq!(infos[2].max_channels, 8);
        }

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

        /// The harness Pi: `plughw:CARD=Headphones,DEV=0` reports cpal's clamp,
        /// and was opened at it -- 64 channels mixed for ALSA to drop all but
        /// the eight the hardware node behind it reports. It opens at eight, and
        /// a song mapped past them fails rather than going nowhere.
        #[test]
        fn plug_node_opens_at_the_width_of_its_hardware() {
            let mut infos = vec![
                info("alsa:hw:CARD=Headphones,DEV=0", 8),
                info("alsa:plughw:CARD=Headphones,DEV=0", CPAL_CHANNEL_CLAMP),
            ];
            resolve_virtual_channel_counts(&mut infos, no_alsa);
            let width = open_channel_count(Some(&infos[1]), CPAL_CHANNEL_CLAMP);
            assert_eq!(width, 8);

            let mut mappings = HashMap::new();
            mappings.insert("click".to_string(), vec![8]);
            assert!(validate_channel_count(&mappings, width, "s", "plug").is_ok());
            mappings.insert("bass".to_string(), vec![9]);
            let err = validate_channel_count(&mappings, width, "s", "plug")
                .unwrap_err()
                .to_string();
            assert!(err.contains("9 channels requested"), "{err}");
            assert!(err.contains("only has 8"), "{err}");
        }

        /// No hardware sibling to ask -- a WING's raw node never lists, and
        /// `default` names no card -- so the width stays what the node reports.
        #[test]
        fn plug_node_with_no_sibling_opens_at_its_own_report() {
            let mut infos = vec![
                info("alsa:plughw:CARD=WING,DEV=0", CPAL_CHANNEL_CLAMP),
                info("alsa:default", 32),
            ];
            resolve_virtual_channel_counts(&mut infos, no_alsa);
            assert_eq!(
                open_channel_count(Some(&infos[0]), CPAL_CHANNEL_CLAMP),
                CPAL_CHANNEL_CLAMP
            );
            assert_eq!(open_channel_count(Some(&infos[1]), 32), 32);
        }

        /// The harness Pi's `dmix:CARD=Headphones,DEV=0` reports two channels
        /// in front of an eight-channel `hw` node. It was opened at eight,
        /// borrowed from the sibling, and ALSA refused the hw_params: no audio.
        /// It opens at its own two, and lists them.
        #[test]
        fn dmix_node_keeps_its_own_narrower_width() {
            let mut infos = vec![
                info("alsa:hw:CARD=Headphones,DEV=0", 8),
                info("alsa:dmix:CARD=Headphones,DEV=0", 2),
                info("alsa:plughw:CARD=Headphones,DEV=0", CPAL_CHANNEL_CLAMP),
            ];
            resolve_virtual_channel_counts(&mut infos, no_alsa);

            assert_eq!(infos[1].max_channels, 2);
            assert!(infos[1].channels_known);
            assert_eq!(open_channel_count(Some(&infos[1]), 2), 2);
            // The plug node beside it still takes the hardware's eight.
            assert_eq!(infos[2].max_channels, 8);
        }

        /// `front:` names the same card and subdevice as `hw:` with a width of
        /// its own, and is not the hardware.
        #[test]
        fn only_the_hw_node_is_a_sibling() {
            let mut infos = vec![
                info("alsa:hw:CARD=PCH,DEV=0", 8),
                info("alsa:front:CARD=PCH,DEV=0", 2),
                info("alsa:plughw:CARD=PCH,DEV=0", CPAL_CHANNEL_CLAMP),
            ];
            resolve_virtual_channel_counts(&mut infos, no_alsa);
            assert_eq!(infos[2].max_channels, 8);
        }

        #[test]
        fn hardware_opens_at_its_own_report() {
            let hw = info("alsa:hw:CARD=MAT,DEV=0", 8);
            assert_eq!(open_channel_count(Some(&hw), 8), 8);
            assert_eq!(open_channel_count(None, 8), 8);
        }

        /// The case that motivated this: `plughw:CARD=MAT` claims cpal's clamped
        /// 32, while the hardware it routes to reports its real 8.
        #[test]
        fn plug_node_inherits_channel_count_from_hardware() {
            let mut infos = vec![
                info("alsa:hw:CARD=MAT,DEV=0", 8),
                info("alsa:plughw:CARD=MAT", CPAL_CHANNEL_CLAMP),
            ];
            resolve_virtual_channel_counts(&mut infos, no_alsa);

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
            resolve_virtual_channel_counts(&mut infos, no_alsa);

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
            resolve_virtual_channel_counts(&mut infos, no_alsa);

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
            resolve_virtual_channel_counts(&mut infos, no_alsa);

            assert_eq!(infos[2].max_channels, 8);
        }

        /// The WING: its raw node reports no cpal-known format, so it never
        /// appears and there is nothing to borrow a count from. The plug node
        /// must then say "virtual" rather than assert a fictional 32.
        #[test]
        fn plug_node_without_hardware_sibling_stays_unknown() {
            let mut infos = vec![info("alsa:plughw:CARD=WING", CPAL_CHANNEL_CLAMP)];
            resolve_virtual_channel_counts(&mut infos, no_alsa);

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
            resolve_virtual_channel_counts(&mut infos, no_alsa);

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
            resolve_virtual_channel_counts(&mut infos, no_alsa);

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

    mod section_at_tests {
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
                crate::player::SectionBounds::at(&song, Duration::from_millis(500)).map(|s| s.name),
                Some("verse".to_string())
            );
            assert_eq!(
                crate::player::SectionBounds::at(&song, Duration::from_millis(2500))
                    .map(|s| s.name),
                Some("chorus".to_string())
            );
        }

        #[test]
        fn boundary_is_half_open() {
            let song = song_with_sections();
            // Exactly at the verse/chorus boundary belongs to chorus: a
            // section owns [start, end), so the measure boundary is the start
            // of the next section, not the tail of the previous one.
            let at_boundary =
                crate::player::SectionBounds::at(&song, Duration::from_secs(2)).unwrap();
            assert_eq!(at_boundary.name, "chorus");
            assert_eq!(at_boundary.start_time, Duration::from_secs(2));
        }

        #[test]
        fn none_when_past_all_sections() {
            let song = song_with_sections();
            assert!(crate::player::SectionBounds::at(&song, Duration::from_secs(10)).is_none());
        }

        #[test]
        fn none_without_beat_grid() {
            // A plain test song has no beat grid, so nothing resolves.
            let song = Song::new_for_test("test", &["click"]);
            assert!(crate::player::SectionBounds::at(&song, Duration::from_secs(1)).is_none());
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
