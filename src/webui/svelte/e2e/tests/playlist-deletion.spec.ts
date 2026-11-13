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
import { PlayerService } from "../../src/gen/player/v1/player_pb";
import type { PlaylistInfo } from "../../src/gen/rules/mtrack_rules";
import { listPlaylistsReply } from "../support/library-rpc";
import { mockRpc } from "../support/rpc";

test.describe("Playlist Deletion", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/#/playlists");
    await expect(
      page.locator(".playlist-list li", { hasText: "setlist" }),
    ).toBeVisible();
  });

  test("confirm delete calls API", async ({ page }) => {
    const rpc = await mockRpc(page);
    rpc.unary(PlayerService.method.deletePlaylist, {});

    const setlist = page.locator(".playlist-list li", { hasText: "setlist" });

    // Click delete (✕)
    await setlist.locator('.btn-icon[title="Delete"]').click();
    // Confirm
    await setlist.locator(".btn-icon.danger", { hasText: "Confirm" }).click();

    await expect
      .poll(() => rpc.calls(PlayerService.method.deletePlaylist).length)
      .toBe(1);
    expect(rpc.calls(PlayerService.method.deletePlaylist)[0].name).toBe(
      "setlist",
    );
  });

  test("delete removes playlist from list", async ({ page }) => {
    const rpc = await mockRpc(page);
    rpc.unary(PlayerService.method.deletePlaylist, {});

    // After delete, mock playlists returns without setlist
    rpc.unary(
      PlayerService.method.listPlaylists,
      listPlaylistsReply([
        {
          name: "all_songs",
          song_count: 2,
          is_active: false,
          reserved: true,
        },
        {
          name: "rehearsal",
          song_count: 2,
          is_active: false,
          reserved: false,
        },
      ] satisfies PlaylistInfo[]),
    );

    const setlist = page.locator(".playlist-list li", { hasText: "setlist" });
    await setlist.locator('.btn-icon[title="Delete"]').click();
    await setlist.locator(".btn-icon.danger", { hasText: "Confirm" }).click();

    // Setlist should no longer be in the list
    await expect(
      page.locator(".playlist-list li", { hasText: "setlist" }),
    ).not.toBeVisible();
  });
});
