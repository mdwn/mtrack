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

import { test, expect, type Page } from "@playwright/test";
import { MVR_INSPECTION, MVR_EXPORT_SUMMARY } from "../mock-server/test-data";
import { Code, ConnectError } from "@connectrpc/connect";
import {
  ConfigService,
  LightingService,
  UploadPurpose,
  type ImportMvrRequest,
  type InspectMvrRequest,
} from "../../src/gen/player/v1/player_pb";
import type {
  MvrImport,
  MvrPlan,
  VenueError,
  VenueList,
} from "../../src/gen/rules/mtrack_rules";
import {
  aimPointsReply,
  exportMvrReply,
  importKind,
  keptReply,
  mvrExportSummaryReply,
  mvrImportReply,
  mvrInspectionReply,
  venueListReply,
} from "../support/lighting-rpc";
import { mockRpc } from "../support/rpc";

// MVR import (the wizard at #/lighting/import) and export (the dialog on
// Venues), design section 11. Every test answers its own calls with
// `mockRpc` (the file is announced, its bytes sent to the mock server's
// `PUT /uploads/{id}`, and `InspectMvr` or `ImportMvr` takes it), so nothing
// depends on state another test left behind.

type Inspection = typeof MVR_INSPECTION;

const MVR_FILE = {
  name: "Kellys.mvr",
  mimeType: "application/octet-stream",
  buffer: Buffer.from("PK-not-really-an-mvr"),
};

/** The choices a call carried, as the form fields the wizard used to send:
 *  `name`, `origin` ("x,y,z" millimeters), `write`, `venues_dir`,
 *  `fixture_types_dir` and `keep` (JSON); a choice not made has no key. */
function fields(
  request: InspectMvrRequest | ImportMvrRequest,
): Record<string, string> {
  const out: Record<string, string> = {};
  const o = request.options;
  if (o?.name) out.name = o.name;
  if (o?.originMm) {
    out.origin = `${o.originMm.x},${o.originMm.y},${o.originMm.z}`;
  }
  if ("write" in request) out.write = String(request.write);
  if (o?.venuesDir) out.venues_dir = o.venuesDir;
  if (o?.fixtureTypesDir) out.fixture_types_dir = o.fixtureTypesDir;
  if (o?.keep) {
    out.keep = JSON.stringify({
      fixtures: Object.fromEntries(
        Object.entries(o.keep.fixtures).map(([name, f]) => [name, f.fields]),
      ),
      focus_points: o.keep.focusPoints,
    });
  }
  return out;
}

function inspection(change: (i: Inspection) => void = () => {}): Inspection {
  const copy = structuredClone(MVR_INSPECTION);
  change(copy);
  return copy;
}

/** Answers inspect and import; the requests are collected. */
async function routeImport(
  page: Page,
  opts: {
    inspect?: Inspection;
    plan?: (origin: string) => MvrPlan;
    report?: MvrImport;
    venueError?: VenueError | null;
  } = {},
) {
  const rpc = await mockRpc(page);
  const inspected: InspectMvrRequest[] = [];
  const imports: ImportMvrRequest[] = [];
  rpc.unary(LightingService.method.inspectMvr, (request) => {
    inspected.push(request);
    return mvrInspectionReply(opts.inspect ?? MVR_INSPECTION);
  });
  rpc.unary(LightingService.method.importMvr, (request) => {
    imports.push(request);
    const f = fields(request);
    return mvrImportReply(
      f.write === "true"
        ? {
            write: true,
            report: opts.report ?? {
              ...MVR_INSPECTION.report,
              written: [
                "lighting/library/Kellys.mvr",
                "lighting/venues/kellys.venue",
              ],
              distillation_warnings: {},
            },
            reloaded: false,
            venue_error: opts.venueError ?? null,
          }
        : {
            write: false,
            plan: opts.plan?.(f.origin) ?? MVR_INSPECTION.report,
          },
    );
  });
  return { rpc, inspected, imports };
}

async function chooseFile(page: Page) {
  await page.getByTestId("mvr-file").setInputFiles(MVR_FILE);
}

test.describe("MVR import wizard", () => {
  test("step 1 reads the file and shows what it holds", async ({ page }) => {
    const { rpc, inspected } = await routeImport(page);
    await page.goto("/#/lighting/import");

    // The stepper is an ordered list, with the current step marked.
    const steps = page.getByRole("list", { name: "Import steps" });
    await expect(steps.getByRole("listitem")).toHaveCount(4);
    await expect(page.getByTestId("mvr-step-file")).toHaveAttribute(
      "aria-current",
      "step",
    );
    await expect(page.getByTestId("mvr-step-origin")).not.toHaveAttribute(
      "aria-current",
      "step",
    );
    // Import is a part of Venues: that tab stays lit, and the page is titled.
    const tabs = page.getByRole("navigation", { name: "Lighting sections" });
    await expect(tabs.getByRole("link", { name: "Venues" })).toHaveAttribute(
      "aria-current",
      "page",
    );

    await chooseFile(page);
    await expect(page.getByTestId("mvr-count-fixtures")).toHaveText("3");
    await expect(page.getByTestId("mvr-count-types")).toHaveText("1");
    await expect(page.getByTestId("mvr-count-without")).toHaveText("1");
    await expect(page.getByTestId("mvr-venue-name")).toHaveValue("Kellys");

    // The browser kept the file and announced it, under its own name, for an
    // inspection; and sent its bytes to the path that answered.
    expect(inspected).toHaveLength(1);
    const begun = rpc.calls(ConfigService.method.beginUpload);
    expect(begun).toHaveLength(1);
    expect(begun[0].fileName).toBe("Kellys.mvr");
    expect(begun[0].purpose).toBe(UploadPurpose.MVR_INSPECT);
    expect(begun[0].size).toBe(BigInt(MVR_FILE.buffer.length));
  });

  test("an error at step 1 is shown in place and retry recovers", async ({
    page,
  }) => {
    let calls = 0;
    const rpc = await mockRpc(page);
    rpc.unary(LightingService.method.inspectMvr, () => {
      calls++;
      if (calls === 1) {
        throw new ConnectError(
          "not a readable MVR archive (is this an MVR file?)",
          Code.InvalidArgument,
        );
      }
      return mvrInspectionReply(MVR_INSPECTION);
    });
    await page.goto("/#/lighting/import");
    await chooseFile(page);

    const alert = page.getByTestId("mvr-error");
    await expect(alert).toContainText("not a readable MVR archive");
    await expect(page.getByTestId("mvr-continue")).toHaveCount(0);
    await expect(page.getByTestId("mvr-step-file")).toHaveAttribute(
      "aria-current",
      "step",
    );

    await alert.getByRole("button", { name: "Retry" }).click();
    await expect(page.getByTestId("mvr-count-fixtures")).toHaveText("3");
    await expect(alert).toHaveCount(0);
  });

  test("a venue name the files refuse keeps the name field, and another name re-inspects", async ({
    page,
  }) => {
    const names: string[] = [];
    const rpc = await mockRpc(page);
    rpc.unary(LightingService.method.inspectMvr, (request) => {
      const name = fields(request).name ?? "";
      names.push(name);
      if (name === "Kellys") {
        throw new ConnectError(
          'cannot import venue "Kellys": Defined in all.venue alongside venue "Annex"; edit that file by hand.',
          Code.FailedPrecondition,
          importKind("venue"),
        );
      }
      return mvrInspectionReply(MVR_INSPECTION);
    });
    await page.goto("/#/lighting/import");
    await chooseFile(page);

    // The reason, and the name field to change: not a dead end.
    await expect(page.getByTestId("mvr-error")).toContainText(
      "Defined in all.venue alongside venue",
    );
    const name = page.getByTestId("mvr-venue-name");
    await expect(name).toHaveValue("Kellys");
    await expect(page.getByTestId("mvr-refused-hint")).toBeVisible();
    await expect(page.getByTestId("mvr-continue")).toHaveCount(0);

    await name.fill("Kellys 2026");
    await page.getByTestId("mvr-recheck").click();
    await expect(page.getByTestId("mvr-count-fixtures")).toHaveText("3");
    await expect(page.getByTestId("mvr-error")).toHaveCount(0);
    await expect(page.getByTestId("mvr-refused-hint")).toHaveCount(0);
    await expect(name).toHaveValue("Kellys 2026");
    expect(names).toEqual(["Kellys", "Kellys 2026"]);
  });

  test("a refusal that another venue name cannot answer offers no rename", async ({
    page,
  }) => {
    const rpc = await mockRpc(page);
    rpc.error(
      LightingService.method.inspectMvr,
      Code.FailedPrecondition,
      'lighting/fixture_types/synth_brick_astera_pb15.fixture holds "Other"; rename or move that file, or import Astera_PB15.gdtf on its own first under a name of your own (`mtrack import-gdtf --name <name>`, or the Fixtures page)',
      importKind("record"),
    );
    await page.goto("/#/lighting/import");
    await chooseFile(page);

    await expect(page.getByTestId("mvr-error")).toContainText(
      "synth_brick_astera_pb15.fixture holds",
    );
    await expect(page.getByTestId("mvr-venue-name")).toHaveCount(0);
    await expect(page.getByTestId("mvr-refused-hint")).toHaveCount(0);
    await expect(page.getByTestId("mvr-recheck")).toHaveCount(0);
  });

  test("an empty venue name can be checked again, as the file's own name", async ({
    page,
  }) => {
    let calls = 0;
    const names: (string | undefined)[] = [];
    const rpc = await mockRpc(page);
    rpc.unary(LightingService.method.inspectMvr, (request) => {
      calls++;
      names.push(fields(request).name);
      if (calls === 1) {
        throw new ConnectError(
          'cannot import venue "Kellys": Kellys.venue is taken',
          Code.FailedPrecondition,
          importKind("venue"),
        );
      }
      return mvrInspectionReply(MVR_INSPECTION);
    });
    await page.goto("/#/lighting/import");
    await chooseFile(page);
    await page.getByTestId("mvr-venue-name").fill("");
    const recheck = page.getByTestId("mvr-recheck");
    await expect(recheck).toBeEnabled();
    await recheck.click();
    await expect(page.getByTestId("mvr-count-fixtures")).toHaveText("3");
    expect(names).toEqual(["Kellys", undefined]);

    // Empty means the file's own name for Continue too: on to the origin,
    // with no second inspection.
    const proceed = page.getByTestId("mvr-continue");
    await expect(proceed).toBeEnabled();
    await proceed.click();
    await expect(page.getByTestId("mvr-step-origin")).toHaveAttribute(
      "aria-current",
      "step",
    );
    expect(names).toEqual(["Kellys", undefined]);
  });

  test("step 2 marks the deck suggestion, applies it, and takes a click", async ({
    page,
  }) => {
    await routeImport(page);
    await page.goto("/#/lighting/import");
    await chooseFile(page);
    await page.getByTestId("mvr-continue").click();

    await expect(page.getByTestId("mvr-step-origin")).toHaveAttribute(
      "aria-current",
      "step",
    );
    // The deck's front edge, in the middle, at the deck's top.
    await expect(page.getByTestId("mvr-suggestion")).toContainText(
      "the middle of the deck's front edge",
    );
    await expect(page.getByTestId("mvr-suggestion-value")).toHaveText(
      "0, 1000, 500",
    );
    await expect(page.getByTestId("mvr-plan-deck")).toBeVisible();
    await expect(page.getByTestId("mvr-plan-suggestion")).toBeVisible();

    // The suggestion is offered, not imposed: it seeds the fields, and a
    // click moves them.
    await expect(page.getByTestId("mvr-origin-x")).toHaveValue("0");
    await expect(page.getByTestId("mvr-origin-y")).toHaveValue("1000");
    await expect(page.getByTestId("mvr-origin-z")).toHaveValue("500");

    const plan = page.getByTestId("mvr-plan");
    const box = (await plan.boundingBox())!;
    // Near the top-left of the plan: stage right of the deck (x negative),
    // far upstage (y large).
    await plan.click({ position: { x: box.width * 0.1, y: box.height * 0.1 } });
    const x = Number(await page.getByTestId("mvr-origin-x").inputValue());
    const y = Number(await page.getByTestId("mvr-origin-y").inputValue());
    expect(x).toBeLessThan(-2000);
    expect(y).toBeGreaterThan(4000);
    // A click keeps the height.
    await expect(page.getByTestId("mvr-origin-z")).toHaveValue("500");
    await expect(page.getByTestId("mvr-plan-origin")).toBeVisible();

    await page.getByTestId("mvr-use-suggestion").click();
    await expect(page.getByTestId("mvr-origin-x")).toHaveValue("0");
    await expect(page.getByTestId("mvr-origin-y")).toHaveValue("1000");
  });

  test("step 2 falls back to the fixtures' footprint without a deck", async ({
    page,
  }) => {
    await routeImport(page, {
      inspect: inspection((i) => {
        i.scene.deck_mm = null;
        i.scene.scenery = [];
      }),
    });
    await page.goto("/#/lighting/import");
    await chooseFile(page);
    await page.getByTestId("mvr-continue").click();

    await expect(page.getByTestId("mvr-suggestion")).toContainText(
      "no deck was found",
    );
    // Middle of the two bricks in x; the lowest y among them; on the floor.
    await expect(page.getByTestId("mvr-suggestion-value")).toHaveText(
      "0, 3500, 0",
    );
    await expect(page.getByTestId("mvr-plan-deck")).toHaveCount(0);
  });

  test("the origin can be typed and moved with the keyboard", async ({
    page,
  }) => {
    const { imports } = await routeImport(page);
    await page.goto("/#/lighting/import");
    await chooseFile(page);
    await page.getByTestId("mvr-continue").click();

    await page.getByTestId("mvr-origin-x").fill("250");
    await page.getByTestId("mvr-origin-y").fill("1200");
    await page.getByTestId("mvr-origin-z").fill("0");
    // Arrow keys on the focused plan nudge by 100 mm.
    await page.getByTestId("mvr-plan").focus();
    await page.keyboard.press("ArrowRight");
    await page.keyboard.press("ArrowUp");
    await expect(page.getByTestId("mvr-origin-x")).toHaveValue("350");
    await expect(page.getByTestId("mvr-origin-y")).toHaveValue("1300");

    await page.getByTestId("mvr-to-review").click();
    await expect(page.getByTestId("mvr-step-review")).toHaveAttribute(
      "aria-current",
      "step",
    );
    // The review asked the server for a plan, not a write, at this origin.
    expect(imports).toHaveLength(1);
    const f = fields(imports[0]);
    expect(f.write).toBe("false");
    expect(f.origin).toBe("350,1300,0");
    expect(f.name).toBe("Kellys");
  });

  test("step 3 reviews a first import: seeds, types, TODOs, scenery", async ({
    page,
  }) => {
    await routeImport(page, {
      plan: () => ({
        ...MVR_INSPECTION.report,
        fixture_types: [
          ...MVR_INSPECTION.report.fixture_types,
          {
            name: "Robe Spot",
            archive: "lighting/library/Robe.gdtf",
            modes: ["Mode 1"],
            existing: true,
          },
        ],
      }),
    });
    await page.goto("/#/lighting/import");
    await chooseFile(page);
    await page.getByTestId("mvr-continue").click();
    await page.getByTestId("mvr-to-review").click();

    await expect(page.getByTestId("mvr-mode")).toHaveAttribute(
      "data-merge",
      "false",
    );
    await expect(page.getByTestId("mvr-mode")).toContainText(
      "first import: a new venue file for Kellys will be seeded",
    );
    await expect(page.getByTestId("mvr-review-seeded")).toHaveText("2");
    await expect(page.getByTestId("mvr-review-types-new")).toContainText(
      "Astera PB15",
    );
    await expect(page.getByTestId("mvr-review-types-existing")).toContainText(
      "Robe Spot",
    );
    // One type per GDTF, with the modes the file uses; no default.
    await expect(page.getByTestId("mvr-type-Astera PB15")).toContainText(
      "in 8: RGBS, 9: RGBWS",
    );
    await expect(page.getByTestId("mvr-type-Robe Spot")).toContainText(
      "in Mode 1",
    );
    await expect(
      page.locator('section[aria-labelledby="mvr-review-title"]'),
    ).not.toContainText(/default/i);
    // Every seeded fixture is listed with its own mode.
    const moded = page.getByTestId("mvr-review-moded");
    await expect(page.getByText("Each fixture's mode (2)")).toBeVisible();
    await expect(moded.getByRole("listitem")).toHaveCount(2);
    await expect(moded).toContainText("mode 8: RGBS");
    await expect(moded).toContainText("mode 9: RGBWS");
    await expect(page.getByTestId("mvr-review-todos")).toContainText("Lost");
    await expect(page.getByTestId("mvr-review-todos")).toContainText(
      "is not embedded in the MVR",
    );
    await expect(page.getByTestId("mvr-review-scenery")).toContainText(
      "2 objects; 1 meshes the 3D view cannot draw",
    );
    await expect(page.getByTestId("mvr-review-warnings")).toContainText(
      "3D view does not draw",
    );
    await expect(page.getByTestId("mvr-review-origin")).toContainText(
      "(0.00, 1.00, 0.50) m",
    );
    await expect(page.getByTestId("mvr-review-merge")).toHaveCount(0);
  });

  test("step 3 reviews a merge and says what it keeps", async ({ page }) => {
    await routeImport(page, {
      plan: () => ({
        ...MVR_INSPECTION.report,
        merge: true,
        kept_fixtures: ["Hand Par"],
        kept_focus_points: ["Singer"],
        removed_fixtures: [{ name: "Old Brick", tags: ["wash"] }],
      }),
    });
    await page.goto("/#/lighting/import");
    await chooseFile(page);
    await page.getByTestId("mvr-continue").click();
    await page.getByTestId("mvr-to-review").click();

    await expect(page.getByTestId("mvr-mode")).toHaveAttribute(
      "data-merge",
      "true",
    );
    await expect(page.getByTestId("mvr-mode")).toContainText("merges into it");
    const keeps = page.getByTestId("mvr-review-merge");
    await expect(keeps).toContainText("Hand Par");
    await expect(keeps).toContainText("Singer");
    await expect(keeps).toContainText("Old Brick");
    await expect(keeps).toContainText("Tags and focus point names are kept");
  });

  test("a merge lists hand edits with keep-my-edits checkboxes and sends the keep list", async ({
    page,
  }) => {
    const { imports } = await routeImport(page, {
      plan: () => ({
        ...MVR_INSPECTION.report,
        merge: true,
        fixtures: MVR_INSPECTION.report.fixtures.map((f, i) =>
          i === 0
            ? {
                ...f,
                overwrites: [
                  {
                    field: "position",
                    mine: "(-1.5, 7, 4.2)",
                    mvr: "(-2, 7, 4.2)",
                  },
                  { field: "patch", mine: "1:101", mvr: "1:1" },
                ],
              }
            : f,
        ),
        focus_points: MVR_INSPECTION.report.focus_points.map((f) => ({
          ...f,
          overwrites: [
            {
              field: "position",
              mine: "(0.4, 5.3, 1.4)",
              mvr: "(0, 5.3, 1.4)",
            },
          ],
        })),
      }),
    });
    await page.goto("/#/lighting/import");
    await chooseFile(page);
    await page.getByTestId("mvr-continue").click();
    await page.getByTestId("mvr-to-review").click();

    const edits = page.getByTestId("mvr-review-edits");
    await expect(edits).toContainText("Brick 1");
    await expect(edits).toContainText("(-1.5, 7, 4.2)");
    // Position starts kept; patch is a rig fact and starts as the MVR's.
    await expect(page.getByTestId("mvr-keep-Brick 1-position")).toBeChecked();
    await expect(page.getByTestId("mvr-keep-Brick 1-patch")).not.toBeChecked();
    await expect(page.getByTestId("mvr-keep-focus-Drummer")).toBeChecked();

    // A change of mind is carried into the write request.
    await page.getByTestId("mvr-keep-Brick 1-patch").check();
    await page.getByTestId("mvr-keep-focus-Drummer").uncheck();
    await page.getByTestId("mvr-do-import").click();
    await expect(page.getByTestId("mvr-done")).toBeVisible();
    const f = fields(imports[1]);
    expect(f.write).toBe("true");
    expect(JSON.parse(f.keep)).toEqual({
      fixtures: { "Brick 1": ["position", "patch"] },
      focus_points: [],
    });
  });

  test("a hand-changed mode is an overwrite, starts as the MVR's, and can be kept", async ({
    page,
  }) => {
    const { imports } = await routeImport(page, {
      plan: () => ({
        ...MVR_INSPECTION.report,
        merge: true,
        fixtures: MVR_INSPECTION.report.fixtures.map((f, i) =>
          i === 1
            ? {
                ...f,
                overwrites: [
                  { field: "mode", mine: '"8: RGBS"', mvr: '"9: RGBWS"' },
                ],
              }
            : f,
        ),
      }),
    });
    await page.goto("/#/lighting/import");
    await chooseFile(page);
    await page.getByTestId("mvr-continue").click();
    await page.getByTestId("mvr-to-review").click();

    await expect(page.getByTestId("mvr-edit-Brick 2-mode")).toContainText(
      'mode: yours "8: RGBS", the MVR\'s "9: RGBWS"',
    );
    // A mode is a rig fact, like the patch: the MVR's unless kept.
    const keep = page.getByTestId("mvr-keep-Brick 2-mode");
    await expect(keep).not.toBeChecked();
    await keep.check();
    await page.getByTestId("mvr-do-import").click();
    await expect(page.getByTestId("mvr-done")).toBeVisible();
    expect(JSON.parse(fields(imports[1]).keep)).toEqual({
      fixtures: { "Brick 2": ["mode"] },
      focus_points: [],
    });
  });

  test("a write that leaves the venue failing says so, prominently", async ({
    page,
  }) => {
    await routeImport(page, {
      venueError: {
        venue: "Kellys",
        fixture: "Brick 2",
        reason: 'mode "9: RGBWZ" of fixture type "Astera PB15" did not load',
      },
    });
    await page.goto("/#/lighting/import");
    await chooseFile(page);
    await page.getByTestId("mvr-continue").click();
    await page.getByTestId("mvr-to-review").click();
    await page.getByTestId("mvr-do-import").click();

    const alert = page.getByTestId("mvr-venue-error");
    await expect(alert).toHaveAttribute("role", "alert");
    await expect(alert).toContainText(
      'Venue "Kellys" will not load: fixture "Brick 2"',
    );
    await expect(alert).toContainText("no fixtures will light");
    await expect(page.getByTestId("mvr-done")).toBeVisible();
  });

  test("a clean write shows no venue error", async ({ page }) => {
    await routeImport(page);
    await page.goto("/#/lighting/import");
    await chooseFile(page);
    await page.getByTestId("mvr-continue").click();
    await page.getByTestId("mvr-to-review").click();
    await page.getByTestId("mvr-do-import").click();
    await expect(page.getByTestId("mvr-done")).toBeVisible();
    await expect(page.getByTestId("mvr-venue-error")).toHaveCount(0);
  });

  test("step 4 writes, reports, and offers Fit your shows and Open in Venues", async ({
    page,
  }) => {
    const { imports } = await routeImport(page);
    await page.goto("/#/lighting/import");
    await chooseFile(page);
    await page.getByTestId("mvr-venue-name").fill("Kellys Bar");
    await page.getByTestId("mvr-continue").click();
    await page.getByTestId("mvr-to-review").click();
    await page.getByTestId("mvr-do-import").click();

    await expect(page.getByTestId("mvr-step-import")).toHaveAttribute(
      "aria-current",
      "step",
    );
    await expect(page.getByTestId("mvr-done")).toContainText("Imported");
    await expect(page.getByTestId("mvr-done-written")).toContainText(
      "lighting/venues/kellys.venue",
    );
    // Importing does not make the venue current, and the page says so.
    await expect(page.getByTestId("mvr-not-current")).toContainText(
      "does not make this venue current",
    );
    await expect(page.getByTestId("mvr-go-fit")).toHaveAttribute(
      "href",
      "#/lighting/fit",
    );
    await expect(page.getByTestId("mvr-go-venues")).toHaveAttribute(
      "href",
      "#/lighting/venues",
    );

    // The file went up again with the write flag and the chosen name.
    expect(imports).toHaveLength(2);
    const f = fields(imports[1]);
    expect(f.write).toBe("true");
    expect(f.name).toBe("Kellys Bar");
    expect(f.origin).toBe("0,1000,500");

    await page.getByTestId("mvr-go-fit").click();
    await expect(page).toHaveURL(/#\/lighting\/fit/);
  });

  test("a server refusal at the write is shown with a retry", async ({
    page,
  }) => {
    const { rpc } = await routeImport(page);
    await page.goto("/#/lighting/import");
    await chooseFile(page);
    await page.getByTestId("mvr-continue").click();
    await page.getByTestId("mvr-to-review").click();
    await expect(page.getByTestId("mvr-step-review")).toHaveAttribute(
      "aria-current",
      "step",
    );

    let calls = 0;
    rpc.unary(LightingService.method.importMvr, () => {
      calls++;
      if (calls === 1) {
        throw new ConnectError(
          "lighting/library/Kellys.mvr differs",
          Code.InvalidArgument,
        );
      }
      return mvrImportReply({
        write: true,
        report: {
          ...MVR_INSPECTION.report,
          written: ["lighting/venues/kellys.venue"],
          distillation_warnings: {},
        },
        reloaded: false,
      });
    });
    await page.getByTestId("mvr-do-import").click();
    await expect(page.getByTestId("mvr-error")).toContainText("differs");
    await page
      .getByTestId("mvr-error")
      .getByRole("button", { name: "Retry" })
      .click();
    await expect(page.getByTestId("mvr-done")).toBeVisible();
  });

  test("the wizard is reached from Venues and from the overview", async ({
    page,
  }) => {
    await page.goto("/#/lighting/venues");
    await page.getByTestId("venues-import-mvr").click();
    await expect(page).toHaveURL(/#\/lighting\/import/);
    await expect(page.getByTestId("mvr-wizard")).toBeVisible();

    await page.goto("/#/lighting");
    await page.getByTestId("hub-import-mvr").click();
    await expect(page).toHaveURL(/#\/lighting\/import/);
  });

  test("works at phone width without sideways scroll", async ({ page }) => {
    await page.setViewportSize({ width: 375, height: 700 });
    await routeImport(page);
    await page.goto("/#/lighting/import");
    await chooseFile(page);
    await page.getByTestId("mvr-continue").click();
    await expect(page.getByTestId("mvr-plan")).toBeVisible();
    const overflow = await page.evaluate(
      () =>
        document.documentElement.scrollWidth >
        document.documentElement.clientWidth,
    );
    expect(overflow).toBe(false);
  });
});

test.describe("MVR export dialog", () => {
  /** Answers the summary, the aim points and the keep; `linked` says whether
   *  Brick 2 has been aimed. An export is staged for download: the answered
   *  path serves the archive. */
  async function routeExport(page: Page) {
    const state = { linked: false };
    const staged = new Map<string, string>();
    const kept = new Set<string>();
    const rpc = await mockRpc(page);
    rpc.unary(LightingService.method.keepMvrExport, (request) => {
      const file = request.file ?? "x.mvr";
      if (kept.has(file) && !request.overwrite) {
        return keptReply({ existing: file });
      }
      kept.add(file);
      return keptReply({ kept: `lighting/export/${file}` });
    });
    rpc.unary(LightingService.method.getMvrExportSummary, () => {
      const summary = structuredClone(MVR_EXPORT_SUMMARY);
      summary.venue = "test-venue";
      if (state.linked) {
        summary.fixed_fixtures[1].focus = "Brick 2 aim";
        summary.focus_points = 2;
      }
      return mvrExportSummaryReply(summary);
    });
    rpc.unary(LightingService.method.exportMvr, (request) => {
      const file = request.file ?? "x.mvr";
      const id = `staged-${staged.size + 1}`;
      staged.set(id, file);
      return exportMvrReply({ path: `/downloads/${id}`, fileName: file });
    });
    rpc.unary(LightingService.method.addVenueAimPoints, () => {
      state.linked = true;
      return aimPointsReply({
        created: [
          { name: "Brick 2 aim", fixture: "Brick 2", point: [2, 2.5, 0] },
        ],
        reloaded: true,
        version: "v2",
        venue_error: null,
      });
    });
    return {
      rpc,
      summaries: () => rpc.calls(LightingService.method.getMvrExportSummary),
      aims: () => rpc.calls(LightingService.method.addVenueAimPoints),
      exports: () => rpc.calls(LightingService.method.exportMvr),
      keeps: () => rpc.calls(LightingService.method.keepMvrExport),
    };
  }

  test("shows the summary and warns about unlinked fixed fixtures", async ({
    page,
  }) => {
    const { summaries } = await routeExport(page);
    await page.goto("/#/lighting/venues");
    await page.getByTestId("venue-export-mvr-test-venue").click();

    const dialog = page.getByTestId("mvr-export-dialog");
    await expect(dialog).toBeVisible();
    await expect(dialog).toHaveAccessibleName("Export test-venue as an MVR");
    await expect(page.getByTestId("mvr-export-file")).toHaveValue(
      "test_venue.mvr",
    );
    await expect(page.getByTestId("mvr-export-fixtures")).toContainText(
      "2 (2 with positions)",
    );
    await expect(page.getByTestId("mvr-export-gdtfs")).toContainText(
      "1 embedded, 0 generated",
    );
    await expect(page.getByTestId("mvr-export-linked")).toHaveText("1 of 2");
    await expect(page.getByTestId("mvr-export-unlinked")).toContainText(
      "1 fixed fixtures are not aimed at a focus point",
    );
    await expect(page.getByTestId("mvr-export-unlinked")).toContainText(
      "Brick 2",
    );
    // The unlinked warning is where the choice is made: the plain Download
    // waits for it.
    await expect(page.getByTestId("mvr-export-download")).toHaveCount(0);
    await expect(page.getByTestId("mvr-export-as-is")).toBeVisible();

    expect(summaries().length).toBeGreaterThan(0);
    expect(summaries()[0].venue).toBe("test-venue");
  });

  test("does not offer aim points for a venue whose file is not its alone", async ({
    page,
  }) => {
    const { aims, rpc } = await routeExport(page);
    const reason =
      'Defined in tour/all.light alongside venue "Arena"; edit that file by hand.';
    rpc.unary(
      LightingService.method.listVenues,
      venueListReply({
        venues: {
          "test-venue": {
            name: "test-venue",
            fixtures: {},
            focus_points: {},
            source: null,
          },
        },
        owners: {
          "test-venue": { file: "tour/all.light", editable: false, reason },
        },
        errors: [],
        versions: { "test-venue": "v1" },
      } satisfies VenueList),
    );
    await page.goto("/#/lighting/venues");
    await page.getByTestId("venue-export-mvr-test-venue").click();
    await expect(page.getByTestId("mvr-export-unlinked")).toBeVisible();
    await expect(page.getByTestId("mvr-export-add-aim")).toHaveCount(0);
    await expect(page.getByTestId("mvr-export-read-only")).toHaveText(reason);
    // Exporting as it is stays on offer.
    await expect(page.getByTestId("mvr-export-as-is")).toBeVisible();
    expect(aims()).toHaveLength(0);
  });

  test("adds an aim point per fixture, refreshes, then downloads", async ({
    page,
  }) => {
    const { summaries, aims, exports, keeps } = await routeExport(page);
    await page.goto("/#/lighting/venues");
    await page.getByTestId("venue-export-mvr-test-venue").click();
    await expect(page.getByTestId("mvr-export-unlinked")).toBeVisible();
    const before = summaries().length;

    await page.getByTestId("mvr-export-add-aim").click();
    await expect(page.getByTestId("mvr-export-added")).toContainText(
      "Added 1 aim points: Brick 2 aim",
    );
    // The action posted to the venue, and the summary was read again.
    expect(aims()).toHaveLength(1);
    expect(aims()[0].name).toBe("test-venue");
    await expect.poll(() => summaries().length).toBeGreaterThan(before);
    await expect(page.getByTestId("mvr-export-linked")).toHaveText("2 of 2");
    await expect(page.getByTestId("mvr-export-unlinked")).toHaveCount(0);

    // Download: the file name, layers and keep flag reach the request, and
    // the browser is handed the file under that name.
    await page.getByTestId("mvr-export-file").fill("tour.mvr");
    await page.getByTestId("mvr-export-layers").check();
    await page.getByTestId("mvr-export-keep").check();
    const download = page.waitForEvent("download");
    await page.getByTestId("mvr-export-download").click();
    const saved = await download;
    expect(saved.suggestedFilename()).toBe("tour.mvr");
    // The browser fetched the archive from the path that answered.
    expect(new URL(saved.url()).pathname).toBe("/downloads/staged-1");

    expect(exports()).toHaveLength(1);
    expect(exports()[0].venue).toBe("test-venue");
    expect(exports()[0].file).toBe("tour.mvr");
    expect(exports()[0].layersFromTags).toBe(true);
    // An export writes nothing: the copy is its own call, after the download.
    // The copy is requested after the download resolves, so wait for the
    // note the dialog shows once the call has answered.
    await expect(page.getByTestId("mvr-export-kept")).toContainText(
      "lighting/export/tour.mvr",
    );
    expect(keeps()).toHaveLength(1);
    expect(keeps()[0].file).toBe("tour.mvr");

    // Again: the name is taken, so the dialog asks before replacing it.
    const again = page.waitForEvent("download");
    await page.getByTestId("mvr-export-download").click();
    await again;
    const confirm = page.getByRole("dialog").filter({
      hasText: "Replace lighting/export/tour.mvr?",
    });
    await expect(confirm).toBeVisible();
    await confirm.getByRole("button", { name: "Confirm" }).click();
    await expect.poll(() => keeps().length).toBe(3);
    expect(keeps()[2].overwrite).toBe(true);
  });

  test("Export as is downloads without adding aim points, keep off by default", async ({
    page,
  }) => {
    const { aims, exports, keeps } = await routeExport(page);
    await page.goto("/#/lighting/venues");
    await page.getByTestId("venue-export-mvr-test-venue").click();

    const download = page.waitForEvent("download");
    await page.getByTestId("mvr-export-as-is").click();
    expect((await download).suggestedFilename()).toBe("test_venue.mvr");

    expect(aims()).toHaveLength(0);
    expect(exports()).toHaveLength(1);
    expect(exports()[0].layersFromTags).toBe(false);
    expect(keeps()).toHaveLength(0);
  });

  test("a file name the server would refuse cannot be downloaded", async ({
    page,
  }) => {
    await routeExport(page);
    await page.goto("/#/lighting/venues");
    await page.getByTestId("venue-export-mvr-test-venue").click();
    await expect(page.getByTestId("mvr-export-unlinked")).toBeVisible();

    await page.getByTestId("mvr-export-file").fill("../tour");
    await expect(page.getByTestId("mvr-export-file")).toHaveAttribute(
      "aria-invalid",
      "true",
    );
    await expect(page.getByTestId("mvr-export-as-is")).toBeDisabled();
    await page.getByTestId("mvr-export-file").fill("tour.mvr");
    await expect(page.getByTestId("mvr-export-as-is")).toBeEnabled();
  });

  test("a summary error is shown with a retry, and Escape closes the dialog", async ({
    page,
  }) => {
    let calls = 0;
    const rpc = await mockRpc(page);
    rpc.unary(LightingService.method.getMvrExportSummary, () => {
      calls++;
      if (calls === 1) {
        throw new ConnectError(
          'fixture type "brick" is not defined',
          Code.InvalidArgument,
        );
      }
      return mvrExportSummaryReply({
        ...MVR_EXPORT_SUMMARY,
        fixed_fixtures: [],
      });
    });
    await page.goto("/#/lighting/venues");
    await page.getByTestId("venue-export-mvr-test-venue").click();
    await expect(page.getByTestId("mvr-export-error")).toContainText(
      "is not defined",
    );
    await page
      .getByRole("dialog")
      .getByRole("button", { name: "Retry" })
      .click();
    await expect(page.getByTestId("mvr-export-download")).toBeVisible();

    await page.keyboard.press("Escape");
    await expect(page.getByTestId("mvr-export-dialog")).toHaveCount(0);
  });
});
