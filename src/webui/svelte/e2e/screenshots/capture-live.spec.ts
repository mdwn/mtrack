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

// Live-data screenshot generator. Captures the data-rich shots — song
// browser, song detail, sections, lighting timeline, playlists, hardware
// profiles, and the bulk-import flow — against a real running mtrack.
// Run via `npm run screenshots:live` (assumes mtrack on http://localhost:8080;
// override with MTRACK_URL=...).
//
// Picks songs/playlists dynamically from /api/songs and /api/playlists so
// the spec doesn't bake in operator-specific names.

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

interface SongSummary {
  name: string;
  has_lighting?: boolean;
  sections?: unknown[];
}

interface PlaylistSummary {
  name: string;
  is_active?: boolean;
  song_count?: number;
}

async function fetchJson<T>(page: Page, urlPath: string): Promise<T> {
  const res = await page.request.get(urlPath);
  if (!res.ok()) {
    throw new Error(`GET ${urlPath} → ${res.status()}`);
  }
  return (await res.json()) as T;
}

async function pickSong(
  page: Page,
  predicate: (s: SongSummary) => boolean,
): Promise<string> {
  const data = await fetchJson<{ songs: SongSummary[] }>(page, "/api/songs");
  const match = data.songs.find(predicate) ?? data.songs[0];
  if (!match) {
    throw new Error("no songs available on the live server");
  }
  return match.name;
}

async function pickPlaylist(page: Page): Promise<string> {
  const playlists = await fetchJson<PlaylistSummary[]>(page, "/api/playlists");
  // Prefer the active playlist with the most songs, ignoring "all_songs"
  // (which renders differently and isn't representative).
  const candidates = playlists.filter((p) => p.name !== "all_songs");
  const sorted = [...candidates].sort(
    (a, b) => (b.song_count ?? 0) - (a.song_count ?? 0),
  );
  const pick = sorted.find((p) => p.is_active) ?? sorted[0] ?? playlists[0];
  if (!pick) {
    throw new Error("no playlists available on the live server");
  }
  return pick.name;
}

test.describe.configure({ mode: "serial" });

test.beforeEach(async ({ page }) => {
  await page.setViewportSize(DESKTOP);
});

test("song-browser", async ({ page }) => {
  await page.goto("/#/songs");
  await expect(page.locator(".song-row").first()).toBeVisible();
  await page.waitForTimeout(300);
  await page.screenshot({ path: path.join(DOCS_IMAGES, "song-browser.png") });
});

test("song-detail", async ({ page }) => {
  // A song with both lighting and sections gives the detail page the most
  // to render — beat grid, section markers, lighting summary.
  const song = await pickSong(
    page,
    (s) => !!s.has_lighting && (s.sections?.length ?? 0) > 0,
  );
  await page.goto(`/#/songs/${encodeURIComponent(song)}`);
  await expect(page.locator(".song-title")).toBeVisible();
  await page.waitForTimeout(400);
  await page.screenshot({ path: path.join(DOCS_IMAGES, "song-detail.png") });
});

test("song-sections", async ({ page }) => {
  const song = await pickSong(page, (s) => (s.sections?.length ?? 0) > 1);
  await page.goto(`/#/songs/${encodeURIComponent(song)}/sections`);
  await expect(page.locator(".tab.active")).toContainText("Sections");
  await page.waitForTimeout(400);
  await page.screenshot({ path: path.join(DOCS_IMAGES, "song-sections.png") });
});

test("timeline-editor", async ({ page }) => {
  // Prefer a song with sections too — those are usually the ones with
  // authored lighting cues, not just an empty .light file.
  const song = await pickSong(
    page,
    (s) => !!s.has_lighting && (s.sections?.length ?? 0) > 0,
  );
  await page.goto(`/#/songs/${encodeURIComponent(song)}/lighting`);
  await expect(page.locator(".tab.active")).toContainText("Lighting");
  // Waveform + lighting-timeline canvases need a couple of frames to settle.
  await page.waitForTimeout(700);
  await page.screenshot({
    path: path.join(DOCS_IMAGES, "timeline-editor.png"),
  });
});

test("playlist-editor", async ({ page }) => {
  const playlist = await pickPlaylist(page);
  await page.goto(`/#/playlists/${encodeURIComponent(playlist)}`);
  await expect(page.locator(".song-columns")).toBeVisible();
  await page.waitForTimeout(300);
  await page.screenshot({
    path: path.join(DOCS_IMAGES, "playlist-editor.png"),
  });
});

test("config-editor", async ({ page }) => {
  await page.goto("/#/config");
  await expect(
    page.getByRole("heading", { name: "Hardware Profiles" }),
  ).toBeVisible();
  await expect(page.locator(".profile-file-row").first()).toBeVisible();
  await page.waitForTimeout(300);
  await page.screenshot({ path: path.join(DOCS_IMAGES, "config-editor.png") });
});

test("config-editor-profile", async ({ page }) => {
  await page.goto("/#/config");
  await page.locator(".profile-file-row").first().click();
  await expect(page.getByRole("button", { name: "Back" })).toBeVisible();
  await page.waitForTimeout(400);
  await page.screenshot({
    path: path.join(DOCS_IMAGES, "config-editor-profile.png"),
  });
});

test("bulk-import-result", async ({ page }) => {
  // The file browser only exists on a real server. Open the dialog so
  // the doc shows the actual import entry point with live folder content.
  await page.goto("/#/songs");
  await page.getByRole("button", { name: /import from filesystem/i }).click();
  // Wait for the file browser to populate at least one entry.
  await expect(page.locator(".entry-list .entry").first()).toBeVisible({
    timeout: 5000,
  });
  await page.waitForTimeout(400);
  await page.screenshot({
    path: path.join(DOCS_IMAGES, "bulk-import-result.png"),
  });
});
