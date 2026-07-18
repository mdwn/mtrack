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

import { writable } from "svelte/store";
import { connect, on, onConnectionStatus } from "./connection";

// --- Types ---

export interface TrackInfo {
  name: string;
  output_channels: number[];
  /** Output track gain in dB; absent on older backends (treat as 0). */
  gain_db?: number;
  /** Whether the track is muted (independently of the gain); absent on
   * older backends (treat as false). */
  muted?: boolean;
}

export interface BeatGrid {
  beats: number[];
  measure_starts: number[];
}

export interface SectionInfo {
  name: string;
  start_measure: number;
  end_measure: number;
}

export interface ActiveSection {
  name: string;
  start_ms: number;
  end_ms: number;
}

export interface PilotHintInfo {
  label: string;
  at_ms: number;
  start_ms: number;
  end_ms: number;
  has_audio: boolean;
}

export interface ReactiveLoopState {
  state:
    | "idle"
    | "section_offered"
    | "loop_armed"
    | "looping"
    | "break_requested";
  section_name?: string;
}

export interface PlaybackState {
  is_playing: boolean;
  elapsed_ms: number;
  song_name: string;
  song_duration_ms: number;
  playlist_name: string;
  playlist_position: number;
  playlist_songs: string[];
  tracks: TrackInfo[];
  available_playlists: string[];
  persisted_playlist_name: string;
  locked: boolean;
  beat_grid: BeatGrid | null;
  looping: boolean;
  available_sections: SectionInfo[];
  active_section: ActiveSection | null;
  /** Start position for the next play, set by seeking while stopped. */
  pending_start_ms: number | null;
  /** Reactive section-loop state machine. */
  reactive_loop_state: ReactiveLoopState | null;
  /** Pilot hints resolved to song times. */
  pilot_hints: PilotHintInfo[];
  /** Current tempo/meter from the song's tempo map, sampled at the playhead. */
  tempo: { bpm: number; time_signature: [number, number] } | null;
  /** performance.now() when this frame was received; lets consumers
   *  extrapolate elapsed_ms smoothly between the 5Hz frames. */
  received_at: number;
}

export interface FixtureChannels {
  [channel: string]: number;
}

export interface FixtureMetadata {
  tags: string[];
  type: string;
}

export interface LogLine {
  level: string;
  target: string;
  message: string;
}

export interface WaveformData {
  song_name: string;
  tracks: { name: string; peaks: number[] }[];
}

export interface ReloadEvent {
  status: "ok" | "error";
  error?: string;
}

// --- Stores ---

export const wsConnected = writable(false);

export const playbackStore = writable<PlaybackState>({
  is_playing: false,
  elapsed_ms: 0,
  song_name: "",
  song_duration_ms: 0,
  playlist_name: "",
  playlist_position: 0,
  playlist_songs: [],
  tracks: [],
  available_playlists: [],
  persisted_playlist_name: "",
  locked: true,
  beat_grid: null,
  looping: false,
  available_sections: [],
  active_section: null,
  pending_start_ms: null,
  reactive_loop_state: null,
  pilot_hints: [],
  tempo: null,
  received_at: 0,
});

export const fixtureStore = writable<Record<string, FixtureChannels>>({});

export const metadataStore = writable<Record<string, FixtureMetadata>>({});

export const effectsStore = writable<string[]>([]);

const MAX_LOG_LINES = 200;
export const logStore = writable<LogLine[]>([]);

export const waveformStore = writable<WaveformData>({
  song_name: "",
  tracks: [],
});

export const reloadStore = writable<ReloadEvent | null>(null);

// --- Wire up ---

onConnectionStatus((connected) => {
  wsConnected.set(connected);
});

on("playback", (msg) => {
  const m = msg as PlaybackState & { type: string };
  playbackStore.set({
    is_playing: m.is_playing,
    elapsed_ms: m.elapsed_ms,
    song_name: m.song_name,
    song_duration_ms: m.song_duration_ms,
    playlist_name: m.playlist_name,
    playlist_position: m.playlist_position,
    playlist_songs: m.playlist_songs,
    tracks: m.tracks ?? [],
    available_playlists: m.available_playlists ?? [],
    persisted_playlist_name: m.persisted_playlist_name ?? "",
    locked: m.locked ?? true,
    beat_grid: m.beat_grid ?? null,
    looping: m.looping ?? false,
    available_sections: m.available_sections ?? [],
    active_section: m.active_section ?? null,
    pending_start_ms: m.pending_start_ms ?? null,
    reactive_loop_state: m.reactive_loop_state ?? null,
    pilot_hints: m.pilot_hints ?? [],
    tempo: m.tempo ?? null,
    received_at: performance.now(),
  });
});

on("state", (msg) => {
  const m = msg as {
    type: string;
    fixtures: Record<string, FixtureChannels>;
    active_effects: string[];
  };
  fixtureStore.set(m.fixtures ?? {});
  effectsStore.set(m.active_effects ?? []);
});

on("metadata", (msg) => {
  const m = msg as {
    type: string;
    fixtures: Record<string, FixtureMetadata>;
  };
  metadataStore.set(m.fixtures ?? {});
});

on("logs", (msg) => {
  const m = msg as { type: string; lines: LogLine[] };
  logStore.update((prev) => {
    const next = [...prev, ...m.lines];
    return next.length > MAX_LOG_LINES
      ? next.slice(next.length - MAX_LOG_LINES)
      : next;
  });
});

on("waveform", (msg) => {
  const m = msg as WaveformData & { type: string };
  waveformStore.set({
    song_name: m.song_name,
    tracks: m.tracks ?? [],
  });
});

on("reload", (msg) => {
  reloadStore.set(msg as ReloadEvent);
  // Clear after 3 seconds
  setTimeout(() => reloadStore.set(null), 3000);
});

// Connect on module load
connect();
