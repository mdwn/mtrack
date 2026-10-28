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
import { PlaylistEditorPage } from "../pages/playlist-editor.page.js";

let testCounter = 0;

async function sendWsMessage(page: Page, wsId: string, msg: object) {
  await page.request.post("http://127.0.0.1:3111/test/send-ws", {
    data: { ...msg, _wsId: wsId },
  });
}

/** Captures each PUT to the setlist, answering it as saved. */
async function capturePlaylistPut(page: Page) {
  const bodies: Record<string, unknown>[] = [];
  await page.route("**/api/playlists/setlist", async (route) => {
    if (route.request().method() === "PUT") {
      bodies.push(route.request().postDataJSON());
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({ status: "saved", name: "setlist" }),
      });
    } else {
      await route.continue();
    }
  });
  return bodies;
}

test.describe("Playlist editor: transitions", () => {
  let playlists: PlaylistEditorPage;

  test.beforeEach(async ({ page }) => {
    playlists = new PlaylistEditorPage(page);
    await playlists.goto();
  });

  test("an entry's fades are set in its transitions panel", async ({
    page,
  }) => {
    const bodies = await capturePlaylistPut(page);
    await playlists.selectPlaylist("setlist");
    const row = playlists.playlistSongs().first();
    await expect(row.getByTestId("transitions-panel")).toHaveCount(0);
    await row.getByTestId("transitions-toggle").click();
    const panel = row.getByTestId("transitions-panel");
    await expect(panel).toBeVisible();
    await expect(panel).toContainText("Only for loop breaks");

    // Settings beyond the length wait for one.
    await expect(panel.getByTestId("fade_out-curve")).toBeDisabled();
    await panel.getByTestId("fade_out-length").fill("1measure");
    await panel.getByTestId("fade_out-length").press("Enter");
    await panel.getByTestId("fade_out-curve").selectOption("linear");
    await panel.getByTestId("fade_out-lights").fill("0s");
    await panel.getByTestId("fade_out-lights").press("Enter");
    // The song's own tracks are offered to leave out.
    const outExclude = panel
      .getByTestId("fade_out-fields")
      .getByTestId("fade_out-exclude");
    await expect(outExclude).toHaveCount(3);
    await panel.getByTestId("fade_out-fields").getByLabel("kick").check();
    await panel.getByTestId("fade_in-length").fill("4beats");
    await panel.getByTestId("fade_in-length").press("Enter");
    await expect(panel.getByTestId("fade_in-lights")).toHaveCount(0);

    // Closed, the row sums its fades up.
    await row.getByTestId("transitions-toggle").click();
    await expect(row.getByTestId("transitions-toggle")).toHaveText(
      "fade out 1 measure · fade in 4 beats",
    );

    await playlists.saveButton.click();
    await expect.poll(() => bodies.length).toBe(1);
    expect(bodies[0].fades).toEqual([
      {
        fade_out: {
          length: "1measure",
          curve: "linear",
          exclude: ["kick"],
          lights: "0s",
        },
        fade_in: "4beats",
      },
    ]);
  });

  test("fades follow their song through a reorder, and clear", async ({
    page,
  }) => {
    const bodies = await capturePlaylistPut(page);
    await page.route("**/api/playlists/setlist", async (route) => {
      if (route.request().method() !== "GET") return route.fallback();
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({
          name: "setlist",
          songs: ["Test Song Alpha", "Test Song Beta"],
          auto_play: [null, null],
          fades: [
            { fade_out: "2s", fade_in: null },
            { fade_out: null, fade_in: { length: "1s", exclude: ["vocals"] } },
          ],
          ambient: null,
          available_songs: [],
          song_tracks: {
            "Test Song Alpha": ["kick"],
            "Test Song Beta": ["guitar", "vocals"],
          },
        }),
      });
    });
    await playlists.selectPlaylist("setlist");
    const rows = playlists.playlistSongs();
    await expect(rows.nth(0).getByTestId("transitions-toggle")).toHaveText(
      "fade out 2s",
    );
    await expect(rows.nth(1).getByTestId("transitions-toggle")).toHaveText(
      "fade in 1s",
    );
    await rows.nth(1).getByRole("button", { name: "Move up" }).click();
    await expect(rows.first()).toContainText("Test Song Beta");

    // Read back as written: Beta's excluded track is ticked.
    await rows.first().getByTestId("transitions-toggle").click();
    const panel = rows.first().getByTestId("transitions-panel");
    await expect(panel.getByTestId("fade_in-length")).toHaveValue("1s");
    await expect(
      panel.getByTestId("fade_in-fields").getByLabel("vocals"),
    ).toBeChecked();
    // Emptying the length removes the fade.
    await panel.getByTestId("fade_in-length").fill("");
    await panel.getByTestId("fade_in-length").press("Enter");

    await playlists.saveButton.click();
    await expect.poll(() => bodies.length).toBe(1);
    expect(bodies[0].songs).toEqual(["Test Song Beta", "Test Song Alpha"]);
    expect(bodies[0].fades).toEqual([
      { fade_out: null, fade_in: null },
      { fade_out: "2s", fade_in: null },
    ]);
  });
});

test.describe("Dashboard: next up", () => {
  test("says what comes next and how it will arrive", async ({ page }) => {
    const wsId = `next-up-${test.info().parallelIndex}-${++testCounter}-${Date.now()}`;
    await page.goto(`/?wsId=${wsId}#/`);
    await expect(page.locator(".playback-card__title")).toBeVisible();

    const frame = {
      type: "playback",
      is_playing: true,
      elapsed_ms: 0,
      song_name: "Walk-in",
      song_duration_ms: 60000,
      playlist_name: "setlist",
      playlist_position: 0,
      playlist_songs: ["Walk-in", "Opener", "The Slow One"],
      tracks: [],
      available_playlists: ["all_songs", "setlist"],
      persisted_playlist_name: "setlist",
      locked: false,
      looping: true,
    };
    const nextUp = page.getByTestId("next-up");

    await sendWsMessage(page, wsId, {
      ...frame,
      next: {
        song: "Opener",
        breaks_loop: true,
        fade_out: {
          length: "1measure",
          curve: "equal_power",
          exclude: null,
          lights: null,
        },
        fade_in: null,
        auto_play_ms: null,
      },
    });
    await expect(nextUp).toHaveText(
      "Next breaks the loop: fades out over 1 measure into Opener",
    );

    await sendWsMessage(page, wsId, {
      ...frame,
      looping: false,
      next: { song: "Opener", breaks_loop: false, auto_play_ms: 2000 },
    });
    await expect(nextUp).toHaveText("Next: Opener — starts by itself after 2s");

    await sendWsMessage(page, wsId, {
      ...frame,
      looping: false,
      next: { song: "The Slow One", breaks_loop: false, auto_play_ms: null },
    });
    await expect(nextUp).toHaveText("Next: The Slow One — waits for Play");

    await sendWsMessage(page, wsId, { ...frame, next: { song: null } });
    await expect(nextUp).toHaveText("End of playlist");

    await sendWsMessage(page, wsId, { ...frame, next: null });
    await expect(nextUp).toHaveCount(0);
  });
});
