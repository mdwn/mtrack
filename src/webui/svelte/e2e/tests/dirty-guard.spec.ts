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

test.describe("Dirty navigation guard", () => {
  test("clean editor: navigating away does not prompt", async ({ page }) => {
    await page.goto("/#/songs/Test%20Song%20Alpha");
    await expect(page.locator(".tab.active")).toContainText("Tracks");

    // Click a top-nav tab; should navigate immediately.
    await page.locator('.topnav__tab[href="#/playlists"]').click();
    await expect(page).toHaveURL(/.*#\/playlists/);
  });

  test("dirty editor: navigating away surfaces a confirm dialog", async ({
    page,
  }) => {
    await page.goto("/#/songs/Test%20Song%20Alpha");
    await expect(page.locator(".tab.active")).toContainText("Tracks");

    // Toggle the Loop Playback checkbox to dirty the form.
    await page.locator("#loop-playback").check();
    await expect(page.locator(".unsaved")).toBeVisible();

    // Click a top-nav tab; the confirm dialog should appear and the
    // URL should remain on the song detail until we resolve it.
    await page.locator('.topnav__tab[href="#/playlists"]').click();
    await expect(page.getByText(/discard unsaved/i)).toBeVisible();

    // Cancel — should stay on the song detail.
    await page.getByRole("button", { name: "Cancel" }).click();
    await expect(page).toHaveURL(/.*#\/songs\/Test%20Song%20Alpha/);
    // Edits survive.
    await expect(page.locator("#loop-playback")).toBeChecked();
    await expect(page.locator(".unsaved")).toBeVisible();
  });

  test("dirty editor: confirming discard navigates", async ({ page }) => {
    await page.goto("/#/songs/Test%20Song%20Alpha");
    await expect(page.locator(".tab.active")).toContainText("Tracks");
    await page.locator("#loop-playback").check();
    await expect(page.locator(".unsaved")).toBeVisible();

    await page.locator('.topnav__tab[href="#/playlists"]').click();

    // Accept discard.
    await page.getByRole("button", { name: /confirm|ok|discard/i }).click();
    await expect(page).toHaveURL(/.*#\/playlists/);
  });

  test("dirty editor: tab switch within song detail does NOT prompt", async ({
    page,
  }) => {
    await page.goto("/#/songs/Test%20Song%20Alpha");
    await page.locator("#loop-playback").check();
    await expect(page.locator(".unsaved")).toBeVisible();

    // Switching tabs is intra-component nav (same scope) — no prompt.
    await page.locator(".tab", { hasText: "Lighting" }).click();
    await expect(page.locator(".tab.active")).toContainText("Lighting");
    // Edits survive.
    await expect(page.locator(".unsaved")).toBeVisible();
  });
});
