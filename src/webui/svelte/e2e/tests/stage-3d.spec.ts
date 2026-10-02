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
    scenery: "scenery/test/scene-v1.json",
  },
};

// The 3D view on the venue card (Venues page, `?view=3d`).
test.describe("3D on the venue card", () => {
  let wsId: string;

  test.beforeEach(() => {
    wsId = `s3d-${test.info().parallelIndex}-${++testCounter}-${Date.now()}`;
  });

  test("the dashboard's card switches to 3D in place", async ({ page }) => {
    await page.goto(`/?wsId=${wsId}#/`);
    await expect(page.locator(".stage-card")).toBeVisible();
    // No current venue yet: nothing to show in 3D.
    await expect(page.getByTestId("stage-view-3d")).toHaveCount(0);
    await sendWsMessage(page, wsId, VENUE_METADATA);
    await page.getByTestId("stage-view-3d").click();
    await expect(page).toHaveURL(/#\/$/);
    await expect(page.getByTestId("stage-view-3d")).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    await expect(page.locator(".stage3d__viewport")).toHaveAttribute(
      "data-source",
      "live",
    );
    // A live view: no show preview, and nothing to select.
    await expect(page.getByTestId("stage3d-mode-preview")).toHaveCount(0);
    await expect(page.getByTestId("stage3d-hint")).toHaveText("Drag to orbit");

    await page.getByTestId("stage-view-plot").click();
    await expect(page).toHaveURL(/#\/$/);
    await expect(page.locator(".stage3d__viewport")).toHaveCount(0);
    await expect(page.locator(".stage-card__viewport > canvas")).toBeVisible();
  });

  test("lit, placed fixtures light the deck, whichever way they point", async ({
    page,
  }) => {
    await page.goto(`/?wsId=${wsId}#/lighting/venues/test-venue?view=3d`);
    // A PAR hung pointing down, one on the floor aimed up at the band with
    // a diffuser's beam angle, and one waiting in the tray.
    await sendWsMessage(page, wsId, {
      type: "metadata",
      fixtures: {
        top: { tags: [], type: "par", position: [0, 3, 4], rig: null },
        floor: {
          tags: [],
          type: "par",
          position: [0, 0.5, 0.1],
          rotation: [120, 0, 0],
          rig: null,
          beam_angle: 90,
        },
        spare: { tags: [], type: "par", position: null, rig: null },
      },
      venue: { name: "test-venue", dir: null, focus_points: {} },
    });
    const viewport = page.locator(".stage3d__viewport");
    await expect(viewport).toHaveAttribute("data-renderer", /webgl|none/, {
      timeout: 15000,
    });
    test.skip(
      (await viewport.getAttribute("data-renderer")) !== "webgl",
      "no WebGL in this browser",
    );
    // Nothing reported yet: every fixture is dark, and so is the deck.
    await expect(viewport).toHaveAttribute("data-deck-lights", "0");
    const on = { red: 255, green: 0, blue: 160, dimmer: 255, strobe: 0 };
    await sendWsMessage(page, wsId, {
      type: "state",
      fixtures: { top: on, floor: on, spare: on },
    });
    // The two on the stage; not the one in the tray.
    await expect(viewport).toHaveAttribute("data-deck-lights", "2");
    await sendWsMessage(page, wsId, {
      type: "state",
      fixtures: { top: on, floor: { ...on, dimmer: 0 }, spare: on },
    });
    await expect(viewport).toHaveAttribute("data-deck-lights", "1");
  });

  test("the page draws the venue from rigs and reports what it drew", async ({
    page,
  }) => {
    const rigRequests: string[] = [];
    page.on("request", (r) => {
      if (r.url().includes("/api/lighting/assets/")) rigRequests.push(r.url());
    });
    // Asset failures surface only in the console; keep them in the report.
    page.on("console", (m) => {
      if (m.type() === "error" || m.type() === "warning") {
        console.log(`[browser ${m.type()}] ${m.text()}`);
      }
    });
    page.on("response", (r) => {
      if (r.url().includes("/api/lighting/assets/") && r.status() !== 200) {
        console.log(`[asset ${r.status()}] ${r.url()}`);
      }
    });
    await page.goto(`/?wsId=${wsId}#/lighting/venues/test-venue?view=3d`);
    await sendWsMessage(page, wsId, VENUE_METADATA);
    await expect(page.locator(".stage3d__viewport")).toBeVisible();
    // The renderer decides: WebGL, or the fallback message — never a blank.
    await expect(page.locator(".stage3d__viewport")).toHaveAttribute(
      "data-renderer",
      /webgl|none/,
      { timeout: 15000 },
    );

    const renderer = await page
      .locator(".stage3d__viewport")
      .getAttribute("data-renderer");
    await expect(page.getByTestId("stage-venue-label")).toContainText(
      "Current venue: test-venue",
    );
    await expect(page.locator(".stage3d__viewport")).toHaveAttribute(
      "data-fixtures",
      "3",
    );
    if (renderer === "webgl") {
      // The mover's rig was fetched from the store; the PARs draw generically.
      await expect
        .poll(() => rigRequests.length, { timeout: 10000 })
        .toBeGreaterThan(0);
      expect(rigRequests[0]).toContain("test-archive/rig-test-v1.json");
      await expect(page.locator(".stage3d__viewport")).toHaveAttribute(
        "data-placed",
        "2",
      );
      await expect(page.getByTestId("stage3d-stats")).toContainText(
        "2 drawn generically",
      );
      // The scenery: the deck's glb drawn, the truss's .3ds reported.
      await expect(page.getByTestId("stage3d-stats")).toContainText(
        "1 scenery mesh (1 not drawn: .3ds)",
        { timeout: 10000 },
      );
      expect(
        rigRequests.some((u) => u.endsWith("scenery/test/models/box.glb")),
      ).toBe(true);
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
  });
});
