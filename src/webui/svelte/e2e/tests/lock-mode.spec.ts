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

// Helper to push a WebSocket message to all connected clients via mock server.
async function sendWsMessage(
  page: import("@playwright/test").Page,
  msg: object,
) {
  await page.request.post("http://127.0.0.1:3111/test/send-ws", {
    data: msg,
  });
}

test.describe("Lock Mode", () => {
  test("lock toggle is visible on dashboard", async ({ page }) => {
    await page.goto("/#/");
    const lockToggle = page.locator(".lock-toggle");
    await expect(lockToggle).toBeVisible();
  });

  test("lock toggle calls API when clicked", async ({ page }) => {
    await page.goto("/#/");
    let lockCalled = false;
    await page.route("**/api/lock", async (route) => {
      if (route.request().method() === "PUT") {
        lockCalled = true;
        await route.fulfill({
          status: 200,
          contentType: "application/json",
          body: JSON.stringify({ locked: true }),
        });
      } else {
        await route.continue();
      }
    });

    const lockToggle = page.locator(".lock-toggle");
    await lockToggle.click();
    expect(lockCalled).toBe(true);
  });

  test("unlocked state does not have locked class", async ({ page }) => {
    await page.goto("/#/");
    await expect(page.locator(".lock-toggle")).toBeVisible();
    await expect(page.locator(".lock-toggle")).not.toHaveClass(/locked/);
  });

  test("locked state prevents config save", async ({ page }) => {
    await page.goto("/#/config");
    await expect(
      page.getByRole("heading", { name: "Hardware Profiles" }),
    ).toBeVisible();

    // Lock the player via WebSocket
    await sendWsMessage(page, {
      type: "playback",
      is_playing: false,
      elapsed_ms: 0,
      song_name: "Test Song Alpha",
      song_duration_ms: 180000,
      playlist_name: "setlist",
      playlist_position: 0,
      playlist_songs: ["Test Song Alpha", "Test Song Beta"],
      tracks: [],
      available_playlists: ["all_songs", "setlist"],
      persisted_playlist_name: "setlist",
      locked: true,
    });

    // Navigate into a profile
    await page.locator(".profile-row", { hasText: "test-host" }).click();
    await expect(page.getByRole("button", { name: "Back" })).toBeVisible();

    // Modify hostname
    const hostnameInput = page.locator("#profile-hostname");
    await hostnameInput.fill("locked-hostname");

    // Try to save
    await page.getByRole("button", { name: "Save" }).click();

    // Should show lock error message
    await expect(page.locator(".save-msg")).toContainText(/locked/i);
  });

  test("locked state prevents playlist save", async ({ page }) => {
    await page.goto("/#/playlists");

    // Lock the player via WebSocket
    await sendWsMessage(page, {
      type: "playback",
      is_playing: false,
      elapsed_ms: 0,
      song_name: "Test Song Alpha",
      song_duration_ms: 180000,
      playlist_name: "setlist",
      playlist_position: 0,
      playlist_songs: ["Test Song Alpha", "Test Song Beta"],
      tracks: [],
      available_playlists: ["all_songs", "setlist"],
      persisted_playlist_name: "setlist",
      locked: true,
    });

    // Select and modify a playlist
    await page.locator(".playlist-item", { hasText: "setlist" }).click();
    await expect(page.locator(".song-columns")).toBeVisible();

    // Add a song
    const addBtn = page
      .locator(".song-list.available li")
      .first()
      .locator('.btn-icon[title="Add"]');
    await addBtn.click();

    // Try to save
    await page.getByRole("button", { name: "Save" }).click();

    // Should show lock error
    await expect(page.locator(".error-banner")).toContainText(/locked/i);
  });
});
