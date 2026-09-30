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

import { test, expect, type Page, type Request } from "@playwright/test";
import { MVR_INSPECTION, MVR_EXPORT_SUMMARY } from "../mock-server/test-data";

// MVR import (the wizard at #/lighting/import) and export (the dialog on
// Venues), design section 11. Every test routes its own answers with
// `page.route`, so nothing depends on state another test left behind.

type Inspection = typeof MVR_INSPECTION;

const MVR_FILE = {
  name: "Kellys.mvr",
  mimeType: "application/octet-stream",
  buffer: Buffer.from("PK-not-really-an-mvr"),
};

/** The text fields of a multipart body, by name. */
function fields(request: Request): Record<string, string> {
  const body = request.postData() ?? "";
  const out: Record<string, string> = {};
  const re = /name="([^"]+)"(?!; filename)\r\n\r\n([^\r]*)\r\n/g;
  for (const m of body.matchAll(re)) out[m[1]] = m[2];
  return out;
}

function inspection(change: (i: Inspection) => void = () => {}): Inspection {
  const copy = structuredClone(MVR_INSPECTION);
  change(copy);
  return copy;
}

/** Routes inspect and import; import requests are collected. */
async function routeImport(
  page: Page,
  opts: {
    inspect?: Inspection;
    plan?: (origin: string) => object;
    report?: object;
  } = {},
) {
  const inspected: Request[] = [];
  const imports: Request[] = [];
  await page.route("**/api/lighting/mvr/inspect", async (route) => {
    inspected.push(route.request());
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(opts.inspect ?? MVR_INSPECTION),
    });
  });
  await page.route("**/api/lighting/mvr/import", async (route) => {
    const request = route.request();
    imports.push(request);
    const f = fields(request);
    const body =
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
          }
        : {
            write: false,
            plan: opts.plan?.(f.origin) ?? MVR_INSPECTION.report,
          };
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(body),
    });
  });
  return { inspected, imports };
}

async function chooseFile(page: Page) {
  await page.getByTestId("mvr-file").setInputFiles(MVR_FILE);
}

test.describe("MVR import wizard", () => {
  test("step 1 reads the file and shows what it holds", async ({ page }) => {
    const { inspected } = await routeImport(page);
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

    // The browser kept the file and sent it, under its own name.
    expect(inspected).toHaveLength(1);
    expect(inspected[0].postData()).toContain('filename="Kellys.mvr"');
  });

  test("an error at step 1 is shown in place and retry recovers", async ({
    page,
  }) => {
    let calls = 0;
    await page.route("**/api/lighting/mvr/inspect", async (route) => {
      calls++;
      if (calls === 1) {
        await route.fulfill({
          status: 400,
          contentType: "application/json",
          body: JSON.stringify({
            error: "not a readable MVR archive (is this an MVR file?)",
          }),
        });
      } else {
        await route.fulfill({
          status: 200,
          contentType: "application/json",
          body: JSON.stringify(MVR_INSPECTION),
        });
      }
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
            mode: "Mode 1",
            existing: true,
            fixture_file: "lighting/fixture_types/robe_spot.fixture",
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
    await routeImport(page);
    await page.goto("/#/lighting/import");
    await chooseFile(page);
    await page.getByTestId("mvr-continue").click();
    await page.getByTestId("mvr-to-review").click();

    await page.unroute("**/api/lighting/mvr/import");
    let calls = 0;
    await page.route("**/api/lighting/mvr/import", async (route) => {
      calls++;
      await route.fulfill(
        calls === 1
          ? {
              status: 400,
              contentType: "application/json",
              body: JSON.stringify({
                error: "lighting/library/Kellys.mvr differs",
              }),
            }
          : {
              status: 200,
              contentType: "application/json",
              body: JSON.stringify({
                write: true,
                report: {
                  ...MVR_INSPECTION.report,
                  written: ["lighting/venues/kellys.venue"],
                  distillation_warnings: {},
                },
                reloaded: false,
              }),
            },
      );
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
  /** Routes the summary; `linked` says whether Brick 2 has been aimed. */
  async function routeExport(page: Page) {
    const state = { linked: false };
    const summaries: Request[] = [];
    const aims: Request[] = [];
    const downloads: Request[] = [];
    await page.route("**/api/lighting/mvr/export/summary*", async (route) => {
      summaries.push(route.request());
      const summary = structuredClone(MVR_EXPORT_SUMMARY);
      summary.venue = "test-venue";
      if (state.linked) {
        summary.fixed_fixtures[1].focus = "Brick 2 aim";
        summary.focus_points = 2;
      }
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify(summary),
      });
    });
    await page.route(/\/api\/lighting\/mvr\/export\?/, async (route) => {
      downloads.push(route.request());
      const url = new URL(route.request().url());
      const file = url.searchParams.get("file") ?? "x.mvr";
      await route.fulfill({
        status: 200,
        headers: {
          "content-type": "application/octet-stream",
          "content-disposition": `attachment; filename="${file}"`,
          "x-mtrack-kept": `lighting/export/${file}`,
        },
        body: "PK-mock",
      });
    });
    await page.route("**/api/lighting/venues/*/aim-points*", async (route) => {
      aims.push(route.request());
      state.linked = true;
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({
          created: [
            { name: "Brick 2 aim", fixture: "Brick 2", point: [2, 2.5, 0] },
          ],
          reloaded: true,
        }),
      });
    });
    return { summaries, aims, downloads };
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

    expect(summaries.length).toBeGreaterThan(0);
    expect(new URL(summaries[0].url()).searchParams.get("venue")).toBe(
      "test-venue",
    );
  });

  test("adds an aim point per fixture, refreshes, then downloads", async ({
    page,
  }) => {
    const { summaries, aims, downloads } = await routeExport(page);
    await page.goto("/#/lighting/venues");
    await page.getByTestId("venue-export-mvr-test-venue").click();
    await expect(page.getByTestId("mvr-export-unlinked")).toBeVisible();
    const before = summaries.length;

    await page.getByTestId("mvr-export-add-aim").click();
    await expect(page.getByTestId("mvr-export-added")).toContainText(
      "Added 1 aim points: Brick 2 aim",
    );
    // The action posted to the venue, and the summary was read again.
    expect(aims).toHaveLength(1);
    expect(aims[0].method()).toBe("POST");
    expect(aims[0].url()).toContain(
      "/api/lighting/venues/test-venue/aim-points",
    );
    await expect.poll(() => summaries.length).toBeGreaterThan(before);
    await expect(page.getByTestId("mvr-export-linked")).toHaveText("2 of 2");
    await expect(page.getByTestId("mvr-export-unlinked")).toHaveCount(0);

    // Download: the file name, layers and keep flag reach the request, and
    // the browser is handed the file under that name.
    await page.getByTestId("mvr-export-file").fill("tour.mvr");
    await page.getByTestId("mvr-export-layers").check();
    await page.getByTestId("mvr-export-keep").check();
    const download = page.waitForEvent("download");
    await page.getByTestId("mvr-export-download").click();
    expect((await download).suggestedFilename()).toBe("tour.mvr");

    expect(downloads).toHaveLength(1);
    const params = new URL(downloads[0].url()).searchParams;
    expect(params.get("venue")).toBe("test-venue");
    expect(params.get("file")).toBe("tour.mvr");
    expect(params.get("layers_from_tags")).toBe("true");
    expect(params.get("keep")).toBe("true");
    await expect(page.getByTestId("mvr-export-kept")).toContainText(
      "lighting/export/tour.mvr",
    );
  });

  test("Export as is downloads without adding aim points, keep off by default", async ({
    page,
  }) => {
    const { aims, downloads } = await routeExport(page);
    await page.goto("/#/lighting/venues");
    await page.getByTestId("venue-export-mvr-test-venue").click();

    const download = page.waitForEvent("download");
    await page.getByTestId("mvr-export-as-is").click();
    expect((await download).suggestedFilename()).toBe("test_venue.mvr");

    expect(aims).toHaveLength(0);
    expect(downloads).toHaveLength(1);
    const params = new URL(downloads[0].url()).searchParams;
    expect(params.get("layers_from_tags")).toBe("false");
    expect(params.get("keep")).toBe("false");
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
    await page.route("**/api/lighting/mvr/export/summary*", async (route) => {
      calls++;
      await route.fulfill(
        calls === 1
          ? {
              status: 400,
              contentType: "application/json",
              body: JSON.stringify({
                error: 'fixture type "brick" is not defined',
              }),
            }
          : {
              status: 200,
              contentType: "application/json",
              body: JSON.stringify({
                ...MVR_EXPORT_SUMMARY,
                fixed_fixtures: [],
              }),
            },
      );
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
