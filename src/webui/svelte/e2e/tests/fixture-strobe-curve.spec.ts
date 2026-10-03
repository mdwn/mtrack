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

// The strobe curve on the fixture page's settings: shown only for a fixture
// whose GDTF has a strobe rate, automatic (as the GDTF declares) until set,
// with the table's own option only when the file has a table.

const SETTINGS = /\/api\/lighting\/fixture-types\/pixelbrick\/settings(\?|$)/;

/** Answers the settings GET with `strobe` (and a stated curve). */
async function settingsWith(
  page: Page,
  strobe: { steps: number; automatic: string } | null,
  curve: string | null = null,
) {
  await page.route(SETTINGS, async (route) => {
    if (route.request().method() !== "GET") return route.fallback();
    await route.fulfill({
      json: {
        name: "pixelbrick",
        movement: { max_pan_speed: null, max_tilt_speed: null },
        strobe_curve: curve,
        strobe,
        version: "mock-v1",
      },
    });
  });
}

async function openPixelbrick(page: Page) {
  await page.goto("/#/lighting/fixtures");
  await page
    .locator(".item-card")
    .filter({ has: page.locator(".item-name", { hasText: /^pixelbrick$/ }) })
    .click();
  await expect(page.getByTestId("ft-set-name")).toHaveValue("pixelbrick");
}

const select = (page: Page) => page.getByTestId("ft-set-strobe");
const options = (page: Page) =>
  select(page)
    .locator("option")
    .evaluateAll((os) => os.map((o) => (o as HTMLOptionElement).text));

test("an endpoints-only strobe: automatic is linear in Hz, period can be set and saved", async ({
  page,
}) => {
  const posts: Record<string, unknown>[] = [];
  page.on("request", (req) => {
    if (req.method() === "POST" && SETTINGS.test(req.url()))
      posts.push(req.postDataJSON());
  });
  await openPixelbrick(page);
  await expect(select(page)).toHaveValue("");
  expect(await options(page)).toEqual([
    "Automatic: as the GDTF declares (linear in Hz)",
    "Period (by flash length, as the Astera PixelBrick runs)",
    "Linear in Hz",
  ]);
  await expect(page.getByTestId("ft-set-strobe-hint")).toContainText(
    "If a 2 Hz strobe does not flash twice a second when you test the fixture, try another curve.",
  );

  await select(page).selectOption("period");
  await page.getByTestId("ft-set-save").click();
  await expect(page.getByTestId("ft-set-msg")).toContainText("Saved");
  expect(posts.map((p) => [p.write, p.strobe_curve])).toEqual([
    [false, "period"],
    [true, "period"],
  ]);
});

test("a declared table is offered with its steps", async ({ page }) => {
  await settingsWith(page, { steps: 12, automatic: "declared" }, "linear");
  await openPixelbrick(page);
  await expect(select(page)).toHaveValue("linear");
  expect(await options(page)).toEqual([
    "Automatic: as the GDTF declares (12 steps)",
    "Period (by flash length, as the Astera PixelBrick runs)",
    "Linear in Hz",
    "As the GDTF declares (12 steps)",
  ]);
  // Back to automatic is a change; discarding returns to what is saved.
  await select(page).selectOption("");
  await expect(page.getByTestId("ft-set-save")).toBeEnabled();
  await page.getByTestId("ft-set-discard").click();
  await expect(select(page)).toHaveValue("linear");
});

test("a fixture with no strobe rate has no strobe curve", async ({ page }) => {
  await settingsWith(page, null);
  await openPixelbrick(page);
  await expect(page.getByTestId("ft-settings")).toBeVisible();
  await expect(select(page)).toHaveCount(0);
  await expect(page.getByTestId("ft-set-strobe-hint")).toHaveCount(0);
});
