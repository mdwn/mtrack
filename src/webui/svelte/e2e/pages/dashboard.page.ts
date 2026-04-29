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

export class DashboardPage {
  readonly grid: Locator;
  readonly songName: Locator;
  readonly playbackStatus: Locator;
  readonly playButton: Locator;
  readonly stopButton: Locator;
  readonly prevButton: Locator;
  readonly nextButton: Locator;
  readonly progressBar: Locator;
  readonly progressTime: Locator;
  readonly playlistSelect: Locator;
  readonly playlistSongs: Locator;
  readonly trackRows: Locator;
  readonly trackCount: Locator;

  constructor(private page: Page) {
    this.grid = page.locator(".playback-card");
    this.songName = page.locator(".playback-card__title");
    this.playbackStatus = page.locator(".playback-card__state");
    this.playButton = page.locator(".btn-play");
    this.stopButton = page
      .locator(".playback-card__transport .btn-icon-circle")
      .filter({ has: page.locator('svg rect[width="12"]') });
    this.prevButton = page
      .locator(".playback-card__transport .btn-icon-circle")
      .first();
    this.nextButton = page
      .locator(".playback-card__transport .btn-icon-circle")
      .last();
    this.progressBar = page.locator(".scrub").first();
    this.progressTime = page.locator(".playback-card__time").first();
    this.playlistSelect = page.locator(".playlist-card__select");
    this.playlistSongs = page.locator(".playlist-card__list li");
    this.trackRows = page.locator(".tracks-card__row");
    this.trackCount = page.locator(".tracks-card__title");
  }

  async goto() {
    await this.page.goto("/#/");
  }
}
