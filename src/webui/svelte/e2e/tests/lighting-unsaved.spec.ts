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

// Leaving a Lighting form with unsaved edits asks first, through the app's
// guard: staying keeps the edits and the address, leaving discards them, a
// clean form leaves without asking, and a save clears the question. Saves
// are caught with page.route, so the shared mock is untouched.

function card(page: Page, name: string) {
  return page.locator(".item-card").filter({
    has: page.locator(".item-name", { hasText: new RegExp(`^${name}$`) }),
  });
}
const sectionLink = (page: Page, name: string) =>
  page.locator(".lighting__tabs").getByRole("link", { name });
const dialog = (page: Page) => page.locator(".dialog-overlay");

test.describe("A fixture's settings", () => {
  test("unsaved: asks; Stay keeps the edit and the address; Leave discards", async ({
    page,
  }) => {
    await page.goto("/#/lighting/fixtures/pixelbrick");
    const name = page.getByTestId("ft-set-name");
    await expect(name).toHaveValue("pixelbrick");
    await name.fill("PB15");

    await sectionLink(page, "Fixture types").click();
    await expect(dialog(page)).toContainText(
      "Discard your unsaved changes to pixelbrick?",
    );
    await dialog(page).getByRole("button", { name: "Cancel" }).click();
    await expect(page).toHaveURL(/#\/lighting\/fixtures\/pixelbrick$/);
    await expect(name).toHaveValue("PB15");

    // Back on the page itself asks the same.
    await page.getByRole("button", { name: "Back" }).click();
    await dialog(page).getByRole("button", { name: "Confirm" }).click();
    await expect(page).toHaveURL(/#\/lighting\/fixtures$/);
    await expect(card(page, "pixelbrick")).toBeVisible();
  });

  test("clean: the section link leaves without asking", async ({ page }) => {
    await page.goto("/#/lighting/fixtures/pixelbrick");
    await expect(page.getByTestId("ft-set-name")).toHaveValue("pixelbrick");
    await sectionLink(page, "Venues").click();
    await expect(page).toHaveURL(/#\/lighting\/venues$/);
    await expect(dialog(page)).toHaveCount(0);
  });
});

test.describe("A hand-written fixture type", () => {
  test("unsaved: Cancel asks; Stay keeps the edit; a save then leaves without asking", async ({
    page,
  }) => {
    const puts: unknown[] = [];
    await page.route(/\/api\/lighting\/fixture-types\/par(\?.*)?$/, (r) => {
      if (r.request().method() !== "PUT") return r.fallback();
      puts.push(r.request().postDataJSON());
      return r.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({ status: "saved" }),
      });
    });
    await page.goto("/#/lighting/fixtures/par");
    const first = page
      .getByTestId("ft-channel-row")
      .nth(0)
      .getByLabel("Channel name");
    await first.fill("rouge");
    await page.getByRole("button", { name: "Cancel" }).click();
    await expect(dialog(page)).toContainText(
      "Discard your unsaved changes to par?",
    );
    await dialog(page).getByRole("button", { name: "Cancel" }).click();
    await expect(page).toHaveURL(/#\/lighting\/fixtures\/par$/);
    await expect(first).toHaveValue("rouge");

    await page
      .locator(".editor-form")
      .getByRole("button", { name: "Save", exact: true })
      .click();
    await expect.poll(() => puts.length).toBe(1);
    await expect(page).toHaveURL(/#\/lighting\/fixtures$/);
    await expect(dialog(page)).toHaveCount(0);
  });

  test("a new type: leaving it unsaved asks", async ({ page }) => {
    await page.goto("/#/lighting/fixtures?new=light");
    await page.locator("#ft-name").fill("Strobe");
    await sectionLink(page, "Venues").click();
    await expect(dialog(page)).toContainText("Discard the new fixture type?");
    await dialog(page).getByRole("button", { name: "Confirm" }).click();
    await expect(page).toHaveURL(/#\/lighting\/venues$/);
  });
});

test.describe("The venue form", () => {
  test("unsaved: the browser's Back asks; Stay keeps it; Leave closes it", async ({
    page,
  }) => {
    await page.goto("/#/lighting/venues/test-venue");
    await card(page, "test-venue")
      .locator('[data-testid^="venue-edit-"]')
      .click();
    await expect(page).toHaveURL(/\?edit$/);
    const tag = page
      .getByTestId("venue-fixture-row")
      .nth(0)
      .locator(".tag-text-input");
    await tag.fill("new-tag");
    await tag.press("Enter");

    await page.goBack();
    await expect(dialog(page)).toContainText(
      "Discard your unsaved changes to the venue test-venue?",
    );
    await dialog(page).getByRole("button", { name: "Cancel" }).click();
    await expect(page).toHaveURL(/#\/lighting\/venues\/test-venue\?edit$/);
    await expect(
      page.locator(".tag-chip", { hasText: "new-tag" }),
    ).toBeVisible();

    await page.getByRole("button", { name: "Cancel" }).click();
    await dialog(page).getByRole("button", { name: "Confirm" }).click();
    await expect(page.locator(".editor-form")).toHaveCount(0);
  });

  test("clean: closing does not ask", async ({ page }) => {
    await page.goto("/#/lighting/venues/test-venue?edit");
    await expect(page.locator("#venue-name")).toHaveValue("test-venue");
    await page.getByRole("button", { name: "Cancel" }).click();
    await expect(page.locator(".editor-form")).toHaveCount(0);
    await expect(dialog(page)).toHaveCount(0);
  });

  test("a save then leaves without asking", async ({ page }) => {
    const puts: unknown[] = [];
    await page.route(/\/api\/lighting\/venues\/test-venue(\?.*)?$/, (r) => {
      if (r.request().method() !== "PUT") return r.fallback();
      puts.push(r.request().postDataJSON());
      return r.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({ status: "saved", venue_error: null }),
      });
    });
    await page.goto("/#/lighting/venues/test-venue?edit");
    const tag = page
      .getByTestId("venue-fixture-row")
      .nth(0)
      .locator(".tag-text-input");
    await tag.fill("saved-tag");
    await tag.press("Enter");
    await page
      .locator(".editor-form")
      .getByRole("button", { name: "Save", exact: true })
      .click();
    await expect.poll(() => puts.length).toBe(1);
    await expect(page.locator(".editor-form")).toHaveCount(0);
    await sectionLink(page, "Fixture types").click();
    await expect(page).toHaveURL(/#\/lighting\/fixtures$/);
    await expect(dialog(page)).toHaveCount(0);
  });
});
