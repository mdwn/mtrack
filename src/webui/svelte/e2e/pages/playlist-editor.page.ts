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

export class PlaylistEditorPage {
  readonly listPanel: Locator;
  readonly detailPanel: Locator;
  readonly playlistItems: Locator;
  readonly newButton: Locator;
  readonly saveButton: Locator;
  readonly cancelButton: Locator;
  readonly newPlaylistInput: Locator;
  readonly createButton: Locator;
  readonly searchInput: Locator;
  readonly activeBadge: Locator;
  readonly errorBanner: Locator;
  readonly songColumns: Locator;

  constructor(private page: Page) {
    this.listPanel = page.locator(".list-panel");
    this.detailPanel = page.locator(".detail-panel");
    this.playlistItems = page.locator(".playlist-list li");
    this.newButton = page.getByRole("button", { name: "New" });
    this.saveButton = page.getByRole("button", { name: "Save" });
    this.cancelButton = page.getByRole("button", { name: "Cancel" });
    this.newPlaylistInput = page.locator(".new-playlist-form input");
    this.createButton = page.getByRole("button", { name: "Create" });
    this.searchInput = page.getByPlaceholder("Search songs...");
    this.activeBadge = page.locator(".badge");
    this.errorBanner = page.locator(".error-banner");
    this.songColumns = page.locator(".song-columns");
  }

  async goto() {
    await this.page.goto("/#/playlists");
  }

  playlistByName(name: string): Locator {
    return this.page.locator(".playlist-list li", { hasText: name });
  }

  async selectPlaylist(name: string) {
    await this.playlistByName(name).click();
  }

  playlistSongs(): Locator {
    return this.page.locator(".song-col:first-child .song-list li");
  }

  availableSongs(): Locator {
    return this.page.locator(".song-col:last-child .song-list li");
  }

  /** A row's transitions button: its summary, or "Transitions". */
  transitionsToggle(row: Locator): Locator {
    return row.getByTestId("transitions-toggle");
  }

  /** A row's transitions panel (present only while open). */
  transitionsPanel(row: Locator): Locator {
    return row.getByTestId("transitions-panel");
  }

  /** Opens a row's transitions panel, if it is closed. */
  async openTransitions(row: Locator): Promise<Locator> {
    const toggle = this.transitionsToggle(row);
    if ((await toggle.getAttribute("aria-expanded")) !== "true") {
      await toggle.click();
    }
    return this.transitionsPanel(row);
  }

  /** The auto play gap, in an open transitions panel. */
  autoPlayGap(row: Locator): Locator {
    return this.transitionsPanel(row).getByTestId("auto-play-gap");
  }

  /** The entry's ambient light picker, in an open transitions panel. */
  entryAmbient(row: Locator): Locator {
    return this.transitionsPanel(row).getByTestId("entry-ambient");
  }

  entryAmbientMode(row: Locator): Locator {
    return this.entryAmbient(row).locator("select");
  }

  /** One of the entry's own ambient fields: show, delay, fade_in, fade_out. */
  entryAmbientField(row: Locator, key: string): Locator {
    return this.entryAmbient(row).locator(`input[id$="-ambient-${key}"]`);
  }
}
