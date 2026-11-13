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

import { test, expect, type Page } from "@playwright/test";
import { PLAYBACK_STATE } from "../mock-server/test-data";
import { sendWatch } from "../support/watch";

test.describe("Config Save and Reload", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/#/config");
    await page.locator(".profile-row", { hasText: "test-host" }).click();
    await expect(page.getByRole("button", { name: "Back" })).toBeVisible();
  });

  test("save sends expected_checksum from initial load", async ({ page }) => {
    let sentChecksum = "";
    await page.route("**/api/config/profiles/0", async (route) => {
      if (route.request().method() === "PUT") {
        const body = JSON.parse(route.request().postData() || "{}");
        sentChecksum = body.expected_checksum;
        await route.fulfill({
          status: 200,
          contentType: "application/json",
          body: JSON.stringify({
            yaml: "profiles:\n  - hostname: new-hostname\n    audio:\n      device: default\nsamples: {}\n",
            checksum: "updated-checksum-1",
          }),
        });
      } else {
        await route.continue();
      }
    });

    // Change hostname to make dirty
    const hostnameInput = page.locator("#profile-hostname");
    await hostnameInput.fill("new-hostname");
    // oninput triggers dirty state immediately, no blur needed

    await page.getByRole("button", { name: "Save" }).click();

    // Should have sent the original checksum
    expect(sentChecksum).toBe("abc123def456");
  });

  test("save shows success message", async ({ page }) => {
    await page.route("**/api/config/profiles/0", async (route) => {
      if (route.request().method() === "PUT") {
        await route.fulfill({
          status: 200,
          contentType: "application/json",
          body: JSON.stringify({
            yaml: "profiles:\n  - hostname: new-hostname\n    audio:\n      device: default\nsamples: {}\n",
            checksum: "updated-checksum-1",
          }),
        });
      } else {
        await route.continue();
      }
    });

    const hostnameInput = page.locator("#profile-hostname");
    await hostnameInput.fill("new-hostname");
    // oninput triggers dirty state immediately, no blur needed

    await page.getByRole("button", { name: "Save" }).click();

    // Should show "Saved" message
    await expect(page.locator(".save-msg")).toContainText("Saved");
  });

  test("save disables button after successful save", async ({ page }) => {
    await page.route("**/api/config/profiles/0", async (route) => {
      if (route.request().method() === "PUT") {
        await route.fulfill({
          status: 200,
          contentType: "application/json",
          body: JSON.stringify({
            yaml: "profiles:\n  - hostname: new-hostname\n    audio:\n      device: default\nsamples: {}\n",
            checksum: "updated-checksum-2",
          }),
        });
      } else {
        await route.continue();
      }
    });

    const hostnameInput = page.locator("#profile-hostname");
    await hostnameInput.fill("new-hostname");
    // oninput triggers dirty state immediately, no blur needed

    await page.getByRole("button", { name: "Save" }).click();
    await expect(page.locator(".save-msg")).toContainText("Saved");

    // Save button should be disabled again (no longer dirty)
    await expect(page.getByRole("button", { name: "Save" })).toBeDisabled();
  });

  test("second save uses updated checksum", async ({ page }) => {
    let saveCount = 0;
    const sentChecksums: string[] = [];

    await page.route("**/api/config/profiles/0", async (route) => {
      if (route.request().method() === "PUT") {
        const body = JSON.parse(route.request().postData() || "{}");
        sentChecksums.push(body.expected_checksum);
        saveCount++;
        await route.fulfill({
          status: 200,
          contentType: "application/json",
          body: JSON.stringify({
            yaml: `profiles:\n  - hostname: save-${saveCount}\n    audio:\n      device: default\nsamples: {}\n`,
            checksum: `checksum-after-save-${saveCount}`,
          }),
        });
      } else {
        await route.continue();
      }
    });

    // First save
    const hostnameInput = page.locator("#profile-hostname");
    await hostnameInput.fill("save-1");
    // oninput triggers dirty state immediately, no blur needed
    await page.getByRole("button", { name: "Save" }).click();
    await expect(page.locator(".save-msg")).toContainText("Saved");

    // Second save
    await hostnameInput.fill("save-2");
    // oninput triggers dirty state immediately, no blur needed
    await page.getByRole("button", { name: "Save" }).click();
    await expect(page.locator(".save-msg")).toContainText("Saved");

    // First save should use original checksum, second should use updated one
    expect(sentChecksums[0]).toBe("abc123def456");
    expect(sentChecksums[1]).toBe("checksum-after-save-1");
  });

  test("save error shows error message", async ({ page }) => {
    await page.route("**/api/config/profiles/0", async (route) => {
      if (route.request().method() === "PUT") {
        await route.fulfill({
          status: 409,
          contentType: "application/json",
          body: JSON.stringify({
            error: "Checksum mismatch: config was modified",
          }),
        });
      } else {
        await route.continue();
      }
    });

    const hostnameInput = page.locator("#profile-hostname");
    await hostnameInput.fill("conflict-hostname");
    // oninput triggers dirty state immediately, no blur needed

    await page.getByRole("button", { name: "Save" }).click();

    // Should show error message
    await expect(page.locator(".save-msg.save-error")).toBeVisible();
    await expect(page.locator(".save-msg")).toContainText(
      /checksum|mismatch|modified/i,
    );
  });

  // A save while a song plays is kept and its hardware reload waits for the
  // player to be idle: "Saved" alone read as a change already in effect.
  test("a save deferred under playback says the hardware reloads later", async ({
    page,
  }) => {
    await page.route("**/api/config/profiles/0", async (route) => {
      if (route.request().method() === "PUT") {
        await route.fulfill({
          status: 200,
          contentType: "application/json",
          body: JSON.stringify({
            yaml: "profiles:\n  - hostname: new-hostname\n    audio:\n      device: default\nsamples: {}\n",
            checksum: "updated-checksum-1",
            applied: false,
            reload: "deferred",
            message:
              "The config was saved. A song is playing or paused, so the hardware reload waits until the player is idle, and the change takes effect then.",
          }),
        });
      } else {
        await route.continue();
      }
    });

    await page.locator("#profile-hostname").fill("new-hostname");
    await page.getByRole("button", { name: "Save" }).click();

    const notice = page.locator(".save-msg.save-notice");
    await expect(notice).toHaveText(
      "Saved. The hardware reloads when playback stops.",
    );
    // It stays, rather than passing like "Saved".
    await page.waitForTimeout(2500);
    await expect(notice).toBeVisible();
  });

  test("a save whose hardware reload failed says why", async ({ page }) => {
    await page.route("**/api/config/profiles/0", async (route) => {
      if (route.request().method() === "PUT") {
        await route.fulfill({
          status: 200,
          contentType: "application/json",
          body: JSON.stringify({
            yaml: "profiles:\n  - hostname: new-hostname\n    audio:\n      device: default\nsamples: {}\n",
            checksum: "updated-checksum-1",
            applied: false,
            reload: "failed",
            message:
              "The config was saved, but the hardware reload failed, so the change takes effect at the next reload or restart: no store",
          }),
        });
      } else {
        await route.continue();
      }
    });

    await page.locator("#profile-hostname").fill("new-hostname");
    await page.getByRole("button", { name: "Save" }).click();

    await expect(page.locator(".save-msg.save-error")).toContainText(
      "hardware reload failed",
    );
    await expect(page.locator(".save-msg.save-error")).toContainText(
      "no store",
    );
  });
});

let wsCounter = 0;

/** Sends a playback update with the deferred reloads started (and the
 *  last refused). */
async function sendReloads(
  page: Page,
  wsId: string,
  started: number,
  refused: { generation: number; error: string }[] = [],
  processId = "process-a",
) {
  await sendWatch(page, wsId, {
    ...PLAYBACK_STATE,
    hardware_reload_started: started,
    hardware_reload_refused: refused,
    process_id: processId,
  });
}

/** Opens test-host's profile on a page of its own Watch stream, and answers
 *  its save as deferred, waiting for reload generation `awaits` (with the
 *  reply's other fields from `extra`). */
async function openDeferredSave(
  page: Page,
  awaits: number,
  extra: Record<string, unknown> = {},
): Promise<string> {
  const wsId = `deferred-${test.info().parallelIndex}-${++wsCounter}-${Date.now()}`;
  await page.goto(`/?wsId=${wsId}#/config`);
  await page.locator(".profile-row", { hasText: "test-host" }).click();
  await page.route("**/api/config/profiles/0", async (route) => {
    if (route.request().method() === "PUT") {
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({
          yaml: "profiles:\n  - hostname: new-hostname\n    audio:\n      device: default\nsamples: {}\n",
          checksum: "updated-checksum-1",
          applied: false,
          reload: "deferred",
          awaits_reload: awaits,
          awaits_process: "process-a",
          ...extra,
        }),
      });
    } else {
      await route.continue();
    }
  });
  await page.locator("#profile-hostname").fill("new-hostname");
  await page.getByRole("button", { name: "Save" }).click();
  return wsId;
}

test.describe("Config save deferred under playback", () => {
  // The notice is about the wait for playback to stop: it goes once the
  // reload it waits for has started. What the reload makes of each device
  // is the status page's to show.
  test("the deferred notice goes once its reload has started", async ({
    page,
  }) => {
    const wsId = await openDeferredSave(page, 3);
    const notice = page.locator(".save-msg.save-notice");
    await expect(notice).toHaveText(
      "Saved. The hardware reloads when playback stops.",
    );
    await sendReloads(page, wsId, 2);
    await page.waitForTimeout(300);
    await expect(notice).toBeVisible();
    await sendReloads(page, wsId, 3);
    await expect(notice).toHaveCount(0);
  });

  // A stop before the first status update: the reload has started, and
  // more besides, by the first update the page sees.
  test("a stop before the first status update clears the notice", async ({
    page,
  }) => {
    const wsId = await openDeferredSave(page, 1);
    await expect(page.locator(".save-msg.save-notice")).toBeVisible();
    await sendReloads(page, wsId, 4);
    await expect(page.locator(".save-msg.save-notice")).toHaveCount(0);
    await expect(page.locator(".save-msg.save-error")).toHaveCount(0);
  });

  // A reload that could not start at all is known at once, and said.
  test("a deferred reload that could not start says why", async ({ page }) => {
    const wsId = await openDeferredSave(page, 1);
    await sendReloads(page, wsId, 1, [
      { generation: 1, error: "No config store available" },
    ]);
    const failed = page.locator(".save-msg.save-error");
    await expect(failed).toContainText("could not start");
    await expect(failed).toContainText("No config store available");
  });

  // A save whose song reload failed beside a deferred reload says both,
  // and, when that reload could not start either, both failures.
  test("both failures are shown", async ({ page }) => {
    const wsId = await openDeferredSave(page, 1, {
      songs_error: "failed to rescan songs: unreadable",
    });
    const notice = page.locator(".save-msg.save-notice");
    await expect(notice).toContainText("reloads when playback stops");
    await expect(notice).toContainText("songs could not be reloaded");
    await sendReloads(page, wsId, 1, [
      { generation: 1, error: "No config store available" },
    ]);
    await expect(notice).toHaveCount(0);
    const failed = page.locator(".save-msg.save-error");
    await expect(failed).toContainText("could not start");
    await expect(failed).toContainText("songs could not be reloaded");
  });

  // A save whose song reload failed says so, not "Saved".
  test("a save whose song reload failed says why", async ({ page }) => {
    await page.goto("/#/config");
    await page.locator(".profile-row", { hasText: "test-host" }).click();
    await page.route("**/api/config/profiles/0", async (route) => {
      if (route.request().method() === "PUT") {
        await route.fulfill({
          status: 200,
          contentType: "application/json",
          body: JSON.stringify({
            yaml: "profiles:\n  - hostname: new-hostname\n    audio:\n      device: default\nsamples: {}\n",
            checksum: "updated-checksum-1",
            applied: false,
            reload: "applied",
            songs_error: "failed to rescan songs: unreadable",
            message:
              "The config was saved, but the songs could not be reloaded, so the player keeps the songs it had: failed to rescan songs: unreadable",
          }),
        });
      } else {
        await route.continue();
      }
    });
    await page.locator("#profile-hostname").fill("new-hostname");
    await page.getByRole("button", { name: "Save" }).click();
    await expect(page.locator(".save-msg.save-error")).toContainText(
      "songs could not be reloaded",
    );
  });

  // mtrack restarted under the page: its count starts over, so the notice
  // is told by the process id that it was issued under another process,
  // which the restarted one replaced, loading the saved config.
  test("a restart clears the deferred notice", async ({ page }) => {
    const wsId = await openDeferredSave(page, 3);
    const notice = page.locator(".save-msg.save-notice");
    await expect(notice).toBeVisible();
    await sendReloads(page, wsId, 0, [], "process-a");
    await page.waitForTimeout(300);
    await expect(notice).toBeVisible();
    await sendReloads(page, wsId, 0, [], "process-b");
    await expect(notice).toHaveCount(0);
    await expect(page.locator(".save-msg.save-error")).toHaveCount(0);
  });

  // A refusal of a later reload is not blamed on this save.
  test("a later reload's refusal is not this save's", async ({ page }) => {
    const wsId = await openDeferredSave(page, 1);
    await sendReloads(page, wsId, 2, [
      { generation: 2, error: "No config store available" },
    ]);
    await expect(page.locator(".save-msg.save-notice")).toHaveCount(0);
    await expect(page.locator(".save-msg.save-error")).toHaveCount(0);
  });
});
