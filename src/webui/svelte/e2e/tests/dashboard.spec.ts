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
import { DashboardPage } from "../pages/dashboard.page.js";

test.describe("Dashboard", () => {
  let dashboard: DashboardPage;

  test.beforeEach(async ({ page }) => {
    dashboard = new DashboardPage(page);
    await dashboard.goto();
  });

  test("renders dashboard grid", async () => {
    await expect(dashboard.grid).toBeVisible();
  });

  test("shows current song name from WebSocket", async () => {
    await expect(dashboard.songName).toBeVisible();
    await expect(dashboard.songName).toHaveText("Test Song Alpha");
  });

  test("shows stopped status when not playing", async () => {
    await expect(dashboard.playbackStatus).toContainText(/stopped/i);
  });

  test("shows transport controls", async () => {
    await expect(dashboard.playButton).toBeVisible();
    await expect(dashboard.prevButton).toBeVisible();
    await expect(dashboard.nextButton).toBeVisible();
  });

  test("shows progress bar", async () => {
    await expect(dashboard.progressBar).toBeVisible();
  });

  test("shows progress time", async () => {
    await expect(dashboard.progressTime).toBeVisible();
  });

  test("shows playlist songs", async () => {
    await expect(dashboard.playlistSongs.first()).toBeVisible();
    await expect(dashboard.playlistSongs).toHaveCount(2);
    await expect(dashboard.playlistSongs.nth(0)).toContainText(
      "Test Song Alpha",
    );
    await expect(dashboard.playlistSongs.nth(1)).toContainText(
      "Test Song Beta",
    );
  });

  test("highlights current song in playlist", async ({ page }) => {
    const currentSong = page.locator(".playlist-songs li.current");
    await expect(currentSong).toBeVisible();
    await expect(currentSong).toContainText("Test Song Alpha");
  });

  test("shows playlist selector with available playlists", async () => {
    await expect(dashboard.playlistSelect).toBeVisible();
    const options = dashboard.playlistSelect.locator("option");
    await expect(options).toHaveCount(2);
  });

  test("shows tracks from WebSocket", async () => {
    await expect(dashboard.trackRows.first()).toBeVisible();
    await expect(dashboard.trackRows).toHaveCount(3);
    await expect(dashboard.trackRows.nth(0)).toContainText("kick");
    await expect(dashboard.trackRows.nth(1)).toContainText("snare");
    await expect(dashboard.trackRows.nth(2)).toContainText("bass");
  });

  test("shows track count", async () => {
    await expect(dashboard.trackCount).toContainText("3");
  });

  test("play button triggers gRPC call", async ({ page }) => {
    let grpcCalled = false;
    await page.route("**/player.v1.PlayerService/**", async (route) => {
      grpcCalled = true;
      await route.fulfill({
        status: 200,
        headers: {
          "content-type": "application/grpc-web+proto",
          "grpc-status": "0",
        },
        body: Buffer.alloc(0),
      });
    });

    await dashboard.playButton.click();
    expect(grpcCalled).toBe(true);
  });
});
