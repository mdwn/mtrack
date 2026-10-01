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

// The venue card's 3D view against the real binary: Plot | 3D in place; a
// venue that is not the current one drawn from its file with each
// fixture's own rig; inspector edits redrawn where they are made.

const BRICK = `fixture_type "Brick" from gdtf("lighting/library/synth.gdtf") {
}
`;
const PAR = `fixture_type "Par" {
  channels: 3
  channel_map: { "red": 1, "green": 2, "blue": 3 }
}
`;

const viewport = (page: Page) => page.locator(".stage3d__viewport");
const label = (page: Page) => page.getByTestId("stage-venue-label");

async function transform(page: Page, name: string) {
  const raw = (await viewport(page).getAttribute("data-transforms")) ?? "{}";
  return (JSON.parse(raw) as Record<string, { rotation: number[] }>)[name];
}

test.describe("3D on the venue card", () => {
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

  test("3D on another venue's card is that venue, from its file, in place", async ({
    page,
    project,
  }) => {
    await page.goto("/#/lighting/venues/club");
    await expect(label(page)).toContainText("Venue file: club");
    await page.getByTestId("stage-view-3d").click();
    await expect(page).toHaveURL(/#\/lighting\/venues\/club\?view=3d$/);
    await expect(label(page)).toContainText("Venue file: club");
    await expect(viewport(page)).toHaveAttribute("data-source", "file");
    await expect(viewport(page)).toHaveAttribute("data-fixtures", "3");
    await expect(viewport(page)).toHaveAttribute("data-placed", "1");
    // The unplaced line, inside the card; Preview is the current venue's.
    await expect(
      page.locator(".stage-card").getByTestId("stage3d-unplaced"),
    ).toContainText("2 fixtures have no position yet");
    await expect(page.getByTestId("stage3d-no-preview")).toContainText(
      "Groups page",
    );
    await expect(page.getByTestId("stage3d-mode-preview")).toHaveCount(0);

    // Each fixture carries its own (archive, mode) rig, and the store has it.
    const scene = await api<{
      fixtures: Record<string, { rig: string | null; mode: string | null }>;
    }>(project, "/lighting/venues/club/scene");
    expect(scene.fixtures.M.mode).toBe("Mover 16bit");
    expect(scene.fixtures.B1.rig).toBeTruthy();
    expect(scene.fixtures.M.rig).not.toBe(scene.fixtures.B1.rig);
    const rig = await page.request.get(
      `${project.url}/api/lighting/assets/${scene.fixtures.M.rig}`,
    );
    expect(rig.ok()).toBe(true);
    if ((await viewport(page).getAttribute("data-renderer")) === "webgl") {
      // Drawn from the rigs, not generically.
      await expect(page.getByTestId("stage3d-stats")).toHaveCount(0);
    }

    // A reload keeps 3D on that venue.
    await page.reload();
    await expect(viewport(page)).toHaveAttribute("data-fixtures", "3");
    await expect(label(page)).toContainText("Venue file: club");
  });

  test("a rotation applied in the inspector is redrawn in 3D", async ({
    page,
    project,
  }) => {
    await page.goto("/#/lighting/venues/club?view=3d");
    await expect(viewport(page)).toHaveAttribute("data-transforms", /"B1"/);
    expect((await transform(page, "B1")).rotation).toEqual([0, 0, 0]);
    const camera = async () =>
      JSON.parse((await viewport(page).getAttribute("data-view")) ?? "{}")
        .camera as number[] | undefined;
    await expect.poll(camera).toBeTruthy();
    const before = (await camera())!;

    await page.locator(".inspector").getByLabel("B1", { exact: true }).check();
    await page.locator("#insp-direction").selectOption("upstage");
    await page.locator("#insp-tilt").fill("30");
    await page.getByRole("button", { name: "Face this way" }).click();

    await expect
      .poll(async () => JSON.stringify((await transform(page, "B1")).rotation))
      .not.toBe("[0,0,0]");
    // The save writes the venue's own file (a saved venue is a `.venue`).
    const file = project.exists("lighting/venues/club.venue")
      ? "lighting/venues/club.venue"
      : "lighting/venues/club.light";
    expect(project.read(file)).toMatch(/fixture "B1"[^\n]*rotation/);
    // Still 3D, still that fixture, and the view did not jump.
    await expect(page).toHaveURL(/\?view=3d$/);
    await expect(viewport(page)).toHaveAttribute("data-selected", "B1");
    const after = (await camera())!;
    after.forEach((v, i) => expect(Math.abs(v - before[i])).toBeLessThan(0.05));
  });

  test("the current venue's 3D is live, with Live | Preview", async ({
    page,
  }) => {
    await page.goto("/#/lighting/venues/house");
    await expect(label(page)).toContainText("Current venue: house");
    await page.getByTestId("stage-view-3d").click();
    await expect(page).toHaveURL(/#\/lighting\/venues\/house\?view=3d$/);
    await expect(viewport(page)).toHaveAttribute("data-source", "live");
    await expect(viewport(page)).toHaveAttribute("data-fixtures", "1");
    await page.getByTestId("stage3d-mode-preview").click();
    await expect(page).toHaveURL(/\?view=3d&mode=preview/);
    await expect(page.getByTestId("preview-panel")).toBeVisible();
  });

  test("the dashboard's 3D link and an old 3D address land on the card", async ({
    page,
  }) => {
    await page.goto("/#/");
    await expect(page.locator(".stage-card__3d")).toHaveAttribute(
      "href",
      "#/lighting/venues/house?view=3d",
    );
    await page.locator(".stage-card__3d").click();
    await expect(viewport(page)).toHaveAttribute("data-source", "live");

    await page.goto("/#/lighting/stage?mode=preview&t=4");
    await expect(page).toHaveURL(
      /#\/lighting\/venues(\/house)?\?view=3d&mode=preview&t=4$/,
    );
    await expect(viewport(page)).toHaveAttribute("data-source", /live|preview/);
  });
});
