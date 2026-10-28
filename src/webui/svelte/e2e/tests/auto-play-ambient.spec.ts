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
import { fromBinary } from "@bufbuild/protobuf";
import { PlaylistEditorPage } from "../pages/playlist-editor.page.js";
import { SetAmbientModeRequestSchema } from "../../src/gen/player/v1/player_pb";

let testCounter = 0;

async function sendWsMessage(page: Page, wsId: string, msg: object) {
  await page.request.post("http://127.0.0.1:3111/test/send-ws", {
    data: { ...msg, _wsId: wsId },
  });
}

/** Captures the next PUT to the setlist, answering it as saved. */
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
    await expect(row.getByTestId("auto-play-gap")).toHaveCount(0);
    await row.getByTestId("auto-play-toggle").check();
    await row.getByTestId("auto-play-gap").fill("2s");
    await row.getByTestId("auto-play-gap").press("Enter");
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
    await expect(row.getByTestId("auto-play-gap")).toHaveValue("3s");
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
      type: "playback",
      is_playing: false,
      elapsed_ms: 0,
      song_name: "Test Song Beta",
      song_duration_ms: 180000,
      playlist_name: "setlist",
      playlist_position: 1,
      playlist_songs: ["Test Song Alpha", "Test Song Beta"],
      tracks: [],
      available_playlists: ["all_songs", "setlist"],
      persisted_playlist_name: "setlist",
      locked: false,
    };
    await sendWsMessage(page, wsId, {
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
    await expect(page.getByTestId("ambient-badge")).toContainText(
      "Ambient light",
    );

    await sendWsMessage(page, wsId, { ...frame, is_playing: true });
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
      type: "playback",
      is_playing: false,
      elapsed_ms: 0,
      song_name: "Test Song Beta",
      song_duration_ms: 180000,
      playlist_name: "setlist",
      playlist_position: 1,
      playlist_songs: ["Test Song Alpha", "Test Song Beta"],
      tracks: [],
      available_playlists: ["all_songs", "setlist"],
      persisted_playlist_name: "setlist",
      locked: false,
    };
    await sendWsMessage(page, wsId, {
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

    await sendWsMessage(page, wsId, { ...frame, ending: null });
    await expect(page.getByTestId("ending-fade")).toHaveCount(0);
  });
});

test.describe("Dashboard: ambient light mode", () => {
  test("the mode shows, and a click sets it over gRPC", async ({ page }) => {
    // A well-formed grpc-web unary reply: an empty message frame followed by
    // a trailers frame.
    const trailers = Buffer.from("grpc-status: 0\r\n");
    const grpcWebBody = Buffer.concat([
      Buffer.from([0x00, 0, 0, 0, 0]),
      Buffer.from([0x80, 0, 0, 0, trailers.length]),
      trailers,
    ]);
    await page.route("**/player.v1.PlayerService/**", async (route) => {
      await route.fulfill({
        status: 200,
        headers: { "content-type": "application/grpc-web+proto" },
        body: grpcWebBody,
      });
    });

    const wsId = `ambient-mode-${test.info().parallelIndex}-${++testCounter}-${Date.now()}`;
    await page.goto(`/?wsId=${wsId}#/`);
    await expect(page.locator(".playback-card__title")).toBeVisible();

    const frame = {
      type: "playback",
      is_playing: false,
      elapsed_ms: 0,
      song_name: "Test Song Alpha",
      song_duration_ms: 180000,
      playlist_name: "setlist",
      playlist_position: 0,
      playlist_songs: ["Test Song Alpha", "Test Song Beta"],
      tracks: [],
      available_playlists: ["all_songs", "setlist"],
      persisted_playlist_name: "setlist",
      // Locked: ambient mode is a show control, open like transport.
      locked: true,
    };
    await sendWsMessage(page, wsId, {
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
    await sendWsMessage(page, wsId, {
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
    await sendWsMessage(page, wsId, { ...frame, ambient: null });
    await expect(page.getByTestId("ambient-mode")).toHaveCount(0);
  });
});
