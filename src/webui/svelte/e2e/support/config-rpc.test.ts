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

// The spec helpers that answer the config calls from the shapes the page
// holds are the inverse of the page's own mapping.
import { describe, expect, it } from "vitest";
import { create } from "@bufbuild/protobuf";
import {
  GetConfigResponseSchema,
  GetProfileFileResponseSchema,
  ListProfileFilesResponseSchema,
  ProfileFileSavedSchema,
  UpdateConfigResponseSchema,
} from "../../src/gen/player/v1/player_pb";
import {
  mapConfigSaved,
  mapConfigStore,
  mapFileSaved,
  mapProfileFile,
  mapProfileFiles,
} from "../../src/lib/grpc/config";
import {
  getConfigReply,
  getProfileFileReply,
  listProfileFilesReply,
  profileFileSavedReply,
  savedReply,
} from "./config-rpc";

describe("answers made from the page's own shapes", () => {
  it("a config read reads back as it was", () => {
    const snapshot = { yaml: "songs: s\n", checksum: "abc" };
    expect(
      mapConfigStore(create(GetConfigResponseSchema, getConfigReply(snapshot))),
    ).toEqual(snapshot);
  });

  it("an applied save reads back as one", () => {
    const snapshot = { yaml: "y", checksum: "c" };
    expect(
      mapConfigSaved(create(UpdateConfigResponseSchema, savedReply(snapshot))),
    ).toEqual({ ...snapshot, applied: true, reload: "applied" });
  });

  it("a deferred save reads back with what it waits for", () => {
    const snapshot = { yaml: "y", checksum: "c" };
    const reload = {
      applied: false,
      reload: "deferred",
      message: "waits",
      songs_error: "no songs",
      awaits_reload: 2,
      awaits_process: "99",
    };
    expect(
      mapConfigSaved(
        create(UpdateConfigResponseSchema, savedReply(snapshot, reload)),
      ),
    ).toEqual({ ...snapshot, ...reload });
  });
});

describe("answers made from the profile files' own shapes", () => {
  it("the list reads back as it was", () => {
    const files = [
      {
        file: "a.yaml",
        filename: "a",
        addressable: true,
        error: null,
        hostname: "host-a",
        has_audio: true,
        has_midi: false,
        has_dmx: false,
        has_trigger: true,
        has_controllers: false,
      },
      {
        file: "b.yml",
        filename: "b",
        addressable: false,
        error: "does not parse",
        hostname: null,
        has_audio: false,
        has_midi: false,
        has_dmx: false,
        has_trigger: false,
        has_controllers: false,
      },
    ];
    expect(
      mapProfileFiles(
        create(ListProfileFilesResponseSchema, listProfileFilesReply(files)),
      ),
    ).toEqual(files);
  });

  it("a file reads back as it was", () => {
    const file = {
      profile: { hostname: "h", audio: { device: "d" } },
      yaml: "hostname: h\n",
      version: "v1",
    };
    expect(
      mapProfileFile(
        create(GetProfileFileResponseSchema, getProfileFileReply(file)),
      ),
    ).toEqual(file);
  });

  it("a save reads back with its version and an applied reload", () => {
    expect(
      mapFileSaved(
        create(
          ProfileFileSavedSchema,
          profileFileSavedReply({ filename: "a", version: "v2" }),
        ),
      ),
    ).toEqual({
      status: "saved",
      filename: "a",
      version: "v2",
      applied: true,
      reload: "applied",
    });
  });

  it("a delete reads back with no version and a deferred reload", () => {
    const reload = {
      applied: false,
      reload: "deferred",
      message: "waits",
      awaits_reload: 1,
      awaits_process: "7",
    };
    expect(
      mapFileSaved(
        create(
          ProfileFileSavedSchema,
          profileFileSavedReply({ status: "deleted", filename: "a" }, reload),
        ),
      ),
    ).toEqual({ status: "deleted", filename: "a", ...reload });
  });
});
