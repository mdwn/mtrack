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

test.describe("Keyboard Shortcuts", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/#/");
    // Wait for WebSocket to deliver playback state
    await expect(page.locator(".playback-song")).toContainText(
      "Test Song Alpha",
    );
  });

  test("space bar triggers play via gRPC", async ({ page }) => {
    let grpcCalled = false;
    await page.route("**/player.v1.PlayerService/**", async (route) => {
      grpcCalled = true;
      await route.fulfill({
        status: 200,
        headers: {
          "content-type": "application/grpc-web+proto",
          "grpc-status": "0",
        },
        body: Buffer.alloc(0),
      });
    });

    await page.keyboard.press("Space");
    expect(grpcCalled).toBe(true);
  });

  test("right arrow triggers next via gRPC", async ({ page }) => {
    let urlCalled = "";
    await page.route("**/player.v1.PlayerService/**", async (route) => {
      urlCalled = route.request().url();
      await route.fulfill({
        status: 200,
        headers: {
          "content-type": "application/grpc-web+proto",
          "grpc-status": "0",
        },
        body: Buffer.alloc(0),
      });
    });

    await page.keyboard.press("ArrowRight");
    expect(urlCalled).toContain("Next");
  });

  test("left arrow triggers previous via gRPC", async ({ page }) => {
    let urlCalled = "";
    await page.route("**/player.v1.PlayerService/**", async (route) => {
      urlCalled = route.request().url();
      await route.fulfill({
        status: 200,
        headers: {
          "content-type": "application/grpc-web+proto",
          "grpc-status": "0",
        },
        body: Buffer.alloc(0),
      });
    });

    await page.keyboard.press("ArrowLeft");
    expect(urlCalled).toContain("Previous");
  });

  test("space bar does not trigger when typing in input", async ({ page }) => {
    // Navigate to songs page which has a search input
    await page.goto("/#/songs");
    await expect(page.locator(".search-input")).toBeVisible();

    let grpcCalled = false;
    await page.route("**/player.v1.PlayerService/**", async (route) => {
      grpcCalled = true;
      await route.fulfill({
        status: 200,
        headers: {
          "content-type": "application/grpc-web+proto",
          "grpc-status": "0",
        },
        body: Buffer.alloc(0),
      });
    });

    // Focus the search input and press space
    await page.locator(".search-input").focus();
    await page.keyboard.press("Space");

    // gRPC should NOT have been called
    expect(grpcCalled).toBe(false);
  });
});
