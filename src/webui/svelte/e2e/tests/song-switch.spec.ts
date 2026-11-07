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
});
