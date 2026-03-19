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
});
