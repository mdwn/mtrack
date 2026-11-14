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

import { Code, ConnectError } from "@connectrpc/connect";
import type {
  Browse,
  BulkImport,
  SongFile,
  SongList,
  TempoGuess,
  Waveform,
} from "../../gen/rules/mtrack_rules";
import { configClient, playerClient } from "../grpc/client";
import { callError, mapSongList } from "../grpc/library";
import {
  mapBrowse,
  mapBulkImport,
  mapSongConfig,
  mapSongFiles,
  mapTempoGuess,
  songCallError,
} from "../grpc/songs";
export { SongNameTakenError } from "../grpc/songs";
import { mapWaveform } from "../grpc/watch";
import { uploadFile, uploadFiles } from "./rest";

/** Every song in the library, with what the song pages show, and those that
 *  failed to load. */
export async function fetchSongs(): Promise<SongList> {
  try {
    return mapSongList(await playerClient.listSongs({ detailed: true }));
  } catch (e) {
    throw callError(e, "Failed to fetch songs");
  }
}

/** A song's config file, as written. */
export async function fetchSongConfig(name: string): Promise<string> {
  try {
    return mapSongConfig(await configClient.getSongConfig({ name }));
  } catch (e) {
    throw songCallError(e, "Failed to fetch song");
  }
}

/** Creates a song of `yaml` in the folder `name` (a nested song's has "/").
 *  A song that exists already, or a name another song holds, is refused with
 *  an error of `kind` "exists" (the latter a {@link SongNameTakenError}). */
export async function createSong(name: string, yaml: string): Promise<void> {
  try {
    await configClient.createSong({ name, yaml });
  } catch (e) {
    throw songCallError(e, "Failed to create song");
  }
}

/** Saves a song's config. A config that is not valid is refused with every
 *  problem in the message. */
export async function updateSong(name: string, yaml: string): Promise<void> {
  try {
    await configClient.saveSongConfig({ name, yaml });
  } catch (e) {
    throw songCallError(e, "Save failed");
  }
}

export async function uploadTrack(
  songName: string,
  file: File,
): Promise<Response> {
  return uploadFile(
    `/songs/${encodeURIComponent(songName)}/tracks/${encodeURIComponent(file.name)}`,
    file,
  );
}

export async function uploadTracks(
  songName: string,
  files: File[],
): Promise<Response> {
  return uploadFiles(`/songs/${encodeURIComponent(songName)}/tracks`, files);
}

/** Copies a file of the project into a song. Answers the name the song has:
 *  a new one's may not be the one asked for, when that is taken. */
export async function importFileToSong(
  songName: string,
  sourcePath: string,
  filename?: string,
): Promise<string> {
  try {
    const imported = await configClient.importSongFile({
      name: songName,
      path: sourcePath,
      filename,
    });
    return imported.song;
  } catch (e) {
    throw songCallError(e, "Import failed");
  }
}

export async function deleteSong(name: string): Promise<void> {
  try {
    await configClient.deleteSong({ name });
  } catch (e) {
    throw songCallError(e, "Delete failed");
  }
}

// The song.yaml `tempo:` and `metronome:` blocks, and the beat grid: the
// player's own types, generated from the Rust structs with the rules module
// (`lib/rules.ts`), so the UI holds no copy of them to drift.
export type {
  SongFile,
  BeatGrid,
  TempoConfig,
  TempoChangeConfig,
  TransitionConfig,
  MetronomeConfig,
  MetronomeChange as MetronomeChangeConfig,
  MetronomeSounds,
  ClickSound as ClickSoundConfig,
  Subdivision as SubdivisionValue,
} from "../../gen/rules/mtrack_rules";

/** A tempo map suggested for a song, or none when there is none to make: the
 *  song has neither a MIDI file nor a beat grid (not found), or failed to load
 *  (invalid), whose load error the song view already shows. Neither tempo
 *  editor has anywhere to say more, so both leave the tempo as it is. */
export async function fetchTempoGuess(
  name: string,
): Promise<TempoGuess | null> {
  try {
    return mapTempoGuess(await configClient.guessSongTempo({ name }));
  } catch (e) {
    if (
      e instanceof ConnectError &&
      (e.code === Code.NotFound || e.code === Code.InvalidArgument)
    ) {
      return null;
    }
    throw songCallError(e, "Failed to fetch tempo guess");
  }
}

export async function fetchWaveform(name: string): Promise<Waveform> {
  try {
    return mapWaveform(await configClient.getSongWaveform({ name }));
  } catch (e) {
    throw songCallError(e, "Failed to fetch waveform");
  }
}

export async function fetchSongFiles(name: string): Promise<SongFile[]> {
  try {
    return mapSongFiles(await configClient.listSongFiles({ name }));
  } catch (e) {
    throw songCallError(e, "Failed to fetch song files");
  }
}

export async function browseDirectory(path?: string): Promise<Browse> {
  try {
    return mapBrowse(await configClient.browseDirectory({ path: path ?? "" }));
  } catch (e) {
    throw songCallError(e, "Failed to browse");
  }
}

/** Makes a song of a folder of the project; answers its name. A folder that
 *  has a config, or a name another song holds, is refused with an error of
 *  `kind` "exists" (the latter a {@link SongNameTakenError}). */
export async function createSongInDirectory(
  dirPath: string,
  name?: string,
): Promise<string> {
  try {
    const created = await configClient.createSongInDirectory({
      path: dirPath,
      name,
    });
    return created.song;
  } catch (e) {
    throw songCallError(e, "Failed to create song");
  }
}

export async function bulkImportSongs(dirPath: string): Promise<BulkImport> {
  try {
    return mapBulkImport(await configClient.bulkImportSongs({ path: dirPath }));
  } catch (e) {
    throw songCallError(e, "Bulk import failed");
  }
}
