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
import { ALL_SONGS } from "./helpers.js";
import { playerClientAt } from "../support/player-client";

/**
 * These tests exercise the REST API directly to verify the backend
 * is serving real data from the example song repository (and, for the
 * status, the stream the web UI reads it from).
 */
test.describe("Smoke: REST API", () => {
  test("the Watch stream says the build and the hardware status", async ({
    page,
  }) => {
    // The status page is drawn from the first `hardware` event of the stream.
    await page.goto("/#/status");

    // Build info.
    await expect(
      page.locator(".card").filter({ hasText: "Build Info" }),
    ).toBeVisible();
    await expect(page.locator(".info-value").first()).not.toHaveText("");

    // Hardware info.
    await expect(
      page.locator(".subsystem-row").filter({ hasText: "Audio" }),
    ).toBeVisible();
  });

  test("ListSongs returns all example songs", async ({ baseURL }) => {
    const body = await playerClientAt(baseURL!).listSongs({ detailed: true });

    expect(body.songs.length).toBeGreaterThanOrEqual(ALL_SONGS.length);
    const songNames = body.songs.map((s) => s.name);
    for (const expected of ALL_SONGS) {
      expect(songNames).toContain(expected);
    }
  });

  test("GET /api/songs/:name returns individual song", async ({ request }) => {
    const response = await request.get(
      `/api/songs/${encodeURIComponent("Another cool song")}`,
    );
    expect(response.ok()).toBe(true);
    const body = await response.json();

    expect(body.name).toBe("Another cool song");
    expect(body.tracks).toBeDefined();
    expect(body.tracks.length).toBeGreaterThanOrEqual(1);
  });

  test("GET /api/devices/audio returns device list", async ({ request }) => {
    const response = await request.get("/api/devices/audio");
    expect(response.ok()).toBe(true);
    const body = await response.json();

    expect(Array.isArray(body)).toBe(true);
    // Should have at least one audio device on any real machine.
    expect(body.length).toBeGreaterThanOrEqual(1);

    // Each device should have expected fields.
    const device = body[0];
    expect(device.name).toBeDefined();
    expect(device.max_channels).toBeDefined();
  });

  test("ListPlaylists returns playlist list", async ({ baseURL }) => {
    const body = await playerClientAt(baseURL!).listPlaylists({
      detailed: true,
    });

    expect(body.names.length).toBeGreaterThanOrEqual(1);
    expect(body.playlists.map((p) => p.name)).toEqual(body.names);
  });

  test("GET /api/config/store returns config", async ({ request }) => {
    const response = await request.get("/api/config/store");
    expect(response.ok()).toBe(true);
    const body = await response.json();

    // Should return config content.
    expect(body).toBeDefined();
  });
});
