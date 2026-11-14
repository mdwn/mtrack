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
import { Code, ConnectError } from "@connectrpc/connect";
import { PlayerService } from "../../src/gen/player/v1/player_pb";
import { CONFIG_STORE } from "../mock-server/test-data";
import { getConfigReply } from "../support/config-rpc";
import { listSongsReply } from "../support/library-rpc";
import { mockRpc } from "../support/rpc";

test.describe("Error States", () => {
  test("songs page shows error on API failure", async ({ page }) => {
    const rpc = await mockRpc(page);
    rpc.error(
      PlayerService.method.listSongs,
      Code.Internal,
      "Internal server error",
    );

    await page.goto("/#/songs");
    await expect(page.locator(".status.error, .error-banner")).toBeVisible();
  });

  test("playlists page shows error on API failure", async ({ page }) => {
    const rpc = await mockRpc(page);
    rpc.error(
      PlayerService.method.listPlaylists,
      Code.Internal,
      "Internal server error",
    );

    await page.goto("/#/playlists");
    await expect(page.locator(".error-banner")).toBeVisible();
  });

  test("config page shows error on API failure", async ({ page }) => {
    const rpc = await mockRpc(page);
    rpc.error(
      PlayerService.method.getConfig,
      Code.Internal,
      "The config store is unavailable",
    );

    await page.goto("/#/config");
    // Config editor shows error state with retry button
    await expect(page.getByText(/error/i)).toBeVisible();
    await expect(page.getByRole("button", { name: "Retry" })).toBeVisible();
  });

  test("config error retry button refetches", async ({ page }) => {
    let callCount = 0;
    const rpc = await mockRpc(page);
    rpc.unary(PlayerService.method.getConfig, () => {
      callCount++;
      if (callCount === 1) {
        throw new ConnectError("Temporary error", Code.Internal);
      }
      return getConfigReply(CONFIG_STORE);
    });

    await page.goto("/#/config");
    await expect(page.getByRole("button", { name: "Retry" })).toBeVisible();

    // Click retry - should succeed on second attempt
    await page.getByRole("button", { name: "Retry" }).click();
    await expect(
      page.getByRole("heading", { name: "Hardware Profiles" }),
    ).toBeVisible();
  });

  test("playlist save error shows error banner", async ({ page }) => {
    await page.goto("/#/playlists");

    const rpc = await mockRpc(page);
    rpc.error(
      PlayerService.method.savePlaylist,
      Code.AlreadyExists,
      "Conflict: playlist modified",
    );

    // Select setlist and modify it
    await page.locator(".playlist-item", { hasText: "setlist" }).click();
    await expect(page.locator(".song-columns")).toBeVisible();

    // Add a song to make dirty
    const addBtn = page
      .locator(".song-list.available li")
      .first()
      .locator('.btn-icon[title="Add"]');
    await addBtn.click();

    // Try to save
    await page.getByRole("button", { name: "Save" }).click();
    await expect(page.locator(".error-banner")).toBeVisible();
    // The server's own message is what the banner says.
    await expect(page.locator(".error-banner")).toContainText(
      "Conflict: playlist modified",
    );
  });

  test("song detail shows error for non-existent song", async ({ page }) => {
    const rpc = await mockRpc(page);
    rpc.unary(
      PlayerService.method.listSongs,
      listSongsReply({ songs: [], failures: [] }),
    );

    await page.goto("/#/songs/NonExistentSong");
    // The player's own answer is what the page says.
    await expect(
      page.getByText(/unable to find song NonExistentSong/i),
    ).toBeVisible();
  });
});
