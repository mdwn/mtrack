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

// The venue form: a save shows on the page at once, and each GDTF row
// chooses its own mode. Every test serves its own venue directory from a
// closure (page.route), so the shared mock is never written.

interface Fixture {
  name: string;
  fixture_type: string;
  universe: number;
  start_channel: number;
  tags: string[];
  position?: [number, number, number] | null;
  rotation?: [number, number, number] | null;
  mode?: string | null;
}

const MODES = [
  { name: "8: RGBS", channel_count: 4, footprint: 4 },
  { name: "9: RGBWS", channel_count: 5, footprint: 5 },
  { name: "99: Broken", channel_count: 1, footprint: 1, refused: "no" },
];

function typeEntry(referential: boolean, footprint: number) {
  return {
    fixture_type: {
      name: "t",
      channels: {},
      max_strobe_frequency: null,
      min_strobe_frequency: null,
      strobe_dmx_offset: null,
    },
    file: referential ? "brick.fixture" : "par.light",
    extension: referential ? "fixture" : "light",
    referential,
    default_mode: referential ? "8: RGBS" : null,
    rich: false,
    footprint,
    gdtf: null,
  };
}

/** Serves a venue directory holding `initial`, keeping every save. */
async function serve(page: Page, initial: Record<string, Fixture[]>) {
  const venues: Record<string, Fixture[]> = structuredClone(initial);
  let version = 1;
  const puts: { name: string; fixtures: Fixture[] }[] = [];
  const json = (body: unknown) => ({
    status: 200,
    contentType: "application/json",
    body: JSON.stringify(body),
  });
  const venueJson = (name: string) => ({
    name,
    fixtures: Object.fromEntries(venues[name].map((f) => [f.name, f])),
    focus_points: {},
    source: null,
  });
  await page.route(/\/api\/lighting\/venues(\?.*)?$/, (route) =>
    route.fulfill(
      json({
        venues: Object.fromEntries(
          Object.keys(venues).map((n) => [n, venueJson(n)]),
        ),
        versions: Object.fromEntries(
          Object.keys(venues).map((n) => [n, `v${version}`]),
        ),
        errors: [],
      }),
    ),
  );
  await page.route(/\/api\/lighting\/venues\/[^/?]+(\?.*)?$/, (route) => {
    const name = decodeURIComponent(
      new URL(route.request().url()).pathname.split("/").pop()!,
    );
    if (route.request().method() === "PUT") {
      const body = route.request().postDataJSON();
      venues[name] = body.fixtures;
      puts.push({ name, fixtures: body.fixtures });
      version++;
      return route.fulfill(
        json({ status: "saved", version: `v${version}`, venue_error: null }),
      );
    }
    if (route.request().method() !== "GET") return route.fallback();
    if (!venues[name]) return route.fulfill({ status: 404, body: "{}" });
    return route.fulfill(
      json({ venue: venueJson(name), dsl: "", version: `v${version}` }),
    );
  });
  await page.route("**/api/lighting/fixture-types*", (route) => {
    if (/fixture-types\/[^?]/.test(route.request().url()))
      return route.fallback();
    return route.fulfill(
      json({
        fixture_types: {
          brick: typeEntry(true, 4),
          par: typeEntry(false, 3),
        },
        errors: [],
      }),
    );
  });
  await page.route("**/api/lighting/fixture-types/brick/gdtf*", (route) =>
    route.fulfill(
      json({
        archive: "library/pb15.gdtf",
        mode: "8: RGBS",
        matched_mode: "8: RGBS",
        rig: null,
        thumbnail: null,
        beam: null,
        about: null,
        venues: [],
        inspection: {
          fixture: "PB15",
          manufacturer: "Astera",
          modes: MODES,
        },
      }),
    ),
  );
  await page.goto("/#/lighting/venues");
  return { puts, venues };
}

const card = (page: Page, name: string) =>
  page.locator(".item-card").filter({
    has: page.locator(".item-name", { hasText: new RegExp(`^${name}$`) }),
  });
const rows = (page: Page) => page.getByTestId("venue-fixture-row");
const save = (page: Page) =>
  page.locator(".editor-form").getByRole("button", { name: "Save" }).click();
const plotTitle = (page: Page) => page.locator(".stage-card__title");

const placed = (name: string, at: number): Fixture => ({
  name,
  fixture_type: "par",
  universe: 1,
  start_channel: at,
  tags: [],
  position: [at / 2, 2, 3],
  rotation: null,
});

test.describe("A save shows at once", () => {
  test("editing a selected venue updates its plot without navigating away", async ({
    page,
  }) => {
    await serve(page, { club: [placed("A", 1), placed("B", 4)] });
    await card(page, "club").click();
    await expect(plotTitle(page)).toContainText("club");
    await expect(plotTitle(page)).toContainText("2 fixtures");

    await card(page, "club").locator('[data-testid^="venue-edit-"]').click();
    await page.getByRole("button", { name: "Add Fixture" }).click();
    await save(page);
    await expect(page.locator(".editor-form")).toHaveCount(0);

    await expect(card(page, "club")).toContainText("3");
    await expect(plotTitle(page)).toContainText("3 fixtures");
  });

  test("an empty venue gaining fixtures updates its plot", async ({ page }) => {
    await serve(page, { test: [] });
    await card(page, "test").click();
    await expect(plotTitle(page)).toContainText("0 fixtures");
    await card(page, "test").locator('[data-testid^="venue-edit-"]').click();
    await page.getByRole("button", { name: "Add Fixture" }).click();
    await page.getByRole("button", { name: "Add Fixture" }).click();
    await save(page);
    await expect(page.locator(".editor-form")).toHaveCount(0);
    await expect(plotTitle(page)).toContainText("2 fixtures");
  });
});

test.describe("A mode per fixture in the venue form", () => {
  test("rows mix modes, continue by each row's own footprint, and save exactly that", async ({
    page,
  }) => {
    const { puts } = await serve(page, {});
    await page.getByRole("button", { name: "New Venue" }).click();
    await page.locator("#venue-name").fill("mix");
    await page.getByRole("button", { name: "Add Fixture" }).click();
    await rows(page).nth(0).getByLabel("Fixture type").selectOption("brick");
    const mode = (i: number) => rows(page).nth(i).getByTestId("venue-row-mode");
    await expect(mode(0)).toHaveValue("");
    await expect(mode(0).locator("option").first()).toHaveText(
      "Type default (8: RGBS)",
    );
    await expect(mode(0).locator('option[value="99: Broken"]')).toBeDisabled();
    await expect(rows(page).nth(0).getByTestId("venue-row-span")).toHaveText(
      "Addresses 1–4",
    );

    await page.getByRole("button", { name: "Add Fixture" }).click();
    await expect(rows(page).nth(1).locator("#fix-channel-1")).toHaveValue("5");
    await mode(1).selectOption("9: RGBWS");
    await expect(rows(page).nth(1).getByTestId("venue-row-span")).toHaveText(
      "Addresses 5–9",
    );
    // The next row continues after the 5-address mode, not the default's 4.
    await page.getByRole("button", { name: "Add Fixture" }).click();
    await expect(rows(page).nth(2).locator("#fix-channel-2")).toHaveValue("10");
    await expect(mode(2)).toHaveValue("");

    await save(page);
    await expect.poll(() => puts.length).toBe(1);
    expect(
      puts[0].fixtures.map((f) => [f.name, f.start_channel, f.mode ?? null]),
    ).toEqual([
      ["Fixture 1", 1, null],
      ["Fixture 2", 5, "9: RGBWS"],
      ["Fixture 3", 10, null],
    ]);

    // Reopened, the form shows the same.
    await card(page, "mix").locator('[data-testid^="venue-edit-"]').click();
    await expect(mode(1)).toHaveValue("9: RGBWS");
    await expect(mode(0)).toHaveValue("");
  });

  test("an overlap is marked, and the save asks first", async ({ page }) => {
    const { puts } = await serve(page, {
      rig: [
        { ...placed("A", 1), fixture_type: "brick" },
        { ...placed("B", 5), fixture_type: "brick" },
      ],
    });
    await card(page, "rig").locator('[data-testid^="venue-edit-"]').click();
    await rows(page)
      .nth(0)
      .getByTestId("venue-row-mode")
      .selectOption("9: RGBWS");
    await expect(rows(page).nth(0).getByTestId("venue-row-overlap")).toHaveText(
      "Runs into B (addresses 5–5).",
    );
    await expect(rows(page).nth(1).getByTestId("venue-row-overlap")).toHaveText(
      "Runs into A (addresses 5–5).",
    );
    await save(page);
    const dialog = page.locator(".dialog-overlay");
    await expect(dialog).toContainText("Save anyway?");
    await expect(dialog).toContainText("Runs into B");
    await dialog.getByRole("button", { name: "Cancel" }).click();
    await page.waitForTimeout(300);
    expect(puts).toHaveLength(0);

    await save(page);
    await dialog.getByRole("button", { name: "Confirm" }).click();
    await expect.poll(() => puts.length).toBe(1);
    expect(puts[0].fixtures[0].mode).toBe("9: RGBWS");
  });

  test("a saved mode the archive lacks is shown, marked, and kept", async ({
    page,
  }) => {
    const { puts } = await serve(page, {
      odd: [{ ...placed("A", 1), fixture_type: "brick", mode: "7: Gone" }],
    });
    await card(page, "odd").locator('[data-testid^="venue-edit-"]').click();
    const mode = rows(page).nth(0).getByTestId("venue-row-mode");
    await expect(mode).toHaveValue("7: Gone");
    await expect(mode.locator('option[value="7: Gone"]')).toHaveText(
      "7: Gone — not in this archive",
    );
    await save(page);
    await expect.poll(() => puts.length).toBe(1);
    expect(puts[0].fixtures[0].mode).toBe("7: Gone");
  });

  test("a native type has no mode select, and a type change resets the mode", async ({
    page,
  }) => {
    const { puts } = await serve(page, {
      rig: [{ ...placed("A", 1), fixture_type: "brick", mode: "9: RGBWS" }],
    });
    await card(page, "rig").locator('[data-testid^="venue-edit-"]').click();
    const row = rows(page).nth(0);
    await expect(row.getByTestId("venue-row-mode")).toHaveValue("9: RGBWS");
    await row.getByLabel("Fixture type").selectOption("par");
    await expect(row.getByTestId("venue-row-mode")).toHaveCount(0);
    await row.getByLabel("Fixture type").selectOption("brick");
    await expect(row.getByTestId("venue-row-mode")).toHaveValue("");
    await save(page);
    await expect.poll(() => puts.length).toBe(1);
    expect(puts[0].fixtures[0].mode ?? null).toBeNull();
  });
});
