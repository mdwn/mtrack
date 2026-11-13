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
import { stubPlayerService } from "../support/grpc";
import { fromBinary } from "@bufbuild/protobuf";
import { PlaylistEditorPage } from "../pages/playlist-editor.page.js";
import { SetAmbientModeRequestSchema } from "../../src/gen/player/v1/player_pb";
import { sendWatch } from "../support/watch";
import { capturePlaylistPut } from "../support/playlists";
import { PLAYBACK_STATE } from "../mock-server/test-data";

let testCounter = 0;

test.describe("Playlist editor: auto play and ambient light", () => {
  let playlists: PlaylistEditorPage;

  test.beforeEach(async ({ page }) => {
    playlists = new PlaylistEditorPage(page);
    await playlists.goto();
  });

  test("an entry's auto play and gap are saved with its song", async ({
    page,
  }) => {
    const bodies = await capturePlaylistPut(page);
    await playlists.selectPlaylist("setlist");
    const row = playlists.playlistSongs().first();
    // The gap lives in the transitions panel, shown only with auto play on.
    await playlists.openTransitions(row);
    await expect(playlists.autoPlayGap(row)).toHaveCount(0);
    await row.getByTestId("auto-play-toggle").check();
    await playlists.autoPlayGap(row).fill("2s");
    await playlists.autoPlayGap(row).press("Enter");
    await playlists.transitionsToggle(row).click();
    await expect(playlists.transitionsToggle(row)).toHaveText("2s gap");
    // A song added after keeps waiting for play.
    await playlists
      .availableSongs()
      .first()
      .locator('.btn-icon[title="Add"]')
      .click();
    await playlists.saveButton.click();
    await expect.poll(() => bodies.length).toBe(1);
    expect(bodies[0].songs).toEqual(["Test Song Alpha", "Test Song Beta"]);
    expect(bodies[0].auto_play).toEqual(["2s", null]);
    expect(bodies[0].ambient).toBeNull();
  });

  test("auto play follows its song when the order changes", async ({
    page,
  }) => {
    const bodies = await capturePlaylistPut(page);
    await playlists.selectPlaylist("setlist");
    await playlists
      .availableSongs()
      .first()
      .locator('.btn-icon[title="Add"]')
      .click();
    await playlists
      .playlistSongs()
      .nth(1)
      .getByTestId("auto-play-toggle")
      .check();
    await playlists
      .playlistSongs()
      .nth(1)
      .getByRole("button", { name: "Move up" })
      .click();
    await expect(playlists.playlistSongs().first()).toContainText(
      "Test Song Beta",
    );
    await playlists.saveButton.click();
    await expect.poll(() => bodies.length).toBe(1);
    expect(bodies[0].songs).toEqual(["Test Song Beta", "Test Song Alpha"]);
    expect(bodies[0].auto_play).toEqual([true, null]);
  });

  test("a playlist reads its settings back", async ({ page }) => {
    await page.route("**/api/playlists/setlist", async (route) => {
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({
          name: "setlist",
          songs: ["Test Song Alpha"],
          auto_play: ["3s"],
          ambient: { show: "lighting/walk_in.light", fade_in: "6s" },
          available_songs: ["Test Song Beta"],
        }),
      });
    });
    await playlists.selectPlaylist("setlist");
    const row = playlists.playlistSongs().first();
    await expect(row.getByTestId("auto-play-toggle")).toBeChecked();
    await playlists.openTransitions(row);
    await expect(playlists.autoPlayGap(row)).toHaveValue("3s");
    await expect(page.locator("#playlist-ambient-mode")).toHaveValue("custom");
    await expect(page.locator("#playlist-ambient-show")).toHaveValue(
      "lighting/walk_in.light",
    );
    await expect(page.locator("#playlist-ambient-fade_in")).toHaveValue("6s");
    await expect(playlists.saveButton).toBeDisabled();
  });

  test("a playlist can turn ambient light off, or have its own", async ({
    page,
  }) => {
    const bodies = await capturePlaylistPut(page);
    await playlists.selectPlaylist("setlist");
    const mode = page.locator("#playlist-ambient-mode");
    await expect(mode).toHaveValue("profile");
    await expect(page.locator("#playlist-ambient-show")).toHaveCount(0);

    await mode.selectOption("off");
    await playlists.saveButton.click();
    await expect.poll(() => bodies.length).toBe(1);
    expect(bodies[0].ambient).toBe(false);

    await mode.selectOption("custom");
    await page.locator("#playlist-ambient-show").fill("lighting/a.light");
    await page.locator("#playlist-ambient-show").press("Enter");
    await page.locator("#playlist-ambient-delay").fill("4s");
    await page.locator("#playlist-ambient-delay").press("Enter");
    await playlists.saveButton.click();
    await expect.poll(() => bodies.length).toBe(2);
    expect(bodies[1].ambient).toEqual({
      show: "lighting/a.light",
      delay: "4s",
    });
  });
});

test.describe("Playlist editor: an entry's ambient light", () => {
  let playlists: PlaylistEditorPage;

  test.beforeEach(async ({ page }) => {
    playlists = new PlaylistEditorPage(page);
    await playlists.goto();
  });

  test("an entry can have none, or its own, before it", async ({ page }) => {
    const bodies = await capturePlaylistPut(page);
    await playlists.selectPlaylist("setlist");
    const row = playlists.playlistSongs().first();
    await expect(playlists.transitionsToggle(row)).toHaveText("Transitions");
    await playlists.openTransitions(row);
    // Grouped by gap: the ambient light and fade in come before the song.
    const before = playlists
      .transitionsPanel(row)
      .getByTestId("transitions-before");
    await expect(before).toContainText("Before this song");
    await expect(before.getByTestId("entry-ambient")).toBeVisible();
    await expect(before.getByTestId("fade_in-fields")).toBeVisible();
    await expect(
      playlists
        .transitionsPanel(row)
        .getByTestId("transitions-after")
        .getByTestId("fade_out-fields"),
    ).toBeVisible();

    const mode = playlists.entryAmbientMode(row);
    await expect(mode).toHaveValue("playlist");
    await expect(playlists.entryAmbientField(row, "show")).toHaveCount(0);
    await mode.selectOption("off");
    await expect(playlists.saveButton).toBeEnabled();
    await playlists.transitionsToggle(row).click();
    await expect(playlists.transitionsToggle(row)).toHaveText(
      "no ambient before",
    );

    // A song added after inherits the playlist's.
    await playlists
      .availableSongs()
      .first()
      .locator('.btn-icon[title="Add"]')
      .click();
    await playlists.saveButton.click();
    await expect.poll(() => bodies.length).toBe(1);
    expect(bodies[0].entry_ambient).toEqual([false, null]);

    await playlists.openTransitions(row);
    await mode.selectOption("custom");
    await playlists.entryAmbientField(row, "show").fill("lighting/b.light");
    await playlists.entryAmbientField(row, "show").press("Enter");
    await playlists.entryAmbientField(row, "delay").fill("1s");
    await playlists.entryAmbientField(row, "delay").press("Enter");
    // Each field left empty inherits.
    await expect(playlists.entryAmbientField(row, "fade_in")).toHaveValue("");
    await expect(playlists.entryAmbientField(row, "fade_in")).toHaveAttribute(
      "placeholder",
      "playlist's",
    );
    await playlists.transitionsToggle(row).click();
    await expect(playlists.transitionsToggle(row)).toHaveText("own ambient");
    await playlists.saveButton.click();
    await expect.poll(() => bodies.length).toBe(2);
    expect(bodies[1].entry_ambient).toEqual([
      { show: "lighting/b.light", delay: "1s" },
      null,
    ]);
  });

  test("an entry's ambient light reads back and follows its song", async ({
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
          songs: ["Test Song Alpha", "Test Song Beta", "Test Song Gamma"],
          auto_play: [null, "2s", null],
          fades: [
            { fade_out: null, fade_in: null },
            { fade_out: null, fade_in: "2measures" },
            { fade_out: null, fade_in: null },
          ],
          entry_ambient: [null, false, { fade_in: "3s" }],
          ambient: null,
          available_songs: [],
        }),
      });
    });
    await playlists.selectPlaylist("setlist");
    const rows = playlists.playlistSongs();
    await expect(playlists.transitionsToggle(rows.nth(0))).toHaveText(
      "Transitions",
    );
    await expect(playlists.transitionsToggle(rows.nth(1))).toHaveText(
      "fade in 2 measures · 2s gap · no ambient before",
    );
    await expect(playlists.transitionsToggle(rows.nth(2))).toHaveText(
      "own ambient",
    );
    await expect(playlists.saveButton).toBeDisabled();

    await playlists.openTransitions(rows.nth(2));
    await expect(playlists.entryAmbientMode(rows.nth(2))).toHaveValue("custom");
    await expect(
      playlists.entryAmbientField(rows.nth(2), "fade_in"),
    ).toHaveValue("3s");
    await expect(playlists.entryAmbientField(rows.nth(2), "show")).toHaveValue(
      "",
    );

    // Up a place, then the first removed: each keeps its own.
    await rows.nth(2).getByRole("button", { name: "Move up" }).click();
    await expect(rows.nth(1)).toContainText("Test Song Gamma");
    await rows.nth(0).getByRole("button", { name: "Remove" }).click();
    await expect(rows).toHaveCount(2);
    await expect(playlists.transitionsToggle(rows.nth(0))).toHaveText(
      "own ambient",
    );

    await playlists.saveButton.click();
    await expect.poll(() => bodies.length).toBe(1);
    expect(bodies[0].songs).toEqual(["Test Song Gamma", "Test Song Beta"]);
    expect(bodies[0].auto_play).toEqual([null, "2s"]);
    expect(bodies[0].entry_ambient).toEqual([{ fade_in: "3s" }, false]);
  });
});

test.describe("Profile lighting: ambient light", () => {
  test("the profile's ambient light is saved with it", async ({ page }) => {
    const yaml = `songs: songs
profiles:
  - hostname: test-host
    dmx:
      universes: []
      lighting:
        current_venue: test-venue
`;
    await page.route("**/api/config/store", async (route) => {
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({ yaml, checksum: "one" }),
      });
    });
    let put: {
      profile: { dmx: { lighting: Record<string, unknown> } };
    } | null = null;
    await page.route("**/api/config/profiles/*", async (route) => {
      put = route.request().postDataJSON();
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({ yaml, checksum: "two" }),
      });
    });
    await page.goto("/#/lighting/groups");
    await expect(page.getByTestId("lighting-ambient")).toBeVisible();
    await page.locator("#lighting-ambient-show").fill("lighting/ambient.light");
    await page.locator("#lighting-ambient-show").press("Enter");
    await page.locator("#lighting-ambient-fade_in").fill("4s");
    await page.locator("#lighting-ambient-fade_in").press("Enter");
    await page.getByRole("button", { name: "Save", exact: true }).click();
    await expect.poll(() => put).not.toBeNull();
    expect(put!.profile.dmx.lighting.ambient).toEqual({
      show: "lighting/ambient.light",
      fade_in: "4s",
    });
    expect(put!.profile.dmx.lighting.current_venue).toBe("test-venue");
  });
});

test.describe("Dashboard: auto play and ambient light", () => {
  test("a song waiting out its gap counts down, and ambient shows", async ({
    page,
  }) => {
    const wsId = `auto-play-${test.info().parallelIndex}-${++testCounter}-${Date.now()}`;
    await page.goto(`/?wsId=${wsId}#/`);
    await expect(page.locator(".playback-card__title")).toBeVisible();
    await expect(page.getByTestId("auto-play-countdown")).toHaveCount(0);

    const frame = {
      ...PLAYBACK_STATE,
      is_playing: false,
      elapsed_ms: 0,
      song_name: "Test Song Beta",
      song_duration_ms: 180000,
      playlist_name: "setlist",
      playlist_position: 1,
      playlist_at_first: false,
      playlist_at_last: true,
      playlist_songs: ["Test Song Alpha", "Test Song Beta"],
      tracks: [],
      available_playlists: ["all_songs", "setlist"],
      persisted_playlist_name: "setlist",
      locked: false,
    };
    await sendWatch(page, wsId, {
      ...frame,
      auto_play: { song: "Test Song Beta", starts_in_ms: 2400 },
      ambient: {
        mode: "auto",
        show: "/p/lighting/ambient.light",
        state: "playing",
      },
    });
    await expect(page.getByTestId("auto-play-countdown")).toContainText(
      "Test Song Beta starts in 3s",
    );
    // "Stop cancels it": the button is there to press while nothing plays.
    await expect(
      page.getByRole("button", { name: "Stop", exact: true }),
    ).toBeEnabled();
    await expect(page.getByTestId("ambient-badge")).toContainText(
      "Ambient light",
    );
    // Without a source (an older server), the badge names none.
    await expect(page.getByTestId("ambient-source")).toHaveCount(0);

    // The entry's own settings are up: the badge says whose.
    await sendWatch(page, wsId, {
      ...frame,
      auto_play: { song: "Test Song Beta", starts_in_ms: 2400 },
      ambient: {
        mode: "auto",
        show: "/p/lighting/entry.light",
        state: "playing",
        source: "entry",
      },
    });
    await expect(page.getByTestId("ambient-badge")).toHaveText(
      "Ambient light · this song's",
    );
    await sendWatch(page, wsId, {
      ...frame,
      ambient: {
        mode: "auto",
        show: "/p/lighting/walk_in.light",
        state: "pending",
        source: "playlist",
      },
    });
    await expect(page.getByTestId("ambient-badge")).toHaveText(
      "Ambient light soon · the playlist's",
    );

    // A status read of the DMX engine that did not answer says so, rather
    // than passing for no DMX engine; the mode is not known, so no buttons.
    await sendWatch(page, wsId, {
      ...frame,
      ambient: {
        available: false,
        reason: "could not read the ambient light's state",
      },
    });
    await expect(page.getByTestId("ambient-badge")).toHaveText(
      "Ambient light: no reading",
    );
    await expect(page.getByTestId("ambient-mode")).toHaveCount(0);

    await sendWatch(page, wsId, { ...frame, is_playing: true });
    await expect(page.getByTestId("auto-play-countdown")).toHaveCount(0);
    await expect(page.getByTestId("ambient-badge")).toHaveCount(0);
  });

  test("a looping song ending shows its fade, and Stop stays live", async ({
    page,
  }) => {
    const wsId = `ending-${test.info().parallelIndex}-${++testCounter}-${Date.now()}`;
    await page.goto(`/?wsId=${wsId}#/`);
    await expect(page.locator(".playback-card__title")).toBeVisible();
    await expect(page.getByTestId("ending-fade")).toHaveCount(0);

    const frame = {
      ...PLAYBACK_STATE,
      is_playing: false,
      elapsed_ms: 0,
      song_name: "Test Song Beta",
      song_duration_ms: 180000,
      playlist_name: "setlist",
      playlist_position: 1,
      playlist_at_first: false,
      playlist_at_last: true,
      playlist_songs: ["Test Song Alpha", "Test Song Beta"],
      tracks: [],
      available_playlists: ["all_songs", "setlist"],
      persisted_playlist_name: "setlist",
      locked: false,
    };
    await sendWatch(page, wsId, {
      ...frame,
      ending: { song: "Test Song Beta", remaining_ms: 3600 },
    });
    await expect(page.getByTestId("ending-fade")).toContainText(
      "Test Song Beta is fading out (4s)",
    );
    await expect(page.getByTestId("ending-fade")).toContainText(
      "Stop again to cut.",
    );
    await expect(
      page.getByRole("button", { name: "Stop", exact: true }),
    ).toBeEnabled();

    await sendWatch(page, wsId, { ...frame, ending: null });
    await expect(page.getByTestId("ending-fade")).toHaveCount(0);
  });
});

test.describe("Dashboard: ambient light mode", () => {
  test("the mode shows, and a click sets it over gRPC", async ({ page }) => {
    await stubPlayerService(page, { reply: "framed" });

    const wsId = `ambient-mode-${test.info().parallelIndex}-${++testCounter}-${Date.now()}`;
    await page.goto(`/?wsId=${wsId}#/`);
    await expect(page.locator(".playback-card__title")).toBeVisible();

    const frame = {
      ...PLAYBACK_STATE,
      is_playing: false,
      elapsed_ms: 0,
      song_name: "Test Song Alpha",
      song_duration_ms: 180000,
      playlist_name: "setlist",
      playlist_position: 0,
      playlist_at_first: true,
      playlist_at_last: false,
      playlist_songs: ["Test Song Alpha", "Test Song Beta"],
      tracks: [],
      available_playlists: ["all_songs", "setlist"],
      persisted_playlist_name: "setlist",
      // Locked: ambient mode is a show control, open like transport.
      locked: true,
    };
    await sendWatch(page, wsId, {
      ...frame,
      ambient: { mode: "off", show: "/p/lighting/ambient.light", state: "off" },
    });
    await expect(page.getByTestId("ambient-mode")).toBeVisible();
    await expect(page.getByTestId("ambient-mode-off")).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    await expect(page.getByTestId("ambient-badge")).toHaveCount(0);

    const request = page.waitForRequest((req) =>
      req.url().includes("/SetAmbientMode"),
    );
    await page.getByTestId("ambient-mode-on").click();
    const body = (await request).postDataBuffer();
    if (!body) throw new Error("set ambient mode had no body");
    // grpc-web frames the message after a 5-byte header.
    expect(fromBinary(SetAmbientModeRequestSchema, body.subarray(5)).mode).toBe(
      "on",
    );

    // On with no show to play says so.
    await sendWatch(page, wsId, {
      ...frame,
      ambient: { mode: "on", show: null, state: "unavailable" },
    });
    await expect(page.getByTestId("ambient-mode-on")).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    await expect(page.getByTestId("ambient-badge")).toContainText(
      "No ambient show",
    );

    // No DMX engine: no control.
    await sendWatch(page, wsId, { ...frame, ambient: null });
    await expect(page.getByTestId("ambient-mode")).toHaveCount(0);
  });
});
