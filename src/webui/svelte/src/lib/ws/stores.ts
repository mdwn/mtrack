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
  StageMetadata,
  StageState,
  StageValues,
  StageVenue,
  Waveform,
} from "../../gen/rules/mtrack_rules";
import { writable } from "svelte/store";
import type { FeedLease } from "../stage/grpc-feed";
import { stageTiming } from "../stage/timing";
import { connect as openSocket, on, onConnectionStatus } from "./connection";

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
  playlist_at_first: true,
  playlist_at_last: true,
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

// The stage's stores are filled by the gRPC stream `WatchStage`
// (lib/stage/grpc-feed.ts), which one lease shares between everything that
// shows the stage: the stream opens when one of these first has a
// subscriber and closes a while after the last leaves. The app provides the
// lease (main.ts); a page fed another way (the stage-only page, which is
// handed the stream by its host) provides none and opens nothing.
let stageLease: FeedLease | null = null;

/** Says where the stage's stores are filled from. Call once, as the app
 *  starts. */
export function provideStageFeed(lease: FeedLease): void {
  stageLease = lease;
}

/** A store of the stage: watching it holds the stage stream open. */
function stageStore<T>(initial: T) {
  return writable<T>(initial, () => stageLease?.acquire());
}

export const fixtureStore = stageStore<StageValues["fixtures"]>({});
export const strobeRateStore = stageStore<StageValues["strobe_rates"]>({});
/**
 * Per-cell values for fixtures a per-cell effect is driving this frame;
 * a fixture absent here shows its own channels on every cell.
 */
export const cellStore = stageStore<StageValues["cells"]>({});

/** Where a mover points, from the engine's pose memory. */
export const poseStore = stageStore<Record<string, PoseSnapshot>>({});
/**
 * Venue fixtures a fixture test's output covers: their channels, cells and
 * poses in the stores above are the test's bytes, what leaves for olad.
 */
export const underTestStore = stageStore<string[]>([]);
export const metadataStore = stageStore<Record<string, FixtureMetadata>>({});
export const venueStore = stageStore<StageVenue | null>(null);

export const effectsStore = stageStore<string[]>([]);

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

/**
 * Puts the stage's state in the stores: a frame of the gRPC stage stream
 * read against its layout (lib/stage/feed.ts).
 */
export function applyStageState(m: Partial<StageState>): void {
  // Before the channels: a plot redraws on them and reads these.
  underTestStore.set(m.under_test ?? []);
  strobeRateStore.set(m.strobe_rates ?? {});
  fixtureStore.set(m.fixtures ?? {});
  effectsStore.set(m.active_effects ?? []);
  poseStore.set(m.poses ?? {});
  cellStore.set(m.cells ?? {});
  stageTiming.applied();
}

/**
 * Puts what is on the stage in the stores: a layout of the gRPC stage
 * stream.
 */
export function applyStageMetadata(m: Partial<StageMetadata>): void {
  metadataStore.set(m.fixtures ?? {});
  venueStore.set(m.venue ?? null);
}

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

/**
 * Opens the websocket that fills these stores, and keeps it open. The app
 * calls this once as it starts (main.ts). Importing this module opens
 * nothing: a page fed another way (the stage-only page, which is given the
 * gRPC stage stream) imports the stores and never calls it.
 */
export function connect(): void {
  openSocket();
}
