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
// `AddProfile`, `UpdateProfile`, `RemoveProfile` on `PlayerService`;
// `UpdateSamples`, `UpdateMetronome` and the profile file calls on
// `ConfigService`), mapped into the shapes the components hold
// (`StoreSnapshot`, `ConfigSaved`, `ProfileFile`, `ProfileFileInfo`,
// `FileSaved`). Only the mapping lives here, so a unit test can run it
// without a page; the calls are in src/lib/api.

import type {
  GetProfileFileResponse,
  ListProfileFilesResponse,
  ProfileFileSaved,
  GetConfigResponse,
  UpdateConfigResponse,
} from "../../gen/player/v1/player_pb";
import type {
  ConfigReload,
  ConfigSaved,
  FileSaved,
  MetronomeSounds,
  Profile,
  ProfileFile,
  ProfileFileInfo,
  StoreSnapshot,
} from "../../gen/rules/mtrack_rules";

/** The config as written, and the checksum a save presents. */
export function mapConfigStore(r: GetConfigResponse): StoreSnapshot {
  return { yaml: r.yaml, checksum: r.checksum };
}

/** A save's answer: the config after it, and what became of the hardware
 *  reload. What the reply leaves empty is absent, as the notice reads it
 *  (`message ?? ...`); a server that does not say which reload it was
 *  (`reload` empty) is read by `applied`. */
export function mapConfigSaved(r: UpdateConfigResponse): ConfigSaved {
  return {
    yaml: r.yaml,
    checksum: r.checksum,
    ...reloadOf(r),
  };
}

/** What a save made of the hardware reload, from the fields every save's
 *  answer carries. */
function reloadOf(r: {
  applied: boolean;
  reload: string;
  message: string;
  songsError?: string;
  awaitsReload?: bigint;
  awaitsProcess?: string;
}): ConfigReload {
  const reload: ConfigReload = {
    applied: r.applied,
    reload: r.reload || (r.applied ? "applied" : "deferred"),
  };
  if (r.message) reload.message = r.message;
  if (r.songsError !== undefined) reload.songs_error = r.songsError;
  if (r.awaitsReload !== undefined)
    reload.awaits_reload = Number(r.awaitsReload);
  if (r.awaitsProcess !== undefined) reload.awaits_process = r.awaitsProcess;
  return reload;
}

/** The profile files of the profiles directory, as the list shows them: what
 *  the server leaves out (a hostname, why a file does not parse) is `null`. */
export function mapProfileFiles(
  r: ListProfileFilesResponse,
): ProfileFileInfo[] {
  return r.files.map((f) => ({
    file: f.file,
    filename: f.filename,
    addressable: f.addressable,
    error: f.error ?? null,
    hostname: f.hostname ?? null,
    has_audio: f.hasAudio,
    has_midi: f.hasMidi,
    has_dmx: f.hasDmx,
    has_trigger: f.hasTrigger,
    has_controllers: f.hasControllers,
  }));
}

/** A profile file read: the profile, the file as written and its version. */
export function mapProfileFile(r: GetProfileFileResponse): ProfileFile {
  return {
    profile: JSON.parse(r.profileJson) as Profile,
    yaml: r.yaml,
    version: r.version,
  };
}

/** A profile file saved or deleted, with what became of the hardware reload.
 *  A deleted file has no version. */
export function mapFileSaved(r: ProfileFileSaved): FileSaved {
  const saved: FileSaved = {
    status: r.status,
    ...reloadOf(r),
  };
  if (r.filename) saved.filename = r.filename;
  if (r.version !== undefined) saved.version = r.version;
  return saved;
}

/** The metronome defaults as the server takes them: only what differs from
 *  the defaults is sent, and `null` (an empty string on the wire) when
 *  nothing does, which removes them. */
export function metronomeJson(
  sounds: MetronomeSounds | null,
  enabled: boolean,
  volume: number | null,
): string {
  const hasSounds = sounds && Object.keys(sounds).length > 0;
  const hasVolume = volume !== null && volume !== 1;
  if (!hasSounds && !enabled && !hasVolume) return "null";
  return JSON.stringify({
    ...(enabled ? { enabled } : {}),
    ...(hasVolume ? { volume } : {}),
    ...(hasSounds ? { sounds } : {}),
  });
}
