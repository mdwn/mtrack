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
// The player's gRPC stream `Watch` as a test describes it, framed for
// gRPC-Web. A test pushes the JSON-shaped messages the web UI's stores hold
// (the playback, the waveform, the log lines, a show reload, the status),
// each with a `type`, as they were once pushed down the web UI's websocket; this
// encodes them as the `WatchEvent` the player sends. Pure: the mock server
// holds a page's stream open and writes what this returns into it; specs
// push a message through ./watch.ts.
//
// The mirror of src/lib/grpc/watch.ts, which reads the events back into
// these shapes (watch-stream.test.ts holds the two to each other).

import { create, toBinary } from "@bufbuild/protobuf";
import {
  WatchEventSchema,
  type WatchEvent,
} from "../../src/gen/player/v1/player_pb";
import type {
  AmbientState,
  FadeStatus,
  HardwareStatusSnapshot,
  LogLine,
  NextUp,
  Playback,
  ShowReload,
  Status,
  SubsystemStatus,
  Waveform,
} from "../../src/gen/rules/mtrack_rules";
import { dataFrame } from "./stage-stream";

/** What a test pushes, as the stores' shapes say it. A playback, a status or
 *  a waveform may say only some of its fields; the rest are the proto's
 *  defaults. The mock's defaults and the shared helpers are checked against
 *  this. */
export type TypedWatchPush =
  | ({ type: "playback" } & Partial<Playback>)
  | { type: "logs"; lines: LogLine[] }
  | ({ type: "waveform" } & Partial<Waveform>)
  | { type: "config_changed" }
  | ({ type: "reload" } & ShowReload)
  | ({ type: "hardware" } & PartialStatus);

/** A status saying only some of its fields. */
export interface PartialStatus {
  build?: Partial<Status["build"]>;
  hardware?: Partial<HardwareStatusSnapshot>;
  controllers?: Status["controllers"];
  locked?: boolean;
}

/** The kinds of push. */
export type WatchType = TypedWatchPush["type"];

/** What a spec may push: a message of a kind, saying what it likes of its
 *  fields (a spec's own playback may name a track by its name alone). */
export interface WatchPush {
  type: WatchType;
  [field: string]: unknown;
}

/** A length in ms as a protobuf `Duration`. */
function duration(ms: number) {
  const whole = Math.floor(ms / 1000);
  return {
    seconds: BigInt(whole),
    nanos: Math.round((ms - whole * 1000) * 1_000_000),
  };
}

const durationOrUndefined = (ms: number | null | undefined) =>
  ms === null || ms === undefined ? undefined : duration(ms);

function fade(f: FadeStatus) {
  return {
    length: f.length,
    curve: f.curve,
    exclude: f.exclude ? { names: f.exclude } : undefined,
    lights: f.lights ?? undefined,
  };
}

/** The three states of a field `NextUp` may decide: absent, `null` (decided
 *  to have none) and a value. */
function next(n: Partial<NextUp>) {
  return {
    song: n.song ?? undefined,
    breaksLoop: n.breaks_loop,
    endsLoop: n.ends_loop,
    fadeOut:
      n.fade_out === undefined
        ? undefined
        : { fade: n.fade_out ? fade(n.fade_out) : undefined },
    fadeIn:
      n.fade_in === undefined
        ? undefined
        : { fade: n.fade_in ? fade(n.fade_in) : undefined },
    autoPlay:
      n.auto_play_ms === undefined
        ? undefined
        : { gapMs: n.auto_play_ms ?? undefined },
  };
}

function ambient(a: AmbientState) {
  if ("available" in a) {
    return { available: a.available, reason: a.reason };
  }
  return {
    mode: a.mode,
    state: a.state,
    show: a.show ?? undefined,
    source: a.source ?? undefined,
  };
}

function playback(p: Partial<Playback>) {
  return {
    isPlaying: p.is_playing,
    elapsed: duration(p.elapsed_ms ?? 0),
    songName: p.song_name,
    songDuration: duration(p.song_duration_ms ?? 0),
    playlistName: p.playlist_name,
    playlistPosition: p.playlist_position,
    playlistOffList: p.playlist_off_list,
    playlistAtFirst: p.playlist_at_first,
    playlistAtLast: p.playlist_at_last,
    playlistSongs: p.playlist_songs,
    tracks: p.tracks?.map((t) => ({
      name: t.name,
      outputChannels: t.output_channels,
      gainDb: t.gain_db,
      muted: t.muted,
    })),
    availablePlaylists: p.available_playlists,
    persistedPlaylistName: p.persisted_playlist_name,
    locked: p.locked,
    hardwareReloadStarted: BigInt(p.hardware_reload_started ?? 0),
    hardwareReloadRefused: p.hardware_reload_refused?.map((r) => ({
      generation: BigInt(r.generation),
      error: r.error,
    })),
    processId: p.process_id,
    beatGrid: p.beat_grid
      ? {
          grid: {
            beats: p.beat_grid.beats,
            measureStarts: p.beat_grid.measure_starts,
          },
          accentLevels: p.beat_grid.accent_levels
            ? Uint8Array.from(p.beat_grid.accent_levels)
            : undefined,
        }
      : undefined,
    availableSections: p.available_sections?.map((s) => ({
      name: s.name,
      startMeasure: s.start_measure,
      endMeasure: s.end_measure,
      startBeat: s.start_beat,
      endBeat: s.end_beat,
      color: s.color,
    })),
    activeSection: p.active_section
      ? {
          name: p.active_section.name,
          start: duration(p.active_section.start_ms),
          end: duration(p.active_section.end_ms),
        }
      : undefined,
    looping: p.looping,
    reactiveLoopState: p.reactive_loop_state
      ? {
          state: p.reactive_loop_state.state,
          sectionName:
            "section_name" in p.reactive_loop_state
              ? p.reactive_loop_state.section_name
              : undefined,
        }
      : undefined,
    pendingStart: durationOrUndefined(p.pending_start_ms),
    pilotHints: p.pilot_hints?.map((h) => ({
      label: h.label,
      at: duration(h.at_ms),
      start: duration(h.start_ms),
      end: duration(h.end_ms),
      hasAudio: h.has_audio,
    })),
    tempo: p.tempo
      ? {
          bpm: p.tempo.bpm,
          numerator: p.tempo.time_signature[0],
          denominator: p.tempo.time_signature[1],
        }
      : undefined,
    autoPlay: p.auto_play
      ? { song: p.auto_play.song, startsIn: duration(p.auto_play.starts_in_ms) }
      : undefined,
    ending: p.ending
      ? { song: p.ending.song, remaining: duration(p.ending.remaining_ms) }
      : undefined,
    ambient: p.ambient ? ambient(p.ambient) : undefined,
    next: p.next ? next(p.next) : undefined,
  };
}

function subsystem(s: SubsystemStatus | undefined) {
  return s
    ? { status: s.status, name: s.name ?? undefined, error: s.error }
    : undefined;
}

function hardware(h: Partial<HardwareStatusSnapshot> | undefined) {
  if (!h) return undefined;
  return {
    initDone: h.init_done,
    hostname: h.hostname ?? undefined,
    profile: h.profile ?? undefined,
    profileFile: h.profile_file ?? undefined,
    audio: subsystem(h.audio),
    audioOutput: h.audio_output
      ? {
          status: h.audio_output.status,
          callbackAlive: h.audio_output.callback_alive,
          writingSignal: h.audio_output.writing_signal,
          sinceLastCallback: durationOrUndefined(
            h.audio_output.since_last_callback_ms,
          ),
          sinceLastSignal: durationOrUndefined(
            h.audio_output.since_last_signal_ms,
          ),
          callbacks: BigInt(h.audio_output.callbacks),
          recoveries: BigInt(h.audio_output.recoveries),
          underruns: BigInt(h.audio_output.underruns),
          lastError: h.audio_output.last_error ?? undefined,
        }
      : undefined,
    midi: subsystem(h.midi),
    dmx: subsystem(h.dmx),
    trigger: subsystem(h.trigger),
    lightingVenue: h.lighting_venue
      ? {
          name: h.lighting_venue.name ?? undefined,
          status: h.lighting_venue.status,
          error: h.lighting_venue.error ?? undefined,
        }
      : undefined,
    testOutput: h.test_output
      ? {
          universe: h.test_output.universe,
          address: h.test_output.address,
          footprint: h.test_output.footprint,
          fixtureType: h.test_output.fixture_type,
          mode: h.test_output.mode ?? undefined,
          expiresInSecs: h.test_output.expires_in_secs,
        }
      : undefined,
  };
}

/** A push as the `WatchEvent` the player would send. */
export function toWatchEvent(pushed: WatchPush): WatchEvent {
  const push = pushed as TypedWatchPush;
  switch (push.type) {
    case "playback": {
      const { type: _type, ...p } = push;
      void _type;
      return create(WatchEventSchema, {
        event: { case: "playback", value: playback(p) },
      });
    }
    case "logs":
      return create(WatchEventSchema, {
        event: { case: "logs", value: { lines: push.lines } },
      });
    case "waveform":
      return create(WatchEventSchema, {
        event: {
          case: "waveform",
          value: { songName: push.song_name, tracks: push.tracks },
        },
      });
    case "config_changed":
      return create(WatchEventSchema, {
        event: { case: "configChanged", value: {} },
      });
    case "reload":
      return create(WatchEventSchema, {
        event: {
          case: "reload",
          value: { error: push.status === "error" ? push.error : undefined },
        },
      });
    case "hardware":
      return create(WatchEventSchema, {
        event: {
          case: "hardware",
          value: {
            build: push.build
              ? {
                  version: push.build.version,
                  gitHash: push.build.git_hash,
                  buildTime: push.build.build_time,
                  uiBuild: push.build.ui_build ?? undefined,
                }
              : undefined,
            hardware: hardware(push.hardware),
            controllers: push.controllers?.map((c) => ({
              kind: c.kind,
              status: c.status,
              detail: c.detail ?? undefined,
              error: c.error ?? undefined,
            })),
            locked: push.locked,
          },
        },
      });
  }
}

/** The gRPC-Web frame carrying a push. */
export function watchFrame(push: WatchPush): Buffer {
  return dataFrame(toBinary(WatchEventSchema, toWatchEvent(push)));
}
