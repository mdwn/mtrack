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

import { afterEach, describe, expect, it, vi } from "vitest";
import { Code, ConnectError } from "@connectrpc/connect";
import { create } from "@bufbuild/protobuf";
import {
  AddSampleFileResponseSchema,
  AddTrackFilesResponseSchema,
  GdtfImportSchema,
  ImportMvrResponseSchema,
  MvrInspectionSchema,
  MvrOptionsSchema,
} from "../../gen/player/v1/player_pb";
import { ApiError } from "../api/errors";
import { BadAnswerError } from "../lighting/answer";
import { MVR_INSPECTION } from "../../../e2e/mock-server/test-data";
import {
  gdtfImportReply,
  importKind,
  mvrImportReply,
  mvrInspectionReply,
} from "../../../e2e/support/lighting-rpc";
import {
  importCallError,
  mapGdtfImport,
  mapMvrImportAnswer,
  mapMvrInspection,
  mapSampleUploaded,
  mapTrackUploaded,
  mapTracksUploaded,
  mvrOptions,
} from "./uploads";

afterEach(() => vi.restoreAllMocks());

describe("a track upload's answer", () => {
  const added = (replaced: boolean) =>
    create(AddTrackFilesResponseSchema, {
      song: "Song (2)",
      files: [{ file: "kick.wav", replaced }],
    });

  it("says whether it replaced a file, and the name the song has", () => {
    expect(mapTrackUploaded(added(false))).toEqual({
      status: "uploaded",
      replaced: false,
      song: "Song (2)",
      file: "kick.wav",
    });
    expect(mapTrackUploaded(added(true))).toMatchObject({
      status: "replaced",
      replaced: true,
    });
  });

  it("is not understood without a file", () => {
    expect(() =>
      mapTrackUploaded(create(AddTrackFilesResponseSchema, { song: "S" })),
    ).toThrow(BadAnswerError);
  });

  it("lists every file of an upload of several", () => {
    expect(
      mapTracksUploaded(
        create(AddTrackFilesResponseSchema, {
          song: "S",
          files: [{ file: "a.wav" }, { file: "b.wav", replaced: true }],
        }),
      ),
    ).toEqual({ status: "uploaded", song: "S", files: ["a.wav", "b.wav"] });
  });
});

it("a sample's answer is the path a definition names it by", () => {
  expect(
    mapSampleUploaded(
      create(AddSampleFileResponseSchema, {
        file: "kick.wav",
        path: "samples/kick.wav",
      }),
    ),
  ).toEqual({
    status: "uploaded",
    file: "kick.wav",
    path: "samples/kick.wav",
  });
});

describe("a GDTF import's answer", () => {
  const report = {
    type_name: "pixelbrick",
    fixture: "PB15 PixelBrick",
    manufacturer: "Astera",
    modes: 2,
    archive: "lighting/library/pb15.gdtf",
    already_imported: false,
    renamed_from: null,
    refused_modes: [{ mode: "Weird", reason: "no channels" }],
    fixture_file: null,
  };

  it("reads back as the page holds it", () => {
    expect(
      mapGdtfImport(create(GdtfImportSchema, gdtfImportReply(report))),
    ).toEqual(report);
    const pinned = {
      ...report,
      renamed_from: "PB15",
      fixture_file: "lighting/fixture_types/pb.fixture",
    };
    expect(
      mapGdtfImport(create(GdtfImportSchema, gdtfImportReply(pinned))),
    ).toEqual(pinned);
  });
});

describe("an MVR's answers", () => {
  it("an inspection reads back as the wizard holds it", () => {
    expect(
      mapMvrInspection(
        create(MvrInspectionSchema, mvrInspectionReply(MVR_INSPECTION)),
      ),
    ).toEqual(MVR_INSPECTION);
  });

  it("an inspection without its plan or scene is not understood", () => {
    expect(() =>
      mapMvrInspection(create(MvrInspectionSchema, { fileName: "a.mvr" })),
    ).toThrow(BadAnswerError);
  });

  it("an import's plan reads back, with nothing of a write", () => {
    const answer = { write: false as const, plan: MVR_INSPECTION.report };
    const read = mapMvrImportAnswer(
      create(ImportMvrResponseSchema, mvrImportReply(answer)),
    );
    expect(read).toEqual(answer);
    expect("reloaded" in read).toBe(false);
    expect("venue_error" in read).toBe(false);
  });

  it("an import's report reads back with the reload and the venue's failure", () => {
    const report = {
      ...MVR_INSPECTION.report,
      written: ["lighting/venues/kellys.venue"],
      distillation_warnings: { "Astera PB15": ["8: RGBS: a warning"] },
    };
    const clean = {
      write: true as const,
      report,
      reloaded: true,
      venue_error: null,
    };
    expect(
      mapMvrImportAnswer(
        create(ImportMvrResponseSchema, mvrImportReply(clean)),
      ),
    ).toEqual(clean);
    const failing = {
      ...clean,
      reloaded: false,
      venue_error: { venue: "Kellys", fixture: "Brick 2", reason: "no mode" },
    };
    expect(
      mapMvrImportAnswer(
        create(ImportMvrResponseSchema, mvrImportReply(failing)),
      ),
    ).toEqual(failing);
  });

  it("a write with no report is not understood", () => {
    expect(() =>
      mapMvrImportAnswer(create(ImportMvrResponseSchema, { write: true })),
    ).toThrow(BadAnswerError);
  });
});

describe("the choices of an inspection or import", () => {
  it("leave out what was not given", () => {
    const options = create(
      MvrOptionsSchema,
      mvrOptions({ name: "", venuesDir: "", fixtureTypesDir: undefined }),
    );
    expect(options.name).toBeUndefined();
    expect(options.originMm).toBeUndefined();
    expect(options.venuesDir).toBeUndefined();
    expect(options.fixtureTypesDir).toBeUndefined();
    expect(options.keep).toBeUndefined();
  });

  it("carry the origin in millimeters and the hand edits a merge keeps", () => {
    const options = create(
      MvrOptionsSchema,
      mvrOptions({
        name: "Kellys",
        origin: [0, 1000, 500],
        venuesDir: "v",
        fixtureTypesDir: "t",
        keep: {
          fixtures: { "Brick 1": ["position", "patch"] },
          focus_points: ["Drummer"],
        },
      }),
    );
    expect(options.name).toBe("Kellys");
    expect(options.originMm).toMatchObject({ x: 0, y: 1000, z: 500 });
    expect(options.venuesDir).toBe("v");
    expect(options.fixtureTypesDir).toBe("t");
    expect(options.keep?.fixtures["Brick 1"].fields).toEqual([
      "position",
      "patch",
    ]);
    expect(options.keep?.focusPoints).toEqual(["Drummer"]);
  });
});

describe("a refused import", () => {
  it("carries the kind of refusal the server named", () => {
    const refused = new ConnectError(
      "cannot import venue",
      Code.FailedPrecondition,
      importKind("venue"),
    );
    const err = importCallError(refused, "Failed to read the MVR");
    expect(err).toBeInstanceOf(ApiError);
    expect(err.message).toBe("cannot import venue");
    expect((err as ApiError).kind).toBe("venue");
  });

  it("has no kind when the server named none", () => {
    const err = importCallError(
      new ConnectError("not a zip", Code.InvalidArgument),
      "Failed to import GDTF",
    );
    expect((err as ApiError).kind).toBeNull();
  });

  it("passes on what is not a failed call", () => {
    const own = new TypeError("the page's own");
    expect(importCallError(own, "Failed")).toBe(own);
  });
});
