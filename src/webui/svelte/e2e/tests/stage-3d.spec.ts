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

/** A venue with one placed mover carrying a rig, one placed PAR without
 *  one, one unplaced fixture, and a focus point. */
const VENUE_METADATA = {
  type: "metadata",
  fixtures: {
    "mover-1": {
      tags: ["rear"],
      type: "Test Mover",
      position: [0, 3.5, 4.2],
      rotation: [0, 0, 180],
      rig: "test-archive/rig-test-v1.json",
    },
    "front-left": {
      tags: ["front", "left"],
      type: "par",
      position: [-2, 0.5, 3],
      rotation: [0, 0, 0],
      rig: null,
    },
    spare: { tags: [], type: "par", position: null, rig: null },
  },
  venue: {
    name: "test-venue",
    dir: null,
    focus_points: { drummer: [0, 2.8, 1.4] },
  },
};

test.describe("Stage 3D", () => {
  let wsId: string;

  test.beforeEach(() => {
    wsId = `s3d-${test.info().parallelIndex}-${++testCounter}-${Date.now()}`;
  });

  test("the dashboard's stage card opens the 3D page", async ({ page }) => {
    await page.goto(`/?wsId=${wsId}#/`);
    await expect(page.locator(".stage-card")).toBeVisible();
    await page.locator(".stage-card__3d").click();
    await expect(page).toHaveURL(/#\/stage$/);
    await expect(page.locator(".stage3d .page__title")).toHaveText("Stage 3D");
  });

  test("the page draws the venue from rigs and reports what it drew", async ({
    page,
  }) => {
    const rigRequests: string[] = [];
    page.on("request", (r) => {
      if (r.url().includes("/api/lighting/assets/")) rigRequests.push(r.url());
    });
    await page.goto(`/?wsId=${wsId}#/stage`);
    await expect(page.locator(".stage3d__viewport")).toBeVisible();
    // The renderer decides: WebGL, or the fallback message — never a blank.
    await expect(page.locator(".stage3d__viewport")).toHaveAttribute(
      "data-renderer",
      /webgl|none/,
      { timeout: 15000 },
    );
    await sendWsMessage(page, wsId, VENUE_METADATA);

    const renderer = await page
      .locator(".stage3d__viewport")
      .getAttribute("data-renderer");
    await expect(page.locator(".stage3d__subtitle")).toContainText(
      "test-venue",
    );
    await expect(page.locator(".stage3d__subtitle")).toContainText(
      "3 fixtures",
    );
    if (renderer === "webgl") {
      // The mover's rig was fetched from the store; the PARs draw generically.
      await expect
        .poll(() => rigRequests.length, { timeout: 10000 })
        .toBeGreaterThan(0);
      expect(rigRequests[0]).toContain("test-archive/rig-test-v1.json");
      await expect(page.locator(".stage3d__subtitle")).toContainText(
        "2 of 3 placed",
      );
      await expect(page.locator(".stage3d__subtitle")).toContainText(
        "2 drawn generically",
      );
      // Frames keep coming: the scene is live, not a still.
      const before = Number(
        await page.locator(".stage3d__viewport").getAttribute("data-frames"),
      );
      await page.waitForTimeout(400);
      await expect
        .poll(
          async () =>
            Number(
              await page
                .locator(".stage3d__viewport")
                .getAttribute("data-frames"),
            ),
          { timeout: 5000 },
        )
        .toBeGreaterThan(before);
    } else {
      await expect(page.locator(".stage3d__fallback")).toContainText("WebGL");
    }

    // Camera presets are buttons that take the active state.
    await page.getByRole("button", { name: "Top" }).click();
    await expect(page.getByRole("button", { name: "Top" })).toHaveClass(
      /stage3d__preset--active/,
    );
  });
});
