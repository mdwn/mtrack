// Copyright (C) 2024 Michael Wilson <mike@mdwn.dev>
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
use std::fs::{self, File};
use std::io::BufReader;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;
use std::{cmp, fmt, thread};

use hound::{SampleFormat, WavReader, WavSamples};
use midly::live::LiveEvent;
use midly::{Format, Smf};
use nodi::timers::Ticker;
use nodi::Sheet;
use ringbuf::{HeapConsumer, HeapProducer, HeapRb};
use tracing::error;

/// A song with associated tracks for multitrack playback. Can contain:
/// - An optional MIDI event, which will be played when the song is selected in a playlist.
/// - An optional MIDI file, which will be played along with the audio tracks.
pub struct Song {
    /// The name of the song.
    pub name: String,
    /// The MIDI event to play when the song is selected in a playlist.
    pub midi_event: Option<LiveEvent<'static>>,
    /// The MIDI file to play along with the audio tracks.
    midi_file: Option<PathBuf>,
    /// The number of channels required to play this song.
    pub num_channels: u16,
    /// The sample rate of this song.
    pub sample_rate: u32,
    /// The sample format.
    pub sample_format: hound::SampleFormat,
    /// The bits per sample.
    pub bits_per_sample: u16,
    /// The total duration of the song.
    pub duration: Duration,
    /// The individual audio tracks.
    tracks: Vec<Track>,
}

/// A simple sample for songs. Boils down to i32 or f32, which we can be reasonably assured that
/// hound is able to read.
pub trait Sample:
    cpal::SizedSample + hound::Sample + Default + Send + Sync + std::ops::AddAssign + 'static
{
    /// Scales the sample given the bits per sample. i8, i16, i24 are all read using i32, but
    /// will be significantly reduced volume. By scaling it, the samples won't be anywhere
    /// near as quiet and volume should be decently normalized.
    fn scale(&self, bits_per_sample: u16) -> Self;
}
impl Sample for i32 {
    fn scale(&self, bits_per_sample: u16) -> Self {
        // Do a left shift to increase the magnitude of the sample.
        (self << (32 - bits_per_sample)) as Self
    }
}
impl Sample for f32 {
    fn scale(&self, _: u16) -> Self {
        // Only f32 is supported, so there's no need to scale this.
        *self
    }
}

impl Song {
    // Create a new song.
    pub fn new(
        name: String,
        midi_event: Option<LiveEvent<'static>>,
        midi_file: Option<PathBuf>,
        tracks: Vec<Track>,
    ) -> Result<Song, Box<dyn Error>> {
        // If there's a MIDI event present, let's make a LiveEvent out of it.
        // Get the MIDI file if it's present.
        let midi_file = match midi_file {
            Some(parsed_midi_file_path) => {
                let midi_file = parsed_midi_file_path;
                if !midi_file.is_file() {
                    return Err(format!(
                        "midi file {} does not exist or is not a file",
                        midi_file.to_string_lossy(),
                    )
                    .into());
                }
                Some(midi_file)
            }
            None => None,
        };

        // Calculate the number of channels and sample rate by reading the wav headers of each file.
        let mut num_channels = 0;
        let mut sample_rate = 0;
        let mut max_duration = Duration::ZERO;

        let mut sample_format: Option<SampleFormat> = None;
        let mut bits_per_sample: Option<u16> = None;
        for track in tracks.iter() {
            // Keep track of the maximum output channel in the tracks, as it dictates how many tracks we need
            // to play this song.
            num_channels = cmp::max(num_channels, track.channel);

            // Set the sample rate and formatif it's not already set.
            if sample_rate == 0 {
                sample_rate = track.sample_rate;
            } else if sample_rate != track.sample_rate {
                // All songs need to have the same sample rate.
                return Err(format!(
                    "mismatching sample rates in WAV file: {}, {}",
                    sample_rate, track.sample_rate,
                )
                .into());
            }
            max_duration = cmp::max(track.duration, max_duration);

            match sample_format {
                Some(sample_format) => {
                    if sample_format != track.sample_format {
                        return Err("all tracks must have the same sample format".into());
                    }
                }
                None => sample_format = Some(track.sample_format),
            }

            match bits_per_sample {
                Some(bits_per_sample) => {
                    if bits_per_sample != track.bits_per_sample {
                        return Err("all tracks must have the same bits per sample".into());
                    }
                }
                None => bits_per_sample = Some(track.bits_per_sample),
            }
        }

        if sample_format.is_none() {
            return Err("no sample format found".into());
        }
        if bits_per_sample.is_none() {
            return Err("no bits per sample found".into());
        }

        Ok(Song {
            name,
            midi_event,
            midi_file,
            num_channels,
            sample_rate,
            sample_format: sample_format.expect("sample format not found"),
            bits_per_sample: bits_per_sample.expect("bits per sample not found"),
            duration: max_duration,
            tracks,
        })
    }

    /// Returns the duration string in minutes and seconds.
    pub fn duration_string(&self) -> String {
        let secs = self.duration.as_secs();
        format!("{}:{:02}", secs / 60, secs % 60)
    }

    /// Returns a rodio source for the song.
    pub fn source<S>(&self) -> Result<SongSource<S>, Box<dyn Error>>
    where
        S: Sample,
    {
        SongSource::<S>::new(self.num_channels, self)
    }

    /// Returns a MIDI sheet for the song.
    pub fn midi_sheet(&self) -> Result<Option<MidiSheet>, Box<dyn Error>> {
        match &self.midi_file {
            Some(midi_file) => {
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
                Ok(Some(midi_sheet))
            }
            None => Ok(None),
        }
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
            self.midi_file,
            self.tracks
                .iter()
                .map(|track| track.name.clone())
                .collect::<Vec<String>>()
                .join(", "),
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
    /// The bits per sample.
    bits_per_sample: u16,
    /// The output channel for this file.
    channel: u16,
    /// The duration of the track.
    duration: Duration,
}

impl Track {
    pub fn new(
        name: String,
        track_file: PathBuf,
        file_channel: Option<u16>,
        channel: u16,
    ) -> Result<Track, Box<dyn Error>> {
        let reader = WavReader::open(track_file.clone())?;
        let spec = reader.spec();
        let sample_rate = spec.sample_rate;
        let duration = Duration::from_secs(u64::from(reader.duration()) / u64::from(sample_rate));
        let bits_per_sample = spec.bits_per_sample;

        if spec.channels > 1 && file_channel.is_none() {
            return Err(format!(
                "track {} has more than one channel but file_channel is not specified",
                name,
            )
            .into());
        }
        let file_channel = file_channel.unwrap_or(1);

        Ok(Track {
            name,
            file: track_file.clone(),
            file_channel,
            sample_rate,
            sample_format: spec.sample_format,
            bits_per_sample,
            channel,
            duration,
        })
    }
}

/// A source for all of the combined tracks in the song.
pub struct SongSource<S>
where
    S: Sample,
{
    finished: Arc<AtomicBool>,
    cons: HeapConsumer<S>,
    join_handle: Option<JoinHandle<()>>,
    channels: u16,
    frame_pos: Arc<AtomicU16>,
}

/// The decoder and the file channel mapping.
struct DecoderAndMapping {
    decoder: WavReader<BufReader<File>>,
    file_channel_to_output_channels: HashMap<u16, Vec<usize>>,
    num_channels: u16,
}

impl<S> SongSource<S>
where
    S: Sample,
{
    fn new(channels: u16, song: &Song) -> Result<SongSource<S>, Box<dyn Error>> {
        let mut files_to_tracks = HashMap::<PathBuf, Vec<&Track>>::new();

        song.tracks.iter().for_each(|track| {
            let file_path = &track.file;
            if !files_to_tracks.contains_key(file_path) {
                files_to_tracks.insert(file_path.clone(), Vec::new());
            }

            files_to_tracks.get_mut(file_path).unwrap().push(track);
        });

        let buf = HeapRb::new(usize::from(channels) * usize::try_from(song.sample_rate)? * 10);
        let (prod, cons) = buf.split();
        let mut decoders_and_mappings = Vec::<DecoderAndMapping>::new();

        for (file_path, tracks) in files_to_tracks {
            // Open the various files for reading.
            let decoder = WavReader::open(file_path)?;
            let num_channels = decoder.spec().channels;

            let mut file_channel_to_output_channels: HashMap<u16, Vec<usize>> = HashMap::new();
            tracks.into_iter().for_each(|track| {
                let file_channel = track.file_channel - 1;
                let output_channel = track.channel - 1;
                file_channel_to_output_channels
                    .entry(file_channel)
                    .or_default()
                    .push(output_channel.into());
            });

            decoders_and_mappings.push(DecoderAndMapping {
                decoder,
                file_channel_to_output_channels,
                num_channels,
            })
        }

        let finished = Arc::new(AtomicBool::new(false));
        let join_handle = {
            SongSource::reader_thread(
                usize::from(channels),
                song.bits_per_sample,
                decoders_and_mappings,
                prod,
                finished.clone(),
            )
        };

        let source = SongSource {
            finished,
            cons,
            join_handle: Some(join_handle),
            channels,
            frame_pos: Arc::new(AtomicU16::new(0)),
        };

        source.wait_for_buffer_or_stop();

        Ok(source)
    }

    fn reader_thread(
        num_channels: usize,
        bits_per_sample: u16,
        mut decoder_and_mappings: Vec<DecoderAndMapping>,
        mut prod: HeapProducer<S>,
        finished: Arc<AtomicBool>,
    ) -> JoinHandle<()> {
        thread::spawn(move || {
            // The number of frames to read at a time from the source. We'll make it one quarter of the total capacity.
            let num_frames = prod.capacity() / num_channels / 2;
            let num_sources = decoder_and_mappings.len();
            let mut sample_sources: Vec<WavSamples<'_, BufReader<File>, S>> =
                Vec::with_capacity(num_sources);
            let mut channels: Vec<u16> = Vec::with_capacity(num_sources);
            let mut file_mappings: Vec<&HashMap<u16, Vec<usize>>> = Vec::with_capacity(num_sources);
            let mut frames = vec![S::default(); num_channels * num_frames];

            for decoder_and_mappings in decoder_and_mappings.iter_mut() {
                sample_sources.push(decoder_and_mappings.decoder.samples());
                channels.push(decoder_and_mappings.num_channels);
                file_mappings.push(&decoder_and_mappings.file_channel_to_output_channels);
            }

            loop {
                // Wait until the buffer is half empty before proceeding.
                while prod.len() > prod.capacity() / 2 {
                    thread::sleep(Duration::from_millis(200))
                }

                // Load the entire buffer until it's full.
                let num_frames_to_take = prod.free_len() / num_channels;
                for i in 0..num_frames_to_take {
                    // If finished gets set, we'll return immediately.
                    if finished.load(Ordering::Relaxed) {
                        return;
                    }

                    let mut all_files_finished = true;
                    let current_frame = i % num_frames;
                    for j in 0..num_sources {
                        let file_channel_to_output_channels = file_mappings[j];
                        let samples = &mut sample_sources[j];

                        // Populate the current frame.
                        for file_channel in 0..channels[j] {
                            let result = samples.next();
                            if let Some(sample) = result {
                                all_files_finished = false;
                                if let Some(targets) =
                                    file_channel_to_output_channels.get(&file_channel)
                                {
                                    for target in targets {
                                        frames[*target + current_frame * num_channels] +=
                                            match sample {
                                                Ok(sample) => sample.scale(bits_per_sample),
                                                Err(ref e) => {
                                                    error!(
                                                        err = e.to_string(),
                                                        "Error reading sample"
                                                    );
                                                    S::default()
                                                }
                                            };
                                    }
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
                        for sample in 0..current_frame * num_channels {
                            frames[sample] = S::default();
                        }
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
        let sample = self.cons.pop();

        if sample.is_some() {
            self.frame_pos
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |val| {
                    Some((val + 1) % self.channels)
                })
                .expect("Got none from frame position update function");
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

#[cfg(test)]
mod test {
    use std::{error::Error, path::PathBuf};

    use crate::{config, test::write_wav};

    use super::{Sample, SongSource};

    #[test]
    fn read_files() {
        let songs = config::parse_songs(&PathBuf::from("assets/songs/songs.yaml"))
            .expect("Unable to parse songs.");

        // Check that the first few songs were parsed correctly.
        let song = &songs[0];
        assert_eq!("Song 1", song.name);
        assert_eq!(1, song.num_channels);
        assert_eq!(22050, song.sample_rate);
        assert!(song.midi_event.is_none());
        assert!(song.midi_file.is_some());

        let song = &songs[1];
        assert_eq!("Song 2", song.name);
        assert_eq!(1, song.num_channels);
        assert_eq!(44100, song.sample_rate);
        assert!(song.midi_event.is_some());
        assert!(song.midi_file.is_none());

        let song = &songs[2];
        assert_eq!("Song 3", song.name);
        assert_eq!(2, song.num_channels);
        assert_eq!(44100, song.sample_rate);
        assert!(song.midi_event.is_none());
        assert!(song.midi_file.is_some());

        let song = &songs[3];
        assert_eq!("Song 4", song.name);
        assert_eq!(8, song.num_channels);
        assert_eq!(44100, song.sample_rate);
        assert!(song.midi_event.is_none());
        assert!(song.midi_file.is_some());
    }

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
        let tempwav1_path = tempdir.join("tempwav1.wav");
        let tempwav2_path = tempdir.join("tempwav2.wav");
        let tempwav3_path = tempdir.join("tempwav3.wav");

        write_wav(
            tempwav1_path.clone(),
            vec![1_i32, 2_i32, 3_i32, 4_i32, 5_i32],
        )?;
        write_wav(tempwav2_path.clone(), vec![2_i32, 3_i32, 4_i32])?;
        write_wav(tempwav3_path.clone(), vec![0_i32, 0_i32, 1_i32, 2_i32])?;

        let track1 = super::Track::new("test 1".into(), tempwav1_path, Some(1), 1)?;
        let track2 = super::Track::new("test 2".into(), tempwav2_path, Some(1), 4)?;
        let track3 = super::Track::new("test 3".into(), tempwav3_path, Some(1), 4)?;

        let song = super::Song::new("song name".into(), None, None, vec![track1, track2, track3])?;
        song.source()
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
}
