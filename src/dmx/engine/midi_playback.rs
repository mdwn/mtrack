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

use super::Engine;

impl Engine {
    /// Advances all MIDI DMX playback cursors to the current song time,
    /// dispatching events via handle_midi_event_by_id.
    pub(super) fn advance_midi_dmx_playbacks(&self) {
        let song_time = match self.get_song_time().checked_sub(self.playback_delay) {
            Some(t) => t,
            None => return, // Still within the playback delay period
        };
        let mut playbacks = self.midi_dmx_playbacks.lock();
        for playback in playbacks.iter_mut() {
            let events = playback.precomputed.events();
            while playback.cursor < events.len() && events[playback.cursor].time <= song_time {
                let event = &events[playback.cursor];
                if playback.midi_channels.is_empty()
                    || playback.midi_channels.contains(&event.channel)
                {
                    self.handle_midi_event_by_id(playback.universe_id, event.message);
                }
                playback.cursor += 1;
            }
        }
    }

    /// Returns true if all MIDI DMX playbacks have finished.
    pub(super) fn midi_dmx_playbacks_finished(&self) -> bool {
        let playbacks = self.midi_dmx_playbacks.lock();
        playbacks.iter().all(|p| p.cursor >= p.precomputed.len())
    }
}
