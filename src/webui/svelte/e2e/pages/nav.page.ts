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

export class NavPage {
  readonly brand: Locator;
  readonly dashboardLink: Locator;
  readonly configLink: Locator;
  readonly songsLink: Locator;
  readonly playlistsLink: Locator;
  readonly statusLink: Locator;
  readonly statusIndicator: Locator;
  readonly lockToggle: Locator;
  readonly nowPlaying: Locator;
  readonly nowPlayingSong: Locator;
  readonly hamburger: Locator;

  constructor(private page: Page) {
    this.brand = page.locator(".nav-brand");
    this.dashboardLink = page.locator('a.nav-link[href="#/"]');
    this.configLink = page.locator('a.nav-link[href="#/config"]');
    this.songsLink = page.locator('a.nav-link[href="#/songs"]');
    this.playlistsLink = page.locator('a.nav-link[href="#/playlists"]');
    this.statusLink = page.locator('a.nav-link[href="#/status"]');
    this.statusIndicator = page.locator(".status-indicator");
    this.lockToggle = page.locator(".lock-toggle");
    this.nowPlaying = page.locator(".now-playing");
    this.nowPlayingSong = page.locator(".now-playing-song");
    this.hamburger = page.locator(".hamburger");
  }

  async goto(hash: string = "/") {
    await this.page.goto(`/#${hash}`);
  }

  navLinks(): Locator {
    return this.page.locator("a.nav-link");
  }

  activeLink(): Locator {
    return this.page.locator("a.nav-link.active");
  }
}
