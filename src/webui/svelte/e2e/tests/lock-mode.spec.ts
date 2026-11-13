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
import { sendWatch } from "../support/watch";
import { PLAYBACK_STATE } from "../mock-server/test-data";
import { PlayerService } from "../../src/gen/player/v1/player_pb";
import { mockRpc } from "../support/rpc";

let lockTestCounter = 0;

test.describe("Lock Mode", () => {
  test("lock toggle is visible on dashboard", async ({ page }) => {
    await page.goto("/#/");
    const lockToggle = page.locator(".topnav__lock");
    await expect(lockToggle).toBeVisible();
  });

  test("lock toggle calls API when clicked", async ({ page }) => {
    await page.goto("/#/");
    const rpc = await mockRpc(page);
    rpc.unary(PlayerService.method.setLock, { locked: true });

    const lockToggle = page.locator(".topnav__lock");
    // Locked until the first playback event: wait for it, or the click
    // would be an unlock, which asks first.
    await expect(lockToggle).not.toHaveClass(/topnav__lock--locked/);
    await lockToggle.click();
    await expect
      .poll(() => rpc.calls(PlayerService.method.setLock).length)
      .toBe(1);
    expect(rpc.calls(PlayerService.method.setLock)[0].locked).toBe(true);
  });

  test("unlocked state does not have locked class", async ({ page }) => {
    await page.goto("/#/");
    await expect(page.locator(".topnav__lock")).toBeVisible();
    await expect(page.locator(".topnav__lock")).not.toHaveClass(
      /topnav__lock--locked/,
    );
  });

  test("locked state disables config Save button", async ({ page }) => {
    const wsId = `lock-config-${++lockTestCounter}-${Date.now()}`;
    await page.goto(`/?wsId=${wsId}#/config`);
    await expect(
      page.getByRole("heading", { name: "Hardware Profiles" }),
    ).toBeVisible();

    // Lock the player via the Watch stream
    await sendWatch(page, wsId, {
      ...PLAYBACK_STATE,
      is_playing: false,
      elapsed_ms: 0,
      song_name: "Test Song Alpha",
      song_duration_ms: 180000,
      playlist_name: "setlist",
      playlist_position: 0,
      playlist_at_first: true,
      playlist_at_last: false,
      playlist_songs: ["Test Song Alpha", "Test Song Beta"],
      tracks: [],
      available_playlists: ["all_songs", "setlist"],
      persisted_playlist_name: "setlist",
      locked: true,
    });

    // Navigate into a profile and dirty the form.
    await page.locator(".profile-row", { hasText: "test-host" }).click();
    await expect(page.getByRole("button", { name: "Back" })).toBeVisible();
    await page.locator("#profile-hostname").fill("locked-hostname");

    // The Save button must surface "disabled" while the player is locked,
    // so a click can't even reach the API.
    await expect(page.getByRole("button", { name: "Save" })).toBeDisabled();
  });

  test("locked state disables playlist Save button", async ({ page }) => {
    const wsId = `lock-playlist-${++lockTestCounter}-${Date.now()}`;
    await page.goto(`/?wsId=${wsId}#/playlists`);

    await expect(page.locator(".topnav__conn")).not.toHaveClass(
      /topnav__conn--off/,
    );

    // Send unlocked first so we can dirty the playlist, then lock it.
    await sendWatch(page, wsId, {
      ...PLAYBACK_STATE,
      is_playing: false,
      elapsed_ms: 0,
      song_name: "Test Song Alpha",
      song_duration_ms: 180000,
      playlist_name: "setlist",
      playlist_position: 0,
      playlist_at_first: true,
      playlist_at_last: false,
      playlist_songs: ["Test Song Alpha", "Test Song Beta"],
      tracks: [],
      available_playlists: ["all_songs", "setlist"],
      persisted_playlist_name: "setlist",
      locked: false,
    });
    await expect(page.locator(".topnav__lock--locked")).not.toBeVisible();

    await page.locator(".playlist-item", { hasText: "setlist" }).click();
    await expect(page.locator(".song-columns")).toBeVisible();
    const addBtn = page
      .locator(".song-list.available li")
      .first()
      .locator(".btn-icon");
    await addBtn.click();

    // Now lock — the Save button should immediately become disabled.
    await sendWatch(page, wsId, {
      ...PLAYBACK_STATE,
      is_playing: false,
      elapsed_ms: 0,
      song_name: "Test Song Alpha",
      song_duration_ms: 180000,
      playlist_name: "setlist",
      playlist_position: 0,
      playlist_at_first: true,
      playlist_at_last: false,
      playlist_songs: ["Test Song Alpha", "Test Song Beta"],
      tracks: [],
      available_playlists: ["all_songs", "setlist"],
      persisted_playlist_name: "setlist",
      locked: true,
    });
    await expect(page.locator(".topnav__lock--locked")).toBeVisible();

    await expect(page.getByRole("button", { name: "Save" })).toBeDisabled();
  });

  test("LIVE-locked stripe surfaces when locked", async ({ page }) => {
    const wsId = `lock-stripe-${++lockTestCounter}-${Date.now()}`;
    await page.goto(`/?wsId=${wsId}#/`);
    await sendWatch(page, wsId, {
      ...PLAYBACK_STATE,
      is_playing: false,
      elapsed_ms: 0,
      song_name: "Test Song Alpha",
      song_duration_ms: 180000,
      playlist_name: "setlist",
      playlist_position: 0,
      playlist_at_first: true,
      playlist_at_last: false,
      playlist_songs: ["Test Song Alpha", "Test Song Beta"],
      tracks: [],
      available_playlists: ["all_songs", "setlist"],
      persisted_playlist_name: "setlist",
      locked: true,
    });
    await expect(page.locator(".live-stripe")).toBeVisible();
    await expect(page.locator(".live-stripe")).toContainText(/locked/i);
  });

  test("connection dot links to status page", async ({ page }) => {
    await page.goto("/#/");
    await expect(page.locator(".topnav__conn")).toHaveAttribute(
      "href",
      "#/status",
    );
  });
});
