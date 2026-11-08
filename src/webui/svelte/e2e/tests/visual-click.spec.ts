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
import { sendWs } from "../support/ws";
import { playbackState } from "../support/playback";

let testCounter = 0;

/** A 7/8 grid: eighth = 0.25s, seven beats per measure, two measures. */
function sevenEightGrid() {
  const beats: number[] = [];
  const measure_starts: number[] = [];
  for (let m = 0; m < 2; m++) {
    measure_starts.push(beats.length);
    for (let b = 0; b < 7; b++) {
      beats.push((m * 7 + b) * 0.25);
    }
  }
  return { beats, measure_starts };
}

test.describe("Visual click", () => {
  let wsId: string;

  test.beforeEach(async ({ page }) => {
    // The worker index keeps ids unique across parallel workers, whose
    // per-worker counters can otherwise collide within the same millisecond.
    wsId = `visual-click-${test.info().parallelIndex}-${++testCounter}-${Date.now()}`;
    await page.goto(`/?wsId=${wsId}#/`);
    await expect(page.locator(".playback-card__title")).toContainText(
      "Test Song Alpha",
    );
  });

  test("renders one dot per beat of the current meter (7/8)", async ({
    page,
  }) => {
    await sendWs(page, wsId, playbackState({ beat_grid: sevenEightGrid() }));

    await expect(page.locator(".beat-dot")).toHaveCount(7);
  });

  test("active dot follows the playhead", async ({ page }) => {
    // Elapsed 0.5s in the 7/8 grid = third eighth note (index 2).
    await sendWs(
      page,
      wsId,
      playbackState({ beat_grid: sevenEightGrid(), elapsed_ms: 500 }),
    );

    await expect(page.locator(".beat-dot--active")).toHaveCount(1);
    const dots = page.locator(".beat-dot");
    await expect(dots.nth(2)).toHaveClass(/beat-dot--active/);
  });

  test("downbeat dot is marked as accent", async ({ page }) => {
    await sendWs(
      page,
      wsId,
      playbackState({ beat_grid: sevenEightGrid(), elapsed_ms: 0 }),
    );

    const dots = page.locator(".beat-dot");
    await expect(dots.nth(0)).toHaveClass(/beat-dot--accent/);
    await expect(dots.nth(0)).toHaveClass(/beat-dot--active/);
  });

  test("flash pulses while playing", async ({ page }) => {
    await sendWs(
      page,
      wsId,
      playbackState({
        is_playing: true,
        beat_grid: sevenEightGrid(),
        elapsed_ms: 250,
      }),
    );

    await expect(page.locator(".beat-flash")).toBeVisible();
    await expect(page.locator(".beat-flash")).not.toHaveClass(
      /beat-flash--off/,
    );
  });

  test("hidden when no beat grid is available", async ({ page }) => {
    await sendWs(page, wsId, playbackState({ beat_grid: null }));

    await expect(page.locator(".beat-indicator")).toHaveCount(0);
  });

  test("resolved accent levels style the dots", async ({ page }) => {
    // 4/4 grid with per-beat levels: accent, silent, normal, half.
    const beats = [0, 0.5, 1.0, 1.5];
    await sendWs(
      page,
      wsId,
      playbackState({
        beat_grid: {
          beats,
          measure_starts: [0],
          accent_levels: [3, 0, 1, 2],
        },
      }),
    );

    const dots = page.locator(".beat-dot");
    await expect(dots).toHaveCount(4);
    await expect(dots.nth(0)).toHaveClass(/beat-dot--accent/);
    await expect(dots.nth(1)).toHaveClass(/beat-dot--silent/);
    await expect(dots.nth(1)).not.toHaveClass(/beat-dot--accent/);
    await expect(dots.nth(2)).not.toHaveClass(
      /beat-dot--accent|beat-dot--half|beat-dot--silent/,
    );
    await expect(dots.nth(3)).toHaveClass(/beat-dot--half/);
  });

  test("half-accent beat flashes with the half style", async ({ page }) => {
    await sendWs(
      page,
      wsId,
      playbackState({
        is_playing: true,
        elapsed_ms: 1500,
        beat_grid: {
          beats: [0, 0.5, 1.0, 1.5],
          measure_starts: [0],
          accent_levels: [3, 0, 1, 2],
        },
      }),
    );

    await expect(page.locator(".beat-flash")).toHaveClass(/beat-flash--half/);
  });

  test("silent beat does not flash", async ({ page }) => {
    await sendWs(
      page,
      wsId,
      playbackState({
        is_playing: true,
        elapsed_ms: 500,
        beat_grid: {
          beats: [0, 0.5, 1.0, 1.5],
          measure_starts: [0],
          accent_levels: [3, 0, 1, 2],
        },
      }),
    );

    await expect(page.locator(".beat-flash")).toHaveClass(/beat-flash--off/);
  });
});
