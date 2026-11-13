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
// What a spec pushes (the stores' own shapes) comes out of the web UI's
// reader as itself: the encoder specs push through and the reader the page
// maps with agree on every field.

import { describe, expect, it } from "vitest";
import { fromBinary, toBinary } from "@bufbuild/protobuf";
import { WatchEventSchema } from "../../src/gen/player/v1/player_pb";
import type { Playback, Status } from "../../src/gen/rules/mtrack_rules";
import { mapWatchEvent } from "../../src/lib/grpc/watch";
import {
  LOG_LINES,
  PLAYBACK_STATE,
  STATUS,
  WAVEFORM_DATA,
} from "../mock-server/test-data";
import { toWatchEvent, watchFrame, type WatchPush } from "./watch-stream";

/** A push, through the wire and back through the page's reader. */
function roundTrip(push: WatchPush) {
  const wire = toBinary(WatchEventSchema, toWatchEvent(push));
  return mapWatchEvent(fromBinary(WatchEventSchema, wire));
}

const { type: _playback, ...BASE } = PLAYBACK_STATE;
void _playback;

describe("a spec's push, read back by the page", () => {
  it("round-trips the mock's playback", () => {
    expect(roundTrip(PLAYBACK_STATE)).toStrictEqual({
      type: "playback",
      playback: BASE,
    });
  });

  it("round-trips a playback saying everything", () => {
    const full: Playback = {
      ...BASE,
      is_playing: true,
      elapsed_ms: 83250,
      hardware_reload_started: 12,
      hardware_reload_refused: [{ generation: 11, error: "no" }],
      beat_grid: {
        beats: [0.5, 1],
        measure_starts: [0],
        accent_levels: [3, 1],
      },
      available_sections: [
        {
          name: "verse",
          start_measure: 1,
          end_measure: 3,
          start_beat: 1.5,
          color: "#00ff00",
        },
      ],
      active_section: { name: "verse", start_ms: 1000, end_ms: 9000 },
      looping: true,
      reactive_loop_state: { state: "loop_armed", section_name: "verse" },
      pending_start_ms: 4000,
      pilot_hints: [
        { label: "c", at_ms: 1, start_ms: 2, end_ms: 3, has_audio: false },
      ],
      tempo: { bpm: 120.5, time_signature: [7, 8] },
      auto_play: { song: "Beta", starts_in_ms: 1500 },
      ending: { song: "Alpha", remaining_ms: 2500 },
      ambient: {
        mode: "auto",
        source: "playlist",
        show: "x.light",
        state: "pending",
      },
      next: {
        song: "Beta",
        breaks_loop: true,
        fade_out: {
          length: "2s",
          curve: "linear",
          exclude: ["a"],
          lights: null,
        },
        fade_in: null,
        auto_play_ms: null,
      },
    };
    expect(roundTrip({ type: "playback", ...full })).toStrictEqual({
      type: "playback",
      playback: full,
    });
  });

  it("keeps the three states of a next-up field, and ambient light that was not read", () => {
    for (const next of [
      null,
      { song: null },
      { song: "B" },
      { song: "B", fade_out: null, fade_in: null, auto_play_ms: null },
      { song: null, ends_loop: true },
    ]) {
      expect(
        roundTrip({ type: "playback", ...BASE, next }),
        JSON.stringify(next),
      ).toStrictEqual({ type: "playback", playback: { ...BASE, next } });
    }
    const ambient = { available: false, reason: "no answer" };
    expect(roundTrip({ type: "playback", ...BASE, ambient })).toStrictEqual({
      type: "playback",
      playback: { ...BASE, ambient },
    });
  });

  it("round-trips the waveform, the logs and the reloads", () => {
    expect(roundTrip(WAVEFORM_DATA)).toStrictEqual({
      type: "waveform",
      waveform: {
        song_name: WAVEFORM_DATA.song_name,
        // The f32s the player sends, as the browser reads them.
        tracks: WAVEFORM_DATA.tracks.map((t) => ({
          name: t.name,
          peaks: t.peaks.map(Math.fround),
        })),
      },
    });
    expect(roundTrip(LOG_LINES)).toStrictEqual({
      type: "logs",
      lines: LOG_LINES.lines,
    });
    expect(roundTrip({ type: "reload", status: "ok" })).toStrictEqual({
      type: "reload",
      reload: { status: "ok" },
    });
    expect(
      roundTrip({ type: "reload", status: "error", error: "bad" }),
    ).toStrictEqual({
      type: "reload",
      reload: { status: "error", error: "bad" },
    });
    expect(roundTrip({ type: "config_changed" })).toStrictEqual({
      type: "config_changed",
    });
  });

  it("round-trips the mock's status, and one with a fault and a test", () => {
    expect(roundTrip({ type: "hardware", ...STATUS })).toStrictEqual({
      type: "hardware",
      status: STATUS,
    });
    const faulty: Status = {
      ...STATUS,
      hardware: {
        ...STATUS.hardware,
        audio: { status: "failed", name: null, error: "could not be opened" },
        lighting_venue: { name: "v", status: "failed", error: "fixture x" },
        test_output: {
          universe: 1,
          address: 5,
          footprint: 4,
          fixture_type: "par",
          mode: null,
          expires_in_secs: 2.5,
        },
      },
      controllers: [
        { kind: "grpc", status: "failed", detail: null, error: "e" },
      ],
      locked: true,
    };
    expect(roundTrip({ type: "hardware", ...faulty })).toStrictEqual({
      type: "hardware",
      status: faulty,
    });
  });

  it("frames a push as one gRPC-Web data frame", () => {
    const frame = watchFrame({ type: "config_changed" });
    expect(frame[0]).toBe(0);
    expect(frame.readUInt32BE(1)).toBe(frame.length - 5);
  });
});
