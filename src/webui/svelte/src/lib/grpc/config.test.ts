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
  UpdateConfigResponseSchema,
} from "../../gen/player/v1/player_pb";
import { mapConfigSaved, mapConfigStore } from "./config";

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
