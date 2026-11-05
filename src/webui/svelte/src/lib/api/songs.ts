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

import type {
  SongList,
  TempoGuess,
  Waveform,
} from "../../gen/rules/mtrack_rules";
import { get, post, postYaml, putYaml, uploadFile, uploadFiles } from "./rest";

export async function fetchSongs(): Promise<SongList> {
  const res = await get("/songs");
  if (!res.ok) throw new Error(`Failed to fetch songs: ${res.status}`);
  return res.json();
}

export async function fetchSongConfig(name: string): Promise<string> {
  const res = await get(`/songs/${encodeURIComponent(name)}`);
  if (!res.ok) throw new Error(`Failed to fetch song: ${res.status}`);
  return res.text();
}

export async function createSong(
  name: string,
  yaml: string,
): Promise<Response> {
  return postYaml(`/songs/${encodeURIComponent(name)}`, yaml);
}

export async function updateSong(
  name: string,
  yaml: string,
): Promise<Response> {
  return putYaml(`/songs/${encodeURIComponent(name)}`, yaml);
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

export async function importFileToSong(
  songName: string,
  sourcePath: string,
  filename?: string,
): Promise<Response> {
  return post(
    `/songs/${encodeURIComponent(songName)}/import`,
    JSON.stringify({ path: sourcePath, filename }),
  );
}

export async function deleteSong(name: string): Promise<Response> {
  return fetch(`/api/songs/${encodeURIComponent(name)}`, { method: "DELETE" });
}

// The song.yaml `tempo:` and `metronome:` blocks, and the beat grid: the
// player's own types, generated from the Rust structs with the rules module
// (`lib/rules.ts`), so the UI holds no copy of them to drift.
export type {
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

export async function fetchTempoGuess(
  name: string,
): Promise<TempoGuess | null> {
  const res = await get(`/songs/${encodeURIComponent(name)}/tempo-guess`);
  if (res.status === 404) return null;
  if (!res.ok) throw new Error(`Failed to fetch tempo guess: ${res.status}`);
  return res.json();
}

export async function fetchWaveform(name: string): Promise<Waveform> {
  const res = await get(`/songs/${encodeURIComponent(name)}/waveform`);
  if (!res.ok) throw new Error(`Failed to fetch waveform: ${res.status}`);
  return res.json();
}

export interface SongFile {
  name: string;
  type: "audio" | "midi" | "lighting" | "other";
}

export async function fetchSongFiles(name: string): Promise<SongFile[]> {
  const res = await get(`/songs/${encodeURIComponent(name)}/files`);
  if (!res.ok) throw new Error(`Failed to fetch song files: ${res.status}`);
  const data = await res.json();
  return data.files;
}

export interface BrowseEntry {
  name: string;
  path: string;
  type: "directory" | "audio" | "midi" | "lighting" | "other";
  is_dir: boolean;
}

export interface BrowseResult {
  path: string;
  root: string;
  entries: BrowseEntry[];
}

export async function browseDirectory(path?: string): Promise<BrowseResult> {
  const query = path ? `?path=${encodeURIComponent(path)}` : "";
  const res = await get(`/browse${query}`);
  if (!res.ok) {
    const data = await res.json().catch(() => null);
    throw new Error(data?.error ?? `Failed to browse: ${res.status}`);
  }
  return res.json();
}

export async function createSongInDirectory(
  dirPath: string,
  name?: string,
): Promise<Response> {
  return post("/browse/create-song", JSON.stringify({ path: dirPath, name }));
}

export interface BulkImportResult {
  created: string[];
  skipped: string[];
  failed: { name: string; error: string }[];
}

export async function bulkImportSongs(
  dirPath: string,
): Promise<BulkImportResult> {
  const res = await post(
    "/browse/bulk-import",
    JSON.stringify({ path: dirPath }),
  );
  if (!res.ok) {
    const data = await res.json().catch(() => null);
    throw new Error(data?.error ?? `Bulk import failed: ${res.status}`);
  }
  return res.json();
}
