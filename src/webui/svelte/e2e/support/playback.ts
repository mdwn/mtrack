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

// Playback messages for specs to push down the page's `Watch` stream.
import type { Playback } from "../../src/gen/rules/mtrack_rules";
import type { TypedWatchPush } from "./watch-stream";
import { PLAYBACK_STATE } from "../mock-server/test-data";

/** The mock server's playback state, moved on to the setlist's second and
 * last song, Test Song Beta (4:00), with no tracks; `overrides` on top. */
export function playbackState(overrides: Partial<Playback> = {}) {
  return {
    ...PLAYBACK_STATE,
    song_name: "Test Song Beta",
    song_duration_ms: 240000,
    playlist_position: 1,
    playlist_at_first: false,
    playlist_at_last: true,
    tracks: [],
    ...overrides,
  } satisfies TypedWatchPush;
}
