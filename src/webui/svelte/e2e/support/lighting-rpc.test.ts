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
  FixtureTestAnswerSchema,
  FixtureTestOptionsSchema,
  GetLightShowResponseSchema,
  LightingFitSchema,
  LightingPreviewSchema,
  LightingReadinessSchema,
  ListLightingGroupsResponseSchema,
  ValidateLightShowResponseSchema,
} from "../../src/gen/player/v1/player_pb";
import {
  mapFit,
  mapGroups,
  mapPreview,
  mapReadiness,
  mapTestAnswer,
  mapTestOptions,
} from "../../src/lib/grpc/lighting";
import type {
  FitReport,
  Preview,
  Readiness,
  TestAnswer,
  TestOptions,
} from "../../src/gen/rules/mtrack_rules";
import { FIT, READINESS } from "../mock-server/test-data";
import {
  fitReply,
  groupsReply,
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
});
