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

test.describe("Playback State Transitions", () => {
  test.beforeEach(async ({ page }) => {
    // Intercept gRPC calls so play/stop don't fail
    await page.route("**/player.v1.PlayerService/**", async (route) => {
      await route.fulfill({
        status: 200,
        headers: {
          "content-type": "application/grpc-web+proto",
          "grpc-status": "0",
        },
        body: Buffer.alloc(0),
      });
    });

    await page.goto("/#/");
    await expect(page.locator(".playback-song")).toContainText(
      "Test Song Alpha",
    );
  });

  test("initial state shows stopped", async ({ page }) => {
    await expect(page.locator(".playback-status")).toContainText(/stopped/i);
    await expect(page.getByRole("button", { name: "Play" })).toBeVisible();
  });

  test("play button becomes stop when playing", async ({ page }) => {
    // Click play
    await page.getByRole("button", { name: "Play" }).click();

    // Simulate server responding with playing state
    await sendWsMessage(page, {
      type: "playback",
      is_playing: true,
      elapsed_ms: 1000,
      song_name: "Test Song Alpha",
      song_duration_ms: 180000,
      playlist_name: "setlist",
      playlist_position: 0,
      playlist_songs: ["Test Song Alpha", "Test Song Beta"],
      tracks: [
        { name: "kick", output_channels: [0, 1] },
        { name: "snare", output_channels: [2, 3] },
        { name: "bass", output_channels: [4, 5] },
      ],
      available_playlists: ["all_songs", "setlist"],
      persisted_playlist_name: "setlist",
      locked: false,
    });

    // Status should change to playing
    await expect(page.locator(".playback-status")).toContainText(/playing/i);
    // Play button should become Stop
    await expect(page.getByRole("button", { name: "Stop" })).toBeVisible();
  });

  test("progress bar updates during playback", async ({ page }) => {
    // Send playing state at 50% progress
    await sendWsMessage(page, {
      type: "playback",
      is_playing: true,
      elapsed_ms: 90000,
      song_name: "Test Song Alpha",
      song_duration_ms: 180000,
      playlist_name: "setlist",
      playlist_position: 0,
      playlist_songs: ["Test Song Alpha", "Test Song Beta"],
      tracks: [],
      available_playlists: ["all_songs", "setlist"],
      persisted_playlist_name: "setlist",
      locked: false,
    });

    // Progress bar should reflect ~50%
    const progressBar = page.locator(".progress-bar");
    await expect(progressBar).toBeVisible();
    const ariaValue = await progressBar.getAttribute("aria-valuenow");
    expect(Number(ariaValue)).toBeGreaterThan(40);
    expect(Number(ariaValue)).toBeLessThan(60);
  });

  test("next song changes current song", async ({ page }) => {
    await page.getByRole("button", { name: "Next" }).click();

    // Simulate server advancing to next song
    await sendWsMessage(page, {
      type: "playback",
      is_playing: false,
      elapsed_ms: 0,
      song_name: "Test Song Beta",
      song_duration_ms: 240000,
      playlist_name: "setlist",
      playlist_position: 1,
      playlist_songs: ["Test Song Alpha", "Test Song Beta"],
      tracks: [
        { name: "guitar", output_channels: [0, 1] },
        { name: "vocals", output_channels: [2, 3] },
      ],
      available_playlists: ["all_songs", "setlist"],
      persisted_playlist_name: "setlist",
      locked: false,
    });

    await expect(page.locator(".playback-song")).toContainText(
      "Test Song Beta",
    );
  });

  test("stop returns to stopped state", async ({ page }) => {
    // First put into playing state
    await sendWsMessage(page, {
      type: "playback",
      is_playing: true,
      elapsed_ms: 5000,
      song_name: "Test Song Alpha",
      song_duration_ms: 180000,
      playlist_name: "setlist",
      playlist_position: 0,
      playlist_songs: ["Test Song Alpha", "Test Song Beta"],
      tracks: [],
      available_playlists: ["all_songs", "setlist"],
      persisted_playlist_name: "setlist",
      locked: false,
    });

    await expect(page.getByRole("button", { name: "Stop" })).toBeVisible();
    await page.getByRole("button", { name: "Stop" }).click();

    // Simulate server stopping
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
      locked: false,
    });

    await expect(page.locator(".playback-status")).toContainText(/stopped/i);
    await expect(page.getByRole("button", { name: "Play" })).toBeVisible();
  });

  test("playing state updates page title with song name", async ({ page }) => {
    await sendWsMessage(page, {
      type: "playback",
      is_playing: true,
      elapsed_ms: 1000,
      song_name: "Test Song Alpha",
      song_duration_ms: 180000,
      playlist_name: "setlist",
      playlist_position: 0,
      playlist_songs: ["Test Song Alpha", "Test Song Beta"],
      tracks: [],
      available_playlists: ["all_songs", "setlist"],
      persisted_playlist_name: "setlist",
      locked: false,
    });

    // Title should show the play symbol and song name
    await expect(page).toHaveTitle(/▶.*Test Song Alpha/, { timeout: 5000 });
  });

  test("playlist position highlights correct song", async ({ page }) => {
    // Move to second song
    await sendWsMessage(page, {
      type: "playback",
      is_playing: false,
      elapsed_ms: 0,
      song_name: "Test Song Beta",
      song_duration_ms: 240000,
      playlist_name: "setlist",
      playlist_position: 1,
      playlist_songs: ["Test Song Alpha", "Test Song Beta"],
      tracks: [],
      available_playlists: ["all_songs", "setlist"],
      persisted_playlist_name: "setlist",
      locked: false,
    });

    const currentSong = page.locator(".playlist-songs li.current");
    await expect(currentSong).toContainText("Test Song Beta");
  });

  test("elapsed time updates in progress display", async ({ page }) => {
    await sendWsMessage(page, {
      type: "playback",
      is_playing: true,
      elapsed_ms: 65000,
      song_name: "Test Song Alpha",
      song_duration_ms: 180000,
      playlist_name: "setlist",
      playlist_position: 0,
      playlist_songs: ["Test Song Alpha", "Test Song Beta"],
      tracks: [],
      available_playlists: ["all_songs", "setlist"],
      persisted_playlist_name: "setlist",
      locked: false,
    });

    // Should show elapsed time (1:05)
    await expect(page.locator(".progress-time")).toContainText("1:05");
  });
});
