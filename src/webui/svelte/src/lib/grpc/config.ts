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
// `FileSaved`). The device lists, the audio probe and the trigger
// calibration are `ConfigService`'s too (`AudioDeviceInfo`, `MidiDeviceInfo`,
// `AudioProbe`, `NoiseFloor`, `ChannelCalibration`). Only the mapping lives
// here, so a unit test can run it without a page; the calls are in
// src/lib/api.

import type { MessageInitShape } from "@bufbuild/protobuf";
import {
  ProbeOutcome,
  StreamBufferKind,
  type CalibrationNoiseFloor,
  type CalibrationResult,
  type GetProfileFileResponse,
  type ListAudioDevicesResponse,
  type ListMidiDevicesResponse,
  type ListProfileFilesResponse,
  type ProbeAudioDeviceRequestSchema,
  type ProbeAudioDeviceResponse,
  type ProfileFileSaved,
  type GetConfigResponse,
  type StartCalibrationRequestSchema,
  type UpdateConfigResponse,
} from "../../gen/player/v1/player_pb";
import type {
  AudioDeviceInfo,
  AudioProbe,
  ChannelCalibration,
  ConfigReload,
  ConfigSaved,
  FileSaved,
  MetronomeSounds,
  MidiDeviceInfo,
  NoiseFloor,
  ProbeAudioRequest,
  ProbeKind,
  Profile,
  ProfileFile,
  ProfileFileInfo,
  StoreSnapshot,
} from "../../gen/rules/mtrack_rules";
import { audioOutput } from "./watch";

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

// ---- Devices ----

/** The audio devices, as the picker lists them. */
export function mapAudioDevices(
  r: ListAudioDevicesResponse,
): AudioDeviceInfo[] {
  return r.devices.map((d) => ({
    name: d.name,
    max_channels: d.maxChannels,
    host_name: d.hostName,
    supported_sample_rates: d.supportedSampleRates,
    supported_formats: d.supportedFormats.map((f) => ({
      sample_format: f.sampleFormat,
      bits_per_sample: f.bitsPerSample,
    })),
    virtual_node: d.virtualNode,
    channels_known: d.channelsKnown,
  }));
}

/** The MIDI devices, as the picker lists them. */
export function mapMidiDevices(r: ListMidiDevicesResponse): MidiDeviceInfo[] {
  return r.devices.map((d) => ({
    name: d.name,
    has_input: d.hasInput,
    has_output: d.hasOutput,
  }));
}

/** What the editor asks to have probed, as the call takes it: what the form
 *  leaves unset (`null` or absent) is left out. */
export function probeAudioRequest(
  req: ProbeAudioRequest,
): MessageInitShape<typeof ProbeAudioDeviceRequestSchema> {
  const size = req.stream_buffer_size;
  return {
    device: req.device,
    sampleRate: req.sample_rate ?? undefined,
    sampleFormat: req.sample_format ?? undefined,
    bitsPerSample: req.bits_per_sample ?? undefined,
    bufferSize:
      req.buffer_size !== undefined && req.buffer_size !== null
        ? BigInt(req.buffer_size)
        : undefined,
    streamBufferSize:
      size === undefined || size === null
        ? undefined
        : {
            size:
              typeof size === "number"
                ? { case: "frames", value: BigInt(size) }
                : {
                    case: "kind",
                    value:
                      size === "min"
                        ? StreamBufferKind.MIN
                        : StreamBufferKind.DEFAULT,
                  },
          },
  };
}

const PROBE_KINDS: Partial<Record<ProbeOutcome, ProbeKind>> = {
  [ProbeOutcome.STREAMING]: "streaming",
  [ProbeOutcome.OPENED_BUT_SILENT]: "opened_but_silent",
  [ProbeOutcome.COULD_NOT_OPEN]: "could_not_open",
  [ProbeOutcome.OPENED_CANNOT_REPORT]: "opened_cannot_report",
  [ProbeOutcome.IN_USE]: "in_use",
};

/** A probe's answer. The fields a case does not decide are absent, as the
 *  panel reads them: the held device's health is there only for `in_use`
 *  (`null` when it cannot report one). */
export function mapAudioProbe(r: ProbeAudioDeviceResponse): AudioProbe {
  const outcome = PROBE_KINDS[r.outcome];
  if (!outcome) throw new Error(`Device test gave an unknown outcome`);
  const probe: AudioProbe = {
    outcome,
    ok: r.ok,
    reason: r.reason ?? null,
    callbacks: r.callbacks !== undefined ? Number(r.callbacks) : null,
  };
  if (outcome === "in_use") {
    probe.health = r.health ? audioOutput(r.health) : null;
  }
  return probe;
}

// ---- Calibration ----

/** The call that begins a calibration: the duration is left out unless one
 *  is given, and the player records for 3 seconds. */
export function startCalibrationRequest(
  device: string,
  channel: number,
  duration?: number,
): MessageInitShape<typeof StartCalibrationRequestSchema> {
  return { device, channel, duration };
}

/** A channel's noise floor. The player measures in f32 and sends it widened,
 *  as the number has always read. */
export function mapNoiseFloor(r: CalibrationNoiseFloor): NoiseFloor {
  return {
    peak: r.peak,
    rms: r.rms,
    low_freq_energy: r.lowFreqEnergy,
    channel: r.channel,
    sample_rate: r.sampleRate,
    device_channels: r.deviceChannels,
  };
}

/** The shortest decimal that reads back as the same 32-bit float, which is
 *  how the player has always written one (`0.1`, not `0.10000000149011612`).
 *  The calibration's values go into the config and are shown as they are. */
export function f32(value: number): number {
  for (let digits = 1; digits < 9; digits++) {
    const shorter = Number(value.toPrecision(digits));
    if (Math.fround(shorter) === value) return shorter;
  }
  return Number(value.toPrecision(9));
}

/** A channel's calibrated parameters. The optional ones are absent when the
 *  player derived none. */
export function mapCalibration(r: CalibrationResult): ChannelCalibration {
  const calibration: ChannelCalibration = {
    channel: r.channel,
    threshold: f32(r.threshold),
    gain: f32(r.gain),
    scan_time_ms: r.scanTimeMs,
    retrigger_time_ms: r.retriggerTimeMs,
    num_hits_detected: Number(r.numHitsDetected),
    noise_floor_peak: f32(r.noiseFloorPeak),
    max_hit_amplitude: f32(r.maxHitAmplitude),
  };
  if (r.highpassFreq !== undefined)
    calibration.highpass_freq = f32(r.highpassFreq);
  if (r.dynamicThresholdDecayMs !== undefined)
    calibration.dynamic_threshold_decay_ms = r.dynamicThresholdDecayMs;
  return calibration;
}
