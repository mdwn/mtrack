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
import { DashboardPage } from "../pages/dashboard.page.js";
import { PLAYBACK_STATE } from "../mock-server/test-data.js";

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

test.describe("Track Gain Sliders", () => {
  let dashboard: DashboardPage;
  let wsId: string;

  test.beforeEach(async ({ page }) => {
    wsId = `gain-${++testCounter}-${Date.now()}`;

    // Stub the gRPC endpoint so gain changes succeed.
    await page.route("**/player.v1.PlayerService/**", async (route) => {
      await route.fulfill({
        status: 200,
        headers: {
          "content-type": "application/grpc-web+proto",
          "grpc-status": "0",
        },
        body: Buffer.alloc(0),
      });
    });

    dashboard = new DashboardPage(page);
    await page.goto(`/?wsId=${wsId}#/`);
    await expect(dashboard.trackRows.first()).toBeVisible();
  });

  test("renders a slider per track with dB readouts", async () => {
    await expect(dashboard.gainSliders).toHaveCount(3);
    // Mock data: kick 0 dB, snare -6 dB, bass -60 dB (muted → -∞).
    await expect(dashboard.gainReadouts.nth(0)).toHaveText("0.0 dB");
    await expect(dashboard.gainReadouts.nth(1)).toHaveText("-6.0 dB");
    await expect(dashboard.gainReadouts.nth(2)).toHaveText("-∞");
  });

  test("adjusting a slider fires SetTrackGain and updates the readout", async ({
    page,
  }) => {
    const requestPromise = page.waitForRequest((req) =>
      req.url().includes("/player.v1.PlayerService/SetTrackGain"),
    );

    // Keyboard interaction reliably moves a range input and fires both
    // input and change events.
    const kick = dashboard.gainSliders.nth(0);
    await kick.focus();
    await kick.press("ArrowUp");

    await requestPromise;
    await expect(dashboard.gainReadouts.nth(0)).toHaveText("+0.5 dB");
  });

  test("double-click resets gain to 0 dB", async ({ page }) => {
    const requestPromise = page.waitForRequest((req) =>
      req.url().includes("/player.v1.PlayerService/SetTrackGain"),
    );

    const snare = dashboard.gainSliders.nth(1);
    await expect(dashboard.gainReadouts.nth(1)).toHaveText("-6.0 dB");
    await snare.dblclick();

    await requestPromise;
    await expect(dashboard.gainReadouts.nth(1)).toHaveText("0.0 dB");
  });

  test("mute button fires SetTrackMute and lights up from server state", async ({
    page,
  }) => {
    const muteButtons = page.locator(".tracks-card__mute");
    await expect(muteButtons).toHaveCount(3);
    await expect(muteButtons.nth(0)).toHaveAttribute("aria-pressed", "false");

    const requestPromise = page.waitForRequest((req) =>
      req.url().includes("/player.v1.PlayerService/SetTrackMute"),
    );
    await muteButtons.nth(0).click();
    await requestPromise;

    // Mute state is server-authoritative: the button reflects the next
    // WS frame, and the gain readout is untouched.
    await sendWsMessage(page, wsId, {
      ...PLAYBACK_STATE,
      tracks: [
        { name: "kick", output_channels: [0, 1], gain_db: 0, muted: true },
        { name: "snare", output_channels: [2, 3], gain_db: -6 },
        { name: "bass", output_channels: [4, 5], gain_db: -60 },
      ],
    });
    await expect(muteButtons.nth(0)).toHaveAttribute("aria-pressed", "true");
    await expect(dashboard.gainReadouts.nth(0)).toHaveText("0.0 dB");
  });

  test("server-pushed gain updates the slider when idle", async ({ page }) => {
    await sendWsMessage(page, wsId, {
      ...PLAYBACK_STATE,
      tracks: [
        { name: "kick", output_channels: [0, 1], gain_db: 3 },
        { name: "snare", output_channels: [2, 3], gain_db: -6 },
        { name: "bass", output_channels: [4, 5], gain_db: -60 },
      ],
    });

    await expect(dashboard.gainReadouts.nth(0)).toHaveText("+3.0 dB");
  });
});
