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

import type { Page, Locator } from "@playwright/test";

export class SongBrowserPage {
  readonly heading: Locator;
  readonly songRows: Locator;
  readonly searchInput: Locator;
  readonly failureRows: Locator;

  constructor(private page: Page) {
    this.heading = page.getByRole("heading", { name: "Songs" });
    this.songRows = page.locator(".song-row");
    this.searchInput = page.getByPlaceholder(/search/i);
    this.failureRows = page.locator(".song-row.failure, .failure-row");
  }

  async goto() {
    await this.page.goto("/#/songs");
  }

  songByName(name: string): Locator {
    return this.page.locator(".song-row", { hasText: name });
  }

  async selectSong(name: string) {
    await this.songByName(name).click();
  }
}
