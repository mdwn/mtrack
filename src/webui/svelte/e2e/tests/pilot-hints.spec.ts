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

let testCounter = 0;

async function sendWsMessage(
  page: import("@playwright/test").Page,
  wsId: string,
  msg: object,
) {
  await page.request.post("http://127.0.0.1:3111/test/send-ws", {
    data: { ...msg, _wsId: wsId },
  });
}

const HINTS = [
  {
    label: "bridge in 3..2..1",
    at_ms: 60000,
    start_ms: 57000,
    end_ms: 60000,
    has_audio: true,
  },
  {
    label: "solo",
    at_ms: 120000,
    start_ms: 120000,
    end_ms: 120000,
    has_audio: false,
  },
];

function playbackState(overrides: Record<string, unknown> = {}) {
  return {
    type: "playback",
    is_playing: true,
    elapsed_ms: 0,
    song_name: "Test Song Beta",
    song_duration_ms: 240000,
    playlist_name: "setlist",
    playlist_position: 1,
    playlist_songs: ["Test Song Alpha", "Test Song Beta"],
    tracks: [],
    available_playlists: ["all_songs", "setlist"],
    persisted_playlist_name: "setlist",
    locked: false,
    available_sections: [],
    active_section: null,
    pilot_hints: HINTS,
    ...overrides,
  };
}

test.describe("Pilot hints", () => {
  let wsId: string;

  test.beforeEach(async ({ page }) => {
    wsId = `pilot-hints-${++testCounter}-${Date.now()}`;
    await page.goto(`/?wsId=${wsId}#/`);
    await expect(page.locator(".playback-card__title")).toContainText(
      "Test Song Alpha",
    );
  });

  test("hint markers render on the progress bar", async ({ page }) => {
    await sendWsMessage(page, wsId, playbackState());

    await expect(page.locator(".scrub__hint")).toHaveCount(2);
    await expect(page.locator(".scrub__hint").first()).toHaveAttribute(
      "title",
      "bridge in 3..2..1",
    );
  });

  test("hint label appears in its lead window and highlights while live", async ({
    page,
  }) => {
    // Before the lead window — no hint label.
    await sendWsMessage(page, wsId, playbackState({ elapsed_ms: 40000 }));
    await expect(page.locator(".playback-card__hint")).toHaveCount(0);

    // In the lead window (5s before) — label shows, not yet live.
    await sendWsMessage(page, wsId, playbackState({ elapsed_ms: 56000 }));
    const hint = page.locator(".playback-card__hint");
    await expect(hint).toContainText("bridge in 3..2..1");
    await expect(hint).not.toHaveClass(/playback-card__hint--live/);

    // While the sample plays — highlighted.
    await sendWsMessage(page, wsId, playbackState({ elapsed_ms: 58000 }));
    await expect(hint).toHaveClass(/playback-card__hint--live/);

    // After the anchor — gone again.
    await sendWsMessage(page, wsId, playbackState({ elapsed_ms: 70000 }));
    await expect(page.locator(".playback-card__hint")).toHaveCount(0);
  });

  test("hint label hidden while stopped", async ({ page }) => {
    await sendWsMessage(
      page,
      wsId,
      playbackState({ is_playing: false, elapsed_ms: 58000 }),
    );
    await expect(page.locator(".playback-card__hint")).toHaveCount(0);
  });
});
