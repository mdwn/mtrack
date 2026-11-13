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
import { PLAYBACK_STATE, STATUS } from "../mock-server/test-data";
import { sendWatch, setWatchOpening, type WatchPush } from "../support/watch";
import { uiOutdated } from "../../src/lib/buildCheck";

// An open tab after mtrack is replaced: it keeps its old bundle until it is
// reloaded, so it must say so. Each browser test says the player's status
// itself, on its page's Watch stream (setWatchOpening, sendWatch); the shared
// mock is untouched. The dev server stamps no UI build, so in the browser the
// build-time rule is the one exercised; the UI-build rule is checked as plain
// values below.

let counter = 0;

/** The mock's status, built at `buildTime`, as the player's `hardware`
 *  event. */
function statusBuilt(buildTime: string): WatchPush {
  const body = structuredClone(STATUS);
  body.build.build_time = buildTime;
  return { type: "hardware", ...body };
}

/** Opens the app on a player built at `buildTime`; the page's id. */
async function openBuilt(page: Page, buildTime: string): Promise<string> {
  const id = `stale-${test.info().parallelIndex}-${++counter}-${Date.now()}`;
  await setWatchOpening(page, id, statusBuilt(buildTime));
  await page.goto(`/?wsId=${id}#/`);
  await expect(page.locator(".playback-card__title")).toContainText(
    "Test Song Alpha",
  );
  return id;
}

test("a server replaced under the page shows the notice, with a Reload that reloads", async ({
  page,
}) => {
  const id = await openBuilt(page, "2026-01-01T00:00:00Z");
  const notice = page.getByTestId("ui-updated");
  await expect(notice).toHaveCount(0);
  // The player is replaced: its stream says the new build.
  await sendWatch(page, id, statusBuilt("2026-02-02T00:00:00Z"));
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
  const id = await openBuilt(page, "2026-01-01T00:00:00Z");
  await sendWatch(page, id, statusBuilt("2026-01-01T00:00:00Z"));
  // The stream is in order: once the playback after the status shows, the
  // status has been read.
  await sendWatch(page, id, { ...PLAYBACK_STATE, song_name: "Test Song Beta" });
  await expect(page.locator(".playback-card__title")).toContainText(
    "Test Song Beta",
  );
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
