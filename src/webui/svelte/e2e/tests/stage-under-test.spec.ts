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

// A fixture test drives a venue fixture: the state stream carries the
// test's bytes for it and names it in `under_test`. The plot draws that
// colour with a "test" badge, 3D tints its label, and the stage card says a
// test is live, with Stop.

let counter = 0;

const METADATA = {
  type: "metadata",
  fixtures: {
    "house-par": {
      tags: [],
      type: "par",
      position: [-2, 2, 4],
      rotation: [0, 0, 0],
      rig: null,
    },
    "back-par": {
      tags: [],
      type: "par",
      position: [2, 4, 4],
      rotation: [0, 0, 0],
      rig: null,
    },
  },
  venue: { name: "built-in", dir: null, focus_points: {} },
};

const TESTED = {
  type: "state",
  fixtures: {
    "house-par": { dimmer: 255, red: 255, green: 0, blue: 0 },
    "back-par": { dimmer: 255, red: 0, green: 0, blue: 255 },
  },
  active_effects: [],
  poses: {},
  cells: {},
  under_test: ["house-par"],
};

const RELEASED = {
  ...TESTED,
  fixtures: {
    "house-par": { dimmer: 255, red: 0, green: 0, blue: 255 },
    "back-par": { dimmer: 255, red: 0, green: 0, blue: 255 },
  },
  under_test: [],
};

const plot = (page: Page) => page.locator(".stage-card__viewport > canvas");

async function send(page: Page, wsId: string, msg: object) {
  await page.request.post("http://127.0.0.1:3111/test/send-ws", {
    data: { ...msg, _wsId: wsId },
  });
}

/** The plot's pixel at a fixture's centre. */
async function pixelAt(page: Page, name: string): Promise<number[]> {
  return plot(page).evaluate((canvas: HTMLCanvasElement, fixture) => {
    const at = JSON.parse(canvas.dataset.positions ?? "{}")[fixture];
    const k = canvas.width / canvas.clientWidth;
    const ctx = canvas.getContext("2d")!;
    return Array.from(
      ctx.getImageData(Math.round(at.x * k), Math.round(at.y * k), 1, 1).data,
    ).slice(0, 3);
  }, name);
}

test("the venue's stage shows what a fixture test sends, and says so", async ({
  page,
}) => {
  const wsId = `under-test-${test.info().parallelIndex}-${++counter}-${Date.now()}`;
  let live = true;
  const deletes: string[] = [];
  await page.route("**/api/status", async (route) => {
    const res = await route.fetch();
    const body = await res.json();
    body.hardware.test_output = live
      ? {
          universe: 1,
          address: 1,
          footprint: 4,
          fixture_type: "par",
          mode: null,
          expires_in_secs: 4,
        }
      : null;
    await route.fulfill({ response: res, json: body });
  });
  await page.route(/\/api\/lighting\/fixture-test(\?.*)?$/, async (route) => {
    if (route.request().method() === "DELETE") {
      deletes.push("DELETE");
      live = false;
    }
    await route.fallback();
  });

  await page.goto(`/?wsId=${wsId}#/lighting/venues/built-in`);
  await send(page, wsId, METADATA);
  await expect(plot(page)).toHaveAttribute("data-positions", /house-par/);
  await send(page, wsId, TESTED);

  // The plot: the test's colour on the fixture it covers, and the badge.
  await expect(plot(page)).toHaveAttribute("data-under-test", "house-par");
  await expect.poll(() => pixelAt(page, "house-par")).toEqual([255, 0, 0]);
  await expect.poll(() => pixelAt(page, "back-par")).toEqual([0, 0, 255]);

  // The card says why.
  const line = page.getByTestId("stage-test-output");
  await expect(line).toContainText(
    "Test output is live on universe 1, addresses 1–4 (par).",
  );
  await expect(line.getByRole("link", { name: "Open" })).toHaveAttribute(
    "href",
    "#/lighting/fixtures/par",
  );

  // 3D: the same fixture marked.
  await page.getByTestId("stage-view-3d").click();
  await expect(page.locator(".stage3d__viewport")).toHaveAttribute(
    "data-under-test",
    "house-par",
  );
  await page.getByTestId("stage-view-plot").click();

  // Stop from the card: released, and the next state draws the show.
  await page.getByTestId("stage-test-output-stop").click();
  await expect.poll(() => deletes.length).toBe(1);
  await expect(line).toHaveCount(0);
  await send(page, wsId, RELEASED);
  await expect(plot(page)).toHaveAttribute("data-under-test", "");
  await expect.poll(() => pixelAt(page, "house-par")).toEqual([0, 0, 255]);
});
