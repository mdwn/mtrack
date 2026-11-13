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

import type { Page } from "@playwright/test";
import { api, expect, test } from "./harness";
import { silentWav } from "../support/wav";

// A song's lighting files live in the song's folder and song.yaml names
// them from there. The editor addresses files under the songs root, which
// is the lighting API's view; the two must not get mixed up, or the file
// lands beside the song folders, or song.yaml names a path the loader
// cannot find, and the song stops loading.

const PAR = `fixture_type "Par" {
  channels: 3
  channel_map: { "red": 1, "green": 2, "blue": 3 }
}
`;
const HOUSE = `venue "house" {
  fixture "Par 1" Par @ 1:1
}
`;

interface Songs {
  songs: { name: string; lighting_files: string[] }[];
  failures: { name: string; error: string }[];
}

test.use({
  files: {
    "lighting/fixture_types/par.light": PAR,
    "lighting/venues/house.light": HOUSE,
    "songs/Lit/kick.wav": silentWav(44100 * 2),
    "songs/Lit/song.yaml":
      "kind: song\nname: Lit\ntracks:\n  - name: kick\n    file: kick.wav\n",
  },
});

/** The song's Lighting tab, once the page is live: the player reports
 *  locked until the first status message arrives on the Watch stream, and
 *  the editor's add and remove buttons refuse while it does. */
async function openLightingTab(page: Page) {
  await expect(
    page.locator(".topnav__lock:not(.topnav__lock--locked)"),
  ).toBeVisible();
  await page.locator("#song-tab-lighting").click();
}

test("a light show added in the song editor is written to the song's folder and loads", async ({
  page,
  project,
}) => {
  await page.goto("/#/songs/Lit");
  await openLightingTab(page);

  // Add a show file by name; the prompt is the app's own dialog.
  await page.getByRole("button", { name: "+ DSL" }).click();
  const dialog = page.getByRole("dialog");
  await dialog.locator("input").fill("verse");
  await dialog.locator(".dialog-actions button").last().click();
  await expect(
    page.locator(".light-file-name", { hasText: "verse.light" }),
  ).toBeVisible();

  const save = page.locator(".tab-save button");
  await save.click();
  await expect(page.locator(".save-msg")).toHaveText("Saved");

  // The file is in the song's folder, not beside the song folders.
  expect(project.exists("songs/Lit/verse.light")).toBe(true);
  expect(project.exists("songs/verse.light")).toBe(false);
  // song.yaml names it from the song's folder.
  expect(project.read("songs/Lit/song.yaml")).toContain("file: verse.light");

  // The player loads the song with the show.
  let songs = await api<Songs>(project, "/songs");
  expect(songs.failures).toEqual([]);
  expect(songs.songs.find((s) => s.name === "Lit")?.lighting_files).toEqual([
    "Lit/verse.light",
  ]);

  // Removing it takes the entry out of song.yaml as well, so the song
  // still loads without the file.
  await page.reload();
  await openLightingTab(page);
  await page.locator(".light-file-remove").click();
  await page
    .getByRole("dialog")
    .locator(".dialog-actions button")
    .last()
    .click();
  await save.click();
  await expect(page.locator(".save-msg")).toHaveText("Saved");

  expect(project.exists("songs/Lit/verse.light")).toBe(false);
  expect(project.read("songs/Lit/song.yaml")).not.toContain("lighting");
  songs = await api<Songs>(project, "/songs");
  expect(songs.failures).toEqual([]);
  expect(songs.songs.find((s) => s.name === "Lit")?.lighting_files).toEqual([]);
});

// The #639 case itself: a song with no lighting opens on a placeholder
// file named after the song, and the first show is typed into it.
test("a show typed into a lighting-less song's Raw DSL tab is saved in the song's folder", async ({
  page,
  project,
}) => {
  await page.goto("/#/songs/Lit");
  await openLightingTab(page);
  await page.getByRole("button", { name: "Raw DSL" }).click();
  await page
    .locator(".raw-textarea")
    .fill('show "Main" {\n    @00:00.000\n}\n');
  await page.locator(".tab-save button").click();
  await expect(page.locator(".save-msg")).toHaveText("Saved");

  expect(project.exists("songs/Lit/Lit.light")).toBe(true);
  expect(project.exists("songs/Lit.light")).toBe(false);
  expect(project.read("songs/Lit/Lit.light")).toContain('show "Main"');
  expect(project.read("songs/Lit/song.yaml")).toContain("file: Lit.light");
  const songs = await api<Songs>(project, "/songs");
  expect(songs.failures).toEqual([]);
  expect(songs.songs.find((s) => s.name === "Lit")?.lighting_files).toEqual([
    "Lit/Lit.light",
  ]);
});

// Naming the first file after the song fills the placeholder in rather
// than sitting beside it under the same path.
test("the song's own name for the first show gives one file", async ({
  page,
  project,
}) => {
  await page.goto("/#/songs/Lit");
  await openLightingTab(page);
  await page.getByRole("button", { name: "+ DSL" }).click();
  const dialog = page.getByRole("dialog");
  await dialog.locator("input").fill("Lit");
  await dialog.locator(".dialog-actions button").last().click();
  await expect(page.locator(".light-file-name")).toHaveCount(1);
  await page.locator(".tab-save button").click();
  await expect(page.locator(".save-msg")).toHaveText("Saved");

  expect(project.read("songs/Lit/Lit.light")).toContain('show "Lit"');
  const yaml = project.read("songs/Lit/song.yaml");
  expect(yaml.match(/file: Lit\.light/g)).toHaveLength(1);
  const songs = await api<Songs>(project, "/songs");
  expect(songs.failures).toEqual([]);
  expect(songs.songs.find((s) => s.name === "Lit")?.lighting_files).toEqual([
    "Lit/Lit.light",
  ]);
});
