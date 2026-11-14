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
import {
  GetConfigResponseSchema,
  GetProfileFileResponseSchema,
  ListProfileFilesResponseSchema,
  ProfileFileSavedSchema,
  UpdateConfigResponseSchema,
} from "../../gen/player/v1/player_pb";
import {
  mapConfigSaved,
  mapConfigStore,
  mapFileSaved,
  mapProfileFile,
  mapProfileFiles,
  metronomeJson,
} from "./config";

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
