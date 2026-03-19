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

    const requestPromise = page.waitForRequest((req) =>
      req.url().includes("player.v1.PlayerService"),
    );
    await page.keyboard.press("Space");
    await requestPromise;
  });

  test("right arrow triggers next via gRPC", async ({ page }) => {
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

    const requestPromise = page.waitForRequest((req) =>
      req.url().includes("Next"),
    );
    await page.keyboard.press("ArrowRight");
    await requestPromise;
  });

  test("left arrow triggers previous via gRPC", async ({ page }) => {
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

    const requestPromise = page.waitForRequest((req) =>
      req.url().includes("Previous"),
    );
    await page.keyboard.press("ArrowLeft");
    await requestPromise;
  });

  test("space bar does not trigger when typing in input", async ({ page }) => {
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

    // Even on the dashboard, focusing an input should suppress shortcuts
    // (The playlist select is an input-like element on the dashboard)
    // Navigate to songs page to find a text input
    await page.goto("/#/songs");
    await expect(page.locator(".search-input")).toBeVisible();
    await page.locator(".search-input").focus();
    await page.keyboard.press("Space");

    // gRPC should NOT have been called (wrong page + input focused)
    expect(grpcCalled).toBe(false);
  });

  test("shortcuts do not trigger on non-dashboard pages", async ({ page }) => {
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

    // Navigate to config page and press space
    await page.goto("/#/config");
    await expect(
      page.getByRole("heading", { name: "Hardware Profiles" }),
    ).toBeVisible();
    await page.keyboard.press("Space");

    // gRPC should NOT have been called
    expect(grpcCalled).toBe(false);
  });
});
