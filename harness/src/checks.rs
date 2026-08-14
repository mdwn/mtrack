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
//! The checks, and the order they run in.

pub mod devices;
pub mod lighting;
pub mod midi_output;
pub mod persistence;
pub mod playback;
pub mod startup;
pub mod subsystems;

use std::future::Future;
use std::pin::Pin;

use crate::outcome::CheckOutcome;

/// One verification, and enough about it to report it well.
pub struct Check {
    pub area: &'static str,
    pub name: &'static str,
    /// What a failure of this check would mean, shown alongside the failure.
    pub description: &'static str,
    pub run: fn() -> Pin<Box<dyn Future<Output = CheckOutcome>>>,
}

macro_rules! entry {
    ($area:expr, $name:expr, $desc:expr, $path:path) => {
        Check {
            area: $area,
            name: $name,
            description: $desc,
            run: || Box::pin($path()),
        }
    };
}

/// Checks whose sabotage breaks the *world* the assertion observes, rather than
/// substituting the value it reads.
///
/// The distinction matters and is easy to lose. A world-level control breaks an
/// input and requires the check to notice; a predicate-level one swaps the
/// string or number the assertion compares. The second proves the assertion is
/// wired up and reachable, but **cannot detect the class of vacuity that
/// motivated the self-test**: if such a check were later made vacuous the way
/// `bogus_*_device_degrades_gracefully` was -- reading a value that is the same
/// whatever mtrack does -- a predicate-level control would still score
/// "assertion fired", because it replaces the value after the read.
///
/// The weaker class is the *default*: anything absent from this list counts as
/// predicate-level, so a new check, or one whose control is later weakened, is
/// treated as weak evidence until someone deliberately promotes it. Listing the
/// weak ones instead would let an omission overstate the score.
///
/// Absences are usually deliberate trade-offs.
/// `configured_midi_device_transmits` is predicate-level because its
/// world-level control pointed the profile at a bogus device and timed out
/// during startup, never reaching the assertion;
/// `plays_a_song_to_completion` because making a transport clock misbehave
/// needs a broken player. The self-test prints how many of its passes rest on
/// this class rather than burying it in one number.
const WORLD_LEVEL: &[&str] = &[
    "selected_device_actually_streams",
    "stop_halts_playback",
    "playlist_navigation_moves_between_songs",
    "tracks_route_to_their_mapped_channels",
    "beat_clock_is_silent_when_disabled",
    "beat_clock_holds_tempo_after_a_song_stops",
    "beat_clock_is_silent_after_stop_without_persist",
    "beat_clock_bridges_the_gap_between_songs",
    "stale_checksum_is_rejected",
    "active_playlist_persists_across_restart",
    "absent_midi_is_skipped_not_fatal",
    "absent_dmx_is_skipped_not_fatal",
    "bogus_midi_device_degrades_gracefully",
    "bogus_audio_device_degrades_gracefully",
    "unopenable_audio_device_is_distinguished_from_a_missing_one",
    "first_profile_wins",
    "generated_show_passes_validation",
    "malformed_show_is_rejected",
    "song_lighting_produces_cues",
    "lighting_effects_activate_during_playback",
];

/// Whether this check's negative control only substitutes the asserted value.
///
/// Unlisted means predicate-level, so a new check -- or one whose control is
/// later weakened -- is counted as weak evidence until someone says otherwise.
/// Listing the weak ones instead would let an omission silently overstate the
/// score, which is the exact failure this accounting exists to prevent.
pub fn is_predicate_level(name: &str) -> bool {
    !WORLD_LEVEL.contains(&name)
}

/// Names in [`WORLD_LEVEL`] that no longer match a registered check.
///
/// A stale entry credits a check that no longer exists, so it is reported.
/// The opposite direction needs no check: an unlisted name is already treated
/// as the weaker class.
pub fn stale_world_level_entries() -> Vec<&'static str> {
    let names: Vec<&str> = all().into_iter().map(|c| c.name).collect();
    WORLD_LEVEL
        .iter()
        .filter(|p| !names.contains(p))
        .copied()
        .collect()
}

/// Every check, in execution order.
///
/// Ordering is deliberate: cheap structural checks first, so a broken project
/// or an unopenable device is reported before minutes are spent playing audio.
pub fn all() -> Vec<Check> {
    vec![
        entry!(
            "devices",
            "advertised_devices_are_openable",
            "Devices offered by the API must be openable by the player, or the web UI\n  \
             lists devices that fail with \"no device found\" when selected.",
            devices::advertised_devices_are_openable
        ),
        entry!(
            "devices",
            "selected_device_actually_streams",
            "The device the suite plays through must open and run its callback, not\n  \
             merely resolve. A device can advertise itself, accept a name lookup, and\n  \
             still refuse the format.",
            devices::selected_device_actually_streams
        ),
        entry!(
            "devices",
            "selected_output_is_real_hardware",
            "The chosen output must be a real interface, not an ALSA plug node whose\n  \
             advertised channel count is fictional.",
            devices::selected_output_is_real_hardware
        ),
        entry!(
            "devices",
            "probing_the_device_in_use_reports_it_rather_than_failing",
            "Testing the device the player already holds must report it, not try to reopen\n  \
             it. A second open is refused, so the naive answer calls the working interface\n  \
             broken.",
            devices::probing_the_device_in_use_reports_it_rather_than_failing
        ),
        entry!(
            "startup",
            "player_starts_against_detected_hardware",
            "The player must start against a generated config and claim the device.",
            startup::player_starts_against_detected_hardware
        ),
        entry!(
            "startup",
            "generated_project_loads_all_songs",
            "Every generated song must load, or later failures are the harness's fault.",
            startup::generated_project_loads_all_songs
        ),
        entry!(
            "playback",
            "plays_a_song_to_completion",
            "The transport clock must advance monotonically and stop on its own.",
            playback::plays_a_song_to_completion
        ),
        entry!(
            "playback",
            "stop_halts_playback",
            "Stop must take effect mid-song.",
            playback::stop_halts_playback
        ),
        entry!(
            "playback",
            "playlist_navigation_moves_between_songs",
            "Next and previous must move through the playlist.",
            playback::playlist_navigation_moves_between_songs
        ),
        entry!(
            "audio-routing",
            "tracks_route_to_their_mapped_channels",
            "Each track must reach the physical output channel it was mapped to, and\n  \
             no other. Bleed across channels is what track-mapping bugs look like.",
            playback::tracks_route_to_their_mapped_channels
        ),
        entry!(
            "midi-transmit",
            "song_midi_notes_are_transmitted",
            "The notes in a song's MIDI file must appear on the wire, in order.",
            midi_output::song_midi_notes_are_transmitted
        ),
        entry!(
            "midi-transmit",
            "beat_clock_runs_at_the_song_tempo",
            "Beat clock must run at 24 pulses per quarter note at the song's tempo.",
            midi_output::beat_clock_runs_at_the_song_tempo
        ),
        entry!(
            "midi-transmit",
            "beat_clock_is_silent_when_disabled",
            "No timing clock may be transmitted when beat clock is off.",
            midi_output::beat_clock_is_silent_when_disabled
        ),
        entry!(
            "midi-transmit",
            "beat_clock_holds_tempo_after_a_song_stops",
            "With persist_tempo on, the clock must keep running at the song's tempo once\n  \
             the song stops, so downstream gear holds tempo between songs.",
            midi_output::beat_clock_holds_tempo_after_a_song_stops
        ),
        entry!(
            "midi-transmit",
            "beat_clock_is_silent_after_stop_without_persist",
            "With persist_tempo off, the clock must go silent once the song stops. The\n  \
             default must not change.",
            midi_output::beat_clock_is_silent_after_stop_without_persist
        ),
        entry!(
            "midi-transmit",
            "beat_clock_bridges_the_gap_between_songs",
            "Across a song change the clock must neither go silent nor stay at the old\n  \
             song's tempo.",
            midi_output::beat_clock_bridges_the_gap_between_songs
        ),
        entry!(
            "midi-config",
            "configured_midi_device_transmits",
            "The MIDI device in the profile must open and transmit. The byte-level checks\n  \
             use a loopback port, so without this the real interface is never exercised.",
            midi_output::configured_midi_device_transmits
        ),
        entry!(
            "midi-config",
            "midi_beat_clock_persists",
            "A MIDI setting must reach the profile file that owns it and survive a\n  \
             restart, rather than landing where the loader discards it.",
            persistence::midi_beat_clock_persists
        ),
        entry!(
            "midi-config",
            "stale_checksum_is_rejected",
            "Optimistic concurrency must stop two editors overwriting each other.",
            persistence::stale_checksum_is_rejected
        ),
        entry!(
            "persistence",
            "active_playlist_persists_across_restart",
            "The active playlist is documented as persisted; it must survive a restart.",
            persistence::active_playlist_persists_across_restart
        ),
        entry!(
            "subsystems",
            "absent_midi_is_skipped_not_fatal",
            "A profile without MIDI must run without it.",
            subsystems::absent_midi_is_skipped_not_fatal
        ),
        entry!(
            "subsystems",
            "absent_dmx_is_skipped_not_fatal",
            "A profile without DMX must run without it.",
            subsystems::absent_dmx_is_skipped_not_fatal
        ),
        entry!(
            "subsystems",
            "bogus_midi_device_degrades_gracefully",
            "An unresolvable MIDI device must degrade, not panic.",
            subsystems::bogus_midi_device_degrades_gracefully
        ),
        entry!(
            "subsystems",
            "bogus_audio_device_degrades_gracefully",
            "An unresolvable audio device must degrade, not panic.",
            subsystems::bogus_audio_device_degrades_gracefully
        ),
        entry!(
            "subsystems",
            "unopenable_audio_device_is_distinguished_from_a_missing_one",
            "A device that is present but will not open must say so, and must never\n  \
             report connected. Reporting it as missing sends an operator waiting out a\n  \
             retry that can never succeed.",
            subsystems::unopenable_audio_device_is_distinguished_from_a_missing_one
        ),
        entry!(
            "subsystems",
            "first_profile_wins",
            "The first matching profile must be the one applied.",
            subsystems::first_profile_wins
        ),
        entry!(
            "subsystems",
            "controllers_restart_while_idle",
            "Controllers must come back after a restart, or control is silently lost.",
            subsystems::controllers_restart_while_idle
        ),
        entry!(
            "lighting",
            "generated_show_passes_validation",
            "The generated show must satisfy mtrack's own validator.",
            lighting::generated_show_passes_validation
        ),
        entry!(
            "lighting",
            "malformed_show_is_rejected",
            "A malformed show must be rejected, or validation proves nothing.",
            lighting::malformed_show_is_rejected
        ),
        entry!(
            "lighting",
            "song_lighting_produces_cues",
            "A song's show must produce cues during playback.",
            lighting::song_lighting_produces_cues
        ),
        entry!(
            "lighting",
            "lighting_effects_activate_during_playback",
            "Effects must become active once the show starts.",
            lighting::lighting_effects_activate_during_playback
        ),
        entry!(
            "lighting",
            "show_written_via_api_is_readable",
            "A show written through the API must read back.",
            lighting::show_written_via_api_is_readable
        ),
    ]
}

/// The standard song set: a handful of short tone songs, enough to exercise
/// playlist navigation without making a run take minutes.
pub fn standard_songs() -> Vec<crate::songs::SongSpec> {
    use crate::songs::SongSpec;
    vec![
        SongSpec::tones("E2E Tones A", "e2e-tones-a", 4, 6.0),
        SongSpec::tones("E2E Tones B", "e2e-tones-b", 2, 4.0),
        SongSpec::tones("E2E Short", "e2e-short", 1, 2.0),
    ]
}

/// A project against the detected hardware with the standard song set.
pub fn standard_project() -> Result<crate::project::Project, Box<dyn std::error::Error>> {
    crate::project::ProjectBuilder::new()
        .songs(standard_songs())
        .build()
}
