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

import type {
  FixtureMetadata,
  LogLine,
  Playback,
  PoseSnapshot,
  ShowReload,
  StageValues,
  VenueMetadata,
  Waveform,
} from "../../gen/rules/mtrack_rules";
import { writable } from "svelte/store";
import { connect, on, onConnectionStatus } from "./connection";

/** The playback status, with `received_at`: performance.now() when the
 *  frame was received, so consumers extrapolate `elapsed_ms` smoothly
 *  between the 5Hz frames. */
export type PlaybackState = Playback & { received_at: number };

// --- Stores ---

export const wsConnected = writable(false);

/** Before the first frame; a frame's missing fields keep these too. */
const IDLE: PlaybackState = {
  is_playing: false,
  elapsed_ms: 0,
  song_name: "",
  song_duration_ms: 0,
  playlist_name: "",
  playlist_position: 0,
  playlist_off_list: false,
  playlist_songs: [],
  tracks: [],
  available_playlists: [],
  persisted_playlist_name: "",
  locked: true,
  hardware_reload_started: 0,
  hardware_reload_refused: [],
  process_id: "",
  beat_grid: null,
  looping: false,
  available_sections: [],
  active_section: null,
  pending_start_ms: null,
  reactive_loop_state: { state: "idle" },
  pilot_hints: [],
  tempo: null,
  auto_play: null,
  ending: null,
  ambient: null,
  next: null,
  received_at: 0,
};

export const playbackStore = writable<PlaybackState>(IDLE);

export const fixtureStore = writable<StageValues["fixtures"]>({});
export const strobeRateStore = writable<StageValues["strobe_rates"]>({});
/**
 * Per-cell values for fixtures a per-cell effect is driving this frame;
 * a fixture absent here shows its own channels on every cell.
 */
export const cellStore = writable<StageValues["cells"]>({});

/** Where a mover points, from the engine's pose memory. */
export const poseStore = writable<Record<string, PoseSnapshot>>({});
/**
 * Venue fixtures a fixture test's output covers: their channels, cells and
 * poses in the stores above are the test's bytes, what leaves for olad.
 */
export const underTestStore = writable<string[]>([]);
export const metadataStore = writable<Record<string, FixtureMetadata>>({});
export const venueStore = writable<VenueMetadata | null>(null);

export const effectsStore = writable<string[]>([]);

const MAX_LOG_LINES = 200;
export const logStore = writable<LogLine[]>([]);

export const waveformStore = writable<Waveform>({
  song_name: "",
  tracks: [],
});

export const reloadStore = writable<ShowReload | null>(null);

// --- Wire up ---

onConnectionStatus((connected) => {
  wsConnected.set(connected);
});

on("playback", (m) => {
  playbackStore.set({ ...IDLE, ...m, received_at: performance.now() });
});

on("state", (m) => {
  // Before the channels: a plot redraws on them and reads these.
  underTestStore.set(m.under_test ?? []);
  strobeRateStore.set(m.strobe_rates ?? {});
  fixtureStore.set(m.fixtures ?? {});
  effectsStore.set(m.active_effects ?? []);
  poseStore.set(m.poses ?? {});
  cellStore.set(m.cells ?? {});
});

on("metadata", (m) => {
  metadataStore.set(m.fixtures ?? {});
  venueStore.set(m.venue ?? null);
});

on("logs", (m) => {
  logStore.update((prev) => {
    const next = [...prev, ...m.lines];
    return next.length > MAX_LOG_LINES
      ? next.slice(next.length - MAX_LOG_LINES)
      : next;
  });
});

on("waveform", (m) => {
  waveformStore.set({ song_name: m.song_name, tracks: m.tracks ?? [] });
});

on("reload", (m) => {
  reloadStore.set(m);
  // Clear after 3 seconds
  setTimeout(() => reloadStore.set(null), 3000);
});

// Connect on module load
connect();
