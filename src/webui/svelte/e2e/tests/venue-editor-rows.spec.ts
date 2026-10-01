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

// The venue editor's rows: a new row continues the patch and has a name, and
// a row that cannot be saved blocks the save instead of being dropped. Every
// answer is routed per test (page.route); the shared mock is untouched, and
// every PUT is caught here, never reaching it.

interface Put {
  fixtures: {
    name: string;
    fixture_type: string;
    universe: number;
    start_channel: number;
  }[];
}

function type(footprint: number | null) {
  return {
    fixture_type: {
      name: "t",
      channels: {},
      max_strobe_frequency: null,
      min_strobe_frequency: null,
      strobe_dmx_offset: null,
    },
    file: "t.light",
    extension: "light",
    referential: false,
    rich: false,
    footprint,
  };
}

async function setUp(page: Page) {
  const puts: Put[] = [];
  await page.route("**/api/lighting/fixture-types*", (route) => {
    if (/fixture-types\/[^?]/.test(route.request().url()))
      return route.fallback();
    return route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        fixture_types: {
          // Listed in this order, "apar" sorts first: a new row after a
          // "wash" row must still be a "wash".
          apar: type(3),
          wash: type(4),
          mystery: type(null),
        },
        errors: [],
      }),
    });
  });
  await page.route(/\/api\/lighting\/venues\/[^/?]+(\?.*)?$/, (route) => {
    if (route.request().method() !== "PUT") return route.fallback();
    puts.push(route.request().postDataJSON());
    return route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({ status: "saved", venue_error: null }),
    });
  });
  await page.goto("/#/lighting/venues");
  await page.getByRole("button", { name: "New Venue" }).click();
  await page.locator("#venue-name").fill("test");
  return puts;
}

const rows = (page: Page) => page.getByTestId("venue-fixture-row");
const addFixture = (page: Page) =>
  page.getByRole("button", { name: "Add Fixture" }).click();
const save = (page: Page) =>
  page.locator(".editor-form").getByRole("button", { name: "Save" }).click();

async function patchOf(page: Page, i: number) {
  const row = rows(page).nth(i);
  return {
    name: await row.getByLabel("Fixture name").inputValue(),
    type: await row.getByLabel("Fixture type").inputValue(),
    universe: Number(await row.locator(`#fix-universe-${i}`).inputValue()),
    address: Number(await row.locator(`#fix-channel-${i}`).inputValue()),
  };
}

test("two fixtures added and saved with their defaults are both saved", async ({
  page,
}) => {
  const puts = await setUp(page);
  await addFixture(page);
  await addFixture(page);
  await save(page);
  await expect.poll(() => puts.length).toBe(1);
  expect(puts[0].fixtures.map((f) => f.name)).toEqual([
    "Fixture 1",
    "Fixture 2",
  ]);
});

test("Add Fixture three times continues the patch from the last row", async ({
  page,
}) => {
  await setUp(page);
  await addFixture(page);
  expect(await patchOf(page, 0)).toEqual({
    name: "Fixture 1",
    type: "apar",
    universe: 1,
    address: 1,
  });
  // The second takes the first's type and starts after its 3 addresses.
  await addFixture(page);
  expect(await patchOf(page, 1)).toMatchObject({
    type: "apar",
    universe: 1,
    address: 4,
  });
  // Made a wash (4 addresses): the third is a wash at 4 + 4.
  await rows(page).nth(1).getByLabel("Fixture type").selectOption("wash");
  await addFixture(page);
  expect(await patchOf(page, 2)).toEqual({
    name: "Fixture 3",
    type: "wash",
    universe: 1,
    address: 8,
  });
  // Changing an earlier row renumbers nothing.
  await rows(page).nth(0).locator("#fix-channel-0").fill("100");
  expect(await patchOf(page, 2)).toMatchObject({ address: 8 });
});

test("a blanked name blocks the save, marks the row and PUTs nothing", async ({
  page,
}) => {
  const puts = await setUp(page);
  await addFixture(page);
  await addFixture(page);
  const name = rows(page).nth(1).getByLabel("Fixture name");
  await name.fill("");
  await save(page);
  await expect(page.locator(".save-msg")).toHaveText(
    "Not saved: 1 fixture needs attention.",
  );
  await expect(name).toHaveAttribute("aria-invalid", "true");
  await expect(name).toBeFocused();
  await expect(rows(page).nth(1).getByTestId("venue-row-error")).toHaveText(
    "Give this fixture a name.",
  );
  await expect(page.locator(".editor-form")).toBeVisible();
  await page.waitForTimeout(300);
  expect(puts).toHaveLength(0);

  // Typing a name clears the mark; the save then goes through with both.
  await name.fill("Back");
  await expect(rows(page).nth(1).getByTestId("venue-row-error")).toHaveCount(0);
  await save(page);
  await expect.poll(() => puts.length).toBe(1);
  expect(puts[0].fixtures).toHaveLength(2);
});

test("two rows of one name block the save", async ({ page }) => {
  const puts = await setUp(page);
  await addFixture(page);
  await addFixture(page);
  await rows(page).nth(1).getByLabel("Fixture name").fill("Fixture 1");
  await save(page);
  await expect(page.locator(".save-msg")).toHaveText(
    "Not saved: 2 fixtures need attention.",
  );
  await expect(page.getByTestId("venue-row-error")).toHaveText([
    "Another fixture has this name.",
    "Another fixture has this name.",
  ]);
  await page.waitForTimeout(300);
  expect(puts).toHaveLength(0);
});
