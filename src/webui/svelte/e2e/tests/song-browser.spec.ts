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
import { SongBrowserPage } from "../pages/song-browser.page.js";

test.describe("Song Browser", () => {
  let songs: SongBrowserPage;

  test.beforeEach(async ({ page }) => {
    songs = new SongBrowserPage(page);
    await songs.goto();
  });

  test("shows songs heading", async () => {
    await expect(songs.heading).toBeVisible();
  });

  test("lists songs from API", async () => {
    await expect(songs.songByName("Test Song Alpha")).toBeVisible();
    await expect(songs.songByName("Test Song Beta")).toBeVisible();
  });

  test("shows song metadata", async () => {
    const alpha = songs.songByName("Test Song Alpha");
    await expect(alpha).toContainText("3:00");
    await expect(alpha).toContainText("3 tracks");
  });

  test("shows song metadata for second song", async () => {
    const beta = songs.songByName("Test Song Beta");
    await expect(beta).toContainText("4:00");
    await expect(beta).toContainText("2 tracks");
  });

  test("shows MIDI badge for songs with MIDI", async () => {
    const alpha = songs.songByName("Test Song Alpha");
    await expect(alpha).toContainText(/midi/i);
  });

  test("shows lighting badge for songs with lighting", async () => {
    const alpha = songs.songByName("Test Song Alpha");
    await expect(alpha).toContainText(/light/i);
  });

  test("shows failed songs", async () => {
    await expect(songs.songByName("Broken Song")).toBeVisible();
  });

  test("clicking a song navigates to detail view", async ({ page }) => {
    await songs.selectSong("Test Song Alpha");
    await expect(page).toHaveURL(/.*#\/songs\/Test%20Song%20Alpha/);
  });

  test("search filters songs", async () => {
    await songs.searchInput.fill("Alpha");
    await expect(songs.songByName("Test Song Alpha")).toBeVisible();
    await expect(songs.songByName("Test Song Beta")).not.toBeVisible();
  });

  test("clearing search shows all songs", async () => {
    await songs.searchInput.fill("Alpha");
    await songs.searchInput.clear();
    await expect(songs.songByName("Test Song Alpha")).toBeVisible();
    await expect(songs.songByName("Test Song Beta")).toBeVisible();
  });
});
