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

import type { BeatGrid as RuleGrid } from "../../gen/rules/mtrack_rules";
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

/** The player's beat grid, with the accent levels the backend resolves. */
export type BeatGrid = RuleGrid & {
  /** Per-beat accent levels (0 silent, 1 normal, 2 half, 3 accent);
   * present when the song has a metronome. */
  accent_levels?: number[];
};

export interface SectionInfo {
  name: string;
  /** 1-indexed, inclusive. */
  start_measure: number;
  /** 1-indexed, exclusive. */
  end_measure: number;
  /** Beat within `start_measure` (1-based, fractional); absent = the
   *  measure line. */
  start_beat?: number | null;
  /** Beat within `end_measure` (1-based, fractional); absent = the
   *  measure line, keeping the bound exclusive. */
  end_beat?: number | null;
  /** Explicit display color; absent = UI palette rotation. */
  color?: string | null;
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

/** A transition's fade as configured: `exclude` and `lights` null for the
 *  defaults. */
export interface FadeInfo {
  length: string;
  curve: "equal_power" | "linear";
  exclude: string[] | null;
  lights: string | null;
}

/** The song after the current one. A looping song that is playing goes on
 *  only by a loop break, with the playlist's fades; otherwise the next song
 *  starts by itself after `auto_play_ms`, or (null) waits for Play. */
export interface NextUp {
  song: string | null;
  breaks_loop?: boolean;
  /** At the end of the playlist, a looping song playing: Next (or Previous
   *  on the first entry) ends it over `fade_out` rather than moving on. */
  ends_loop?: boolean;
  fade_out?: FadeInfo | null;
  fade_in?: FadeInfo | null;
  auto_play_ms?: number | null;
}

export interface PlaybackState {
  is_playing: boolean;
  elapsed_ms: number;
  song_name: string;
  song_duration_ms: number;
  playlist_name: string;
  playlist_position: number;
  /** A song an edit took off the list, playing on: the playlist sits before
   *  the entry at `playlist_position` rather than on it. */
  playlist_off_list: boolean;
  playlist_songs: string[];
  tracks: TrackInfo[];
  available_playlists: string[];
  persisted_playlist_name: string;
  locked: boolean;
  /** The last deferred hardware reload generation started (or refused). */
  hardware_reload_started: number;
  /** The recent deferred hardware reloads refused, with why. */
  hardware_reload_refused: { generation: number; error: string }[];
  /** The mtrack process the updates come from. */
  process_id?: string;
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
  /** A song about to start by itself, its playlist entry's auto-play gap
   *  running. */
  auto_play?: { song: string; starts_in_ms: number } | null;
  /** A looping song fading out after a stop, with nothing after it; a
   *  second stop cuts it. */
  ending?: { song: string; remaining_ms: number } | null;
  /** The show the rig plays while no song is: its mode, its show (null for
   *  none) and what it is doing (`pending` while its delay runs;
   *  `unavailable` when on with no show to play). Null without DMX. */
  ambient?: {
    mode: AmbientMode;
    show: string | null;
    state: "pending" | "playing" | "off" | "unavailable";
    /** Whose settings are up: the profile's, the playlist's, or the
     *  entry's (the song about to play). */
    source?: AmbientSource;
  } | null;
  /** What comes after the current song and how it arrives; `song` is null
   *  at the end of the playlist. */
  next?: NextUp | null;
  /** performance.now() when this frame was received; lets consumers
   *  extrapolate elapsed_ms smoothly between the 5Hz frames. */
  received_at: number;
}

/** Whose ambient light settings are up. */
export type AmbientSource = "profile" | "playlist" | "entry" | null;

/** Ambient light's mode: between songs, held on, or none. */
export type AmbientMode = "auto" | "on" | "off";

/**
 * A fixture's channel bytes by canonical name, as the `state` message's
 * `fixtures` map carries them. (Hand-written mirror of the server's
 * payload, like the rest of this file.)
 */
export interface FixtureChannels {
  [channel: string]: number;
}

/**
 * The rate each strobing fixture flashes at, in hertz, by fixture: the
 * `state` message's `strobe_rates` map (and an evaluation's). The player
 * says it — a strobe byte read back through the fixture's own strobe rule;
 * in an evaluation also a software strobe's rate — and the views flash at
 * it rather than guess a rate from the byte. A fixture that is not
 * strobing is absent.
 */
export type StrobeRates = Record<string, number>;

export type Vec3 = [number, number, number];

export interface FixtureMetadata {
  tags: string[];
  type: string;
  /** Stage position in meters, when the venue places the fixture. */
  position?: Vec3 | null;
  /** Mounting rotation in degrees about X, Y, Z, when the venue states it. */
  rotation?: Vec3 | null;
  /** The venue's own beam angle in degrees, when a diffuser or filter makes
   *  the fixture wider than its type says; drawing only. */
  beam_angle?: number | null;
  /**
   * The fixture type's rig model, as a path under /api/lighting/assets/,
   * when the type was distilled from a GDTF; the 3D view draws a generic
   * body without one.
   */
  rig?: string | null;
  /**
   * What the fixture can do, as the engine names it: `color`, `pan_tilt`,
   * `strobe`, `cells`. A fixture with `pan_tilt` is a mover.
   */
  capabilities?: string[];
  /**
   * The cells of a pixel fixture, in the manufacturer's order, with their
   * offsets in the fixture's frame (meters); empty for a one-colour
   * fixture. The plot draws one segment each.
   */
  cells?: { name: string; offset: Vec3 }[];
}

/** Per-cell channel values, by fixture then cell (design §17.4). */
export type CellChannels = Record<string, Record<string, FixtureChannels>>;

/** The current venue as the stage view needs it. */
export interface VenueMetadata {
  name: string;
  /** The configured venues directory, for saving edits back. */
  dir: string | null;
  focus_points: Record<string, Vec3>;
  /**
   * The venue's scenery file under /api/lighting/assets/, when the venue
   * was seeded from an MVR; the 3D view draws it.
   */
  scenery?: string | null;
  /** Why the scenery is missing, when the venue should have had some. */
  scenery_error?: string | null;
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
  playlist_off_list: false,
  playlist_songs: [],
  tracks: [],
  available_playlists: [],
  persisted_playlist_name: "",
  locked: true,
  hardware_reload_started: 0,
  hardware_reload_refused: [],
  beat_grid: null,
  looping: false,
  available_sections: [],
  active_section: null,
  pending_start_ms: null,
  reactive_loop_state: null,
  pilot_hints: [],
  tempo: null,
  auto_play: null,
  ending: null,
  ambient: null,
  next: null,
  received_at: 0,
});

export const fixtureStore = writable<Record<string, FixtureChannels>>({});
export const strobeRateStore = writable<StrobeRates>({});
/**
 * Per-cell values for fixtures a per-cell effect is driving this frame;
 * a fixture absent here shows its own channels on every cell.
 */
export const cellStore = writable<CellChannels>({});

/** Where a mover points, from the engine's pose memory. */
export interface FixturePose {
  pan: number;
  tilt: number;
  /** Beam direction in stage space, unit length. */
  aim: Vec3;
  /** Where the beam meets the deck, when it points down. */
  floor: [number, number] | null;
}

export const poseStore = writable<Record<string, FixturePose>>({});
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
    playlist_off_list: m.playlist_off_list ?? false,
    playlist_songs: m.playlist_songs,
    tracks: m.tracks ?? [],
    available_playlists: m.available_playlists ?? [],
    persisted_playlist_name: m.persisted_playlist_name ?? "",
    locked: m.locked ?? true,
    hardware_reload_started: m.hardware_reload_started ?? 0,
    hardware_reload_refused: m.hardware_reload_refused ?? [],
    process_id: m.process_id,
    beat_grid: m.beat_grid ?? null,
    looping: m.looping ?? false,
    available_sections: m.available_sections ?? [],
    active_section: m.active_section ?? null,
    pending_start_ms: m.pending_start_ms ?? null,
    reactive_loop_state: m.reactive_loop_state ?? null,
    pilot_hints: m.pilot_hints ?? [],
    tempo: m.tempo ?? null,
    auto_play: m.auto_play ?? null,
    ending: m.ending ?? null,
    ambient: m.ambient ?? null,
    next: m.next ?? null,
    received_at: performance.now(),
  });
});

on("state", (msg) => {
  const m = msg as {
    type: string;
    fixtures: Record<string, FixtureChannels>;
    active_effects: string[];
    poses?: Record<string, FixturePose>;
    cells?: CellChannels;
    strobe_rates?: StrobeRates;
    under_test?: string[];
  };
  // Before the channels: a plot redraws on them and reads these.
  underTestStore.set(m.under_test ?? []);
  strobeRateStore.set(m.strobe_rates ?? {});
  fixtureStore.set(m.fixtures ?? {});
  effectsStore.set(m.active_effects ?? []);
  poseStore.set(m.poses ?? {});
  cellStore.set(m.cells ?? {});
});

on("metadata", (msg) => {
  const m = msg as {
    type: string;
    fixtures: Record<string, FixtureMetadata>;
    venue?: VenueMetadata | null;
  };
  metadataStore.set(m.fixtures ?? {});
  venueStore.set(
    m.venue ? { ...m.venue, focus_points: m.venue.focus_points ?? {} } : null,
  );
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
