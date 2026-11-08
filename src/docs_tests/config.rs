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
//! The configuration docs' tables of keys and defaults against the
//! defaults the config structs take: the player and profile keys, the
//! OSC addresses, the trigger inputs, the metronome and the samples.

use std::collections::BTreeMap;
use std::time::Duration;

use serde_json::json;

use super::stated;
use crate::config::metronome::{
    validate_volume, DEFAULT_ACCENT_FREQ, DEFAULT_HALF_FREQ, DEFAULT_MASTER_VOLUME,
    DEFAULT_NORMAL_FREQ, DEFAULT_SUB_FREQ,
};
use crate::config::samples::{SampleDefinition, DEFAULT_VELOCITY};
use crate::config::trigger::AudioTriggerInput;
use crate::config::{
    Audio, Dmx, MetronomeConfig, MetronomeDefaults, Midi, OscController, Player, Profile,
};
use crate::testutil::markdown::{plain, table, tables_headed, Table};

const PLAYER_CONFIG_MD: &str = include_str!("../../docs/src/configuration/player-config.md");
const OSC_MD: &str = include_str!("../../docs/src/interfaces/osc.md");
const TRIGGERS_MD: &str = include_str!("../../docs/src/configuration/triggers.md");
const SONG_CONFIG_MD: &str = include_str!("../../docs/src/configuration/song-config.md");
const SAMPLES_MD: &str = include_str!("../../docs/src/configuration/samples.md");

/// A value as serde writes it: an enum's keyword.
fn keyword(value: impl serde::Serialize) -> String {
    match serde_json::to_value(value).unwrap() {
        serde_json::Value::String(s) => s,
        other => other.to_string(),
    }
}

/// A duration as the tables write a zero one.
fn secs(d: Duration) -> String {
    d.as_secs_f64().to_string()
}

/// One row of a key table: the key, its Default cell, and its Required
/// cell where the table has that column.
struct Row {
    key: String,
    default: String,
    required: Option<String>,
}

/// The rows of `table`, a key table with a `Default` column.
fn rows(table: &Table) -> Vec<Row> {
    let default = table.column("Default");
    let required = table.header.iter().position(|h| h == "Required");
    table
        .rows
        .iter()
        .map(|row| Row {
            key: plain(&row[0]),
            default: plain(&row[default]),
            required: required.map(|c| plain(&row[c])),
        })
        .collect()
}

/// What a key table says of the keys whose default is not a value.
struct NoValue<'a> {
    /// Keys with no default at all: their Default cell is a dash.
    none: &'a [&'a str],
    /// Keys whose default the table describes in words, with the words.
    described: &'a [(&'a str, &'a str)],
}

/// The cell a key without a default has.
const NO_DEFAULT: &str = "—";

/// `rows` against the code: the Default cell of every key in `code` is
/// its value, of a key in `no_value.none` a dash, and of one in
/// `no_value.described` its words; every row is one of those. Where the
/// table has a Required column, it says "yes" exactly for the keys
/// `requires` says must be written (`None`: no Required column).
fn assert_defaults(
    what: &str,
    rows: Vec<Row>,
    code: BTreeMap<&str, String>,
    no_value: NoValue,
    requires: Option<&dyn Fn(&str) -> bool>,
) {
    for (key, value) in &code {
        let row = rows
            .iter()
            .find(|r| r.key == *key)
            .unwrap_or_else(|| panic!("{what}: no row for {key}"));
        assert_eq!(stated(&row.default), stated(value), "{what}: {key}");
    }
    for row in &rows {
        let key = row.key.as_str();
        if no_value.none.contains(&key) {
            assert!(
                row.default.starts_with(NO_DEFAULT),
                "{what}: {key} has no default, but the table gives {}",
                row.default
            );
        } else if let Some((_, words)) = no_value.described.iter().find(|(k, _)| *k == key) {
            assert_eq!(row.default, *words, "{what}: {key}");
        } else {
            assert!(
                code.contains_key(key),
                "{what}: {key} is in the table but not compared with the code"
            );
        }
        if let (Some(required), Some(requires)) = (&row.required, requires) {
            assert_eq!(
                required.starts_with("yes"),
                requires(key),
                "{what}: {key} is required: {required}"
            );
        }
    }
}

/// `whole` without `key`: a field of it, or `section.field` of one of its
/// sections.
fn without(whole: &serde_json::Value, key: &str) -> serde_json::Value {
    let mut out = whole.clone();
    let (fields, field) = match key.split_once('.') {
        Some((section, field)) => (out.get_mut(section), field),
        None => (Some(&mut out), key),
    };
    if let Some(fields) = fields.and_then(|f| f.as_object_mut()) {
        fields.remove(field);
    }
    out
}

/// Whether `key` must be written: `minimal` (which is accepted) is not
/// without it.
fn must_write(
    minimal: &serde_json::Value,
    key: &str,
    parses: fn(&serde_json::Value) -> bool,
) -> bool {
    assert!(parses(minimal), "{minimal}");
    !parses(&without(minimal, key))
}

#[test]
fn the_top_level_keys_table_is_the_players_defaults() {
    let player = Player::deserialize_from_str("songs: .").unwrap();
    let metronome = MetronomeDefaults::default();
    let rows = rows(&table(
        PLAYER_CONFIG_MD,
        &["Key", "Type", "Default", "Required"],
    ));
    let code = BTreeMap::from([
        ("active_playlist", player.active_playlist().to_string()),
        ("metronome.enabled", metronome.enabled.to_string()),
        (
            "metronome.volume",
            metronome
                .volume
                .unwrap_or(DEFAULT_MASTER_VOLUME)
                .to_string(),
        ),
        ("max_sample_voices", player.max_sample_voices().to_string()),
        ("samples", "{}".to_string()),
    ]);
    // A player config is YAML, of which JSON is a part.
    let parses = |v: &serde_json::Value| {
        Player::deserialize_from_str(&v.to_string()).is_ok_and(|p| p.validate().is_ok())
    };
    let minimal = json!({"songs": ".", "metronome": {}});
    assert_defaults(
        "top-level keys",
        rows,
        code,
        NoValue {
            none: &[
                "songs",
                "playlist",
                "profiles_dir",
                "profiles",
                "samples_file",
            ],
            described: &[
                ("playlists_dir", "playlists/ next to mtrack.yaml"),
                ("metronome.sounds", "synthesized"),
            ],
        },
        Some(&|key| must_write(&minimal, key, parses)),
    );
    // The volume's stated range is the one the player checks.
    let volume_type = table(PLAYER_CONFIG_MD, &["Key", "Type", "Default", "Required"])
        .pairs("Type")
        .into_iter()
        .find(|(k, _)| k == "metronome.volume")
        .unwrap()
        .1;
    assert_eq!(volume_type, "float 0.0–2.0");
    assert_volume_range(0.0, 2.0);
}

/// `validate_volume` admits exactly `low..=high`.
fn assert_volume_range(low: f64, high: f64) {
    assert!(validate_volume(low).is_ok() && validate_volume(high).is_ok());
    assert!(validate_volume(low - 1e-9).is_err() && validate_volume(high + 1e-9).is_err());
}

#[test]
fn the_profile_keys_table_is_the_profiles_defaults() {
    let audio: Audio = serde_json::from_value(json!({"device": "d"})).unwrap();
    let midi: Midi = serde_json::from_value(json!({"device": "d"})).unwrap();
    let dmx: Dmx = serde_json::from_value(json!({"universes": []})).unwrap();
    // The second table of its header: the first is the top-level keys.
    let profile_table =
        &tables_headed(PLAYER_CONFIG_MD, &["Key", "Type", "Default", "Required"])[1];
    let code = BTreeMap::from([
        ("kind", "hardware_profile".to_string()),
        ("audio.buffer_size", audio.buffer_size().to_string()),
        ("audio.buffer_threads", audio.buffer_threads().to_string()),
        ("audio.sample_rate", audio.sample_rate().to_string()),
        (
            "audio.sample_format",
            audio.sample_format().unwrap().as_str().to_string(),
        ),
        ("audio.bits_per_sample", audio.bits_per_sample().to_string()),
        ("audio.resampler", keyword(audio.resampler())),
        (
            "audio.playback_delay",
            secs(audio.playback_delay().unwrap()),
        ),
        ("audio.track_mappings", "{}".to_string()),
        ("audio.track_gains", "{}".to_string()),
        ("midi.playback_delay", secs(midi.playback_delay().unwrap())),
        ("midi.beat_clock", midi.beat_clock().to_string()),
        ("midi.persist_tempo", midi.persist_tempo().to_string()),
        ("midi.beat_clock_seams", keyword(midi.beat_clock_seams())),
        (
            "dmx.dim_speed_modifier",
            dmx.dimming_speed_modifier().to_string(),
        ),
        ("dmx.playback_delay", secs(dmx.playback_delay().unwrap())),
        ("dmx.ola_http_port", dmx.ola_http_port().to_string()),
        ("dmx.null_client", dmx.null_client().to_string()),
        ("controllers", "[]".to_string()),
    ]);
    assert!(
        audio.stream_buffer_size().is_none(),
        "defaults to buffer_size"
    );
    // Written: read and checked, as a profile is loaded.
    let parses = |v: &serde_json::Value| {
        serde_json::from_value::<Profile>(v.clone()).is_ok_and(|p| p.validate().is_ok())
    };
    let minimal = json!({
        "audio": {"device": "d"},
        "midi": {"device": "d"},
        "dmx": {"universes": []},
    });
    assert_defaults(
        "profile keys",
        rows(profile_table),
        code,
        NoValue {
            none: &[
                "hostname",
                "audio.device",
                "midi.device",
                "dmx.lighting",
                "dmx.universes",
                "trigger",
                "status_events",
                "notifications",
            ],
            described: &[
                ("audio.stream_buffer_size", "buffer_size"),
                // `DEFAULT_OLA_PORT` and `Dmx::ola_port` are left out of
                // test builds, so no test reaches the OLA daemon.
                ("dmx.ola_port", "9010"),
            ],
        },
        Some(&|key| must_write(&minimal, key, parses)),
    );
}

/// Each OSC key and its default address, as `OscController` takes them.
fn osc_defaults() -> BTreeMap<String, String> {
    let osc: OscController = serde_json::from_value(json!({})).unwrap();
    match serde_json::to_value(osc).unwrap() {
        serde_json::Value::Object(fields) => fields
            .into_iter()
            .filter_map(|(k, v)| match v {
                serde_json::Value::String(s) if s.starts_with('/') => Some((k, s)),
                _ => None,
            })
            .collect(),
        other => panic!("{other}"),
    }
}

#[test]
fn the_osc_tables_are_the_default_addresses() {
    let defaults = osc_defaults();
    let mut seen = BTreeMap::new();
    for header in [
        &["Key", "Default address", "Argument", "Action"][..],
        &["Key", "Default address", "Arguments"][..],
    ] {
        for (key, address) in table(OSC_MD, header).pairs("Default address") {
            let want = defaults
                .get(&key)
                .unwrap_or_else(|| panic!("osc.md documents {key}, which is not a key"));
            // The outgoing gain address names the track where the incoming
            // one matches any.
            assert_eq!(address.replace("<name>", "*"), *want, "{key}");
            seen.insert(key, address);
        }
    }
    for key in defaults.keys() {
        assert!(seen.contains_key(key), "osc.md does not document {key}");
    }
}

/// The `key: /address` lines of the first yaml block after `marker`.
fn yaml_addresses(markdown: &str, marker: &str) -> Vec<(String, String)> {
    let from = markdown.find(marker).unwrap_or_else(|| panic!("{marker}"));
    let block = &markdown[from..];
    let block = &block[..block.find("\n```\n").unwrap()];
    block
        .lines()
        .filter_map(|line| {
            let (key, value) = line.trim().split_once(": ")?;
            value
                .starts_with('/')
                .then(|| (key.to_string(), value.to_string()))
        })
        .collect()
}

#[test]
fn the_osc_examples_show_the_default_addresses() {
    let defaults = osc_defaults();
    // osc.md's block is every address at its default.
    let shown = yaml_addresses(OSC_MD, "- kind: osc");
    for (key, address) in &shown {
        assert_eq!(Some(address), defaults.get(key), "osc.md: {key}");
    }
    let keys: Vec<&String> = shown.iter().map(|(k, _)| k).collect();
    assert_eq!(
        keys.len(),
        defaults.len(),
        "osc.md's block lists every address"
    );
    // player-config.md's says the ones it shows are the defaults.
    for (key, address) in yaml_addresses(PLAYER_CONFIG_MD, "- kind: osc") {
        assert_eq!(
            Some(&address),
            defaults.get(&key),
            "player-config.md: {key}"
        );
    }
}

#[test]
fn the_trigger_tables_are_the_inputs_defaults() {
    let input: AudioTriggerInput =
        serde_json::from_value(json!({"channel": 1, "sample": "kick"})).unwrap();
    let rows: Vec<Row> = tables_headed(TRIGGERS_MD, &["Parameter", "Default", "Description"])
        .iter()
        .flat_map(rows)
        .collect();
    let disabled = "(disabled)".to_string();
    let optional = |v: Option<String>| v.unwrap_or_else(|| disabled.clone());
    let code = BTreeMap::from([
        ("threshold", input.threshold().to_string()),
        ("scan_time_ms", input.scan_time_ms().to_string()),
        ("retrigger_time_ms", input.retrigger_time_ms().to_string()),
        ("gain", input.gain().to_string()),
        ("velocity_curve", keyword(input.velocity_curve())),
        ("fixed_velocity", input.fixed_velocity().to_string()),
        (
            "highpass_freq",
            optional(input.highpass_freq().map(|f| f.to_string())),
        ),
        (
            "dynamic_threshold_decay_ms",
            optional(input.dynamic_threshold_decay_ms().map(|f| f.to_string())),
        ),
        (
            "noise_floor_sensitivity",
            optional(input.noise_floor_sensitivity().map(|f| f.to_string())),
        ),
        (
            "noise_floor_decay_ms",
            input.noise_floor_decay_ms().to_string(),
        ),
    ]);
    let trigger: crate::config::trigger::TriggerConfig = serde_json::from_value(json!({})).unwrap();
    let mut code = code;
    code.insert(
        "crosstalk_window_ms",
        optional(trigger.crosstalk_window_ms().map(|f| f.to_string())),
    );
    code.insert(
        "crosstalk_threshold",
        optional(trigger.crosstalk_threshold().map(|f| f.to_string())),
    );
    // The device's stream settings default to what the device says.
    assert!(trigger.sample_format().is_none());
    assert!(trigger.bits_per_sample().is_none() && trigger.buffer_size().is_none());
    assert_defaults(
        "trigger inputs",
        rows,
        code,
        NoValue {
            none: &[],
            described: &[
                ("sample_format", "device native"),
                ("bits_per_sample", "device native"),
                ("buffer_size", "device default"),
            ],
        },
        None,
    );
}

#[test]
fn the_metronome_prose_states_the_defaults() {
    // Its lines wrapped anywhere: one space between words.
    let prose = SONG_CONFIG_MD
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    for (name, freq) in [
        ("accent", DEFAULT_ACCENT_FREQ),
        ("normal", DEFAULT_NORMAL_FREQ),
        ("half", DEFAULT_HALF_FREQ),
        ("sub", DEFAULT_SUB_FREQ),
    ] {
        let said = format!("{name} {freq} Hz");
        assert!(prose.contains(&said), "song-config.md: {said}");
    }
    let defaults: MetronomeConfig = serde_json::from_value(json!({})).unwrap();
    assert!(SONG_CONFIG_MD.contains(&format!(
        "# the output track name (default \"{}\")",
        defaults.track
    )));
    assert!(SONG_CONFIG_MD.contains("(1-12, default 1)"));
    let subdivision = |n: u32| {
        serde_json::from_value::<MetronomeConfig>(json!({ "subdivision": n }))
            .unwrap()
            .validate()
    };
    assert!(subdivision(1).is_ok() && subdivision(12).is_ok());
    assert!(subdivision(0).is_err() && subdivision(13).is_err());
    assert_eq!(keyword(&defaults.subdivision), "1");
    assert!(SONG_CONFIG_MD.contains("master level (0.0-2.0)"));
    assert_volume_range(0.0, 2.0);
}

#[test]
fn the_samples_prose_states_the_defaults() {
    let sample: SampleDefinition = serde_json::from_value(json!({"file": "x.wav"})).unwrap();
    assert!(SAMPLES_MD.contains(&format!(
        "release_behavior is \"fade\" (default: {})",
        sample.fade_time_ms()
    )));
    assert!(SAMPLES_MD.contains(&format!("defaults to {DEFAULT_VELOCITY}")));
    assert!(SAMPLES_MD.contains(&format!(
        "- **`{}`** (default)",
        keyword(sample.release_behavior())
    )));
    assert!(SAMPLES_MD.contains(&format!(
        "- **`{}`** (default)",
        keyword(sample.retrigger())
    )));
    assert_eq!(SAMPLES_MD.matches("(default)").count(), 2);
}
