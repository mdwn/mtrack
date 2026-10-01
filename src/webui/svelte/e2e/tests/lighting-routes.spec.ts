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

// What a Lighting page has open is its address: the section's link, the
// browser's Back and a reload all land where they say. Read-only against the
// shared mock (nothing is saved).

function card(page: Page, name: string) {
  return page.locator(".item-card").filter({
    has: page.locator(".item-name", { hasText: new RegExp(`^${name}$`) }),
  });
}
const sectionLink = (page: Page, name: string) =>
  page.locator(".lighting__tabs").getByRole("link", { name });

test.describe("Fixture types", () => {
  test("a fixture's page has its own address; Back, the section link and a reload all work", async ({
    page,
  }) => {
    await page.goto("/#/lighting/fixtures");
    await card(page, "pixelbrick").click();
    await expect(page).toHaveURL(/#\/lighting\/fixtures\/pixelbrick$/);
    await expect(page.getByTestId("ft-title")).toHaveText("pixelbrick");

    // The browser's Back returns to the list.
    await page.goBack();
    await expect(page).toHaveURL(/#\/lighting\/fixtures$/);
    await expect(card(page, "pixelbrick")).toBeVisible();

    // Forward again, then the section's own link returns to the list.
    await page.goForward();
    await expect(page.getByTestId("ft-title")).toHaveText("pixelbrick");
    await sectionLink(page, "Fixture types").click();
    await expect(card(page, "pixelbrick")).toBeVisible();

    // A reload (or a shared link) on a fixture's address opens it.
    await page.goto("/#/lighting/fixtures/pixelbrick");
    await page.reload();
    await expect(page.getByTestId("ft-title")).toHaveText("pixelbrick");
  });

  test("a hand-written type's form and a new one are addresses too", async ({
    page,
  }) => {
    await page.goto("/#/lighting/fixtures/par");
    await expect(page.locator(".channel-row")).toHaveCount(4);
    await page.getByRole("button", { name: "Cancel" }).click();
    await expect(page).toHaveURL(/#\/lighting\/fixtures$/);

    await page.goto("/#/lighting/fixtures?new=fixture");
    await expect(page.getByTestId("ft-dsl")).toHaveValue(/fixture_type "Name"/);
    await sectionLink(page, "Fixture types").click();
    await expect(page.getByTestId("ft-dsl")).toHaveCount(0);
  });

  test("an address naming no fixture says so and shows the list", async ({
    page,
  }) => {
    await page.goto("/#/lighting/fixtures/nope");
    await expect(page.locator(".save-msg")).toHaveText(
      "There is no fixture called nope.",
    );
    await expect(card(page, "pixelbrick")).toBeVisible();
  });
});

test.describe("Venues", () => {
  test("the selected venue and its open form are addresses", async ({
    page,
  }) => {
    await page.goto("/#/lighting/venues");
    await card(page, "test-venue").click();
    await expect(page).toHaveURL(/#\/lighting\/venues\/test-venue$/);

    await card(page, "test-venue")
      .locator('[data-testid^="venue-edit-"]')
      .click();
    await expect(page).toHaveURL(/#\/lighting\/venues\/test-venue\?edit$/);
    await expect(page.locator("#venue-name")).toHaveValue("test-venue");

    // Back closes the form and keeps the selection.
    await page.goBack();
    await expect(page.locator(".editor-form")).toHaveCount(0);
    await expect(card(page, "test-venue")).toHaveAttribute(
      "aria-pressed",
      "true",
    );

    // A reload on the form's address reopens it.
    await page.goto("/#/lighting/venues/test-venue?edit");
    await page.reload();
    await expect(page.locator("#venue-name")).toHaveValue("test-venue");

    // The section link returns to the plain list.
    await sectionLink(page, "Venues").click();
    await expect(page.locator(".editor-form")).toHaveCount(0);
    await expect(page).toHaveURL(/#\/lighting\/venues$/);
  });

  test("New Venue is an address; Cancel goes back to the list", async ({
    page,
  }) => {
    await page.goto("/#/lighting/venues");
    await page.getByRole("button", { name: "New Venue" }).click();
    await expect(page).toHaveURL(/#\/lighting\/venues\?new=venue$/);
    await expect(page.locator("#venue-name")).toHaveValue("");
    await page.getByRole("button", { name: "Cancel" }).click();
    await expect(page).toHaveURL(/#\/lighting\/venues$/);
  });
});
