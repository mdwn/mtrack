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
import { STATUS } from "../mock-server/test-data";
import { uiOutdated } from "../../src/lib/buildCheck";

// An open tab after mtrack is replaced: it keeps its old bundle until it is
// reloaded, so it must say so. Each browser test answers /api/status itself
// (page.route); the shared mock is untouched. The dev server stamps no UI
// build, so in the browser the build-time rule is the one exercised; the
// UI-build rule is checked as plain values below.

/** Answers /api/status with `builds[n]` on the n-th poll (the last one
 *  repeating). */
async function statusSequence(page: Page, buildTimes: string[]) {
  let n = 0;
  await page.route("**/api/status", (route) => {
    const body = structuredClone(STATUS);
    body.build.build_time = buildTimes[Math.min(n, buildTimes.length - 1)];
    n++;
    return route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(body),
    });
  });
  return () => n;
}

test("a server replaced under the page shows the notice, with a Reload that reloads", async ({
  page,
}) => {
  const polls = await statusSequence(page, [
    "2026-01-01T00:00:00Z",
    "2026-02-02T00:00:00Z",
  ]);
  await page.goto("/#/");
  const notice = page.getByTestId("ui-updated");
  // The status poll runs every 5 seconds; the second answer is the new build.
  await expect.poll(polls, { timeout: 15000 }).toBeGreaterThan(1);
  await expect(notice).toBeVisible();
  await expect(notice).toContainText(
    "mtrack was updated. Reload to use the new version.",
  );
  // It does not reload by itself, and it stays up.
  await page.waitForTimeout(500);
  await expect(notice).toBeVisible();

  const reloaded = page.waitForEvent("load", { timeout: 30000 });
  await page.getByTestId("ui-updated-reload").click();
  await reloaded;
});

test("an unchanged server shows no notice", async ({ page }) => {
  const polls = await statusSequence(page, ["2026-01-01T00:00:00Z"]);
  await page.goto("/#/");
  await expect.poll(polls, { timeout: 15000 }).toBeGreaterThan(1);
  await expect(page.getByTestId("ui-updated")).toHaveCount(0);
});

test("the UI build is compared when both sides have one, the build time otherwise", () => {
  const at = (build_time: string, ui_build: string | null = null) => ({
    build_time,
    ui_build,
  });
  // Both stamped: the UI build decides, whatever the build time says.
  expect(uiOutdated("aaa", at("t1", "aaa"), at("t2", "aaa"))).toBe(false);
  expect(uiOutdated("aaa", at("t1", "aaa"), at("t1", "bbb"))).toBe(true);
  // This bundle unstamped (dev server): the build time decides.
  expect(uiOutdated("", at("t1", "bbb"), at("t1", "bbb"))).toBe(false);
  expect(uiOutdated("", at("t1", "bbb"), at("t2", "bbb"))).toBe(true);
  // An older server reporting no UI build: the build time decides.
  expect(uiOutdated("aaa", at("t1"), at("t1"))).toBe(false);
  expect(uiOutdated("aaa", at("t1"), at("t2"))).toBe(true);
  // No answer yet, or nothing seen at load: nothing to say.
  expect(uiOutdated("aaa", null, null)).toBe(false);
  expect(uiOutdated("", null, at("t2"))).toBe(false);
});
