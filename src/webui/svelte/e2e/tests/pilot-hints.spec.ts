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
  // The mock server drops messages while this page's WebSocket isn't
  // registered (initial connect, or a reconnect through vite's proxy) and
  // reports it via `sent` — retry until the message was actually delivered.
  await expect(async () => {
    const res = await page.request.post("http://127.0.0.1:3111/test/send-ws", {
      data: { ...msg, _wsId: wsId },
    });
    expect((await res.json()).sent).toBe(1);
  }).toPass({ timeout: 10000 });
}

const HINTS = [
  // A label-only hint directly followed by its countdown: the display
  // windows overlap, so both surface together as one group.
  {
    label: "bridge",
    at_ms: 55000,
    start_ms: 55000,
    end_ms: 55000,
    has_audio: false,
  },
  {
    label: "3..2..1",
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
    // Include the worker index: parallel workers each start their own
    // counter at 1 in the same millisecond, so counter+timestamp alone can
    // collide across workers — colliding ids make the mock server route one
    // test's messages to another test's page.
    wsId = `pilot-hints-${test.info().parallelIndex}-${++testCounter}-${Date.now()}`;
    await page.goto(`/?wsId=${wsId}#/`);
    await expect(page.locator(".playback-card__title")).toContainText(
      "Test Song Alpha",
    );
  });

  test("hint markers render on the progress bar", async ({ page }) => {
    await sendWsMessage(page, wsId, playbackState());

    await expect(page.locator(".scrub__hint")).toHaveCount(3);
    await expect(page.locator(".scrub__hint").first()).toHaveAttribute(
      "title",
      "bridge",
    );
  });

  test("adjacent hints surface together; only the live one highlights", async ({
    page,
  }) => {
    // Before the group's lead window — no hint labels.
    await sendWsMessage(page, wsId, playbackState({ elapsed_ms: 40000 }));
    await expect(page.locator(".playback-card__hint")).toHaveCount(0);

    // In the lead window — both labels of the group show, none live.
    await sendWsMessage(page, wsId, playbackState({ elapsed_ms: 52000 }));
    const labels = page.locator(".playback-card__hint-label");
    await expect(labels).toHaveCount(2);
    await expect(labels.nth(0)).toContainText("bridge");
    await expect(labels.nth(1)).toContainText("3..2..1");
    await expect(page.locator(".playback-card__hint-label--live")).toHaveCount(
      0,
    );

    // At the label-only hint's anchor — "bridge" highlights, countdown stays.
    await sendWsMessage(page, wsId, playbackState({ elapsed_ms: 55500 }));
    await expect(labels.nth(0)).toHaveClass(/playback-card__hint-label--live/);
    await expect(labels.nth(1)).not.toHaveClass(
      /playback-card__hint-label--live/,
    );

    // While the countdown sample plays — the highlight hands off to it,
    // and the passed "bridge" label stays visible in grey.
    await sendWsMessage(page, wsId, playbackState({ elapsed_ms: 58000 }));
    await expect(labels).toHaveCount(2);
    await expect(labels.nth(0)).not.toHaveClass(
      /playback-card__hint-label--live/,
    );
    await expect(labels.nth(1)).toHaveClass(/playback-card__hint-label--live/);

    // After the group has passed — gone.
    await sendWsMessage(page, wsId, playbackState({ elapsed_ms: 70000 }));
    await expect(page.locator(".playback-card__hint")).toHaveCount(0);
  });

  test("label-only hint highlights briefly at its anchor", async ({ page }) => {
    // In the lead window — visible, not live.
    await sendWsMessage(page, wsId, playbackState({ elapsed_ms: 118000 }));
    const labels = page.locator(".playback-card__hint-label");
    await expect(labels).toHaveCount(1);
    await expect(labels.first()).toContainText("solo");
    await expect(page.locator(".playback-card__hint-label--live")).toHaveCount(
      0,
    );

    // Just after its anchor — highlighted even though it has no audio.
    await sendWsMessage(page, wsId, playbackState({ elapsed_ms: 120300 }));
    await expect(labels.first()).toHaveClass(/playback-card__hint-label--live/);

    // After the brief highlight window — gone.
    await sendWsMessage(page, wsId, playbackState({ elapsed_ms: 121500 }));
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
