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

test.describe("Tempo import between timeline and light shows", () => {
  test.beforeEach(async ({ page }) => {
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
        body: LIGHT_SHOW_WITH_TEMPO,
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
