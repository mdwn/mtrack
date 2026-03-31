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

// Minimal valid tempo-guess response — well-aligned MIDI (no warning expected).
const GOOD_MIDI_TEMPO = {
  source: "midi",
  tempo: {
    start_seconds: 0.0,
    bpm: 120,
    time_signature: [4, 4],
    changes: [],
    alignment_rms_ms: 4.2,
  },
};

// Tempo-guess response with poor alignment (warning should appear).
const BAD_MIDI_TEMPO = {
  source: "midi",
  tempo: {
    start_seconds: 0.0,
    bpm: 120,
    time_signature: [4, 4],
    changes: [],
    alignment_rms_ms: 87.5,
  },
};

// Beat-grid-only response (no alignment_rms_ms — no warning expected).
const BEAT_GRID_TEMPO = {
  source: "beat_grid",
  tempo: {
    start_seconds: 0.0,
    bpm: 120,
    time_signature: [4, 4],
    changes: [],
  },
};

/** Navigate to a song's lighting page and open the TempoEditor panel. */
async function openTempoEditor(
  page: import("@playwright/test").Page,
  songName: string,
) {
  await page.goto(`/#/songs/${encodeURIComponent(songName)}/lighting`);
  await expect(page.locator("h2.song-title")).toContainText(songName);
  await page.locator(".tempo-lane-clickable").click();
  await expect(page.locator(".tempo-editor")).toBeVisible();
}

test.describe("Tempo Detection - MIDI source (Test Song Alpha)", () => {
  test.beforeEach(async ({ page }) => {
    // Test Song Alpha: has_midi=true, beat_grid=null
    await openTempoEditor(page, "Test Song Alpha");
  });

  test("shows Detect from MIDI button when song has MIDI", async ({ page }) => {
    await expect(
      page.getByRole("button", { name: /detect from midi/i }),
    ).toBeVisible();
  });

  test("clicking Detect from MIDI calls the tempo-guess API", async ({
    page,
  }) => {
    let apiCalled = false;
    await page.route("**/api/songs/*/tempo-guess", async (route) => {
      apiCalled = true;
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify(GOOD_MIDI_TEMPO),
      });
    });

    await page.getByRole("button", { name: /detect from midi/i }).click();
    await expect(page.locator(".estimated-badge")).toBeVisible();
    expect(apiCalled).toBe(true);
  });

  test("shows from MIDI badge after successful detection", async ({ page }) => {
    await page.route("**/api/songs/*/tempo-guess", async (route) => {
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify(GOOD_MIDI_TEMPO),
      });
    });

    await page.getByRole("button", { name: /detect from midi/i }).click();
    await expect(page.locator(".estimated-badge")).toContainText("from MIDI");
  });

  test("no alignment warning for well-matched MIDI", async ({ page }) => {
    await page.route("**/api/songs/*/tempo-guess", async (route) => {
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify(GOOD_MIDI_TEMPO),
      });
    });

    await page.getByRole("button", { name: /detect from midi/i }).click();
    await expect(page.locator(".estimated-badge")).toBeVisible();
    await expect(page.locator(".alignment-warn-badge")).not.toBeVisible();
  });

  test("shows alignment warning when MIDI alignment is poor", async ({
    page,
  }) => {
    await page.route("**/api/songs/*/tempo-guess", async (route) => {
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify(BAD_MIDI_TEMPO),
      });
    });

    await page.getByRole("button", { name: /detect from midi/i }).click();
    await expect(page.locator(".alignment-warn-badge")).toBeVisible();
    await expect(page.locator(".alignment-warn-badge")).toContainText(
      "MIDI may not match audio",
    );
  });

  test("alignment warning includes RMS value", async ({ page }) => {
    await page.route("**/api/songs/*/tempo-guess", async (route) => {
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify(BAD_MIDI_TEMPO),
      });
    });

    await page.getByRole("button", { name: /detect from midi/i }).click();
    // Badge should show the rounded ms value (87.5 rounds to 88)
    await expect(page.locator(".alignment-warn-badge")).toContainText("88ms");
  });

  test("no alignment warning when alignment_rms_ms is absent (no click track)", async ({
    page,
  }) => {
    // Song has no click track so the server omits alignment_rms_ms entirely.
    await page.route("**/api/songs/*/tempo-guess", async (route) => {
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({
          source: "midi",
          tempo: {
            start_seconds: 0.0,
            bpm: 120,
            time_signature: [4, 4],
            changes: [],
            // alignment_rms_ms intentionally absent
          },
        }),
      });
    });

    await page.getByRole("button", { name: /detect from midi/i }).click();
    await expect(page.locator(".estimated-badge")).toBeVisible();
    await expect(page.locator(".alignment-warn-badge")).not.toBeVisible();
  });
});

test.describe("Tempo Detection - beat grid source (Test Song Beta)", () => {
  test.beforeEach(async ({ page }) => {
    // Test Song Beta: has_midi=false, beat_grid present
    await openTempoEditor(page, "Test Song Beta");
  });

  test("no alignment warning for beat-grid-sourced tempo", async ({ page }) => {
    await page.route("**/api/songs/*/tempo-guess", async (route) => {
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify(BEAT_GRID_TEMPO),
      });
    });

    await page.getByRole("button", { name: /guess from beat grid/i }).click();
    await expect(page.locator(".estimated-badge")).toContainText(
      "estimated from beat grid",
    );
    await expect(page.locator(".alignment-warn-badge")).not.toBeVisible();
  });
});

test.describe("Tempo Detection - error and state transitions", () => {
  test.beforeEach(async ({ page }) => {
    await openTempoEditor(page, "Test Song Alpha");
  });

  test("button shows Re-detect after a successful detection", async ({
    page,
  }) => {
    await page.route("**/api/songs/*/tempo-guess", async (route) => {
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify(GOOD_MIDI_TEMPO),
      });
    });

    await page.getByRole("button", { name: /detect from midi/i }).click();
    await expect(page.locator(".estimated-badge")).toBeVisible();
    // Button label should switch to "Re-detect" once a source is set.
    await expect(
      page.getByRole("button", { name: /re-detect/i }),
    ).toBeVisible();
  });

  test("API error does not break the UI", async ({ page }) => {
    await page.route("**/api/songs/*/tempo-guess", async (route) => {
      await route.fulfill({ status: 500, body: "Internal Server Error" });
    });

    await page.getByRole("button", { name: /detect from midi/i }).click();

    // The button should recover — not be stuck in a "Loading..." state.
    await expect(
      page.getByRole("button", { name: /detect from midi/i }),
    ).toBeVisible();
    // No badge should appear on failure.
    await expect(page.locator(".estimated-badge")).not.toBeVisible();
  });
});
