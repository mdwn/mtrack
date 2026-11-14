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

import { describe, expect, it } from "vitest";
import { create } from "@bufbuild/protobuf";
import { Code, ConnectError } from "@connectrpc/connect";
import {
  AudioOutputHealthSchema,
  CalibrationNoiseFloorSchema,
  CalibrationResultSchema,
  GetConfigResponseSchema,
  GetProfileFileResponseSchema,
  ListAudioDevicesResponseSchema,
  ListMidiDevicesResponseSchema,
  ListProfileFilesResponseSchema,
  ProbeAudioDeviceRequestSchema,
  ProbeAudioDeviceResponseSchema,
  ProbeOutcome,
  ProfileFileSavedSchema,
  UpdateConfigResponseSchema,
} from "../../gen/player/v1/player_pb";
import {
  f32,
  mapAudioDevices,
  mapAudioProbe,
  mapCalibration,
  mapConfigSaved,
  mapConfigStore,
  mapFileSaved,
  mapMidiDevices,
  mapNoiseFloor,
  mapProfileFile,
  mapProfileFiles,
  metronomeJson,
  probeAudioRequest,
  startCalibrationRequest,
} from "./config";
import { callError } from "./library";

describe("the config read", () => {
  it("is the yaml and the checksum a save presents", () => {
    expect(
      mapConfigStore(
        create(GetConfigResponseSchema, {
          yaml: "songs: s\n",
          checksum: "abc",
        }),
      ),
    ).toEqual({ yaml: "songs: s\n", checksum: "abc" });
  });
});

describe("a config save's answer", () => {
  it("has the config after the save and an applied reload, and nothing else", () => {
    const saved = mapConfigSaved(
      create(UpdateConfigResponseSchema, {
        yaml: "y",
        checksum: "c",
        applied: true,
        reload: "applied",
      }),
    );
    expect(saved).toEqual({
      yaml: "y",
      checksum: "c",
      applied: true,
      reload: "applied",
    });
    // The notice reads an absent message, not an empty one.
    expect("message" in saved).toBe(false);
  });

  it("carries a deferred reload's generation and process, and a songs error", () => {
    expect(
      mapConfigSaved(
        create(UpdateConfigResponseSchema, {
          yaml: "y",
          checksum: "c",
          applied: false,
          reload: "deferred",
          message: "waits until idle",
          songsError: "no songs",
          awaitsReload: 3n,
          awaitsProcess: "1234",
        }),
      ),
    ).toEqual({
      yaml: "y",
      checksum: "c",
      applied: false,
      reload: "deferred",
      message: "waits until idle",
      songs_error: "no songs",
      awaits_reload: 3,
      awaits_process: "1234",
    });
  });

  it("is read by applied when the server does not say which reload it was", () => {
    const reload = (applied: boolean) =>
      mapConfigSaved(create(UpdateConfigResponseSchema, { applied })).reload;
    expect(reload(true)).toBe("applied");
    expect(reload(false)).toBe("deferred");
  });

  it("keeps a generation of zero", () => {
    expect(
      mapConfigSaved(
        create(UpdateConfigResponseSchema, {
          reload: "deferred",
          awaitsReload: 0n,
        }),
      ).awaits_reload,
    ).toBe(0);
  });
});

describe("the profile files", () => {
  it("are listed with what the server leaves out as null", () => {
    expect(
      mapProfileFiles(
        create(ListProfileFilesResponseSchema, {
          files: [
            {
              file: "stage.yml",
              filename: "stage",
              addressable: true,
              hostname: "pi-a",
              hasAudio: true,
              hasControllers: true,
            },
            {
              file: "broken.yaml",
              filename: "broken",
              error: "bad",
            },
          ],
        }),
      ),
    ).toEqual([
      {
        file: "stage.yml",
        filename: "stage",
        addressable: true,
        error: null,
        hostname: "pi-a",
        has_audio: true,
        has_midi: false,
        has_dmx: false,
        has_trigger: false,
        has_controllers: true,
      },
      {
        file: "broken.yaml",
        filename: "broken",
        addressable: false,
        error: "bad",
        hostname: null,
        has_audio: false,
        has_midi: false,
        has_dmx: false,
        has_trigger: false,
        has_controllers: false,
      },
    ]);
  });

  it("is read as a profile, the file as written and its version", () => {
    expect(
      mapProfileFile(
        create(GetProfileFileResponseSchema, {
          profileJson: '{"hostname":"h","audio":{"device":"d"}}',
          yaml: "hostname: h\n",
          version: "abc",
        }),
      ),
    ).toEqual({
      profile: { hostname: "h", audio: { device: "d" } },
      yaml: "hostname: h\n",
      version: "abc",
    });
  });

  it("is saved with its new version and what became of the reload", () => {
    expect(
      mapFileSaved(
        create(ProfileFileSavedSchema, {
          status: "saved",
          filename: "stage.yml",
          version: "v2",
          applied: false,
          reload: "deferred",
          message: "waits until idle",
          awaitsReload: 2n,
          awaitsProcess: "99",
          songsError: "no songs",
        }),
      ),
    ).toEqual({
      status: "saved",
      filename: "stage.yml",
      version: "v2",
      applied: false,
      reload: "deferred",
      message: "waits until idle",
      awaits_reload: 2,
      awaits_process: "99",
      songs_error: "no songs",
    });
  });

  it("is deleted with no version, and read by applied from a server that does not say which reload", () => {
    const deleted = mapFileSaved(
      create(ProfileFileSavedSchema, {
        status: "deleted",
        filename: "a",
        applied: true,
      }),
    );
    expect(deleted).toEqual({
      status: "deleted",
      filename: "a",
      applied: true,
      reload: "applied",
    });
    expect("version" in deleted).toBe(false);
    expect("message" in deleted).toBe(false);
  });
});

describe("the metronome defaults", () => {
  it("send only what differs from the defaults", () => {
    expect(
      JSON.parse(metronomeJson({ accent: "a.wav" } as never, true, 0.5)),
    ).toEqual({ enabled: true, volume: 0.5, sounds: { accent: "a.wav" } });
    expect(JSON.parse(metronomeJson(null, true, 1))).toEqual({ enabled: true });
    expect(JSON.parse(metronomeJson(null, false, 0.8))).toEqual({
      volume: 0.8,
    });
  });

  it("send null, which removes them, when nothing differs", () => {
    expect(metronomeJson(null, false, null)).toBe("null");
    expect(metronomeJson({}, false, 1)).toBe("null");
  });
});

describe("the device lists", () => {
  it("map an audio device with what it supports", () => {
    const [device] = mapAudioDevices(
      create(ListAudioDevicesResponseSchema, {
        devices: [
          {
            name: "USB Interface",
            maxChannels: 18,
            hostName: "ALSA",
            supportedSampleRates: [44100, 48000],
            supportedFormats: [{ sampleFormat: "int", bitsPerSample: 24 }],
            virtualNode: false,
            channelsKnown: true,
          },
        ],
      }),
    );
    expect(device).toEqual({
      name: "USB Interface",
      max_channels: 18,
      host_name: "ALSA",
      supported_sample_rates: [44100, 48000],
      supported_formats: [{ sample_format: "int", bits_per_sample: 24 }],
      virtual_node: false,
      channels_known: true,
    });
  });

  it("map a MIDI device, and no devices to none", () => {
    expect(
      mapMidiDevices(
        create(ListMidiDevicesResponseSchema, {
          devices: [{ name: "Pad", hasInput: true, hasOutput: false }],
        }),
      ),
    ).toEqual([{ name: "Pad", has_input: true, has_output: false }]);
    expect(mapMidiDevices(create(ListMidiDevicesResponseSchema))).toEqual([]);
  });
});

describe("the audio probe", () => {
  it("asks with what the form has set, and leaves the rest out", () => {
    const bare = create(
      ProbeAudioDeviceRequestSchema,
      probeAudioRequest({ device: "d", sample_rate: null }),
    );
    expect(bare.device).toBe("d");
    expect(bare.sampleRate).toBeUndefined();
    expect(bare.bufferSize).toBeUndefined();
    expect(bare.streamBufferSize).toBeUndefined();

    const full = create(
      ProbeAudioDeviceRequestSchema,
      probeAudioRequest({
        device: "d",
        sample_rate: 48000,
        sample_format: "int",
        bits_per_sample: 24,
        buffer_size: 2048,
        stream_buffer_size: 1024,
      }),
    );
    expect(full.sampleRate).toBe(48000);
    expect(full.sampleFormat).toBe("int");
    expect(full.bitsPerSample).toBe(24);
    expect(full.bufferSize).toBe(2048n);
    expect(full.streamBufferSize?.size).toEqual({
      case: "frames",
      value: 1024n,
    });
  });

  it("says the stream buffer keywords as kinds", () => {
    const kind = (size: "default" | "min") =>
      create(
        ProbeAudioDeviceRequestSchema,
        probeAudioRequest({ device: "d", stream_buffer_size: size }),
      ).streamBufferSize?.size;
    expect(kind("min")).toEqual({ case: "kind", value: 2 });
    expect(kind("default")).toEqual({ case: "kind", value: 1 });
  });

  it("reads a device that streams", () => {
    expect(
      mapAudioProbe(
        create(ProbeAudioDeviceResponseSchema, {
          outcome: ProbeOutcome.STREAMING,
          ok: true,
          callbacks: 7n,
        }),
      ),
    ).toEqual({ outcome: "streaming", ok: true, reason: null, callbacks: 7 });
  });

  it("reads why a device would not open", () => {
    expect(
      mapAudioProbe(
        create(ProbeAudioDeviceResponseSchema, {
          outcome: ProbeOutcome.COULD_NOT_OPEN,
          ok: false,
          reason: "Unsupported bit depth",
        }),
      ),
    ).toEqual({
      outcome: "could_not_open",
      ok: false,
      reason: "Unsupported bit depth",
      callbacks: null,
    });
  });

  it("reads the held device from its health, or none for one that has none", () => {
    const held = mapAudioProbe(
      create(ProbeAudioDeviceResponseSchema, {
        outcome: ProbeOutcome.IN_USE,
        ok: true,
        callbacks: 4821n,
        health: create(AudioOutputHealthSchema, {
          status: "healthy",
          callbackAlive: true,
          callbacks: 4821n,
        }),
      }),
    );
    expect(held.outcome).toBe("in_use");
    expect(held.health?.status).toBe("healthy");
    expect(held.callbacks).toBe(4821);

    const mock = mapAudioProbe(
      create(ProbeAudioDeviceResponseSchema, {
        outcome: ProbeOutcome.IN_USE,
        ok: true,
      }),
    );
    expect(mock.health).toBeNull();
    expect(mock.callbacks).toBeNull();
  });

  it("refuses an outcome it does not know", () => {
    expect(() =>
      mapAudioProbe(create(ProbeAudioDeviceResponseSchema, { ok: true })),
    ).toThrow();
  });
});

describe("the calibration", () => {
  it("begins with the duration only when one is given", () => {
    expect(startCalibrationRequest("d", 2)).toEqual({
      device: "d",
      channel: 2,
      duration: undefined,
    });
    expect(startCalibrationRequest("d", 2, 5).duration).toBe(5);
  });

  it("reads the noise floor as the player measured it", () => {
    expect(
      mapNoiseFloor(
        create(CalibrationNoiseFloorSchema, {
          peak: 0.01,
          rms: 0.005,
          lowFreqEnergy: 0.002,
          channel: 3,
          sampleRate: 48000,
          deviceChannels: 8,
        }),
      ),
    ).toEqual({
      peak: 0.01,
      rms: 0.005,
      low_freq_energy: 0.002,
      channel: 3,
      sample_rate: 48000,
      device_channels: 8,
    });
  });

  it("reads a 32-bit float as the shortest number it prints as", () => {
    expect(f32(Math.fround(0.1))).toBe(0.1);
    expect(f32(Math.fround(0.8))).toBe(0.8);
    expect(f32(Math.fround(1234.5))).toBe(1234.5);
    expect(f32(Math.fround(1 / 3))).toBe(0.33333334);
    expect(f32(0)).toBe(0);
  });

  it("reads the calibrated parameters, the optional ones only when derived", () => {
    const fully = mapCalibration(
      create(CalibrationResultSchema, {
        channel: 1,
        threshold: Math.fround(0.1),
        gain: 1,
        scanTimeMs: 50,
        retriggerTimeMs: 200,
        highpassFreq: Math.fround(80.5),
        dynamicThresholdDecayMs: 150,
        numHitsDetected: 5n,
        noiseFloorPeak: Math.fround(0.01),
        maxHitAmplitude: Math.fround(0.8),
      }),
    );
    expect(fully).toEqual({
      channel: 1,
      threshold: 0.1,
      gain: 1,
      scan_time_ms: 50,
      retrigger_time_ms: 200,
      highpass_freq: 80.5,
      dynamic_threshold_decay_ms: 150,
      num_hits_detected: 5,
      noise_floor_peak: 0.01,
      max_hit_amplitude: 0.8,
    });
    const bare = mapCalibration(
      create(CalibrationResultSchema, { channel: 2 }),
    );
    expect(bare).not.toHaveProperty("highpass_freq");
    expect(bare).not.toHaveProperty("dynamic_threshold_decay_ms");
  });

  it("shows the player's refusal as it said it", () => {
    const refused = callError(
      new ConnectError(
        "No calibration session — call StartCalibration first",
        Code.FailedPrecondition,
      ),
      "Capture start failed",
    );
    expect(refused.message).toBe(
      "No calibration session — call StartCalibration first",
    );
  });
});
