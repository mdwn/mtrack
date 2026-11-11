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

//! The song library over gRPC: `ListSongs`, `GetSong` and
//! `UpdateSongSettings`, for a phone that debugs a song on the fly.
//!
//! An update changes named settings (looping, tracks, samples) in the song's
//! file and leaves everything else in it alone: the file is read as a YAML
//! tree, the keys a setting lives under are changed where the value differs
//! from what the file has, and the tree is written back through the web UI's
//! song save ([`songs_api::save_song`]), which checks it as it checks a save
//! from the song editor. Comments do not survive, as they do not survive the
//! web UI's editor either.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use prost::Message;
use sha2::{Digest, Sha256};
use tonic::Status;
use yaml_rust2::yaml::Hash;
use yaml_rust2::{Yaml, YamlEmitter, YamlLoader};

use crate::config;
use crate::player::{Player, SongSources};
use crate::proto::player::v1::{
    GetSongResponse, ListSongsResponse, SampleVelocity, SongFile, SongListing, SongLooping,
    SongOverview, SongSample, SongSamples, SongTrack, SongTracks, SongTrigger,
    UpdateSongSettingsRequest,
};
use crate::webui::api::songs_api::{self, SONG_WRITES};
use crate::webui::wire::songs::FileKind;

/// How much of a Bluetooth message a page of the library may fill: the
/// rest of the 64 KiB frame is the envelope's, with room to spare.
pub(super) const PAGE_BUDGET: usize = 48 * 1024;

/// A load failure is cut to this many characters in the list; GetSong has
/// it whole.
const LISTED_FAILURE: usize = 200;

/// The page of the library from `start`: as many songs, loaded and failed,
/// sorted by name, as fit `budget` bytes (one at least).
pub(super) fn list(player: &Player, start: u32, budget: usize) -> ListSongsResponse {
    let songs = player.songs();
    let mut rows: Vec<SongListing> = songs
        .sorted_list()
        .iter()
        .map(|song| SongListing {
            name: song.name().to_string(),
            duration: prost_types::Duration::try_from(song.length()).ok(),
            loops: song.loop_playback(),
            has_lighting: !song.light_shows().is_empty() || !song.dsl_lighting_shows().is_empty(),
            has_midi: song.midi_playback().is_some(),
            has_samples: !song.samples_config().samples().is_empty(),
            failure: None,
        })
        .chain(songs.failures().iter().map(|failure| SongListing {
            name: failure.name().to_string(),
            failure: Some(shortened(failure.error(), LISTED_FAILURE)),
            ..Default::default()
        }))
        .collect();
    rows.sort_by(|a, b| a.name.cmp(&b.name));
    let total = rows.len();
    let mut page = Vec::new();
    let mut size = 0;
    let mut next = (start as usize).min(total);
    while next < total {
        let len = rows[next].encoded_len();
        let cost = 1 + prost::length_delimiter_len(len) + len;
        if !page.is_empty() && size + cost > budget {
            break;
        }
        size += cost;
        page.push(std::mem::take(&mut rows[next]));
        next += 1;
    }
    ListSongsResponse {
        songs: page,
        next: (next < total).then_some(next as u32),
        total: total as u32,
    }
}

/// `text` cut to at most `max` characters, with an ellipsis when it was.
fn shortened(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((at, _)) => format!("{}…", &text[..at]),
        None => text.to_string(),
    }
}

/// Where a song is and what the player made of it.
struct Located {
    /// Its file, when it has one there.
    config_path: Option<PathBuf>,
    /// Its directory.
    dir: PathBuf,
    loaded: Option<Arc<crate::songs::Song>>,
    failure: Option<String>,
}

#[allow(clippy::result_large_err)]
fn locate(player: &Player, name: &str) -> Result<Located, Status> {
    let songs = player.songs();
    let loaded = songs.get(name).ok();
    let failure = songs.failure(name);
    let dir = match (&loaded, failure) {
        (Some(song), _) => song.base_path().to_path_buf(),
        (None, Some(failure)) => failure.base_path().to_path_buf(),
        (None, None) => return Err(Status::not_found(format!("Song '{name}' not found"))),
    };
    Ok(Located {
        config_path: songs.config_path(name).filter(|path| path.is_file()),
        dir,
        failure: failure.map(|f| f.error().to_string()),
        loaded,
    })
}

/// The songs directory, as the player reloads from it.
async fn songs_root(
    player: &Player,
    store: Option<&config::ConfigStore>,
) -> Result<PathBuf, Status> {
    if let Some(sources) = player.song_sources() {
        return Ok(sources.songs);
    }
    match store {
        Some(store) => {
            Ok(SongSources::resolve(&store.read_config().await, store.path(), None).songs)
        }
        None => Err(Status::failed_precondition("No songs directory configured")),
    }
}

/// The song's file as read, with what it says.
struct Read {
    text: String,
    config: Result<config::Song, String>,
    /// The audio files beside it, sorted, with their channel counts.
    files: Vec<SongFile>,
}

/// Reads the song's file and lists its directory, on the blocking pool.
async fn read(located: &Located) -> Result<Read, Status> {
    let config_path = located.config_path.clone();
    let dir = located.dir.clone();
    let named: Vec<PathBuf> = located
        .loaded
        .as_ref()
        .map(|song| song.named_files().map(Path::to_path_buf).collect())
        .unwrap_or_default();
    tokio::task::spawn_blocking(move || {
        let text = match &config_path {
            Some(path) => crate::util::read_to_string(path)
                .map_err(|e| Status::internal(format!("reading the song's file failed: {e}")))?,
            None => String::new(),
        };
        let config = match config_path {
            Some(_) => parse(&text),
            None => Err("the song has no file of its own".to_string()),
        };
        Ok(Read {
            config,
            files: audio_files(&dir, &named),
            text,
        })
    })
    .await
    .map_err(|e| Status::internal(format!("reading the song failed: {e}")))?
}

/// A song file's text as the player reads one.
fn parse(text: &str) -> Result<config::Song, String> {
    ::config::Config::builder()
        .add_source(::config::File::from_str(text, ::config::FileFormat::Yaml))
        .build()
        .and_then(|c| c.try_deserialize::<config::Song>())
        .map_err(|e| e.to_string())
}

/// The audio files directly in `dir`, as the web UI's file list finds them,
/// sorted, each with its channel count (0 when it does not read).
fn audio_files(dir: &Path, named: &[PathBuf]) -> Vec<SongFile> {
    let mut files: Vec<SongFile> = songs_api::song_dir_files(dir, named)
        .unwrap_or_default()
        .into_iter()
        .filter(|file| file.kind == FileKind::Audio)
        .map(|file| SongFile {
            channels: crate::audio::sample_source::create_sample_source_from_file(
                dir.join(&file.name),
                None,
                1024,
            )
            .map(|source| u32::from(source.channel_count()))
            .unwrap_or(0),
            name: file.name,
        })
        .collect();
    files.sort_by(|a, b| a.name.cmp(&b.name));
    files
}

/// A fingerprint of a song file: the first 64 bits of a SHA-256 over its
/// text, in hex.
fn revision(text: &str) -> String {
    Sha256::digest(text.as_bytes())[..8]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Whether `name` is the song playing.
async fn is_playing(player: &Player, name: &str) -> bool {
    player.is_playing().await
        && player
            .get_playlist()
            .current()
            .is_some_and(|song| song.name() == name)
}

/// The song `name` as `GetSong` reports it.
pub(super) async fn get(
    player: &Player,
    store: Option<&config::ConfigStore>,
    name: &str,
) -> Result<GetSongResponse, Status> {
    let located = locate(player, name)?;
    // Under the songs directory, as every song write checks.
    songs_api::resolve_song_dir(player, &songs_root(player, store).await?, name)
        .map_err(|_| Status::not_found(format!("Song '{name}' not found")))?;
    let read = read(&located).await?;
    let mut response = report(name, &located, &read);
    response.playing = is_playing(player, name).await;
    Ok(response)
}

fn report(name: &str, located: &Located, read: &Read) -> GetSongResponse {
    let mut response = GetSongResponse {
        name: name.to_string(),
        duration: located
            .loaded
            .as_ref()
            .and_then(|song| prost_types::Duration::try_from(song.length()).ok()),
        failure: located.failure.clone(),
        files: read.files.clone(),
        revision: revision(&read.text),
        ..Default::default()
    };
    match &read.config {
        Ok(config) => {
            response.looping = Some(SongLooping {
                loop_playback: config.loop_playback(),
                crossfade: config.loop_crossfade_text().unwrap_or_default().to_string(),
                cut: config.loop_crossfade_cut().to_vec(),
            });
            response.tracks = Some(SongTracks {
                tracks: config
                    .tracks()
                    .iter()
                    .map(|t| SongTrack {
                        name: t.name().to_string(),
                        file: t.file().to_string(),
                        file_channel: t.file_channel().map(u32::from),
                    })
                    .collect(),
            });
            let mut samples: Vec<SongSample> = config
                .samples()
                .iter()
                .map(|(name, def)| sample_message(name, def))
                .collect();
            samples.sort_by(|a, b| a.name.cmp(&b.name));
            response.samples = Some(SongSamples { samples });
            response.triggers = config
                .sample_triggers()
                .iter()
                .map(|t| SongTrigger {
                    event: describe_event(t.trigger()),
                    sample: t.sample().to_string(),
                })
                .collect();
            response.overview = Some(overview(config));
        }
        Err(e) => {
            response.read_only = Some(if located.config_path.is_none() {
                "The song has no file of its own to change.".to_string()
            } else {
                format!("The song's file does not read, so it is not changed from here: {e}")
            });
            // What the player loaded, when it did.
            if let Some(song) = &located.loaded {
                response.tracks = Some(SongTracks {
                    tracks: song
                        .tracks()
                        .iter()
                        .map(|t| SongTrack {
                            name: t.name().to_string(),
                            file: crate::util::filename_display(t.file()).to_string(),
                            file_channel: Some(u32::from(t.file_channel())),
                        })
                        .collect(),
                });
            }
        }
    }
    response
}

fn sample_message(name: &str, def: &config::SampleDefinition) -> SongSample {
    let fields = SampleFields::of(Some(def));
    let velocity = def.velocity();
    SongSample {
        name: name.to_string(),
        original_name: name.to_string(),
        file: fields.file,
        output_channels: fields.output_channels,
        output_track: fields.output_track,
        release_behavior: fields.release,
        retrigger: fields.retrigger,
        max_voices: fields.max_voices,
        fade_time_ms: Some(fields.fade_time_ms),
        velocity: Some(SampleVelocity {
            mode: word(&velocity.mode()),
            default_velocity: velocity.default_velocity().map(u32::from),
            layers: velocity.layers().len() as u32,
            scale: velocity.scales().unwrap_or(false),
        }),
    }
}

/// An enum's name as a config file writes it (`play_to_completion`).
fn word<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

/// A MIDI event in the web UI's words: "Note On, channel 10, key 36".
fn describe_event<T: serde::Serialize>(event: &T) -> String {
    let Ok(serde_json::Value::Object(fields)) = serde_json::to_value(event) else {
        return String::new();
    };
    let kind = match fields.get("type").and_then(|t| t.as_str()) {
        Some("note_off") => "Note Off",
        Some("note_on") => "Note On",
        Some("aftertouch") => "Aftertouch",
        Some("control_change") => "Control Change",
        Some("program_change") => "Program Change",
        Some("channel_aftertouch") => "Channel Aftertouch",
        Some("pitch_bend") => "Pitch Bend",
        Some(other) => other,
        None => "",
    };
    let mut words = vec![kind.to_string()];
    for field in [
        "channel",
        "key",
        "velocity",
        "controller",
        "value",
        "program",
        "bend",
    ] {
        let Some(value) = fields.get(field) else {
            continue;
        };
        // A trigger's velocity of 0 is no velocity to match.
        if field == "velocity" && value.as_u64() == Some(0) {
            continue;
        }
        words.push(format!("{field} {value}"));
    }
    words.join(", ")
}

fn overview(config: &config::Song) -> SongOverview {
    SongOverview {
        tempo: config.tempo().map(|tempo| {
            let mut words = format!("{} bpm, {}", tempo.bpm, tempo.time_signature);
            match tempo.changes.len() {
                0 => {}
                1 => words.push_str(", 1 change"),
                n => words.push_str(&format!(", {n} changes")),
            }
            words
        }),
        sections: config.sections().iter().map(|s| s.name.clone()).collect(),
        metronome: config.metronome().is_some(),
        pilot_hints: config.pilot().map_or(0, |p| p.hints.len() as u32),
        midi_file: config.midi_playback().map(|m| m.file().to_string()),
        lighting: config
            .lighting()
            .unwrap_or_default()
            .iter()
            .map(|show| show.file().to_string())
            .collect(),
        midi_light_shows: config.light_shows().map_or(0, |s| s.len() as u32),
        notification_audio: config.notification_audio().is_some(),
        song_select_event: config.has_midi_event(),
    }
}

/// A sample's settings that an update sets, as read or as asked for.
#[derive(Debug, PartialEq)]
struct SampleFields {
    file: Option<String>,
    output_channels: Vec<u32>,
    output_track: Option<String>,
    release: String,
    retrigger: String,
    max_voices: Option<u32>,
    fade_time_ms: u32,
}

const DEFAULT_RELEASE: &str = "play_to_completion";
const DEFAULT_RETRIGGER: &str = "cut";
const DEFAULT_FADE_MS: u32 = 50;

impl SampleFields {
    /// A sample's as the player reads them; a new one's with none.
    fn of(def: Option<&config::SampleDefinition>) -> SampleFields {
        match def {
            Some(def) => SampleFields {
                file: def.file().map(str::to_string),
                output_channels: def
                    .output_channels()
                    .iter()
                    .map(|&c| u32::from(c))
                    .collect(),
                output_track: def.output_track().map(str::to_string),
                release: word(&def.release_behavior()),
                retrigger: word(&def.retrigger()),
                max_voices: def.max_voices(),
                fade_time_ms: def.fade_time_ms(),
            },
            None => SampleFields {
                file: None,
                output_channels: Vec::new(),
                output_track: None,
                release: DEFAULT_RELEASE.to_string(),
                retrigger: DEFAULT_RETRIGGER.to_string(),
                max_voices: None,
                fade_time_ms: DEFAULT_FADE_MS,
            },
        }
    }

    /// Those an update asks for, an empty choice the default.
    fn asked(sample: &SongSample) -> SampleFields {
        let or = |value: &str, default: &str| match value.trim() {
            "" => default.to_string(),
            value => value.to_string(),
        };
        let text = |value: &Option<String>| {
            value
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(str::to_string)
        };
        SampleFields {
            file: text(&sample.file),
            output_channels: sample.output_channels.clone(),
            output_track: text(&sample.output_track),
            release: or(&sample.release_behavior, DEFAULT_RELEASE),
            retrigger: or(&sample.retrigger, DEFAULT_RETRIGGER),
            max_voices: sample.max_voices,
            fade_time_ms: sample.fade_time_ms.unwrap_or(DEFAULT_FADE_MS),
        }
    }
}

/// The files an update may point a track or a sample at: those the song
/// names already, as written, and the audio files in its directory.
struct AllowedFiles {
    named: BTreeSet<String>,
    beside: BTreeSet<String>,
}

impl AllowedFiles {
    fn new(config: &config::Song, files: &[SongFile]) -> AllowedFiles {
        let mut named: BTreeSet<String> = config
            .tracks()
            .iter()
            .map(|t| t.file().to_string())
            .collect();
        for def in config.samples().values() {
            named.extend(def.file().map(str::to_string));
            named.extend(def.velocity().layers().iter().map(|l| l.file().to_string()));
        }
        AllowedFiles {
            named,
            beside: files.iter().map(|f| f.name.clone()).collect(),
        }
    }

    #[allow(clippy::result_large_err)]
    fn check(&self, file: &str) -> Result<(), Status> {
        if self.named.contains(file) || self.beside.contains(file) {
            return Ok(());
        }
        Err(Status::invalid_argument(format!(
            "{file:?} is not an audio file in the song's folder"
        )))
    }
}

fn key(name: &str) -> Yaml {
    Yaml::String(name.to_string())
}

/// Sets `name` in `map` to `value`, in its place when it is there; removes
/// it for `None`.
fn put(map: &mut Hash, name: &str, value: Option<Yaml>) {
    match value {
        Some(value) => match map.get_mut(&key(name)) {
            Some(slot) => *slot = value,
            None => {
                map.insert(key(name), value);
            }
        },
        None => {
            map.remove(&key(name));
        }
    }
}

fn strings(list: &[String]) -> Yaml {
    Yaml::Array(list.iter().map(|s| Yaml::String(s.clone())).collect())
}

/// The song file `text` (read as `config`) with the settings `req` sets.
/// What it does not set, and what it sets to what the file has, stays as
/// the file has it. The text unchanged when nothing changed. Not checked
/// beyond the files named: the save checks the result as the web UI's does.
#[allow(clippy::result_large_err)]
fn apply(
    text: &str,
    config: &config::Song,
    req: &UpdateSongSettingsRequest,
    files: &[SongFile],
) -> Result<String, Status> {
    let unreadable = || {
        Status::failed_precondition(
            "The song's file is not a YAML mapping, so it is not changed from here; \
             fix it in the web UI first",
        )
    };
    let mut docs = YamlLoader::load_from_str(text).map_err(|_| unreadable())?;
    let mut map = match docs.drain(..).next() {
        Some(Yaml::Hash(map)) => map,
        _ => return Err(unreadable()),
    };
    let before = map.clone();
    let allowed = AllowedFiles::new(config, files);

    if let Some(looping) = &req.looping {
        if looping.loop_playback != config.loop_playback() {
            put(
                &mut map,
                "loop_playback",
                looping.loop_playback.then_some(Yaml::Boolean(true)),
            );
        }
        let crossfade = looping.crossfade.trim();
        if crossfade != config.loop_crossfade_text().unwrap_or_default() {
            put(
                &mut map,
                "loop_crossfade",
                (!crossfade.is_empty()).then(|| Yaml::String(crossfade.to_string())),
            );
        }
        if looping.cut != config.loop_crossfade_cut() {
            put(
                &mut map,
                "loop_crossfade_cut",
                (!looping.cut.is_empty()).then(|| strings(&looping.cut)),
            );
        }
    }

    if let Some(tracks) = &req.tracks {
        let old = config.tracks();
        let same = |i: usize, t: &SongTrack| {
            old.get(i).is_some_and(|o| {
                o.name() == t.name
                    && o.file() == t.file
                    && o.file_channel().map(u32::from) == t.file_channel
            })
        };
        let unchanged = tracks.tracks.len() == old.len()
            && tracks.tracks.iter().enumerate().all(|(i, t)| same(i, t));
        if !unchanged {
            let old_nodes = before
                .get(&key("tracks"))
                .and_then(Yaml::as_vec)
                .cloned()
                .unwrap_or_default();
            let mut list = Vec::new();
            for (i, track) in tracks.tracks.iter().enumerate() {
                if let (true, Some(node)) = (same(i, track), old_nodes.get(i)) {
                    list.push(node.clone());
                    continue;
                }
                allowed.check(&track.file)?;
                let mut node = Hash::new();
                node.insert(key("name"), Yaml::String(track.name.clone()));
                node.insert(key("file"), Yaml::String(track.file.clone()));
                if let Some(channel) = track.file_channel {
                    node.insert(key("file_channel"), Yaml::Integer(i64::from(channel)));
                }
                list.push(Yaml::Hash(node));
            }
            put(&mut map, "tracks", Some(Yaml::Array(list)));
        }
    }

    if let Some(samples) = &req.samples {
        let wanted = &samples.samples;
        let mut names = BTreeSet::new();
        let mut originals = BTreeSet::new();
        for sample in wanted {
            let name = sample.name.trim();
            if name.is_empty() {
                return Err(Status::invalid_argument("a sample needs a name"));
            }
            if name != sample.name {
                return Err(Status::invalid_argument(format!(
                    "sample name {:?} starts or ends with a space",
                    sample.name
                )));
            }
            if !names.insert(name) {
                return Err(Status::invalid_argument(format!(
                    "two samples are named {name:?}"
                )));
            }
            if sample.original_name.is_empty() {
                continue;
            }
            if !config.samples().contains_key(&sample.original_name) {
                return Err(Status::invalid_argument(format!(
                    "sample {:?} is not in the song",
                    sample.original_name
                )));
            }
            if !originals.insert(sample.original_name.as_str()) {
                return Err(Status::invalid_argument(format!(
                    "sample {:?} is listed twice",
                    sample.original_name
                )));
            }
        }
        let old_nodes = before
            .get(&key("samples"))
            .and_then(Yaml::as_hash)
            .cloned()
            .unwrap_or_default();
        let mut next = Hash::new();
        // Those kept, in the file's order, then the new ones.
        for (name, node) in &old_nodes {
            let Some(name) = name.as_str() else { continue };
            if let Some(sample) = wanted.iter().find(|s| s.original_name == name) {
                let edited =
                    edit_sample(node.clone(), config.samples().get(name), sample, &allowed)?;
                next.insert(key(&sample.name), edited);
            }
        }
        for sample in wanted.iter().filter(|s| s.original_name.is_empty()) {
            let made = edit_sample(Yaml::Hash(Hash::new()), None, sample, &allowed)?;
            next.insert(key(&sample.name), made);
        }
        if next != old_nodes {
            put(
                &mut map,
                "samples",
                (!next.is_empty()).then_some(Yaml::Hash(next)),
            );
        }
    }

    if map == before {
        return Ok(text.to_string());
    }
    let mut out = String::new();
    YamlEmitter::new(&mut out)
        .dump(&Yaml::Hash(map))
        .map_err(|e| Status::internal(format!("writing the song failed: {e}")))?;
    let mut out = out.strip_prefix("---\n").unwrap_or(&out).to_string();
    out.push('\n');
    Ok(out)
}

/// A sample's node with what `sample` sets that differs from `was` (its
/// definition as read; `None` for a new one). Its velocity, and any key an
/// update does not name, are left as they are.
#[allow(clippy::result_large_err)]
fn edit_sample(
    node: Yaml,
    was: Option<&config::SampleDefinition>,
    sample: &SongSample,
    allowed: &AllowedFiles,
) -> Result<Yaml, Status> {
    let Yaml::Hash(mut map) = node else {
        return Err(Status::failed_precondition(format!(
            "sample {:?} is not a mapping in the song's file, so it is not changed from here",
            sample.original_name
        )));
    };
    let was = SampleFields::of(was);
    let want = SampleFields::asked(sample);
    if want.file != was.file {
        if let Some(file) = &want.file {
            allowed.check(file)?;
        }
        put(&mut map, "file", want.file.clone().map(Yaml::String));
    }
    if want.output_channels != was.output_channels {
        put(
            &mut map,
            "output_channels",
            (!want.output_channels.is_empty()).then(|| {
                Yaml::Array(
                    want.output_channels
                        .iter()
                        .map(|&c| Yaml::Integer(i64::from(c)))
                        .collect(),
                )
            }),
        );
    }
    if want.output_track != was.output_track {
        put(
            &mut map,
            "output_track",
            want.output_track.clone().map(Yaml::String),
        );
    }
    if want.release != was.release {
        // The old spelling goes with the old value.
        map.remove(&key("note_off"));
        put(
            &mut map,
            "release_behavior",
            (want.release != DEFAULT_RELEASE).then(|| Yaml::String(want.release.clone())),
        );
    }
    if want.retrigger != was.retrigger {
        put(
            &mut map,
            "retrigger",
            (want.retrigger != DEFAULT_RETRIGGER).then(|| Yaml::String(want.retrigger.clone())),
        );
    }
    if want.max_voices != was.max_voices {
        put(
            &mut map,
            "max_voices",
            want.max_voices.map(|v| Yaml::Integer(i64::from(v))),
        );
    }
    if want.fade_time_ms != was.fade_time_ms {
        put(
            &mut map,
            "fade_time_ms",
            (want.fade_time_ms != DEFAULT_FADE_MS)
                .then(|| Yaml::Integer(i64::from(want.fade_time_ms))),
        );
    }
    Ok(Yaml::Hash(map))
}

/// The refusal of an update made from a file that is no longer the one
/// read. The app matches the start of the message.
fn changed() -> Status {
    Status::failed_precondition("the song has changed since it was read")
}

/// Loads the song as `text` would have it, as the player loads it, so a
/// song that plays is not saved into one that does not; and checks the
/// playlists that play it still would: a fade that leaves out a track the
/// song would no longer have stops its playlist loading.
#[allow(clippy::result_large_err)]
fn check_loads(player: &Player, name: &str, dir: &Path, text: &str) -> Result<(), Status> {
    let config = parse(text).map_err(Status::invalid_argument)?;
    config
        .validate()
        .map_err(|errors| Status::invalid_argument(errors.join("; ")))?;
    let song = crate::songs::Song::new(dir, &config)
        .map_err(|e| Status::invalid_argument(e.to_string()))?;
    for (playlist_name, playlist) in player.playlists_snapshot() {
        if playlist_name == crate::playlist::ALL_SONGS {
            continue;
        }
        for (entry, fades) in playlist.songs().iter().zip(playlist.entry_fades()) {
            if entry != name {
                continue;
            }
            for (field, fade) in [("fade_out", &fades.fade_out), ("fade_in", &fades.fade_in)] {
                let Some(config::FadeConfig::Full {
                    exclude: Some(tracks),
                    ..
                }) = fade
                else {
                    continue;
                };
                if let Some(track) = tracks.iter().find(|t| !song.has_track(t)) {
                    return Err(Status::invalid_argument(format!(
                        "playlist {playlist_name}: {field}.exclude names track {track:?}, \
                         which the song would no longer have, and the playlist would not load"
                    )));
                }
            }
        }
    }
    Ok(())
}

/// Changes the song's settings `req` names, as the web UI's song save
/// writes a song, with the lock its saves take held from reading the file
/// to reloading the player; then answers with the song as `GetSong` now
/// reports it.
pub(super) async fn update(
    player: &Arc<Player>,
    store: Option<&config::ConfigStore>,
    req: UpdateSongSettingsRequest,
) -> Result<GetSongResponse, Status> {
    let name = req.name.clone();
    let root = songs_root(player, store).await?;
    {
        let _writing = SONG_WRITES.lock().await;
        let located = locate(player, &name)?;
        let read = read(&located).await?;
        if !req.expected_revision.is_empty() && revision(&read.text) != req.expected_revision {
            return Err(changed());
        }
        let config = match &read.config {
            Ok(config) => config,
            Err(_) if located.config_path.is_none() => {
                return Err(Status::failed_precondition(
                    "The song has no file of its own to change",
                ))
            }
            // What is wrong with it stays out of the message, as a
            // playlist's does: GetSong says it.
            Err(_) => {
                return Err(Status::failed_precondition(
                    "The song's file does not read, so changing it would lose what is in it; \
                     fix it in the web UI first",
                ))
            }
        };
        let updated = apply(&read.text, config, &req, &read.files)?;
        if updated != read.text {
            // A song that fails to load already may be being fixed: its
            // reload says whether it is.
            if located.loaded.is_some() {
                let (player, name, dir, text) = (
                    player.clone(),
                    name.clone(),
                    located.dir.clone(),
                    updated.clone(),
                );
                tokio::task::spawn_blocking(move || check_loads(&player, &name, &dir, &text))
                    .await
                    .map_err(|e| Status::internal(format!("checking the song failed: {e}")))??;
            }
            if let Err(response) = songs_api::save_song(player, &root, &name, updated).await {
                return Err(super::playlists::status_of(response).await);
            }
            player.reload_songs().await.map_err(|e| {
                Status::internal(format!(
                    "the change was written, but the player could not reload its songs: {e}"
                ))
            })?;
        }
    }
    get(player, store, &name).await
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A song with every feature a song file has.
    const FULL: &str = "kind: song
name: Full
midi_event:
  type: program_change
  channel: 1
  program: 4
midi_playback:
  file: song.mid
  exclude_midi_channels: [10]
light_shows:
- universe_name: light-show
  dmx_file: dmx.mid
  midi_channels: [15]
lighting:
- file: show.light
tracks:
- name: click
  file: click.wav
- name: keys
  file: keys.wav
  file_channel: 2
samples:
  kick:
    file: kick.wav
    output_channels: [1, 2]
    note_off: stop
    velocity:
      mode: layers
      scale: true
      layers:
      - range: [1, 63]
        file: kick_soft.wav
      - range: [64, 127]
        file: kick_hard.wav
  snare:
    file: snare.wav
    output_track: drums
    retrigger: polyphonic
    max_voices: 4
    fade_time_ms: 80
    velocity:
      mode: ignore
      default: 90
sample_triggers:
- trigger:
    type: note_on
    channel: 10
    key: 36
  sample: kick
loop_playback: true
loop_crossfade: 2measures
loop_crossfade_cut:
- click
sections:
- name: verse
  start_measure: 1
  end_measure: 8
- name: chorus
  start_measure: 9
  end_measure: 16
  color: '#5ecaea'
tempo:
  bpm: 120.0
  time_signature: 4/4
  changes:
  - measure: 9
    bpm: 140.0
metronome:
  track: metronome
pilot:
  hints:
  - label: count in
    at:
      measure: 1
notification_audio:
  song_start: start.wav
";

    fn files(names: &[&str]) -> Vec<SongFile> {
        names
            .iter()
            .map(|n| SongFile {
                name: n.to_string(),
                channels: 2,
            })
            .collect()
    }

    fn full() -> config::Song {
        parse(FULL).unwrap()
    }

    fn tree(text: &str) -> Hash {
        match YamlLoader::load_from_str(text).unwrap().remove(0) {
            Yaml::Hash(map) => map,
            other => panic!("not a mapping: {other:?}"),
        }
    }

    /// `a` and `b` with the keys in `except` taken out of each: the rest
    /// must be the same, key by key and in order.
    fn assert_same_but(a: &str, b: &str, except: &[&str]) {
        let strip = |text: &str| {
            let mut map = tree(text);
            for name in except {
                map.remove(&key(name));
            }
            map
        };
        let (a, b) = (strip(a), strip(b));
        assert_eq!(
            a.keys().collect::<Vec<_>>(),
            b.keys().collect::<Vec<_>>(),
            "keys and their order"
        );
        assert_eq!(a, b);
    }

    #[test]
    fn a_song_is_reported_as_its_file_has_it() {
        let located = Located {
            config_path: Some(PathBuf::from("song.yaml")),
            dir: PathBuf::from("."),
            loaded: None,
            failure: None,
        };
        let read = Read {
            text: FULL.to_string(),
            config: parse(FULL),
            files: files(&["click.wav"]),
        };
        let got = report("Full", &located, &read);
        let looping = got.looping.unwrap();
        assert!(looping.loop_playback);
        assert_eq!(looping.crossfade, "2measures");
        assert_eq!(looping.cut, ["click"]);
        let tracks = got.tracks.unwrap().tracks;
        assert_eq!(tracks[1].file_channel, Some(2));
        assert_eq!(tracks[0].file_channel, None);
        let samples = got.samples.unwrap().samples;
        assert_eq!(samples[0].name, "kick");
        assert_eq!(samples[0].original_name, "kick");
        assert_eq!(samples[0].release_behavior, "stop");
        assert_eq!(samples[0].output_channels, [1, 2]);
        let velocity = samples[0].velocity.as_ref().unwrap();
        assert_eq!((velocity.mode.as_str(), velocity.layers), ("layers", 2));
        assert!(velocity.scale);
        assert_eq!(samples[1].retrigger, "polyphonic");
        assert_eq!(samples[1].max_voices, Some(4));
        assert_eq!(samples[1].fade_time_ms, Some(80));
        assert_eq!(
            samples[1].velocity.as_ref().unwrap().default_velocity,
            Some(90)
        );
        assert_eq!(got.triggers[0].event, "Note On, channel 10, key 36");
        assert_eq!(got.triggers[0].sample, "kick");
        let overview = got.overview.unwrap();
        assert_eq!(overview.tempo.as_deref(), Some("120 bpm, 4/4, 1 change"));
        assert_eq!(overview.sections, ["verse", "chorus"]);
        assert!(overview.metronome && overview.notification_audio && overview.song_select_event);
        assert_eq!(overview.pilot_hints, 1);
        assert_eq!(overview.midi_file.as_deref(), Some("song.mid"));
        assert_eq!(overview.lighting, ["show.light"]);
        assert_eq!(overview.midi_light_shows, 1);
        assert_eq!(got.revision, revision(FULL));
        assert_eq!(got.read_only, None);
    }

    #[test]
    fn settings_read_and_sent_back_change_nothing() {
        let located = Located {
            config_path: Some(PathBuf::from("song.yaml")),
            dir: PathBuf::from("."),
            loaded: None,
            failure: None,
        };
        let read = Read {
            text: FULL.to_string(),
            config: parse(FULL),
            files: vec![],
        };
        let got = report("Full", &located, &read);
        let req = UpdateSongSettingsRequest {
            name: "Full".into(),
            looping: got.looping,
            tracks: got.tracks,
            samples: got.samples,
            ..Default::default()
        };
        assert_eq!(apply(FULL, &full(), &req, &[]).unwrap(), FULL);
    }

    #[test]
    fn a_looping_change_keeps_everything_else() {
        let req = UpdateSongSettingsRequest {
            looping: Some(SongLooping {
                loop_playback: true,
                crossfade: "6s".into(),
                cut: vec![],
            }),
            ..Default::default()
        };
        let out = apply(FULL, &full(), &req, &[]).unwrap();
        let song = parse(&out).unwrap();
        assert_eq!(song.loop_crossfade_text(), Some("6s"));
        assert!(song.loop_crossfade_cut().is_empty());
        assert!(song.loop_playback());
        assert_same_but(FULL, &out, &["loop_crossfade", "loop_crossfade_cut"]);
        // In its place, not moved to the end.
        let keys: Vec<_> = tree(&out)
            .keys()
            .map(|k| k.as_str().unwrap().to_string())
            .collect();
        let at = keys.iter().position(|k| k == "loop_crossfade").unwrap();
        assert_eq!(keys[at + 1], "sections");

        // Looping off takes the key out, as the web UI's editor does.
        let off = UpdateSongSettingsRequest {
            looping: Some(SongLooping {
                loop_playback: false,
                crossfade: String::new(),
                cut: vec![],
            }),
            ..Default::default()
        };
        let out = apply(FULL, &full(), &off, &[]).unwrap();
        let map = tree(&out);
        for gone in ["loop_playback", "loop_crossfade", "loop_crossfade_cut"] {
            assert!(!map.contains_key(&key(gone)), "{gone}");
        }
        assert_same_but(
            FULL,
            &out,
            &["loop_playback", "loop_crossfade", "loop_crossfade_cut"],
        );
    }

    #[test]
    fn tracks_are_renamed_repointed_added_and_removed() {
        let req = UpdateSongSettingsRequest {
            tracks: Some(SongTracks {
                tracks: vec![
                    SongTrack {
                        name: "keys left".into(),
                        file: "keys.wav".into(),
                        file_channel: Some(1),
                    },
                    SongTrack {
                        name: "bass".into(),
                        file: "bass.wav".into(),
                        file_channel: None,
                    },
                ],
            }),
            ..Default::default()
        };
        let out = apply(FULL, &full(), &req, &files(&["bass.wav", "keys.wav"])).unwrap();
        let song = parse(&out).unwrap();
        let tracks: Vec<_> = song
            .tracks()
            .iter()
            .map(|t| (t.name(), t.file(), t.file_channel()))
            .collect();
        assert_eq!(
            tracks,
            [
                ("keys left", "keys.wav", Some(1)),
                ("bass", "bass.wav", None)
            ]
        );
        assert_same_but(FULL, &out, &["tracks"]);
    }

    #[test]
    fn a_file_outside_the_songs_folder_is_refused() {
        for file in [
            "../other/song.wav",
            "/etc/passwd",
            "missing.wav",
            "stems/x.wav",
        ] {
            let req = UpdateSongSettingsRequest {
                tracks: Some(SongTracks {
                    tracks: vec![SongTrack {
                        name: "click".into(),
                        file: file.into(),
                        file_channel: None,
                    }],
                }),
                ..Default::default()
            };
            let err = apply(FULL, &full(), &req, &files(&["bass.wav"])).unwrap_err();
            assert_eq!(err.code(), tonic::Code::InvalidArgument, "{file}");
            assert!(
                err.message().contains("not an audio file"),
                "{}",
                err.message()
            );
            let sample = UpdateSongSettingsRequest {
                samples: Some(SongSamples {
                    samples: vec![SongSample {
                        name: "new".into(),
                        file: Some(file.into()),
                        ..Default::default()
                    }],
                }),
                ..Default::default()
            };
            assert!(apply(FULL, &full(), &sample, &files(&["bass.wav"])).is_err());
        }
        // A file the song names already may stay named, wherever it is.
        let flat = "name: Flat\ntracks:\n- name: t\n  file: ../shared.wav\n";
        let req = UpdateSongSettingsRequest {
            tracks: Some(SongTracks {
                tracks: vec![SongTrack {
                    name: "renamed".into(),
                    file: "../shared.wav".into(),
                    file_channel: None,
                }],
            }),
            ..Default::default()
        };
        let out = apply(flat, &parse(flat).unwrap(), &req, &[]).unwrap();
        assert!(
            out.contains("file: \"../shared.wav\"") || out.contains("file: ../shared.wav"),
            "{out}"
        );
    }

    #[test]
    fn samples_are_edited_keeping_their_velocity() {
        let mut got: Vec<SongSample> = full()
            .samples()
            .iter()
            .map(|(name, def)| sample_message(name, def))
            .collect();
        got.sort_by(|a, b| a.name.cmp(&b.name));
        // Rename kick, change its release and file; drop snare; add one.
        got[0].name = "bass drum".into();
        got[0].release_behavior = "fade".into();
        got[0].file = Some("kick2.wav".into());
        got.remove(1);
        got.push(SongSample {
            name: "clap".into(),
            file: Some("clap.wav".into()),
            retrigger: "polyphonic".into(),
            ..Default::default()
        });
        let req = UpdateSongSettingsRequest {
            samples: Some(SongSamples { samples: got }),
            ..Default::default()
        };
        let out = apply(FULL, &full(), &req, &files(&["clap.wav", "kick2.wav"])).unwrap();
        let samples = tree(&out)[&key("samples")].as_hash().unwrap().clone();
        let names: Vec<_> = samples.keys().map(|k| k.as_str().unwrap()).collect();
        assert_eq!(names, ["bass drum", "clap"]);
        let kick = samples[&key("bass drum")].as_hash().unwrap();
        // The old spelling of the release went with it, and the velocity
        // layers came through untouched.
        assert!(!kick.contains_key(&key("note_off")));
        assert_eq!(kick[&key("release_behavior")].as_str(), Some("fade"));
        assert_eq!(kick[&key("file")].as_str(), Some("kick2.wav"));
        let original = tree(FULL)[&key("samples")]["kick"]["velocity"].clone();
        assert_eq!(kick[&key("velocity")], original);
        // A new sample has what was set and no defaults written out.
        let clap = samples[&key("clap")].as_hash().unwrap();
        let clap_keys: Vec<_> = clap.keys().map(|k| k.as_str().unwrap()).collect();
        assert_eq!(clap_keys, ["file", "retrigger"]);
        // The file loads, and the triggers were not touched.
        let song = parse(&out).unwrap();
        assert_eq!(song.sample_triggers()[0].sample(), "kick");
        assert_same_but(FULL, &out, &["samples"]);
    }

    #[test]
    fn bad_sample_lists_are_refused() {
        let cases = [
            (
                vec![SongSample {
                    name: String::new(),
                    ..Default::default()
                }],
                "needs a name",
            ),
            (
                vec![
                    SongSample {
                        name: "a".into(),
                        ..Default::default()
                    },
                    SongSample {
                        name: "a".into(),
                        ..Default::default()
                    },
                ],
                "two samples",
            ),
            (
                vec![SongSample {
                    name: "a".into(),
                    original_name: "nope".into(),
                    ..Default::default()
                }],
                "not in the song",
            ),
        ];
        for (samples, says) in cases {
            let req = UpdateSongSettingsRequest {
                samples: Some(SongSamples { samples }),
                ..Default::default()
            };
            let err = apply(FULL, &full(), &req, &[]).unwrap_err();
            assert_eq!(err.code(), tonic::Code::InvalidArgument);
            assert!(err.message().contains(says), "{says}: {}", err.message());
        }
    }

    #[test]
    fn the_revision_follows_the_text() {
        assert_eq!(revision(FULL).len(), 16);
        assert_eq!(revision(FULL), revision(FULL));
        assert_ne!(revision(FULL), revision(&FULL.replace("120.0", "121.0")));
    }

    #[test]
    fn a_long_failure_is_cut_in_the_list() {
        assert_eq!(shortened("abc", 5), "abc");
        assert_eq!(shortened("abcdef", 3), "abc…");
        assert_eq!(shortened("ééé", 2), "éé…");
    }

    mod served {
        use tonic::{Code, Request};

        use super::super::super::PlayerServer;
        use super::FULL;
        use crate::proto::player::v1::{
            player_service_server::PlayerService, GetSongRequest, GetSongResponse,
            ListSongsRequest, SongLooping, SongTrack, SongTracks, UpdateSongSettingsRequest,
        };
        use crate::testutil::player::{playlist_project, PlaylistProject};

        fn server(project: &PlaylistProject) -> PlayerServer {
            PlayerServer::with_config_store(project.player.clone(), project.store.clone())
        }

        fn song_dir(project: &PlaylistProject, name: &str) -> std::path::PathBuf {
            project.dir.path().join("assets/songs").join(name)
        }

        /// The playlists' project with the song "Full" (every feature a
        /// song file has, all loading) in a folder of its own, and "Broken",
        /// whose track is missing.
        async fn project() -> PlaylistProject {
            let project = playlist_project().await.unwrap();
            let dir = song_dir(&project, "Full");
            std::fs::create_dir_all(&dir).unwrap();
            let mono = || vec![vec![0_i32; 8000 * 20]];
            for file in [
                "click.wav",
                "kick.wav",
                "kick_soft.wav",
                "kick_hard.wav",
                "snare.wav",
                "start.wav",
                "bass.wav",
            ] {
                crate::testutil::write_wav(dir.join(file), mono(), 8000).unwrap();
            }
            crate::testutil::write_wav(
                dir.join("keys.wav"),
                vec![vec![0_i32; 8000 * 20], vec![0_i32; 8000 * 20]],
                8000,
            )
            .unwrap();
            std::fs::copy("assets/song.mid", dir.join("song.mid")).unwrap();
            std::fs::copy("assets/song.mid", dir.join("dmx.mid")).unwrap();
            std::fs::write(
                dir.join("show.light"),
                "show \"Full\" {\n    @00:00.000\n    front_wash: static color: \"blue\", duration: 2s\n}\n",
            )
            .unwrap();
            std::fs::write(dir.join("song.yaml"), FULL).unwrap();
            let broken = song_dir(&project, "Broken");
            std::fs::create_dir_all(&broken).unwrap();
            std::fs::write(
                broken.join("song.yaml"),
                "name: Broken\ntracks:\n- name: t\n  file: gone.wav\n",
            )
            .unwrap();
            project.player.reload_songs().await.unwrap();
            project
        }

        async fn get(
            project: &PlaylistProject,
            name: &str,
        ) -> Result<GetSongResponse, tonic::Status> {
            server(project)
                .get_song(Request::new(GetSongRequest { name: name.into() }))
                .await
                .map(|r| r.into_inner())
        }

        async fn update(
            project: &PlaylistProject,
            req: UpdateSongSettingsRequest,
        ) -> Result<GetSongResponse, tonic::Status> {
            server(project)
                .update_song_settings(Request::new(req))
                .await
                .map(|r| r.into_inner())
        }

        fn written(project: &PlaylistProject) -> String {
            std::fs::read_to_string(song_dir(project, "Full").join("song.yaml")).unwrap()
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn the_library_lists_loaded_and_failed_songs_in_pages() {
            let project = project().await;
            let all = server(&project)
                .list_songs(Request::new(ListSongsRequest { start: 0 }))
                .await
                .unwrap()
                .into_inner();
            assert_eq!(all.next, None);
            assert_eq!(all.total as usize, all.songs.len());
            let names: Vec<_> = all.songs.iter().map(|s| s.name.as_str()).collect();
            let mut sorted = names.clone();
            sorted.sort();
            assert_eq!(names, sorted);
            let full = all.songs.iter().find(|s| s.name == "Full").unwrap();
            assert!(
                full.loops && full.has_lighting && full.has_midi && full.has_samples,
                "{full:?}"
            );
            assert!(full.duration.is_some() && full.failure.is_none());
            let broken = all.songs.iter().find(|s| s.name == "Broken").unwrap();
            assert!(broken.failure.as_deref().unwrap().contains("gone.wav"));
            assert!(broken.duration.is_none());

            // A small budget pages it; the pages are the list.
            let mut paged = Vec::new();
            let mut start = 0;
            loop {
                let page = super::super::list(&project.player, start, 64);
                assert_eq!(page.total, all.total);
                assert!(!page.songs.is_empty());
                paged.extend(page.songs);
                match page.next {
                    Some(next) => start = next,
                    None => break,
                }
            }
            assert_eq!(paged, all.songs);
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn a_song_reports_its_settings_and_the_files_beside_it() {
            let project = project().await;
            let got = get(&project, "Full").await.unwrap();
            assert!(got.looping.as_ref().unwrap().loop_playback);
            assert_eq!(got.tracks.as_ref().unwrap().tracks.len(), 2);
            let files: Vec<_> = got
                .files
                .iter()
                .map(|f| (f.name.as_str(), f.channels))
                .collect();
            assert!(files.contains(&("keys.wav", 2)), "{files:?}");
            assert!(files.contains(&("click.wav", 1)), "{files:?}");
            // Audio only: not the song's file, its MIDI or its show.
            assert!(
                files.iter().all(|(name, _)| name.ends_with(".wav")),
                "{files:?}"
            );
            assert!(!got.playing);
            assert_eq!(got.read_only, None);
            assert!(got.duration.is_some());

            // A song that failed says why, and is still there to fix.
            let broken = get(&project, "Broken").await.unwrap();
            assert!(broken.failure.as_deref().unwrap().contains("gone.wav"));
            assert_eq!(broken.tracks.unwrap().tracks[0].file, "gone.wav");
            assert_eq!(broken.read_only, None);

            let err = get(&project, "Nope").await.unwrap_err();
            assert_eq!(err.code(), Code::NotFound);
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn an_update_is_written_reloaded_and_answered() {
            let project = project().await;
            let read = get(&project, "Full").await.unwrap();
            let req = UpdateSongSettingsRequest {
                name: "Full".into(),
                expected_revision: read.revision.clone(),
                looping: Some(SongLooping {
                    loop_playback: true,
                    crossfade: "6s".into(),
                    cut: vec!["click".into()],
                }),
                ..Default::default()
            };
            let saved = update(&project, req).await.unwrap();
            assert_eq!(saved.looping.as_ref().unwrap().crossfade, "6s");
            assert_ne!(saved.revision, read.revision);
            // As GetSong reports it, and the player has it.
            assert_eq!(get(&project, "Full").await.unwrap(), saved);
            let loaded = project.player.songs().get("Full").unwrap();
            assert_eq!(
                loaded.loop_crossfade(),
                Some(std::time::Duration::from_secs(6))
            );
            // Everything else in the file is as it was.
            super::assert_same_but(FULL, &written(&project), &["loop_crossfade"]);
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn a_stale_revision_is_refused() {
            let project = project().await;
            let read = get(&project, "Full").await.unwrap();
            let edited = FULL.replace("bpm: 120.0", "bpm: 121.0");
            std::fs::write(song_dir(&project, "Full").join("song.yaml"), &edited).unwrap();
            let req = UpdateSongSettingsRequest {
                name: "Full".into(),
                expected_revision: read.revision,
                looping: Some(SongLooping::default()),
                ..Default::default()
            };
            let err = update(&project, req).await.unwrap_err();
            assert_eq!(err.code(), Code::FailedPrecondition);
            assert!(err.message().starts_with("the song has changed"));
            assert_eq!(written(&project), edited);
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn a_change_the_song_would_not_load_with_is_refused_saying_why() {
            let project = project().await;
            let tracks = |channel: Option<u32>, file: &str| {
                Some(SongTracks {
                    tracks: vec![
                        SongTrack {
                            name: "click".into(),
                            file: "click.wav".into(),
                            file_channel: None,
                        },
                        SongTrack {
                            name: "keys".into(),
                            file: file.into(),
                            file_channel: channel,
                        },
                    ],
                })
            };
            let looping = |crossfade: &str, cut: &[&str]| {
                Some(SongLooping {
                    loop_playback: true,
                    crossfade: crossfade.into(),
                    cut: cut.iter().map(|s| s.to_string()).collect(),
                })
            };
            let cases = [
                (
                    UpdateSongSettingsRequest {
                        tracks: tracks(Some(3), "keys.wav"),
                        ..Default::default()
                    },
                    "exceeds the file's channel count",
                ),
                (
                    UpdateSongSettingsRequest {
                        tracks: tracks(None, "keys.wav"),
                        ..Default::default()
                    },
                    "more than one channel",
                ),
                (
                    UpdateSongSettingsRequest {
                        tracks: tracks(Some(0), "keys.wav"),
                        ..Default::default()
                    },
                    "file_channel must be 1 or greater",
                ),
                (
                    UpdateSongSettingsRequest {
                        tracks: tracks(Some(1), "../song1.yaml"),
                        ..Default::default()
                    },
                    "not an audio file",
                ),
                (
                    UpdateSongSettingsRequest {
                        looping: looping("lots", &["click"]),
                        ..Default::default()
                    },
                    "lots",
                ),
                (
                    UpdateSongSettingsRequest {
                        looping: looping("2s", &["nope"]),
                        ..Default::default()
                    },
                    "nope",
                ),
            ];
            for (req, says) in cases {
                let req = UpdateSongSettingsRequest {
                    name: "Full".into(),
                    ..req
                };
                let err = update(&project, req).await.unwrap_err();
                assert_eq!(err.code(), Code::InvalidArgument, "{says}");
                assert!(err.message().contains(says), "{says}: {}", err.message());
                assert_eq!(written(&project), FULL, "{says}");
            }
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn a_rename_that_would_drop_a_playlist_is_refused() {
            let project = project().await;
            project
                .write(
                    "set",
                    "songs:\n- song: Full\n  fade_out: {length: 2s, exclude: [click]}\n- Song 1\n",
                )
                .await;
            assert!(project.player.playlists_snapshot().contains_key("set"));
            let req = UpdateSongSettingsRequest {
                name: "Full".into(),
                tracks: Some(SongTracks {
                    tracks: vec![
                        SongTrack {
                            name: "clicks".into(),
                            file: "click.wav".into(),
                            file_channel: None,
                        },
                        SongTrack {
                            name: "keys".into(),
                            file: "keys.wav".into(),
                            file_channel: Some(2),
                        },
                    ],
                }),
                looping: Some(SongLooping {
                    loop_playback: true,
                    crossfade: "2measures".into(),
                    cut: vec!["clicks".into()],
                }),
                ..Default::default()
            };
            let err = update(&project, req).await.unwrap_err();
            assert_eq!(err.code(), Code::InvalidArgument);
            assert!(err.message().contains("playlist set"), "{}", err.message());
            assert_eq!(written(&project), FULL);
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn a_failed_song_can_be_fixed() {
            let project = project().await;
            std::fs::copy(
                song_dir(&project, "Full").join("click.wav"),
                song_dir(&project, "Broken").join("there.wav"),
            )
            .unwrap();
            let read = get(&project, "Broken").await.unwrap();
            assert!(read.files.iter().any(|f| f.name == "there.wav"));
            let req = UpdateSongSettingsRequest {
                name: "Broken".into(),
                expected_revision: read.revision,
                tracks: Some(SongTracks {
                    tracks: vec![SongTrack {
                        name: "t".into(),
                        file: "there.wav".into(),
                        file_channel: None,
                    }],
                }),
                ..Default::default()
            };
            let saved = update(&project, req).await.unwrap();
            assert_eq!(saved.failure, None);
            assert!(project.player.songs().get("Broken").is_ok());
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn nothing_changed_writes_nothing() {
            let project = project().await;
            let read = get(&project, "Full").await.unwrap();
            let path = song_dir(&project, "Full").join("song.yaml");
            let before = std::fs::metadata(&path).unwrap().modified().unwrap();
            let saved = update(
                &project,
                UpdateSongSettingsRequest {
                    name: "Full".into(),
                    expected_revision: read.revision.clone(),
                    looping: read.looping.clone(),
                    tracks: read.tracks.clone(),
                    samples: read.samples.clone(),
                },
            )
            .await
            .unwrap();
            assert_eq!(saved.revision, read.revision);
            assert_eq!(
                std::fs::metadata(&path).unwrap().modified().unwrap(),
                before
            );
        }

        #[test]
        fn two_hundred_long_named_songs_fit_one_page() {
            use prost::Message;
            let row = |i: usize| crate::proto::player::v1::SongListing {
                name: format!("Song {i:03} {}", "with a long name ".repeat(4)),
                duration: Some(prost_types::Duration {
                    seconds: 300,
                    nanos: 500_000_000,
                }),
                loops: true,
                has_lighting: true,
                has_midi: true,
                has_samples: true,
                failure: i
                    .is_multiple_of(20)
                    .then(|| "x".repeat(super::LISTED_FAILURE)),
            };
            let response = crate::proto::player::v1::ListSongsResponse {
                songs: (0..200).map(row).collect(),
                next: None,
                total: 200,
            };
            let size = response.encoded_len();
            println!("ListSongsResponse: {size} bytes for 200 songs");
            assert!(size < super::PAGE_BUDGET, "{size}");
            const { assert!(super::PAGE_BUDGET < crate::controller::ble::codec::MAX_FRAME) };
        }
    }
}
