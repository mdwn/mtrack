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

// The shared measure/beat picker behind the section and tempo dialogs: a
// three-measure ruler you tap, over a transport row that steps and rolls over
// measure lines. Both are driven by the song's meter, not by 4/4 assumptions.

import { test, expect, type Locator, type Page } from "@playwright/test";
import { SONGS } from "../mock-server/test-data";

const SONG = "Test Song Beta";
const ENC = encodeURIComponent(SONG);

// A meter change at measure 9 — 4/4 before it, 3/4 from there on.
const SONG_YAML = `name: ${SONG}
tracks:
  - name: guitar
    file: guitar.wav
sections:
  - name: verse
    start_measure: 5
    end_measure: 8
tempo:
  bpm: 120
  time_signature: 4/4
  changes:
    - measure: 9
      time_signature: 3/4
pilot:
  track: pilot
  hints:
    - at: { measure: 6 }
      label: verse
`;

let wsCounter = 0;

/** Puts this song under the playhead: stopped at 5.0s, which on the 120 BPM
 * 4/4 fixture grid is measure 3, beat 3. */
async function openWithPlayhead(page: Page, yaml = SONG_YAML): Promise<void> {
  const wsId = `picker-${test.info().parallelIndex}-${++wsCounter}-${Date.now()}`;
  await routes(page, yaml);
  await page.goto(`/?wsId=${wsId}#/songs/${ENC}/sections`);
  await expect(page.locator(".section-timeline-editor")).toBeVisible();
  const res = await page.request.post("http://127.0.0.1:3111/test/send-ws", {
    data: {
      type: "playback",
      is_playing: false,
      elapsed_ms: 5000,
      song_name: SONG,
      song_duration_ms: 34000,
      playlist_name: "setlist",
      playlist_position: 1,
      playlist_songs: [SONG],
      tracks: [],
      available_playlists: ["setlist"],
      persisted_playlist_name: "setlist",
      locked: false,
      available_sections: [],
      _wsId: wsId,
    },
  });
  expect((await res.json()).sent).toBe(1);
  await expect(page.locator(".playhead-info__pos")).toHaveText("m3.3");
}

async function routes(page: Page, yaml = SONG_YAML) {
  await page.route(`**/api/songs/${ENC}`, async (route) => {
    if (route.request().method() !== "GET") {
      await route.fulfill({ status: 200, body: "{}" });
      return;
    }
    await route.fulfill({
      status: 200,
      contentType: "text/yaml",
      body: yaml,
    });
  });
  await page.route("**/api/songs", async (route) => {
    const data = structuredClone(SONGS) as typeof SONGS;
    for (const s of data.songs) {
      if (s.name === SONG) {
        s.has_tempo_map = true;
        s.duration_ms = 34000;
        s.duration_display = "0:34";
      }
    }
    await route.fulfill({ json: data });
  });
}

async function open(page: Page) {
  await routes(page);
  await page.goto(`/#/songs/${ENC}/sections`);
  await expect(page.locator(".section-timeline-editor")).toBeVisible();
}

/** Opens the section dialog by clicking the verse block. */
async function openSectionDialog(page: Page): Promise<Locator> {
  const block = page.locator(".section-block").first();
  const box = (await block.boundingBox())!;
  await page.mouse.click(box.x + box.width / 2, box.y + box.height / 2);
  const dialog = page.locator(".marker-dialog");
  await expect(dialog).toBeVisible();
  // The sheet slides in; measuring a ruler mid-animation gives a stale box.
  await page.waitForTimeout(200);
  return dialog;
}

/** The picker under a "Start"/"End"/"Marker" label. */
function picker(dialog: Locator, label: string): Locator {
  return dialog
    .locator(".position-picker")
    .filter({ has: dialog.page().getByLabel(`${label}: beat ruler`) });
}

/** Opens the pilot hint dialog from its marker in the second lane. */
async function openPilotDialog(page: Page): Promise<Locator> {
  await page.locator(".marker-lane").nth(1).locator(".marker").first().click();
  const dialog = page.locator(".marker-dialog");
  await expect(dialog).toBeVisible();
  await page.waitForTimeout(200);
  return dialog;
}

test.describe("Position picker", () => {
  test("tapping the ruler moves the boundary to that beat", async ({
    page,
  }) => {
    await open(page);
    const dialog = await openSectionDialog(page);
    const start = picker(dialog, "Start");
    await expect(start.locator(".pp-value")).toHaveText("m5 · b1");

    // The window shows m4–m6; tap three quarters into the middle measure,
    // which in 4/4 is beat 4.
    const measure = start.locator('.pp-measure[data-measure="5"]');
    const box = (await measure.boundingBox())!;
    await page.mouse.click(box.x + box.width * 0.78, box.y + box.height - 8);

    await expect(start.locator(".pp-value")).toHaveText("m5 · b4");
    await expect(page.locator(".range-note")).toHaveText("m5.4–8");
  });

  test("stepping a beat rolls over the measure line", async ({ page }) => {
    await open(page);
    const dialog = await openSectionDialog(page);
    const start = picker(dialog, "Start");
    await expect(start.locator(".pp-value")).toHaveText("m5 · b1");

    // Back one half-beat from the downbeat lands on the last half of the
    // previous measure, not on beat 0.
    await start.getByRole("button", { name: "Start: previous beat" }).click();
    await expect(start.locator(".pp-value")).toHaveText("m4 · b4½");

    await start.getByRole("button", { name: "Start: next beat" }).click();
    await expect(start.locator(".pp-value")).toHaveText("m5 · b1");
  });

  test("the ruler follows the meter in effect", async ({ page }) => {
    await open(page);
    const dialog = await openSectionDialog(page);
    const end = picker(dialog, "End");

    // The end sits at m8, so the window covers m6–m8, all still in 4/4.
    await expect(
      end.locator('.pp-measure[data-measure="8"] .pp-num'),
    ).toHaveCount(4);

    // Nudge the window on to reach the 3/4 measure; the ruler re-counts its
    // beats, halves included, and labels the new meter.
    await end.getByRole("button", { name: "End: later measures" }).click();
    await expect(
      end.locator('.pp-measure[data-measure="9"] .pp-num'),
    ).toHaveCount(3);
    await expect(
      end.locator('.pp-measure[data-measure="9"] .pp-tick'),
    ).toHaveCount(5);
    await expect(
      end.locator('.pp-measure[data-measure="9"] .pp-sig'),
    ).toHaveText("3/4");
  });

  test("pilot hints anchor through the same picker", async ({ page }) => {
    await open(page);
    const dialog = await openPilotDialog(page);
    const hint = picker(dialog, "Hint");
    await expect(hint.locator(".pp-value")).toHaveText("m6 · b1");
    // m6 downbeat of a 120 BPM 4/4 grid: 5 measures in, 2s each.
    await expect(hint.locator(".pp-sub")).toHaveText("0:10.000");

    await hint.getByRole("button", { name: "Hint: next beat" }).click();
    await expect(hint.locator(".pp-value")).toHaveText("m6 · b2");
  });

  test("a hint in time mode comes off the grid", async ({ page }) => {
    await open(page);
    const dialog = await openPilotDialog(page);
    const hint = picker(dialog, "Hint");

    // A hint can store a time, so the toggle decouples it: seconds steps
    // appear and the position becomes an approximation.
    await hint.getByRole("button", { name: "time", exact: true }).click();
    await expect(hint.locator(".pp-value")).toHaveText("0:10.000");
    await expect(hint.locator(".pp-chip")).toHaveCount(3);

    await hint.getByRole("button", { name: "Hint: next beat" }).click();
    await expect(hint.locator(".pp-value")).toHaveText("0:10.100");
    await expect(hint.locator(".pp-sub")).toContainText("≈ m6");

    // Back to beats and it snaps to the nearest one it can store.
    await hint.getByRole("button", { name: "beat", exact: true }).click();
    await expect(hint.locator(".pp-value")).toHaveText("m6 · b1");
  });

  test("a boundary in time mode still snaps to the grid", async ({ page }) => {
    await open(page);
    const dialog = await openSectionDialog(page);
    const start = picker(dialog, "Start");

    // One toggle drives both boundaries — mixed units in one dialog would be
    // unreadable. A section can only store a beat, so time is a unit, not a
    // freer position: the sub-line reads "=" and the beat steps stay.
    await dialog.getByRole("button", { name: "time", exact: true }).click();
    await expect(start.locator(".pp-value")).toHaveText("0:08.000");
    await expect(picker(dialog, "End").locator(".pp-value")).toHaveText(
      "0:14.000",
    );
    await expect(start.locator(".pp-sub")).toHaveText("= m5 · b1");
    await expect(start.locator(".pp-chip")).toHaveCount(2);

    // Typing a time lands on the nearest beat, and the section takes it.
    await start.getByRole("button", { name: "Start: type a position" }).click();
    const field = start.getByRole("textbox", {
      name: "Start: type a position",
    });
    await field.fill("0:08.400");
    await field.press("Enter");
    await expect(start.locator(".pp-value")).toHaveText("0:08.500");
    await expect(start.locator(".pp-sub")).toHaveText("= m5 · b2");
    await expect(page.locator(".range-note")).toHaveText("m5.2–8");
  });

  test("the playhead can be captured into a hint", async ({ page }) => {
    await openWithPlayhead(page);
    const dialog = await openPilotDialog(page);
    const hint = picker(dialog, "Hint");
    // The row reads the playhead out in both units before you take it.
    await expect(dialog.locator(".playhead-capture")).toContainText("0:05.000");
    await expect(dialog.locator(".playhead-capture")).toContainText("m3 · b3");

    await dialog.getByRole("button", { name: "Use playhead" }).click();
    await expect(hint.locator(".pp-value")).toHaveText("m3 · b3");

    // Time-anchored, the same button takes the exact time instead.
    await hint.getByRole("button", { name: "time", exact: true }).click();
    await hint.getByRole("button", { name: "Hint: next beat" }).click();
    await expect(hint.locator(".pp-value")).toHaveText("0:05.100");
    await dialog.getByRole("button", { name: "Use playhead" }).click();
    await expect(hint.locator(".pp-value")).toHaveText("0:05.000");
  });

  test("the playhead can be captured into a tempo marker", async ({ page }) => {
    await openWithPlayhead(page);
    await page
      .locator(".marker-lane")
      .first()
      .locator(".marker")
      .last()
      .click();
    const dialog = page.locator(".marker-dialog");
    await expect(dialog).toBeVisible();
    await page.waitForTimeout(200);
    const marker = picker(dialog, "Marker");
    await expect(marker.locator(".pp-value")).toHaveText("m9 · b1");

    // A tempo change can only sit on a beat, so the capture snaps to one.
    await dialog.getByRole("button", { name: "Use playhead" }).click();
    await expect(marker.locator(".pp-value")).toHaveText("m3 · b3");

    // Capturing again is a harmless no-op: the marker is already there.
    await expect(
      dialog.getByRole("button", { name: "Use playhead" }),
    ).toBeEnabled();
  });

  test("capture is refused when a change already holds that beat", async ({
    page,
  }) => {
    // A change already sits on m3 b3, where the playhead is.
    await openWithPlayhead(
      page,
      SONG_YAML.replace(
        "    - measure: 9",
        "    - measure: 3\n      beat: 3\n      bpm: 132\n    - measure: 9",
      ),
    );
    await page
      .locator(".marker-lane")
      .first()
      .locator(".marker")
      .last()
      .click();
    const dialog = page.locator(".marker-dialog");
    await expect(dialog).toBeVisible();
    await expect(picker(dialog, "Marker").locator(".pp-value")).toHaveText(
      "m9 · b1",
    );
    await expect(
      dialog.getByRole("button", { name: "Use playhead" }),
    ).toBeDisabled();
  });
});
