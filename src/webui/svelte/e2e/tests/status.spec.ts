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
import type { Status } from "../../src/gen/rules/mtrack_rules";
import { StatusPageModel } from "../pages/status.page.js";
import { STATUS } from "../mock-server/test-data";
import { sendWatch, type WatchPush } from "../support/watch";
import { Code } from "@connectrpc/connect";
import { ConfigService } from "../../src/gen/player/v1/player_pb";
import { restartControllersReply } from "../support/config-rpc";
import { mockRpc } from "../support/rpc";

let testCounter = 0;

/** The mock's status with `change` made to a copy, as the player's
 *  `hardware` event says it. */
function hardware(change: (status: Status) => void): WatchPush {
  const status: Status = structuredClone(STATUS);
  change(status);
  return { type: "hardware", ...status };
}

test.describe("Status Page", () => {
  let status: StatusPageModel;
  let wsId: string;

  test.beforeEach(async ({ page }) => {
    // The worker index keeps ids unique across parallel workers.
    wsId = `status-${test.info().parallelIndex}-${++testCounter}-${Date.now()}`;
    status = new StatusPageModel(page);
    await status.goto(wsId);
  });

  test("shows Status heading", async () => {
    await expect(status.heading).toBeVisible();
  });

  test("shows refresh button", async () => {
    await expect(status.refreshButton).toBeVisible();
  });

  test("shows three cards", async () => {
    await expect(status.cards).toHaveCount(3);
  });

  test("shows build info card with version", async () => {
    await expect(status.buildInfoCard).toBeVisible();
    await expect(status.buildInfoCard).toContainText("0.1.0-test");
  });

  test("shows git hash", async () => {
    await expect(status.buildInfoCard).toContainText("deadbeef");
  });

  test("shows build time", async () => {
    await expect(status.buildInfoCard).toContainText("2026-01-01");
  });

  test("shows hardware card", async () => {
    await expect(status.hardwareCard).toBeVisible();
  });

  test("shows hostname", async () => {
    await expect(status.hardwareCard).toContainText("test-host");
  });

  test("shows audio subsystem as connected", async () => {
    const audioRow = status.subsystemRow("Audio");
    await expect(audioRow).toBeVisible();
    await expect(audioRow).toContainText("Connected");
  });

  test("shows audio device name", async () => {
    const audioRow = status.subsystemRow("Audio");
    await expect(audioRow).toContainText("Default Audio Device");
  });

  test("shows MIDI subsystem as not configured", async () => {
    const midiRow = status.subsystemRow("MIDI");
    await expect(midiRow).toBeVisible();
    await expect(midiRow).toContainText("Not Configured");
  });

  test("shows DMX subsystem as not configured", async () => {
    const dmxRow = status.subsystemRow("DMX");
    await expect(dmxRow).toBeVisible();
    await expect(dmxRow).toContainText("Not Configured");
  });

  test("shows Trigger subsystem as not configured", async () => {
    const triggerRow = status.subsystemRow("Trigger");
    await expect(triggerRow).toBeVisible();
    await expect(triggerRow).toContainText("Not Configured");
  });

  test("shows controllers card", async () => {
    await expect(status.controllersCard).toBeVisible();
  });

  test("shows OSC controller as running", async () => {
    await expect(status.controllersCard).toContainText("OSC");
    await expect(status.controllersCard).toContainText("Running");
    await expect(status.controllersCard).toContainText("0.0.0.0:9000");
  });

  test("shows restart button for controllers", async () => {
    await expect(status.restartButton).toBeVisible();
  });

  test("refresh button fetches status", async () => {
    await status.refreshButton.click();
    // After refresh, data should still be displayed
    await expect(status.buildInfoCard).toContainText("0.1.0-test");
  });

  test("restart controllers calls the player", async ({ page }) => {
    const rpc = await mockRpc(page);
    rpc.unary(
      ConfigService.method.restartControllers,
      restartControllersReply([
        {
          kind: "osc",
          status: "running",
          detail: "0.0.0.0:9000",
          error: null,
        },
      ]),
    );

    await status.restartButton.click();
    await expect
      .poll(() => rpc.calls(ConfigService.method.restartControllers).length)
      .toBe(1);
  });

  test("a refused restart says why", async ({ page }) => {
    const rpc = await mockRpc(page);
    rpc.error(
      ConfigService.method.restartControllers,
      Code.FailedPrecondition,
      "Cannot restart controllers during playback",
    );

    await status.restartButton.click();
    await expect(
      page.getByText("Cannot restart controllers during playback"),
    ).toBeVisible();
  });

  test("shows error banner when the Watch stream fails", async ({ page }) => {
    // A stream the player answers with an error: the page opens it as it
    // starts, so the route goes in before the page loads.
    await page.route("**/player.v1.PlayerService/Watch", (route) =>
      route.fulfill({
        status: 200,
        headers: {
          "content-type": "application/grpc-web+proto",
          "grpc-status": "13",
          "grpc-message": "Internal server error",
        },
        body: "",
      }),
    );

    await page.goto("/#/status");
    const freshStatus = new StatusPageModel(page);
    await expect(freshStatus.errorBanner).toBeVisible();
    await expect(freshStatus.errorBanner).toContainText(
      "Failed to load status",
    );
    await page.unrouteAll({ behavior: "ignoreErrors" });
  });

  /** The failure this whole surface exists for.
   *
   * `audio.status` stays `connected` while the interface is physically gone —
   * it only means a device handle exists. On the rig, unplugging the interface
   * mid-song left it reporting `connected` for the entire outage. If the page
   * takes that at face value it shows a green dot to someone standing in a
   * silent room. */
  test("audio row reports a stalled callback, not a green Connected", async ({
    page,
  }) => {
    await sendWatch(
      page,
      wsId,
      hardware((s) => {
        s.hardware.audio_output = {
          status: "stalled",
          callback_alive: false,
          writing_signal: false,
          since_last_callback_ms: 4200,
          since_last_signal_ms: 4200,
          callbacks: 1477,
          recoveries: 0,
          underruns: 0,
          last_error: null,
        };
      }),
    );

    const audioRow = page
      .locator(".subsystem-row")
      .filter({ hasText: "Audio" });
    await expect(audioRow).toContainText("Open, but not playing out");
    await expect(audioRow).not.toContainText("Connected");
  });

  /** The point of coming up degraded: init gives up on a device that will never
   * open, so the rest of the rig starts and the operator can reach this page to
   * fix the config. The reason has to be here — "Failed" alone leaves them
   * exactly where the perpetual retry did. */
  test("a subsystem that gave up reports Failed with the reason", async ({
    page,
  }) => {
    await sendWatch(
      page,
      wsId,
      hardware((s) => {
        s.hardware.init_done = true;
        s.hardware.audio = {
          status: "failed",
          name: null,
          error:
            "'hw:CARD=WING,DEV=0' was found but could not be opened: Unsupported bit depth for integer format: 24",
        };
        s.hardware.audio_output = null;
      }),
    );

    const audioRow = page
      .locator(".subsystem-row")
      .filter({ hasText: "Audio" });
    await expect(audioRow).toContainText("Failed");
    // "Not Configured" is what a failed row falls back to if the label order is
    // wrong, and it tells the operator there is nothing to fix.
    await expect(audioRow).not.toContainText("Not Configured");
    await expect(page.locator(".subsystem-error")).toContainText(
      "Unsupported bit depth",
    );
  });

  /** Audio that came up with a profile mapping tracks past the device's
   * channels stays connected and carries the reason; the operator needs the
   * way to the profile as much as for a failure. A clean connected row has no
   * link. */
  test("a connected subsystem with a startup problem offers Fix", async ({
    page,
  }) => {
    const audioRow = page
      .locator(".subsystem-row")
      .filter({ hasText: "Audio" });
    await expect(audioRow).toContainText("Connected");
    await expect(audioRow.locator(".subsystem-link")).toHaveCount(0);

    await sendWatch(
      page,
      wsId,
      hardware((s) => {
        s.hardware.audio = {
          ...s.hardware.audio,
          status: "connected",
          error:
            "audio device alsa:plughw:CARD=Headphones,DEV=0 has 8 channels, but the profile maps tracks past them (bass -> 9); songs using those tracks will not play",
        };
      }),
    );

    await expect(audioRow).toContainText("Connected");
    await expect(page.locator(".subsystem-error")).toContainText(
      "maps tracks past them",
    );
    await expect(
      audioRow.getByRole("link", { name: "Fix Audio" }),
    ).toBeVisible();
  });

  /** A rig whose audio gave up must not read as healthy anywhere. `init_done`
   * now fires on a degraded rig, so the nav indicator no longer has the
   * "still initializing" amber to hide behind. */
  test("a failed subsystem shows as an error in the nav health indicator", async ({
    page,
  }) => {
    await sendWatch(
      page,
      wsId,
      hardware((s) => {
        s.hardware.init_done = true;
        s.hardware.audio = {
          status: "failed",
          name: null,
          error: "'hw:CARD=WING,DEV=0' was found but could not be opened",
        };
        s.hardware.audio_output = null;
      }),
    );

    // The indicator carries its verdict in the class and the accessible label;
    // the label is what a person actually reads out.
    await expect(page.locator(".topnav__conn--error")).toBeVisible();
  });

  test("audio row distinguishes recovering from stalled", async ({ page }) => {
    await sendWatch(
      page,
      wsId,
      hardware((s) => {
        s.hardware.audio_output = {
          status: "recovering",
          callback_alive: false,
          writing_signal: false,
          since_last_callback_ms: 900,
          since_last_signal_ms: 900,
          callbacks: 1477,
          recoveries: 2,
          underruns: 0,
          last_error: "ALSA function 'snd_pcm_open' failed",
        };
      }),
    );

    const audioRow = page
      .locator(".subsystem-row")
      .filter({ hasText: "Audio" });
    await expect(audioRow).toContainText("Reconnecting to the interface");
    // The after-the-fact half: a rig that rebuilt its stream mid-set reads as
    // healthy again the moment it succeeds, so the count and the reason are the
    // only trace left.
    await expect(audioRow).toContainText("2 stream recoveries");
    await expect(audioRow).toContainText("snd_pcm_open");
  });

  /** A healthy device must look ordinary: the fault styling is worthless if it
   * fires during the few milliseconds before the first callback lands, or on a
   * device that is simply idle between songs. */
  test("healthy audio still reads as Connected", async () => {
    const audioRow = status.subsystemRows.filter({ hasText: "Audio" });
    await expect(audioRow).toContainText("Connected");
  });
});
