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
use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use std::{cmp, fmt};

use crate::audio::sample_source::create_sample_source_from_file;
use crate::audio::SampleFormat;
use midly::live::LiveEvent;
use midly::Smf;

use tracing::{debug, info, warn};

use crate::audio::TargetFormat;
use crate::config;
use crate::lighting::parser::LightShow as ParsedLightShow;
use crate::proto::player;
use crate::util::filename_display;

/// Returns true if the extension is a supported audio (non-MIDI) format.
pub fn is_supported_audio_extension(ext: &str) -> bool {
    matches!(
        ext,
        "wav" | "flac" | "mp3" | "ogg" | "aac" | "m4a" | "mp4" | "aiff" | "aif"
    )
}

/// A resolved DSL lighting show with absolute file path and cached parsed shows.
#[derive(Debug, Clone)]
pub struct DslLightingShow {
    /// The absolute path to the DSL file
    file_path: PathBuf,
    /// Cached parsed light shows from the DSL file
    shows: HashMap<String, ParsedLightShow>,
}

impl DslLightingShow {
    /// Creates a new DSL lighting show, validating that the file exists and can be parsed.
    pub fn new(start_path: &Path, config: &config::LightingShow) -> Result<Self, Box<dyn Error>> {
        let file_path = if config.file().starts_with('/') {
            PathBuf::from(config.file())
        } else {
            start_path.join(config.file())
        };

        // Validate file exists
        if !file_path.exists() {
            return Err(format!(
                "DSL lighting show file does not exist: {}",
                file_path.display()
            )
            .into());
        }

        // Validate file can be read and parsed
        let content = std::fs::read_to_string(&file_path).map_err(|e| {
            format!(
                "Failed to read DSL lighting show {}: {}",
                file_path.display(),
                e
            )
        })?;

        let shows = crate::lighting::parser::parse_light_shows(&content).map_err(|e| {
            // Prepend the file path to the error, preserving newlines in the original error
            format!(
                "Failed to parse DSL lighting show {}:\n{}",
                file_path.display(),
                e
            )
        })?;

        Ok(DslLightingShow { file_path, shows })
    }

    /// Gets the absolute file path
    pub fn file_path(&self) -> &Path {
        &self.file_path
    }

    /// Gets the cached parsed light shows
    pub fn shows(&self) -> &HashMap<String, ParsedLightShow> {
        &self.shows
    }
}

/// A song with associated tracks for multitrack playback. Can contain:
/// - An optional MIDI event, which will be played when the song is selected in a playlist.
/// - An optional MIDI file, which will be played along with the audio tracks.
pub struct Song {
    /// The name of the song.
    name: String,
    /// The base path of the song (directory containing the song config).
    base_path: PathBuf,
    /// The MIDI event to play when the song is selected in a playlist.
    midi_event: Option<LiveEvent<'static>>,
    /// The MIDI playback configuration.
    midi_playback: Option<MidiPlayback>,
    /// The light show configurations
    light_shows: Vec<LightShow>,
    /// The DSL lighting shows (resolved to absolute paths)
    dsl_lighting_shows: Vec<DslLightingShow>,
    /// The number of channels required to play this song.
    num_channels: u16,
    /// The sample rate of this song.
    sample_rate: u32,
    /// The sample format.
    sample_format: SampleFormat,
    /// The total duration of the song.
    duration: Duration,
    /// The individual audio tracks.
    tracks: Vec<Track>,
    /// Per-song samples configuration.
    samples_config: config::SamplesConfig,
}

/// A simple sample for songs. Boils down to i32 or f32, which we can be reasonably assured that
/// symphonia is able to read.
impl Song {
    // Create a new song.
    pub fn new(start_path: &Path, config: &config::Song) -> Result<Song, Box<dyn Error>> {
        let midi_playback = match config.midi_playback() {
            Some(midi_playback) => Some(MidiPlayback::new(start_path, midi_playback)?),
            None => None,
        };
        let light_shows = match config.light_shows() {
            Some(light_shows) => light_shows
                .iter()
                .map(|light_show| LightShow::new(start_path, light_show))
                .collect::<Result<Vec<LightShow>, Box<dyn Error>>>()?,
            None => Vec::default(),
        };
        // Resolve DSL lighting shows to absolute paths
        let dsl_lighting_shows = match config.lighting() {
            Some(lighting_shows) => lighting_shows
                .iter()
                .map(|lighting_show| DslLightingShow::new(start_path, lighting_show))
                .collect::<Result<Vec<DslLightingShow>, Box<dyn Error>>>()?,
            None => Vec::new(),
        };

        // Calculate the number of channels and sample rate by reading the wav headers of each file.
        let tracks = config
            .tracks()
            .iter()
            .map(|track| Track::new(start_path, track))
            .collect::<Result<Vec<Track>, Box<dyn Error>>>()?;
        let num_channels = u16::try_from(tracks.len())?;
        let mut sample_rate = 0;
        let mut max_duration = Duration::ZERO;

        let mut sample_format: Option<SampleFormat> = None;
        for track in tracks.iter() {
            // Set the sample rate and formatif it's not already set.
            if sample_rate == 0 {
                sample_rate = track.sample_rate;
            } else if sample_rate != track.sample_rate {
                // AudioTranscoder handles different sample rates
            }
            max_duration = cmp::max(track.duration, max_duration);

            match sample_format {
                Some(sample_format) => {
                    if sample_format != track.sample_format {
                        // AudioTranscoder handles different sample formats
                    }
                }
                None => sample_format = Some(track.sample_format),
            }
        }

        if sample_format.is_none() {
            warn!("no sample format found");
        }

        Ok(Song {
            name: config.name().to_string(),
            base_path: start_path.to_path_buf(),
            midi_event: config.midi_event()?,
            midi_playback,
            light_shows,
            dsl_lighting_shows,
            num_channels,
            sample_rate,
            sample_format: sample_format.unwrap_or(SampleFormat::Int),
            duration: max_duration,
            tracks,
            samples_config: config.samples_config(),
        })
    }

    /// Create a song from a directory without a configuration file
    pub fn initialize(song_directory: &PathBuf) -> Result<Self, Box<dyn Error>> {
        let song_files = fs::read_dir(song_directory)?;
        let name = filename_display(song_directory).to_string();

        let mut light_shows = vec![];
        let mut dsl_lighting_shows = vec![];
        let mut midi_playback = None;
        let mut tracks = vec![];
        for song_file in song_files {
            let entry = song_file?;
            let file_type = entry.file_type()?;
            if file_type.is_dir() {
                warn!("Song directory {song_directory:?} has a subdirectory called '{entry:?}'. It will be ignored for initialization.");
                continue;
            }
            if !file_type.is_file() {
                warn!("Song directory {song_directory:?} has an entry '{entry:?}' that is not a regular file. It will be ignored during initialization.");
                continue;
            }

            let path = entry.path();
            let stem = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or("Unreadable file stem");
            let extension = path
                .extension()
                .and_then(|extension| extension.to_str())
                .unwrap_or("Unreadable file extension");

            match extension {
                "mid" => {
                    if stem.starts_with("dmx_") {
                        light_shows.push(LightShow {
                            universe_name: "default universe".to_string(),
                            dmx_file: path,
                            midi_channels: vec![],
                        });
                    } else {
                        midi_playback = Some(MidiPlayback {
                            file: path,
                            exclude_midi_channels: vec![],
                        })
                    }
                }
                "light" => {
                    let content = std::fs::read_to_string(&path).map_err(|e| {
                        format!("Failed to read DSL lighting show {}: {}", path.display(), e)
                    })?;
                    let shows =
                        crate::lighting::parser::parse_light_shows(&content).map_err(|e| {
                            format!(
                                "Failed to parse DSL lighting show {}:\n{}",
                                path.display(),
                                e
                            )
                        })?;
                    dsl_lighting_shows.push(DslLightingShow {
                        file_path: path,
                        shows,
                    });
                }
                ext if is_supported_audio_extension(ext) => {
                    let mut new_tracks = Track::load_tracks(&path)?;
                    tracks.append(&mut new_tracks);
                }
                unknown_extension => {
                    info!("Unknown extension: {unknown_extension}. Ignoring file.");
                }
            }
        }
        // Deduplicate track names by appending numeric suffixes on collision.
        deduplicate_track_names(&mut tracks);

        let song = Self {
            name,
            base_path: song_directory.clone(),
            midi_playback,
            light_shows,
            dsl_lighting_shows,
            tracks,
            samples_config: config::SamplesConfig::default(),
            ..Default::default()
        };
        Ok(song)
    }

    pub fn get_config(&self) -> config::Song {
        let name = self.name();
        let midi_event = None;
        let midi_file = self
            .midi_playback
            .as_ref()
            .map(|midi_playback| filename_display(&midi_playback.file).to_string());
        let midi_playback = None;
        let light_shows = match &self.light_shows().len() {
            0 => None,
            _ => {
                let light_shows = self.light_shows();
                Some(
                    light_shows
                        .iter()
                        .map(|light_show| light_show.get_config())
                        .collect(),
                )
            }
        };
        let tracks = self
            .tracks()
            .iter()
            .map(|track| track.get_config())
            .collect();
        config::Song::new(
            name,
            midi_event,
            midi_file,
            midi_playback,
            light_shows,
            if self.dsl_lighting_shows.is_empty() {
                None
            } else {
                Some(
                    self.dsl_lighting_shows
                        .iter()
                        .map(|show| {
                            config::LightingShow::new(
                                filename_display(show.file_path()).to_string(),
                            )
                        })
                        .collect(),
                )
            },
            tracks,
            std::collections::HashMap::new(), // No sample overrides when creating from Song
            Vec::new(),                       // No sample trigger overrides
        )
    }

    /// Gets the name of the song.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Gets the base path of the song (directory containing the song config).
    pub fn base_path(&self) -> &Path {
        &self.base_path
    }

    /// Gets the per-song samples configuration.
    pub fn samples_config(&self) -> &config::SamplesConfig {
        &self.samples_config
    }

    /// Gets the MIDI event.
    pub fn midi_event(&self) -> Option<LiveEvent<'static>> {
        self.midi_event
    }

    /// Gets the sample format.
    pub fn sample_format(&self) -> SampleFormat {
        self.sample_format
    }

    /// Gets the duration of the song.
    pub fn duration(&self) -> Duration {
        self.duration
    }

    /// Gets the number of channels.
    pub fn num_channels(&self) -> u16 {
        self.num_channels
    }

    /// Gets the MIDI playback info.
    pub fn midi_playback(&self) -> Option<&MidiPlayback> {
        self.midi_playback.as_ref()
    }

    /// Gets the song light shows.
    pub fn light_shows(&self) -> &[LightShow] {
        &self.light_shows
    }

    /// Gets the DSL lighting shows.
    pub fn dsl_lighting_shows(&self) -> &[DslLightingShow] {
        &self.dsl_lighting_shows
    }

    /// Gets the song tracks.
    pub fn tracks(&self) -> &[Track] {
        &self.tracks
    }

    /// Checks if this song requires transcoding for the given target format
    pub fn needs_transcoding(&self, target_format: &TargetFormat) -> bool {
        // Check if any track has different sample rate, format, or bit depth
        self.tracks.iter().any(|track| {
            // Use the generic SampleSource infrastructure to check transcoding needs
            match crate::audio::sample_source::create_sample_source_from_file(
                &track.file,
                None,
                1024,
            ) {
                Ok(sample_source) => {
                    // Create source format from the SampleSource metadata
                    let source_format = TargetFormat::new(
                        sample_source.sample_rate(),
                        track.sample_format,
                        sample_source.bits_per_sample(),
                    );

                    if let Ok(source_format) = source_format {
                        // Check if any format parameter differs
                        source_format.sample_rate != target_format.sample_rate
                            || source_format.sample_format != target_format.sample_format
                            || source_format.bits_per_sample != target_format.bits_per_sample
                    } else {
                        true // If we can't create source format, assume transcoding is needed
                    }
                }
                Err(_) => {
                    // Fallback: assume transcoding is needed if we can't read the file
                    true
                }
            }
        })
    }

    /// Returns the duration string in minutes and seconds.
    pub fn duration_string(&self) -> String {
        let secs = self.duration.as_secs();
        format!("{}:{:02}", secs / 60, secs % 60)
    }

    /// Creates ChannelMappedSampleSource instances for each track in the song, starting from a specific time.
    /// Uses the given playback context for target format, buffer size, and optional buffered-source pool.
    pub fn create_channel_mapped_sources_from(
        &self,
        context: &crate::audio::PlaybackContext,
        start_time: Duration,
        track_mappings: &HashMap<String, Vec<u16>>,
    ) -> Result<Vec<Box<dyn crate::audio::sample_source::ChannelMappedSampleSource>>, Box<dyn Error>>
    {
        use crate::audio::sample_source::create_channel_mapped_sample_source;
        use crate::audio::sample_source::create_sample_source_from_file;
        use crate::audio::sample_source::BufferedSampleSource;

        let mut sources = Vec::new();

        // Group tracks by file (like the old SongSource did)
        let mut files_to_tracks = HashMap::<PathBuf, Vec<&Track>>::new();
        for track in &self.tracks {
            files_to_tracks
                .entry(track.file.clone())
                .or_default()
                .push(track);
        }

        // Sort files by path to ensure deterministic processing order
        // This ensures all tracks are processed in the same order every time,
        // which is critical for synchronization when seeking
        let mut sorted_files: Vec<_> = files_to_tracks.into_iter().collect();
        sorted_files.sort_by_key(|(path, _)| path.clone());

        for (file_path, tracks) in sorted_files {
            // Create the sample source once and reuse it for both metadata and playback
            // This avoids creating two instances which can cause issues with symphonia's global state
            let sample_source = create_sample_source_from_file(
                &file_path,
                if start_time == Duration::ZERO {
                    None
                } else {
                    Some(start_time)
                },
                context.buffer_size,
            )?;

            // Get the channel count from the source we just created
            let wav_channels = sample_source.channel_count();

            // Create channel mappings for each channel in the WAV file
            let mut channel_mappings = Vec::new();
            for channel in 0..wav_channels {
                let mut labels = Vec::new();

                // Find tracks that use this channel
                for track in &tracks {
                    if track.file_channel == (channel + 1) {
                        // Check if this track name is in the track mappings
                        if track_mappings.contains_key(&track.name) {
                            labels.push(track.name.clone());
                        }
                    }
                }

                channel_mappings.push(labels);
            }

            let source = create_channel_mapped_sample_source(
                sample_source,
                context.target_format.clone(),
                channel_mappings,
                context.resampler_type,
            )?;
            let source: Box<dyn crate::audio::sample_source::ChannelMappedSampleSource> =
                if let Some(pool) = &context.buffer_fill_pool {
                    Box::new(BufferedSampleSource::new(
                        source,
                        pool.clone(),
                        context.buffer_size,
                    ))
                } else {
                    source
                };

            sources.push(source);
        }
        Ok(sources)
    }

    /// Returns a proto version of the song.
    pub fn to_proto(&self) -> Result<player::v1::Song, std::io::Error> {
        let duration = match prost_types::Duration::try_from(self.duration) {
            Ok(duration) => duration,
            Err(e) => return Err(std::io::Error::other(e.to_string())),
        };
        Ok(player::v1::Song {
            name: self.name.to_string(),
            duration: Some(duration),
            tracks: self.tracks.iter().map(|track| track.name.clone()).collect(),
        })
    }
}

impl fmt::Display for Song {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Name: {}\n  Duration: {}\n  Channels: {}\n  Sample Rate: {}\n  Midi Message: {:?}\n  Midi File:{:?}\n  Tracks: {}",
            self.name,
            self.duration_string(),
            self.num_channels,
            self.sample_rate,
            self.midi_event,
            self.midi_playback.as_ref().map(|midi_playback|&midi_playback.file),
            self.tracks
                .iter()
                .map(|track| track.name.clone())
                .collect::<Vec<String>>()
                .join(", "),
        )
    }
}

impl Default for Song {
    fn default() -> Self {
        Self {
            name: Default::default(),
            base_path: PathBuf::new(),
            midi_event: Default::default(),
            midi_playback: Default::default(),
            light_shows: Vec::new(),
            dsl_lighting_shows: Vec::new(),
            num_channels: Default::default(),
            sample_rate: Default::default(),
            sample_format: SampleFormat::Int,
            duration: Default::default(),
            tracks: Default::default(),
            samples_config: config::SamplesConfig::default(),
        }
    }
}

impl Song {
    /// Creates a Song with the given name and track names for use in tests.
    /// Avoids the need for real audio files.
    #[cfg(test)]
    pub fn new_for_test(name: &str, track_names: &[&str]) -> Song {
        Song {
            name: name.to_string(),
            tracks: track_names
                .iter()
                .map(|n| Track {
                    name: n.to_string(),
                    file: PathBuf::from("/dev/null"),
                    file_channel: 1,
                    sample_rate: 44100,
                    sample_format: SampleFormat::Int,
                    duration: Duration::ZERO,
                })
                .collect(),
            ..Default::default()
        }
    }
}

/// Midi playback configuration for the song.
#[derive(Clone)]
pub struct MidiPlayback {
    /// The path to the MIDI file.
    file: PathBuf,

    /// The MIDI channels to exclude from playback.
    exclude_midi_channels: Vec<u8>,
}

impl MidiPlayback {
    /// Creates a new MIDI playback object.
    pub fn new(
        start_path: &Path,
        config: config::MidiPlayback,
    ) -> Result<MidiPlayback, Box<dyn Error>> {
        let file = start_path.join(config.file());

        if !file.exists() {
            return Err(format!("file {} does not exist", file.display()).into());
        }
        Ok(MidiPlayback {
            file,
            exclude_midi_channels: config.exclude_midi_channels(),
        })
    }

    /// Returns a MIDI sheet for the song.
    pub fn midi_sheet(&self) -> Result<MidiSheet, Box<dyn Error>> {
        parse_midi(&self.file)
    }

    /// Gets the MIDI channels to exclude.
    pub fn exclude_midi_channels(&self) -> Vec<u8> {
        self.exclude_midi_channels.clone()
    }
}

/// Returns a MIDI sheet for the given file.
fn parse_midi(midi_file: &PathBuf) -> Result<MidiSheet, Box<dyn Error>> {
    let buf: Vec<u8> = fs::read(midi_file)
        .map_err(|e| format!("Failed to read MIDI file {}: {}", midi_file.display(), e))?;
    let smf = Smf::parse(&buf)
        .map_err(|e| format!("Failed to parse MIDI file {}: {}", midi_file.display(), e))?;
    let ticks_per_beat = match smf.header.timing {
        midly::Timing::Metrical(tpb) => tpb.as_int(),
        _ => return Err("timecode-based MIDI timing not supported".into()),
    };
    let (tempo_map, tpb, total_ticks) = crate::midi::playback::PrecomputedMidi::build_tempo_info(
        &smf.tracks,
        ticks_per_beat,
        smf.header.format,
    );
    let precomputed = crate::midi::playback::PrecomputedMidi::from_tracks(
        &smf.tracks,
        ticks_per_beat,
        smf.header.format,
    );
    // Only generate beat clock when the MIDI file contains explicit tempo events.
    // Files without tempo maps have no tempo opinion, so mtrack stays out of the way
    // and lets musicians control their own tempo.
    let beat_clock = if tempo_map.is_empty() {
        None
    } else {
        Some(
            crate::midi::beat_clock::PrecomputedBeatClock::from_tempo_info(
                &tempo_map,
                tpb,
                total_ticks,
            ),
        )
    };
    Ok(MidiSheet {
        precomputed,
        beat_clock,
    })
}

/// A light show for the song.
#[derive(Clone)]
pub struct LightShow {
    /// The name of the universe. Will be matched against the universes configured in the DMX engine
    /// to determine where (if anywhere) this light show should be sent.
    universe_name: String,

    /// The associated MIDI file to interpret as DMX to play.
    dmx_file: PathBuf,

    /// The MIDI channels from this MIDI file to use as lighting data. If none are supplied, all channels
    /// will be used.
    midi_channels: Vec<u8>,
}

impl LightShow {
    pub fn new(start_path: &Path, config: &config::LightShow) -> Result<LightShow, Box<dyn Error>> {
        let dmx_file = start_path.join(config.dmx_file());

        if !dmx_file.exists() {
            return Err(format!("file {} does not exist", dmx_file.display()).into());
        }
        Ok(LightShow {
            universe_name: config.universe_name(),
            dmx_file,
            midi_channels: config.midi_channels(),
        })
    }

    /// Gets the absolute file path of the DMX MIDI file.
    pub fn dmx_file_path(&self) -> &Path {
        &self.dmx_file
    }

    /// Gets the universe name associated with the DMX playback.
    pub fn universe_name(&self) -> String {
        self.universe_name.clone()
    }

    /// Returns a MIDI sheet for the DMX file.
    pub fn dmx_midi_sheet(&self) -> Result<MidiSheet, Box<dyn Error>> {
        parse_midi(&self.dmx_file)
    }

    /// Gets the MIDI channels to include.
    pub fn midi_channels(&self) -> Vec<u8> {
        self.midi_channels.clone()
    }

    pub fn get_config(&self) -> config::LightShow {
        config::LightShow::new(
            self.universe_name(),
            filename_display(&self.dmx_file).to_string(),
            Some(self.midi_channels()),
        )
    }
}

/// Track is an individual audio track to play.
#[derive(Clone)]
pub struct Track {
    /// The name of the audio track.
    name: String,
    /// The file that contains the contents of this audio track.
    file: PathBuf,
    /// The channel to use in the file for this audio track.
    file_channel: u16,
    /// The sample rate of the track.
    sample_rate: u32,
    /// The sample format of the track.
    sample_format: SampleFormat,
    /// The duration of the track.
    duration: Duration,
}

impl Track {
    /// Creates a new track.
    pub fn new(start_path: &Path, config: &config::Track) -> Result<Track, Box<dyn Error>> {
        let track_file = start_path.join(config.file());
        let file_channel = config.file_channel();
        let name = config.name();

        let source = create_sample_source_from_file(&track_file, None, 1024).map_err(
            |e| -> Box<dyn Error> {
                format!("track \"{}\" (file {}): {}", name, track_file.display(), e).into()
            },
        )?;

        // Extract all metadata before the source might be dropped or cause issues
        let sample_rate = source.sample_rate();
        let duration = source.duration().unwrap_or(Duration::ZERO);
        let channel_count = source.channel_count();

        if channel_count > 1 && file_channel.is_none() {
            return Err(format!(
                "track {} has more than one channel but file_channel is not specified",
                name,
            )
            .into());
        }
        let file_channel = file_channel.unwrap_or(1);
        if file_channel == 0 {
            return Err(format!(
                "track {}: file_channel must be 1 or greater (channels are 1-indexed)",
                name,
            )
            .into());
        }
        if file_channel > channel_count {
            return Err(format!(
                "track {}: file_channel {} exceeds the file's channel count ({})",
                name, file_channel, channel_count,
            )
            .into());
        }

        let sample_format = source.sample_format();
        Ok(Track {
            name: name.to_string(),
            file: track_file.clone(),
            file_channel,
            sample_rate,
            sample_format,
            duration,
        })
    }

    pub fn load_tracks(track_path: &PathBuf) -> Result<Vec<Track>, Box<dyn Error>> {
        let stem = track_path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("Unreadable file stem");
        let extension = track_path
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or("Unreadable file extension");

        if !is_supported_audio_extension(extension) {
            return Err(format!(
                "Unsupported audio format '.{}' for file '{}'",
                extension,
                track_path.display()
            )
            .into());
        }
        let track_name = crate::util::to_kebab_case(stem);
        let source = create_sample_source_from_file(track_path, None, 1024).map_err(
            |e| -> Box<dyn Error> { format!("file {}: {}", track_path.display(), e).into() },
        )?;
        let sample_rate = source.sample_rate();
        let sample_format = source.sample_format();
        let duration = source.duration().unwrap_or(Duration::ZERO);
        let tracks = match source.channel_count() {
            0 => vec![],
            1 => vec![Track {
                name: track_name,
                file: track_path.to_path_buf(),
                file_channel: 1,
                sample_rate,
                sample_format,
                duration,
            }],
            2 => vec![
                Track {
                    name: format!("{track_name}-l"),
                    file: track_path.to_path_buf(),
                    file_channel: 1,
                    sample_rate,
                    sample_format,
                    duration,
                },
                Track {
                    name: format!("{track_name}-r"),
                    file: track_path.to_path_buf(),
                    file_channel: 2,
                    sample_rate,
                    sample_format,
                    duration,
                },
            ],
            _ => (0..source.channel_count())
                .map(|channel| Track {
                    name: format!("{track_name}-{}", channel + 1),
                    file: track_path.to_path_buf(),
                    file_channel: channel + 1,
                    sample_rate,
                    sample_format,
                    duration,
                })
                .collect(),
        };
        Ok(tracks)
    }

    /// Gets the track name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Sets the track name.
    fn set_name(&mut self, name: String) {
        self.name = name;
    }

    /// Gets the path to the audio file for this track.
    pub fn file(&self) -> &Path {
        &self.file
    }

    /// Gets the channel index within the audio file for this track.
    pub fn file_channel(&self) -> u16 {
        self.file_channel
    }

    pub fn get_config(&self) -> config::Track {
        config::Track::new(
            self.name().to_string(),
            filename_display(&self.file),
            Some(self.file_channel),
        )
    }
}

/// Renames tracks in-place so that every name is unique. When two or more tracks
/// share a name, a `-2`, `-3`, … suffix is appended to duplicates (the first
/// occurrence keeps its original name). Suffixed names are checked against all
/// existing names to avoid secondary collisions.
fn deduplicate_track_names(tracks: &mut [Track]) {
    let mut seen: HashSet<String> = HashSet::new();
    for track in tracks.iter_mut() {
        if seen.insert(track.name().to_string()) {
            continue;
        }
        let base = track.name().to_string();
        let mut n = 2u32;
        loop {
            let candidate = format!("{}-{}", base, n);
            if seen.insert(candidate.clone()) {
                track.set_name(candidate);
                break;
            }
            n += 1;
        }
    }
}

/// Contains a pre-computed MIDI timeline for playback.
pub struct MidiSheet {
    pub(crate) precomputed: crate::midi::playback::PrecomputedMidi,
    pub(crate) beat_clock: Option<crate::midi::beat_clock::PrecomputedBeatClock>,
}

/// A song that failed to load from disk.
#[derive(Clone, Debug)]
pub struct SongLoadFailure {
    name: String,
    base_path: PathBuf,
    error: String,
}

impl SongLoadFailure {
    /// Returns the name derived from the song's directory.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the base path of the failed song.
    pub fn base_path(&self) -> &Path {
        &self.base_path
    }

    /// Returns the error message describing the failure.
    pub fn error(&self) -> &str {
        &self.error
    }
}

/// A registry of songs for use by the multitrack player.
#[derive(Clone)]
pub struct Songs {
    /// A mapping of the songs in the repository.
    songs: HashMap<String, Arc<Song>>,
    /// Songs that failed to load.
    failures: Vec<SongLoadFailure>,
}

impl Songs {
    /// Creates a new songs registry.
    pub fn new(songs: HashMap<String, Arc<Song>>) -> Songs {
        Songs {
            songs,
            failures: vec![],
        }
    }

    /// Creates a new songs registry with load failures.
    pub fn with_failures(
        songs: HashMap<String, Arc<Song>>,
        failures: Vec<SongLoadFailure>,
    ) -> Songs {
        Songs { songs, failures }
    }

    /// Returns true if the song registry is empty.
    pub fn is_empty(&self) -> bool {
        self.songs.is_empty()
    }

    /// Gets a song from the song registry.
    pub fn get(&self, name: &str) -> Result<Arc<Song>, Box<dyn Error>> {
        match self.songs.get(name) {
            Some(song) => Ok(Arc::clone(song)),
            None => Err(format!("unable to find song {}", name).into()),
        }
    }

    /// Returns an unsorted list of songs in the song registry.
    pub fn list(&self) -> Vec<Arc<Song>> {
        self.songs
            .iter()
            .map(|song| song.1.clone())
            .collect::<Vec<Arc<Song>>>()
    }

    /// Returns a sorted list of the songs in the song registry.
    pub fn sorted_list(&self) -> Vec<Arc<Song>> {
        let mut sorted_songs = self.list();
        sorted_songs.sort_by_key(|song| song.name.clone());
        sorted_songs
    }

    /// Returns the length of the songs in the song registry.
    pub fn len(&self) -> usize {
        self.songs.len()
    }

    /// Returns the list of songs that failed to load.
    pub fn failures(&self) -> &[SongLoadFailure] {
        &self.failures
    }
}

/// Create default song configurations in a repository of song directories
pub fn initialize_songs(start_path: &Path) -> Result<usize, Box<dyn Error>> {
    let song_directories = fs::read_dir(start_path)?;
    let mut num_songs_found = 0;
    for song_directory in song_directories {
        let entry = song_directory?;
        let file_type = entry.file_type()?;
        if !file_type.is_dir() {
            continue;
        }
        let song_config = match Song::initialize(&entry.path()) {
            Ok(song) => {
                num_songs_found += 1;
                song.get_config()
            }
            Err(error) => return Err(error),
        };
        song_config.save(entry.path().join("song.yaml").as_path())?
    }
    info!("Found {num_songs_found} songs");
    Ok(num_songs_found)
}

/// Recurse into the given path and return all valid songs found.
pub fn get_all_songs(path: &Path) -> Result<Arc<Songs>, Box<dyn Error>> {
    debug!("Getting songs for directory {path:?}");
    let mut songs: HashMap<String, Arc<Song>> = HashMap::new();
    let mut failures: Vec<SongLoadFailure> = Vec::new();
    // codeql[rust/path-injection] path is the songs directory configured at startup.
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let path = entry.path();

        if path.is_dir() {
            // Skip hidden directories (.git, .claude, etc.).
            if path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with('.'))
            {
                continue;
            }

            let child = get_all_songs(path.as_path())?;
            child.list().iter().for_each(|song| {
                songs.insert(song.name().to_string(), song.clone());
            });
            failures.extend(child.failures().iter().cloned());
        }

        // Only attempt to deserialize YAML files as song configs.
        let is_yaml = path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext == "yaml" || ext == "yml");
        if is_yaml {
            // Peek at the `kind` field to decide how to handle this file:
            //   kind: song     → deserialize, record failure on error
            //   kind absent    → deserialize (backward compat), skip silently on error
            //   kind: playlist → skip (not a song config)
            let peeked_kind = config::peek_kind(path.as_path());
            let is_declared_song = peeked_kind.as_ref() == Some(&config::ConfigKind::Song);
            let should_try = peeked_kind.is_none() || is_declared_song;

            if should_try {
                match config::Song::deserialize(path.as_path()) {
                    Ok(song_config) => match path.parent() {
                        Some(parent) => match parent.canonicalize() {
                            Ok(canonical_parent) => {
                                match Song::new(&canonical_parent, &song_config) {
                                    Ok(song) => {
                                        songs.insert(song.name().to_string(), Arc::new(song));
                                    }
                                    Err(e) => {
                                        warn!("Skipping song at {}: {}", path.display(), e);
                                        // Song::new failures are always recorded — the YAML
                                        // parsed as a song but its content is invalid.
                                        failures.push(SongLoadFailure {
                                            name: song_config.name().to_string(),
                                            base_path: canonical_parent,
                                            error: format!("{}", e),
                                        });
                                    }
                                }
                            }
                            Err(e) => {
                                warn!(
                                    "Skipping song at {}: failed to canonicalize: {}",
                                    path.display(),
                                    e
                                );
                            }
                        },
                        None => {
                            warn!("Skipping song at {}: no parent directory", path.display());
                        }
                    },
                    Err(e) => {
                        // Only record deserialize failures for files with `kind: song`.
                        // Files without a kind that fail to deserialize are silently
                        // skipped — they may be playlists or other non-song YAML.
                        if is_declared_song {
                            if let Some(parent) = path.parent() {
                                let name = parent
                                    .file_name()
                                    .and_then(|n| n.to_str())
                                    .unwrap_or("unknown")
                                    .to_string();
                                let base_path = parent
                                    .canonicalize()
                                    .unwrap_or_else(|_| parent.to_path_buf());
                                warn!("Failed to deserialize song at {}: {}", path.display(), e);
                                failures.push(SongLoadFailure {
                                    name,
                                    base_path,
                                    error: format!("{}", e),
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    // Remove failures for songs that successfully loaded, and deduplicate
    // failures by name (a directory with multiple non-media files can produce
    // multiple failures with the same directory-derived name).
    failures.retain(|f| !songs.contains_key(f.name()));
    {
        let mut seen = std::collections::HashSet::new();
        failures.retain(|f| seen.insert(f.name().to_string()));
    }

    Ok(Arc::new(Songs::with_failures(songs, failures)))
}

#[cfg(test)]
mod test {
    use std::{
        error::Error,
        fs, io,
        path::{Path, PathBuf},
    };

    use thiserror::Error;

    use crate::{
        songs::{get_all_songs, initialize_songs},
        testutil::write_wav,
    };

    fn count_songs(path: &Path) -> Result<usize, TestError> {
        let songs = get_all_songs(path)?;
        Ok(songs.len())
    }

    fn create_song_dir(path: &Path, song_name: &str) -> Result<PathBuf, Box<dyn Error>> {
        let song_path = path.join(song_name);
        if !fs::exists(&song_path)? {
            fs::create_dir(&song_path)?;
        }
        Ok(song_path)
    }

    fn create_mono_song(path: &Path) -> Result<(), Box<dyn Error>> {
        let song_path = create_song_dir(path, "1 Song with mono track")?;

        write_wav(
            song_path.join("mono_track.wav"),
            vec![vec![1_i32, 2_i32, 3_i32, 4_i32, 5_i32]],
            44100,
        )?;

        Ok(())
    }

    fn create_stereo_song(path: &Path) -> Result<(), Box<dyn Error>> {
        let song_path = create_song_dir(path, "2 Song with stereo track")?;

        write_wav(
            song_path.join("stereo_track.wav"),
            vec![
                vec![1_i32, 2_i32, 3_i32, 4_i32, 5_i32],
                vec![5_i32, 4_i32, 3_i32, 2_i32, 1_i32],
            ],
            44100,
        )?;

        Ok(())
    }

    fn create_mono_and_stereo_song(path: &Path) -> Result<(), Box<dyn Error>> {
        let song_path = create_song_dir(path, "3 Song with mono and stereo tracks")?;

        write_wav(
            song_path.join("mono_track.wav"),
            vec![vec![1_i32, 2_i32, 3_i32, 4_i32, 5_i32]],
            44100,
        )?;

        write_wav(
            song_path.join("stereo_track.wav"),
            vec![
                vec![1_i32, 2_i32, 3_i32, 4_i32, 5_i32],
                vec![5_i32, 4_i32, 3_i32, 2_i32, 1_i32],
            ],
            44100,
        )?;

        Ok(())
    }

    fn create_eight_channel_song(path: &Path) -> Result<(), Box<dyn Error>> {
        let song_path = create_song_dir(path, "4 Song with eight tracks")?;

        write_wav(
            song_path.join("eight_channels_track.wav"),
            vec![
                vec![1_i32, 2_i32, 3_i32, 4_i32, 5_i32],
                vec![2_i32, 4_i32, 3_i32, 2_i32, 1_i32],
                vec![3_i32, 2_i32, 3_i32, 4_i32, 5_i32],
                vec![4_i32, 4_i32, 3_i32, 2_i32, 1_i32],
                vec![5_i32, 2_i32, 3_i32, 4_i32, 5_i32],
                vec![6_i32, 4_i32, 3_i32, 2_i32, 1_i32],
                vec![7_i32, 2_i32, 3_i32, 4_i32, 5_i32],
                vec![8_i32, 4_i32, 3_i32, 2_i32, 1_i32],
            ],
            44100,
        )?;

        Ok(())
    }

    fn create_midi_and_dmx_song(path: &Path) -> Result<(), Box<dyn Error>> {
        let song_path = create_song_dir(path, "5 Song with MIDI and DMX")?;
        fs::write(song_path.join("song.mid"), "")?;
        fs::write(song_path.join("dmx_lightshow.mid"), "")?;
        Ok(())
    }

    #[derive(Debug, Error)]
    enum TestError {
        #[error("Generic error! {0}")]
        Generic(#[from] Box<dyn Error>),
        #[error("I/O error! {0}")]
        IoError(#[from] io::Error),
        #[error("No song found")]
        NoSongFound,
    }

    #[test]
    fn test_init() -> Result<(), TestError> {
        let temp_dir = tempfile::tempdir()?;
        let temp_dir = temp_dir.path();
        assert_eq!(
            count_songs(temp_dir)?,
            0,
            "Expected no song in a newly created directory."
        );

        create_mono_song(temp_dir)?;
        check_first_song(temp_dir)?;

        create_stereo_song(temp_dir)?;
        check_second_song(temp_dir)?;

        create_mono_and_stereo_song(temp_dir)?;
        check_third_song(temp_dir)?;

        create_eight_channel_song(temp_dir)?;
        check_fourth_song(temp_dir)?;

        create_midi_and_dmx_song(temp_dir)?;
        check_fifth_song(temp_dir)?;

        Ok(())
    }

    fn check_first_song(temp_dir: &Path) -> Result<(), TestError> {
        assert_eq!(
            count_songs(temp_dir)?,
            0,
            "Expected no song in a newly created directory with one wav file."
        );
        let num_songs = initialize_songs(temp_dir)?;
        assert_eq!(num_songs, 1, "Expected to find one song configuration.");
        let songs = get_all_songs(temp_dir)?;
        let songs_list = songs.list();
        assert_eq!(
            songs_list.len(),
            num_songs,
            "Expected to find the number of songs to be equal to the number of configs created."
        );
        let first_song = songs_list.first().ok_or(TestError::NoSongFound)?;
        let tracks = first_song.tracks();
        assert_eq!(tracks.len(), 1, "Expected to find one track");
        assert_eq!(
            first_song.name(),
            "1 Song with mono track",
            "Name is not correct"
        );
        assert_eq!(first_song.num_channels, 1, "Unexpected number of channels");
        let track = tracks.first().unwrap();
        assert_eq!(track.name, "mono-track", "Unexpected name");
        assert!(fs::exists(&track.file).unwrap(), "Track file not found");
        Ok(())
    }

    fn check_second_song(temp_dir: &Path) -> Result<(), TestError> {
        let num_songs = initialize_songs(temp_dir)?;
        assert_eq!(num_songs, 2, "Expected to find two song configurations.");
        let songs = get_all_songs(temp_dir)?;
        let songs_list = songs.sorted_list();
        assert_eq!(
            songs_list.len(),
            num_songs,
            "Expected to find the number of songs to be equal to the number of configs created."
        );
        let first_song = songs_list.first().ok_or(TestError::NoSongFound)?;
        assert_eq!(first_song.tracks().len(), 1, "Expected to find one track");
        let second_song = songs_list.iter().last().ok_or(TestError::NoSongFound)?;
        assert_eq!(second_song.tracks().len(), 2, "Expected to find two tracks");
        Ok(())
    }

    fn check_third_song(temp_dir: &Path) -> Result<(), TestError> {
        let num_songs = initialize_songs(temp_dir)?;
        assert_eq!(num_songs, 3, "Expected to find three song configurations.");
        let songs = get_all_songs(temp_dir)?;
        let songs_list = songs.sorted_list();
        assert_eq!(
            songs_list.len(),
            num_songs,
            "Expected to find the number of songs to be equal to the number of configs created."
        );
        let first_song = songs_list.first().ok_or(TestError::NoSongFound)?;
        assert_eq!(first_song.tracks().len(), 1, "Expected to find one track");
        let third_song = songs_list.iter().last().ok_or(TestError::NoSongFound)?;
        assert_eq!(
            third_song.tracks().len(),
            3,
            "Expected to find three tracks."
        );
        Ok(())
    }

    fn check_fourth_song(temp_dir: &Path) -> Result<(), TestError> {
        let num_songs = initialize_songs(temp_dir)?;
        assert_eq!(num_songs, 4, "Expected to find four song configurations.");
        let songs = get_all_songs(temp_dir)?;
        let songs_list = songs.sorted_list();
        assert_eq!(
            songs_list.len(),
            num_songs,
            "Expected to find the number of songs to be equal to the number of configs created."
        );
        let fourth_song = songs_list.iter().last().ok_or(TestError::NoSongFound)?;
        assert_eq!(
            fourth_song.tracks().len(),
            8,
            "Expected to find eight tracks."
        );
        assert_eq!(fourth_song.num_channels, 8, "Unexpected number of channels");
        Ok(())
    }

    fn check_fifth_song(temp_dir: &Path) -> Result<(), TestError> {
        let num_songs = initialize_songs(temp_dir)?;
        assert_eq!(num_songs, 5, "Expected to find five song configurations.");
        let songs = get_all_songs(temp_dir)?;
        let songs_list = songs.sorted_list();
        assert_eq!(
            songs_list.len(),
            num_songs,
            "Expected to find the number of songs to be equal to the number of configs created."
        );
        let fifth_song = songs_list.iter().last().ok_or(TestError::NoSongFound)?;
        assert_eq!(
            fifth_song.tracks().len(),
            0,
            "Expected to find zero tracks."
        );
        assert_eq!(fifth_song.num_channels, 0, "Unexpected number of channels.");

        assert!(
            fifth_song.midi_playback().is_some(),
            "Expected song to have MIDI playback."
        );
        assert_eq!(
            fifth_song.light_shows().len(),
            1,
            "Expected song to have a light show."
        );
        Ok(())
    }

    #[test]
    fn test_write_wav_formats() -> Result<(), Box<dyn Error>> {
        let tempdir = tempfile::tempdir()?.keep();

        let i32_samples: Vec<i32> = vec![1, 2, 3, 4, 5];
        let i32_path = tempdir.join("test_i32.wav");
        write_wav(i32_path.clone(), vec![i32_samples], 44100)?;

        let f32_samples: Vec<f32> = vec![0.1, 0.2, 0.3, 0.4, 0.5];
        let f32_path = tempdir.join("test_f32.wav");
        write_wav(f32_path.clone(), vec![f32_samples], 44100)?;

        println!("Both formats work!");
        Ok(())
    }

    #[test]
    fn test_transcoding_detection() {
        use crate::audio::SampleFormat;
        use crate::audio::TargetFormat;
        use crate::testutil::write_wav_with_bits;
        use tempfile::tempdir;

        let tempdir = tempdir().unwrap();
        let wav_path = tempdir.path().join("test.wav");

        // Create a test WAV file
        let samples: Vec<i32> = vec![1000, 2000, 3000, 4000, 5000];
        write_wav_with_bits(wav_path.clone(), vec![samples], 44100, 16).unwrap();

        // Create a test song with the WAV file
        let song = super::Song {
            sample_rate: 44100,
            sample_format: SampleFormat::Int,
            tracks: vec![super::Track {
                name: "test".to_string(),
                file: wav_path,
                file_channel: 1,
                sample_rate: 44100,
                sample_format: SampleFormat::Int,
                duration: std::time::Duration::from_secs(1),
            }],
            ..Default::default()
        };

        // Test with same format - should not need transcoding
        let target_format = TargetFormat::new(44100, SampleFormat::Int, 16).unwrap();
        assert!(!song.needs_transcoding(&target_format));

        // Test with different sample rate - should need transcoding
        let target_format = TargetFormat::new(48000, SampleFormat::Int, 16).unwrap();
        assert!(song.needs_transcoding(&target_format));

        // Test with different format - should need transcoding
        let target_format = TargetFormat::new(44100, SampleFormat::Float, 32).unwrap();
        assert!(song.needs_transcoding(&target_format));
    }

    #[test]
    fn test_no_transcoding_for_identical_formats() {
        use crate::audio::SampleFormat;
        use crate::audio::TargetFormat;
        use crate::testutil::write_wav_with_bits;
        use tempfile::tempdir;

        let tempdir = tempdir().unwrap();
        let wav_path = tempdir.path().join("test.wav");

        // Create a test WAV file
        let samples: Vec<i32> = vec![1000, 2000, 3000, 4000, 5000];
        write_wav_with_bits(wav_path.clone(), vec![samples], 44100, 16).unwrap();

        // Test that identical formats don't trigger transcoding
        let target_format = TargetFormat::new(44100, SampleFormat::Int, 16).unwrap();

        // Create a song with identical format
        let song = super::Song {
            sample_rate: 44100,
            sample_format: SampleFormat::Int,
            tracks: vec![super::Track {
                name: "test".to_string(),
                file: wav_path,
                file_channel: 1,
                sample_rate: 44100,
                sample_format: SampleFormat::Int,
                duration: std::time::Duration::from_secs(1),
            }],
            ..Default::default()
        };

        // Should not need transcoding for identical formats
        assert!(!song.needs_transcoding(&target_format));
    }

    #[test]
    fn test_file_io_performance() -> Result<(), Box<dyn Error>> {
        use crate::testutil::write_wav_with_bits;
        use std::time::Instant;
        use tempfile::tempdir;

        let tempdir = tempdir().unwrap();

        // Create a large WAV file to test I/O performance
        let sample_rate = 44100;
        let duration_samples = 1000000; // ~22.7 seconds of audio
        let amplitude = 0.5;

        println!(
            "Creating large WAV file with {} samples...",
            duration_samples
        );

        // Generate a sine wave
        let sine_wave: Vec<f32> = (0..duration_samples)
            .map(|i| {
                (i as f32 * 440.0 * 2.0 * std::f32::consts::PI / sample_rate as f32).sin()
                    * amplitude
            })
            .collect();

        let samples: Vec<i32> = sine_wave
            .iter()
            .map(|&x| (x * 8388607.0) as i32) // 24-bit range
            .collect();

        // Measure file writing time
        let wav_path = tempdir.path().join("large_test.wav");
        let start = Instant::now();
        write_wav_with_bits(wav_path.clone(), vec![samples], sample_rate, 24).unwrap();
        let write_time = start.elapsed();

        println!("File write time: {:?}", write_time);
        println!(
            "Write speed: {:.2} MB/s",
            (duration_samples * 4) as f64 / write_time.as_secs_f64() / 1_000_000.0
        );

        // Measure file reading time with sample source
        let start = Instant::now();
        let mut source =
            crate::audio::sample_source::create_sample_source_from_file(&wav_path, None, 1024)?;
        println!(
            "WAV file spec: {}Hz, {}bit, {}ch",
            source.sample_rate(),
            source.bits_per_sample(),
            source.channel_count()
        );

        // Read all samples
        let mut samples_read = 0;
        while let Ok(Some(_)) = source.next_sample() {
            samples_read += 1;
        }
        let read_time = start.elapsed();

        println!("File read time: {:?}", read_time);
        println!(
            "Read speed: {:.2} MB/s",
            (samples_read * 4) as f64 / read_time.as_secs_f64() / 1_000_000.0
        );
        println!("Samples read: {}", samples_read);

        // Measure AudioSampleSource performance
        let start = Instant::now();
        let mut wav_source = crate::audio::sample_source::audio::AudioSampleSource::from_file(
            &wav_path, None, 1024,
        )?;
        let mut samples_processed = 0;

        loop {
            match crate::audio::sample_source::traits::SampleSource::next_sample(&mut wav_source) {
                Ok(Some(_)) => samples_processed += 1,
                Ok(None) => break,
                Err(e) => return Err(e.into()),
            }
        }
        let wav_source_time = start.elapsed();

        println!("AudioSampleSource processing time: {:?}", wav_source_time);
        println!(
            "AudioSampleSource speed: {:.2} MB/s",
            (samples_processed * 4) as f64 / wav_source_time.as_secs_f64() / 1_000_000.0
        );
        println!("Samples processed: {}", samples_processed);

        // Verify we got the expected number of samples
        assert_eq!(samples_read, duration_samples);
        assert_eq!(samples_processed, duration_samples);

        Ok(())
    }

    // Removed test_buffer_fill_performance - not relevant with Crossbeam channels

    // ── Pure function tests (no I/O) ─────────────────────────────────

    #[test]
    fn duration_string_zero() {
        let song = super::Song::new_for_test("test", &["t1"]);
        assert_eq!(song.duration_string(), "0:00");
    }

    #[test]
    fn duration_string_formatted() {
        let mut song = super::Song::new_for_test("test", &["t1"]);
        song.duration = std::time::Duration::from_secs(125); // 2:05
        assert_eq!(song.duration_string(), "2:05");
    }

    #[test]
    fn duration_string_exact_minute() {
        let mut song = super::Song::new_for_test("test", &["t1"]);
        song.duration = std::time::Duration::from_secs(180); // 3:00
        assert_eq!(song.duration_string(), "3:00");
    }

    #[test]
    fn duration_string_long() {
        let mut song = super::Song::new_for_test("test", &["t1"]);
        song.duration = std::time::Duration::from_secs(3661); // 61:01
        assert_eq!(song.duration_string(), "61:01");
    }

    #[test]
    fn song_name() {
        let song = super::Song::new_for_test("My Song", &["t1"]);
        assert_eq!(song.name(), "My Song");
    }

    #[test]
    fn song_tracks_count() {
        let song = super::Song::new_for_test("test", &["kick", "snare", "bass"]);
        assert_eq!(song.tracks().len(), 3);
        assert_eq!(song.tracks()[0].name, "kick");
    }

    #[test]
    fn song_num_channels() {
        let song = super::Song::new_for_test("test", &["t1", "t2"]);
        // new_for_test creates tracks with 0 num_channels by default
        assert_eq!(song.num_channels, 0);
    }

    #[test]
    fn song_default_no_midi() {
        let song = super::Song::new_for_test("test", &["t1"]);
        assert!(song.midi_playback().is_none());
        assert!(song.midi_event.is_none());
    }

    #[test]
    fn song_light_shows_empty() {
        let song = super::Song::new_for_test("test", &["t1"]);
        assert!(song.light_shows().is_empty());
    }

    #[test]
    fn song_dsl_lighting_shows_empty() {
        let song = super::Song::new_for_test("test", &["t1"]);
        assert!(song.dsl_lighting_shows().is_empty());
    }

    // ── Songs registry tests ─────────────────────────────────────────

    #[test]
    fn songs_empty() {
        let songs = super::Songs::new(std::collections::HashMap::new());
        assert!(songs.is_empty());
        assert_eq!(songs.len(), 0);
        assert!(songs.list().is_empty());
    }

    #[test]
    fn songs_get_found() {
        let mut map = std::collections::HashMap::new();
        map.insert(
            "Song A".to_string(),
            std::sync::Arc::new(super::Song::new_for_test("Song A", &["t1"])),
        );
        let songs = super::Songs::new(map);
        let song = songs.get("Song A").unwrap();
        assert_eq!(song.name(), "Song A");
    }

    #[test]
    fn songs_get_not_found() {
        let songs = super::Songs::new(std::collections::HashMap::new());
        assert!(songs.get("nonexistent").is_err());
    }

    #[test]
    fn songs_len() {
        let mut map = std::collections::HashMap::new();
        map.insert(
            "A".to_string(),
            std::sync::Arc::new(super::Song::new_for_test("A", &["t"])),
        );
        map.insert(
            "B".to_string(),
            std::sync::Arc::new(super::Song::new_for_test("B", &["t"])),
        );
        let songs = super::Songs::new(map);
        assert_eq!(songs.len(), 2);
        assert!(!songs.is_empty());
    }

    #[test]
    fn songs_sorted_list() {
        let mut map = std::collections::HashMap::new();
        for name in &["Charlie", "Alpha", "Bravo"] {
            map.insert(
                name.to_string(),
                std::sync::Arc::new(super::Song::new_for_test(name, &["t"])),
            );
        }
        let songs = super::Songs::new(map);
        let sorted = songs.sorted_list();
        assert_eq!(sorted[0].name(), "Alpha");
        assert_eq!(sorted[1].name(), "Bravo");
        assert_eq!(sorted[2].name(), "Charlie");
    }

    #[test]
    fn song_to_proto() {
        let mut song = super::Song::new_for_test("Proto Song", &["kick", "snare"]);
        song.duration = std::time::Duration::from_secs(90);
        let proto = song.to_proto().unwrap();
        assert_eq!(proto.name, "Proto Song");
        assert_eq!(proto.tracks, vec!["kick", "snare"]);
        let dur = proto.duration.unwrap();
        assert_eq!(dur.seconds, 90);
        assert_eq!(dur.nanos, 0);
    }

    #[test]
    fn song_to_proto_zero_duration() {
        let song = super::Song::new_for_test("Zero", &[]);
        let proto = song.to_proto().unwrap();
        assert_eq!(proto.name, "Zero");
        assert!(proto.tracks.is_empty());
    }

    #[test]
    fn song_display_no_midi() {
        let mut song = super::Song::new_for_test("Display Song", &["t1", "t2"]);
        song.num_channels = 2;
        song.sample_rate = 44100;
        song.duration = std::time::Duration::from_secs(65);
        let display = format!("{song}");
        assert!(display.contains("Name: Display Song"));
        assert!(display.contains("Duration: 1:05"));
        assert!(display.contains("Channels: 2"));
        assert!(display.contains("Sample Rate: 44100"));
        assert!(display.contains("Tracks: t1, t2"));
        assert!(display.contains("Midi Message: None"));
        assert!(display.contains("Midi File:None"));
    }

    #[test]
    fn song_display_with_midi_playback() {
        let mut song = super::Song::new_for_test("Midi Song", &["bass"]);
        song.midi_playback = Some(super::MidiPlayback {
            file: PathBuf::from("/tmp/test.mid"),
            exclude_midi_channels: vec![],
        });
        let display = format!("{song}");
        assert!(display.contains("Midi File:Some"));
        assert!(display.contains("test.mid"));
    }

    #[test]
    fn song_samples_config_default() {
        let song = super::Song::new_for_test("test", &["t1"]);
        let config = song.samples_config();
        assert!(config.samples().is_empty());
    }

    #[test]
    fn song_new_from_config_with_midi() -> Result<(), Box<dyn Error>> {
        let assets = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets");
        let song_config =
            crate::config::Song::deserialize(assets.join("songs/song1.yaml").as_path())
                .expect("Failed to deserialize song1.yaml");
        let start = assets.join("songs").canonicalize()?;
        let song = super::Song::new(&start, &song_config)?;
        assert_eq!(song.name(), "Song 1");
        assert!(song.midi_playback().is_some());
        assert_eq!(song.tracks().len(), 1);
        assert_eq!(song.tracks()[0].name(), "track 1");
        assert!(song.light_shows().is_empty());
        assert!(song.dsl_lighting_shows().is_empty());
        Ok(())
    }

    #[test]
    fn song_new_from_config_with_midi_event() -> Result<(), Box<dyn Error>> {
        let assets = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets");
        let song_config =
            crate::config::Song::deserialize(assets.join("songs/song2.yaml").as_path())
                .expect("Failed to deserialize song2.yaml");
        let start = assets.join("songs").canonicalize()?;
        let song = super::Song::new(&start, &song_config)?;
        assert_eq!(song.name(), "Song 2");
        assert!(song.midi_event().is_some());
        assert!(song.midi_playback().is_none());
        Ok(())
    }

    #[test]
    fn song_new_from_config_multichannel() -> Result<(), Box<dyn Error>> {
        let assets = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets");
        let song_config =
            crate::config::Song::deserialize(assets.join("songs/song3.yaml").as_path())
                .expect("Failed to deserialize song3.yaml");
        let start = assets.join("songs").canonicalize()?;
        let song = super::Song::new(&start, &song_config)?;
        assert_eq!(song.name(), "Song 3");
        assert_eq!(song.tracks().len(), 2);
        assert_eq!(song.num_channels(), 2);
        assert!(song.midi_playback().is_some());
        Ok(())
    }

    #[test]
    fn song_new_from_config_eight_channels() -> Result<(), Box<dyn Error>> {
        let assets = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets");
        let song_config =
            crate::config::Song::deserialize(assets.join("songs/song4.yaml").as_path())
                .expect("Failed to deserialize song4.yaml");
        let start = assets.join("songs").canonicalize()?;
        let song = super::Song::new(&start, &song_config)?;
        assert_eq!(song.name(), "Song 4");
        assert_eq!(song.tracks().len(), 8);
        assert_eq!(song.num_channels(), 8);
        Ok(())
    }

    #[test]
    fn midi_playback_new_file_not_found() {
        let song_config = crate::config::Song::new(
            "test",
            None,
            Some("nonexistent.mid".to_string()),
            None,
            None,
            None,
            vec![],
            std::collections::HashMap::new(),
            vec![],
        );
        let midi_pb = song_config.midi_playback().unwrap();
        let err = super::MidiPlayback::new(Path::new("/tmp"), midi_pb)
            .err()
            .expect("expected error")
            .to_string();
        assert!(err.contains("does not exist"), "Error: {err}");
    }

    #[test]
    fn midi_playback_midi_sheet_valid() -> Result<(), Box<dyn Error>> {
        // Create a minimal valid MIDI file
        let tempdir = tempfile::tempdir()?;
        let midi_path = tempdir.path().join("test.mid");
        // Standard MIDI File: header chunk + one empty track
        let midi_bytes: Vec<u8> = vec![
            0x4D, 0x54, 0x68, 0x64, // MThd
            0x00, 0x00, 0x00, 0x06, // chunk length = 6
            0x00, 0x00, // format 0
            0x00, 0x01, // 1 track
            0x00, 0x60, // 96 ticks per beat
            0x4D, 0x54, 0x72, 0x6B, // MTrk
            0x00, 0x00, 0x00, 0x04, // chunk length = 4
            0x00, 0xFF, 0x2F, 0x00, // delta=0, end of track
        ];
        fs::write(&midi_path, &midi_bytes)?;
        let song_config = crate::config::Song::new(
            "test",
            None,
            Some("test.mid".to_string()),
            None,
            None,
            None,
            vec![],
            std::collections::HashMap::new(),
            vec![],
        );
        let midi_pb = song_config.midi_playback().unwrap();
        let playback = super::MidiPlayback::new(tempdir.path(), midi_pb)?;
        let sheet = playback.midi_sheet()?;
        // No tempo events in the MIDI file, so beat clock should be None
        assert!(sheet.beat_clock.is_none());
        Ok(())
    }

    #[test]
    fn midi_sheet_with_tempo_has_beat_clock() -> Result<(), Box<dyn Error>> {
        // Create a MIDI file with an explicit tempo event
        let tempdir = tempfile::tempdir()?;
        let midi_path = tempdir.path().join("test.mid");
        let midi_bytes: Vec<u8> = vec![
            0x4D, 0x54, 0x68, 0x64, // MThd
            0x00, 0x00, 0x00, 0x06, // chunk length = 6
            0x00, 0x00, // format 0
            0x00, 0x01, // 1 track
            0x00, 0x60, // 96 ticks per beat
            0x4D, 0x54, 0x72, 0x6B, // MTrk
            0x00, 0x00, 0x00, 0x0B, // chunk length = 11
            0x00, 0xFF, 0x51, 0x03, 0x07, 0xA1,
            0x20, // delta=0, tempo = 500_000 µs (120 BPM)
            0x60, 0xFF, 0x2F, 0x00, // delta=96, end of track (1 beat later)
        ];
        fs::write(&midi_path, &midi_bytes)?;
        let song_config = crate::config::Song::new(
            "test",
            None,
            Some("test.mid".to_string()),
            None,
            None,
            None,
            vec![],
            std::collections::HashMap::new(),
            vec![],
        );
        let midi_pb = song_config.midi_playback().unwrap();
        let playback = super::MidiPlayback::new(tempdir.path(), midi_pb)?;
        let sheet = playback.midi_sheet()?;
        // Has tempo events, so beat clock should be Some
        assert!(sheet.beat_clock.is_some());
        Ok(())
    }

    #[test]
    fn midi_playback_exclude_channels() -> Result<(), Box<dyn Error>> {
        // Create a minimal valid MIDI file for the test
        let tempdir = tempfile::tempdir()?;
        let midi_path = tempdir.path().join("test.mid");
        let midi_bytes: Vec<u8> = vec![
            0x4D, 0x54, 0x68, 0x64, 0x00, 0x00, 0x00, 0x06, 0x00, 0x00, 0x00, 0x01, 0x00, 0x60,
            0x4D, 0x54, 0x72, 0x6B, 0x00, 0x00, 0x00, 0x04, 0x00, 0xFF, 0x2F, 0x00,
        ];
        fs::write(&midi_path, &midi_bytes)?;
        let song_config = crate::config::Song::new(
            "test",
            None,
            Some("test.mid".to_string()),
            None,
            None,
            None,
            vec![],
            std::collections::HashMap::new(),
            vec![],
        );
        let midi_pb = song_config.midi_playback().unwrap();
        let playback = super::MidiPlayback::new(tempdir.path(), midi_pb)?;
        let excluded = playback.exclude_midi_channels();
        assert!(excluded.is_empty());
        Ok(())
    }

    #[test]
    fn light_show_new_file_not_found() {
        let config = crate::config::LightShow::new(
            "universe".to_string(),
            "nonexistent.mid".to_string(),
            None,
        );
        let err = super::LightShow::new(Path::new("/tmp"), &config)
            .err()
            .expect("expected error")
            .to_string();
        assert!(err.contains("does not exist"), "Error: {err}");
    }

    #[test]
    fn track_new_file_not_found() {
        let config =
            crate::config::Track::new("test track".to_string(), "nonexistent.wav", Some(1));
        let err = super::Track::new(Path::new("/tmp"), &config)
            .err()
            .expect("expected error")
            .to_string();
        assert!(err.contains("track \"test track\""), "Error: {err}");
    }

    #[test]
    fn track_new_multichannel_without_file_channel() {
        let assets = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets");
        let config = crate::config::Track::new("stereo".to_string(), "2Channel44.1k.wav", None);
        let err = super::Track::new(&assets, &config)
            .err()
            .expect("expected error")
            .to_string();
        assert!(err.contains("more than one channel"), "Error: {err}");
    }

    #[test]
    fn track_load_tracks_wrong_extension() {
        let err = super::Track::load_tracks(&PathBuf::from("/tmp/test.mp3"))
            .err()
            .expect("expected error")
            .to_string();
        assert!(err.contains(".mp3"), "Error: {err}");
    }

    #[test]
    fn track_load_tracks_mono() -> Result<(), Box<dyn Error>> {
        let tempdir = tempfile::tempdir()?;
        let wav_path = tempdir.path().join("mono.wav");
        crate::testutil::write_wav(wav_path.clone(), vec![vec![1_i32, 2, 3, 4, 5]], 44100)?;
        let tracks = super::Track::load_tracks(&wav_path)?;
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].name(), "mono");
        assert_eq!(tracks[0].file_channel(), 1);
        Ok(())
    }

    #[test]
    fn track_load_tracks_stereo() -> Result<(), Box<dyn Error>> {
        let tempdir = tempfile::tempdir()?;
        let wav_path = tempdir.path().join("stereo.wav");
        crate::testutil::write_wav(
            wav_path.clone(),
            vec![vec![1_i32, 2, 3, 4, 5], vec![5, 4, 3, 2, 1]],
            44100,
        )?;
        let tracks = super::Track::load_tracks(&wav_path)?;
        assert_eq!(tracks.len(), 2);
        assert_eq!(tracks[0].name(), "stereo-l");
        assert_eq!(tracks[0].file_channel(), 1);
        assert_eq!(tracks[1].name(), "stereo-r");
        assert_eq!(tracks[1].file_channel(), 2);
        Ok(())
    }

    #[test]
    fn track_load_tracks_multichannel() -> Result<(), Box<dyn Error>> {
        let tempdir = tempfile::tempdir()?;
        let wav_path = tempdir.path().join("multi.wav");
        let channels: Vec<Vec<i32>> = (0..4).map(|i| vec![i; 5]).collect();
        crate::testutil::write_wav(wav_path.clone(), channels, 44100)?;
        let tracks = super::Track::load_tracks(&wav_path)?;
        assert_eq!(tracks.len(), 4);
        assert_eq!(tracks[0].name(), "multi-1");
        assert_eq!(tracks[0].file_channel(), 1);
        assert_eq!(tracks[3].name(), "multi-4");
        assert_eq!(tracks[3].file_channel(), 4);
        Ok(())
    }

    #[test]
    fn track_get_config() -> Result<(), Box<dyn Error>> {
        let tempdir = tempfile::tempdir()?;
        let wav_path = tempdir.path().join("test.wav");
        crate::testutil::write_wav(wav_path.clone(), vec![vec![1_i32, 2, 3]], 44100)?;
        let tracks = super::Track::load_tracks(&wav_path)?;
        let config = tracks[0].get_config();
        assert_eq!(config.name(), "test");
        assert_eq!(config.file_channel(), Some(1));
        Ok(())
    }

    #[test]
    fn deduplicate_track_names_no_collision() {
        let mut tracks = vec![make_track("click"), make_track("cue")];
        super::deduplicate_track_names(&mut tracks);
        assert_eq!(tracks[0].name(), "click");
        assert_eq!(tracks[1].name(), "cue");
    }

    #[test]
    fn deduplicate_track_names_with_collision() {
        let mut tracks = vec![
            make_track("backing-track"),
            make_track("backing-track"),
            make_track("click"),
            make_track("backing-track"),
        ];
        super::deduplicate_track_names(&mut tracks);
        assert_eq!(tracks[0].name(), "backing-track");
        assert_eq!(tracks[1].name(), "backing-track-2");
        assert_eq!(tracks[2].name(), "click");
        assert_eq!(tracks[3].name(), "backing-track-3");
    }

    #[test]
    fn deduplicate_track_names_suffix_collision() {
        // "foo-2" already exists, so the duplicate "foo" must skip to "foo-3".
        let mut tracks = vec![make_track("foo-2"), make_track("foo"), make_track("foo")];
        super::deduplicate_track_names(&mut tracks);
        assert_eq!(tracks[0].name(), "foo-2");
        assert_eq!(tracks[1].name(), "foo");
        assert_eq!(tracks[2].name(), "foo-3");
    }

    fn make_track(name: &str) -> super::Track {
        super::Track {
            name: name.to_string(),
            file: PathBuf::from("/dev/null"),
            file_channel: 1,
            sample_rate: 44100,
            sample_format: crate::audio::SampleFormat::Int,
            duration: std::time::Duration::ZERO,
        }
    }

    #[test]
    fn dsl_lighting_show_file_not_found() {
        let config = crate::config::LightingShow::new("nonexistent.dsl".to_string());
        let err = super::DslLightingShow::new(Path::new("/tmp"), &config)
            .expect_err("expected error")
            .to_string();
        assert!(err.contains("does not exist"), "Error: {err}");
    }

    #[test]
    fn dsl_lighting_show_parse_error() -> Result<(), Box<dyn Error>> {
        let tempdir = tempfile::tempdir()?;
        fs::write(tempdir.path().join("bad.dsl"), "show {")?;
        let config = crate::config::LightingShow::new("bad.dsl".to_string());
        let err = super::DslLightingShow::new(tempdir.path(), &config)
            .expect_err("expected error")
            .to_string();
        assert!(
            err.contains("Failed to parse DSL lighting show"),
            "Error: {err}"
        );
        Ok(())
    }

    #[test]
    fn dsl_lighting_show_valid() -> Result<(), Box<dyn Error>> {
        let tempdir = tempfile::tempdir()?;
        fs::write(tempdir.path().join("valid.dsl"), "# just a comment\n")?;
        let config = crate::config::LightingShow::new("valid.dsl".to_string());
        let show = super::DslLightingShow::new(tempdir.path(), &config)?;
        assert_eq!(show.file_path(), tempdir.path().join("valid.dsl"));
        assert!(show.shows().is_empty());
        Ok(())
    }

    #[test]
    fn dsl_lighting_show_absolute_path() -> Result<(), Box<dyn Error>> {
        let tempdir = tempfile::tempdir()?;
        let abs_path = tempdir.path().join("absolute.dsl");
        fs::write(&abs_path, "")?;
        let config = crate::config::LightingShow::new(abs_path.to_string_lossy().to_string());
        let show = super::DslLightingShow::new(Path::new("/some/other/path"), &config)?;
        assert_eq!(show.file_path(), abs_path);
        Ok(())
    }

    #[test]
    fn song_initialize_with_subdirectory() -> Result<(), Box<dyn Error>> {
        let tempdir = tempfile::tempdir()?;
        let song_dir = tempdir.path().join("my_song");
        fs::create_dir(&song_dir)?;
        fs::create_dir(song_dir.join("subdir"))?;
        crate::testutil::write_wav(
            song_dir.join("track.wav"),
            vec![vec![1_i32, 2, 3, 4, 5]],
            44100,
        )?;
        let song = super::Song::initialize(&song_dir)?;
        assert_eq!(song.name(), "my_song");
        assert_eq!(song.tracks().len(), 1);
        Ok(())
    }

    #[test]
    fn song_initialize_unknown_extension() -> Result<(), Box<dyn Error>> {
        let tempdir = tempfile::tempdir()?;
        let song_dir = tempdir.path().join("unknown_ext_song");
        fs::create_dir(&song_dir)?;
        fs::write(song_dir.join("notes.txt"), "some notes")?;
        let song = super::Song::initialize(&song_dir)?;
        assert!(song.tracks().is_empty());
        Ok(())
    }

    #[test]
    fn song_get_config_roundtrip() -> Result<(), Box<dyn Error>> {
        let assets = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets");
        let song_config =
            crate::config::Song::deserialize(assets.join("songs/song1.yaml").as_path())
                .expect("Failed to deserialize song1.yaml");
        let start = assets.join("songs").canonicalize()?;
        let song = super::Song::new(&start, &song_config)?;
        let config = song.get_config();
        assert_eq!(config.name(), "Song 1");
        assert_eq!(config.tracks().len(), 1);
        Ok(())
    }

    #[test]
    fn needs_transcoding_no_tracks() {
        use crate::audio::TargetFormat;
        let song = super::Song {
            tracks: vec![],
            ..Default::default()
        };
        let target = TargetFormat::new(44100, crate::audio::SampleFormat::Int, 16).unwrap();
        assert!(!song.needs_transcoding(&target));
    }

    #[test]
    fn needs_transcoding_bad_file() {
        use crate::audio::TargetFormat;
        let song = super::Song {
            tracks: vec![super::Track {
                name: "bad".to_string(),
                file: PathBuf::from("/nonexistent/file.wav"),
                file_channel: 1,
                sample_rate: 44100,
                sample_format: crate::audio::SampleFormat::Int,
                duration: std::time::Duration::ZERO,
            }],
            ..Default::default()
        };
        let target = TargetFormat::new(44100, crate::audio::SampleFormat::Int, 16).unwrap();
        assert!(song.needs_transcoding(&target));
    }

    #[test]
    fn get_all_songs_skips_git_directory() -> Result<(), Box<dyn Error>> {
        let tempdir = tempfile::tempdir()?;
        fs::create_dir(tempdir.path().join(".git"))?;
        let songs = get_all_songs(tempdir.path())?;
        assert!(songs.is_empty());
        Ok(())
    }

    #[test]
    fn get_all_songs_with_song_config() -> Result<(), Box<dyn Error>> {
        let tempdir = tempfile::tempdir()?;
        let song_dir = tempdir.path().join("my_song");
        fs::create_dir(&song_dir)?;
        crate::testutil::write_wav(
            song_dir.join("track.wav"),
            vec![vec![1_i32, 2, 3, 4, 5]],
            44100,
        )?;
        initialize_songs(tempdir.path())?;
        let songs = get_all_songs(tempdir.path())?;
        assert_eq!(songs.len(), 1);
        Ok(())
    }

    #[test]
    fn get_all_songs_nonexistent_path() {
        let result = get_all_songs(Path::new("/nonexistent/path"));
        assert!(result.is_err());
    }

    #[test]
    fn needs_transcoding_different_sample_rate() {
        use crate::audio::TargetFormat;
        let song = super::Song {
            tracks: vec![super::Track {
                name: "track".to_string(),
                file: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/1Channel44.1k.wav"),
                file_channel: 1,
                sample_rate: 48000,
                sample_format: crate::audio::SampleFormat::Int,
                duration: std::time::Duration::ZERO,
            }],
            ..Default::default()
        };
        let target = TargetFormat::new(44100, crate::audio::SampleFormat::Int, 16).unwrap();
        assert!(song.needs_transcoding(&target));
    }

    #[test]
    fn needs_transcoding_different_sample_format() {
        use crate::audio::TargetFormat;
        let song = super::Song {
            tracks: vec![super::Track {
                name: "track".to_string(),
                file: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/1Channel44.1k.wav"),
                file_channel: 1,
                sample_rate: 44100,
                sample_format: crate::audio::SampleFormat::Float,
                duration: std::time::Duration::ZERO,
            }],
            ..Default::default()
        };
        let target = TargetFormat::new(44100, crate::audio::SampleFormat::Int, 16).unwrap();
        assert!(song.needs_transcoding(&target));
    }

    #[test]
    fn song_new_with_different_sample_rates() -> Result<(), Box<dyn Error>> {
        let tempdir = tempfile::tempdir()?;
        crate::testutil::write_wav(
            tempdir.path().join("track1.wav"),
            vec![vec![1_i32; 100]],
            44100,
        )?;
        crate::testutil::write_wav(
            tempdir.path().join("track2.wav"),
            vec![vec![1_i32; 100]],
            48000,
        )?;

        let song_config = crate::config::Song::new(
            "mixed rates",
            None,
            None,
            None,
            None,
            None,
            vec![
                crate::config::Track::new("t1".to_string(), "track1.wav", Some(1)),
                crate::config::Track::new("t2".to_string(), "track2.wav", Some(1)),
            ],
            std::collections::HashMap::new(),
            vec![],
        );
        let song = super::Song::new(tempdir.path(), &song_config)?;
        assert_eq!(song.tracks.len(), 2);
        Ok(())
    }

    #[test]
    fn song_new_with_different_sample_formats() -> Result<(), Box<dyn Error>> {
        let tempdir = tempfile::tempdir()?;
        crate::testutil::write_wav(
            tempdir.path().join("track_int.wav"),
            vec![vec![1_i32; 100]],
            44100,
        )?;

        // Create a float WAV file
        let float_path = tempdir.path().join("track_float.wav");
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 44100,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        };
        let mut writer = hound::WavWriter::create(&float_path, spec)?;
        for i in 0..100 {
            writer.write_sample(i as f32 / 100.0)?;
        }
        writer.finalize()?;

        let song_config = crate::config::Song::new(
            "mixed formats",
            None,
            None,
            None,
            None,
            None,
            vec![
                crate::config::Track::new("t_int".to_string(), "track_int.wav", Some(1)),
                crate::config::Track::new("t_float".to_string(), "track_float.wav", Some(1)),
            ],
            std::collections::HashMap::new(),
            vec![],
        );
        let song = super::Song::new(tempdir.path(), &song_config)?;
        assert_eq!(song.tracks.len(), 2);
        Ok(())
    }

    #[test]
    fn initialize_songs_skips_non_directory_entries() -> Result<(), Box<dyn Error>> {
        let tempdir = tempfile::tempdir()?;
        // Create a regular file at the top level (not a directory)
        fs::write(tempdir.path().join("readme.txt"), "hello")?;
        let count = initialize_songs(tempdir.path())?;
        assert_eq!(count, 0);
        Ok(())
    }

    #[test]
    fn song_initialize_skips_non_file_entries() -> Result<(), Box<dyn Error>> {
        let tempdir = tempfile::tempdir()?;
        let song_dir = tempdir.path().join("my_song");
        fs::create_dir(&song_dir)?;
        // Create a subdirectory inside the song directory (skipped with warning)
        fs::create_dir(song_dir.join("subdir"))?;
        // Also add a real WAV so initialize doesn't fail
        crate::testutil::write_wav(
            song_dir.join("track.wav"),
            vec![vec![1_i32, 2, 3, 4, 5]],
            44100,
        )?;
        let song = super::Song::initialize(&song_dir)?;
        // Should have the track but not the subdir
        assert_eq!(song.tracks.len(), 1);
        Ok(())
    }

    #[test]
    fn create_channel_mapped_sources_mono() -> Result<(), Box<dyn Error>> {
        let tempdir = tempfile::tempdir()?;
        let wav_path = tempdir.path().join("track.wav");
        crate::testutil::write_wav(wav_path, vec![vec![1_i32; 4410]], 44100)?;

        let song_config = crate::config::Song::new(
            "test",
            None,
            None,
            None,
            None,
            None,
            vec![crate::config::Track::new(
                "track".to_string(),
                "track.wav",
                Some(1),
            )],
            std::collections::HashMap::new(),
            vec![],
        );
        let song = super::Song::new(tempdir.path(), &song_config)?;
        let target = crate::audio::TargetFormat::new(44100, crate::audio::SampleFormat::Int, 16)?;
        let context = crate::audio::PlaybackContext::new(target, 1024, None, Default::default());
        let mut mappings = std::collections::HashMap::new();
        mappings.insert("track".to_string(), vec![1_u16, 2]);
        let sources = song.create_channel_mapped_sources_from(
            &context,
            std::time::Duration::ZERO,
            &mappings,
        )?;
        assert_eq!(sources.len(), 1);
        Ok(())
    }

    #[test]
    fn create_channel_mapped_sources_stereo() -> Result<(), Box<dyn Error>> {
        let tempdir = tempfile::tempdir()?;
        crate::testutil::write_wav(
            tempdir.path().join("stereo.wav"),
            vec![vec![1_i32; 4410], vec![2_i32; 4410]],
            44100,
        )?;

        let song_config = crate::config::Song::new(
            "test",
            None,
            None,
            None,
            None,
            None,
            vec![
                crate::config::Track::new("left".to_string(), "stereo.wav", Some(1)),
                crate::config::Track::new("right".to_string(), "stereo.wav", Some(2)),
            ],
            std::collections::HashMap::new(),
            vec![],
        );
        let song = super::Song::new(tempdir.path(), &song_config)?;
        let target = crate::audio::TargetFormat::new(44100, crate::audio::SampleFormat::Int, 16)?;
        let context = crate::audio::PlaybackContext::new(target, 1024, None, Default::default());
        let mut mappings = std::collections::HashMap::new();
        mappings.insert("left".to_string(), vec![1_u16]);
        mappings.insert("right".to_string(), vec![2_u16]);
        let sources = song.create_channel_mapped_sources_from(
            &context,
            std::time::Duration::ZERO,
            &mappings,
        )?;
        assert_eq!(sources.len(), 1);
        Ok(())
    }

    #[test]
    fn create_channel_mapped_sources_with_start_time() -> Result<(), Box<dyn Error>> {
        let tempdir = tempfile::tempdir()?;
        crate::testutil::write_wav(
            tempdir.path().join("track.wav"),
            vec![vec![1_i32; 44100]],
            44100,
        )?;

        let song_config = crate::config::Song::new(
            "test",
            None,
            None,
            None,
            None,
            None,
            vec![crate::config::Track::new(
                "track".to_string(),
                "track.wav",
                Some(1),
            )],
            std::collections::HashMap::new(),
            vec![],
        );
        let song = super::Song::new(tempdir.path(), &song_config)?;
        let target = crate::audio::TargetFormat::new(44100, crate::audio::SampleFormat::Int, 16)?;
        let context = crate::audio::PlaybackContext::new(target, 1024, None, Default::default());
        let mut mappings = std::collections::HashMap::new();
        mappings.insert("track".to_string(), vec![1_u16]);
        let sources = song.create_channel_mapped_sources_from(
            &context,
            std::time::Duration::from_millis(500),
            &mappings,
        )?;
        assert_eq!(sources.len(), 1);
        Ok(())
    }

    #[test]
    fn create_channel_mapped_sources_unmapped_track() -> Result<(), Box<dyn Error>> {
        let tempdir = tempfile::tempdir()?;
        crate::testutil::write_wav(
            tempdir.path().join("track.wav"),
            vec![vec![1_i32; 4410]],
            44100,
        )?;

        let song_config = crate::config::Song::new(
            "test",
            None,
            None,
            None,
            None,
            None,
            vec![crate::config::Track::new(
                "track".to_string(),
                "track.wav",
                Some(1),
            )],
            std::collections::HashMap::new(),
            vec![],
        );
        let song = super::Song::new(tempdir.path(), &song_config)?;
        let target = crate::audio::TargetFormat::new(44100, crate::audio::SampleFormat::Int, 16)?;
        let context = crate::audio::PlaybackContext::new(target, 1024, None, Default::default());
        let mappings = std::collections::HashMap::new();
        let sources = song.create_channel_mapped_sources_from(
            &context,
            std::time::Duration::ZERO,
            &mappings,
        )?;
        assert_eq!(sources.len(), 1);
        Ok(())
    }

    #[test]
    fn create_channel_mapped_sources_multiple_files() -> Result<(), Box<dyn Error>> {
        let tempdir = tempfile::tempdir()?;
        crate::testutil::write_wav(
            tempdir.path().join("file_a.wav"),
            vec![vec![1_i32; 4410]],
            44100,
        )?;
        crate::testutil::write_wav(
            tempdir.path().join("file_b.wav"),
            vec![vec![2_i32; 4410]],
            44100,
        )?;

        let song_config = crate::config::Song::new(
            "multi",
            None,
            None,
            None,
            None,
            None,
            vec![
                crate::config::Track::new("a".to_string(), "file_a.wav", Some(1)),
                crate::config::Track::new("b".to_string(), "file_b.wav", Some(1)),
            ],
            std::collections::HashMap::new(),
            vec![],
        );
        let song = super::Song::new(tempdir.path(), &song_config)?;
        let target = crate::audio::TargetFormat::new(44100, crate::audio::SampleFormat::Int, 16)?;
        let context = crate::audio::PlaybackContext::new(target, 1024, None, Default::default());
        let mut mappings = std::collections::HashMap::new();
        mappings.insert("a".to_string(), vec![1_u16]);
        mappings.insert("b".to_string(), vec![2_u16]);
        let sources = song.create_channel_mapped_sources_from(
            &context,
            std::time::Duration::ZERO,
            &mappings,
        )?;
        assert_eq!(sources.len(), 2);
        Ok(())
    }

    #[test]
    fn to_proto_conversion() -> Result<(), Box<dyn Error>> {
        let tempdir = tempfile::tempdir()?;
        crate::testutil::write_wav(
            tempdir.path().join("track.wav"),
            vec![vec![1_i32; 4410]],
            44100,
        )?;
        let song_config = crate::config::Song::new(
            "proto test",
            None,
            None,
            None,
            None,
            None,
            vec![crate::config::Track::new(
                "track".to_string(),
                "track.wav",
                Some(1),
            )],
            std::collections::HashMap::new(),
            vec![],
        );
        let song = super::Song::new(tempdir.path(), &song_config)?;
        let proto = song.to_proto()?;
        assert_eq!(proto.name, "proto test");
        assert_eq!(proto.tracks.len(), 1);
        assert_eq!(proto.tracks[0], "track");
        Ok(())
    }

    #[test]
    fn get_all_songs_skips_non_song_files() -> Result<(), Box<dyn Error>> {
        let tempdir = tempfile::tempdir()?;
        // Create a regular file that's not a song config
        fs::write(tempdir.path().join("readme.txt"), "not a song")?;
        // Create a WAV file at the top level (should be skipped — no .yaml)
        crate::testutil::write_wav(
            tempdir.path().join("random.wav"),
            vec![vec![1_i32; 100]],
            44100,
        )?;
        let songs = get_all_songs(tempdir.path())?;
        assert!(songs.is_empty());
        Ok(())
    }

    #[test]
    fn initialize_with_light_file() -> Result<(), Box<dyn Error>> {
        let tempdir = tempfile::tempdir()?;
        let song_dir = tempdir.path().join("light_song");
        fs::create_dir(&song_dir)?;

        // Create a WAV file so the song has at least one track.
        crate::testutil::write_wav(
            song_dir.join("track.wav"),
            vec![vec![1_i32, 2, 3, 4, 5]],
            44100,
        )?;

        // Create a valid .light DSL file.
        let dsl = r#"show "test" {
    @00:00.000
    front: static color: "blue"
}"#;
        fs::write(song_dir.join("lighting.light"), dsl)?;

        let song = super::Song::initialize(&song_dir)?;
        assert_eq!(song.tracks().len(), 1, "Expected one audio track");
        assert!(
            !song.dsl_lighting_shows().is_empty(),
            "Expected at least one DSL lighting show"
        );
        let dsl_show = &song.dsl_lighting_shows()[0];
        assert!(
            !dsl_show.shows().is_empty(),
            "Expected parsed shows to be non-empty"
        );
        assert!(
            dsl_show.shows().contains_key("test"),
            "Expected a show named 'test'"
        );
        Ok(())
    }

    #[test]
    fn initialize_with_invalid_light_file() -> Result<(), Box<dyn Error>> {
        let tempdir = tempfile::tempdir()?;
        let song_dir = tempdir.path().join("bad_light_song");
        fs::create_dir(&song_dir)?;

        crate::testutil::write_wav(
            song_dir.join("track.wav"),
            vec![vec![1_i32, 2, 3, 4, 5]],
            44100,
        )?;

        // Write invalid DSL content.
        fs::write(song_dir.join("bad.light"), "show {")?;

        let result = super::Song::initialize(&song_dir);
        assert!(result.is_err(), "Expected an error for invalid .light file");
        let err = result.err().unwrap().to_string();
        assert!(
            err.contains("Failed to parse DSL lighting show"),
            "Error: {err}"
        );
        Ok(())
    }

    #[test]
    fn light_show_dmx_file_path() -> Result<(), Box<dyn Error>> {
        let tempdir = tempfile::tempdir()?;
        let dmx_path = tempdir.path().join("dmx_show.mid");
        fs::write(&dmx_path, "")?;

        let light_show = super::LightShow {
            universe_name: "test_universe".to_string(),
            dmx_file: dmx_path.clone(),
            midi_channels: vec![],
        };
        assert_eq!(light_show.dmx_file_path(), dmx_path.as_path());
        Ok(())
    }

    #[test]
    fn is_supported_audio_extension_accepted() {
        for ext in &["wav", "mp3", "flac", "ogg", "aac", "m4a", "aiff"] {
            assert!(
                super::is_supported_audio_extension(ext),
                "Expected '{ext}' to be a supported audio extension"
            );
        }
    }

    #[test]
    fn is_supported_audio_extension_rejected() {
        for ext in &["txt", "yaml", "mid", "light", ""] {
            assert!(
                !super::is_supported_audio_extension(ext),
                "Expected '{ext}' to NOT be a supported audio extension"
            );
        }
    }

    #[test]
    fn initialize_with_mixed_files() -> Result<(), Box<dyn Error>> {
        let tempdir = tempfile::tempdir()?;
        let song_dir = tempdir.path().join("mixed_song");
        fs::create_dir(&song_dir)?;

        // Audio track
        crate::testutil::write_wav(
            song_dir.join("track.wav"),
            vec![vec![1_i32, 2, 3, 4, 5]],
            44100,
        )?;

        // MIDI playback file (regular .mid)
        let midi_bytes: Vec<u8> = vec![
            0x4D, 0x54, 0x68, 0x64, 0x00, 0x00, 0x00, 0x06, 0x00, 0x00, 0x00, 0x01, 0x00, 0x60,
            0x4D, 0x54, 0x72, 0x6B, 0x00, 0x00, 0x00, 0x04, 0x00, 0xFF, 0x2F, 0x00,
        ];
        fs::write(song_dir.join("song.mid"), &midi_bytes)?;

        // MIDI DMX light show file (dmx_ prefix)
        fs::write(song_dir.join("dmx_light.mid"), &midi_bytes)?;

        // DSL lighting show file
        let dsl = r#"show "mixed" {
    @00:00.000
    front: static color: "blue"
}"#;
        fs::write(song_dir.join("show.light"), dsl)?;

        let song = super::Song::initialize(&song_dir)?;

        // Should have one audio track from the WAV file.
        assert_eq!(song.tracks().len(), 1, "Expected one audio track");

        // Should have MIDI playback from song.mid.
        assert!(
            song.midi_playback().is_some(),
            "Expected MIDI playback from song.mid"
        );

        // Should have one MIDI DMX light show from dmx_light.mid.
        assert_eq!(
            song.light_shows().len(),
            1,
            "Expected one MIDI DMX light show from dmx_light.mid"
        );

        // Should have one DSL lighting show from show.light.
        assert_eq!(
            song.dsl_lighting_shows().len(),
            1,
            "Expected one DSL lighting show from show.light"
        );
        assert!(
            song.dsl_lighting_shows()[0].shows().contains_key("mixed"),
            "Expected parsed show named 'mixed'"
        );
        Ok(())
    }

    #[test]
    fn test_declared_song_invalid_yaml_produces_failure() -> Result<(), TestError> {
        let temp_dir = tempfile::tempdir()?;
        let song_dir = create_song_dir(temp_dir.path(), "broken-song")?;
        // kind: song is present, so the failure is recorded.
        fs::write(
            song_dir.join("song.yaml"),
            "kind: song\nname: broken\n  bad indent!!",
        )?;

        let songs = get_all_songs(temp_dir.path())?;
        assert_eq!(songs.len(), 0, "Expected no valid songs");
        assert_eq!(songs.failures().len(), 1, "Expected one failure");
        assert_eq!(songs.failures()[0].name(), "broken-song");
        assert!(!songs.failures()[0].error().is_empty());
        Ok(())
    }

    #[test]
    fn test_undeclared_song_invalid_yaml_no_failure() -> Result<(), TestError> {
        let temp_dir = tempfile::tempdir()?;
        let song_dir = create_song_dir(temp_dir.path(), "not-a-song")?;
        // No kind field — silently skipped, no failure recorded.
        fs::write(song_dir.join("song.yaml"), "{{invalid yaml!!")?;

        let songs = get_all_songs(temp_dir.path())?;
        assert_eq!(songs.len(), 0, "Expected no valid songs");
        assert_eq!(
            songs.failures().len(),
            0,
            "Expected no failures for undeclared YAML"
        );
        Ok(())
    }

    #[test]
    fn test_missing_audio_produces_failure() -> Result<(), TestError> {
        let temp_dir = tempfile::tempdir()?;
        let song_dir = create_song_dir(temp_dir.path(), "missing-audio")?;
        // Song::new failure is always recorded regardless of kind.
        fs::write(
            song_dir.join("song.yaml"),
            "name: missing-audio\ntracks:\n  - name: track1\n    file: nonexistent.wav\n",
        )?;

        let songs = get_all_songs(temp_dir.path())?;
        assert_eq!(songs.len(), 0, "Expected no valid songs");
        assert_eq!(songs.failures().len(), 1, "Expected one failure");
        assert_eq!(songs.failures()[0].name(), "missing-audio");
        Ok(())
    }

    #[test]
    fn test_valid_and_invalid_songs_mixed() -> Result<(), TestError> {
        let temp_dir = tempfile::tempdir()?;

        // Create a valid song.
        create_mono_song(temp_dir.path())?;
        initialize_songs(temp_dir.path())?;

        // Create an invalid song with kind: song.
        let broken_dir = create_song_dir(temp_dir.path(), "broken-song")?;
        fs::write(
            broken_dir.join("song.yaml"),
            "kind: song\nname: broken\n  bad indent!!",
        )?;

        let songs = get_all_songs(temp_dir.path())?;
        assert_eq!(songs.len(), 1, "Expected one valid song");
        assert_eq!(songs.failures().len(), 1, "Expected one failure");
        assert_eq!(songs.failures()[0].name(), "broken-song");
        Ok(())
    }

    #[test]
    fn test_playlist_yaml_not_treated_as_song() -> Result<(), TestError> {
        let temp_dir = tempfile::tempdir()?;
        let sub_dir = create_song_dir(temp_dir.path(), "playlists")?;
        fs::write(
            sub_dir.join("my_playlist.yaml"),
            "kind: playlist\nsongs:\n  - song1\n",
        )?;

        let songs = get_all_songs(temp_dir.path())?;
        assert_eq!(songs.len(), 0);
        assert_eq!(
            songs.failures().len(),
            0,
            "Playlist YAML should not produce a failure"
        );
        Ok(())
    }
}
