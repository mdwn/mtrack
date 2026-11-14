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

// The playlist editor's palette: a colour strip of the set (each song's
// colours as they play, side by side), and per entry the song's slots with
// an override picker that writes the entry's palette.

import { test, expect, type Locator } from "@playwright/test";
import { PlaylistEditorPage } from "../pages/playlist-editor.page.js";
import { capturePlaylistPut, stubPlaylist } from "../support/playlists";

const SONG_PALETTES = {
  "Test Song Alpha": {
    slots: [
      { name: "primary", color: "#c01020" },
      { name: "secondary", color: "#ffa500" },
    ],
    overrides: {},
  },
  "Test Song Beta": {
    slots: [{ name: "primary", color: "#c01020" }],
    // The song's own recolour.
    overrides: { primary: "#00aa44" },
  },
  "Test Song Gamma": { slots: [], overrides: {} },
};

const SETLIST = {
  name: "setlist",
  songs: ["Test Song Alpha", "Test Song Beta", "Test Song Gamma"],
  available_songs: [],
  song_tracks: {},
  song_palettes: SONG_PALETTES,
};

/** The colours of a strip tile, band by band, top to bottom. */
async function tileColors(tile: Locator): Promise<string[]> {
  return tile
    .getByTestId("strip-band")
    .evaluateAll((bands) =>
      bands.map((b) => b.getAttribute("data-color") ?? ""),
    );
}

test.describe("Playlist editor: palette", () => {
  test("the strip shows each song's colours as they play, side by side", async ({
    page,
  }) => {
    await stubPlaylist(page, "setlist", {
      ...SETLIST,
      entry_palette: [{}, {}, {}],
    });
    const playlists = new PlaylistEditorPage(page);
    await playlists.goto();
    await playlists.selectPlaylist("setlist");

    const strip = page.getByTestId("palette-strip");
    await expect(strip).toBeVisible();
    await expect(strip.getByRole("listitem")).toHaveCount(3);
    // The show's default, then the song's own override.
    expect(await tileColors(page.getByTestId("strip-tile-1"))).toEqual([
      "#c01020",
      "#ffa500",
    ]);
    expect(await tileColors(page.getByTestId("strip-tile-2"))).toEqual([
      "#00aa44",
    ]);
    await expect(
      page.getByTestId("strip-tile-2").getByTestId("strip-band"),
    ).toHaveAttribute("data-source", "song");
    // A song without a palette is a tile without bands, and says so.
    await expect(
      page.getByTestId("strip-tile-3").getByTestId("strip-band"),
    ).toHaveCount(0);
    await expect(page.getByTestId("strip-tile-3")).toHaveAttribute(
      "aria-label",
      "3. Test Song Gamma: no palette",
    );
    await expect(page.getByTestId("strip-tile-1")).toHaveAttribute(
      "aria-label",
      "1. Test Song Alpha: primary #c01020, secondary #ffa500",
    );
  });

  test("an entry's override recolours the strip and is saved with the entry", async ({
    page,
  }) => {
    await stubPlaylist(page, "setlist", {
      ...SETLIST,
      entry_palette: [{}, {}, {}],
    });
    const bodies = await capturePlaylistPut(page);
    const playlists = new PlaylistEditorPage(page);
    await playlists.goto();
    await playlists.selectPlaylist("setlist");

    const row = playlists.playlistSongs().first();
    const panel = await playlists.openTransitions(row);
    await expect(panel.getByTestId("transitions-palette")).toBeVisible();
    await panel.locator("input[type=color]").first().fill("#2040ff");
    expect(await tileColors(page.getByTestId("strip-tile-1"))).toEqual([
      "#2040ff",
      "#ffa500",
    ]);
    await expect(
      page.getByTestId("strip-tile-1").getByTestId("strip-band").first(),
    ).toHaveAttribute("data-source", "entry");
    // The closed entry says so.
    await playlists.transitionsToggle(row).click();
    await expect(playlists.transitionsToggle(row)).toHaveText(
      "1 color recolored",
    );

    await playlists.saveButton.click();
    await expect.poll(() => bodies.length).toBe(1);
    expect(bodies[0].entry_palette).toEqual([{ primary: "#2040ff" }, {}, {}]);
  });

  test("resetting an override sends the entry's palette empty", async ({
    page,
  }) => {
    await stubPlaylist(page, "setlist", {
      ...SETLIST,
      entry_palette: [{ primary: "cyan" }, {}, {}],
    });
    const bodies = await capturePlaylistPut(page);
    const playlists = new PlaylistEditorPage(page);
    await playlists.goto();
    await playlists.selectPlaylist("setlist");
    // A name the player reads is shown as its colour.
    expect(await tileColors(page.getByTestId("strip-tile-1"))).toEqual([
      "#00ffff",
      "#ffa500",
    ]);

    const row = playlists.playlistSongs().first();
    const panel = await playlists.openTransitions(row);
    await panel.getByTestId(/-palette-reset-primary$/).click();
    expect(await tileColors(page.getByTestId("strip-tile-1"))).toEqual([
      "#c01020",
      "#ffa500",
    ]);
    await playlists.saveButton.click();
    await expect.poll(() => bodies.length).toBe(1);
    expect(bodies[0].entry_palette).toEqual([{}, {}, {}]);
  });

  test("the same song twice plays in the colours of each entry", async ({
    page,
  }) => {
    await stubPlaylist(page, "setlist", {
      ...SETLIST,
      songs: ["Test Song Alpha", "Test Song Alpha"],
      entry_palette: [{}, { secondary: "#112233" }],
    });
    const playlists = new PlaylistEditorPage(page);
    await playlists.goto();
    await playlists.selectPlaylist("setlist");
    expect(await tileColors(page.getByTestId("strip-tile-1"))).toEqual([
      "#c01020",
      "#ffa500",
    ]);
    expect(await tileColors(page.getByTestId("strip-tile-2"))).toEqual([
      "#c01020",
      "#112233",
    ]);
  });

  test("an override for a slot the song lacks is flagged and can be removed", async ({
    page,
  }) => {
    await stubPlaylist(page, "setlist", {
      ...SETLIST,
      entry_palette: [{ ghost: "#123456" }, {}, {}],
    });
    const bodies = await capturePlaylistPut(page);
    const playlists = new PlaylistEditorPage(page);
    await playlists.goto();
    await playlists.selectPlaylist("setlist");

    const row = playlists.playlistSongs().first();
    await expect(row.getByTestId("entry-palette-warn")).toHaveText(
      "Not declared",
    );
    await expect(page.getByTestId("strip-warn")).toHaveCount(1);
    const panel = await playlists.openTransitions(row);
    const ghost = panel.getByTestId(/-palette-unknown-ghost$/);
    await expect(ghost).toContainText("Not declared");
    await ghost.getByRole("button", { name: "Remove" }).click();
    await expect(row.getByTestId("entry-palette-warn")).toHaveCount(0);
    await expect(page.getByTestId("strip-warn")).toHaveCount(0);
    await playlists.saveButton.click();
    await expect.poll(() => bodies.length).toBe(1);
    expect(bodies[0].entry_palette).toEqual([{}, {}, {}]);
  });

  test("a set with no palettes has no strip", async ({ page }) => {
    const playlists = new PlaylistEditorPage(page);
    await playlists.goto();
    await playlists.selectPlaylist("setlist");
    await expect(playlists.songColumns).toBeVisible();
    await expect(page.getByTestId("palette-strip")).toHaveCount(0);
  });
});
