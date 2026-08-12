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

// The backend requires `tempo.changes` in strictly ascending (measure, beat)
// order — `TempoConfig::to_tempo_map` rejects anything else and the save comes
// back as a 400 — so the timeline editor has to keep the array sorted no
// matter which order the markers were authored in.

import { test, expect, type Page } from "@playwright/test";
import { parse } from "yaml";
import { SONGS } from "../mock-server/test-data";

const SONG = "Test Song Beta";
// aria-labels come from the i18n bundle; match them exactly so "Measure
// (1-indexed)" does not also pick up "Beat within measure (optional)".
const MEASURE = "Measure (1-indexed)";
const ENC = encodeURIComponent(SONG);

// One late tempo change, so anything added earlier has to sort ahead of it.
const SONG_YAML = `name: ${SONG}
tracks:
  - name: guitar
    file: guitar.wav
  - name: vocals
    file: vocals.wav
sections:
  - name: verse
    start_measure: 1
    end_measure: 4
tempo:
  bpm: 120
  time_signature: 4/4
  changes:
    - measure: 13
      bpm: 140
`;

interface SavedTempo {
  bpm: number;
  changes?: { measure: number; beat?: number; bpm?: number }[];
}

/** The tempo block of the YAML the UI PUTs when Save is clicked. */
async function saveAndReadTempo(page: Page): Promise<SavedTempo> {
  const request = page.waitForRequest(
    (req) => req.url().includes(`/api/songs/${ENC}`) && req.method() === "PUT",
  );
  await page.getByRole("button", { name: "Save" }).first().click();
  const body = (await request).postData() ?? "";
  return (parse(body) as { tempo: SavedTempo }).tempo;
}

async function openSections(page: Page) {
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
  // The fixture's beat grid only covers 16 measures; serve the list from the
  // fixture with the duration trimmed to match, so lane pixels map onto
  // measures instead of 4 minutes of empty timeline.
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
  await expect(
    page.locator(".marker-lane").first().locator(".marker").first(),
  ).toBeVisible();
  const fit = page.locator('button[title="Fit to view"]');
  if (await fit.count()) await fit.click();
}

/** Taps empty tempo-lane space at `fraction` of the song's length. */
async function tapTempoLane(page: Page, fraction: number) {
  const lane = page.locator(".marker-lane").first().locator(".lane-content");
  const box = (await lane.boundingBox())!;
  await lane.click({
    position: { x: box.width * fraction, y: box.height / 2 },
  });
  await expect(page.locator(".marker-dialog")).toBeVisible();
}

test.describe("Tempo marker ordering", () => {
  test("a change added before an existing one is saved in order", async ({
    page,
  }) => {
    await openSections(page);
    // Roughly a quarter in — measure 5 of 16 — well ahead of the measure-13
    // change that is already in the map.
    await tapTempoLane(page, 0.25);
    const added = parseInt(
      (await page
        .locator(".marker-dialog")
        .getByRole("textbox", { name: MEASURE, exact: true })
        .inputValue()) || "0",
    );
    expect(added).toBeGreaterThan(1);
    expect(added).toBeLessThan(13);

    await page.locator(".marker-dialog .done-btn").click();
    const tempo = await saveAndReadTempo(page);
    expect(tempo.changes?.map((c) => c.measure)).toEqual([added, 13]);
  });

  test("moving a change past a later one keeps the dialog on it", async ({
    page,
  }) => {
    await openSections(page);
    await tapTempoLane(page, 0.25);
    const dialog = page.locator(".marker-dialog");
    // The new change carries the tempo in effect where it was dropped (120),
    // the existing measure-13 one runs at 140.
    await expect(
      dialog.getByRole("textbox", { name: "BPM", exact: true }),
    ).toHaveValue("120");

    await dialog
      .getByRole("textbox", { name: MEASURE, exact: true })
      .fill("15");
    await dialog
      .getByRole("textbox", { name: MEASURE, exact: true })
      .press("Enter");

    // Sorting moved it behind the measure-13 change; the dialog must follow
    // rather than silently start editing that one.
    await expect(
      dialog.getByRole("textbox", { name: MEASURE, exact: true }),
    ).toHaveValue("15");
    await expect(
      dialog.getByRole("textbox", { name: "BPM", exact: true }),
    ).toHaveValue("120");

    await dialog.locator(".done-btn").click();
    const tempo = await saveAndReadTempo(page);
    expect(tempo.changes?.map((c) => c.measure)).toEqual([13, 15]);
    expect(tempo.changes?.map((c) => c.bpm)).toEqual([140, 120]);
  });

  test("a change cannot be moved onto an occupied position", async ({
    page,
  }) => {
    await openSections(page);
    await tapTempoLane(page, 0.25);
    const dialog = page.locator(".marker-dialog");
    const measure = dialog.getByRole("textbox", { name: MEASURE, exact: true });
    const before = await measure.inputValue();

    await measure.fill("13");
    await measure.press("Enter");

    // Refused: two changes on one beat are rejected by the backend the same
    // way a reversed pair is.
    await expect(measure).toHaveValue(before);
    await dialog.locator(".done-btn").click();
    const tempo = await saveAndReadTempo(page);
    expect(tempo.changes).toHaveLength(2);
  });
});
