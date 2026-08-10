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
//! Captures the MIDI mtrack actually transmits.
//!
//! Everything else in the suite observes MIDI through mtrack's own reporting,
//! which cannot distinguish "sent the right bytes at the right times" from
//! "believed it did". Listening on the same port mtrack sends to gives
//! byte-exact timings: a beat clock is either 24 pulses per quarter note at
//! the song's tempo or it is not.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use midir::{Ignore, MidiInput, MidiInputConnection};

/// MIDI real-time status byte for a timing clock pulse.
pub const TIMING_CLOCK: u8 = 0xF8;
/// MIDI real-time status byte for start.
pub const START: u8 = 0xFA;
/// MIDI real-time status byte for stop.
pub const STOP: u8 = 0xFC;

/// One received message and when it arrived.
#[derive(Debug, Clone)]
pub struct TimedMessage {
    /// Microseconds since the port was opened, as reported by the driver.
    ///
    /// Good for the *rate* of a continuous run of messages, and nothing else.
    /// On ALSA it does not advance while no events are flowing: a Stop and the
    /// Start of the next song a second later report the same value. Anything
    /// measuring silence must use [`TimedMessage::at`], or it will measure zero
    /// however long the silence was.
    pub micros: u64,
    /// When the harness itself received the message.
    ///
    /// One monotonic clock that keeps running whether or not MIDI is arriving,
    /// so gaps are real. Costs a syscall in the callback and carries the
    /// scheduling delay between the wire and this process -- irrelevant at the
    /// scale of a dropped clock, fatal for measuring inter-tick jitter, which is
    /// what `micros` is for.
    pub at: Instant,
    pub bytes: Vec<u8>,
}

impl TimedMessage {
    pub fn status(&self) -> Option<u8> {
        self.bytes.first().copied()
    }

    /// Whether this is a note-on with a non-zero velocity.
    ///
    /// A note-on with velocity 0 is a note-off by convention, so counting raw
    /// 0x9n statuses would double-count every note.
    pub fn is_note_on(&self) -> bool {
        matches!(self.bytes.as_slice(), [status, _, vel]
            if status & 0xF0 == 0x90 && *vel > 0)
    }

    /// The note number, for note-on and note-off messages.
    pub fn note(&self) -> Option<u8> {
        match self.bytes.as_slice() {
            [status, note, _] if status & 0xF0 == 0x90 || status & 0xF0 == 0x80 => Some(*note),
            _ => None,
        }
    }
}

/// An open MIDI input capturing everything it receives.
pub struct MidiCapture {
    messages: Arc<Mutex<Vec<TimedMessage>>>,
    /// Held to keep the port open; dropping it closes the connection.
    _connection: MidiInputConnection<()>,
}

impl MidiCapture {
    /// Opens the port whose name contains `needle` and starts recording.
    pub fn open(needle: &str) -> Result<MidiCapture, Box<dyn std::error::Error>> {
        let mut input = MidiInput::new("mtrack-e2e-capture")?;
        // midir filters out timing, active sense, and sysex by default. The
        // beat clock is precisely what this capture exists to see, so nothing
        // may be ignored.
        input.ignore(Ignore::None);

        let ports = input.ports();
        let port = ports
            .iter()
            .find(|p| {
                input
                    .port_name(p)
                    .map(|name| name.contains(needle))
                    .unwrap_or(false)
            })
            .ok_or_else(|| {
                let available: Vec<String> = ports
                    .iter()
                    .filter_map(|p| input.port_name(p).ok())
                    .collect();
                format!("no MIDI input port matching '{needle}'; available: {available:?}")
            })?
            .clone();

        let port_name = input.port_name(&port)?;
        let messages = Arc::new(Mutex::new(Vec::new()));
        let sink = messages.clone();

        let connection = input.connect(
            &port,
            "mtrack-e2e",
            move |micros, bytes, _| {
                sink.lock()
                    .expect("midi capture buffer poisoned")
                    .push(TimedMessage {
                        micros,
                        at: Instant::now(),
                        bytes: bytes.to_vec(),
                    });
            },
            (),
        )?;

        println!("  capturing MIDI on '{port_name}'");
        Ok(MidiCapture {
            messages,
            _connection: connection,
        })
    }

    /// Everything received so far.
    pub fn messages(&self) -> Vec<TimedMessage> {
        self.messages
            .lock()
            .expect("midi capture buffer poisoned")
            .clone()
    }

    /// Discards anything received so far.
    pub fn clear(&self) {
        self.messages
            .lock()
            .expect("midi capture buffer poisoned")
            .clear();
    }

    /// Timing clock pulses received.
    pub fn clock_pulses(&self) -> Vec<TimedMessage> {
        self.messages()
            .into_iter()
            .filter(|m| m.status() == Some(TIMING_CLOCK))
            .collect()
    }

    /// Note-on messages received, in arrival order.
    pub fn note_ons(&self) -> Vec<TimedMessage> {
        self.messages()
            .into_iter()
            .filter(|m| m.is_note_on())
            .collect()
    }

    /// Measured beat clock rate in pulses per second.
    ///
    /// Measured across the span between the first and last pulse rather than
    /// from a nominal duration, so scheduling jitter at the edges of the
    /// capture does not bias the result.
    pub fn clock_hz(&self) -> Option<f32> {
        let pulses = self.clock_pulses();
        if pulses.len() < 2 {
            return None;
        }
        let span = pulses.last()?.micros.checked_sub(pulses.first()?.micros)?;
        if span == 0 {
            return None;
        }
        Some((pulses.len() - 1) as f32 * 1_000_000.0 / span as f32)
    }
}

/// Waits for a condition on the capture, polling until a deadline.
pub async fn wait_for_messages(
    capture: &MidiCapture,
    timeout: Duration,
    predicate: impl Fn(&MidiCapture) -> bool,
) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        if predicate(capture) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    false
}
