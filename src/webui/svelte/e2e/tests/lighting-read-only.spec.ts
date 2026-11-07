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

// The web UI writes a venue or fixture type only where its file is that one
// thing's alone. Anything else (a file shared with other venues or types,
// holding other items, or a name declared twice) is shown read-only, with
// the file and why, and can be edited by hand.

import { test, expect, type Page } from "@playwright/test";
import type {
  Fixture,
  FixtureTypeEntry,
  FixtureTypeList,
  VenueList,
} from "../../src/gen/rules/mtrack_rules";
import { card } from "../support/locators";

const SHARED =
  'Defined in tour/all.light alongside venue "Arena"; edit that file by hand.';

const spot: Fixture = {
  name: "Spot",
  fixture_type: "par",
  universe: 1,
  start_channel: 1,
  tags: [],
  position: null,
  rotation: null,
};

const venues = {
  venues: {
    Club: {
      name: "Club",
      fixtures: { Spot: spot },
      focus_points: {},
      source: null,
    },
    House: {
      name: "House",
      fixtures: { Spot: spot },
      focus_points: {},
      source: null,
    },
  },
  owners: {
    Club: { file: "tour/all.light", editable: false, reason: SHARED },
    House: { file: "house.venue", editable: true, reason: null },
  },
  errors: [],
  versions: { Club: "v1", House: "v1" },
} satisfies VenueList;

/** Serves `venues` as the listing, and records every write of a venue. */
async function routeVenues(page: Page) {
  const writes: string[] = [];
  await page.route(/\/api\/lighting\/venues(\?.*)?$/, (route) =>
    route.fulfill({ json: venues }),
  );
  await page.route(/\/api\/lighting\/venues\/[^/?]+(\?.*)?$/, (route) => {
    const method = route.request().method();
    if (method !== "GET") writes.push(`${method} ${route.request().url()}`);
    return route.fallback();
  });
  return writes;
}

test.describe("A venue whose file is not its alone", () => {
  test("its editor names the file and why, with save and rename disabled", async ({
    page,
  }) => {
    const writes = await routeVenues(page);
    await page.goto("/#/lighting/venues");
    await page.getByTestId("venue-edit-Club").click();
    const note = page.getByTestId("venue-read-only");
    await expect(note).toBeVisible();
    await expect(page.getByTestId("venue-read-only-file")).toHaveText(
      "tour/all.light",
    );
    await expect(page.getByTestId("venue-read-only-reason")).toHaveText(SHARED);
    await expect(
      page.locator(".editor-form").getByRole("button", { name: "Save" }),
    ).toBeDisabled();
    await expect(page.locator("#venue-name")).toBeDisabled();
    // A click on the disabled Save, forced past the check, sends nothing.
    await page
      .locator(".editor-form")
      .getByRole("button", { name: "Save" })
      .click({ force: true });
    await page.waitForTimeout(300);
    expect(writes).toHaveLength(0);
  });

  test("its card cannot delete it; an editable venue's can", async ({
    page,
  }) => {
    const writes = await routeVenues(page);
    await page.goto("/#/lighting/venues");
    await expect(page.getByTestId("venue-delete-Club")).toBeDisabled();
    await expect(page.getByTestId("venue-delete-Club")).toHaveAttribute(
      "title",
      SHARED,
    );
    await expect(page.getByTestId("venue-delete-House")).toBeEnabled();
    // Forced, the disabled Delete asks nothing and deletes nothing.
    await page.getByTestId("venue-delete-Club").click({ force: true });
    await page.waitForTimeout(300);
    await expect(page.locator(".dialog-overlay")).toHaveCount(0);
    expect(writes).toHaveLength(0);
  });

  test("an editable venue's editor has no note and saves", async ({ page }) => {
    await routeVenues(page);
    await page.goto("/#/lighting/venues");
    await page.getByTestId("venue-edit-House").click();
    await expect(page.locator(".editor-form")).toBeVisible();
    await expect(page.getByTestId("venue-read-only")).toHaveCount(0);
    await expect(
      page.locator(".editor-form").getByRole("button", { name: "Save" }),
    ).toBeEnabled();
    await expect(page.locator("#venue-name")).toBeEnabled();
  });
});

const TYPE_SHARED =
  'Defined in all.light alongside fixture type "Wash B"; edit that file by hand.';

const washType = (name: string): FixtureTypeEntry => ({
  fixture_type: {
    name,
    channels: { dimmer: 1 },
    max_strobe_frequency: null,
    min_strobe_frequency: null,
    strobe_dmx_offset: null,
  },
  file: "all.light",
  extension: "light",
  owner: { file: "all.light", editable: false, reason: TYPE_SHARED },
  form_editable: true,
  referential: false,
  rich: false,
  footprint: 1,
});

const types = {
  fixture_types: { "Wash A": washType("Wash A") },
  errors: [],
} satisfies FixtureTypeList;

test.describe("A fixture type whose file is not its alone", () => {
  test.beforeEach(async ({ page }) => {
    await page.route(/\/api\/lighting\/fixture-types(\?.*)?$/, (route) =>
      route.fulfill({ json: types }),
    );
  });

  test("its card cannot delete it", async ({ page }) => {
    await page.goto("/#/lighting/fixtures");
    const del = card(page, "Wash A").getByTestId("ft-delete");
    await expect(del).toBeDisabled();
    await expect(del).toHaveAttribute("title", TYPE_SHARED);
  });

  test("its editor names the file and why, with save and rename disabled", async ({
    page,
  }) => {
    const writes: string[] = [];
    page.on("request", (req) => {
      if (
        req.url().includes("/api/lighting/fixture-types/") &&
        req.method() !== "GET"
      )
        writes.push(req.method());
    });
    await page.goto("/#/lighting/fixtures");
    await card(page, "Wash A").click();
    await expect(page.getByTestId("ft-read-only")).toBeVisible();
    await expect(page.getByTestId("ft-read-only-file")).toHaveText("all.light");
    await expect(page.getByTestId("ft-read-only-reason")).toHaveText(
      TYPE_SHARED,
    );
    await expect(
      page.locator(".editor-form").getByRole("button", { name: "Save" }),
    ).toBeDisabled();
    await expect(page.locator("#ft-name")).toBeDisabled();
    // A click on the disabled Save, forced past the check, sends nothing.
    await page
      .locator(".editor-form")
      .getByRole("button", { name: "Save" })
      .click({ force: true });
    await page.waitForTimeout(300);
    expect(writes).toHaveLength(0);
  });
});
