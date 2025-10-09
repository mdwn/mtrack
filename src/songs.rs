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
use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;
use std::{cmp, fmt, thread};

use hound::{SampleFormat, WavReader};
use midly::live::LiveEvent;
use midly::{Format, Smf};
use nodi::timers::Ticker;
use nodi::Sheet;
use ringbuf::traits::{Consumer, Observer, Producer, Split};
use ringbuf::{HeapCons, HeapProd, HeapRb};
use tracing::{debug, error, info, warn};

use crate::audio::{
    sample_source::{create_wav_sample_source, AnySampleSource, SampleSource},
    TargetFormat,
};
use crate::config;
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
    /// The number of channels required to play this song.
    num_channels: u16,
    /// The sample rate of this song.
    sample_rate: u32,
    /// The sample format.
    sample_format: hound::SampleFormat,
    /// The total duration of the song.
    duration: Duration,
    /// The individual audio tracks.
    tracks: Vec<Track>,
}

/// A simple sample for songs. Boils down to i32 or f32, which we can be reasonably assured that
/// hound is able to read.
pub trait Sample:
    cpal::SizedSample + hound::Sample + Default + Send + Sync + std::ops::AddAssign + 'static
{
    /// Converts from f32 to this sample type
    fn from_f32(value: f32) -> Self;
}
impl Sample for i32 {
    fn from_f32(value: f32) -> Self {
        (value * 2147483647.0) as i32
    }
}
impl Sample for f32 {
    fn from_f32(value: f32) -> Self {
        value
    }
}

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

    /// Gets the song tracks.
    pub fn tracks(&self) -> Vec<Track> {
        self.tracks.clone()
    }

    /// Checks if this song requires transcoding for the given target format
    pub fn needs_transcoding(&self, target_format: &TargetFormat) -> bool {
        // Check if any track has different sample rate, format, or bit depth
        self.tracks.iter().any(|track| {
            // Read the actual bit depth from the WAV file
            let actual_bits_per_sample = match std::fs::File::open(&track.file) {
                Ok(file) => {
                    match hound::WavReader::new(file) {
                        Ok(reader) => reader.spec().bits_per_sample,
                        Err(_) => {
                            // Fallback to defaults if we can't read the file
                            match track.sample_format {
                                hound::SampleFormat::Int => 16,
                                hound::SampleFormat::Float => 32,
                            }
                        }
                    }
                }
                Err(_) => {
                    // Fallback to defaults if we can't open the file
                    match track.sample_format {
                        hound::SampleFormat::Int => 16,
                        hound::SampleFormat::Float => 32,
                    }
                }
            };

            let source_format = TargetFormat::new(
                track.sample_rate,
                track.sample_format,
                actual_bits_per_sample,
            );

            if let Ok(source_format) = source_format {
                // Simple check: if sample rate or format differs, transcoding is needed
                source_format.sample_rate != target_format.sample_rate
                    || source_format.sample_format != target_format.sample_format
                    || source_format.bits_per_sample != target_format.bits_per_sample
            } else {
                true // If we can't create source format, assume transcoding is needed
            }
        })
    }

    /// Returns the duration string in minutes and seconds.
    pub fn duration_string(&self) -> String {
        let secs = self.duration.as_secs();
        format!("{}:{:02}", secs / 60, secs % 60)
    }

    /// Returns a rodio source for the song.
    pub fn source<S>(
        &self,
        track_mappings: &HashMap<String, Vec<u16>>,
        target_format: TargetFormat,
    ) -> Result<SongSource<S>, Box<dyn Error>>
    where
        S: Sample,
    {
        SongSource::<S>::new(self, track_mappings, target_format)
    }

    /// Returns a proto version of the song.
    pub fn to_proto(&self) -> Result<player::v1::Song, std::io::Error> {
        let duration = match prost_types::Duration::try_from(self.duration) {
            Ok(duration) => duration,
            Err(e) => {
                return Err(std::io::Error::new::<String>(
                    std::io::ErrorKind::Other,
                    e.to_string(),
                ))
            }
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

        let reader = WavReader::open(&track_file)?;
        let spec = reader.spec();
        let sample_rate = spec.sample_rate;
        let duration = Duration::from_secs(u64::from(reader.duration()) / u64::from(sample_rate));
        if spec.channels > 1 && file_channel.is_none() {
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
            sample_format: spec.sample_format,
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
        let reader = WavReader::open(track_path)?;
        let spec = reader.spec();
        let sample_rate = spec.sample_rate;
        let sample_format = spec.sample_format;
        let duration = Duration::from_secs(u64::from(reader.duration()) / u64::from(sample_rate));
        let tracks = match spec.channels {
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
            _ => (0..spec.channels)
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

/// A source for all of the combined tracks in the song.
pub struct SongSource<S>
where
    S: Sample,
{
    finished: Arc<AtomicBool>,
    cons: HeapCons<S>,
    join_handle: Option<JoinHandle<()>>,
    channels: u16,
    frame_pos: Arc<AtomicU16>,
}

/// The sample source and the file channel mapping.
struct SampleSourceAndMapping {
    sample_source: AnySampleSource,
    file_channel_to_output_channels: HashMap<u16, Vec<usize>>,
    num_channels: u16,
}

impl<S> SongSource<S>
where
    S: Sample,
{
    fn new(
        song: &Song,
        track_mapping: &HashMap<String, Vec<u16>>,
        target_format: TargetFormat,
    ) -> Result<SongSource<S>, Box<dyn Error>> {
        let mut files_to_tracks = HashMap::<PathBuf, Vec<&Track>>::new();

        song.tracks.iter().for_each(|track| {
            let file_path = &track.file;
            if !files_to_tracks.contains_key(file_path) {
                files_to_tracks.insert(file_path.clone(), Vec::new());
            }

            files_to_tracks
                .get_mut(file_path)
                .expect("unable to get tracks")
                .push(track);
        });

        let num_channels = *track_mapping
            .iter()
            .flat_map(|entry| entry.1)
            .max()
            .ok_or("no max channel found")?;

        // Get a 30 second buffer here. Raspberry Pis have multiple gigs of memory and 30 seconds at 44.1Khz for 22 channels
        // is less than 30 MB. At 192KHz, it's ~188 MB. These seem reasonable to me.
        let buf = HeapRb::new(usize::from(num_channels) * usize::try_from(song.sample_rate)? * 30);
        let (prod, cons) = buf.split();
        let mut sample_sources_and_mappings = Vec::<SampleSourceAndMapping>::new();

        for (file_path, tracks) in files_to_tracks {
            // Get the WAV file spec to determine the actual source format
            let file = std::fs::File::open(&file_path)?;
            let wav_reader = hound::WavReader::new(file)?;
            let spec = wav_reader.spec();
            let wav_channels = spec.channels;

            // Create source format from the actual WAV file spec (for transcoding detection)
            let _source_format =
                TargetFormat::new(spec.sample_rate, spec.sample_format, spec.bits_per_sample)?;

            let sample_source = create_wav_sample_source(file_path, target_format.clone())?;

            let mut file_channel_to_output_channels: HashMap<u16, Vec<usize>> = HashMap::new();
            tracks.into_iter().for_each(|track| {
                let file_channel = track.file_channel - 1;
                if let Some(channel_mappings) = track_mapping.get(&track.name.to_string()) {
                    channel_mappings.iter().for_each(|channel_mapping| {
                        let output_channel = channel_mapping - 1;
                        file_channel_to_output_channels
                            .entry(file_channel)
                            .or_default()
                            .push(output_channel.into());
                    })
                }
            });

            sample_sources_and_mappings.push(SampleSourceAndMapping {
                sample_source,
                file_channel_to_output_channels,
                num_channels: wav_channels,
            })
        }

        let finished = Arc::new(AtomicBool::new(false));
        let join_handle = {
            SongSource::reader_thread(
                usize::from(num_channels),
                sample_sources_and_mappings,
                prod,
                finished.clone(),
            )
        };

        let source = SongSource {
            finished,
            cons,
            join_handle: Some(join_handle),
            channels: num_channels,
            frame_pos: Arc::new(AtomicU16::new(0)),
        };

        source.wait_for_buffer_or_stop();

        Ok(source)
    }

    fn reader_thread(
        num_channels: usize,
        mut sample_sources_and_mappings: Vec<SampleSourceAndMapping>,
        mut prod: HeapProd<S>,
        finished: Arc<AtomicBool>,
    ) -> JoinHandle<()> {
        thread::spawn(move || {
            // The number of frames to read at a time from the source. We'll make it one quarter of the total capacity.
            let num_frames = Into::<usize>::into(prod.capacity()) / num_channels / 2;
            let num_sources = sample_sources_and_mappings.len();
            let mut channels: Vec<u16> = Vec::with_capacity(num_sources);
            let mut file_mappings: Vec<HashMap<u16, Vec<usize>>> = Vec::with_capacity(num_sources);
            let mut frames = vec![S::default(); num_channels * num_frames];

            for sample_source_and_mapping in sample_sources_and_mappings.iter() {
                channels.push(sample_source_and_mapping.num_channels);
                file_mappings.push(
                    sample_source_and_mapping
                        .file_channel_to_output_channels
                        .clone(),
                );
            }

            loop {
                // Wait until the buffer is half empty before proceeding.
                while prod.occupied_len() > Into::<usize>::into(prod.capacity()) / 2 {
                    thread::sleep(Duration::from_millis(200))
                }

                // Load the entire buffer until it's full.
                let num_frames_to_take = prod.vacant_len() / num_channels;
                for i in 0..num_frames_to_take {
                    // If finished gets set, we'll return immediately.
                    if finished.load(Ordering::Relaxed) {
                        return;
                    }

                    let mut all_files_finished = true;
                    let current_frame = i % num_frames;

                    // Read one sample from each source for this frame
                    for j in 0..num_sources {
                        let file_channel_to_output_channels = &file_mappings[j];
                        let sample_source = &mut sample_sources_and_mappings[j].sample_source;

                        // Read one sample from each channel of this source
                        for file_channel in 0..channels[j] {
                            let result = sample_source.next_sample();
                            match result {
                                Ok(Some(sample)) => {
                                    all_files_finished = false;
                                    if let Some(targets) =
                                        file_channel_to_output_channels.get(&file_channel)
                                    {
                                        for target in targets {
                                            // Convert f32 sample to the target sample type
                                            let converted_sample = S::from_f32(sample);
                                            frames[*target + current_frame * num_channels] +=
                                                converted_sample;
                                        }
                                    }
                                }
                                Ok(None) => {
                                    // Sample source is finished
                                }
                                Err(e) => {
                                    error!(
                                        err = e.to_string(),
                                        "Error reading sample from SampleSource"
                                    );
                                }
                            }
                        }
                    }

                    // Push the entirety of the frame to the buffer if we've read all the frames in this
                    // batch or if we're at the end of the loop.
                    if i == num_frames_to_take - 1
                        || current_frame == num_frames - 1
                        || all_files_finished
                    {
                        let _ = prod.push_slice(&frames[0..current_frame * num_channels]);

                        // Reset the frames to default.
                        frames
                            .iter_mut()
                            .take(current_frame * num_channels)
                            .for_each(|sample| *sample = S::default());
                    }

                    if all_files_finished {
                        finished.store(true, Ordering::Relaxed);
                        return;
                    }
                }
            }
        })
    }

    // Waits for the buffer to fill or for the song to stop. Will return false if the song is stopped.
    fn wait_for_buffer_or_stop(&self) -> bool {
        // We'll wait while we're waiting for the buffer to fill or the song to stop.
        // It's not expected that this will take long.
        loop {
            if self.finished.load(Ordering::Relaxed) {
                return false;
            }
            if !self.cons.is_empty() {
                break;
            }
            thread::sleep(Duration::from_millis(50))
        }

        true
    }

    /// Gets the current frame position in the song source.
    pub fn get_frame_position(&self) -> u16 {
        self.frame_pos.load(Ordering::Relaxed)
    }
}

impl<S> Iterator for SongSource<S>
where
    S: Sample,
{
    type Item = S;

    fn next(&mut self) -> Option<Self::Item> {
        self.wait_for_buffer_or_stop();
        let sample = self.cons.try_pop();

        if sample.is_some() {
            self.frame_pos
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |val| {
                    Some((val + 1) % self.channels)
                })
                .expect("got none from frame position update function");
        }

        sample
    }
}

impl<S> Drop for SongSource<S>
where
    S: Sample,
{
    fn drop(&mut self) {
        // Let the thread know that we shouldn't read any more.
        self.finished.store(true, Ordering::Relaxed);

        // Clear out the consumer, which will force the thread to move to the next iteration if it's still running.
        self.cons.clear();

        // Join the thread to make sure that it's stopped properly.
        self.join_handle
            .take()
            .expect("No join handle found")
            .join()
            .expect("Join failed");
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
        collections::HashMap,
        error::Error,
        fs, io,
        path::{Path, PathBuf},
    };

    use thiserror::Error;

    use crate::{audio::TargetFormat, config, songs::initialize_songs, testutil::write_wav};

    use super::{get_all_songs, Sample, SongSource};

    #[test]
    fn song_source_frame_read() -> Result<(), Box<dyn Error>> {
        let mut source = test_source()?;

        // First frame should have first samples, second frame second, and so on.
        let frame = get_frame(4, &mut source)?.expect("Expected a frame");
        assert_eq!(vec![1_i32, 0_i32, 0_i32, 2_i32], frame);

        let frame = get_frame(4, &mut source)?.expect("Expected a frame");
        assert_eq!(vec![2_i32, 0_i32, 0_i32, 3_i32], frame);

        // Track 3 is added to track 2 samples.
        let frame = get_frame(4, &mut source)?.expect("Expected a frame");
        assert_eq!(vec![3_i32, 0_i32, 0_i32, 5_i32], frame);

        // Track 2 has ended, but track 3 isn't done yet.
        let frame = get_frame(4, &mut source)?.expect("Expected a frame");
        assert_eq!(vec![4_i32, 0_i32, 0_i32, 2_i32], frame);

        // Track 3 has ended.
        let frame = get_frame(4, &mut source)?.expect("Expected a frame");
        assert_eq!(vec![5_i32, 0_i32, 0_i32, 0_i32], frame);

        // All tracks have ended, we expect None.
        let frame = get_frame(4, &mut source)?;
        assert!(frame.is_none());

        Ok(())
    }

    fn test_source() -> Result<SongSource<i32>, Box<dyn Error>> {
        let tempdir = tempfile::tempdir()?.into_path();
        let tempwav1 = "tempwav1.wav";
        let tempwav2 = "tempwav2.wav";
        let tempwav3 = "tempwav3.wav";

        write_wav(
            tempdir.join(tempwav1),
            vec![vec![1_i32, 2_i32, 3_i32, 4_i32, 5_i32]],
            44100,
        )?;
        write_wav(
            tempdir.join(tempwav2),
            vec![vec![2_i32, 3_i32, 4_i32]],
            44100,
        )?;
        write_wav(
            tempdir.join(tempwav3),
            vec![vec![0_i32, 0_i32, 1_i32, 2_i32]],
            44100,
        )?;

        let track1 = config::Track::new("test 1".into(), tempwav1, Some(1));
        let track2 = config::Track::new("test 2".into(), tempwav2, Some(1));
        let track3 = config::Track::new("test 3".into(), tempwav3, Some(1));

        let song = super::Song::new(
            &tempdir,
            &config::Song::new(
                "song name",
                None,
                None,
                None,
                None,
                vec![track1, track2, track3],
            ),
        )?;
        let mut mapping: HashMap<String, Vec<u16>> = HashMap::new();
        mapping.insert("test 1".into(), vec![1]);
        mapping.insert("test 2".into(), vec![4]);
        mapping.insert("test 3".into(), vec![4]);

        song.source(&mapping, TargetFormat::default())
    }

    fn get_frame<S: Sample>(
        frame_size: u16,
        source: &mut SongSource<S>,
    ) -> Result<Option<Vec<S>>, Box<dyn Error>> {
        let mut frame = Vec::new();

        for i in 0..frame_size {
            match source.next() {
                Some(sample) => frame.push(sample),
                None => {
                    if i == 0 {
                        return Ok(None);
                    } else {
                        return Err("Stopped mid frame".into());
                    }
                }
            };
        }

        Ok(Some(frame))
    }

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
        let song_path = create_song_dir(&path, "1 Song with mono track")?;

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
        assert!(
            fs::exists(track.file.to_path_buf()).unwrap(),
            "Track file not found"
        );
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
        use crate::testutil::write_wav;

        let tempdir = tempfile::tempdir()?.into_path();

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
    fn test_song_source_multiformat_integration() -> Result<(), Box<dyn Error>> {
        use crate::testutil::write_wav;
        use std::collections::HashMap;

        // Create a temporary directory for test files
        let tempdir = tempfile::tempdir()?.into_path();

        // Test parameters
        let duration_seconds = 0.5; // All sources will be 0.5 seconds
        let target_sample_rate = 48000;
        let target_format =
            TargetFormat::new(target_sample_rate, hound::SampleFormat::Float, 32).unwrap();

        // Create multiple input sources with different formats and sample rates
        let test_sources = vec![
            // Source 1: 44.1kHz, 32-bit int, mono
            {
                let sample_rate = 44100;
                let sample_count = (sample_rate as f32 * duration_seconds) as usize;
                let samples: Vec<i32> = (0..sample_count)
                    .map(|i| {
                        ((i as f32 * 440.0 * 2.0 * std::f32::consts::PI / sample_rate as f32).sin()
                            * (1 << 23) as f32) as i32
                    })
                    .collect();
                let wav_path = tempdir.join("source_44k_32bit_mono.wav");
                write_wav(wav_path.clone(), vec![samples], sample_rate)?;
                ("source_44k_32bit_mono.wav", 1, wav_path)
            },
            // Source 2: 48kHz, 32-bit int, stereo
            {
                let sample_rate = 48000;
                let sample_count = (sample_rate as f32 * duration_seconds) as usize;
                let left_samples: Vec<i32> = (0..sample_count)
                    .map(|i| {
                        ((i as f32 * 880.0 * 2.0 * std::f32::consts::PI / sample_rate as f32).sin()
                            * (1 << 23) as f32) as i32
                    })
                    .collect();
                let right_samples: Vec<i32> = (0..sample_count)
                    .map(|i| {
                        ((i as f32 * 1320.0 * 2.0 * std::f32::consts::PI / sample_rate as f32)
                            .sin()
                            * (1 << 23) as f32) as i32
                    })
                    .collect();
                let wav_path = tempdir.join("source_48k_32bit_int_stereo.wav");
                write_wav(
                    wav_path.clone(),
                    vec![left_samples, right_samples],
                    sample_rate,
                )?;
                ("source_48k_32bit_int_stereo.wav", 2, wav_path)
            },
            // Source 3: 96kHz, 32-bit int, mono
            {
                let sample_rate = 96000;
                let sample_count = (sample_rate as f32 * duration_seconds) as usize;
                let samples: Vec<i32> = (0..sample_count)
                    .map(|i| {
                        ((i as f32 * 220.0 * 2.0 * std::f32::consts::PI / sample_rate as f32).sin()
                            * (1 << 23) as f32) as i32
                    })
                    .collect();
                let wav_path = tempdir.join("source_96k_32bit_mono.wav");
                write_wav(wav_path.clone(), vec![samples], sample_rate)?;
                ("source_96k_32bit_mono.wav", 1, wav_path)
            },
            // Source 4: 22.05kHz, 32-bit int, mono
            {
                let sample_rate = 22050;
                let sample_count = (sample_rate as f32 * duration_seconds) as usize;
                let samples: Vec<i32> = (0..sample_count)
                    .map(|i| {
                        ((i as f32 * 660.0 * 2.0 * std::f32::consts::PI / sample_rate as f32).sin()
                            * (1 << 23) as f32) as i32
                    })
                    .collect();
                let wav_path = tempdir.join("source_22k_32bit_int_mono.wav");
                write_wav(wav_path.clone(), vec![samples], sample_rate)?;
                ("source_22k_32bit_int_mono.wav", 1, wav_path)
            },
        ];

        // Create tracks for each source
        let tracks: Vec<config::Track> = test_sources
            .iter()
            .enumerate()
            .map(|(i, (filename, channels, _))| {
                config::Track::new(format!("track_{}", i + 1), filename, Some(*channels))
            })
            .collect();

        // Create the song
        let song = super::Song::new(
            &tempdir,
            &config::Song::new("multiformat_test_song", None, None, None, None, tracks),
        )?;

        // Create channel mapping (each track to a different output channel)
        let mut channel_mapping: HashMap<String, Vec<u16>> = HashMap::new();
        for (i, _) in test_sources.iter().enumerate() {
            channel_mapping.insert(format!("track_{}", i + 1), vec![(i + 1) as u16]);
        }

        // Create the song source
        let mut song_source: SongSource<f32> = song.source(&channel_mapping, target_format)?;

        // Collect all samples from the song source
        let mut all_samples = Vec::new();

        while let Some(sample) = song_source.next() {
            all_samples.push(sample);
        }

        // Calculate expected duration and sample count
        // For a multitrack song source, the duration should be the longest track
        // Each track is 0.5 seconds, so the total should be 0.5 seconds at 48kHz
        let expected_duration_samples = (target_sample_rate as f32 * duration_seconds) as usize;

        // The song source should output samples for the duration of the longest track
        // Since all tracks are 0.5 seconds, the output should be 0.5 seconds worth of samples
        // But we need to account for the fact that the song source interleaves multiple channels
        let num_tracks = test_sources.len();
        let expected_total_samples = expected_duration_samples * num_tracks;

        // Verify the output - we expect samples for the duration of the longest track
        // multiplied by the number of tracks (since they're interleaved)
        // Allow for streaming variations (1% tolerance)
        let tolerance = (expected_total_samples as f32 * 0.01) as usize;
        assert!(
            (all_samples.len() as i32 - expected_total_samples as i32).abs() <= tolerance as i32,
            "Output sample count mismatch: got {}, expected {} (duration: {}s, tracks: {}, tolerance: {})",
            all_samples.len(),
            expected_total_samples,
            duration_seconds,
            num_tracks,
            tolerance
        );

        // Verify duration is correct (within 1% tolerance)
        // The duration should be calculated as total_samples / (sample_rate * num_channels)
        // because the samples are interleaved across multiple channels
        let actual_duration =
            all_samples.len() as f32 / (target_sample_rate as f32 * num_tracks as f32);
        let expected_duration = duration_seconds;
        let duration_ratio = actual_duration / expected_duration;

        assert!(
            duration_ratio > 0.99 && duration_ratio < 1.01,
            "Duration mismatch: got {:.3}s, expected {:.3}s (ratio: {:.4})",
            actual_duration,
            expected_duration,
            duration_ratio
        );

        // Verify that we have samples from all channels
        let num_channels = test_sources.len();
        let samples_per_channel = all_samples.len() / num_channels;

        assert!(
            samples_per_channel > 0,
            "No samples per channel: {} samples / {} channels",
            all_samples.len(),
            num_channels
        );

        // Verify that the samples are not all zeros (basic quality check)
        let non_zero_samples = all_samples.iter().filter(|&&s| s != 0.0).count();
        let non_zero_ratio = non_zero_samples as f32 / all_samples.len() as f32;

        assert!(
            non_zero_ratio > 0.1, // At least 10% non-zero samples
            "Too many zero samples: {:.1}% non-zero (expected > 10%)",
            non_zero_ratio * 100.0
        );

        println!(
            "Integration test passed: {} samples, {:.3}s duration, {:.1}% non-zero samples",
            all_samples.len(),
            actual_duration,
            non_zero_ratio * 100.0
        );

        Ok(())
    }

    #[test]
    fn test_transcoding_detection() {
        use crate::audio::TargetFormat;
        use hound::SampleFormat;

        // Create a test song with 44.1kHz sample rate and a track
        let mut song = super::Song::default();
        song.sample_rate = 44100;
        song.sample_format = SampleFormat::Int;
        song.tracks = vec![super::Track {
            name: "test".to_string(),
            file: std::path::PathBuf::new(),
            file_channel: 1,
            sample_rate: 44100,
            sample_format: SampleFormat::Int,
            duration: std::time::Duration::from_secs(1),
        }];

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
        use crate::audio::TargetFormat;
        use hound::SampleFormat;

        // Test that identical formats don't trigger transcoding
        let target_format = TargetFormat::new(44100, SampleFormat::Int, 16).unwrap();

        // Create a song with identical format
        let mut song = super::Song::default();
        song.sample_rate = 44100;
        song.sample_format = SampleFormat::Int;
        song.tracks = vec![super::Track {
            name: "test".to_string(),
            file: std::path::PathBuf::new(),
            file_channel: 1,
            sample_rate: 44100,
            sample_format: SampleFormat::Int,
            duration: std::time::Duration::from_secs(1),
        }];

        // Should not need transcoding for identical formats
        assert!(!song.needs_transcoding(&target_format));
    }
}
