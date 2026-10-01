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

// The venue form against the real binary: a save shows on the page at once,
// and a row chooses its own mode.

const BRICK = `fixture_type "Brick" from gdtf("lighting/library/synth.gdtf") {
}
`;
const PAR = `fixture_type "Par" {
  channels: 3
  channel_map: { "red": 1, "green": 2, "blue": 3 }
}
`;

const card = (page: Page, name: string) =>
  page.locator(".item-card").filter({
    has: page.locator(".item-name", { hasText: new RegExp(`^${name}$`) }),
  });
const rows = (page: Page) => page.getByTestId("venue-fixture-row");
const save = (page: Page) =>
  page.locator(".editor-form").getByRole("button", { name: "Save" }).click();
const plotTitle = (page: Page) => page.locator(".stage-card__title");

test.describe("A saved venue", () => {
  test.use({
    files: {
      "lighting/fixture_types/par.light": PAR,
      "lighting/venues/house.light": 'venue "house" {\n}\n',
      "lighting/venues/club.light":
        'venue "club" {\n  fixture "A" Par @ 1:1\n  fixture "B" Par @ 1:4\n}\n',
      "lighting/venues/test.light": 'venue "test" {\n}\n',
    },
  });

  test("shows its new fixtures on the plot at once, without navigating away", async ({
    page,
    project,
  }) => {
    await page.goto("/#/lighting/venues");
    await card(page, "club").click();
    await expect(plotTitle(page)).toContainText("club");
    await expect(plotTitle(page)).toContainText("2 fixtures");
    await card(page, "club").locator('[data-testid^="venue-edit-"]').click();
    await page.getByRole("button", { name: "Add Fixture" }).click();
    await save(page);
    await expect(page.locator(".editor-form")).toHaveCount(0);
    await expect(plotTitle(page)).toContainText("3 fixtures");
    expect(project.read("lighting/venues/club.light")).toContain(
      'fixture "Fixture 1" Par @ 1:7',
    );
  });

  test("an empty venue gaining fixtures shows them at once", async ({
    page,
  }) => {
    await page.goto("/#/lighting/venues");
    await card(page, "test").click();
    await expect(plotTitle(page)).toContainText("0 fixtures");
    await card(page, "test").locator('[data-testid^="venue-edit-"]').click();
    await page.getByRole("button", { name: "Add Fixture" }).click();
    await page.getByRole("button", { name: "Add Fixture" }).click();
    await save(page);
    await expect(page.locator(".editor-form")).toHaveCount(0);
    await expect(plotTitle(page)).toContainText("2 fixtures");
  });
});

test.describe("A mode per fixture in the form", () => {
  test.use({
    files: {
      "lighting/library/synth.gdtf": synthGdtf(),
      "lighting/fixture_types/brick.fixture": BRICK,
      "lighting/venues/house.light": 'venue "house" {\n}\n',
    },
  });

  test("fixtures in two modes save each with its own mode and reopen the same", async ({
    page,
    project,
  }) => {
    await page.goto("/#/lighting/venues");
    await page.getByRole("button", { name: "New Venue" }).click();
    await page.locator("#venue-name").fill("mix");
    for (let i = 0; i < 3; i++)
      await page.getByRole("button", { name: "Add Fixture" }).click();
    const mode = (i: number) => rows(page).nth(i).getByTestId("venue-row-mode");
    await expect(mode(1)).toBeVisible();
    await mode(1).selectOption("Mover 16bit");
    // Row 3 was added before row 2 changed mode: nothing is renumbered.
    await expect(rows(page).nth(2).locator("#fix-channel-2")).toHaveValue("9");
    // Row 3 now runs into row 2: marked, not moved.
    await expect(
      rows(page).nth(2).getByTestId("venue-row-overlap"),
    ).toContainText("Runs into Fixture 2");
    await rows(page).nth(2).locator("#fix-channel-2").fill("10");
    await expect(
      rows(page).nth(2).getByTestId("venue-row-overlap"),
    ).toHaveCount(0);
    // A new row continues after row 3 (4 addresses), in row 3's mode.
    await page.getByRole("button", { name: "Add Fixture" }).click();
    await expect(rows(page).nth(3).locator("#fix-channel-3")).toHaveValue("14");
    await save(page);
    await expect(page.locator(".editor-form")).toHaveCount(0);

    const file = project.read("lighting/venues/mix.venue");
    const lines = file.split("\n").filter((l) => l.includes("fixture "));
    // Every line names its mode; only the second is in another.
    expect(lines).toEqual([
      '  fixture "Fixture 1" Brick mode "8: RGBS" @ 1:1',
      '  fixture "Fixture 2" Brick mode "Mover 16bit" @ 1:5',
      '  fixture "Fixture 3" Brick mode "8: RGBS" @ 1:10',
      '  fixture "Fixture 4" Brick mode "8: RGBS" @ 1:14',
    ]);
    const got = await api<{
      venue: { fixtures: Record<string, { mode?: string }> };
    }>(project, "/lighting/venues/mix");
    expect(got.venue.fixtures["Fixture 2"].mode).toBe("Mover 16bit");

    await page.reload();
    await card(page, "mix").locator('[data-testid^="venue-edit-"]').click();
    await expect(mode(1)).toHaveValue("Mover 16bit");
    await expect(mode(0)).toHaveValue("8: RGBS");
    await expect(rows(page).nth(1).getByTestId("venue-row-span")).toHaveText(
      "Addresses 5–9",
    );
  });
});
