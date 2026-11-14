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

// The song editor's palette overrides: the slots the song's shows declare,
// each with its show default, and a picker per slot that writes the song's
// `palette:` (clearing one removes the key). An override naming a slot no
// show declares is kept, with a warning.

import { test, expect, type Page } from "@playwright/test";
import YAML from "yaml";
import { SONGS } from "../mock-server/test-data";
import { PlayerService } from "../../src/gen/player/v1/player_pb";
import { listSongsReply } from "../support/library-rpc";
import { mockRpc } from "../support/rpc";

const SHOW = `palette {
    primary: "#c01020"
    secondary: "orange"
}

show "Main" {
    @00:00.000
    front: static color: palette.primary, duration: 5s
    back: static color: palette.secondary, duration: 5s
}
`;

const SONG_URL = /\/api\/songs\/Test(%20| )Song(%20| )Alpha$/;

/** The song page of Test Song Alpha, with the show above and the song file
 *  as `yaml` says; returns what is saved to the song. */
async function openSong(
  page: Page,
  yaml: string,
): Promise<{ saved: string[] }> {
  const saved: string[] = [];
  const rpc = await mockRpc(page);
  rpc.unary(
    PlayerService.method.listSongs,
    listSongsReply({
      ...SONGS,
      songs: SONGS.songs.map((s) =>
        s.name === "Test Song Alpha"
          ? {
              ...s,
              palette: {
                slots: [
                  { name: "primary", color: "#c01020" },
                  { name: "secondary", color: "#ffa500" },
                ],
                overrides: {},
              },
            }
          : s,
      ),
    }),
  );
  await page.route(/\/api\/lighting\/show\.light$/, (route) =>
    route.request().method() === "GET"
      ? route.fulfill({ status: 200, contentType: "text/plain", body: SHOW })
      : route.fallback(),
  );
  await page.route(SONG_URL, (route) => {
    const method = route.request().method();
    if (method === "PUT") {
      saved.push(route.request().postData() ?? "");
      return route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({ status: "saved" }),
      });
    }
    if (method === "GET") {
      return route.fulfill({
        status: 200,
        contentType: "text/yaml",
        body: yaml,
      });
    }
    return route.fallback();
  });
  await page.goto("/#/songs/Test%20Song%20Alpha/lighting");
  await expect(page.getByTestId("song-palette")).toBeVisible();
  return { saved };
}

const BASE_YAML = `name: Test Song Alpha
tracks:
  - name: kick
    file: kick.wav
lighting:
  - file: show.light
`;

test.describe("Song editor: palette overrides", () => {
  test("lists the song's slots with their show defaults", async ({ page }) => {
    await openSong(page, BASE_YAML);
    const primary = page.getByTestId("song-palette-slot-primary");
    await expect(primary).toBeVisible();
    await expect(primary.getByRole("img")).toHaveAttribute(
      "aria-label",
      "Show default: #c01020",
    );
    await expect(primary).toContainText("not overridden");
    await expect(page.getByTestId("song-palette-slot-secondary")).toBeVisible();
    // Nothing overridden: nothing to flag.
    await expect(page.getByTestId("song-palette-reset-primary")).toHaveCount(0);
  });

  test("an override is written to the song's palette and cleared by reset", async ({
    page,
  }) => {
    const { saved } = await openSong(page, BASE_YAML);
    await page.locator("#song-palette-primary").fill("#2040ff");
    await expect(page.getByTestId("song-palette-reset-primary")).toBeVisible();
    await page.getByRole("button", { name: "Save", exact: true }).click();
    await expect.poll(() => saved.length).toBe(1);
    const first = YAML.parse(saved[0]);
    expect(first.palette).toEqual({ primary: "#2040ff" });
    // The rest of the song is as it was.
    expect(first.lighting).toEqual([{ file: "show.light" }]);

    await page.getByTestId("song-palette-reset-primary").click();
    await expect(page.getByTestId("song-palette-reset-primary")).toHaveCount(0);
    await page.getByRole("button", { name: "Save", exact: true }).click();
    await expect.poll(() => saved.length).toBe(2);
    // Clearing the last override removes the key.
    expect(YAML.parse(saved[1])).not.toHaveProperty("palette");
    expect(saved[1]).not.toContain("palette");
  });

  test("shows the song's existing overrides, and flags one no show declares", async ({
    page,
  }) => {
    const { saved } = await openSong(
      page,
      `${BASE_YAML}palette:\n  secondary: "#00ff00"\n  ghost: cyan\n`,
    );
    const secondary = page.getByTestId("song-palette-slot-secondary");
    await expect(secondary.locator("input[type=color]")).toHaveValue("#00ff00");
    await expect(secondary).toContainText("#00ff00");
    const ghost = page.getByTestId("song-palette-unknown-ghost");
    await expect(ghost).toContainText("Not declared");
    await expect(ghost).toContainText("cyan");

    // Removing it takes the key out and keeps the rest.
    await ghost.getByRole("button", { name: "Remove" }).click();
    await expect(ghost).toHaveCount(0);
    await page.getByRole("button", { name: "Save", exact: true }).click();
    await expect.poll(() => saved.length).toBe(1);
    expect(YAML.parse(saved[0]).palette).toEqual({ secondary: "#00ff00" });
  });
});
