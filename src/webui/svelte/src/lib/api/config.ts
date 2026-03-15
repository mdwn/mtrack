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

import { get, post, put, del, uploadFile, putText, postText } from "./rest";

// Calibration types
export interface NoiseFloorResult {
  peak: number;
  rms: number;
  low_freq_energy: number;
  channel: number;
  sample_rate: number;
  device_channels: number;
}

export interface ChannelCalibration {
  channel: number;
  threshold: number;
  gain: number;
  scan_time_ms: number;
  retrigger_time_ms: number;
  highpass_freq?: number;
  dynamic_threshold_decay_ms?: number;
  num_hits_detected: number;
  noise_floor_peak: number;
  max_hit_amplitude: number;
}

export interface SupportedFormat {
  sample_format: string;
  bits_per_sample: number;
}

export interface AudioDeviceInfo {
  name: string;
  max_channels: number;
  host_name: string;
  supported_sample_rates: number[];
  supported_formats: SupportedFormat[];
}

export interface MidiDeviceInfo {
  name: string;
  has_input: boolean;
  has_output: boolean;
}

export interface ConfigSnapshot {
  yaml: string;
  checksum: string;
}

export async function fetchConfigStore(): Promise<ConfigSnapshot> {
  const res = await get("/config/store");
  if (!res.ok) throw new Error(`Failed to fetch config store: ${res.status}`);
  return res.json();
}

export async function fetchAudioDevices(): Promise<AudioDeviceInfo[]> {
  const res = await get("/devices/audio");
  if (!res.ok) throw new Error(`Failed to fetch audio devices: ${res.status}`);
  return res.json();
}

export async function fetchMidiDevices(): Promise<MidiDeviceInfo[]> {
  const res = await get("/devices/midi");
  if (!res.ok) throw new Error(`Failed to fetch MIDI devices: ${res.status}`);
  return res.json();
}

export async function addProfile(
  profile: object,
  checksum: string,
): Promise<ConfigSnapshot> {
  const res = await post(
    "/config/profiles",
    JSON.stringify({ expected_checksum: checksum, profile }),
  );
  if (!res.ok) {
    const body = await res.json().catch(() => ({}));
    throw new Error(body.error || `Failed to add profile: ${res.status}`);
  }
  return res.json();
}

export async function updateProfile(
  index: number,
  profile: object,
  checksum: string,
): Promise<ConfigSnapshot> {
  const res = await put(
    `/config/profiles/${index}`,
    JSON.stringify({ expected_checksum: checksum, profile }),
  );
  if (!res.ok) {
    const body = await res.json().catch(() => ({}));
    throw new Error(body.error || `Failed to update profile: ${res.status}`);
  }
  return res.json();
}

export async function deleteProfile(
  index: number,
  checksum: string,
): Promise<ConfigSnapshot> {
  const res = await del(
    `/config/profiles/${index}?expected_checksum=${encodeURIComponent(checksum)}`,
  );
  if (!res.ok) {
    const body = await res.json().catch(() => ({}));
    throw new Error(body.error || `Failed to delete profile: ${res.status}`);
  }
  return res.json();
}

// ---- File-based Profiles API (profiles_dir) ----

export interface ProfileFileInfo {
  filename: string;
  hostname: string | null;
  has_audio: boolean;
  has_midi: boolean;
  has_dmx: boolean;
}

export async function fetchProfileFiles(): Promise<ProfileFileInfo[]> {
  const res = await get("/profiles");
  if (!res.ok) throw new Error(`Failed to fetch profiles: ${res.status}`);
  return res.json();
}

export async function fetchProfileFile(
  filename: string,
): Promise<{ profile: object; yaml: string }> {
  const res = await get(`/profiles/${encodeURIComponent(filename)}`);
  if (!res.ok) {
    const d = await res.json().catch(() => ({}));
    throw new Error(d.error || `Failed to fetch profile: ${res.status}`);
  }
  return res.json();
}

export async function saveProfileFile(
  filename: string,
  profile: object,
): Promise<void> {
  const res = await put(
    `/profiles/${encodeURIComponent(filename)}`,
    JSON.stringify(profile),
  );
  if (!res.ok) {
    const d = await res.json().catch(() => ({}));
    throw new Error(d.error || `Failed to save profile: ${res.status}`);
  }
}

export async function deleteProfileFile(filename: string): Promise<void> {
  const res = await del(`/profiles/${encodeURIComponent(filename)}`);
  if (!res.ok) {
    const d = await res.json().catch(() => ({}));
    throw new Error(d.error || `Failed to delete profile: ${res.status}`);
  }
}

// Samples API

export async function updateSamples(
  samples: Record<string, unknown>,
  checksum: string,
): Promise<ConfigSnapshot> {
  const res = await put(
    "/config/samples",
    JSON.stringify({ expected_checksum: checksum, samples }),
  );
  if (!res.ok) {
    const body = await res.json().catch(() => ({}));
    throw new Error(body.error || `Failed to update samples: ${res.status}`);
  }
  return res.json();
}

export interface SampleUploadResult {
  status: string;
  file: string;
  path: string;
}

export async function uploadSampleFile(
  file: File,
): Promise<SampleUploadResult> {
  const res = await uploadFile(
    `/samples/upload/${encodeURIComponent(file.name)}`,
    file,
  );
  if (!res.ok) {
    const body = await res.json().catch(() => ({}));
    throw new Error(body.error || `Failed to upload sample: ${res.status}`);
  }
  return res.json();
}

// Calibration API

export async function startCalibration(
  device: string,
  channel: number,
  duration?: number,
): Promise<NoiseFloorResult> {
  const body: Record<string, unknown> = { device, channel };
  if (duration !== undefined) body.duration = duration;
  const res = await post("/calibrate/start", JSON.stringify(body));
  if (!res.ok) {
    const data = await res.json().catch(() => ({}));
    throw new Error(data.error || `Calibration start failed: ${res.status}`);
  }
  return res.json();
}

export async function startCapture(): Promise<void> {
  const res = await post("/calibrate/capture");
  if (!res.ok) {
    const data = await res.json().catch(() => ({}));
    throw new Error(data.error || `Capture start failed: ${res.status}`);
  }
}

export async function stopCapture(): Promise<ChannelCalibration> {
  const res = await post("/calibrate/stop");
  if (!res.ok) {
    const data = await res.json().catch(() => ({}));
    throw new Error(data.error || `Capture stop failed: ${res.status}`);
  }
  return res.json();
}

export async function cancelCalibration(): Promise<void> {
  await del("/calibrate");
}

// Lighting fixture type & venue API

export interface FixtureTypeData {
  name: string;
  channels: Record<string, number>;
  max_strobe_frequency: number | null;
  min_strobe_frequency: number | null;
  strobe_dmx_offset: number | null;
}

export interface FixtureData {
  name: string;
  fixture_type: string;
  universe: number;
  start_channel: number;
  tags: string[];
}

export interface VenueData {
  name: string;
  fixtures: Record<string, FixtureData>;
  groups: Record<string, { name: string; fixtures: string[] }>;
}

export async function fetchFixtureTypes(
  dir?: string,
): Promise<Record<string, FixtureTypeData>> {
  const params = dir ? `?dir=${encodeURIComponent(dir)}` : "";
  const res = await get(`/lighting/fixture-types${params}`);
  if (!res.ok) {
    const data = await res.json().catch(() => ({}));
    throw new Error(
      data.error || `Failed to fetch fixture types: ${res.status}`,
    );
  }
  const data = await res.json();
  return data.fixture_types;
}

export async function fetchFixtureType(
  name: string,
  dir?: string,
): Promise<{ fixture_type: FixtureTypeData; dsl: string }> {
  const params = dir ? `?dir=${encodeURIComponent(dir)}` : "";
  const res = await get(
    `/lighting/fixture-types/${encodeURIComponent(name)}${params}`,
  );
  if (!res.ok) {
    const data = await res.json().catch(() => ({}));
    throw new Error(
      data.error || `Failed to fetch fixture type: ${res.status}`,
    );
  }
  return res.json();
}

export async function saveFixtureType(
  name: string,
  data: {
    channels: Record<string, number>;
    max_strobe_frequency?: number | null;
    min_strobe_frequency?: number | null;
    strobe_dmx_offset?: number | null;
  },
  dir?: string,
): Promise<void> {
  const params = dir ? `?dir=${encodeURIComponent(dir)}` : "";
  const res = await put(
    `/lighting/fixture-types/${encodeURIComponent(name)}${params}`,
    JSON.stringify(data),
  );
  if (!res.ok) {
    const d = await res.json().catch(() => ({}));
    throw new Error(d.error || `Failed to save fixture type: ${res.status}`);
  }
}

export async function deleteFixtureType(
  name: string,
  dir?: string,
): Promise<void> {
  const params = dir ? `?dir=${encodeURIComponent(dir)}` : "";
  const res = await del(
    `/lighting/fixture-types/${encodeURIComponent(name)}${params}`,
  );
  if (!res.ok) {
    const data = await res.json().catch(() => ({}));
    throw new Error(
      data.error || `Failed to delete fixture type: ${res.status}`,
    );
  }
}

export async function fetchVenues(
  dir?: string,
): Promise<Record<string, VenueData>> {
  const params = dir ? `?dir=${encodeURIComponent(dir)}` : "";
  const res = await get(`/lighting/venues${params}`);
  if (!res.ok) {
    const data = await res.json().catch(() => ({}));
    throw new Error(data.error || `Failed to fetch venues: ${res.status}`);
  }
  const data = await res.json();
  return data.venues;
}

export async function fetchVenue(
  name: string,
  dir?: string,
): Promise<{ venue: VenueData; dsl: string }> {
  const params = dir ? `?dir=${encodeURIComponent(dir)}` : "";
  const res = await get(
    `/lighting/venues/${encodeURIComponent(name)}${params}`,
  );
  if (!res.ok) {
    const data = await res.json().catch(() => ({}));
    throw new Error(data.error || `Failed to fetch venue: ${res.status}`);
  }
  return res.json();
}

export async function saveVenue(
  name: string,
  data: {
    fixtures: {
      name: string;
      fixture_type: string;
      universe: number;
      start_channel: number;
      tags: string[];
    }[];
    groups?: Record<string, string[]>;
  },
  dir?: string,
): Promise<void> {
  const params = dir ? `?dir=${encodeURIComponent(dir)}` : "";
  const res = await put(
    `/lighting/venues/${encodeURIComponent(name)}${params}`,
    JSON.stringify(data),
  );
  if (!res.ok) {
    const d = await res.json().catch(() => ({}));
    throw new Error(d.error || `Failed to save venue: ${res.status}`);
  }
}

export async function deleteVenue(name: string, dir?: string): Promise<void> {
  const params = dir ? `?dir=${encodeURIComponent(dir)}` : "";
  const res = await del(
    `/lighting/venues/${encodeURIComponent(name)}${params}`,
  );
  if (!res.ok) {
    const data = await res.json().catch(() => ({}));
    throw new Error(data.error || `Failed to delete venue: ${res.status}`);
  }
}

// ---- Playlist CRUD ----

export interface PlaylistInfo {
  name: string;
  song_count: number;
  is_active: boolean;
}

export interface PlaylistData {
  name: string;
  songs: string[];
  available_songs: string[];
}

export async function fetchPlaylists(): Promise<PlaylistInfo[]> {
  const res = await get("/playlists");
  if (!res.ok) throw new Error(`Failed to fetch playlists: ${res.status}`);
  return res.json();
}

export async function fetchPlaylist(name: string): Promise<PlaylistData> {
  const res = await get(`/playlists/${encodeURIComponent(name)}`);
  if (!res.ok) throw new Error(`Failed to fetch playlist: ${res.status}`);
  return res.json();
}

export async function savePlaylist(
  name: string,
  songs: string[],
): Promise<void> {
  const res = await put(
    `/playlists/${encodeURIComponent(name)}`,
    JSON.stringify({ songs }),
  );
  if (!res.ok) {
    const d = await res.json().catch(() => ({}));
    throw new Error(d.error || `Failed to save playlist: ${res.status}`);
  }
}

export async function deletePlaylist(name: string): Promise<void> {
  const res = await del(`/playlists/${encodeURIComponent(name)}`);
  if (!res.ok) {
    const d = await res.json().catch(() => ({}));
    throw new Error(d.error || `Failed to delete playlist: ${res.status}`);
  }
}

export async function activatePlaylist(name: string): Promise<void> {
  const res = await post(`/playlists/${encodeURIComponent(name)}/activate`);
  if (!res.ok) {
    const d = await res.json().catch(() => ({}));
    throw new Error(d.error || `Failed to activate playlist: ${res.status}`);
  }
}

// ---- Lighting Show Files ----

export interface LightFileInfo {
  path: string;
  name: string;
}

export async function fetchLightingFiles(): Promise<LightFileInfo[]> {
  const res = await get("/lighting");
  if (!res.ok) throw new Error(`Failed to fetch lighting files: ${res.status}`);
  const data = await res.json();
  return data.files;
}

export async function fetchLightingFile(name: string): Promise<string> {
  const res = await get(`/lighting/${encodeURIComponent(name)}`);
  if (!res.ok) {
    throw new Error(`Failed to fetch lighting file: ${res.status}`);
  }
  return res.text();
}

export async function saveLightingFile(
  name: string,
  content: string,
): Promise<void> {
  const res = await putText(`/lighting/${encodeURIComponent(name)}`, content);
  if (!res.ok) {
    const d = await res.json().catch(() => ({}));
    const errors = d.errors ? d.errors.join("; ") : d.error;
    throw new Error(errors || `Failed to save lighting file: ${res.status}`);
  }
}

export async function validateLighting(
  content: string,
): Promise<{ valid: boolean; errors?: string[] }> {
  const res = await postText("/lighting/validate", content);
  return res.json();
}
