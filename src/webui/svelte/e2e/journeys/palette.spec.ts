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

import type { Locator, Page } from "@playwright/test";
import { CONFIG, expect, songList, test } from "./harness";
import { silentWav } from "../support/wav";

// A palette, end to end against the real player: a slot added in the show
// editor and used by an effect, saved to the song's folder and loaded by the
// player; a playlist entry that recolours it; and the playlist's strip, which
// shows the song's colours as they will play.

const PAR = `fixture_type "Par" {
  channels: 3
  channel_map: { "red": 1, "green": 2, "blue": 3 }
}
`;
const HOUSE = `venue "house" {
  fixture "Par 1" Par @ 1:1
}
`;

test.use({
  config: `${CONFIG}playlists_dir: playlists\n`,
  files: {
    "lighting/fixture_types/par.light": PAR,
    "lighting/venues/house.light": HOUSE,
    "songs/Lit/kick.wav": silentWav(44100 * 2),
    "songs/Lit/song.yaml":
      "kind: song\nname: Lit\ntracks:\n  - name: kick\n    file: kick.wav\nlighting:\n  - file: show.light\n",
    "songs/Lit/show.light":
      'palette {\n    primary: "#c01020"\n}\n\nshow "Main" {\n    @00:00.000\n    all: static color: palette.primary, duration: 5s\n}\n',
    "playlists/set.yaml": "songs:\n  - Lit\n",
  },
});

/** The colors of a strip tile, band by band, as the page paints them. */
async function colors(tile: Locator): Promise<string[]> {
  const all = await tile
    .getByTestId("strip-band")
    .evaluateAll((bands) =>
      bands.map((b) => b.getAttribute("data-color") ?? ""),
    );
  return all.map((c) => c.toLowerCase());
}

/** The page once it is live: the player reports locked until the first
 *  status message arrives on the Watch stream, and the editors refuse
 *  while it does. */
async function live(page: Page) {
  await expect(
    page.locator(".topnav__lock:not(.topnav__lock--locked)"),
  ).toBeVisible();
}

test("a slot added in the show editor, used by an effect and recolored by a playlist entry shows in the strip", async ({
  page,
  project,
}) => {
  // The song's page, its show's palette.
  await page.goto("/#/songs/Lit");
  await live(page);
  await page.locator("#song-tab-lighting").click();
  await page.getByTestId("palette-card").locator("summary").click();
  await expect(page.getByTestId("palette-0-slot")).toHaveCount(1);
  await page.getByTestId("palette-0-new-name").fill("accent");
  await page.locator("input[aria-label='New slot color']").fill("#00ff80");
  await page.getByTestId("palette-0-add").click();
  await expect(page.getByTestId("palette-0-slot")).toHaveCount(2);

  // The first effect takes the new slot.
  await page.locator(".cue-block").first().click({ force: true });
  await page.locator(".effect-form .expand-toggle").first().click();
  await page.getByTestId("color-source").first().selectOption("accent");
  await expect(page.getByTestId("color-slot-swatch").first()).toBeVisible();

  // The song recolors another slot itself.
  await page.locator("#song-palette-primary").fill("#2040ff");

  await page.locator(".tab-save button").click();
  await expect(page.locator(".save-msg")).toHaveText("Saved");

  // On disk: the slot, its use, and the song's override.
  const show = project.read("songs/Lit/show.light");
  expect(show).toContain('accent: "#00ff80"');
  expect(show).toContain("all: static color: palette.accent");
  expect(show).toContain('primary: "#c01020"');
  expect(project.read("songs/Lit/song.yaml")).toMatch(
    /palette:\s+primary: ['"]?#2040ff/,
  );

  // The player loaded it: the song says its slots and its override.
  const library = await songList(project);
  expect(library.failures).toEqual([]);
  const lit = library.songs.find((s) => s.name === "Lit")!;
  expect(lit.palette.slots.map((s) => [s.name, s.color.toLowerCase()])).toEqual(
    [
      ["primary", "#c01020"],
      ["accent", "#00ff80"],
    ],
  );
  expect(
    Object.fromEntries(
      Object.entries(lit.palette.overrides).map(([k, v]) => [
        k,
        v.toLowerCase(),
      ]),
    ),
  ).toEqual({ primary: "#2040ff" });

  // The playlist: its strip is the song's colors, the override included.
  await page.goto("/#/playlists/set");
  await live(page);
  await expect(page.getByTestId("palette-strip")).toBeVisible();
  expect(await colors(page.getByTestId("strip-tile-1"))).toEqual([
    "#2040ff",
    "#00ff80",
  ]);

  // The entry recolors the new slot; the strip follows at once.
  const row = page.locator(".song-col:first-child .song-list li").first();
  await row.getByTestId("transitions-toggle").click();
  await row.getByLabel("Override color of accent").fill("#ff0000");
  expect(await colors(page.getByTestId("strip-tile-1"))).toEqual([
    "#2040ff",
    "#ff0000",
  ]);
  await expect(
    page.getByTestId("strip-tile-1").getByTestId("strip-band").nth(1),
  ).toHaveAttribute("data-source", "entry");

  await page.getByRole("button", { name: "Save", exact: true }).click();
  await expect(page.locator(".dirty-flag")).toHaveCount(0);
  expect(project.read("playlists/set.yaml")).toMatch(
    /palette:\s+accent: ['"]?#ff0000/,
  );

  // A reload reads the entry's palette back from the file.
  await page.reload();
  await live(page);
  expect(await colors(page.getByTestId("strip-tile-1"))).toEqual([
    "#2040ff",
    "#ff0000",
  ]);
});
