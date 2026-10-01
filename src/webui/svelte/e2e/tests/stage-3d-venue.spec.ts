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

// Stage 3D shows the venue its address names: the current venue live, any
// other from its file — never the current one in its place.

let counter = 0;

/** The engine's current venue: "built-in", with one placed PAR. */
const BUILT_IN = {
  type: "metadata",
  fixtures: {
    "house-par": {
      tags: [],
      type: "par",
      position: [0, 2, 4],
      rotation: [0, 0, 0],
      rig: null,
    },
  },
  venue: { name: "built-in", dir: null, focus_points: {} },
};

/** "club" as its file reads: one fixture placed, two not. */
const CLUB_SCENE = {
  fixtures: {
    A: {
      tags: [],
      type: "par",
      mode: null,
      position: [-1, 2, 3],
      rotation: [0, 0, 0],
      rig: null,
    },
    B: {
      tags: [],
      type: "par",
      mode: null,
      position: null,
      rotation: null,
      rig: null,
    },
    C: {
      tags: [],
      type: "par",
      mode: null,
      position: null,
      rotation: null,
      rig: null,
    },
  },
  venue: {
    name: "club",
    focus_points: { dj: [0, 3, 1] },
    scenery: null,
    scenery_error: null,
  },
};

async function withCurrentVenue(page: Page, path: string) {
  const wsId = `s3dv-${test.info().parallelIndex}-${++counter}-${Date.now()}`;
  let sceneReads = 0;
  await page.route(/\/api\/lighting\/venues\/club\/scene(\?.*)?$/, (r) => {
    sceneReads++;
    return r.fulfill({ json: CLUB_SCENE });
  });
  await page.goto(`/?wsId=${wsId}${path}`);
  // The page is up once the store has the current venue.
  await page.request.post("http://127.0.0.1:3111/test/send-ws", {
    data: { ...BUILT_IN, _wsId: wsId },
  });
  return { wsId, reads: () => sceneReads };
}

const viewport = (page: Page) => page.locator(".stage3d__viewport");
const venueLabel = (page: Page) => page.getByTestId("stage3d-venue");

test("another venue's 3D, from its plot, shows that venue from its file", async ({
  page,
}) => {
  await withCurrentVenue(page, "#/lighting/venues/club");
  await expect(page.getByTestId("stage-venue-label")).toContainText(
    "Venue file: club",
  );
  await page.getByTestId("stage-3d-link").click();
  await expect(page).toHaveURL(/#\/lighting\/stage\/club$/);

  await expect(venueLabel(page)).toHaveText("Venue file: club · not live");
  await expect(viewport(page)).toHaveAttribute("data-venue", "club");
  await expect(viewport(page)).toHaveAttribute("data-source", "file");
  await expect(viewport(page)).toHaveAttribute("data-fixtures", "3");
  await expect(viewport(page)).toHaveAttribute("data-placed", "1");
  await expect(page.getByTestId("stage3d-mode")).toHaveText("Not live");

  // The unplaced fixtures are said, with the way to place them.
  const unplaced = page.getByTestId("stage3d-unplaced");
  await expect(unplaced).toContainText(
    "2 fixtures have no position yet — drawn in a row in front of the stage.",
  );
  await expect(
    unplaced.getByRole("link", { name: "Place them on the venue's plot." }),
  ).toHaveAttribute("href", "#/lighting/venues/club");

  // Preview evaluates against the engine's venue: off, with why.
  await expect(page.getByTestId("stage3d-mode-preview")).toBeDisabled();
  await expect(page.getByTestId("stage3d-no-preview")).toContainText(
    "Groups page",
  );
  await expect(page.getByTestId("stage3d-grid")).toHaveText("Grid: 1 m");

  // A reload on the address stays on that venue.
  await page.reload();
  await expect(venueLabel(page)).toHaveText("Venue file: club · not live");
  await expect(viewport(page)).toHaveAttribute("data-fixtures", "3");
});

test("a saved venue or fixture type re-reads the file view", async ({
  page,
}) => {
  const { reads } = await withCurrentVenue(page, "#/lighting/stage/club");
  await expect(viewport(page)).toHaveAttribute("data-fixtures", "3");
  const before = reads();
  await page.evaluate(async () => {
    const m = await import(/* @vite-ignore */ "/src/lib/lighting/changes.ts");
    m.venueChanged("club");
  });
  await expect.poll(reads).toBe(before + 1);
  await page.evaluate(async () => {
    const m = await import(/* @vite-ignore */ "/src/lib/lighting/changes.ts");
    m.venueChanged("elsewhere");
    m.fixtureTypesChanged();
  });
  await expect.poll(reads).toBe(before + 2);
});

test("the current venue's 3D is live, from its plot or with no venue named", async ({
  page,
}) => {
  const { reads } = await withCurrentVenue(page, "#/lighting/venues/built-in");
  await expect(page.getByTestId("stage-venue-label")).toContainText(
    "Current venue: built-in",
  );
  await page.getByTestId("stage-3d-link").click();
  await expect(page).toHaveURL(/#\/lighting\/stage\/built-in$/);
  await expect(venueLabel(page)).toHaveText("Current venue: built-in · live");
  await expect(viewport(page)).toHaveAttribute("data-source", "live");
  await expect(viewport(page)).toHaveAttribute("data-fixtures", "1");
  await expect(page.getByTestId("stage3d-unplaced")).toHaveCount(0);
  await expect(page.getByTestId("stage3d-mode-preview")).toBeEnabled();

  await page
    .locator(".lighting__tabs")
    .getByRole("link", { name: "3D", exact: true })
    .click();
  await expect(page).toHaveURL(/#\/lighting\/stage$/);
  await expect(venueLabel(page)).toHaveText("Current venue: built-in · live");
  expect(reads()).toBe(0);
});

test("a venue that does not exist says so, and is not the current one", async ({
  page,
}) => {
  await withCurrentVenue(page, "#/lighting/stage/nowhere");
  await expect(page.getByTestId("stage3d-missing")).toContainText(
    'There is no venue named "nowhere".',
  );
  await expect(venueLabel(page)).toHaveText("Venue file: nowhere · not live");
  await expect(viewport(page)).toHaveAttribute("data-fixtures", "0");
  await expect(page.locator(".stage3d__subtitle")).not.toContainText(
    "built-in",
  );
});
