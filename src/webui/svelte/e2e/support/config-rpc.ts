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
//   rpc.unary(ConfigService.method.listProfileFiles, listProfileFilesReply(files));
//
// The mock server answers the same calls with the same helpers.
import type { MessageInitShape } from "@bufbuild/protobuf";
import {
  ProbeOutcome,
  type CalibrationNoiseFloorSchema,
  type CalibrationResultSchema,
  type GetConfigResponseSchema,
  type GetProfileFileResponseSchema,
  type ListAudioDevicesResponseSchema,
  type ListMidiDevicesResponseSchema,
  type ListProfileFilesResponseSchema,
  type ProbeAudioDeviceResponseSchema,
  type ProfileFileSavedSchema,
  type RestartControllersResponseSchema,
  type UpdateConfigResponseSchema,
} from "../../src/gen/player/v1/player_pb";
import type {
  AudioDeviceInfo,
  AudioProbe,
  ChannelCalibration,
  ConfigReload,
  ControllerStatus,
  MidiDeviceInfo,
  NoiseFloor,
  Profile,
  ProfileFileInfo,
  ProbeKind,
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
  return {
    yaml: snapshot.yaml,
    checksum: snapshot.checksum,
    ...reloadFields(reload),
  };
}

/** The reload facts of a save's answer, from the shape the page holds. */
function reloadFields(reload: Partial<ConfigReload>) {
  const applied = reload.applied ?? true;
  return {
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

/** The profile files of the profiles directory. */
export function listProfileFilesReply(
  files: ProfileFileInfo[],
): MessageInitShape<typeof ListProfileFilesResponseSchema> {
  return {
    files: files.map((f) => ({
      file: f.file,
      filename: f.filename,
      addressable: f.addressable,
      error: f.error ?? undefined,
      hostname: f.hostname ?? undefined,
      hasAudio: f.has_audio,
      hasMidi: f.has_midi,
      hasDmx: f.has_dmx,
      hasTrigger: f.has_trigger,
      hasControllers: f.has_controllers,
    })),
  };
}

/** One profile file as read: the profile, the file as written and its
 *  version (none when a spec does not care). */
export function getProfileFileReply(file: {
  profile: Profile;
  yaml: string;
  version?: string;
}): MessageInitShape<typeof GetProfileFileResponseSchema> {
  return {
    profileJson: JSON.stringify(file.profile),
    yaml: file.yaml,
    version: file.version ?? "",
  };
}

/** A profile file saved (or, with `status: "deleted"`, deleted) and what
 *  became of the hardware reload (applied at once unless the spec says
 *  otherwise). */
export function profileFileSavedReply(
  saved: { status?: string; filename?: string; version?: string } = {},
  reload: Partial<ConfigReload> = {},
): MessageInitShape<typeof ProfileFileSavedSchema> {
  return {
    status: saved.status ?? "saved",
    filename: saved.filename ?? "",
    version: saved.version,
    ...reloadFields(reload),
  };
}

/** The audio devices the picker lists. */
export function listAudioDevicesReply(
  devices: AudioDeviceInfo[],
): MessageInitShape<typeof ListAudioDevicesResponseSchema> {
  return {
    devices: devices.map((d) => ({
      name: d.name,
      maxChannels: d.max_channels,
      hostName: d.host_name,
      supportedSampleRates: d.supported_sample_rates,
      supportedFormats: d.supported_formats.map((f) => ({
        sampleFormat: f.sample_format,
        bitsPerSample: f.bits_per_sample,
      })),
      virtualNode: d.virtual_node,
      channelsKnown: d.channels_known,
    })),
  };
}

/** The MIDI devices the picker lists. */
export function listMidiDevicesReply(
  devices: MidiDeviceInfo[],
): MessageInitShape<typeof ListMidiDevicesResponseSchema> {
  return {
    devices: devices.map((d) => ({
      name: d.name,
      hasInput: d.has_input,
      hasOutput: d.has_output,
    })),
  };
}

const PROBE_OUTCOMES: Record<ProbeKind, ProbeOutcome> = {
  streaming: ProbeOutcome.STREAMING,
  opened_but_silent: ProbeOutcome.OPENED_BUT_SILENT,
  could_not_open: ProbeOutcome.COULD_NOT_OPEN,
  opened_cannot_report: ProbeOutcome.OPENED_CANNOT_REPORT,
  in_use: ProbeOutcome.IN_USE,
};

/** An audio probe's answer. A held device's health is not carried here: a
 *  spec that needs it names the message itself. */
export function probeAudioReply(
  probe: AudioProbe,
): MessageInitShape<typeof ProbeAudioDeviceResponseSchema> {
  return {
    outcome: PROBE_OUTCOMES[probe.outcome],
    ok: probe.ok,
    reason: probe.reason ?? undefined,
    callbacks:
      probe.callbacks !== null && probe.callbacks !== undefined
        ? BigInt(probe.callbacks)
        : undefined,
  };
}

/** A calibration's noise floor. */
export function noiseFloorReply(
  floor: NoiseFloor,
): MessageInitShape<typeof CalibrationNoiseFloorSchema> {
  return {
    peak: floor.peak,
    rms: floor.rms,
    lowFreqEnergy: floor.low_freq_energy,
    channel: floor.channel,
    sampleRate: floor.sample_rate,
    deviceChannels: floor.device_channels,
  };
}

/** A calibration's result. */
export function calibrationReply(
  c: ChannelCalibration,
): MessageInitShape<typeof CalibrationResultSchema> {
  return {
    channel: c.channel,
    threshold: c.threshold,
    gain: c.gain,
    scanTimeMs: c.scan_time_ms,
    retriggerTimeMs: c.retrigger_time_ms,
    highpassFreq: c.highpass_freq,
    dynamicThresholdDecayMs: c.dynamic_threshold_decay_ms,
    numHitsDetected: BigInt(c.num_hits_detected),
    noiseFloorPeak: c.noise_floor_peak,
    maxHitAmplitude: c.max_hit_amplitude,
  };
}

/** The controllers after a restart. */
export function restartControllersReply(
  controllers: ControllerStatus[],
): MessageInitShape<typeof RestartControllersResponseSchema> {
  return {
    controllers: controllers.map((c) => ({
      kind: c.kind,
      status: c.status,
      detail: c.detail ?? undefined,
      error: c.error ?? undefined,
    })),
  };
}
