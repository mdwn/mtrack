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
    this.grid = page.locator(".dashboard-grid");
    this.songName = page.locator(".playback-song");
    this.playbackStatus = page.locator(".playback-status");
    this.playButton = page.getByRole("button", { name: "Play" });
    this.stopButton = page.getByRole("button", { name: "Stop" });
    this.prevButton = page.getByRole("button", { name: "Prev", exact: true });
    this.nextButton = page.getByRole("button", { name: "Next", exact: true });
    this.progressBar = page.locator(".progress-bar");
    this.progressTime = page.locator(".progress-time");
    this.playlistSelect = page.locator(".playlist-select");
    this.playlistSongs = page.locator(".playlist-songs li");
    this.trackRows = page.locator(".track-row");
    this.trackCount = page.locator(".track-count");
  }

  async goto() {
    await this.page.goto("/#/");
  }
}
