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
`;

async function open(page: Page) {
  await page.route(`**/api/songs/${ENC}`, async (route) => {
    if (route.request().method() !== "GET") {
      await route.fulfill({ status: 200, body: "{}" });
      return;
    }
    await route.fulfill({
      status: 200,
      contentType: "text/yaml",
      body: SONG_YAML,
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
  return dialog;
}

/** The picker under a "Start"/"End"/"Marker" label. */
function picker(dialog: Locator, label: string): Locator {
  return dialog
    .locator(".position-picker")
    .filter({ has: dialog.page().getByLabel(`${label}: beat ruler`) });
}

test.describe("Position picker", () => {
  test("tapping the ruler moves the boundary to that beat", async ({
    page,
  }) => {
    await open(page);
    const dialog = await openSectionDialog(page);
    const start = picker(dialog, "Start");
    await expect(start.locator(".pp-readout")).toHaveText("m5 · b1");

    // The window shows m4–m6; tap three quarters into the middle measure,
    // which in 4/4 is beat 4.
    const measure = start.locator('.pp-measure[data-measure="5"]');
    const box = (await measure.boundingBox())!;
    await page.mouse.click(box.x + box.width * 0.78, box.y + box.height - 8);

    await expect(start.locator(".pp-readout")).toHaveText("m5 · b4");
    await expect(page.locator(".range-note")).toHaveText("m5.4–8");
  });

  test("stepping a beat rolls over the measure line", async ({ page }) => {
    await open(page);
    const dialog = await openSectionDialog(page);
    const start = picker(dialog, "Start");
    await expect(start.locator(".pp-readout")).toHaveText("m5 · b1");

    // Back one half-beat from the downbeat lands on the last half of the
    // previous measure, not on beat 0.
    await start.getByRole("button", { name: "Start: previous beat" }).click();
    await expect(start.locator(".pp-readout")).toHaveText("m4 · b4½");

    await start.getByRole("button", { name: "Start: next beat" }).click();
    await expect(start.locator(".pp-readout")).toHaveText("m5 · b1");
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
});
