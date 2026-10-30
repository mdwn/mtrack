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

  test("next and previous end a looping song at the ends of the list", async ({
    page,
  }) => {
    // #579: they were disabled there, though pressing them ends the song
    // over its fade out.
    const wsId = `loop-end-${test.info().parallelIndex}-${++testCounter}-${Date.now()}`;
    await page.goto(`/?wsId=${wsId}#/`);
    await expect(page.locator(".playback-card__title")).toBeVisible();

    const nextUp = page.getByTestId("next-up");
    const prevButton = page.getByRole("button", { name: "Prev", exact: true });
    const nextButton = page.getByRole("button", { name: "Next", exact: true });
    const fade = (length: string) => ({
      length,
      curve: "equal_power",
      exclude: null,
      lights: null,
    });
    const last = {
      type: "playback",
      is_playing: true,
      elapsed_ms: 0,
      song_name: "Daegrima",
      song_duration_ms: 60000,
      playlist_name: "setlist",
      playlist_position: 1,
      playlist_songs: ["Opener", "Daegrima"],
      tracks: [],
      available_playlists: ["all_songs", "setlist"],
      persisted_playlist_name: "setlist",
      locked: false,
      looping: true,
    };

    // The last entry, with a fade out: Next ends it.
    await sendWsMessage(page, wsId, {
      ...last,
      next: { song: null, ends_loop: true, fade_out: fade("2measures") },
    });
    await expect(nextUp).toHaveText(
      "Next ends the loop: fades out over 2 measures",
    );
    await expect(nextButton).toBeEnabled();

    // Without one, the press would cut: still off.
    await sendWsMessage(page, wsId, {
      ...last,
      next: { song: null, ends_loop: true, fade_out: null },
    });
    await expect(nextUp).toHaveText("End of playlist");
    await expect(nextButton).toBeDisabled();

    // The first entry: Previous ends it, and says so.
    const first = {
      ...last,
      song_name: "Throneless Postlude",
      playlist_position: 0,
      playlist_songs: ["Throneless Postlude", "Thunar Prelude"],
    };
    await sendWsMessage(page, wsId, {
      ...first,
      next: {
        song: "Thunar Prelude",
        breaks_loop: true,
        fade_out: fade("4s"),
        fade_in: null,
        auto_play_ms: null,
      },
    });
    await expect(prevButton).toBeEnabled();
    await expect(prevButton).toHaveAttribute(
      "title",
      "Previous ends the loop: fades out over 4s (Left Arrow)",
    );
    await sendWsMessage(page, wsId, {
      ...first,
      next: {
        song: "Thunar Prelude",
        breaks_loop: true,
        fade_out: null,
        fade_in: null,
        auto_play_ms: null,
      },
    });
    await expect(prevButton).toBeDisabled();

    // Stopped, the ends of the list are the ends again.
    await sendWsMessage(page, wsId, {
      ...last,
      is_playing: false,
      next: { song: null },
    });
    await expect(nextButton).toBeDisabled();
  });

  test("a song taken off the list is none of its rows", async ({ page }) => {
    // #576: an edit took the looping song playing off the list, which sits
    // before the entry that took its place. That row showed as playing, and
    // Next, which lands on it, was off on the last entry.
    const wsId = `off-list-${test.info().parallelIndex}-${++testCounter}-${Date.now()}`;
    await page.goto(`/?wsId=${wsId}#/`);
    await expect(page.locator(".playback-card__title")).toBeVisible();

    const nextButton = page.getByRole("button", { name: "Next", exact: true });
    const rows = page.locator(".playlist-card__row");
    const frame = {
      type: "playback",
      is_playing: true,
      elapsed_ms: 0,
      song_name: "Daegrima",
      song_duration_ms: 60000,
      playlist_name: "setlist",
      playlist_position: 1,
      playlist_off_list: true,
      playlist_songs: ["Opener", "Encore"],
      tracks: [],
      available_playlists: ["all_songs", "setlist"],
      persisted_playlist_name: "setlist",
      locked: false,
      looping: true,
      next: {
        song: "Encore",
        breaks_loop: true,
        fade_out: null,
        fade_in: null,
        auto_play_ms: null,
      },
    };
    await sendWsMessage(page, wsId, frame);
    await expect(rows).toHaveCount(2);
    await expect(page.locator(".playlist-card__row--active")).toHaveCount(0);
    await expect(page.locator(".playlist-card__playing-dot")).toHaveCount(0);
    await expect(nextButton).toBeEnabled();

    // On the list again, the row is the song's.
    await sendWsMessage(page, wsId, {
      ...frame,
      song_name: "Encore",
      playlist_off_list: false,
      looping: false,
      next: { song: null },
    });
    await expect(rows.nth(1)).toHaveClass(/playlist-card__row--active/);
    await expect(nextButton).toBeDisabled();
  });

  test("Previous stays on off the list before the first entry", async ({
    page,
  }) => {
    // #579: off the list at position 0, Previous lands on the first entry,
    // but the dashboard and the mini player took it as the start of the
    // list and turned Previous off.
    const wsId = `off-list-first-${test.info().parallelIndex}-${++testCounter}-${Date.now()}`;
    await page.goto(`/?wsId=${wsId}#/`);
    await expect(page.locator(".playback-card__title")).toBeVisible();

    const frame = {
      type: "playback",
      is_playing: false,
      elapsed_ms: 0,
      song_name: "Daegrima",
      song_duration_ms: 60000,
      playlist_name: "setlist",
      playlist_position: 0,
      playlist_off_list: true,
      playlist_songs: ["Opener", "Encore"],
      tracks: [],
      available_playlists: ["all_songs", "setlist"],
      persisted_playlist_name: "setlist",
      locked: false,
      looping: true,
      next: { song: "Opener", breaks_loop: false, auto_play_ms: null },
    };
    const prevButton = page.getByRole("button", { name: "Prev", exact: true });
    await sendWsMessage(page, wsId, frame);
    await expect(prevButton).toBeEnabled();
    // Playing, with no fade out of its own to end on: still a move.
    await sendWsMessage(page, wsId, {
      ...frame,
      is_playing: true,
      next: {
        song: "Opener",
        breaks_loop: true,
        fade_out: null,
        fade_in: null,
        auto_play_ms: null,
      },
    });
    await expect(prevButton).toBeEnabled();

    // The mini player, on a narrow screen away from the dashboard, agrees.
    await page.setViewportSize({ width: 600, height: 800 });
    await page.evaluate(() => (window.location.hash = "#/status"));
    const miniPrev = page
      .locator(".miniplayer")
      .getByRole("button", { name: "Prev", exact: true });
    await expect(miniPrev).toBeEnabled();

    // On the first entry, it is the start of the list again.
    await sendWsMessage(page, wsId, {
      ...frame,
      song_name: "Opener",
      playlist_off_list: false,
      looping: false,
      next: { song: "Encore", breaks_loop: false, auto_play_ms: null },
    });
    await expect(miniPrev).toBeDisabled();
  });
});
