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
    this.brand = page.locator(".topnav__brand");
    this.dashboardLink = page.locator('.topnav__tab[href="#/"]');
    this.configLink = page.locator('.topnav__tab[href="#/config"]');
    this.songsLink = page.locator('.topnav__tab[href="#/songs"]');
    this.playlistsLink = page.locator('.topnav__tab[href="#/playlists"]');
    this.statusLink = page.locator('.topnav__tab[href="#/status"]');
    this.statusIndicator = page.locator(".topnav__conn");
    this.lockToggle = page.locator(".topnav__lock");
    this.nowPlaying = page.locator(".topnav__transport");
    this.nowPlayingSong = page.locator(".topnav__transport-song");
    this.hamburger = page.locator(".topnav__hamburger");
  }

  async goto(hash: string = "/") {
    await this.page.goto(`/#${hash}`);
  }

  navLinks(): Locator {
    return this.page.locator(".topnav__tab");
  }

  activeLink(): Locator {
    return this.page.locator(".topnav__tab--active");
  }
}
