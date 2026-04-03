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

test.describe("File Upload - Track Upload", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/#/songs/Test%20Song%20Alpha");
    await expect(page.locator(".tab.active")).toContainText("Tracks");
    // Wait for track editor and drop zone to fully render
    await expect(page.locator(".track-editor")).toBeVisible();
    await expect(page.locator(".drop-zone")).toBeVisible({ timeout: 10000 });
  });

  test("track upload via file input triggers upload", async ({ page }) => {
    // The hidden file input triggers uploadTrack which PUTs to the mock server
    const fileInput = page.locator('.drop-zone input[type="file"]');
    await fileInput.setInputFiles({
      name: "test-track.wav",
      mimeType: "audio/wav",
      buffer: Buffer.from("fake-audio-data"),
    });

    // After upload, should show success or the song reloads
    // Wait briefly for the async upload to complete
    await page.waitForTimeout(500);

    // The upload should not show an error message
    const errorMsg = page.locator(".msg.error");
    await expect(errorMsg).not.toBeVisible();
  });

  test("drop zone has correct accept attribute for audio", async ({ page }) => {
    const fileInput = page.locator('.drop-zone input[type="file"]');
    const accept = await fileInput.getAttribute("accept");
    expect(accept).toContain(".wav");
    expect(accept).toContain(".flac");
    expect(accept).toContain(".mp3");
  });
});

test.describe("File Upload - MIDI Upload", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/#/songs/Test%20Song%20Alpha/tracks");
    await expect(page.locator(".tab.active")).toContainText("Tracks");
    // Expand the MIDI collapsible section
    await page.locator(".collapsible-header", { hasText: "MIDI" }).click();
  });

  test("MIDI section drop zone accepts .mid files", async ({ page }) => {
    const fileInput = page.locator(
      '.collapsible-body .drop-zone input[type="file"]',
    );
    const accept = await fileInput.getAttribute("accept");
    expect(accept).toContain(".mid");
  });
});

test.describe("File Upload - Sample Upload", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/#/config");
    // Add a sample to get the upload area
    await page.getByRole("button", { name: "Add Sample" }).click();
    await expect(page.locator(".sample-card")).toBeVisible();
  });

  test("sample card shows file upload area", async ({ page }) => {
    await expect(page.locator(".sample-card .drop-zone")).toBeVisible();
  });

  test("sample card has file input field", async ({ page }) => {
    // Verify the sample card has a file-related input
    await expect(
      page.locator('.sample-card input[id^="sample-file-"]'),
    ).toBeVisible();
  });
});
