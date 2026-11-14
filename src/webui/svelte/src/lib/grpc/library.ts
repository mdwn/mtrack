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

// The playlists and the song library over gRPC: the calls the playlist
// editor and the song pages make (`ListPlaylists`, `GetPlaylist`,
// `SavePlaylist`, `DeletePlaylist`, `SwitchToPlaylist`, `ListSongs`,
// `SetLock`), mapped into the shapes the components hold (`PlaylistInfo`,
// `PlaylistDetail`, `SongList`), the way watch.ts maps the live feeds. This
// module is only the mapping and the error table, so a unit test can run it
// without a page; the calls are in src/lib/api.

import { Code, ConnectError } from "@connectrpc/connect";
import type { Duration } from "@bufbuild/protobuf/wkt";
import { ApiError, ConflictError } from "../api/rest";
import type { MessageInitShape } from "@bufbuild/protobuf";
import type {
  AmbientSetting,
  Fade,
  GetPlaylistResponse,
  ListPlaylistsResponse,
  ListSongsResponse,
  SavePlaylistRequestSchema,
  SongListing,
} from "../../gen/player/v1/player_pb";
import type {
  Ambient,
  AutoPlayConfig,
  EntryFades,
  FadeConfig,
  PlaylistBody,
  PlaylistDetail,
  PlaylistInfo,
  SongFailure,
  SongInfo,
  SongList,
} from "../../gen/rules/mtrack_rules";

// ---- Errors ----

/** The metadata key of an `ABORTED` save of a profile file that says the
 *  version the file is at (src/controller/grpc/config_service.rs). */
const CURRENT_VERSION_KEY = "x-current-version";

/** The error a failed call shows, as the REST routes' errors were: the
 *  server's own message, whatever the code (a locked player is
 *  `FAILED_PRECONDITION` with "Player is locked...", an invalid playlist
 *  `INVALID_ARGUMENT` with what is wrong with it, a missing one `NOT_FOUND`,
 *  one that exists `ALREADY_EXISTS`, one changed elsewhere `ABORTED`, which is
 *  a `ConflictError` as a stale version was, carrying the version the file is
 *  at when the server says it); with
 *  none, `fallback` and the code. Anything that is not a failed call (an
 *  error in the page's own code) passes through. */
export function callError(e: unknown, fallback: string): Error {
  const err = ConnectError.from(e);
  if (!(e instanceof ConnectError)) return e instanceof Error ? e : err;
  const message = err.rawMessage || `${fallback}: ${Code[err.code]}`;
  return err.code === Code.Aborted
    ? new ConflictError(message, err.metadata.get(CURRENT_VERSION_KEY))
    : new ApiError(message);
}

// ---- Lengths ----

/** A length in whole milliseconds, cut as the player cuts one. */
export function wholeMillis(d: Duration | undefined): number {
  return d ? Number(d.seconds) * 1000 + Math.floor(d.nanos / 1_000_000) : 0;
}

/** `m:ss`, as the player writes a length (src/position.rs). */
export function minutesSeconds(ms: number): string {
  const secs = Math.floor(ms / 1000);
  const s = secs % 60;
  return `${Math.floor(secs / 60)}:${s < 10 ? "0" : ""}${s}`;
}

// ---- Playlists ----

/** The playlist list the editor shows. */
export function mapPlaylistList(r: ListPlaylistsResponse): PlaylistInfo[] {
  return r.playlists.map((p) => ({
    name: p.name,
    song_count: p.songCount,
    is_active: p.name === r.active,
    reserved: p.reserved,
  }));
}

function ambientConfig(a: AmbientSetting): false | Ambient {
  if (!a.on) return false;
  const ambient: Ambient = {};
  if (a.show !== undefined) ambient.show = a.show;
  if (a.delay !== undefined) ambient.delay = a.delay;
  if (a.fadeIn !== undefined) ambient.fade_in = a.fadeIn;
  if (a.fadeOut !== undefined) ambient.fade_out = a.fadeOut;
  return ambient;
}

/** A fade as a playlist file writes it: its length alone when it has no
 *  other setting. */
function fadeConfig(f: Fade | undefined): FadeConfig | null {
  if (!f) return null;
  if (f.curve === undefined && !f.exclude && f.lights === undefined) {
    return f.length;
  }
  const fade: Exclude<FadeConfig, string> = { length: f.length };
  if (f.curve !== undefined) fade.curve = f.curve;
  if (f.exclude) fade.exclude = [...f.exclude.names];
  if (f.lights !== undefined) fade.lights = f.lights;
  return fade;
}

/** One entry's auto play: `true` for at once (an empty gap), the gap, or
 *  `null` for none. */
function autoPlayConfig(a: { gap: string } | undefined): AutoPlayConfig | null {
  if (!a) return null;
  return a.gap === "" ? true : a.gap;
}

/** The playlist the editor edits. The library (`availableSongs`,
 *  `songTracks`) is there when the read asked for it. */
export function mapPlaylistDetail(r: GetPlaylistResponse): PlaylistDetail {
  const settings = r.entries.map((e) => e.settings);
  return {
    name: r.name,
    songs: r.entries.map((e) => e.songName),
    auto_play: settings.map((s) => autoPlayConfig(s?.autoPlay)),
    fades: settings.map(
      (s): EntryFades => ({
        fade_out: fadeConfig(s?.fadeOut),
        fade_in: fadeConfig(s?.fadeIn),
      }),
    ),
    ambient: r.ambient ? ambientConfig(r.ambient) : null,
    entry_ambient: settings.map((s) =>
      s?.ambient ? ambientConfig(s.ambient) : null,
    ),
    available_songs: [...r.availableSongs],
    song_tracks: Object.fromEntries(
      Object.entries(r.songTracks).map(([song, t]) => [song, [...t.names]]),
    ),
  };
}

function fadeMessage(f: FadeConfig | null | undefined) {
  if (f === null || f === undefined) return undefined;
  if (typeof f === "string") return { length: f };
  return {
    length: f.length,
    curve: f.curve,
    exclude: f.exclude ? { names: [...f.exclude] } : undefined,
    lights: f.lights,
  };
}

function ambientMessage(a: false | Ambient | null | undefined) {
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

/** What a save sets beyond the songs: each setting the editor leaves out
 *  keeps what the playlist has. */
export type PlaylistSettings = Omit<PlaylistBody, "songs">;

/** The save request for `songs` and `settings`. `create` makes a new
 *  playlist (and refuses one that exists); without it the playlist must
 *  exist. The entries' settings go together (the editor sends them all, as
 *  one per song), the playlist's own ambient light on its own. */
export function savePlaylistRequest(
  name: string,
  songs: string[],
  settings: PlaylistSettings,
  create: boolean,
): MessageInitShape<typeof SavePlaylistRequestSchema> {
  const request: MessageInitShape<typeof SavePlaylistRequestSchema> = {
    name,
    songs,
    create,
  };
  const { auto_play, fades, entry_ambient } = settings;
  if (auto_play || fades || entry_ambient) {
    if (!auto_play || !fades || !entry_ambient) {
      throw new Error(
        "a playlist save sets auto play, fades and entry ambient together",
      );
    }
    request.setSettings = true;
    request.settings = songs.map((_, i) => ({
      fadeOut: fadeMessage(fades[i]?.fade_out),
      fadeIn: fadeMessage(fades[i]?.fade_in),
      autoPlay: autoPlayMessage(auto_play[i]),
      ambient: ambientMessage(entry_ambient[i]),
    }));
  }
  // Absent keeps the playlist's ambient light; `null` removes it.
  if (settings.ambient !== undefined) {
    request.setAmbient = true;
    request.ambient = ambientMessage(settings.ambient);
  }
  return request;
}

function autoPlayMessage(a: AutoPlayConfig | null | undefined) {
  if (a === null || a === undefined || a === false) return undefined;
  return { gap: a === true ? "" : a };
}

// ---- Songs ----

function songInfo(s: SongListing): SongInfo {
  const ms = wholeMillis(s.duration);
  return {
    name: s.name,
    duration_ms: ms,
    duration_display: minutesSeconds(ms),
    num_channels: s.channels,
    sample_format: s.sampleFormat,
    track_count: s.tracks.length,
    tracks: [...s.tracks],
    has_midi: s.hasMidi,
    has_lighting: s.hasLighting,
    base_dir: s.baseDir ?? "",
    lighting_files: [...s.lightingFiles],
    midi_dmx_files: [...s.midiDmxFiles],
    beat_grid: s.beatGrid
      ? {
          beats: [...s.beatGrid.beats],
          measure_starts: [...s.beatGrid.measureStarts],
        }
      : null,
    has_tempo_map: s.hasTempoMap,
    loop_playback: s.loops,
    loop_crossfade_ms:
      s.loopCrossfadeMs !== undefined ? Number(s.loopCrossfadeMs) : null,
    loop_crossfade_cut: [...s.loopCrossfadeCut],
    sections: s.sections.map((section) => ({
      name: section.name,
      start_measure: section.startMeasure,
      end_measure: section.endMeasure,
      ...(section.startBeat !== undefined
        ? { start_beat: section.startBeat }
        : {}),
      ...(section.endBeat !== undefined ? { end_beat: section.endBeat } : {}),
      ...(section.color !== undefined ? { color: section.color } : {}),
    })),
  };
}

function songFailure(s: SongListing): SongFailure {
  return {
    name: s.name,
    error: s.failure ?? "",
    base_dir: s.baseDir ?? "",
    failed: true,
  };
}

/** The library as the song pages hold it: the songs that loaded, and
 *  those that did not, each in the order the player sorts them. */
export function mapSongList(r: ListSongsResponse): SongList {
  return {
    songs: r.songs.filter((s) => s.failure === undefined).map(songInfo),
    failures: r.songs.filter((s) => s.failure !== undefined).map(songFailure),
  };
}
