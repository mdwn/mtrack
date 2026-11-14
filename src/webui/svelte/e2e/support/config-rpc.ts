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
import { Code, ConnectError } from "@connectrpc/connect";
import {
  ConfigService,
  FileKind as FileKindMessage,
  ProbeOutcome,
  TempoGuessSource,
  type BluetoothStatusSchema,
  type BrowseDirectoryResponseSchema,
  type BulkImportSongsResponseSchema,
  type CalibrationNoiseFloorSchema,
  type CalibrationResultSchema,
  type CreateSongInDirectoryResponseSchema,
  type CreateSongResponseSchema,
  type GetConfigResponseSchema,
  type GetSongConfigResponseSchema,
  type GuessSongTempoResponseSchema,
  type ImportSongFileResponseSchema,
  type ListSongFilesResponseSchema,
  type WaveformSchema,
  type GetProfileFileResponseSchema,
  type ListAudioDevicesResponseSchema,
  type ListMidiDevicesResponseSchema,
  type ListProfileFilesResponseSchema,
  type ProbeAudioDeviceResponseSchema,
  type ProfileFileSavedSchema,
  type RestartControllersResponseSchema,
  type UpdateConfigResponseSchema,
} from "../../src/gen/player/v1/player_pb";
import type { RpcMock } from "./rpc";
import type {
  AudioDeviceInfo,
  AudioProbe,
  BluetoothStatus,
  Browse,
  BulkImport,
  ChannelCalibration,
  ConfigReload,
  ControllerStatus,
  FileKind,
  MidiDeviceInfo,
  NoiseFloor,
  Profile,
  ProfileFileInfo,
  ProbeKind,
  SongFile,
  StoreSnapshot,
  TempoGuess,
  Waveform,
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

/** The Bluetooth controller, as `GetBluetooth` and every change to it
 *  answers. */
export function bluetoothReply(
  status: BluetoothStatus,
): MessageInitShape<typeof BluetoothStatusSchema> {
  return {
    configured: status.configured,
    running: status.running,
    error: status.error ?? undefined,
    pairing: {
      open: status.pairing.open,
      secondsLeft: status.pairing.seconds_left,
    },
    devices: status.devices.map((d) => ({
      address: d.address,
      name: d.name ?? undefined,
      connected: d.connected,
    })),
  };
}

// ---- Songs ----

const FILE_KINDS: Record<FileKind, FileKindMessage> = {
  directory: FileKindMessage.DIRECTORY,
  audio: FileKindMessage.AUDIO,
  midi: FileKindMessage.MIDI,
  lighting: FileKindMessage.LIGHTING,
  other: FileKindMessage.OTHER,
};

/** A song's config file. */
export function getSongConfigReply(
  yaml: string,
): MessageInitShape<typeof GetSongConfigResponseSchema> {
  return { yaml };
}

/** A song's waveform, track by track. */
export function waveformReply(
  waveform: Waveform,
): MessageInitShape<typeof WaveformSchema> {
  return {
    songName: waveform.song_name,
    tracks: waveform.tracks.map((t) => ({ name: t.name, peaks: t.peaks })),
  };
}

/** The files in a song's folder. */
export function listSongFilesReply(
  files: SongFile[],
): MessageInitShape<typeof ListSongFilesResponseSchema> {
  return {
    files: files.map((f) => ({ name: f.name, kind: FILE_KINDS[f.type] })),
  };
}

/** A folder of the project, as the file browser lists it. */
export function browseReply(
  browse: Browse,
): MessageInitShape<typeof BrowseDirectoryResponseSchema> {
  return {
    path: browse.path,
    root: browse.root,
    entries: browse.entries.map((e) => ({
      name: e.name,
      path: e.path,
      kind: FILE_KINDS[e.type],
      isDir: e.is_dir,
    })),
  };
}

/** What a bulk import made of each folder. */
export function bulkImportReply(
  imported: BulkImport,
): MessageInitShape<typeof BulkImportSongsResponseSchema> {
  return {
    created: imported.created,
    skipped: imported.skipped,
    failed: imported.failed,
  };
}

/** A song created in a folder of the project. */
export function createInDirectoryReply(created: {
  path?: string;
  song: string;
}): MessageInitShape<typeof CreateSongInDirectoryResponseSchema> {
  return { path: created.path ?? "", song: created.song };
}

/** A song created. */
export function createSongReply(
  song: string,
): MessageInitShape<typeof CreateSongResponseSchema> {
  return { song };
}

/** A file imported into a song. */
export function importSongFileReply(imported: {
  file: string;
  song: string;
}): MessageInitShape<typeof ImportSongFileResponseSchema> {
  return imported;
}

/** A tempo guess. */
export function tempoGuessReply(
  guess: TempoGuess,
): MessageInitShape<typeof GuessSongTempoResponseSchema> {
  const [numerator, denominator] = guess.tempo.time_signature;
  return {
    source:
      guess.source === "midi"
        ? TempoGuessSource.MIDI
        : TempoGuessSource.BEAT_GRID,
    tempo: {
      startSeconds: guess.tempo.start_seconds,
      bpm: guess.tempo.bpm,
      timeSignatureNumerator: numerator,
      timeSignatureDenominator: denominator,
      alignmentRmsMs: guess.tempo.alignment_rms_ms,
      changes: guess.tempo.changes.map((c) => ({
        measure: c.measure,
        beat: c.beat,
        bpm: c.bpm,
        timeSignatureNumerator: c.time_signature[0],
        timeSignatureDenominator: c.time_signature[1],
        transitionBeats: c.transition_beats,
      })),
    },
    config: {
      bpm: guess.config.bpm,
      timeSignature: guess.config.time_signature ?? "4/4",
      start: guess.config.start,
      changes: (guess.config.changes ?? []).map((c) => ({
        measure: c.measure,
        beat: c.beat,
        bpm: c.bpm,
        timeSignature: c.time_signature,
        transition: c.transition
          ? {
              ramp:
                "beats" in c.transition
                  ? { case: "beats" as const, value: c.transition.beats }
                  : { case: "measures" as const, value: c.transition.measures },
            }
          : undefined,
      })),
    },
  };
}

/** Makes `rpc` serve song `name`'s config as `yaml`, and no other song's.
 *  The saves it gets are `rpc.calls(ConfigService.method.saveSongConfig)`. */
export function serveSongConfig(
  rpc: RpcMock,
  name: string,
  yaml: string,
): void {
  rpc.unary(ConfigService.method.getSongConfig, (req) => {
    if (req.name !== name) {
      throw new ConnectError(`unable to find song ${req.name}`, Code.NotFound);
    }
    return getSongConfigReply(yaml);
  });
}
