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

//! Frames for ATT chunks. A frame is a serialized envelope; on the wire it
//! is split into chunks, each one ATT write or notification. Byte 0 of a
//! chunk is a header: [`START`] begins a frame, [`CONTINUE`] carries on one.
//! A start chunk goes on with the frame's length as a protobuf varint, then
//! frame bytes; a continuation chunk carries frame bytes only.

use std::fmt;

/// The largest frame either side handles.
pub const MAX_FRAME: usize = 64 * 1024;

/// The header byte of the first chunk of a frame.
const START: u8 = 0x01;
/// The header byte of every later chunk of a frame.
const CONTINUE: u8 = 0x00;
/// A length varint longer than this many bytes is refused. A frame length
/// within [`MAX_FRAME`] takes three, so this leaves room for padding only.
const MAX_VARINT_LEN: usize = 5;

/// Why a frame could not be cut into chunks or put back together.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameError {
    /// A chunk with no bytes, so no header.
    EmptyChunk,
    /// A header byte that is neither a start nor a continuation.
    BadHeader(u8),
    /// A continuation chunk with no frame in progress.
    ContinuationWithoutStart,
    /// A start chunk that ends inside its length varint.
    TruncatedLength,
    /// A length varint of more than five bytes.
    OverlongLength,
    /// A frame longer than [`MAX_FRAME`], declared or offered.
    FrameTooLarge(usize),
    /// More frame bytes than the start chunk declared.
    TooManyBytes,
    /// A chunk size too small to carry a header, a length and a frame byte.
    ChunkTooSmall(usize),
}

impl FrameError {
    /// A stable name for the error, as the golden vectors give it.
    pub fn name(&self) -> &'static str {
        match self {
            FrameError::EmptyChunk => "empty_chunk",
            FrameError::BadHeader(_) => "bad_header",
            FrameError::ContinuationWithoutStart => "continuation_without_start",
            FrameError::TruncatedLength => "truncated_length",
            FrameError::OverlongLength => "overlong_length",
            FrameError::FrameTooLarge(_) => "frame_too_large",
            FrameError::TooManyBytes => "too_many_bytes",
            FrameError::ChunkTooSmall(_) => "chunk_too_small",
        }
    }
}

impl fmt::Display for FrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FrameError::EmptyChunk => write!(f, "empty chunk"),
            FrameError::BadHeader(byte) => write!(f, "bad chunk header {byte:#04x}"),
            FrameError::ContinuationWithoutStart => {
                write!(f, "continuation chunk with no frame in progress")
            }
            FrameError::TruncatedLength => write!(f, "frame length cut short"),
            FrameError::OverlongLength => write!(f, "frame length varint too long"),
            FrameError::FrameTooLarge(len) => {
                write!(f, "frame of {len} bytes exceeds the {MAX_FRAME} byte limit")
            }
            FrameError::TooManyBytes => write!(f, "more bytes than the frame declared"),
            FrameError::ChunkTooSmall(size) => write!(f, "chunk size {size} is too small"),
        }
    }
}

impl std::error::Error for FrameError {}

/// Appends `value` as a protobuf varint.
fn put_varint(out: &mut Vec<u8>, mut value: usize) {
    while value >= 0x80 {
        out.push((value & 0x7f) as u8 | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

/// How many bytes `value` takes as a varint.
fn varint_len(value: usize) -> usize {
    let mut len = 1;
    let mut rest = value >> 7;
    while rest > 0 {
        len += 1;
        rest >>= 7;
    }
    len
}

/// Reads a varint from the front of `bytes`: its value and how many bytes
/// it took.
fn read_varint(bytes: &[u8]) -> Result<(u64, usize), FrameError> {
    let mut value = 0u64;
    for (i, &byte) in bytes.iter().enumerate() {
        if i >= MAX_VARINT_LEN {
            return Err(FrameError::OverlongLength);
        }
        value |= u64::from(byte & 0x7f) << (7 * i);
        if byte & 0x80 == 0 {
            return Ok((value, i + 1));
        }
    }
    if bytes.len() >= MAX_VARINT_LEN {
        Err(FrameError::OverlongLength)
    } else {
        Err(FrameError::TruncatedLength)
    }
}

/// Cuts `frame` into chunks of at most `max_chunk` bytes each. A frame
/// with no bytes is one chunk.
pub fn chunks(frame: &[u8], max_chunk: usize) -> Result<Vec<Vec<u8>>, FrameError> {
    if frame.len() > MAX_FRAME {
        return Err(FrameError::FrameTooLarge(frame.len()));
    }
    let head = 1 + varint_len(frame.len());
    if max_chunk < head + usize::from(!frame.is_empty()) {
        return Err(FrameError::ChunkTooSmall(max_chunk));
    }

    let first_room = max_chunk - head;
    let (first_body, mut rest) = frame.split_at(frame.len().min(first_room));
    let mut first = Vec::with_capacity(head + first_body.len());
    first.push(START);
    put_varint(&mut first, frame.len());
    first.extend_from_slice(first_body);

    let mut out = vec![first];
    while !rest.is_empty() {
        let (body, after) = rest.split_at(rest.len().min(max_chunk - 1));
        let mut chunk = Vec::with_capacity(1 + body.len());
        chunk.push(CONTINUE);
        chunk.extend_from_slice(body);
        out.push(chunk);
        rest = after;
    }
    Ok(out)
}

/// Puts chunks back together into frames. It holds the frame in progress and
/// grows its buffer only as bytes arrive, never to a length a peer declared.
#[derive(Debug, Default)]
pub struct Reassembler {
    /// The frame bytes so far.
    buf: Vec<u8>,
    /// The length the frame in progress declared; `None` with no frame in
    /// progress.
    expected: Option<usize>,
}

impl Reassembler {
    /// A reassembler with no frame in progress.
    pub fn new() -> Reassembler {
        Reassembler::default()
    }

    /// Drops the frame in progress, for a disconnect.
    pub fn reset(&mut self) {
        self.buf.clear();
        self.expected = None;
    }

    /// Takes one chunk: the frame, when it completes it. An error drops the
    /// frame in progress. A start chunk while a frame is in progress drops
    /// that frame and begins the new one.
    pub fn push(&mut self, chunk: &[u8]) -> Result<Option<Vec<u8>>, FrameError> {
        let result = self.push_inner(chunk);
        if result.is_err() {
            self.reset();
        }
        result
    }

    fn push_inner(&mut self, chunk: &[u8]) -> Result<Option<Vec<u8>>, FrameError> {
        let (&header, body) = chunk.split_first().ok_or(FrameError::EmptyChunk)?;
        let body = match header {
            START => {
                self.reset();
                let (len, used) = read_varint(body)?;
                if len > MAX_FRAME as u64 {
                    return Err(FrameError::FrameTooLarge(
                        usize::try_from(len).unwrap_or(usize::MAX),
                    ));
                }
                self.expected = Some(len as usize);
                &body[used..]
            }
            CONTINUE => {
                if self.expected.is_none() {
                    return Err(FrameError::ContinuationWithoutStart);
                }
                body
            }
            other => return Err(FrameError::BadHeader(other)),
        };

        let expected = self.expected.unwrap_or(0);
        if self.buf.len() + body.len() > expected {
            return Err(FrameError::TooManyBytes);
        }
        self.buf.extend_from_slice(body);
        if self.buf.len() == expected {
            self.expected = None;
            return Ok(Some(std::mem::take(&mut self.buf)));
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The smallest chunk size that can carry a frame of `len` bytes.
    fn min_chunk(len: usize) -> usize {
        1 + varint_len(len) + usize::from(len > 0)
    }

    fn frame_of(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i % 251) as u8).collect()
    }

    fn unhex(text: &str) -> Vec<u8> {
        assert!(text.len().is_multiple_of(2), "odd hex {text}");
        (0..text.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&text[i..i + 2], 16).expect("hex"))
            .collect()
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// Feeds every chunk to a reassembler; the frame its last one completes.
    fn reassemble(chunks: &[Vec<u8>]) -> Vec<u8> {
        let mut reassembler = Reassembler::new();
        let mut frame = None;
        for (i, chunk) in chunks.iter().enumerate() {
            let done = reassembler.push(chunk).expect("chunk");
            if i + 1 < chunks.len() {
                assert!(done.is_none(), "frame completed early at chunk {i}");
            } else {
                frame = done;
            }
        }
        frame.expect("a complete frame")
    }

    #[test]
    fn round_trips_every_chunk_size_over_a_spread_of_frames() {
        for len in [
            0, 1, 2, 17, 18, 19, 127, 128, 129, 1000, 16383, 16384, 16385, MAX_FRAME,
        ] {
            let frame = frame_of(len);
            // Every size for small frames; a spread of sizes up to the
            // largest ATT payload for large ones, to keep the run short.
            let sizes: Vec<usize> = if len <= 1000 {
                (min_chunk(len)..=517).collect()
            } else {
                let mut sizes = vec![
                    min_chunk(len),
                    min_chunk(len) + 1,
                    20,
                    23,
                    185,
                    244,
                    512,
                    517,
                ];
                sizes.retain(|&size| size >= min_chunk(len));
                sizes
            };
            for max_chunk in sizes {
                let chunks = chunks(&frame, max_chunk).expect("chunks");
                assert!(chunks.iter().all(|c| !c.is_empty() && c.len() <= max_chunk));
                assert_eq!(chunks[0][0], START);
                assert!(chunks[1..].iter().all(|c| c[0] == CONTINUE));
                assert_eq!(reassemble(&chunks), frame, "len {len} at {max_chunk}");
            }
        }
    }

    #[test]
    fn a_chunk_size_below_the_minimum_is_refused() {
        for len in [0, 1, 127, 128, 16384, MAX_FRAME] {
            let frame = frame_of(len);
            let min = min_chunk(len);
            assert!(chunks(&frame, min).is_ok(), "len {len} at {min}");
            assert_eq!(
                chunks(&frame, min - 1),
                Err(FrameError::ChunkTooSmall(min - 1)),
                "len {len}"
            );
        }
        assert_eq!(chunks(&[], 0), Err(FrameError::ChunkTooSmall(0)));
    }

    #[test]
    fn a_frame_over_the_limit_is_refused_both_ways() {
        let frame = frame_of(MAX_FRAME + 1);
        assert_eq!(
            chunks(&frame, 517),
            Err(FrameError::FrameTooLarge(MAX_FRAME + 1))
        );

        let mut reassembler = Reassembler::new();
        let mut start = vec![START];
        put_varint(&mut start, MAX_FRAME + 1);
        assert_eq!(
            reassembler.push(&start),
            Err(FrameError::FrameTooLarge(MAX_FRAME + 1))
        );
        // The limit itself is fine.
        assert_eq!(
            reassemble(&chunks(&frame_of(MAX_FRAME), 517).unwrap()).len(),
            MAX_FRAME
        );
    }

    #[test]
    fn a_hostile_length_allocates_nothing() {
        let mut reassembler = Reassembler::new();
        let mut start = vec![START];
        put_varint(&mut start, MAX_FRAME);
        start.push(0xaa);
        assert_eq!(reassembler.push(&start), Ok(None));
        assert_eq!(reassembler.buf.len(), 1);
        assert!(reassembler.buf.capacity() < 1024);
    }

    #[test]
    fn a_new_start_drops_the_frame_in_progress() {
        let mut reassembler = Reassembler::new();
        let abandoned = chunks(&frame_of(100), 20).unwrap();
        for chunk in &abandoned[..3] {
            assert_eq!(reassembler.push(chunk), Ok(None));
        }
        let fresh = frame_of(30);
        let mut done = None;
        for chunk in chunks(&fresh, 20).unwrap() {
            done = reassembler.push(&chunk).unwrap();
        }
        assert_eq!(done, Some(fresh));
        // Nothing of the dropped frame is left to continue.
        assert_eq!(
            reassembler.push(&abandoned[3]),
            Err(FrameError::ContinuationWithoutStart)
        );
    }

    #[test]
    fn reset_drops_the_frame_in_progress() {
        let mut reassembler = Reassembler::new();
        let chunks = chunks(&frame_of(100), 20).unwrap();
        assert_eq!(reassembler.push(&chunks[0]), Ok(None));
        reassembler.reset();
        assert_eq!(
            reassembler.push(&chunks[1]),
            Err(FrameError::ContinuationWithoutStart)
        );
    }

    #[test]
    fn each_error_resets_the_state() {
        // Each begins a frame, then errs; a continuation then has no frame
        // to join, and a whole frame still comes through.
        let errors: [(&[u8], FrameError); 3] = [
            (&[], FrameError::EmptyChunk),
            (&[0x07], FrameError::BadHeader(0x07)),
            (&[CONTINUE, 1, 2, 3, 4, 5], FrameError::TooManyBytes),
        ];
        for (bad, expected) in errors {
            let mut reassembler = Reassembler::new();
            assert_eq!(reassembler.push(&[START, 0x03, 0x01]), Ok(None));
            assert_eq!(reassembler.push(bad), Err(expected));
            assert_eq!(
                reassembler.push(&[CONTINUE, 0x02]),
                Err(FrameError::ContinuationWithoutStart)
            );
            assert_eq!(reassembler.push(&[START, 0x01, 0x09]), Ok(Some(vec![0x09])));
        }

        // An error on a start chunk itself leaves nothing behind either.
        let mut reassembler = Reassembler::new();
        assert_eq!(
            reassembler.push(&[START, 0x80]),
            Err(FrameError::TruncatedLength)
        );
        assert_eq!(
            reassembler.push(&[CONTINUE, 0x01]),
            Err(FrameError::ContinuationWithoutStart)
        );
    }

    #[test]
    fn a_frame_completes_exactly_once() {
        let mut reassembler = Reassembler::new();
        assert_eq!(reassembler.push(&[START, 0x01, 0x05]), Ok(Some(vec![0x05])));
        assert_eq!(
            reassembler.push(&[CONTINUE, 0x05]),
            Err(FrameError::ContinuationWithoutStart)
        );
    }

    #[derive(serde::Deserialize)]
    struct Vectors {
        valid: Vec<Valid>,
        invalid: Vec<Invalid>,
    }

    #[derive(serde::Deserialize)]
    struct Valid {
        name: String,
        max_chunk: usize,
        frame_hex: String,
        chunks_hex: Vec<String>,
    }

    #[derive(serde::Deserialize)]
    struct Invalid {
        name: String,
        chunks_hex: Vec<String>,
        error: String,
    }

    fn vectors() -> Vectors {
        serde_json::from_str(include_str!("../../proto/ble/v1/testdata/frames.json"))
            .expect("frames.json")
    }

    #[test]
    fn golden_valid_frames() {
        let vectors = vectors();
        assert!(!vectors.valid.is_empty());
        for case in vectors.valid {
            let frame = unhex(&case.frame_hex);
            let chunked = chunks(&frame, case.max_chunk).expect(&case.name);
            let chunked: Vec<String> = chunked.iter().map(|c| hex(c)).collect();
            assert_eq!(chunked, case.chunks_hex, "{}", case.name);

            let chunks: Vec<Vec<u8>> = case.chunks_hex.iter().map(|c| unhex(c)).collect();
            assert_eq!(hex(&reassemble(&chunks)), case.frame_hex, "{}", case.name);
        }
    }

    #[test]
    fn golden_invalid_chunks() {
        let vectors = vectors();
        // Every error a chunk can cause has a vector.
        let named: std::collections::HashSet<&str> =
            vectors.invalid.iter().map(|c| c.error.as_str()).collect();
        for name in [
            "empty_chunk",
            "bad_header",
            "continuation_without_start",
            "truncated_length",
            "overlong_length",
            "frame_too_large",
            "too_many_bytes",
        ] {
            assert!(named.contains(name), "no vector for {name}");
        }
        for case in vectors.invalid {
            let mut reassembler = Reassembler::new();
            let (last, earlier) = case.chunks_hex.split_last().expect(&case.name);
            for chunk in earlier {
                assert!(
                    reassembler.push(&unhex(chunk)).is_ok(),
                    "{} failed early",
                    case.name
                );
            }
            let error = reassembler.push(&unhex(last)).expect_err(&case.name);
            assert_eq!(error.name(), case.error, "{}", case.name);
        }
    }
}
