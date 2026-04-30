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

// Documentation screenshot generator (mock-server backed).
//
// Captures shots that need a controlled WebSocket state — the lock chip,
// a fake "now playing" song name, a fixed elapsed time. Mock-server
// fixtures are deterministic, so these shots stay byte-stable across
// machines. Run via `npm run screenshots:mock`.
//
// Data-rich shots (song browser, song detail, hardware profiles, etc.)
// live in capture-live.spec.ts and target a real running mtrack instance.

import { test, expect, type Page } from "@playwright/test";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { mkdirSync } from "node:fs";

const __dirname = path.dirname(fileURLToPath(import.meta.url));

const DOCS_IMAGES = path.resolve(
  __dirname,
  "..",
  "..",
  "..",
  "..",
  "..",
  "docs",
  "src",
  "images",
);

mkdirSync(DOCS_IMAGES, { recursive: true });

const DESKTOP = { width: 1280, height: 800 };

interface PlaybackState {
  type: "playback";
  is_playing: boolean;
  elapsed_ms: number;
  song_name: string;
  song_duration_ms: number;
  playlist_name: string;
  playlist_position: number;
  playlist_songs: string[];
  tracks: { name: string; output_channels: number[] }[];
  available_playlists: string[];
  persisted_playlist_name: string;
  locked: boolean;
  available_sections?: {
    name: string;
    start_measure: number;
    end_measure: number;
  }[];
  active_section?: { name: string; start_ms: number; end_ms: number } | null;
}

const DEFAULT_PLAYBACK: PlaybackState = {
  type: "playback",
  is_playing: false,
  elapsed_ms: 0,
  song_name: "Test Song Alpha",
  song_duration_ms: 180000,
  playlist_name: "setlist",
  playlist_position: 0,
  playlist_songs: ["Test Song Alpha", "Test Song Beta"],
  tracks: [
    { name: "kick", output_channels: [0, 1] },
    { name: "snare", output_channels: [2, 3] },
    { name: "bass", output_channels: [4, 5] },
  ],
  available_playlists: ["all_songs", "setlist"],
  persisted_playlist_name: "setlist",
  locked: false,
};

async function pushWs(
  page: Page,
  wsId: string,
  msg: Record<string, unknown>,
): Promise<void> {
  await page.request.post("http://127.0.0.1:3111/test/send-ws", {
    data: { ...msg, _wsId: wsId },
  });
}

/** Capture a section of the topnav so we can show the lock chip in isolation. */
async function topnavOnly(page: Page, file: string) {
  const nav = page.locator(".topnav");
  await nav.waitFor({ state: "visible" });
  await page.screenshot({
    path: path.join(DOCS_IMAGES, file),
    clip: await (async () => {
      const box = await nav.boundingBox();
      if (!box) throw new Error("topnav has no bounding box");
      return {
        x: 0,
        y: 0,
        width: box.width,
        height: box.height + 24, // include the LIVE-locked stripe if visible
      };
    })(),
  });
}

let counter = 0;
function freshWsId(label: string): string {
  return `screenshot-${label}-${++counter}-${Date.now()}`;
}

test.describe.configure({ mode: "serial" });

test.beforeEach(async ({ page }) => {
  await page.setViewportSize(DESKTOP);
});

test("dashboard", async ({ page }) => {
  const wsId = freshWsId("dashboard");
  await page.goto(`/?wsId=${wsId}#/`);
  // Wait for the mock server's initial state burst (last burst at 200ms) to settle.
  await page.waitForTimeout(300);
  await pushWs(page, wsId, {
    ...DEFAULT_PLAYBACK,
    is_playing: true,
    elapsed_ms: 78000,
    song_name: "Death is a Fine Companion",
    song_duration_ms: 254000,
  });
  await expect(page.locator(".playback-card__title")).toContainText(
    "Death is a Fine Companion",
  );
  // Give the topnav progress bar a frame to render.
  await page.waitForTimeout(200);
  await page.screenshot({ path: path.join(DOCS_IMAGES, "dashboard.png") });
});

test("nav-unlocked", async ({ page }) => {
  const wsId = freshWsId("unlocked");
  await page.goto(`/?wsId=${wsId}#/`);
  await page.waitForTimeout(300);
  await pushWs(page, wsId, { ...DEFAULT_PLAYBACK, locked: false });
  await expect(page.locator(".topnav")).toBeVisible();
  await page.waitForTimeout(150);
  await topnavOnly(page, "nav-unlocked.png");
});

test("nav-locked", async ({ page }) => {
  const wsId = freshWsId("locked");
  await page.goto(`/?wsId=${wsId}#/`);
  await page.waitForTimeout(300);
  await pushWs(page, wsId, { ...DEFAULT_PLAYBACK, locked: true });
  await expect(page.locator(".topnav__lock--locked")).toBeVisible();
  await expect(page.locator(".live-stripe")).toBeVisible();
  await page.waitForTimeout(150);
  await topnavOnly(page, "nav-locked.png");
});

test("timeline-playing", async ({ page }) => {
  const wsId = freshWsId("timeline-playing");
  await page.goto(`/?wsId=${wsId}#/songs/Test%20Song%20Alpha/lighting`);
  await page.waitForTimeout(300);
  await pushWs(page, wsId, {
    ...DEFAULT_PLAYBACK,
    is_playing: true,
    elapsed_ms: 42000,
  });
  await expect(page.locator(".tab.active")).toContainText("Lighting");
  await page.waitForTimeout(500);
  await page.screenshot({
    path: path.join(DOCS_IMAGES, "timeline-playing.png"),
  });
});

test("status-page", async ({ page }) => {
  await page.goto("/#/status");
  await expect(page.locator(".status-page")).toBeVisible();
  // Wait for the polled /api/status fetch.
  await expect(page.locator(".subsystem-row").first()).toBeVisible();
  await page.waitForTimeout(200);
  await page.screenshot({ path: path.join(DOCS_IMAGES, "status-page.png") });
});
