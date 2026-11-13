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

// The spec helpers that answer the playlist and song calls from the shapes
// the page holds are the inverse of the page's own mapping: what they make,
// the page's mapping reads back as what was put in.
import { describe, expect, it } from "vitest";
import { create } from "@bufbuild/protobuf";
import {
  GetPlaylistResponseSchema,
  ListPlaylistsResponseSchema,
  ListSongsResponseSchema,
} from "../../src/gen/player/v1/player_pb";
import type {
  PlaylistDetail,
  SongList,
} from "../../src/gen/rules/mtrack_rules";
import {
  mapPlaylistDetail,
  mapPlaylistList,
  mapSongList,
} from "../../src/lib/grpc/library";
import {
  getPlaylistReply,
  listPlaylistsReply,
  listSongsReply,
} from "./library-rpc";

describe("answers made from the page's own shapes", () => {
  it("a playlist list reads back as it was", () => {
    const list = [
      { name: "all_songs", song_count: 4, is_active: false, reserved: true },
      { name: "set", song_count: 2, is_active: true, reserved: false },
    ];
    expect(
      mapPlaylistList(
        create(ListPlaylistsResponseSchema, listPlaylistsReply(list)),
      ),
    ).toEqual(list);
  });

  it("a playlist with every setting reads back as it was", () => {
    const detail: PlaylistDetail = {
      name: "set",
      songs: ["A", "B", "C"],
      auto_play: [true, "1500ms", null],
      fades: [
        {
          fade_out: {
            length: "1measure",
            curve: "linear",
            exclude: [],
            lights: "0s",
          },
          fade_in: "2beats",
        },
        { fade_out: null, fade_in: null },
        { fade_out: "3s", fade_in: { length: "1s", exclude: ["kick"] } },
      ],
      ambient: { show: "a.light", delay: "3s" },
      entry_ambient: [false, null, { show: "b.light", fade_in: "6s" }],
      available_songs: ["A", "B", "C", "D"],
      song_tracks: { A: ["kick", "snare"], D: [] },
    };
    expect(
      mapPlaylistDetail(
        create(GetPlaylistResponseSchema, getPlaylistReply(detail)),
      ),
    ).toEqual(detail);
    const bare: PlaylistDetail = { ...detail, ambient: null };
    expect(
      mapPlaylistDetail(
        create(GetPlaylistResponseSchema, getPlaylistReply(bare)),
      ).ambient,
    ).toBeNull();
  });

  it("a song list reads back as it was, failures apart", () => {
    const list: SongList = {
      songs: [
        {
          name: "Alpha",
          duration_ms: 185_250,
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
              color: "#5ecaea",
            },
          ],
        },
        {
          name: "Plain",
          duration_ms: 60_000,
          duration_display: "1:00",
          num_channels: 1,
          sample_format: "int",
          track_count: 0,
          tracks: [],
          has_midi: false,
          has_lighting: false,
          base_dir: "",
          lighting_files: [],
          midi_dmx_files: [],
          beat_grid: null,
          has_tempo_map: false,
          loop_playback: false,
          loop_crossfade_ms: null,
          loop_crossfade_cut: [],
          sections: [],
        },
      ],
      failures: [
        { name: "Broken", error: "gone.wav", base_dir: "Broken", failed: true },
      ],
    };
    expect(
      mapSongList(create(ListSongsResponseSchema, listSongsReply(list))),
    ).toEqual(list);
  });
});
