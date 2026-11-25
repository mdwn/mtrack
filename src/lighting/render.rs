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

//! A song's light show rendered to a file: every frame the rig would be
//! sent, as the bytes of its universes.
//!
//! The file is a line naming the format, a line of JSON ([`Header`]), and
//! then the frames with nothing between them: for each frame, earliest
//! first, each universe the header lists, in its order, as 512 bytes
//! (channel 1 first). Frame `n` is the show at `n / frames_per_second`
//! seconds. The bytes are the wire's ([`Wire::universe`]): what a rig that
//! played the show from its start would hold at that moment, a channel no
//! frame has written at 0.
//!
//! [`Wire::universe`]: crate::lighting::engine::Wire::universe

use std::io::{self, Write};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::lighting::evaluate::{evaluate_show_runs, VenueRig};
use crate::lighting::parser::LightShow;
use crate::lighting::tempo::TempoMap;
use crate::songs::Song;

/// The file's first line.
pub const FORMAT: &str = "mtrack-dmx-frames 1";

/// The bytes of one universe in a frame.
pub const UNIVERSE_BYTES: usize = 512;

/// What a rendered file holds, on its second line.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Header {
    pub frames_per_second: u32,
    pub frames: u64,
    /// The universes of each frame, in the order their bytes come: every
    /// universe a fixture of the venue is patched on, lowest first.
    pub universes: Vec<u16>,
}

/// Why a song's show was not rendered.
#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("the song has no light show")]
    NoShow,
    #[error("there is no venue to render the show on")]
    NoVenue,
    #[error("the venue's fixtures could not be loaded: {0}")]
    Venue(String),
    #[error("a show is rendered at one frame a second or more")]
    NoFrames,
    #[error(transparent)]
    Io(#[from] io::Error),
}

/// Renders `shows` on `rig` from their start to `length`, at
/// `frames_per_second`, on up to `workers` threads, and writes the file to
/// `out`. The frames are the same however many threads render them.
pub fn render_shows(
    shows: &[LightShow],
    rig: &VenueRig,
    fallback_tempo_map: Option<&TempoMap>,
    length: Duration,
    frames_per_second: u32,
    workers: usize,
    out: &mut impl Write,
) -> Result<Header, RenderError> {
    if frames_per_second == 0 {
        return Err(RenderError::NoFrames);
    }
    let mut universes: Vec<u16> = rig.fixtures.iter().map(|f| f.universe).collect();
    universes.sort();
    universes.dedup();

    // Frame n is at n / frames_per_second seconds, to the nanosecond, the
    // last one at or before the end.
    let per_second = u128::from(frames_per_second);
    let last = length.as_nanos() * per_second / 1_000_000_000;
    let times: Vec<Duration> = (0..=last)
        .map(|n| {
            let nanos = n * 1_000_000_000 / per_second;
            Duration::new(
                (nanos / 1_000_000_000) as u64,
                (nanos % 1_000_000_000) as u32,
            )
        })
        .collect();

    let frame = universes.len() * UNIVERSE_BYTES;
    let runs = evaluate_show_runs(
        shows,
        &rig.fixtures,
        &rig.focus_points,
        fallback_tempo_map,
        &times,
        workers,
        |effect| rig.resolve(effect),
        |frames| Vec::with_capacity(frames * frame),
        |bytes: &mut Vec<u8>, engine, _| {
            for &universe in &universes {
                bytes.extend_from_slice(&engine.wire().universe(universe));
            }
        },
    );

    let header = Header {
        frames_per_second,
        frames: times.len() as u64,
        universes,
    };
    writeln!(out, "{FORMAT}")?;
    serde_json::to_writer(&mut *out, &header).map_err(io::Error::from)?;
    writeln!(out)?;
    for run in runs {
        out.write_all(&run)?;
    }
    Ok(header)
}

/// Renders `song`'s light show on the current venue of `system`
/// ([`render_shows`]), for as long as the song plays.
pub fn render_song(
    system: &mut crate::lighting::system::LightingSystem,
    song: &Song,
    frames_per_second: u32,
    workers: usize,
    out: &mut impl Write,
) -> Result<Header, RenderError> {
    let shows = crate::songs::registered_shows(song);
    if shows.is_empty() {
        return Err(RenderError::NoShow);
    }
    let rig = VenueRig::of(system, &shows);
    if let Some(reason) = rig.venue_error {
        return Err(RenderError::Venue(reason));
    }
    if rig.fixtures.is_empty() {
        return Err(RenderError::NoVenue);
    }
    render_shows(
        &shows,
        &rig,
        song.lighting_tempo_map().as_ref(),
        song.length(),
        frames_per_second,
        workers,
        out,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lighting::evaluate::evaluate_show;
    use crate::testutil::lighting::{rgb, sorted_shows};
    use std::collections::HashMap;

    const SHOW: &str = r#"
tempo {
    start: 0.0s
    bpm: 120
    time_signature: 4/4
}

show "Two" {
    @1/1
    a: static color: "blue", duration: 3s
    b: pulse duration: 500ms
    @2/1
    b: cycle color: "red", color: "green", color: "blue", speed: 1measure, duration: 2s
    @2/3
    a: dimmer start_level: 1.0, end_level: 0.0, duration: 1s
}
"#;

    /// Two fixtures, on universes 1 and 3.
    fn rig() -> VenueRig {
        let mut far = rgb("b", 10);
        far.universe = 3;
        VenueRig {
            fixtures: vec![rgb("a", 1), far],
            ..VenueRig::default()
        }
    }

    fn render(workers: usize) -> (Header, Vec<u8>) {
        let mut file = Vec::new();
        let header = render_shows(
            &sorted_shows(SHOW),
            &rig(),
            None,
            Duration::from_secs(5),
            20,
            workers,
            &mut file,
        )
        .expect("rendered");
        let text_end = file
            .iter()
            .enumerate()
            .filter(|(_, b)| **b == b'\n')
            .nth(1)
            .map(|(at, _)| at + 1)
            .expect("two lines before the frames");
        let mut lines = std::str::from_utf8(&file[..text_end]).unwrap().lines();
        assert_eq!(lines.next(), Some(FORMAT));
        let read: Header = serde_json::from_str(lines.next().unwrap()).unwrap();
        assert_eq!(read, header);
        (header, file[text_end..].to_vec())
    }

    #[test]
    fn a_rendered_file_is_its_header_and_every_frame_of_every_universe() {
        let (header, frames) = render(1);
        assert_eq!(header.frames_per_second, 20);
        // 0 s to 5 s at twenty a second, both ends in.
        assert_eq!(header.frames, 101);
        assert_eq!(header.universes, [1, 3]);
        assert_eq!(frames.len(), 101 * 2 * UNIVERSE_BYTES);
    }

    /// Each frame holds, at each fixture's channels, what an evaluation of
    /// that moment says the fixture shows.
    #[test]
    fn a_frame_holds_what_an_evaluation_of_its_moment_shows() {
        let (header, frames) = render(1);
        let times: Vec<Duration> = (0..header.frames)
            .map(|n| Duration::from_millis(n * 50))
            .collect();
        let evaluated = evaluate_show(
            sorted_shows(SHOW),
            &rig().fixtures,
            &HashMap::new(),
            None,
            &times,
            |e| e,
        );
        let mut lit = 0;
        for (n, evaluation) in evaluated.iter().enumerate() {
            let frame = &frames[n * 2 * UNIVERSE_BYTES..(n + 1) * 2 * UNIVERSE_BYTES];
            // "a" at 1:1, "b" at 3:10, the second universe of the frame.
            for (fixture, at) in [("a", 0), ("b", UNIVERSE_BYTES + 9)] {
                let shown = evaluation
                    .fixtures
                    .iter()
                    .find(|f| f.name == fixture)
                    .expect("in the evaluation");
                for (offset, colour) in ["red", "green", "blue"].iter().enumerate() {
                    let byte = shown.channels.get(*colour).copied().unwrap_or(0);
                    assert_eq!(frame[at + offset], byte, "{fixture} {colour}, frame {n}");
                    lit += usize::from(byte > 0);
                }
            }
        }
        assert!(lit > 50, "the show lights the fixtures it is compared on");
    }

    #[test]
    fn the_frames_are_the_same_on_any_number_of_threads() {
        let one = render(1);
        for workers in [2, 3, 16] {
            assert_eq!(render(workers), one, "{workers} threads");
        }
    }

    #[test]
    fn no_frames_a_second_is_refused() {
        let refused = render_shows(
            &sorted_shows(SHOW),
            &rig(),
            None,
            Duration::from_secs(1),
            0,
            1,
            &mut Vec::new(),
        );
        assert!(matches!(refused, Err(RenderError::NoFrames)));
    }
}
