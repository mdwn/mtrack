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
use std::collections::HashMap;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use std::{cmp, fmt};

use crate::audio::sample_source::create_sample_source_from_file;
use crate::audio::SampleFormat;
use midly::live::LiveEvent;
use midly::{Format, Smf};
use nodi::timers::Ticker;
use nodi::Sheet;
// Removed unused ringbuf imports after migration to Crossbeam channels

use tracing::{debug, info, warn};

use crate::audio::sample_source::SampleSource;
use crate::audio::TargetFormat;
use crate::config::{self, load_dsl_lighting_files, LightingConfiguration};
use crate::proto::player;

const AUDIO_EXTENSIONS: &[&str] = &["wav", "mid"];

/// A song with associated tracks for multitrack playback. Can contain:
/// - An optional MIDI event, which will be played when the song is selected in a playlist.
/// - An optional MIDI file, which will be played along with the audio tracks.
pub struct Song {
    /// The name of the song.
    name: String,
    /// The MIDI event to play when the song is selected in a playlist.
    midi_event: Option<LiveEvent<'static>>,
    /// The MIDI playback configuration.
    midi_playback: Option<MidiPlayback>,
    /// The light show configurations
    light_shows: Vec<LightShow>,
    /// The lighting configuration
    #[allow(dead_code)]
    lighting: Option<LightingConfiguration>,
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
}

/// A simple sample for songs. Boils down to i32 or f32, which we can be reasonably assured that
/// hound is able to read.
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

        // Load lighting configuration if present
        let lighting = match config.lighting() {
            Some(lighting_shows) => {
                let dsl_file_paths: Vec<String> = lighting_shows
                    .iter()
                    .map(|show| show.file().to_string())
                    .collect();

                match load_dsl_lighting_files(start_path, &dsl_file_paths) {
                    Ok(lighting_config) => Some(lighting_config),
                    Err(e) => {
                        warn!("Failed to load DSL lighting files: {}", e);
                        None
                    }
                }
            }
            None => None,
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
            midi_event: config.midi_event()?,
            midi_playback,
            light_shows,
            lighting,
            num_channels,
            sample_rate,
            sample_format: sample_format.unwrap_or(SampleFormat::Int),
            duration: max_duration,
            tracks,
        })
    }

    /// Create a song from a directory without a configuration file
    fn initialize(song_directory: &PathBuf) -> Result<Self, Box<dyn Error>> {
        let song_files = fs::read_dir(song_directory)?;
        let name = song_directory
            .file_name()
            .and_then(|file_name| file_name.to_str())
            .unwrap_or("unreadable directory name")
            .to_string();

        let mut light_shows = vec![];
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
                "wav" => {
                    let mut new_tracks = Track::load_tracks(&path)?;
                    tracks.append(&mut new_tracks);
                }
                unknown_extension => {
                    info!("Unknown extension: {unknown_extension}. Ignoring file.");
                }
            }
        }
        let song = Self {
            name,
            midi_playback,
            light_shows,
            lighting: None, // No lighting in auto-discovered songs
            tracks,
            ..Default::default()
        };
        Ok(song)
    }

    pub fn get_config(&self) -> config::Song {
        let name = self.name();
        let midi_event = None;
        let midi_file = self.midi_playback.as_ref().map(|midi_playback| {
            midi_playback
                .file
                .file_name()
                .and_then(|file_name| file_name.to_str())
                .unwrap_or("unreadable file name")
                .to_string()
        });
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
            None, // Lighting is stored separately and not exported back to config
            tracks,
        )
    }

    /// Gets the name of the song.
    pub fn name(&self) -> &str {
        &self.name
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
    pub fn midi_playback(&self) -> Option<MidiPlayback> {
        self.midi_playback.clone()
    }

    /// Gets the song light shows.
    pub fn light_shows(&self) -> Vec<LightShow> {
        self.light_shows.clone()
    }

    /// Gets the lighting configuration.
    #[allow(dead_code)]
    pub fn lighting(&self) -> Option<LightingConfiguration> {
        self.lighting.clone()
    }

    /// Gets the song tracks.
    pub fn tracks(&self) -> Vec<Track> {
        self.tracks.clone()
    }

    /// Checks if this song requires transcoding for the given target format
    pub fn needs_transcoding(&self, target_format: &TargetFormat) -> bool {
        // Check if any track has different sample rate, format, or bit depth
        self.tracks.iter().any(|track| {
            // Use the generic SampleSource infrastructure to check transcoding needs
            match crate::audio::sample_source::create_sample_source_from_file(&track.file) {
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

    /// Creates ChannelMappedSampleSource instances for each track in the song
    /// This is the new, simpler architecture that replaces SongSource
    pub fn create_channel_mapped_sources(
        &self,
        track_mappings: &HashMap<String, Vec<u16>>,
        target_format: TargetFormat,
        buffer_size: usize,
        buffer_threshold: usize,
    ) -> Result<Vec<Box<dyn crate::audio::sample_source::ChannelMappedSampleSource>>, Box<dyn Error>>
    {
        use crate::audio::sample_source::create_channel_mapped_sample_source;
        use crate::audio::sample_source::create_sample_source_from_file;

        let mut sources = Vec::new();

        // Group tracks by file (like the old SongSource did)
        let mut files_to_tracks = HashMap::<PathBuf, Vec<&Track>>::new();
        for track in &self.tracks {
            files_to_tracks
                .entry(track.file.clone())
                .or_default()
                .push(track);
        }

        for (file_path, tracks) in files_to_tracks {
            // Get the audio file info to determine the actual number of channels
            let wav_source = crate::audio::sample_source::WavSampleSource::from_file(&file_path)?;
            let wav_channels = wav_source.channel_count();

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

            // Create the channel mapped source for this file
            let sample_source = create_sample_source_from_file(&file_path)?;
            let source = create_channel_mapped_sample_source(
                sample_source,
                target_format.clone(),
                channel_mappings,
                buffer_size,
                buffer_threshold,
            )?;

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
            is_transcoded: false, // TODO: This should be calculated based on target format
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
            midi_event: Default::default(),
            midi_playback: Default::default(),
            light_shows: Default::default(),
            lighting: Default::default(),
            num_channels: Default::default(),
            sample_rate: Default::default(),
            sample_format: SampleFormat::Int,
            duration: Default::default(),
            tracks: Default::default(),
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
    let buf: Vec<u8> = fs::read(midi_file)?;
    let smf = Smf::parse(&buf)?;
    let ticker = Ticker::try_from(smf.header.timing)?;

    let midi_sheet = MidiSheet {
        ticker,
        sheet: match smf.header.format {
            Format::SingleTrack | Format::Sequential => Sheet::sequential(&smf.tracks),
            Format::Parallel => Sheet::parallel(&smf.tracks),
        },
    };
    Ok(midi_sheet)
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
            self.dmx_file
                .file_name()
                .and_then(|file_name| file_name.to_str())
                .unwrap_or("unreadable file name")
                .to_string(),
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

        let source = create_sample_source_from_file(&track_file)?;
        let sample_rate = source.sample_rate();
        let duration = source.duration().unwrap_or(Duration::ZERO);
        if source.channel_count() > 1 && file_channel.is_none() {
            return Err(format!(
                "track {} has more than one channel but file_channel is not specified",
                name,
            )
            .into());
        }
        let file_channel = file_channel.unwrap_or(1);

        Ok(Track {
            name: name.to_string(),
            file: track_file.clone(),
            file_channel,
            sample_rate,
            sample_format: source.sample_format(),
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

        assert_eq!(extension, "wav", "Expected file name to end in '.wav'");
        let track_name = stem.to_string();
        let source = create_sample_source_from_file(track_path)?;
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
                    name: format!("{track_name}-{channel}"),
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

    pub fn get_config(&self) -> config::Track {
        config::Track::new(
            self.name().to_string(),
            self.file
                .file_name()
                .and_then(|file_name| file_name.to_str())
                .unwrap_or("unreadable file name"),
            Some(self.file_channel),
        )
    }
}

/// Contains a parsed timer and MIDI sheet for playback.
pub struct MidiSheet {
    pub ticker: Ticker,
    pub sheet: Sheet,
}

/// A registry of songs for use by the multitrack player.
#[derive(Clone)]
pub struct Songs {
    /// A mapping of the songs in the repository.
    songs: HashMap<String, Arc<Song>>,
}

impl Songs {
    /// Creates a new songs registry.
    pub fn new(songs: HashMap<String, Arc<Song>>) -> Songs {
        Songs { songs }
    }

    /// Returns true if the song registry is empty.
    pub fn is_empty(&self) -> bool {
        self.songs.is_empty()
    }

    /// Gets a song from the song registry.
    pub fn get(&self, name: &String) -> Result<Arc<Song>, Box<dyn Error>> {
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
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let path = entry.path();

        if path.is_dir() {
            // Skip .git subdirectories.
            if path.ends_with(".git") {
                continue;
            }

            get_all_songs(path.as_path())?
                .list()
                .iter()
                .for_each(|song| {
                    songs.insert(song.name().to_string(), song.clone());
                });
        }

        let extension = path.extension();
        if extension.is_some_and(|ext| {
            ext.to_str()
                .is_some_and(|ext| !AUDIO_EXTENSIONS.contains(&ext))
        }) {
            if let Ok(song) = config::Song::deserialize(path.as_path()) {
                let song = match path.parent() {
                    Some(parent) => Song::new(&parent.canonicalize()?, &song)?,
                    None => return Err("unable to get parent for path".into()),
                };
                songs.insert(song.name().to_string(), Arc::new(song));
            }
        }
    }

    Ok(Arc::new(Songs::new(songs)))
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
        assert_eq!(track.name, "mono_track", "Unexpected name");
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
        let mut source = crate::audio::sample_source::create_sample_source_from_file(&wav_path)?;
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

        // Measure WavSampleSource performance
        let start = Instant::now();
        let mut wav_source = crate::audio::sample_source::WavSampleSource::from_file(&wav_path)?;
        let mut samples_processed = 0;

        loop {
            match crate::audio::sample_source::SampleSource::next_sample(&mut wav_source) {
                Ok(Some(_)) => samples_processed += 1,
                Ok(None) => break,
                Err(e) => return Err(e.into()),
            }
        }
        let wav_source_time = start.elapsed();

        println!("WavSampleSource processing time: {:?}", wav_source_time);
        println!(
            "WavSampleSource speed: {:.2} MB/s",
            (samples_processed * 4) as f64 / wav_source_time.as_secs_f64() / 1_000_000.0
        );
        println!("Samples processed: {}", samples_processed);

        // Verify we got the expected number of samples
        assert_eq!(samples_read, duration_samples);
        assert_eq!(samples_processed, duration_samples);

        Ok(())
    }

    // Removed test_buffer_fill_performance - not relevant with Crossbeam channels
}
