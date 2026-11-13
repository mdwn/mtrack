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

// The config store over gRPC: the calls the config editor makes (`GetConfig`,
// `AddProfile`, `UpdateProfile`, `RemoveProfile`), mapped into the shapes the
// components hold (`StoreSnapshot`, `ConfigSaved`). Only the mapping lives
// here, so a unit test can run it without a page; the calls are in
// src/lib/api.

import type {
  GetConfigResponse,
  UpdateConfigResponse,
} from "../../gen/player/v1/player_pb";
import type { ConfigSaved, StoreSnapshot } from "../../gen/rules/mtrack_rules";

/** The config as written, and the checksum a save presents. */
export function mapConfigStore(r: GetConfigResponse): StoreSnapshot {
  return { yaml: r.yaml, checksum: r.checksum };
}

/** A save's answer: the config after it, and what became of the hardware
 *  reload. What the reply leaves empty is absent, as the notice reads it
 *  (`message ?? ...`); a server that does not say which reload it was
 *  (`reload` empty) is read by `applied`. */
export function mapConfigSaved(r: UpdateConfigResponse): ConfigSaved {
  const saved: ConfigSaved = {
    yaml: r.yaml,
    checksum: r.checksum,
    applied: r.applied,
    reload: r.reload || (r.applied ? "applied" : "deferred"),
  };
  if (r.message) saved.message = r.message;
  if (r.songsError !== undefined) saved.songs_error = r.songsError;
  if (r.awaitsReload !== undefined)
    saved.awaits_reload = Number(r.awaitsReload);
  if (r.awaitsProcess !== undefined) saved.awaits_process = r.awaitsProcess;
  return saved;
}
