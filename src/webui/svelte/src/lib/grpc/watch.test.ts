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

import { describe, expect, it } from "vitest";
import { create, fromBinary, toBinary } from "@bufbuild/protobuf";
import {
  WatchEventSchema,
  WaveformSchema,
  type WatchEvent,
} from "../../gen/player/v1/player_pb";
import { STAGE_BACKOFF } from "../stage/grpc-feed";
import { BACKOFF_MS } from "./reconnect";
import {
  mapPlayback,
  mapWatchEvent,
  runWatch,
  WATCH_TOPICS,
  type WatchMessage,
  type WatchSink,
} from "./watch";

const dur = (seconds: number, nanos = 0) => ({
  seconds: BigInt(seconds),
  nanos,
});

/** A playback event of the proto messages, saying `fields` on top of the
 *  proto's defaults. */
function playback(fields: Record<string, unknown> = {}): WatchEvent {
  return create(WatchEventSchema, {
    event: { case: "playback", value: fields as never },
  });
}

function mapped(event: WatchEvent): WatchMessage {
  const message = mapWatchEvent(event);
  if (!message) throw new Error("not mapped");
  return message;
}

function playbackOf(fields: Record<string, unknown> = {}) {
  const message = mapped(playback(fields));
  if (message.type !== "playback") throw new Error("not a playback");
  return message.playback;
}

describe("a playback event, in the shape the store holds", () => {
  it("maps every field", () => {
    const got = playbackOf({
      isPlaying: true,
      elapsed: dur(65, 250_000_000),
      songName: "Alpha",
      songDuration: dur(180),
      playlistName: "setlist",
      playlistPosition: 3,
      playlistOffList: true,
      playlistAtFirst: true,
      playlistAtLast: true,
      playlistSongs: ["Alpha", "Beta"],
      tracks: [
        { name: "kick", outputChannels: [0, 1], gainDb: -6.5, muted: true },
        { name: "bass", outputChannels: [], gainDb: 0, muted: false },
      ],
      availablePlaylists: ["all_songs", "setlist"],
      persistedPlaylistName: "setlist",
      locked: true,
      hardwareReloadStarted: 7n,
      hardwareReloadRefused: [{ generation: 6n, error: "device busy" }],
      processId: "18446744073709551615",
      beatGrid: {
        grid: { beats: [0.5, 1, 1.5], measureStarts: [0, 2] },
        accentLevels: new Uint8Array([3, 1, 2, 0]),
      },
      availableSections: [
        {
          name: "verse",
          startMeasure: 1,
          endMeasure: 5,
          startBeat: 2.5,
          endBeat: 1,
          color: "#ff0000",
        },
        { name: "chorus", startMeasure: 5, endMeasure: 9 },
      ],
      activeSection: {
        name: "verse",
        start: dur(4),
        end: dur(20, 500_000_000),
      },
      looping: true,
      reactiveLoopState: { state: "looping", sectionName: "verse" },
      pendingStart: dur(12),
      pilotHints: [
        {
          label: "cue",
          at: dur(1),
          start: dur(2),
          end: dur(3, 5_000_000),
          hasAudio: true,
        },
      ],
      tempo: { bpm: 127.5, numerator: 6, denominator: 8 },
      autoPlay: { song: "Beta", startsIn: dur(2) },
      ending: { song: "Alpha", remaining: dur(0, 750_000_000) },
      ambient: {
        mode: "on",
        state: "playing",
        show: "a.light",
        source: "entry",
      },
      next: { song: "Beta" },
    });
    expect(got).toStrictEqual({
      is_playing: true,
      elapsed_ms: 65250,
      song_name: "Alpha",
      song_duration_ms: 180000,
      playlist_name: "setlist",
      playlist_position: 3,
      playlist_off_list: true,
      playlist_at_first: true,
      playlist_at_last: true,
      playlist_songs: ["Alpha", "Beta"],
      tracks: [
        { name: "kick", output_channels: [0, 1], gain_db: -6.5, muted: true },
        { name: "bass", output_channels: [], gain_db: 0, muted: false },
      ],
      available_playlists: ["all_songs", "setlist"],
      persisted_playlist_name: "setlist",
      locked: true,
      hardware_reload_started: 7,
      hardware_reload_refused: [{ generation: 6, error: "device busy" }],
      process_id: "18446744073709551615",
      beat_grid: {
        beats: [0.5, 1, 1.5],
        measure_starts: [0, 2],
        accent_levels: [3, 1, 2, 0],
      },
      available_sections: [
        {
          name: "verse",
          start_measure: 1,
          end_measure: 5,
          start_beat: 2.5,
          end_beat: 1,
          color: "#ff0000",
        },
        { name: "chorus", start_measure: 5, end_measure: 9 },
      ],
      active_section: { name: "verse", start_ms: 4000, end_ms: 20500 },
      looping: true,
      reactive_loop_state: { state: "looping", section_name: "verse" },
      pending_start_ms: 12000,
      pilot_hints: [
        {
          label: "cue",
          at_ms: 1000,
          start_ms: 2000,
          end_ms: 3005,
          has_audio: true,
        },
      ],
      tempo: { bpm: 127.5, time_signature: [6, 8] },
      auto_play: { song: "Beta", starts_in_ms: 2000 },
      ending: { song: "Alpha", remaining_ms: 750 },
      ambient: {
        mode: "on",
        state: "playing",
        show: "a.light",
        source: "entry",
      },
      next: { song: "Beta" },
    });
  });

  it("maps what the player leaves out to what the shape says: null, empty, or absent", () => {
    expect(playbackOf()).toStrictEqual({
      is_playing: false,
      elapsed_ms: 0,
      song_name: "",
      song_duration_ms: 0,
      playlist_name: "",
      playlist_position: 0,
      playlist_off_list: false,
      playlist_at_first: false,
      playlist_at_last: false,
      playlist_songs: [],
      tracks: [],
      available_playlists: [],
      persisted_playlist_name: "",
      locked: false,
      hardware_reload_started: 0,
      hardware_reload_refused: [],
      process_id: "",
      beat_grid: null,
      available_sections: [],
      active_section: null,
      looping: false,
      reactive_loop_state: { state: "idle" },
      pending_start_ms: null,
      pilot_hints: [],
      tempo: null,
      auto_play: null,
      ending: null,
      ambient: null,
      next: null,
    });
  });

  it("turns Durations into milliseconds, from seconds and nanos", () => {
    const ms = (d: ReturnType<typeof dur> | undefined) =>
      playbackOf({ elapsed: d }).elapsed_ms;
    expect(ms(undefined)).toBe(0);
    expect(ms(dur(0))).toBe(0);
    expect(ms(dur(0, 1_000_000))).toBe(1);
    expect(ms(dur(1, 999_000_000))).toBe(1999);
    expect(ms(dur(3 * 3600, 123_000_000))).toBe(10_800_123);
  });

  it("keeps a pending start apart from none, even at zero", () => {
    expect(playbackOf({ pendingStart: dur(0) }).pending_start_ms).toBe(0);
    expect(playbackOf({}).pending_start_ms).toBeNull();
  });

  it("says a section's optional boundaries only when the player did", () => {
    const [bare] = playbackOf({
      availableSections: [{ name: "a", startMeasure: 1, endMeasure: 2 }],
    }).available_sections;
    expect(Object.keys(bare).sort()).toEqual([
      "end_measure",
      "name",
      "start_measure",
    ]);
    const [beat] = playbackOf({
      availableSections: [
        { name: "a", startMeasure: 1, endMeasure: 2, startBeat: 1 },
      ],
    }).available_sections;
    expect(beat.start_beat).toBe(1);
    expect("end_beat" in beat).toBe(false);
  });

  it("maps the beat grid, with accent levels only when the song has a metronome", () => {
    const grid = { beats: [1], measureStarts: [0] };
    expect(playbackOf({ beatGrid: { grid } }).beat_grid).toStrictEqual({
      beats: [1],
      measure_starts: [0],
    });
    expect(
      playbackOf({ beatGrid: { grid, accentLevels: new Uint8Array([]) } })
        .beat_grid,
    ).toStrictEqual({ beats: [1], measure_starts: [0], accent_levels: [] });
  });

  it("maps every loop state, with its section", () => {
    for (const state of [
      "section_offered",
      "loop_armed",
      "looping",
      "break_requested",
    ] as const) {
      expect(
        playbackOf({ reactiveLoopState: { state, sectionName: "chorus" } })
          .reactive_loop_state,
      ).toStrictEqual({ state, section_name: "chorus" });
    }
    expect(
      playbackOf({ reactiveLoopState: { state: "idle" } }).reactive_loop_state,
    ).toStrictEqual({ state: "idle" });
  });

  it("tells ambient light that was read from ambient light that could not be", () => {
    expect(
      playbackOf({ ambient: { mode: "auto", state: "off" } }).ambient,
    ).toStrictEqual({ mode: "auto", source: null, show: null, state: "off" });
    expect(
      playbackOf({
        ambient: { mode: "off", state: "off", source: "profile", show: "x" },
      }).ambient,
    ).toStrictEqual({
      mode: "off",
      source: "profile",
      show: "x",
      state: "off",
    });
    expect(
      playbackOf({ ambient: { available: false, reason: "timed out" } })
        .ambient,
    ).toStrictEqual({ available: false, reason: "timed out" });
  });
});

describe("what comes next, in its three states of a field", () => {
  const next = (fields: Record<string, unknown>) =>
    playbackOf({ next: fields }).next;

  it("is null with no current song, and has a null song at the end of the playlist", () => {
    expect(playbackOf().next).toBeNull();
    expect(next({})).toStrictEqual({ song: null });
  });

  it("leaves a field the case does not decide absent", () => {
    expect(next({ song: "Beta" })).toStrictEqual({ song: "Beta" });
  });

  it("says a field the case decides to have no value as null", () => {
    expect(
      next({ song: "Beta", fadeOut: {}, fadeIn: {}, autoPlay: {} }),
    ).toStrictEqual({
      song: "Beta",
      fade_out: null,
      fade_in: null,
      auto_play_ms: null,
    });
  });

  it("maps the flags, the fades and the gap when they are there", () => {
    expect(
      next({
        song: "Beta",
        breaksLoop: false,
        endsLoop: true,
        fadeOut: { fade: { length: "2s", curve: "linear", lights: "1s" } },
        fadeIn: {
          fade: { length: "500ms", exclude: { names: [] } },
        },
        autoPlay: { gapMs: 1500.5 },
      }),
    ).toStrictEqual({
      song: "Beta",
      breaks_loop: false,
      ends_loop: true,
      fade_out: {
        length: "2s",
        curve: "linear",
        exclude: null,
        lights: "1s",
      },
      fade_in: {
        length: "500ms",
        curve: "equal_power",
        exclude: [],
        lights: null,
      },
      auto_play_ms: 1500.5,
    });
    expect(
      next({
        fadeOut: { fade: { length: "1s", exclude: { names: ["a", "b"] } } },
      })?.fade_out?.exclude,
    ).toEqual(["a", "b"]);
  });
});

describe("the other events", () => {
  it("maps log lines", () => {
    expect(
      mapped(
        create(WatchEventSchema, {
          event: {
            case: "logs",
            value: {
              lines: [
                { level: "INFO", target: "t", message: "m" },
                { level: "ERROR", target: "", message: "" },
              ],
            },
          },
        }),
      ),
    ).toStrictEqual({
      type: "logs",
      lines: [
        { level: "INFO", target: "t", message: "m" },
        { level: "ERROR", target: "", message: "" },
      ],
    });
  });

  it("maps a waveform, its peaks the f32s the player sent", () => {
    // Through the wire, where a float is narrowed to 32 bits.
    const event = fromBinary(
      WatchEventSchema,
      toBinary(
        WatchEventSchema,
        create(WatchEventSchema, {
          event: {
            case: "waveform",
            value: create(WaveformSchema, {
              songName: "S",
              tracks: [
                { name: "kick", peaks: [0.5, 0.8, 0.3, 0] },
                { name: "empty", peaks: [] },
              ],
            }),
          },
        }),
      ),
    );
    expect(mapped(event)).toStrictEqual({
      type: "waveform",
      waveform: {
        song_name: "S",
        tracks: [
          {
            name: "kick",
            peaks: [0.5, Math.fround(0.8), Math.fround(0.3), 0],
          },
          { name: "empty", peaks: [] },
        ],
      },
    });
    // The widened value is what the JSON carried.
    expect(Math.fround(0.8)).toBe(0.800000011920929);
  });

  it("maps a reload, ok or failed, and a config change", () => {
    const reload = (value: { error?: string }) =>
      mapped(create(WatchEventSchema, { event: { case: "reload", value } }));
    expect(reload({})).toStrictEqual({
      type: "reload",
      reload: { status: "ok" },
    });
    expect(reload({ error: "boom" })).toStrictEqual({
      type: "reload",
      reload: { status: "error", error: "boom" },
    });
    expect(reload({ error: "" })).toStrictEqual({
      type: "reload",
      reload: { status: "error", error: "" },
    });
    expect(
      mapped(
        create(WatchEventSchema, {
          event: { case: "configChanged", value: {} },
        }),
      ),
    ).toStrictEqual({ type: "config_changed" });
  });

  it("ignores an event it does not know", () => {
    expect(mapWatchEvent(create(WatchEventSchema))).toBeNull();
  });
});

describe("a hardware event, in the shape the status store holds", () => {
  const hardware = (value: Record<string, unknown>) => {
    const message = mapped(
      create(WatchEventSchema, { event: { case: "hardware", value } as never }),
    );
    if (message.type !== "hardware") throw new Error("not a status");
    return message.status;
  };

  it("maps every field", () => {
    expect(
      hardware({
        build: {
          version: "1.2.3",
          gitHash: "abc",
          buildTime: "2026-01-01",
          uiBuild: "ui1",
        },
        hardware: {
          initDone: true,
          hostname: "rig",
          profile: "rig",
          profileFile: "rig.yml",
          audio: { status: "connected", name: "USB" },
          audioOutput: {
            status: "recovering",
            callbackAlive: true,
            writingSignal: true,
            sinceLastCallback: dur(0, 12_000_000),
            sinceLastSignal: dur(2),
            callbacks: 4821n,
            recoveries: 2n,
            underruns: 3n,
            lastError: "xrun",
          },
          midi: { status: "failed", error: "gone" },
          dmx: { status: "initializing", name: "olad" },
          trigger: { status: "not_connected" },
          lightingVenue: { name: "club", status: "failed", error: "bad" },
          testOutput: {
            universe: 1,
            address: 17,
            footprint: 8,
            fixtureType: "par",
            mode: "8ch",
            expiresInSecs: 1.25,
          },
        },
        controllers: [
          { kind: "osc", status: "running", detail: "0.0.0.0:9000" },
          { kind: "midi", status: "failed", error: "no port" },
        ],
        locked: true,
      }),
    ).toStrictEqual({
      build: {
        version: "1.2.3",
        git_hash: "abc",
        build_time: "2026-01-01",
        ui_build: "ui1",
      },
      hardware: {
        init_done: true,
        hostname: "rig",
        profile: "rig",
        profile_file: "rig.yml",
        audio: { status: "connected", name: "USB" },
        audio_output: {
          status: "recovering",
          callback_alive: true,
          writing_signal: true,
          since_last_callback_ms: 12,
          since_last_signal_ms: 2000,
          callbacks: 4821,
          recoveries: 2,
          underruns: 3,
          last_error: "xrun",
        },
        midi: { status: "failed", name: null, error: "gone" },
        dmx: { status: "initializing", name: "olad" },
        trigger: { status: "not_connected", name: null },
        lighting_venue: { name: "club", status: "failed", error: "bad" },
        test_output: {
          universe: 1,
          address: 17,
          footprint: 8,
          fixture_type: "par",
          mode: "8ch",
          expires_in_secs: 1.25,
        },
      },
      controllers: [
        {
          kind: "osc",
          status: "running",
          detail: "0.0.0.0:9000",
          error: null,
        },
        { kind: "midi", status: "failed", detail: null, error: "no port" },
      ],
      locked: true,
    });
  });

  it("maps what the player leaves out to null", () => {
    expect(
      hardware({
        hardware: {
          audio: { status: "not_connected" },
          midi: { status: "not_connected" },
          dmx: { status: "not_connected" },
          trigger: { status: "not_connected" },
          lightingVenue: { status: "none" },
        },
      }),
    ).toStrictEqual({
      build: { version: "", git_hash: "", build_time: "", ui_build: null },
      hardware: {
        init_done: false,
        hostname: null,
        profile: null,
        profile_file: null,
        audio: { status: "not_connected", name: null },
        audio_output: null,
        midi: { status: "not_connected", name: null },
        dmx: { status: "not_connected", name: null },
        trigger: { status: "not_connected", name: null },
        lighting_venue: { name: null, status: "none", error: null },
        test_output: null,
      },
      controllers: [],
      locked: false,
    });
  });

  it("keeps an audio output's absent timings null", () => {
    const status = hardware({
      hardware: {
        audioOutput: {
          status: "never_started",
          callbackAlive: false,
          writingSignal: false,
        },
      },
    });
    expect(status.hardware.audio_output).toStrictEqual({
      status: "never_started",
      callback_alive: false,
      writing_signal: false,
      since_last_callback_ms: null,
      since_last_signal_ms: null,
      callbacks: 0,
      recoveries: 0,
      underruns: 0,
      last_error: null,
    });
  });
});

describe("the topics asked for", () => {
  it("leave out only the config change nothing reads", () => {
    expect(WATCH_TOPICS).toHaveLength(5);
    expect(new Set(WATCH_TOPICS).size).toBe(5);
  });

  it("map a playback message directly too", () => {
    expect(
      mapPlayback(
        create(WatchEventSchema, {
          event: { case: "playback", value: { songName: "x" } },
        }).event.value as never,
      ).song_name,
    ).toBe("x");
  });
});

describe("the stream", () => {
  /** A sink that writes down what happened to it. */
  function sink() {
    const log: string[] = [];
    const s: WatchSink = {
      message: (m) => log.push(m.type),
      connected: () => log.push("connected"),
      disconnected: (e) =>
        log.push(e === null ? "ended" : `failed:${(e as Error).message}`),
    };
    return { log, s };
  }
  const logs = create(WatchEventSchema, {
    event: { case: "logs", value: { lines: [] } },
  });

  it("is connected once it has delivered, and disconnected when it ends, then reopens after the shared backoff", async () => {
    const abort = new AbortController();
    const { log, s } = sink();
    const sleeps: number[] = [];
    let opens = 0;
    await runWatch({
      open: async function* () {
        opens++;
        if (opens === 1) {
          // Nothing said yet: not connected.
          log.push("opened");
          yield logs;
          yield playback();
        } else {
          abort.abort();
        }
      },
      sink: s,
      signal: abort.signal,
      sleep: async (ms) => void sleeps.push(ms),
    });
    expect(log).toEqual(["opened", "connected", "logs", "playback", "ended"]);
    // A connection that delivered starts the backoff over.
    expect(sleeps).toEqual([BACKOFF_MS[0]]);
  });

  it("is disconnected, with why, when it fails, and backs off further for each failure without delivering", async () => {
    const abort = new AbortController();
    const { log, s } = sink();
    const sleeps: number[] = [];
    let opens = 0;
    await runWatch({
      open: async function* () {
        opens++;
        if (opens > 6) {
          abort.abort();
          return;
        }
        throw new Error(`down ${opens}`);
        yield logs;
      },
      sink: s,
      signal: abort.signal,
      sleep: async (ms) => void sleeps.push(ms),
    });
    expect(log).toEqual([
      "failed:down 1",
      "failed:down 2",
      "failed:down 3",
      "failed:down 4",
      "failed:down 5",
      "failed:down 6",
    ]);
    expect(sleeps).toEqual([250, 500, 1000, 2000, 2000, 2000]);
  });

  it("says nothing of an abort, and shares the stage stream's backoff", async () => {
    const abort = new AbortController();
    const { log, s } = sink();
    await runWatch({
      open: async function* () {
        yield logs;
        abort.abort();
        throw new Error("aborted");
      },
      sink: s,
      signal: abort.signal,
    });
    expect(log).toEqual(["connected", "logs"]);
    expect(STAGE_BACKOFF).toBe(BACKOFF_MS);
  });
});
