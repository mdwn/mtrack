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

import type { Page } from "@playwright/test";
import { Code, ConnectError } from "@connectrpc/connect";
import {
  PlayerService,
  type EntrySettings,
  type Fade,
  type AmbientSetting,
  type SavePlaylistRequest,
} from "../../src/gen/player/v1/player_pb";
import type {
  Ambient,
  AutoPlayConfig,
  FadeConfig,
} from "../../src/gen/rules/mtrack_rules";
import { PLAYLIST_DETAILS } from "../mock-server/test-data";
import { getPlaylistReply, type PlaylistFixture } from "./library-rpc";
import { mockRpc } from "./rpc";

function fade(f: Fade | undefined): FadeConfig | null {
  if (!f) return null;
  if (f.curve === undefined && !f.exclude && f.lights === undefined) {
    return f.length;
  }
  return {
    length: f.length,
    ...(f.curve !== undefined ? { curve: f.curve } : {}),
    ...(f.exclude ? { exclude: f.exclude.names } : {}),
    ...(f.lights !== undefined ? { lights: f.lights } : {}),
  };
}

function ambient(a: AmbientSetting | undefined): false | Ambient | null {
  if (!a) return null;
  if (!a.on) return false;
  return {
    ...(a.show !== undefined ? { show: a.show } : {}),
    ...(a.delay !== undefined ? { delay: a.delay } : {}),
    ...(a.fadeIn !== undefined ? { fade_in: a.fadeIn } : {}),
    ...(a.fadeOut !== undefined ? { fade_out: a.fadeOut } : {}),
  };
}

function autoPlay(s: EntrySettings): AutoPlayConfig | null {
  if (!s.autoPlay) return null;
  return s.autoPlay.gap === "" ? true : s.autoPlay.gap;
}

/** What a playlist save says, as the page's playlist editor holds it (the
 *  songs, and each setting it sets), read from the `SavePlaylist` request:
 *  a setting the request does not set is left out, as it was left out of the
 *  body the REST save sent. */
export function playlistBody(
  req: SavePlaylistRequest,
): Record<string, unknown> {
  const body: Record<string, unknown> = { songs: [...req.songs] };
  if (req.setSettings) {
    body.auto_play = req.settings.map(autoPlay);
    body.fades = req.settings.map((s) => ({
      fade_out: fade(s.fadeOut),
      fade_in: fade(s.fadeIn),
    }));
    body.entry_ambient = req.settings.map((s) => ambient(s.ambient));
    body.entry_palette = req.settings.map((s) => ({ ...s.palette?.slots }));
  }
  if (req.setAmbient) body.ambient = ambient(req.ambient);
  return body;
}

/** Answers the saves of the playlist `setlist` as saved, letting the rest
 *  through; the saves as the editor holds them (see {@link playlistBody}),
 *  in order, as they come. */
export async function capturePlaylistPut(page: Page) {
  const bodies: Record<string, unknown>[] = [];
  const rpc = await mockRpc(page);
  rpc.unary(PlayerService.method.savePlaylist, (req) => {
    bodies.push(playlistBody(req));
    return { name: req.name };
  });
  return bodies;
}

/** Answers the read of the playlist `name` with `detail`, the other
 *  playlists as the mock server has them. */
export async function stubPlaylist(
  page: Page,
  name: string,
  detail: PlaylistFixture,
) {
  const rpc = await mockRpc(page);
  rpc.unary(PlayerService.method.getPlaylist, (req) => {
    if (req.name === name) return getPlaylistReply(detail);
    const other = PLAYLIST_DETAILS[req.name];
    if (!other) throw new ConnectError("Playlist not found", Code.NotFound);
    return getPlaylistReply(other);
  });
  return rpc;
}
