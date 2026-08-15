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
import { parse } from "yaml";

// A light show whose tempo map (128 BPM, one change at m5) differs from the
// song timeline's (132 BPM) so each import direction is observable.
const LIGHT_SHOW_WITH_TEMPO = [
  "tempo {",
  "    start: 0s",
  "    bpm: 128",
  "    time_signature: 4/4",
  "    changes: [",
  "        @5/1 { bpm: 90 }",
  "    ]",
  "}",
  "",
  'show "Main" {',
  "    @00:00.000",
  '    front: static, color: "blue", duration: 5s',
  "}",
  "",
].join("\n");

/**
 * A show whose tempo map holds what the song format will not: changes out of
 * order, two changes on one position, and an absolute anchor. Nothing here is
 * invalid — the DSL sorts at playback and accepts either anchor form — but
 * `to_tempo_map` refuses all three, so the import has to resolve them.
 */
const LIGHT_SHOW_MESSY_TEMPO = [
  "tempo {",
  "    start: 0s",
  "    bpm: 128",
  "    time_signature: 4/4",
  "    changes: [",
  "        @20/1 { bpm: 100 }",
  "        @8/1 { bpm: 90 }",
  "        @8/1 { bpm: 95 }",
  "    ]",
  "}",
  "",
].join("\n");

/** A show anchoring a tempo change in time rather than to a measure. */
const LIGHT_SHOW_ABSOLUTE_TEMPO = [
  "tempo {",
  "    start: 0s",
  "    bpm: 128",
  "    time_signature: 4/4",
  "    changes: [",
  "        @00:02.000 { bpm: 140 }",
  "    ]",
  "}",
  "",
].join("\n");

test.describe("Tempo import between timeline and light shows", () => {
  /** The show body served for the current test; set one before navigating. */
  let showBody = LIGHT_SHOW_WITH_TEMPO;

  test.beforeEach(async ({ page }) => {
    showBody = LIGHT_SHOW_WITH_TEMPO;
    // Give Test Song Beta (the mock song with a beat grid) a song-level
    // tempo map and a lighting file on top of the mock server's config.
    // The lighting files list lives on the songs-list JSON; the tempo
    // block lives in the song's YAML config.
    await page.route("**/api/songs", async (route) => {
      if (route.request().method() !== "GET") return route.continue();
      const res = await route.fetch();
      const data = await res.json();
      for (const s of data.songs ?? []) {
        if (s.name === "Test Song Beta") {
          s.has_lighting = true;
          s.lighting_files = ["show.light"];
        }
      }
      await route.fulfill({ response: res, json: data });
    });
    await page.route("**/api/songs/Test%20Song%20Beta", async (route) => {
      if (route.request().method() !== "GET") return route.continue();
      const res = await route.fetch();
      const yaml =
        (await res.text()) +
        "tempo:\n  bpm: 132\nlighting:\n  - file: show.light\n";
      await route.fulfill({ response: res, body: yaml });
    });
    await page.route("**/api/lighting/show.light", async (route) => {
      if (route.request().method() !== "GET") return route.continue();
      await route.fulfill({
        status: 200,
        contentType: "text/plain",
        body: showBody,
      });
    });
  });

  test("the song timeline imports a light show's tempo map", async ({
    page,
  }) => {
    await page.goto("/#/songs/Test%20Song%20Beta/sections");

    // The base tempo marker reflects the song's own map.
    const baseMarker = page.locator(".marker", { hasText: "132" });
    await expect(baseMarker).toBeVisible();
    await baseMarker.click();

    const dialog = page.locator(".marker-dialog");
    await expect(dialog).toBeVisible();
    const bpmInput = dialog.getByRole("textbox", { name: "BPM" });
    await expect(bpmInput).toHaveValue("132");

    await dialog
      .getByRole("button", { name: "Import from show.light" })
      .click();
    await expect(bpmInput).toHaveValue("128");

    // The imported change appears as a marker on the tempo lane.
    await dialog.getByRole("button", { name: "Done" }).click();
    await expect(page.locator(".marker-chip", { hasText: "90" })).toBeVisible();
  });

  test("an import orders its changes and drops repeated positions", async ({
    page,
  }) => {
    showBody = LIGHT_SHOW_MESSY_TEMPO;
    await page.goto("/#/songs/Test%20Song%20Beta/sections");

    await page.locator(".marker", { hasText: "132" }).click();
    const dialog = page.locator(".marker-dialog");
    await dialog
      .getByRole("button", { name: "Import from show.light" })
      .click();

    // The repeat is reported rather than silently resolved.
    await expect(dialog.locator(".import-note")).toContainText(
      "position already taken",
    );
    await dialog.getByRole("button", { name: "Done" }).click();

    const request = page.waitForRequest(
      (req) =>
        req.url().includes("/api/songs/Test%20Song%20Beta") &&
        req.method() === "PUT",
    );
    await page.getByRole("button", { name: "Save" }).first().click();
    const saved = parse((await request).postData() ?? "") as {
      tempo?: { changes?: { measure: number; bpm: number }[] };
    };

    // Ascending, one change per position, and the m8 survivor is the last of
    // the two the show listed there — the one its own playback would use.
    expect(saved.tempo?.changes).toEqual([
      { measure: 8, bpm: 95 },
      { measure: 20, bpm: 100 },
    ]);
  });

  test("a time-anchored change survives the show file and snaps on import", async ({
    page,
  }) => {
    showBody = LIGHT_SHOW_ABSOLUTE_TEMPO;
    await page.goto("/#/songs/Test%20Song%20Beta/sections");

    await page.locator(".marker", { hasText: "132" }).click();
    const dialog = page.locator(".marker-dialog");
    await dialog
      .getByRole("button", { name: "Import from show.light" })
      .click();

    // Reading `@00:02.000` is the whole test: before the parser knew the
    // absolute form the change never reached the import, so the note stayed
    // empty and the change was lost.
    await expect(dialog.locator(".import-note")).toContainText(
      "snapped to the grid",
    );
    await dialog.getByRole("button", { name: "Done" }).click();
    await expect(
      page.locator(".marker-chip", { hasText: "140" }),
    ).toBeVisible();
  });

  test("a light show copies the song timeline's tempo map", async ({
    page,
  }) => {
    await page.goto("/#/songs/Test%20Song%20Beta/lighting");

    // The timeline's tempo lane opens the show's tempo editor.
    await page.getByTitle("Tempo - click to edit").click();

    // The show's own tempo map is active first.
    await expect(page.locator(".tempo-info")).toContainText("128 BPM");

    await page.getByRole("button", { name: "Copy from song timeline" }).click();
    await expect(page.locator(".tempo-info")).toContainText("132 BPM");
  });
});
