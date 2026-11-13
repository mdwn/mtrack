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

import { test, expect, type Page } from "@playwright/test";
import { SONGS } from "../mock-server/test-data";
import { PlayerService } from "../../src/gen/player/v1/player_pb";
import { listSongsReply } from "../support/library-rpc";
import { mockRpc } from "../support/rpc";

import type { SongList } from "../../src/gen/rules/mtrack_rules";

// Init and the bulk import name a nested song by its path when its folder's
// name is taken (`SongA/tracks`, `SongB/tracks`). The name is one encoded
// segment of the address (`#/songs/SongA%2Ftracks`), so a name ending in a
// tab's name is not read as that tab of another song.

/** Each song and the tracks its config names: told apart by their count. */
const SLASH_SONGS: Record<string, string[]> = {
  "SongA/tracks": ["kick"],
  "SongB/tracks": ["guitar", "vocals"],
};

async function routeSlashSongs(page: Page) {
  const list = {
    songs: Object.entries(SLASH_SONGS).map(([name, tracks]) => ({
      ...SONGS.songs[0],
      name,
      base_dir: name,
      tracks,
      track_count: tracks.length,
      lighting_files: [],
      has_lighting: false,
      has_midi: false,
    })),
    failures: [],
  } satisfies SongList;
  const rpc = await mockRpc(page);
  rpc.unary(PlayerService.method.listSongs, listSongsReply(list));
  await page.route(
    (url) =>
      Object.keys(SLASH_SONGS).some(
        (name) => url.pathname === `/api/songs/${encodeURIComponent(name)}`,
      ),
    (route) => {
      const encoded = new URL(route.request().url()).pathname.split("/").pop();
      const name = decodeURIComponent(encoded ?? "");
      const tracks = SLASH_SONGS[name]
        .map((track) => `  - name: ${track}\n    file: ${track}.wav\n`)
        .join("");
      return route.fulfill({
        status: 200,
        contentType: "text/yaml",
        body: `name: ${name}\ntracks:\n${tracks}`,
      });
    },
  );
}

test.describe("Song names with a slash", () => {
  test.beforeEach(async ({ page }) => {
    await routeSlashSongs(page);
  });

  for (const [name, tracks] of Object.entries(SLASH_SONGS)) {
    const encoded = encodeURIComponent(name);

    test(`${name} opens its own page and title`, async ({ page }) => {
      await page.goto(`/#/songs/${encoded}`);
      await expect(page.locator(".song-title")).toHaveText(name);
      await expect(page.locator(".tab.active")).toContainText("Tracks");
      await expect(page.locator(".track-editor .track-row")).toHaveCount(
        tracks.length,
      );
      await expect(page).toHaveTitle(new RegExp(`^Songs - ${name} - `));
    });

    test(`${name}'s tabs stay on ${name}`, async ({ page }) => {
      await page.goto(`/#/songs/${encoded}`);
      await expect(page.locator(".song-title")).toHaveText(name);

      await page.locator(".tab", { hasText: "Config" }).click();
      await expect(page.locator(".tab.active")).toContainText("Config");
      await expect(page).toHaveURL(new RegExp(`#/songs/${encoded}/config$`));
      await expect(page.locator(".song-title")).toHaveText(name);

      await page.locator(".tab", { hasText: "Tracks" }).click();
      await expect(page.locator(".tab.active")).toContainText("Tracks");
      await expect(page).toHaveURL(new RegExp(`#/songs/${encoded}$`));
      await expect(page.locator(".track-editor .track-row")).toHaveCount(
        tracks.length,
      );

      await page.goto(`/#/songs/${encoded}/tracks`);
      await expect(page.locator(".song-title")).toHaveText(name);
      await expect(page.locator(".tab.active")).toContainText("Tracks");
      await expect(page).toHaveTitle(new RegExp(`^Songs - ${name} - `));
    });
  }
});
