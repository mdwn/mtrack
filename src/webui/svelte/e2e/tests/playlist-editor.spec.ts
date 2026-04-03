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
import { PlaylistEditorPage } from "../pages/playlist-editor.page.js";

test.describe("Playlist Editor", () => {
  let playlists: PlaylistEditorPage;

  test.beforeEach(async ({ page }) => {
    playlists = new PlaylistEditorPage(page);
    await playlists.goto();
  });

  test("shows playlist list panel", async () => {
    await expect(playlists.listPanel).toBeVisible();
  });

  test("lists playlists from API", async () => {
    await expect(playlists.playlistByName("all_songs")).not.toBeVisible();
    await expect(playlists.playlistByName("setlist")).toBeVisible();
    await expect(playlists.playlistByName("rehearsal")).toBeVisible();
  });

  test("shows song count for playlists", async () => {
    await expect(playlists.playlistByName("setlist")).toContainText("1");
  });

  test("shows active badge on active playlist", async () => {
    const setlist = playlists.playlistByName("setlist");
    await expect(setlist.locator(".badge")).toBeVisible();
  });

  test("all_songs is not in the playlist list", async () => {
    await expect(playlists.playlistByName("all_songs")).not.toBeVisible();
  });

  test("selecting a playlist shows detail panel", async () => {
    await playlists.selectPlaylist("setlist");
    await expect(playlists.detailPanel).toBeVisible();
  });

  test("selecting a playlist updates URL", async ({ page }) => {
    await playlists.selectPlaylist("setlist");
    await expect(page).toHaveURL(/.*#\/playlists\/setlist/);
  });

  test("detail panel shows playlist songs", async () => {
    await playlists.selectPlaylist("setlist");
    await expect(playlists.playlistSongs().first()).toBeVisible();
    await expect(playlists.playlistSongs().first()).toContainText(
      "Test Song Alpha",
    );
  });

  test("detail panel shows available songs", async () => {
    await playlists.selectPlaylist("setlist");
    await expect(playlists.availableSongs().first()).toBeVisible();
    await expect(playlists.availableSongs().first()).toContainText(
      "Test Song Beta",
    );
  });

  test("new button toggles new playlist form", async () => {
    await playlists.newButton.click();
    await expect(playlists.newPlaylistInput).toBeVisible();
    await expect(playlists.createButton).toBeVisible();
  });

  test("shows position numbers for playlist songs", async () => {
    await playlists.selectPlaylist("rehearsal");
    const songs = playlists.playlistSongs();
    await expect(songs).toHaveCount(2);
    await expect(songs.nth(0).locator(".song-position")).toHaveText("1.");
    await expect(songs.nth(1).locator(".song-position")).toHaveText("2.");
  });

  test("create new playlist calls API", async ({ page }) => {
    let createCalled = false;
    await page.route("**/api/playlists/my-playlist", async (route) => {
      if (route.request().method() === "PUT") {
        createCalled = true;
        await route.fulfill({
          status: 200,
          contentType: "application/json",
          body: JSON.stringify({ status: "saved", name: "my-playlist" }),
        });
      } else {
        await route.continue();
      }
    });

    await playlists.newButton.click();
    await playlists.newPlaylistInput.fill("my-playlist");
    await playlists.createButton.click();
    expect(createCalled).toBe(true);
  });
});
