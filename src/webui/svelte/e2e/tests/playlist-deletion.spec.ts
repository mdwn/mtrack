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

test.describe("Playlist Deletion", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/#/playlists");
    await expect(
      page.locator(".playlist-list li", { hasText: "setlist" }),
    ).toBeVisible();
  });

  test("confirm delete calls API", async ({ page }) => {
    let deleteCalled = false;
    await page.route("**/api/playlists/setlist", async (route) => {
      if (route.request().method() === "DELETE") {
        deleteCalled = true;
        await route.fulfill({
          status: 200,
          contentType: "application/json",
          body: JSON.stringify({ status: "deleted", name: "setlist" }),
        });
      } else {
        await route.continue();
      }
    });

    const setlist = page.locator(".playlist-list li", { hasText: "setlist" });

    // Click delete (✕)
    await setlist.locator('.btn-icon[title="Delete"]').click();
    // Confirm
    await setlist.locator(".btn-icon.danger", { hasText: "Confirm" }).click();

    expect(deleteCalled).toBe(true);
  });

  test("delete removes playlist from list", async ({ page }) => {
    await page.route("**/api/playlists/setlist", async (route) => {
      if (route.request().method() === "DELETE") {
        await route.fulfill({
          status: 200,
          contentType: "application/json",
          body: JSON.stringify({ status: "deleted", name: "setlist" }),
        });
      } else {
        await route.continue();
      }
    });

    // After delete, mock playlists returns without setlist
    await page.route("**/api/playlists", async (route) => {
      if (route.request().method() === "GET") {
        await route.fulfill({
          status: 200,
          contentType: "application/json",
          body: JSON.stringify([
            { name: "all_songs", song_count: 2, is_active: false },
            { name: "rehearsal", song_count: 2, is_active: false },
          ]),
        });
      } else {
        await route.continue();
      }
    });

    const setlist = page.locator(".playlist-list li", { hasText: "setlist" });
    await setlist.locator('.btn-icon[title="Delete"]').click();
    await setlist.locator(".btn-icon.danger", { hasText: "Confirm" }).click();

    // Setlist should no longer be in the list
    await expect(
      page.locator(".playlist-list li", { hasText: "setlist" }),
    ).not.toBeVisible();
  });
});
