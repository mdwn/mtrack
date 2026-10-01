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

import { type Page } from "@playwright/test";
import { api, expect, synthGdtf, test } from "./harness";

// Stage 3D against the real binary: a venue that is not the current one is
// drawn from its file, with each fixture's own rig — never the current
// venue in its place.

const BRICK = `fixture_type "Brick" from gdtf("lighting/library/synth.gdtf") {
}
`;
const PAR = `fixture_type "Par" {
  channels: 3
  channel_map: { "red": 1, "green": 2, "blue": 3 }
}
`;

const viewport = (page: Page) => page.locator(".stage3d__viewport");
const venueLabel = (page: Page) => page.getByTestId("stage3d-venue");

test.describe("Stage 3D for a venue", () => {
  test.use({
    files: {
      "lighting/library/synth.gdtf": synthGdtf(),
      "lighting/fixture_types/brick.fixture": BRICK,
      "lighting/fixture_types/par.light": PAR,
      // The current venue (the harness config names "house").
      "lighting/venues/house.light":
        'venue "house" {\n  fixture "P" Par @ 1:1 position (0, 2, 3)\n}\n',
      // Another venue: one brick placed, two not, in two modes.
      "lighting/venues/club.light":
        'venue "club" {\n' +
        '  fixture "B1" Brick mode "8: RGBS" @ 1:1 position (-1, 2, 3)\n' +
        '  fixture "B2" Brick mode "8: RGBS" @ 1:10\n' +
        '  fixture "M" Brick mode "Mover 16bit" @ 1:20\n' +
        "}\n",
    },
  });

  test("opened from its plot, it is that venue, from its file", async ({
    page,
    project,
  }) => {
    await page.goto("/#/lighting/venues/club");
    await expect(page.getByTestId("stage-venue-label")).toContainText(
      "Venue file: club",
    );
    await page.getByTestId("stage-3d-link").click();
    await expect(page).toHaveURL(/#\/lighting\/stage\/club$/);
    await expect(venueLabel(page)).toHaveText("Venue file: club · not live");
    await expect(viewport(page)).toHaveAttribute("data-source", "file");
    await expect(viewport(page)).toHaveAttribute("data-fixtures", "3");
    await expect(viewport(page)).toHaveAttribute("data-placed", "1");
    await expect(page.getByTestId("stage3d-unplaced")).toContainText(
      "2 fixtures have no position yet",
    );
    await expect(page.getByTestId("stage3d-mode-preview")).toBeDisabled();

    // Each fixture carries its own (archive, mode) rig, and the store has it.
    const scene = await api<{
      fixtures: Record<string, { rig: string | null; mode: string | null }>;
    }>(project, "/lighting/venues/club/scene");
    expect(scene.fixtures.M.mode).toBe("Mover 16bit");
    expect(scene.fixtures.B1.rig).toBeTruthy();
    expect(scene.fixtures.M.rig).toBeTruthy();
    expect(scene.fixtures.M.rig).not.toBe(scene.fixtures.B1.rig);
    const rig = await page.request.get(
      `${project.url}/api/lighting/assets/${scene.fixtures.M.rig}`,
    );
    expect(rig.ok()).toBe(true);
    if ((await viewport(page).getAttribute("data-renderer")) === "webgl") {
      // Drawn from the rigs, not generically.
      await expect(page.locator(".stage3d__subtitle")).toContainText(
        "1 of 3 placed",
      );
      await expect(page.locator(".stage3d__subtitle")).not.toContainText(
        "drawn generically",
      );
    }

    // A reload on the address stays on that venue.
    await page.reload();
    await expect(venueLabel(page)).toHaveText("Venue file: club · not live");
    await expect(viewport(page)).toHaveAttribute("data-fixtures", "3");
  });

  test("the current venue's 3D is live", async ({ page }) => {
    await page.goto("/#/lighting/venues/house");
    await expect(page.getByTestId("stage-venue-label")).toContainText(
      "Current venue: house",
    );
    await page.getByTestId("stage-3d-link").click();
    await expect(page).toHaveURL(/#\/lighting\/stage\/house$/);
    await expect(venueLabel(page)).toHaveText("Current venue: house · live");
    await expect(viewport(page)).toHaveAttribute("data-source", "live");
    await expect(viewport(page)).toHaveAttribute("data-fixtures", "1");
  });

  test("a venue that does not exist says so", async ({ page }) => {
    await page.goto("/#/lighting/stage/nowhere");
    await expect(page.getByTestId("stage3d-missing")).toContainText(
      'There is no venue named "nowhere".',
    );
    await expect(page.locator(".stage3d__subtitle")).not.toContainText("house");
  });
});
