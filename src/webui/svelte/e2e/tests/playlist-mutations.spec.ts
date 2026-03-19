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

test.describe("Playlist Mutations", () => {
  let playlists: PlaylistEditorPage;

  test.beforeEach(async ({ page }) => {
    playlists = new PlaylistEditorPage(page);
    await playlists.goto();
  });

  test("add song moves it from available to playlist", async () => {
    await playlists.selectPlaylist("setlist");
    await expect(playlists.playlistSongs()).toHaveCount(1);
    await expect(playlists.availableSongs()).toHaveCount(1);

    // Click the add button (+) on the available song
    const addBtn = playlists
      .availableSongs()
      .first()
      .locator('.btn-icon[title="Add"]');
    await addBtn.click();

    // Song should now be in the playlist
    await expect(playlists.playlistSongs()).toHaveCount(2);
  });

  test("remove song returns it to available", async () => {
    await playlists.selectPlaylist("setlist");
    await expect(playlists.playlistSongs()).toHaveCount(1);

    // Click remove button (✕ with title="Remove")
    const removeBtn = playlists
      .playlistSongs()
      .first()
      .locator('.btn-icon[title="Remove"]');
    await removeBtn.click();

    // Song should be removed
    await expect(playlists.playlistSongs()).toHaveCount(0);
  });

  test("save button is disabled when no changes", async () => {
    await playlists.selectPlaylist("setlist");
    await expect(playlists.saveButton).toBeDisabled();
  });

  test("save button enables after modification", async () => {
    await playlists.selectPlaylist("setlist");
    const addBtn = playlists
      .availableSongs()
      .first()
      .locator('.btn-icon[title="Add"]');
    await addBtn.click();
    await expect(playlists.saveButton).toBeEnabled();
  });

  test("save calls API", async ({ page }) => {
    let saveCalled = false;
    await page.route("**/api/playlists/setlist", async (route) => {
      if (route.request().method() === "PUT") {
        saveCalled = true;
        await route.fulfill({
          status: 200,
          contentType: "application/json",
          body: JSON.stringify({ status: "saved", name: "setlist" }),
        });
      } else {
        await route.continue();
      }
    });

    await playlists.selectPlaylist("setlist");
    const addBtn = playlists
      .availableSongs()
      .first()
      .locator('.btn-icon[title="Add"]');
    await addBtn.click();

    await playlists.saveButton.click();
    expect(saveCalled).toBe(true);
  });

  test("activate playlist calls API for non-active playlist", async ({
    page,
  }) => {
    let activateCalled = false;
    await page.route("**/api/playlists/rehearsal/activate", async (route) => {
      activateCalled = true;
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({ status: "activated", name: "rehearsal" }),
      });
    });

    // rehearsal is not active, so it should show the ▶ activate button
    const rehearsal = playlists.playlistByName("rehearsal");
    await rehearsal.locator('.btn-icon[title="Activate"]').click();
    expect(activateCalled).toBe(true);
  });

  test("delete button shows on non-all_songs playlists", async () => {
    const setlist = playlists.playlistByName("setlist");
    await expect(setlist.locator('.btn-icon[title="Delete"]')).toBeVisible();
  });

  test("delete click shows confirmation", async () => {
    const setlist = playlists.playlistByName("setlist");
    await setlist.locator('.btn-icon[title="Delete"]').click();

    // Should show Confirm and Cancel buttons
    await expect(
      setlist.locator(".btn-icon.danger", { hasText: "Confirm" }),
    ).toBeVisible();
    await expect(
      setlist.locator(".btn-icon", { hasText: "Cancel" }),
    ).toBeVisible();
  });

  test("delete cancel hides confirmation", async () => {
    const setlist = playlists.playlistByName("setlist");
    await setlist.locator('.btn-icon[title="Delete"]').click();
    await expect(
      setlist.locator(".btn-icon.danger", { hasText: "Confirm" }),
    ).toBeVisible();

    // Click cancel
    await setlist.locator(".btn-icon", { hasText: "Cancel" }).click();

    // Should go back to showing delete button
    await expect(setlist.locator('.btn-icon[title="Delete"]')).toBeVisible();
  });

  test("search filters available songs", async () => {
    await playlists.selectPlaylist("setlist");
    await playlists.searchInput.fill("Beta");
    await expect(playlists.availableSongs()).toHaveCount(1);
    await expect(playlists.availableSongs().first()).toContainText(
      "Test Song Beta",
    );
  });

  test("reorder buttons move songs", async () => {
    await playlists.selectPlaylist("setlist");

    // Add the available song to get 2 songs in the playlist
    const addBtn = playlists
      .availableSongs()
      .first()
      .locator('.btn-icon[title="Add"]');
    await addBtn.click();
    await expect(playlists.playlistSongs()).toHaveCount(2);

    // First song should be "Test Song Alpha"
    await expect(playlists.playlistSongs().nth(0)).toContainText(
      "Test Song Alpha",
    );

    // Click down arrow on first song (second .btn-icon.small in the reorder-btns)
    const downBtn = playlists
      .playlistSongs()
      .nth(0)
      .locator(".btn-icon.small")
      .last();
    await downBtn.click();

    // Now "Test Song Beta" should be first
    await expect(playlists.playlistSongs().nth(0)).toContainText(
      "Test Song Beta",
    );
  });

  test("all_songs playlist has explanatory tooltip", async () => {
    const allSongs = playlists.playlistByName("all_songs");
    const btn = allSongs.locator(".playlist-item");
    await expect(btn).toHaveAttribute("title", /auto-generated/i);
  });

  test("all_songs URL navigation does not select it", async ({ page }) => {
    await page.goto("/#/playlists/all_songs");
    // Should not show the detail panel for all_songs
    await expect(playlists.songColumns).not.toBeVisible();
  });

  test("all_songs playlist is disabled for selection", async () => {
    const allSongs = playlists.playlistByName("all_songs");
    const btn = allSongs.locator(".playlist-item");
    await expect(btn).toBeDisabled();
  });

  test("all_songs has no delete button", async () => {
    const allSongs = playlists.playlistByName("all_songs");
    await expect(
      allSongs.locator('.btn-icon[title="Delete"]'),
    ).not.toBeVisible();
  });
});
