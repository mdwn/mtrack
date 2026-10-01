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
import { PIXEL_METADATA_STATE, PIXEL_STATE } from "../mock-server/test-data";

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

test.describe("Stage cells (design §17.4)", () => {
  let wsId: string;

  test.beforeEach(async () => {
    wsId = `cells-${test.info().parallelIndex}-${++testCounter}-${Date.now()}`;
  });

  test("the dashboard's stage card draws a per-cell state without erroring", async ({
    page,
  }) => {
    const errors: string[] = [];
    page.on("pageerror", (err) => errors.push(String(err)));

    await page.goto(`/?wsId=${wsId}#/`);
    await expect(page.locator(".playback-card__title")).toContainText(
      "Test Song Alpha",
    );
    await expect(page.locator(".stage-card__viewport canvas")).toBeVisible();

    await sendWsMessage(page, wsId, PIXEL_METADATA_STATE);
    await sendWsMessage(page, wsId, PIXEL_STATE);

    await expect(page.locator(".stage-card")).toBeVisible();
    await expect(page.locator(".stage-card__title")).toContainText(
      "3 fixtures",
    );
    await expect(page.locator(".stage-card__viewport canvas")).toBeVisible();

    expect(errors, `page errors: ${errors.join("; ")}`).toEqual([]);
  });

  test("the venue card's 3D draws a per-cell state without erroring", async ({
    page,
  }) => {
    const errors: string[] = [];
    page.on("pageerror", (err) => errors.push(String(err)));

    await page.goto(`/?wsId=${wsId}#/lighting/venues/test-venue?view=3d`);
    await sendWsMessage(page, wsId, {
      ...PIXEL_METADATA_STATE,
      venue: { name: "test-venue", dir: null, focus_points: {} },
    });
    await expect(page.locator(".stage3d__viewport")).toBeVisible();
    await expect(page.locator(".stage3d__viewport")).toHaveAttribute(
      "data-renderer",
      /webgl|none/,
      { timeout: 15000 },
    );

    await sendWsMessage(page, wsId, PIXEL_STATE);

    await expect(page.locator(".stage3d__viewport")).toHaveAttribute(
      "data-source",
      "live",
    );
    await expect(page.locator(".stage3d__viewport")).toHaveAttribute(
      "data-fixtures",
      "3",
    );

    expect(errors, `page errors: ${errors.join("; ")}`).toEqual([]);
  });
});
