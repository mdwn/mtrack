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

// The config store as the player answers it over gRPC, made from the shapes
// the web UI holds of it (`StoreSnapshot`, `ConfigSaved`): the mirror of
// src/lib/grpc/config.ts, so a spec states the config as the page shows it
// and answers the call with it:
//
//   const rpc = await mockRpc(page);
//   rpc.unary(PlayerService.method.getConfig, getConfigReply({ yaml, checksum }));
//   rpc.unary(PlayerService.method.updateProfile, savedReply({ yaml, checksum: "n" }));
//
// The mock server answers the same calls with the same helpers.
import type { MessageInitShape } from "@bufbuild/protobuf";
import type {
  GetConfigResponseSchema,
  UpdateConfigResponseSchema,
} from "../../src/gen/player/v1/player_pb";
import type {
  ConfigReload,
  StoreSnapshot,
} from "../../src/gen/rules/mtrack_rules";

export function getConfigReply(
  snapshot: StoreSnapshot,
): MessageInitShape<typeof GetConfigResponseSchema> {
  return { yaml: snapshot.yaml, checksum: snapshot.checksum };
}

/** A config save's answer: the config after it, and what became of the
 *  hardware reload (applied at once unless the spec says otherwise). */
export function savedReply(
  snapshot: StoreSnapshot,
  reload: Partial<ConfigReload> = {},
): MessageInitShape<typeof UpdateConfigResponseSchema> {
  const applied = reload.applied ?? true;
  return {
    yaml: snapshot.yaml,
    checksum: snapshot.checksum,
    applied,
    reload: reload.reload ?? (applied ? "applied" : "deferred"),
    message: reload.message,
    songsError: reload.songs_error,
    awaitsReload:
      reload.awaits_reload !== undefined
        ? BigInt(reload.awaits_reload)
        : undefined,
    awaitsProcess: reload.awaits_process,
  };
}
