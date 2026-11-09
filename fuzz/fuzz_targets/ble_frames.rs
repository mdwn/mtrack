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

// The frame reassembler must never panic on arbitrary chunks, never hand back
// a frame over MAX_FRAME, and a frame it does hand back must decode as an
// envelope without a panic. The input is cut into chunks: each chunk's length
// is the byte before it.
// Run with: cargo +nightly fuzz run ble_frames
#![no_main]

use libfuzzer_sys::fuzz_target;
use mtrack::controller::ble::codec::{Reassembler, MAX_FRAME};
use mtrack::proto::ble::v1::Request;
use prost::Message;

fuzz_target!(|data: &[u8]| {
    let mut reassembler = Reassembler::new();
    let mut rest = data;
    while let Some((&len, tail)) = rest.split_first() {
        let (chunk, after) = tail.split_at(tail.len().min(usize::from(len)));
        rest = after;
        if let Ok(Some(frame)) = reassembler.push(chunk) {
            assert!(frame.len() <= MAX_FRAME);
            let _ = Request::decode(frame.as_slice());
        }
    }
});
