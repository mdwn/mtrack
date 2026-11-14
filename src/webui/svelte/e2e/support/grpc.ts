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

// Stubbing the player's gRPC-web service for specs that only need its calls
// to succeed. A spec that needs an answer, or the requests, uses ./rpc.ts.
import type { Page } from "@playwright/test";
import {
  ConfigService,
  PlayerService,
} from "../../src/gen/player/v1/player_pb";
import { ConnectError, Code } from "@connectrpc/connect";
import {
  CONFIG_STORE,
  PROFILE_FILE_DETAIL,
  PROFILE_FILES,
  PLAYLIST_DETAILS,
  PLAYLISTS,
  SONGS,
} from "../mock-server/test-data";
import {
  getConfigReply,
  getProfileFileReply,
  listProfileFilesReply,
} from "./config-rpc";
import {
  getPlaylistReply,
  listPlaylistsReply,
  listSongsReply,
} from "./library-rpc";
import { mockRpc, type GrpcReply } from "./rpc";

export type { GrpcReply };

/** The calls a page makes to read the song library, the playlists, the
 *  config and the profile files. A spec that stubs the transport controls still needs the pages to
 *  have their songs, playlists and config, so these are answered as the mock
 *  server answers them, and are not the calls `onCall` counts. */
const LIBRARY_READS = new Set<string>([
  PlayerService.method.listSongs.name,
  PlayerService.method.listPlaylists.name,
  PlayerService.method.getPlaylist.name,
  PlayerService.method.getConfig.name,
  ConfigService.method.listProfileFiles.name,
  ConfigService.method.getProfileFile.name,
]);

/** Answers every PlayerService and ConfigService call but the streams (`WatchStage`) with
 *  success (see {@link GrpcReply}), calling `onCall` first, if given, for
 *  each but the reads of the song library and the playlists, which are
 *  answered with the mock server's songs and playlists. */
export async function stubPlayerService(
  page: Page,
  options: { reply?: GrpcReply; onCall?: () => void } = {},
): Promise<void> {
  const rpc = await mockRpc(page, {
    otherwise: options.reply ?? "trailers-only",
    onCall: (call) => {
      if (!LIBRARY_READS.has(call.method)) options.onCall?.();
    },
  });
  rpc.unary(PlayerService.method.listSongs, listSongsReply(SONGS));
  rpc.unary(PlayerService.method.listPlaylists, listPlaylistsReply(PLAYLISTS));
  rpc.unary(PlayerService.method.getConfig, getConfigReply(CONFIG_STORE));
  rpc.unary(
    ConfigService.method.listProfileFiles,
    listProfileFilesReply(PROFILE_FILES),
  );
  rpc.unary(
    ConfigService.method.getProfileFile,
    getProfileFileReply(PROFILE_FILE_DETAIL),
  );
  rpc.unary(PlayerService.method.getPlaylist, (req) => {
    const detail = PLAYLIST_DETAILS[req.name];
    if (!detail) throw new ConnectError("Playlist not found", Code.NotFound);
    return getPlaylistReply(detail);
  });
}
