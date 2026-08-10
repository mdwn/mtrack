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

// --- Section-timeline editor (tempo + pilot layers) -----------------------
// Route-mocks a single richly-authored song on top of the shared mock server
// so the tempo/pilot/section layers are populated and deterministic.

const STE_SONG = "Test Song Beta";
const STE_ENC = encodeURIComponent(STE_SONG);

const STE_YAML = `name: ${STE_SONG}
tracks:
  - name: guitar
    file: guitar.wav
  - name: vocals
    file: vocals.wav
  - name: bass
    file: bass.wav
loop_playback: true
sections:
  - name: verse
    start_measure: 1
    end_measure: 4
  - name: chorus
    start_measure: 5
    end_measure: 8
  - name: bridge
    start_measure: 9
    end_measure: 12
  - name: outro
    start_measure: 13
    end_measure: 16
tempo:
  bpm: 120
  time_signature: 4/4
  start: 0
  changes:
    - measure: 5
      bpm: 132
    - measure: 9
      time_signature: 3/4
    - measure: 13
      bpm: 120
      transition:
        beats: 8
pilot:
  track: pilot
  hints:
    - at: { measure: 3 }
      label: verse
    - at: { measure: 7, beat: 1 }
      label: big chorus
      file: chorus_cue.wav
      align: end
      offset: -1500
    - at: { measure: 11 }
      label: solo
`;

// Deterministic pseudo-waveform (no RNG so PNGs stay byte-stable).
function stePeaks(seed: number, n = 96): number[] {
  const out: number[] = [];
  for (let i = 0; i < n; i++) {
    const t = i / n;
    const env =
      0.25 +
      0.7 *
        Math.abs(
          Math.sin(t * Math.PI * (2 + seed)) *
            Math.cos(t * Math.PI * (1 + seed * 0.5)),
        );
    out.push(Math.min(1, env));
  }
  return out;
}

const STE_WAVEFORM = {
  song_name: STE_SONG,
  tracks: [
    { name: "guitar", peaks: stePeaks(1) },
    { name: "vocals", peaks: stePeaks(2) },
    { name: "bass", peaks: stePeaks(3) },
  ],
};

async function steInstallRoutes(page: Page, yaml: string = STE_YAML) {
  // The song's raw YAML (the editor parses tempo/pilot/sections from it).
  await page.route(`**/api/songs/${STE_ENC}`, async (route) => {
    await route.fulfill({
      status: 200,
      contentType: "text/yaml",
      body: yaml,
    });
  });
  // A fuller waveform so the lanes look like a real song.
  await page.route(`**/api/songs/${STE_ENC}/waveform`, async (route) => {
    await route.fulfill({ json: STE_WAVEFORM });
  });
  // Trim the reported duration to the authored region so "Fit" fills the width.
  await page.route("**/api/songs", async (route) => {
    const resp = await route.fetch();
    const data = await resp.json();
    for (const s of data.songs ?? []) {
      if (s.name === STE_SONG) {
        s.has_tempo_map = true;
        s.duration_ms = 34000;
        s.duration_display = "0:34";
      }
    }
    await route.fulfill({ response: resp, json: data });
  });
}

async function steOpen(page: Page, yaml: string = STE_YAML) {
  await steInstallRoutes(page, yaml);
  await page.goto(`/#/songs/${STE_ENC}/sections`);
  await expect(page.locator(".section-timeline-editor")).toBeVisible();
  await expect(
    page.locator(".marker-lane").first().locator(".marker").first(),
  ).toBeVisible();
  const fit = page.locator('button[title="Fit to view"]');
  if (await fit.count()) await fit.click();
  await page.waitForTimeout(300);
}

test("section-timeline-editor", async ({ page }) => {
  await steOpen(page);
  await page.locator(".section-timeline-editor").screenshot({
    path: path.join(DOCS_IMAGES, "section-timeline-editor.png"),
  });
});

test("section-timeline-tempo-dialog", async ({ page }) => {
  await steOpen(page);
  await page.locator(".marker-lane").first().locator(".marker").last().click();
  await expect(page.locator(".marker-dialog").first()).toBeVisible();
  await page.waitForTimeout(200);
  await page.screenshot({
    path: path.join(DOCS_IMAGES, "section-timeline-tempo-dialog.png"),
  });
});

test("section-timeline-pilot-dialog", async ({ page }) => {
  await steOpen(page);
  await page.locator(".marker-lane").nth(1).locator(".marker").first().click();
  await expect(page.locator(".marker-dialog").first()).toBeVisible();
  await page.waitForTimeout(200);
  await page.screenshot({
    path: path.join(DOCS_IMAGES, "section-timeline-pilot-dialog.png"),
  });
});

// --- Metronome feel (accents, subdivisions, claves, feel changes) ---------
// The same song with a `metronome:` block, so the tempo markers carry feel
// and the dialog shows the accent pads / subdivision picker.

const MET_YAML = `${STE_YAML}metronome:
  track: click
  accents: [3, 1, 2, 1]
  subdivision: 2
  changes:
    - measure: 5
      accents: [3, 0, 2, 0]
    - measure: 9
      subdivision: 3
    - measure: 13
      subdivision: son
`;

test("metronome-feel-timeline", async ({ page }) => {
  await steOpen(page, MET_YAML);
  await page.locator(".section-timeline-editor").screenshot({
    path: path.join(DOCS_IMAGES, "metronome-feel-timeline.png"),
  });
});

test("metronome-feel-dialog", async ({ page }) => {
  // Taller than DESKTOP so the whole sheet fits — the subdivision picker
  // runs to a second row with the claves.
  await page.setViewportSize({ width: 1280, height: 1100 });
  await steOpen(page, MET_YAML);
  // The base tempo marker: song-level accents and subdivision.
  await page.locator(".marker-lane").first().locator(".marker").first().click();
  await expect(page.locator(".marker-dialog").first()).toBeVisible();
  const dialog = page.locator(".marker-dialog").first();
  await expect(dialog.locator(".accent-pads")).toBeVisible();
  await expect(dialog.locator(".subdiv-options")).toBeVisible();
  await page.waitForTimeout(200);
  await page.screenshot({
    path: path.join(DOCS_IMAGES, "metronome-feel-dialog.png"),
  });
});

test("metronome-feel-change-dialog", async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 1100 });
  await steOpen(page, MET_YAML);
  // The measure-13 marker: a tempo change (with an 8-beat transition) that
  // also carries a feel change, so the per-aspect toggles are all visible.
  await page.locator(".marker-lane").first().locator(".marker").nth(3).click();
  const changeDialog = page.locator(".marker-dialog").first();
  await expect(changeDialog).toBeVisible();
  await expect(changeDialog.locator(".subdiv-options")).toBeVisible();
  await page.waitForTimeout(200);
  await page.screenshot({
    path: path.join(DOCS_IMAGES, "metronome-feel-change-dialog.png"),
  });
});

// The beat dots are ~8px; capture them at 3x so the accent / half / silent
// styling is legible in the docs.
test.describe(() => {
  test.use({ deviceScaleFactor: 3 });

  test("metronome-visual-click", async ({ page }) => {
    const wsId = freshWsId("visual-click");
    await page.goto(`/?wsId=${wsId}#/`);
    await page.waitForTimeout(300);
    // 4/4 at 120 BPM, one measure of dots: accent / silent / half / normal.
    // Elapsed sits on beat 3, the half accent, so the flash shows its style.
    await pushWs(page, wsId, {
      ...DEFAULT_PLAYBACK,
      is_playing: true,
      elapsed_ms: 1000,
      song_name: "Death is a Fine Companion",
      song_duration_ms: 254000,
      beat_grid: {
        beats: [0, 0.5, 1.0, 1.5],
        measure_starts: [0],
        accent_levels: [3, 0, 2, 1],
      },
    });
    await expect(page.locator(".beat-dot")).toHaveCount(4);
    await expect(page.locator(".beat-dot").nth(2)).toHaveClass(
      /beat-dot--half/,
    );
    await page.waitForTimeout(200);
    // The meta row spans the whole card; clip to its left end, where the
    // text and the dots are, so the docs image isn't mostly whitespace.
    const box = await page.locator(".playback-card__meta").boundingBox();
    if (!box) throw new Error("playback-card__meta has no bounding box");
    await page.screenshot({
      path: path.join(DOCS_IMAGES, "metronome-visual-click.png"),
      clip: {
        x: box.x - 10,
        y: box.y - 10,
        width: Math.min(box.width, 330),
        height: box.height + 20,
      },
    });
  });
});
