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

import { get, post, postYaml, putYaml, uploadFile, uploadFiles } from "./rest";

export interface SongSummary {
  name: string;
  duration_ms: number;
  duration_display: string;
  num_channels: number;
  sample_format: string;
  track_count: number;
  tracks: string[];
  has_midi: boolean;
  has_lighting: boolean;
  /** Song directory path relative to the songs root */
  base_dir: string;
  /** DSL lighting file paths relative to the songs root */
  lighting_files: string[];
  /** MIDI DMX file paths relative to the songs root */
  midi_dmx_files: string[];
}

export interface WaveformTrack {
  name: string;
  peaks: number[];
}

export interface WaveformData {
  song_name: string;
  tracks: WaveformTrack[];
}

export async function fetchSongs(): Promise<SongSummary[]> {
  const res = await get("/songs");
  if (!res.ok) throw new Error(`Failed to fetch songs: ${res.status}`);
  const data = await res.json();
  return data.songs;
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

export async function fetchWaveform(name: string): Promise<WaveformData> {
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
