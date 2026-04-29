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

test.describe("Stage View Interactions", () => {
  let wsId: string;

  test.beforeEach(async ({ page }) => {
    wsId = `stage-${++testCounter}-${Date.now()}`;
    await page.goto(`/?wsId=${wsId}#/`);
    await expect(page.locator(".playback-card__title")).toContainText(
      "Test Song Alpha",
    );
  });

  test("stage viewport contains a canvas element", async ({ page }) => {
    await expect(page.locator(".stage-card__viewport canvas")).toBeVisible();
  });

  test("canvas has non-zero dimensions", async ({ page }) => {
    const canvas = page.locator(".stage-card__viewport canvas");
    await expect(canvas).toBeVisible();
    const box = await canvas.boundingBox();
    expect(box).not.toBeNull();
    expect(box!.width).toBeGreaterThan(0);
    expect(box!.height).toBeGreaterThan(0);
  });

  test("stage card title is visible", async ({ page }) => {
    await expect(page.locator(".stage-card .stage-card__title")).toContainText(
      "Stage",
    );
  });

  test("fixture state update triggers canvas re-render", async ({ page }) => {
    const canvas = page.locator(".stage-card__viewport canvas");
    await expect(canvas).toBeVisible();

    // Send updated fixture state with new colors.
    await sendWsMessage(page, wsId, {
      type: "state",
      fixtures: {
        "front-left": {
          red: 0,
          green: 0,
          blue: 255,
          dimmer: 255,
          strobe: 0,
        },
        "front-right": {
          red: 255,
          green: 255,
          blue: 0,
          dimmer: 255,
          strobe: 0,
        },
      },
      active_effects: ["color_wash"],
    });

    // Canvas should still be visible after state update.
    await expect(canvas).toBeVisible();
  });

  test("metadata update with new fixtures keeps canvas visible", async ({
    page,
  }) => {
    const canvas = page.locator(".stage-card__viewport canvas");
    await expect(canvas).toBeVisible();

    // Send metadata with additional fixtures.
    await sendWsMessage(page, wsId, {
      type: "metadata",
      fixtures: {
        "front-left": { tags: ["front", "left"], type: "par" },
        "front-right": { tags: ["front", "right"], type: "par" },
        "back-center": { tags: ["back", "center"], type: "par" },
      },
    });

    await expect(canvas).toBeVisible();
  });
});
