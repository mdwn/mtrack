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

import { test, expect } from "@playwright/test";

let testCounter = 0;

async function sendWsMessage(
  page: import("@playwright/test").Page,
  wsId: string,
  msg: object,
) {
  await page.request.post("http://127.0.0.1:3111/test/send-ws", {
    data: { ...msg, _wsId: wsId },
  });
}

/** A venue with stage geometry: one placed fixture, one not, one focus point. */
const GEOMETRY_METADATA = {
  type: "metadata",
  fixtures: {
    "front-left": {
      tags: ["front", "left"],
      type: "par",
      position: [-2, 0.5, 3],
      rotation: [0, 0, 0],
    },
    "front-right": { tags: ["front", "right"], type: "par", position: null },
  },
  venue: {
    name: "test-venue",
    dir: null,
    focus_points: { drummer: [0, 2.8, 1.4] },
  },
};

test.describe("Stage View geometry", () => {
  let wsId: string;

  test.beforeEach(async ({ page }) => {
    wsId = `geo-${test.info().parallelIndex}-${++testCounter}-${Date.now()}`;
    await page.goto(`/?wsId=${wsId}#/`);
    await expect(page.locator(".playback-card__title")).toContainText(
      "Test Song Alpha",
    );
  });

  test("a venue without geometry shows no focus-point editor", async ({
    page,
  }) => {
    await expect(page.locator(".stage-card__viewport canvas")).toBeVisible();
    await expect(page.locator(".stage-card__focus")).toHaveCount(0);
    await expect(page.locator(".stage-card__placed")).toHaveCount(0);
  });

  test("positional metadata switches the card to the stage plot", async ({
    page,
  }) => {
    await sendWsMessage(page, wsId, GEOMETRY_METADATA);
    await expect(page.locator(".stage-card--geometry")).toBeVisible();
    await expect(page.locator(".stage-card__placed")).toContainText(
      "1 of 2 placed",
    );
    const focus = page.locator(".stage-card__focus");
    await expect(focus).toBeVisible();
    await expect(focus.locator("input.stage-card__focus-name")).toHaveValue(
      "drummer",
    );
    await expect(focus.locator(".stage-card__focus-coords")).toContainText(
      "(0, 2.8, 1.4)",
    );
  });

  test("adding a focus point saves it back to the venue", async ({ page }) => {
    await sendWsMessage(page, wsId, GEOMETRY_METADATA);
    await expect(page.locator(".stage-card--geometry")).toBeVisible();
    await page.locator(".stage-card__add-focus").click();
    await expect(page.locator(".stage-card__reload")).toContainText("Saved");

    const saved = await (
      await page.request.get("http://127.0.0.1:3111/test/last-venue-put")
    ).json();
    expect(saved.name).toBe("test-venue");
    // The mock venue has no focus points of its own, so the new one is the
    // first free name; the existing fixture rides along untouched.
    expect(Object.keys(saved.body.focus_points)).toContain("focus-1");
    expect(saved.body.fixtures[0].name).toBe("front-left");
    // The editor now lists both.
    await expect(
      page.locator(".stage-card__focus input.stage-card__focus-name"),
    ).toHaveCount(2);
  });

  test("renaming a focus point saves the new name", async ({ page }) => {
    await sendWsMessage(page, wsId, GEOMETRY_METADATA);
    const input = page.locator(
      ".stage-card__focus input.stage-card__focus-name",
    );
    await expect(input).toHaveValue("drummer");
    // Seed the mock venue with the pin first, so the rename has something
    // to rename: add, then rename the added one.
    await page.locator(".stage-card__add-focus").click();
    await expect(page.locator(".stage-card__reload")).toContainText("Saved");
    // Names list sorted: "drummer", then the added "focus-1".
    const added = page
      .locator(".stage-card__focus input.stage-card__focus-name")
      .nth(1);
    await expect(added).toHaveValue("focus-1");
    await added.fill("singer");
    await added.press("Enter");
    await expect(page.locator(".stage-card__reload")).toContainText("Saved");
    const saved = await (
      await page.request.get("http://127.0.0.1:3111/test/last-venue-put")
    ).json();
    expect(Object.keys(saved.body.focus_points).sort()).toEqual([
      "drummer",
      "singer",
    ]);
  });
});
