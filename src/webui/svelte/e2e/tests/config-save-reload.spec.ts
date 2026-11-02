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

async function sendPlayback(
  page: Page,
  wsId: string,
  pending: boolean,
  failed: string | null = null,
) {
  await page.request.post("http://127.0.0.1:3111/test/send-ws", {
    data: {
      ...PLAYBACK_STATE,
      hardware_reload_pending: pending,
      hardware_reload_failed: failed,
      _wsId: wsId,
    },
  });
}

/** Opens test-host's profile on a page of its own WebSocket, and answers
 *  its save as deferred. */
async function openDeferredSave(page: Page): Promise<string> {
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
  // The notice says the hardware reloads when playback stops: once the
  // player has run the reload, it goes.
  test("the deferred notice goes once the reload has run", async ({ page }) => {
    const wsId = await openDeferredSave(page);
    const notice = page.locator(".save-msg.save-notice");
    await expect(notice).toBeVisible();

    await sendPlayback(page, wsId, true);
    await page.waitForTimeout(300);
    await expect(notice).toBeVisible();
    await sendPlayback(page, wsId, false);
    await expect(notice).toHaveCount(0);
  });

  // A deferred reload that fails when it runs is not cleared as though it
  // were applied: the notice says it failed, and why.
  test("a deferred reload that fails says so once it has run", async ({
    page,
  }) => {
    const wsId = await openDeferredSave(page);
    await expect(page.locator(".save-msg.save-notice")).toBeVisible();
    await sendPlayback(page, wsId, true);
    await page.waitForTimeout(300);
    await sendPlayback(
      page,
      wsId,
      false,
      "audio: the device could not be opened",
    );
    const failed = page.locator(".save-msg.save-error");
    await expect(failed).toContainText("The hardware reload failed");
    await expect(failed).toContainText("could not be opened");
    await expect(page.locator(".save-msg.save-notice")).toHaveCount(0);
  });

  // A save whose song reload failed beside it says so, not "Saved".
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
});
