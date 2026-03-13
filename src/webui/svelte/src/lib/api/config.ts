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

import { get, post, put, del } from "./rest";

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
