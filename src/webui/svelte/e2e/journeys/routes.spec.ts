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
import { expect, synthGdtf, test } from "./harness";

// A fixture's page and a venue's form are addresses, against the real
// server: Back, the section's link and a reload land where they say.

const card = (page: Page, name: string) =>
  page.locator(".item-card").filter({
    has: page.locator(".item-name", { hasText: new RegExp(`^${name}$`) }),
  });
const sectionLink = (page: Page, name: string) =>
  page.locator(".lighting__tabs").getByRole("link", { name });

test.use({
  files: {
    "lighting/library/synth.gdtf": synthGdtf(),
    "lighting/venues/house.venue":
      'venue "house" {\n  fixture "B1" "Synth Brick" mode "8: RGBS" @ 1:1\n}\n',
  },
});

test("a fixture's page: Back and the section link return to the list; a reload stays", async ({
  page,
}) => {
  await page.goto("/#/lighting/fixtures");
  await card(page, "Synth Brick").click();
  await expect(page).toHaveURL(/#\/lighting\/fixtures\/Synth%20Brick$/);
  await expect(page.getByTestId("ft-title")).toHaveText("Synth Brick");

  await page.reload();
  await expect(page.getByTestId("ft-title")).toHaveText("Synth Brick");

  await page.goBack();
  await expect(card(page, "Synth Brick")).toBeVisible();

  await page.goForward();
  await expect(page.getByTestId("ft-title")).toHaveText("Synth Brick");
  await sectionLink(page, "Fixture types").click();
  await expect(card(page, "Synth Brick")).toBeVisible();
});

test("a venue's form is an address: a reload reopens it, the section link closes it", async ({
  page,
}) => {
  await page.goto("/#/lighting/venues");
  await card(page, "house").locator('[data-testid^="venue-edit-"]').click();
  await expect(page.locator("#venue-name")).toHaveValue("house");
  await page.reload();
  await expect(page.locator("#venue-name")).toHaveValue("house");
  await sectionLink(page, "Venues").click();
  await expect(page.locator(".editor-form")).toHaveCount(0);
});
