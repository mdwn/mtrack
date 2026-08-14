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

import { expect, type Page } from "@playwright/test";

// The example playlist songs in order (from examples/playlist.yaml).
export const PLAYLIST_SONGS = [
  "Sound check",
  "A really cool song",
  "Another cool song",
  "The slow one",
  "A really fast one",
  "Outro tape",
];

// All example songs (alphabetical by display name).
export const ALL_SONGS = [
  "A really cool song",
  "A really fast one",
  "Another cool song",
  "DSL Light Show Song",
  "Outro tape",
  "Sound check",
  "The slow one",
];

// Songs that have DSL lighting shows configured.
export const SONGS_WITH_LIGHTING = [
  "A really cool song",
  "DSL Light Show Song",
  "The slow one",
];

/**
 * Wait for the WebSocket to connect and deliver initial state.
 * Waits for the green status indicator dot in the nav.
 */
export async function waitForWebSocket(page: Page): Promise<void> {
  await expect(page.locator(".status-indicator.connected")).toBeVisible({
    timeout: 15000,
  });
}

/**
 * Wait for the playback card to show a song name (WebSocket has delivered state).
 */
export async function waitForPlaybackState(page: Page): Promise<void> {
  await waitForWebSocket(page);
  // Wait for the playback-song element to have non-empty, non-"No song" text.
  await expect(page.locator(".playback-song")).not.toHaveText("No song", {
    timeout: 15000,
  });
}
