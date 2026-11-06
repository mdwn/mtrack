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

/** Answers PUTs of the `setlist` playlist as saved, letting other requests
 *  through; the bodies PUT, in order, as they come. */
export async function capturePlaylistPut(page: Page) {
  const bodies: Record<string, unknown>[] = [];
  await page.route("**/api/playlists/setlist", async (route) => {
    if (route.request().method() === "PUT") {
      bodies.push(route.request().postDataJSON());
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({ status: "saved", name: "setlist" }),
      });
    } else {
      await route.continue();
    }
  });
  return bodies;
}
