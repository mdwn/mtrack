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

// The playlists and the song library as the player answers them over gRPC,
// made from the shapes the web UI holds of them (`PlaylistInfo`,
// `PlaylistDetail`, `SongList`): the mirror of src/lib/grpc/library.ts, so a
// spec states a playlist or a song as the page shows it and answers the call
// with it:
//
//   const rpc = await mockRpc(page);
//   rpc.unary(PlayerService.method.getPlaylist, getPlaylistReply(detail));
//
// The mock server answers the same calls with the same helpers.
import type { MessageInitShape } from "@bufbuild/protobuf";
import type {
  GetPlaylistResponseSchema,
  ListPlaylistsResponseSchema,
  ListSongsResponseSchema,
} from "../../src/gen/player/v1/player_pb";
import type {
  Ambient,
  AutoPlayConfig,
  FadeConfig,
  PlaylistDetail,
  PlaylistInfo,
  SongList,
} from "../../src/gen/rules/mtrack_rules";

function duration(ms: number) {
  return { seconds: BigInt(Math.floor(ms / 1000)), nanos: (ms % 1000) * 1e6 };
}

export function listPlaylistsReply(
  playlists: PlaylistInfo[],
): MessageInitShape<typeof ListPlaylistsResponseSchema> {
  return {
    names: playlists.map((p) => p.name),
    active: playlists.find((p) => p.is_active)?.name ?? "",
    playlists: playlists.map((p) => ({
      name: p.name,
      songCount: p.song_count,
      reserved: p.reserved,
    })),
  };
}

function fade(f: FadeConfig | null | undefined) {
  if (f === null || f === undefined) return undefined;
  if (typeof f === "string") return { length: f };
  return {
    length: f.length,
    curve: f.curve,
    exclude: f.exclude ? { names: f.exclude } : undefined,
    lights: f.lights,
  };
}

function ambient(a: false | Ambient | null | undefined) {
  if (a === null || a === undefined) return undefined;
  if (a === false) return { on: false };
  return {
    on: true,
    show: a.show,
    delay: a.delay,
    fadeIn: a.fade_in,
    fadeOut: a.fade_out,
  };
}

function autoPlay(a: AutoPlayConfig | null | undefined) {
  if (a === null || a === undefined || a === false) return undefined;
  return { gap: a === true ? "" : a };
}

/** A playlist as a spec states it: the songs, and what else it says (the
 *  rest is as a playlist with no settings has it). */
export type PlaylistFixture = Pick<PlaylistDetail, "name" | "songs"> &
  Partial<PlaylistDetail>;

export function getPlaylistReply(
  detail: PlaylistFixture,
): MessageInitShape<typeof GetPlaylistResponseSchema> {
  return {
    name: detail.name,
    entries: detail.songs.map((songName, i) => ({
      songName,
      settings: {
        fadeOut: fade(detail.fades?.[i]?.fade_out),
        fadeIn: fade(detail.fades?.[i]?.fade_in),
        autoPlay: autoPlay(detail.auto_play?.[i]),
        ambient: ambient(detail.entry_ambient?.[i]),
        palette: detail.entry_palette?.[i]
          ? { slots: detail.entry_palette[i] }
          : undefined,
      },
    })),
    ambient: ambient(detail.ambient),
    availableSongs: detail.available_songs,
    songTracks: Object.fromEntries(
      Object.entries(detail.song_tracks ?? {}).map(([song, names]) => [
        song,
        { names },
      ]),
    ),
  };
}

export function listSongsReply(
  list: SongList,
): MessageInitShape<typeof ListSongsResponseSchema> {
  const songs = list.songs.map((s) => ({
    name: s.name,
    duration: duration(s.duration_ms),
    loops: s.loop_playback,
    hasLighting: s.has_lighting,
    hasMidi: s.has_midi,
    baseDir: s.base_dir,
    channels: s.num_channels,
    sampleFormat: s.sample_format,
    tracks: s.tracks,
    lightingFiles: s.lighting_files,
    midiDmxFiles: s.midi_dmx_files,
    beatGrid: s.beat_grid
      ? { beats: s.beat_grid.beats, measureStarts: s.beat_grid.measure_starts }
      : undefined,
    hasTempoMap: s.has_tempo_map,
    loopCrossfadeMs:
      s.loop_crossfade_ms !== null ? BigInt(s.loop_crossfade_ms) : undefined,
    loopCrossfadeCut: s.loop_crossfade_cut,
    sections: s.sections.map((section) => ({
      name: section.name,
      startMeasure: section.start_measure,
      endMeasure: section.end_measure,
      startBeat: section.start_beat,
      endBeat: section.end_beat,
      color: section.color,
    })),
  }));
  const failures = list.failures.map((f) => ({
    name: f.name,
    failure: f.error,
    baseDir: f.base_dir,
  }));
  return {
    songs: [...songs, ...failures],
    total: songs.length + failures.length,
  };
}
