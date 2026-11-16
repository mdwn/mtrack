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

use crate::lighting::effects::DmxCommand;

/// What the show's frames have left on the wire, channel by channel. A DMX
/// universe holds a byte until it is sent another, so a channel no frame
/// writes any more -- its effect over, cleared, a new song's show not
/// driving it -- keeps the last byte it was sent: a gobo, a zoom, a mode.
/// A level or colour is the exception: the frame after the last one that
/// wrote it sends it 0 (#581).
///
/// The one record of those bytes: the live output sends every frame
/// through it on its way to the universes, an offline evaluation sends the
/// frames a play-through would have, and every fixture snapshot reads its
/// bytes back from it -- so what the state says a channel holds is what
/// the universe holds.
#[derive(Clone, Debug, Default)]
pub struct Wire {
    /// Every channel (universe, DMX channel) a frame has written, at the
    /// byte it was last sent.
    bytes: HashMap<(u16, u16), u8>,
    /// The channels the last frame wrote.
    written: HashSet<(u16, u16)>,
    /// The universes there is an output for, when that is known (the live
    /// output's profile): a byte for any other goes nowhere, so the wire
    /// holds none there. `None` -- an offline evaluation -- takes every
    /// universe as one.
    outputs: Option<HashSet<u16>>,
}

impl Wire {
    /// Sends `frame` out, as the universes take it: each byte held from
    /// now. Answers the levels and colours (of `levels`) the last frame
    /// wrote and this one does not, at 0, to send after it; they are held
    /// at 0 here already.
    /// A byte for a universe with no output ([`Self::set_outputs`]) is
    /// not held: it never reaches a universe.
    pub fn send(&mut self, frame: &[DmxCommand], levels: &HashSet<(u16, u16)>) -> Vec<DmxCommand> {
        let frame: Vec<&DmxCommand> = frame
            .iter()
            .filter(|c| self.outputs_to(c.universe))
            .collect();
        let written: HashSet<(u16, u16)> = frame.iter().map(|c| (c.universe, c.channel)).collect();
        let released: Vec<DmxCommand> = self
            .written
            .difference(&written)
            .filter(|at| levels.contains(at))
            .map(|&(universe, channel)| DmxCommand {
                universe,
                channel,
                value: 0,
            })
            .collect();
        for command in frame.into_iter().chain(&released) {
            self.bytes
                .insert((command.universe, command.channel), command.value);
        }
        self.written = written;
        released
    }

    /// The byte `channel` of `universe` holds, if a frame has ever written
    /// it.
    pub fn byte(&self, universe: u16, channel: u16) -> Option<u8> {
        self.bytes.get(&(universe, channel)).copied()
    }

    /// The channels the last frame wrote.
    pub fn written(&self) -> &HashSet<(u16, u16)> {
        &self.written
    }

    /// Sets the universes there is an output for (`None`: every one). The
    /// bytes held for any other are dropped: they never reached a rig.
    pub fn set_outputs(&mut self, outputs: Option<HashSet<u16>>) {
        if let Some(outputs) = &outputs {
            self.bytes
                .retain(|(universe, _), _| outputs.contains(universe));
            self.written
                .retain(|(universe, _)| outputs.contains(universe));
        }
        self.outputs = outputs;
    }

    /// Whether `universe` has an output, so the wire carries its bytes to
    /// a rig. A fixture patched on one that does not is dark, whatever
    /// its effects compute.
    pub fn outputs_to(&self, universe: u16) -> bool {
        self.outputs.as_ref().is_none_or(|o| o.contains(&universe))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(bytes: &[(u16, u8)]) -> Vec<DmxCommand> {
        bytes
            .iter()
            .map(|&(channel, value)| DmxCommand {
                universe: 1,
                channel,
                value,
            })
            .collect()
    }

    #[test]
    fn a_channel_no_frame_writes_holds_its_byte_but_a_level_goes_to_zero() {
        let mut wire = Wire::default();
        let levels = HashSet::from([(1, 1)]);
        assert!(wire.send(&frame(&[(1, 200), (2, 128)]), &levels).is_empty());
        assert_eq!((wire.byte(1, 1), wire.byte(1, 2)), (Some(200), Some(128)));
        // Neither written now: the level goes to 0, the setting holds.
        let released = wire.send(&[], &levels);
        assert_eq!(released.len(), 1);
        assert_eq!((released[0].channel, released[0].value), (1, 0));
        assert_eq!((wire.byte(1, 1), wire.byte(1, 2)), (Some(0), Some(128)));
        assert_eq!(wire.byte(1, 3), None, "never written");
        // Released once: nothing to send again.
        assert!(wire.send(&[], &levels).is_empty());
    }

    #[test]
    fn a_universe_with_no_output_holds_nothing() {
        let mut wire = Wire::default();
        wire.set_outputs(Some(HashSet::from([1])));
        let levels = HashSet::from([(1, 1), (2, 1)]);
        let mut frame = frame(&[(1, 200)]);
        frame.push(DmxCommand {
            universe: 2,
            channel: 1,
            value: 255,
        });
        assert!(wire.send(&frame, &levels).is_empty());
        assert_eq!((wire.byte(1, 1), wire.byte(2, 1)), (Some(200), None));
        assert!(!wire.outputs_to(2));
        assert!(!wire.written().contains(&(2, 1)));
        // Nothing held there, so nothing to release.
        let released = wire.send(&[], &levels);
        assert_eq!(released.len(), 1);
        assert_eq!(released[0].universe, 1);
    }
}
