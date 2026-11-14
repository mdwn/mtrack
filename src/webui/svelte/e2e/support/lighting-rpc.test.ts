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

// The spec helpers that answer the lighting calls from the shapes the page
// holds are the inverse of the page's own mapping (src/lib/grpc/lighting.ts).
import { describe, expect, it } from "vitest";
import { create } from "@bufbuild/protobuf";
import {
  DeleteFixtureTypeResponseSchema,
  FixtureTestAnswerSchema,
  FixtureTestOptionsSchema,
  FixtureTypeGdtfDetailsSchema,
  FixtureTypeInfoSchema,
  FixtureTypeSettingsSavedSchema,
  FixtureTypeSettingsSchema,
  GetLightShowResponseSchema,
  LightingFitSchema,
  LightingPreviewSchema,
  LightingReadinessSchema,
  ListFixtureTypesResponseSchema,
  ListLightingGroupsResponseSchema,
  SaveFixtureTypeRequestSchema,
  SaveFixtureTypeResponseSchema,
  AddVenueAimPointsResponseSchema,
  ExportMvrResponseSchema,
  KeepMvrExportResponseSchema,
  ListVenuesResponseSchema,
  MvrExportSummarySchema,
  SaveFixtureTypeSettingsRequestSchema,
  SaveVenueRequestSchema,
  SaveVenueResponseSchema,
  ValidateLightShowResponseSchema,
  VenueDetailSchema,
  VenuePatchSchema,
  VenueSceneSchema,
} from "../../src/gen/player/v1/player_pb";
import {
  mapAimPoints,
  mapFit,
  mapFixtureSettings,
  mapFixtureTypeDeleted,
  mapFixtureTypeEntry,
  mapFixtureTypeGdtf,
  mapFixtureTypeList,
  mapFixtureTypeSaved,
  mapGroups,
  mapMvrExportSummary,
  mapPreview,
  mapReadiness,
  mapSettingsSaved,
  mapTestAnswer,
  mapTestOptions,
  mapVenueDetail,
  mapVenueList,
  mapVenuePatch,
  mapVenueSaved,
  mapVenueScene,
  saveFixtureSettingsRequest,
  saveFixtureTypeRequest,
  saveVenueRequest,
} from "../../src/lib/grpc/lighting";
import type {
  FixtureSettingsData,
  FixtureTypeGdtf,
  FixtureTypeList,
  SettingsSaved,
  FitReport,
  Preview,
  Readiness,
  TestAnswer,
  TestOptions,
} from "../../src/gen/rules/mtrack_rules";
import { FIT, READINESS } from "../mock-server/test-data";
import {
  aimPointsReply,
  exportMvrReply,
  fitReply,
  fixtureSettingsReply,
  fixtureTypeDeletedReply,
  fixtureTypeGdtfReply,
  fixtureTypeInfoReply,
  fixtureTypeListReply,
  fixtureTypeSavedReply,
  groupsReply,
  importKind,
  keptReply,
  mvrExportSummaryReply,
  savedSettingsRequest,
  savedTypeRequest,
  savedVenueRequest,
  settingsSavedReply,
  typeEntry,
  venueDetailReply,
  venueListReply,
  venuePatchReply,
  venueSavedReply,
  venueSceneReply,
  lightShowReply,
  previewReply,
  readinessReply,
  testAnswerReply,
  testOptionsReply,
  validationReply,
} from "./lighting-rpc";

describe("answers made from the page's own shapes", () => {
  it("a light show reads back as it was", () => {
    const source = 'show "Main" {\n}\n';
    expect(
      create(GetLightShowResponseSchema, lightShowReply(source)).source,
    ).toBe(source);
  });

  it("a validation reads back with its problems", () => {
    const r = create(
      ValidateLightShowResponseSchema,
      validationReply({ valid: false, errors: ["line 1: bad", "line 2: bad"] }),
    );
    expect([r.valid, r.errors]).toEqual([
      false,
      ["line 1: bad", "line 2: bad"],
    ]);
    const ok = create(
      ValidateLightShowResponseSchema,
      validationReply({ valid: true }),
    );
    expect([ok.valid, ok.errors]).toEqual([true, []]);
  });

  it("groups read back, resolved or not", () => {
    const groups = [{ name: "front", fixtures: ["a", "b"] }];
    expect(
      mapGroups(create(ListLightingGroupsResponseSchema, groupsReply(groups))),
    ).toEqual(groups);
    expect(
      create(ListLightingGroupsResponseSchema, groupsReply(groups, false))
        .resolved,
    ).toBe(false);
  });

  it("the mock server's readiness reads back as it was", () => {
    expect(
      mapReadiness(create(LightingReadinessSchema, readinessReply(READINESS))),
    ).toEqual(READINESS);
  });

  it("every kind of summary, finding and fix reads back", () => {
    const warning = {
      kind: "empty-group",
      message: "no fixtures",
      severity: "attention" as const,
    };
    const report: Readiness = {
      dmx: true,
      venue: { name: "v", fixtures: 3, placed: 1, focus_points: ["a", "b"] },
      venue_error: { venue: "v", fixture: "B", reason: "no mode" },
      patch_warnings: [{ ...warning, severity: "note" }],
      fixture_types: {
        in_use: ["Par"],
        unresolved: [{ fixture: "G", type: "Ghost", reason: "missing" }],
        library: [{ kind: "gdtf-unreadable", file: "j.gdtf", message: "bad" }],
      },
      groups: [{ name: "g", fixtures: 0, songs: ["S"] }],
      shows: [
        { song: "S", files: ["show.light"], warnings: [warning] },
        { song: "B", files: [], error: "no duration", warnings: [] },
      ],
      output: {
        universes: [1, 4],
        unconfigured: [4],
        olad: { reachable: false, unpatched: [1] },
      },
      checks: [
        {
          key: "fixtures",
          state: "blocked",
          summary: [
            { kind: "blocked", count: 1 },
            { kind: "attention", count: 2 },
            { kind: "unknown" },
            { kind: "noDmx" },
            { kind: "needsVenue" },
            { kind: "showsNone" },
            { kind: "groupsNone" },
            { kind: "showsBlocked", count: 3 },
            { kind: "showsReady", count: 4 },
            { kind: "typesReady", count: 5 },
            { kind: "groupsReady", count: 6 },
            { kind: "outputReady", count: 7 },
            { kind: "venueAllPlaced", venue: "v", fixtures: 3, unplaced: 0 },
            { kind: "venueSomeUnplaced", venue: "v", fixtures: 3, unplaced: 2 },
          ],
          findings: [
            {
              severity: "blocked",
              what: { kind: "showError", song: "B", detail: "no duration" },
              fix: { kind: "song", song: "B" },
              song: "B",
            },
            {
              severity: "attention",
              what: { kind: "lint", detail: "d" },
              fix: { kind: "fit", group: "g" },
              song: null,
            },
            {
              severity: "blocked",
              what: { kind: "noDmx" },
              fix: { kind: "config" },
              song: null,
            },
            {
              severity: "blocked",
              what: {
                kind: "typeUnresolved",
                fixture: "G",
                type: "Ghost",
                reason: "r",
              },
              fix: { kind: "fixtures" },
              song: null,
            },
            {
              severity: "blocked",
              what: { kind: "noVenue" },
              fix: { kind: "chooseVenue" },
              song: null,
            },
            {
              severity: "blocked",
              what: {
                kind: "venueFailed",
                venue: "v",
                fixture: "B",
                reason: "r",
              },
              fix: { kind: "venues" },
              song: null,
            },
            {
              severity: "note",
              what: { kind: "emptyVenue", venue: "v" },
              fix: { kind: "fit", group: null },
              song: null,
            },
            {
              severity: "attention",
              what: { kind: "patch", detail: "overlap" },
              fix: { kind: "venues" },
              song: null,
            },
            {
              severity: "attention",
              what: { kind: "groupEmpty", group: "g", venue: "v", songs: "S" },
              fix: { kind: "fit", group: "g" },
              song: null,
            },
            {
              severity: "attention",
              what: { kind: "universeUnconfigured", universe: 4 },
              fix: { kind: "config" },
              song: null,
            },
            {
              severity: "unknown",
              what: { kind: "oladUnreachable" },
              fix: { kind: "config" },
              song: null,
            },
            {
              severity: "attention",
              what: { kind: "universeUnpatched", universe: 1 },
              fix: { kind: "config" },
              song: null,
            },
          ],
        },
      ],
    };
    expect(
      mapReadiness(create(LightingReadinessSchema, readinessReply(report))),
    ).toEqual(report);
  });

  it("the mock server's fit reads back as it was", () => {
    expect(mapFit(create(LightingFitSchema, fitReply(FIT)))).toEqual(FIT);
  });

  it("a fit without a venue, and a group with every part, read back", () => {
    const bare: FitReport = {
      venue: null,
      groups: [],
      focus_points_wanted: [{ name: "drummer", songs: ["S"] }],
      output: {
        unconfigured: [4],
        unpatched: [2],
        reachable: null,
        ola_http_port: null,
      },
    };
    expect(mapFit(create(LightingFitSchema, fitReply(bare)))).toEqual(bare);

    const full: FitReport = {
      ...FIT,
      venue: {
        name: "v",
        fixtures: [
          {
            name: "M",
            type: "Mover",
            tags: ["other"],
            position: null,
            capabilities: [
              "color",
              "pan_tilt",
              "strobe",
              "cells",
              "dimmer",
              "white",
              "zoom",
              "focus",
              "gobo",
              "color_wheel",
            ],
          },
        ],
        focus_points: [],
        error: { fixture: null, reason: "bad" },
      },
      groups: [
        {
          name: "g",
          defined: true,
          needs: { all_of: ["a"], any_of: ["b"], prefer: ["c"] },
          tags: ["a"],
          fixtures: [],
          songs: ["S"],
          wants: ["move", "color", "strobe", "cells", "dimmer"],
          suggestion: {
            fixtures: ["M"],
            tags: ["a"],
            reason: {
              count: 1,
              type: "Mover",
              where: "upstage truss",
              height: "truss",
              depth: "upstage",
              can: ["move"],
            },
          },
          others: [
            {
              fixtures: ["P"],
              type: "Par",
              where: null,
              height: "deck",
              depth: "downstage",
            },
            {
              fixtures: ["Q"],
              type: "Par",
              where: null,
              height: "low",
              depth: "mid",
            },
          ],
          unmet: ["move"],
          unmet_together: true,
        },
      ],
    };
    expect(mapFit(create(LightingFitSchema, fitReply(full)))).toEqual(full);
  });

  it("a preview reads back as it was", () => {
    const preview: Preview = {
      song: "Song",
      evaluations: [
        {
          time: 1.5,
          fixtures: { M: { red: 255, pan: 128 }, P: { dimmer: 0 } },
          poses: {
            M: { pan: 10, tilt: -5, aim: [0, -0.5, -0.5], floor: [1, 2] },
            N: { pan: 0, tilt: 0, aim: [0, 0, -1], floor: null },
          },
          cells: { Bar: { c1: { red: 1 }, c2: { red: 2 } } },
          strobe_rates: { M: 4.5 },
          active_effects: [
            {
              id: "e1",
              groups: ["washes"],
              kind: "Static",
              layer: "background",
              elapsed: 0.5,
              duration: 10,
              fixtures: ["M"],
            },
            {
              id: "e2",
              groups: null,
              kind: "Move",
              layer: "foreground",
              elapsed: 1,
              duration: 4,
              fixtures: ["M", "N"],
            },
          ],
        },
      ],
      untouched: ["P"],
      venue_error: null,
    };
    expect(
      mapPreview(create(LightingPreviewSchema, previewReply(preview))),
    ).toEqual(preview);
    const failed = { ...preview, venue_error: "no venue" };
    expect(
      mapPreview(create(LightingPreviewSchema, previewReply(failed))),
    ).toEqual(failed);
  });

  it("a fixture test's options read back as they were", () => {
    const options: TestOptions = {
      fixture_type: "Brick",
      gdtf: true,
      modes: [
        {
          name: "8: RGBS",
          drivable: true,
          error: null,
          footprint: 4,
          controls: {
            color: true,
            dimmer: "folded",
            strobe: { min_hz: 0, max_hz: 20 },
            pan: null,
            tilt: null,
            levels: ["white"],
          },
          channels: [{ offset: 1, name: "red" }],
        },
        {
          name: "Mover",
          drivable: true,
          error: null,
          footprint: 6,
          controls: {
            color: false,
            dimmer: "channel",
            strobe: null,
            pan: { min: -270, max: 270 },
            tilt: { min: -135, max: 135 },
            levels: [],
          },
          channels: [],
        },
        {
          name: null,
          drivable: false,
          error: "no channels",
          footprint: null,
          controls: null,
          channels: [],
        },
      ],
      default_mode: "8: RGBS",
      universes: [
        { universe: 1, name: "main", patched: true },
        { universe: 2, name: null, patched: null },
      ],
      olad: { reachable: true, port: 9090 },
      available: false,
      unavailable: "playing",
      unavailable_message: "A song is playing.",
    };
    expect(
      mapTestOptions(
        create(FixtureTestOptionsSchema, testOptionsReply(options)),
      ),
    ).toEqual(options);
    for (const unavailable of [
      "locked",
      "no_dmx",
      "no_universes",
      null,
    ] as const) {
      const o = { ...options, unavailable };
      expect(
        mapTestOptions(create(FixtureTestOptionsSchema, testOptionsReply(o)))
          .unavailable,
      ).toBe(unavailable);
    }
    const none: TestOptions = {
      ...options,
      default_mode: null,
      olad: { reachable: null, port: null },
    };
    expect(
      mapTestOptions(create(FixtureTestOptionsSchema, testOptionsReply(none))),
    ).toEqual(none);
  });

  it("a fixture test's answer reads back as it was", () => {
    const answer: TestAnswer = {
      active: true,
      universe: 1,
      address: 510,
      footprint: 4,
      fixture_type: "Par",
      mode: null,
      expires_in_secs: 5,
      frame: [
        { offset: 1, address: 510, name: "dimmer", value: 255, raw: false },
        { offset: 2, address: 511, name: "red", value: 42, raw: true },
      ],
      warnings: [
        { kind: "venue_overlap", fixtures: ["Front"], message: "m1" },
        { kind: "past_universe_end", message: "m2" },
        { kind: "olad_unreachable", message: "m3" },
        { kind: "universe_unpatched", message: "m4" },
        { kind: "unsupported_control", control: "pan", message: "m5" },
      ],
    };
    expect(
      mapTestAnswer(create(FixtureTestAnswerSchema, testAnswerReply(answer))),
    ).toEqual(answer);
    const gdtf = { ...answer, mode: "8: RGBS" };
    expect(
      mapTestAnswer(create(FixtureTestAnswerSchema, testAnswerReply(gdtf)))
        .mode,
    ).toBe("8: RGBS");
  });

  it("a fixture type listing reads back as it was", () => {
    const list: FixtureTypeList = {
      fixture_types: {
        par: typeEntry("par", {
          footprint: 4,
          form_editable: true,
          fixture_type: {
            name: "par",
            channels: { red: 1, green: 2, blue: 3, dimmer: 4 },
            max_strobe_frequency: 25,
            min_strobe_frequency: 0.5,
            strobe_dmx_offset: 10,
          },
        }),
        brick: typeEntry("brick", {
          extension: "fixture",
          referential: true,
          footprint: null,
          gdtf: {
            fixture: "PB15",
            manufacturer: "Astera",
            modes: 2,
            beam: { type: "Wash", angle: 13 },
            thumbnail: "abc/thumb.png",
            used_by: 3,
            in_use: [{ mode: "8: RGBS", count: 3 }],
          },
        }),
        lost: typeEntry("lost", {
          extension: "fixture",
          referential: true,
          footprint: null,
          gdtf: null,
        }),
        loose: typeEntry("loose", {
          extension: null,
          file: null,
          referential: true,
          footprint: null,
          gdtf: null,
        }),
        shared: typeEntry("shared", {
          footprint: 1,
          owner: { file: "all.light", editable: false, reason: "Shared." },
        }),
      },
      errors: [{ file: "broken.light", error: "line 4" }],
    };
    const read = mapFixtureTypeList(
      create(ListFixtureTypesResponseSchema, fixtureTypeListReply(list)),
    );
    expect(read).toEqual(list);
  });

  it("a single fixture type reads back with its text, or with none", () => {
    for (const dsl of ['fixture_type "par" {\n}\n', null]) {
      const entry = typeEntry("par", { dsl });
      expect(
        mapFixtureTypeEntry(
          create(FixtureTypeInfoSchema, fixtureTypeInfoReply(entry)),
          false,
        ),
      ).toEqual(entry);
    }
  });

  it("a GDTF fixture's details read back, a refused mode too", () => {
    const details: FixtureTypeGdtf = {
      archive: "library/pb15.gdtf",
      rig: "abc/rig.json",
      thumbnail: null,
      beam: {
        type: "Wash",
        beam_angle: 13,
        field_angle: null,
        luminous_flux: 475,
        color_temperature: null,
        power: 12,
      },
      about: "A brick.",
      venues: [
        {
          name: "club",
          fixtures: [
            { name: "Solo", mode: "1: RGB" },
            { name: "Odd", mode: null },
          ],
        },
      ],
      inspection: {
        modes: [
          {
            name: "8: RGBS",
            footprint: 4,
            capabilities: ["color", "strobe"],
            wants: ["color", "dimmer", "strobe"],
            cells: 0,
            strobe_range: { min_hz: 0.4, max_hz: 25 },
            channels: [
              [1, "red"],
              [4, "strobe"],
            ],
          },
          {
            name: "Plain",
            footprint: 1,
            capabilities: [],
            wants: [],
            cells: 3,
            strobe_range: { min_hz: null, max_hz: 10 },
            channels: [],
          },
          { name: "Dup", footprint: 2, refused: "another mode has this name" },
        ],
      },
    };
    expect(
      mapFixtureTypeGdtf(
        create(FixtureTypeGdtfDetailsSchema, fixtureTypeGdtfReply(details)),
      ),
    ).toEqual(details);
  });

  it("a fixture's settings and a settings save read back", () => {
    for (const settings of [
      {
        name: "Brick",
        movement: { max_pan_speed: 240, max_tilt_speed: null },
        strobe_curve: "linear",
        strobe: { steps: 3, automatic: "declared" },
        version: "abc",
      },
      {
        name: "Loose",
        movement: { max_pan_speed: null, max_tilt_speed: null },
        strobe_curve: null,
        strobe: null,
        version: "unrecorded",
      },
    ] satisfies FixtureSettingsData[]) {
      expect(
        mapFixtureSettings(
          create(FixtureTypeSettingsSchema, fixtureSettingsReply(settings)),
        ),
      ).toEqual(settings);
    }
    const saved: SettingsSaved = {
      version: "v2",
      rename: {
        from: "Brick",
        to: "PB",
        venues: [{ venue: "club", file: "club.venue", lines: 2 }],
        lines: 2,
      },
      venue_versions: { "club.venue": "v-c" },
      venue_error: { venue: "club", fixture: "C1", reason: "no type" },
    };
    expect(
      mapSettingsSaved(
        create(FixtureTypeSettingsSavedSchema, settingsSavedReply(saved)),
      ),
    ).toEqual(saved);
    const plain: SettingsSaved = {
      version: "v1",
      rename: null,
      venue_versions: {},
      venue_error: null,
    };
    expect(
      mapSettingsSaved(
        create(FixtureTypeSettingsSavedSchema, settingsSavedReply(plain)),
      ),
    ).toEqual(plain);
  });

  it("a type saved or deleted reads back", () => {
    expect(
      mapFixtureTypeSaved(
        create(
          SaveFixtureTypeResponseSchema,
          fixtureTypeSavedReply({ name: "p" }),
        ),
      ),
    ).toEqual({ status: "saved", name: "p", rename: null, venue_error: null });
    const rename = {
      from: "a",
      to: "b",
      venues: [{ venue: "v", file: "v.venue", lines: 1 }],
      lines: 1,
    };
    expect(
      mapFixtureTypeSaved(
        create(
          SaveFixtureTypeResponseSchema,
          fixtureTypeSavedReply({ name: "b", rename }),
        ),
      ).rename,
    ).toEqual(rename);
    expect(
      mapFixtureTypeDeleted(
        create(
          DeleteFixtureTypeResponseSchema,
          fixtureTypeDeletedReply({
            name: "b",
            archive_removed: "lighting/library/b.gdtf",
          }),
        ),
      ),
    ).toEqual({
      status: "deleted",
      name: "b",
      archive_removed: "lighting/library/b.gdtf",
      archive_kept: false,
    });
  });

  it("the requests a page makes read back as the old routes carried them", () => {
    const text = savedTypeRequest(
      create(
        SaveFixtureTypeRequestSchema,
        saveFixtureTypeRequest(
          "mover",
          { text: "fixture_type {}" },
          { ext: "fixture", dir: "t", from: "old", venuesDir: "v" },
        ),
      ),
    );
    expect(text).toEqual({
      name: "mover",
      from: "old",
      dir: "t",
      venuesDir: "v",
      ext: "fixture",
      text: "fixture_type {}",
      form: null,
    });
    const form = savedTypeRequest(
      create(
        SaveFixtureTypeRequestSchema,
        saveFixtureTypeRequest("par", {
          channels: { dimmer: 1 },
          max_strobe_frequency: 25,
        }),
      ),
    );
    expect(form).toMatchObject({
      name: "par",
      from: null,
      ext: null,
      text: null,
      form: {
        channels: { dimmer: 1 },
        max_strobe_frequency: 25,
        min_strobe_frequency: null,
        strobe_dmx_offset: null,
      },
    });
    const settings = savedSettingsRequest(
      create(
        SaveFixtureTypeSettingsRequestSchema,
        saveFixtureSettingsRequest(
          "Brick",
          {
            name: "PB",
            movement: { max_pan_speed: 9, max_tilt_speed: null },
            strobe_curve: "period",
            write: true,
            venue_versions: { "a.venue": "x" },
          },
          { dir: "t", venuesDir: "v" },
          "rev",
        ),
      ),
    );
    expect(settings).toEqual({
      type: "Brick",
      name: "PB",
      movement: { max_pan_speed: 9, max_tilt_speed: null },
      strobe_curve: "period",
      write: true,
      venue_versions: { "a.venue": "x" },
      revision: "rev",
      dir: "t",
      venuesDir: "v",
    });
  });
});

describe("the venue answers made from the page's own shapes", () => {
  const venue = {
    name: "club",
    fixtures: {
      A: {
        name: "A",
        fixture_type: "Brick",
        universe: 1,
        start_channel: 1,
        tags: ["wash"],
        position: [1, 2, 3] as [number, number, number],
        rotation: [0, 0, 90] as [number, number, number],
        mode: "8: RGBS",
        beam_angle: 45,
      },
      B: {
        name: "B",
        fixture_type: "Par",
        universe: 2,
        start_channel: 9,
        tags: [],
        position: null,
        rotation: null,
      },
    },
    focus_points: { dj: [0, 3, 1] as [number, number, number] },
    source: { mvr: "a.mvr", origin: [0, -3.5, 0] as [number, number, number] },
  };

  it("a listing and a venue read back as they were", () => {
    const list = {
      venues: { club: venue },
      owners: {
        club: { file: "club.venue", editable: false, reason: "shared" },
      },
      errors: [{ file: "x.light", error: "bad" }],
      versions: { club: "v1" },
    };
    expect(
      mapVenueList(create(ListVenuesResponseSchema, venueListReply(list))),
    ).toEqual(list);
    const detail = { venue, dsl: "text", version: "v2" };
    expect(
      mapVenueDetail(create(VenueDetailSchema, venueDetailReply(detail))),
    ).toEqual(detail);
  });

  it("a saved venue, a patch, a scene and aim points read back", () => {
    expect(
      mapVenueSaved(
        create(
          SaveVenueResponseSchema,
          venueSavedReply({
            name: "club",
            reloaded: true,
            version: "v3",
            venue_error: { venue: "club", fixture: "A", reason: "r" },
          }),
        ),
      ),
    ).toEqual({
      status: "saved",
      name: "club",
      reloaded: true,
      version: "v3",
      venue_error: { venue: "club", fixture: "A", reason: "r" },
    });
    const patch = {
      spans: [
        {
          fixture: "A",
          universe: 1,
          address: 1,
          footprint: 4,
          type: "Brick",
          mode: "8: RGBS",
        },
        {
          fixture: "B",
          universe: 1,
          address: 3,
          footprint: null,
          type: "Brick",
          mode: null,
        },
      ],
      overlaps: [
        {
          first: "A",
          second: "B",
          first_gang: ["A"],
          second_gang: ["B"],
          universe: 1,
          from: 3,
          to: 4,
        },
      ],
      overruns: [
        { fixture: "C", universe: 1, address: 511, footprint: 4, last: 514 },
      ],
    };
    expect(
      mapVenuePatch(create(VenuePatchSchema, venuePatchReply(patch))),
    ).toEqual(patch);
    const scene = {
      fixtures: {
        A: {
          tags: ["wash"],
          type: "Brick",
          position: [1, 2, 3] as [number, number, number],
          rotation: null,
          beam_angle: 45,
          rig: "abc/rig.json",
          mode: "8: RGBS",
        },
      },
      venue: {
        name: "club",
        focus_points: { dj: [0, 3, 1] as [number, number, number] },
        scenery: null,
        scenery_error: "gone",
      },
    };
    expect(
      mapVenueScene(create(VenueSceneSchema, venueSceneReply(scene))),
    ).toEqual(scene);
    const aim = {
      created: [
        {
          name: "A aim",
          fixture: "A",
          point: [1, 2, 0] as [number, number, number],
        },
      ],
      reloaded: false,
      version: "v4",
      venue_error: null,
    };
    expect(
      mapAimPoints(
        create(AddVenueAimPointsResponseSchema, aimPointsReply(aim)),
      ),
    ).toEqual(aim);
  });

  it("an export's summary and a keep read back", () => {
    const summary = {
      file_name: "club.mvr",
      venue: "club",
      output: "lighting/export/club.mvr",
      fixtures: 3,
      focus_points: 1,
      embedded_gdtfs: ["a.gdtf"],
      generated_gdtfs: { "g.gdtf": "Par" },
      positioned_fixtures: 2,
      fixed_fixtures: [
        { name: "A", focus: "dj" },
        { name: "B", focus: null },
      ],
      warnings: ["w"],
    };
    expect(
      mapMvrExportSummary(
        create(MvrExportSummarySchema, mvrExportSummaryReply(summary)),
      ),
    ).toEqual(summary);
    const kept = create(
      KeepMvrExportResponseSchema,
      keptReply({ kept: "lighting/export/club.mvr" }),
    );
    expect(kept.outcome).toEqual({
      case: "kept",
      value: "lighting/export/club.mvr",
    });
    const left = create(
      KeepMvrExportResponseSchema,
      keptReply({ existing: "club.mvr" }),
    );
    expect(left.outcome).toEqual({ case: "existing", value: "club.mvr" });
  });

  it("a venue save reads back as the page sent it", () => {
    const body = {
      fixtures: [
        {
          name: "A",
          fixture_type: "Brick",
          universe: 1,
          start_channel: 1,
          tags: ["wash"],
          position: [1, 2, 3] as [number, number, number],
          rotation: null,
          mode: "8: RGBS",
          beam_angle: null,
        },
      ],
      focus_points: { dj: [0, 3, 1] as [number, number, number] },
      source: null,
    };
    expect(
      savedVenueRequest(
        create(
          SaveVenueRequestSchema,
          saveVenueRequest("club", body, {
            dir: "v",
            from: "old",
            version: "r1",
          }),
        ),
      ),
    ).toEqual({
      ...body,
      name: "club",
      from: "old",
      dir: "v",
      version: "r1",
      text: null,
    });
  });
});

describe("the MVR export's staged download and the import's refusals", () => {
  it("a staged export reads back with its path, name and size", () => {
    const staged = create(
      ExportMvrResponseSchema,
      exportMvrReply({ path: "/downloads/abc", fileName: "tour.mvr", size: 7 }),
    );
    expect([staged.path, staged.fileName, staged.size]).toEqual([
      "/downloads/abc",
      "tour.mvr",
      7n,
    ]);
  });

  it("names the kind of an import's refusal in the metadata the page reads", () => {
    expect(new Headers(importKind("venue")).get("x-import-kind")).toBe("venue");
  });
});
