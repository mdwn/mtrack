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
// The dashboard reads the player's gRPC stream `Watch` over gRPC-Web: one
// stream per tab, which says what the websocket and the status poll once
// did (playback, log lines, the waveform, reloads, and the build, hardware
// and controllers). Each event is mapped here into the shapes the stores
// hold (`Playback`, `LogLine`, `Waveform`, `ShowReload`, `Status`), so
// nothing above this module knows the wire is protobuf. The mapping is the
// mirror of the player's src/controller/grpc/watch.rs: a field the player
// leaves absent is absent here (or `null`, where the shape says so).

import type { Duration } from "@bufbuild/protobuf/wkt";
import {
  WatchTopic,
  type AmbientStatus,
  type AudioOutputHealth as AudioOutputHealthMessage,
  type Fade,
  type HardwareSnapshot,
  type HardwareStatus,
  type LoopState as LoopStateMessage,
  type NextUp as NextUpMessage,
  type Playback as PlaybackMessage,
  type SubsystemStatus as SubsystemStatusMessage,
  type WatchEvent,
} from "../../gen/player/v1/player_pb";
import type {
  AmbientMode,
  AmbientSource,
  AmbientState,
  AudioOutputHealth,
  FadeStatus,
  HardwareStatusSnapshot,
  LogLine,
  LoopState,
  NextUp,
  OutputStatus,
  Playback,
  ShowReload,
  Status,
  SubsystemStatus,
  Waveform,
} from "../../gen/rules/mtrack_rules";
import { BACKOFF_MS, runReconnecting, type Sleep } from "./reconnect";

/** What a `Watch` event says, in the stores' shapes. */
export type WatchMessage =
  | { type: "playback"; playback: Playback }
  | { type: "logs"; lines: LogLine[] }
  | { type: "waveform"; waveform: Waveform }
  | { type: "config_changed" }
  | { type: "reload"; reload: ShowReload }
  | { type: "hardware"; status: Status };

/** A length in milliseconds. The player sends whole milliseconds; an absent
 *  length is none. */
function ms(d: Duration | undefined): number {
  return d ? Number(d.seconds) * 1000 + d.nanos / 1_000_000 : 0;
}

function msOrNull(d: Duration | undefined): number | null {
  return d ? ms(d) : null;
}

function loopState(state: LoopStateMessage | undefined): LoopState {
  const section_name = state?.sectionName ?? "";
  switch (state?.state) {
    case "section_offered":
    case "loop_armed":
    case "looping":
    case "break_requested":
      return { state: state.state, section_name };
    default:
      return { state: "idle" };
  }
}

function fade(f: Fade): FadeStatus {
  return {
    length: f.length,
    curve: f.curve === "linear" ? "linear" : "equal_power",
    exclude: f.exclude ? [...f.exclude.names] : null,
    lights: f.lights ?? null,
  };
}

/** `NextUp` tells three states of a field apart: absent (the case does not
 *  decide it), present and empty (it decides there is none: `null`), and
 *  present. */
function nextUp(next: NextUpMessage): NextUp {
  const out: NextUp = { song: next.song ?? null };
  if (next.breaksLoop !== undefined) out.breaks_loop = next.breaksLoop;
  if (next.endsLoop !== undefined) out.ends_loop = next.endsLoop;
  if (next.fadeOut)
    out.fade_out = next.fadeOut.fade ? fade(next.fadeOut.fade) : null;
  if (next.fadeIn)
    out.fade_in = next.fadeIn.fade ? fade(next.fadeIn.fade) : null;
  if (next.autoPlay) out.auto_play_ms = next.autoPlay.gapMs ?? null;
  return out;
}

/** Ambient light that was read, or that could not be (`available` is only
 *  present then). */
function ambient(a: AmbientStatus): AmbientState {
  if (a.available !== undefined) {
    return { available: a.available, reason: a.reason ?? "" };
  }
  return {
    mode: a.mode as AmbientMode,
    source: (a.source ?? null) as AmbientSource | null,
    show: a.show ?? null,
    state: a.state,
  };
}

/** The playback message in the shape `playbackStore` holds. */
export function mapPlayback(p: PlaybackMessage): Playback {
  return {
    is_playing: p.isPlaying,
    elapsed_ms: ms(p.elapsed),
    song_name: p.songName,
    song_duration_ms: ms(p.songDuration),
    playlist_name: p.playlistName,
    playlist_position: p.playlistPosition,
    playlist_off_list: p.playlistOffList,
    playlist_at_first: p.playlistAtFirst,
    playlist_at_last: p.playlistAtLast,
    playlist_songs: [...p.playlistSongs],
    tracks: p.tracks.map((t) => ({
      name: t.name,
      output_channels: [...t.outputChannels],
      gain_db: t.gainDb,
      muted: t.muted,
    })),
    available_playlists: [...p.availablePlaylists],
    persisted_playlist_name: p.persistedPlaylistName,
    locked: p.locked,
    hardware_reload_started: Number(p.hardwareReloadStarted),
    hardware_reload_refused: p.hardwareReloadRefused.map((r) => ({
      generation: Number(r.generation),
      error: r.error,
    })),
    process_id: p.processId,
    beat_grid: p.beatGrid
      ? {
          beats: [...(p.beatGrid.grid?.beats ?? [])],
          measure_starts: [...(p.beatGrid.grid?.measureStarts ?? [])],
          ...(p.beatGrid.accentLevels
            ? { accent_levels: Array.from(p.beatGrid.accentLevels) }
            : {}),
        }
      : null,
    available_sections: p.availableSections.map((s) => ({
      name: s.name,
      start_measure: s.startMeasure,
      end_measure: s.endMeasure,
      ...(s.startBeat !== undefined ? { start_beat: s.startBeat } : {}),
      ...(s.endBeat !== undefined ? { end_beat: s.endBeat } : {}),
      ...(s.color !== undefined ? { color: s.color } : {}),
    })),
    active_section: p.activeSection
      ? {
          name: p.activeSection.name,
          start_ms: ms(p.activeSection.start),
          end_ms: ms(p.activeSection.end),
        }
      : null,
    looping: p.looping,
    reactive_loop_state: loopState(p.reactiveLoopState),
    pending_start_ms: msOrNull(p.pendingStart),
    pilot_hints: p.pilotHints.map((h) => ({
      label: h.label,
      at_ms: ms(h.at),
      start_ms: ms(h.start),
      end_ms: ms(h.end),
      has_audio: h.hasAudio,
    })),
    tempo: p.tempo
      ? {
          bpm: p.tempo.bpm,
          time_signature: [p.tempo.numerator, p.tempo.denominator],
        }
      : null,
    auto_play: p.autoPlay
      ? { song: p.autoPlay.song, starts_in_ms: ms(p.autoPlay.startsIn) }
      : null,
    ending: p.ending
      ? { song: p.ending.song, remaining_ms: ms(p.ending.remaining) }
      : null,
    ambient: p.ambient ? ambient(p.ambient) : null,
    next: p.next ? nextUp(p.next) : null,
  };
}

function subsystem(s: SubsystemStatusMessage | undefined): SubsystemStatus {
  return {
    status: s?.status ?? "not_connected",
    name: s?.name ?? null,
    ...(s?.error !== undefined ? { error: s.error } : {}),
  };
}

/** An audio output's health, as the status page holds it. */
export function audioOutput(h: AudioOutputHealthMessage): AudioOutputHealth {
  return {
    status: h.status as OutputStatus,
    callback_alive: h.callbackAlive,
    writing_signal: h.writingSignal,
    since_last_callback_ms: msOrNull(h.sinceLastCallback),
    since_last_signal_ms: msOrNull(h.sinceLastSignal),
    callbacks: Number(h.callbacks),
    recoveries: Number(h.recoveries),
    underruns: Number(h.underruns),
    last_error: h.lastError ?? null,
  };
}

function hardware(h: HardwareSnapshot | undefined): HardwareStatusSnapshot {
  return {
    init_done: h?.initDone ?? false,
    hostname: h?.hostname ?? null,
    profile: h?.profile ?? null,
    profile_file: h?.profileFile ?? null,
    audio: subsystem(h?.audio),
    audio_output: h?.audioOutput ? audioOutput(h.audioOutput) : null,
    midi: subsystem(h?.midi),
    dmx: subsystem(h?.dmx),
    trigger: subsystem(h?.trigger),
    lighting_venue: {
      name: h?.lightingVenue?.name ?? null,
      status: h?.lightingVenue?.status ?? "none",
      error: h?.lightingVenue?.error ?? null,
    },
    test_output: h?.testOutput
      ? {
          universe: h.testOutput.universe,
          address: h.testOutput.address,
          footprint: h.testOutput.footprint,
          fixture_type: h.testOutput.fixtureType,
          mode: h.testOutput.mode ?? null,
          expires_in_secs: h.testOutput.expiresInSecs,
        }
      : null,
  };
}

/** The hardware event in the shape `statusStore` holds. */
export function mapStatus(s: HardwareStatus): Status {
  return {
    build: {
      version: s.build?.version ?? "",
      git_hash: s.build?.gitHash ?? "",
      build_time: s.build?.buildTime ?? "",
      ui_build: s.build?.uiBuild ?? null,
    },
    hardware: hardware(s.hardware),
    controllers: s.controllers.map((c) => ({
      kind: c.kind,
      status: c.status,
      detail: c.detail ?? null,
      error: c.error ?? null,
    })),
    locked: s.locked,
  };
}

/** One `Watch` event in the stores' shapes; `null` for an event this page
 *  does not know (a newer player's new topic), which it ignores. */
export function mapWatchEvent(event: WatchEvent): WatchMessage | null {
  const e = event.event;
  switch (e.case) {
    case "playback":
      return { type: "playback", playback: mapPlayback(e.value) };
    case "logs":
      return {
        type: "logs",
        lines: e.value.lines.map((l) => ({
          level: l.level,
          target: l.target,
          message: l.message,
        })),
      };
    case "waveform":
      return {
        type: "waveform",
        waveform: {
          song_name: e.value.songName,
          tracks: e.value.tracks.map((t) => ({
            name: t.name,
            peaks: [...t.peaks],
          })),
        },
      };
    case "configChanged":
      return { type: "config_changed" };
    case "reload":
      return {
        type: "reload",
        reload:
          e.value.error === undefined
            ? { status: "ok" }
            : { status: "error", error: e.value.error },
      };
    case "hardware":
      return { type: "hardware", status: mapStatus(e.value) };
    default:
      return null;
  }
}

/** The topics the page asks for: every one it has a use for. (The player's
 *  `config_changed` is not asked for: nothing on the page reads it.) */
export const WATCH_TOPICS = [
  WatchTopic.PLAYBACK,
  WatchTopic.LOGS,
  WatchTopic.WAVEFORM,
  WatchTopic.RELOAD,
  WatchTopic.HARDWARE,
];

/** Where the stream's events and its state go. */
export interface WatchSink {
  message(message: WatchMessage): void;
  /** The stream has delivered: the player is there. */
  connected(): void;
  /** The stream ended or failed (`error` says how, `null` when it ended
   *  without one); the page reopens it after a backoff. */
  disconnected(error: unknown): void;
}

export interface WatchOptions {
  /** Opens the stream; ends or throws when it breaks. */
  open(signal: AbortSignal): AsyncIterable<WatchEvent>;
  sink: WatchSink;
  signal: AbortSignal;
  sleep?: Sleep;
  backoff?: number[];
}

/** Reads the stream into `sink` until aborted, reopening it with a short
 *  backoff when it ends or errors. The sink is `connected` once a
 *  connection has delivered its first event, and `disconnected` when it
 *  ends. What the page last held stays while reconnecting. */
export async function runWatch(o: WatchOptions): Promise<void> {
  await runReconnecting({
    session: async (signal, delivered) => {
      let live = false;
      try {
        for await (const event of o.open(signal)) {
          if (!live) {
            live = true;
            o.sink.connected();
          }
          delivered();
          const message = mapWatchEvent(event);
          if (message) o.sink.message(message);
        }
      } catch (error) {
        if (!signal.aborted) o.sink.disconnected(error);
        throw error;
      }
      if (!signal.aborted) o.sink.disconnected(null);
    },
    signal: o.signal,
    sleep: o.sleep,
    backoff: o.backoff ?? BACKOFF_MS,
  });
}

/** Opens the tab's `Watch` stream into `sink`; returns what closes it. A
 *  page that goes away closes it. */
export function openWatch(sink: WatchSink, win: Window = window): () => void {
  const abort = new AbortController();
  const onHide = () => abort.abort();
  win.addEventListener("pagehide", onHide, { once: true });
  void (async () => {
    const { playerClient } = await import("./client");
    await runWatch({
      open: (signal) =>
        playerClient.watch({ topics: WATCH_TOPICS }, { signal }),
      sink,
      signal: abort.signal,
    });
  })();
  return () => {
    win.removeEventListener("pagehide", onHide);
    abort.abort();
  };
}
