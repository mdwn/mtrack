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
import { Code, ConnectError } from "@connectrpc/connect";
import { create } from "@bufbuild/protobuf";
import {
  GetPlaylistResponseSchema,
  ListPlaylistsResponseSchema,
  ListSongsResponseSchema,
  SavePlaylistRequestSchema,
} from "../../gen/player/v1/player_pb";
import { ApiError, ConflictError } from "../api/rest";
import {
  callError,
  mapPlaylistDetail,
  mapPlaylistList,
  mapSongList,
  minutesSeconds,
  savePlaylistRequest,
  wholeMillis,
} from "./library";

describe("the playlist list", () => {
  it("marks the active playlist and carries each one's count and reserved flag", () => {
    const list = mapPlaylistList(
      create(ListPlaylistsResponseSchema, {
        names: ["all_songs", "set"],
        active: "set",
        playlists: [
          { name: "all_songs", songCount: 10, reserved: true },
          { name: "set", songCount: 3 },
        ],
      }),
    );
    expect(list).toEqual([
      { name: "all_songs", song_count: 10, is_active: false, reserved: true },
      { name: "set", song_count: 3, is_active: true, reserved: false },
    ]);
  });
});

describe("a playlist read", () => {
  const response = create(GetPlaylistResponseSchema, {
    name: "set",
    entries: [
      {
        songName: "A",
        settings: {
          fadeOut: {
            length: "1measure",
            curve: "linear",
            exclude: { names: ["kick"] },
            lights: "0s",
          },
          fadeIn: { length: "2beats" },
          autoPlay: { gap: "" },
          ambient: { on: false },
        },
      },
      {
        songName: "B",
        settings: {
          autoPlay: { gap: "1500ms" },
          ambient: { on: true, show: "lighting/b.light", fadeIn: "6s" },
          fadeOut: { length: "2s", exclude: { names: [] } },
        },
      },
      { songName: "C", settings: {} },
    ],
    ambient: { on: true, show: "a.light", delay: "3s" },
    availableSongs: ["A", "B", "C", "D"],
    songTracks: { A: { names: ["kick", "snare"] }, D: { names: [] } },
  });

  it("maps each entry's settings into the editor's shape", () => {
    const detail = mapPlaylistDetail(response);
    expect(detail.name).toBe("set");
    expect(detail.songs).toEqual(["A", "B", "C"]);
    // True for at once, the gap, null for none.
    expect(detail.auto_play).toEqual([true, "1500ms", null]);
    expect(detail.fades).toEqual([
      {
        fade_out: {
          length: "1measure",
          curve: "linear",
          exclude: ["kick"],
          lights: "0s",
        },
        fade_in: "2beats",
      },
      // Present and empty is not absent: no track is left out.
      { fade_out: { length: "2s", exclude: [] }, fade_in: null },
      { fade_out: null, fade_in: null },
    ]);
    // `false` for none, settings, `null` for the level above's.
    expect(detail.entry_ambient).toEqual([
      false,
      { show: "lighting/b.light", fade_in: "6s" },
      null,
    ]);
    expect(detail.ambient).toEqual({ show: "a.light", delay: "3s" });
    expect(detail.available_songs).toEqual(["A", "B", "C", "D"]);
    expect(detail.song_tracks).toEqual({ A: ["kick", "snare"], D: [] });
  });

  it("maps a playlist with no ambient light and no library", () => {
    const detail = mapPlaylistDetail(
      create(GetPlaylistResponseSchema, {
        name: "bare",
        entries: [{ songName: "A", settings: {} }],
      }),
    );
    expect(detail.ambient).toBeNull();
    expect(detail.available_songs).toEqual([]);
    expect(detail.song_tracks).toEqual({});
  });
});

describe("a playlist save", () => {
  const build = (...args: Parameters<typeof savePlaylistRequest>) =>
    create(SavePlaylistRequestSchema, savePlaylistRequest(...args));

  it("sends only the songs, and says whether it creates", () => {
    const made = build("set", ["A"], {}, true);
    expect(made.name).toBe("set");
    expect(made.songs).toEqual(["A"]);
    expect(made.create).toBe(true);
    expect(made.setSettings).toBe(false);
    expect(made.setAmbient).toBe(false);
    // The editor does not ask for the changed-since-read check (it never
    // did): nothing is expected.
    expect(made.checkExpected).toBe(false);
    expect(build("set", ["A"], {}, false).create).toBe(false);
  });

  it("sends every entry's settings, one per song", () => {
    const req = build(
      "set",
      ["A", "B", "C"],
      {
        auto_play: [true, "2s", null],
        fades: [
          { fade_out: "4s", fade_in: null },
          {
            fade_out: { length: "1measure", curve: "linear", exclude: [] },
            fade_in: { length: "2beats" },
          },
          { fade_out: null, fade_in: null },
        ],
        entry_ambient: [false, { show: "x.light", delay: "1s" }, null],
        ambient: false,
      },
      false,
    );
    expect(req.setSettings).toBe(true);
    expect(req.settings).toHaveLength(3);
    expect(req.settings[0].autoPlay?.gap).toBe("");
    expect(req.settings[1].autoPlay?.gap).toBe("2s");
    expect(req.settings[2].autoPlay).toBeUndefined();
    expect(req.settings[0].fadeOut?.length).toBe("4s");
    expect(req.settings[0].fadeOut?.curve).toBeUndefined();
    expect(req.settings[0].fadeOut?.exclude).toBeUndefined();
    expect(req.settings[0].fadeIn).toBeUndefined();
    // Present and empty stays present.
    expect(req.settings[1].fadeOut?.exclude?.names).toEqual([]);
    expect(req.settings[1].fadeOut?.curve).toBe("linear");
    expect(req.settings[1].fadeIn?.length).toBe("2beats");
    expect(req.settings[0].ambient?.on).toBe(false);
    expect(req.settings[1].ambient).toMatchObject({
      on: true,
      show: "x.light",
      delay: "1s",
    });
    expect(req.settings[1].ambient?.fadeIn).toBeUndefined();
    expect(req.settings[2].ambient).toBeUndefined();
    expect(req.setAmbient).toBe(true);
    expect(req.ambient?.on).toBe(false);
  });

  it("keeps the playlist's ambient light when left out, and removes it for null", () => {
    expect(build("s", ["A"], {}, false).setAmbient).toBe(false);
    const removed = build("s", ["A"], { ambient: null }, false);
    expect(removed.setAmbient).toBe(true);
    expect(removed.ambient).toBeUndefined();
    const own = build("s", ["A"], { ambient: { show: "a.light" } }, false);
    expect(own.ambient).toMatchObject({ on: true, show: "a.light" });
  });

  it("refuses settings that do not go together", () => {
    expect(() =>
      savePlaylistRequest("s", ["A"], { auto_play: [null] }, false),
    ).toThrow(/together/);
  });
});

describe("the song list", () => {
  const response = create(ListSongsResponseSchema, {
    total: 3,
    songs: [
      {
        name: "Alpha",
        duration: { seconds: 185n, nanos: 999_000_000 },
        loops: true,
        hasLighting: true,
        hasMidi: true,
        baseDir: "Live/Alpha",
        channels: 2,
        sampleFormat: "int",
        tracks: ["click", "keys"],
        lightingFiles: ["Live/Alpha/show.light"],
        midiDmxFiles: ["Live/Alpha/dmx.mid"],
        beatGrid: { beats: [0.5, 1], measureStarts: [0] },
        hasTempoMap: true,
        loopCrossfadeMs: 3428n,
        loopCrossfadeCut: ["click"],
        sections: [
          { name: "verse", startMeasure: 1, endMeasure: 8 },
          {
            name: "chorus",
            startMeasure: 9,
            endMeasure: 16,
            startBeat: 2,
            endBeat: 3.5,
            color: "#5ecaea",
          },
        ],
      },
      { name: "Broken", failure: "gone.wav is missing", baseDir: "Broken" },
      { name: "Plain", duration: { seconds: 60n } },
    ],
  });

  it("maps a song into the shape the song pages hold", () => {
    const { songs } = mapSongList(response);
    expect(songs.map((s) => s.name)).toEqual(["Alpha", "Plain"]);
    expect(songs[0]).toEqual({
      name: "Alpha",
      duration_ms: 185_999,
      duration_display: "3:05",
      num_channels: 2,
      sample_format: "int",
      track_count: 2,
      tracks: ["click", "keys"],
      has_midi: true,
      has_lighting: true,
      base_dir: "Live/Alpha",
      lighting_files: ["Live/Alpha/show.light"],
      midi_dmx_files: ["Live/Alpha/dmx.mid"],
      beat_grid: { beats: [0.5, 1], measure_starts: [0] },
      has_tempo_map: true,
      loop_playback: true,
      loop_crossfade_ms: 3428,
      loop_crossfade_cut: ["click"],
      sections: [
        { name: "verse", start_measure: 1, end_measure: 8 },
        {
          name: "chorus",
          start_measure: 9,
          end_measure: 16,
          start_beat: 2,
          end_beat: 3.5,
          color: "#5ecaea",
        },
      ],
    });
  });

  it("maps what a song lacks to what the pages test for", () => {
    const plain = mapSongList(response).songs[1];
    // No directory is the top; no beat grid and no crossfade are null.
    expect(plain.base_dir).toBe("");
    expect(plain.beat_grid).toBeNull();
    expect(plain.loop_crossfade_ms).toBeNull();
    expect(plain.duration_display).toBe("1:00");
    expect(plain.track_count).toBe(0);
  });

  it("lists a song that failed to load apart from the songs", () => {
    expect(mapSongList(response).failures).toEqual([
      {
        name: "Broken",
        error: "gone.wav is missing",
        base_dir: "Broken",
        failed: true,
      },
    ]);
  });

  it("lists an empty library", () => {
    expect(mapSongList(create(ListSongsResponseSchema))).toEqual({
      songs: [],
      failures: [],
    });
  });
});

describe("lengths", () => {
  it("cuts milliseconds and seconds as the player does", () => {
    expect(wholeMillis(undefined)).toBe(0);
    expect(wholeMillis({ seconds: 1n, nanos: 500_999_999 } as never)).toBe(
      1500,
    );
    expect(minutesSeconds(0)).toBe("0:00");
    expect(minutesSeconds(59_999)).toBe("0:59");
    expect(minutesSeconds(61_000)).toBe("1:01");
    expect(minutesSeconds(3_600_000)).toBe("60:00");
  });
});

describe("a failed call's error", () => {
  it("shows the server's message for each code the editor meets", () => {
    for (const code of [
      Code.FailedPrecondition, // locked, or a file that does not read
      Code.InvalidArgument, // not valid
      Code.NotFound, // missing
      Code.AlreadyExists, // taken
      Code.Internal,
    ]) {
      const e = callError(new ConnectError("what happened", code), "Failed");
      expect(e).toBeInstanceOf(ApiError);
      expect(e.message).toBe("what happened");
    }
    expect(
      callError(
        new ConnectError("Player is locked. Unlock to make changes.", 9),
        "Failed",
      ).message,
    ).toBe("Player is locked. Unlock to make changes.");
  });

  it("is a conflict when the thing changed elsewhere", () => {
    const e = callError(
      new ConnectError("changed", Code.Aborted),
      "Failed to save",
    );
    expect(e).toBeInstanceOf(ConflictError);
    expect(e.message).toBe("changed");
  });

  it("carries the version a changed profile file is at, when the server says", () => {
    const at = callError(
      new ConnectError(
        "changed",
        Code.Aborted,
        new Headers({ "x-current-version": "v2" }),
      ),
      "Failed to save",
    );
    expect((at as ConflictError).version).toBe("v2");
    // A create onto a file already there has no version to say.
    expect(
      (
        callError(
          new ConnectError("exists", Code.Aborted),
          "Failed to save",
        ) as ConflictError
      ).version,
    ).toBeNull();
  });

  it("falls back to the code when the server says nothing", () => {
    expect(
      callError(new ConnectError("", Code.Unavailable), "Failed to save")
        .message,
    ).toBe("Failed to save: Unavailable");
  });

  it("passes an error that is not a failed call through", () => {
    const own = new TypeError("bug");
    expect(callError(own, "Failed")).toBe(own);
  });
});
