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

/**
 * These tests exercise the REST API directly to verify the backend
 * is serving real data from the example song repository.
 */
test.describe("Smoke: REST API", () => {
  test("GET /api/status returns build info and hardware status", async ({
    request,
  }) => {
    const response = await request.get("/api/status");
    expect(response.ok()).toBe(true);
    const body = await response.json();

    // Build info.
    expect(body.build).toBeDefined();
    expect(body.build.version).toBeDefined();

    // Hardware info.
    expect(body.hardware).toBeDefined();
    expect(body.hardware.audio).toBeDefined();
  });

  test("GET /api/songs returns all example songs", async ({ request }) => {
    const response = await request.get("/api/songs");
    expect(response.ok()).toBe(true);
    const body = await response.json();

    expect(body.songs).toBeDefined();
    expect(Array.isArray(body.songs)).toBe(true);

    const songNames = body.songs.map((s: { name: string }) => s.name);
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

  test("GET /api/playlists returns playlist list", async ({ request }) => {
    const response = await request.get("/api/playlists");
    expect(response.ok()).toBe(true);
    const body = await response.json();

    expect(Array.isArray(body)).toBe(true);
    expect(body.length).toBeGreaterThanOrEqual(1);
  });

  test("GET /api/config/store returns config", async ({ request }) => {
    const response = await request.get("/api/config/store");
    expect(response.ok()).toBe(true);
    const body = await response.json();

    // Should return config content.
    expect(body).toBeDefined();
  });
});
