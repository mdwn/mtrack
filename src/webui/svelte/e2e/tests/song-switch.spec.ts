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

// #632: going straight from one song's page to another's (Back, Forward, a
// pasted link) changed the heading but kept the first song's data.
test.describe("Song Detail - switching songs", () => {
  test("a new song's URL loads that song", async ({ page }) => {
    await page.goto("/#/songs/Test%20Song%20Alpha/config");
    const editor = page.locator(".config-editor");
    await expect(editor).toHaveValue(/name: Test Song Alpha/);

    const beta = page.waitForRequest(
      (req) =>
        req.method() === "GET" &&
        new URL(req.url()).pathname === "/api/songs/Test%20Song%20Beta",
    );
    await page.evaluate(() => {
      window.location.hash = "#/songs/Test%20Song%20Beta/config";
    });
    await beta;
    await expect(page.locator(".song-title")).toHaveText("Test Song Beta");
    await expect(editor).toHaveValue(/name: Test Song Beta/);

    // And back again, as the browser's Back does.
    await page.goBack();
    await expect(page.locator(".song-title")).toHaveText("Test Song Alpha");
    await expect(editor).toHaveValue(/name: Test Song Alpha/);
  });

  // #680: a save that renames the song in its config moves to the new
  // name's page, where the next save or delete goes, not the old name.
  test("a save that renames the song goes to its new page", async ({
    page,
  }) => {
    await page.route("**/api/songs/Renamed%20Song", async (route) => {
      if (route.request().method() === "GET") {
        await route.fulfill({
          status: 200,
          contentType: "text/yaml",
          body: "name: Renamed Song\ntracks:\n  - kick\n",
        });
      } else {
        await route.continue();
      }
    });
    await page.goto("/#/songs/Test%20Song%20Alpha/config");
    const editor = page.locator(".config-editor");
    await expect(editor).toHaveValue(/name: Test Song Alpha/);
    const yaml = await editor.inputValue();
    await editor.fill(
      yaml.replace("name: Test Song Alpha", "name: Renamed Song"),
    );

    const put = page.waitForRequest(
      (req) =>
        req.method() === "PUT" &&
        new URL(req.url()).pathname === "/api/songs/Test%20Song%20Alpha",
    );
    await page.getByRole("button", { name: "Save" }).click();
    await put;
    await expect(page).toHaveURL(/#\/songs\/Renamed%20Song\/config$/);
    await expect(page.locator(".song-title")).toHaveText("Renamed Song");
    await expect(editor).toHaveValue(/name: Renamed Song/);

    // The next save goes to the new name.
    const next = page.waitForRequest(
      (req) =>
        req.method() === "PUT" &&
        new URL(req.url()).pathname === "/api/songs/Renamed%20Song",
    );
    await editor.fill((await editor.inputValue()) + "loop_playback: true\n");
    await page.getByRole("button", { name: "Save" }).click();
    await next;
  });
});
