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
import { NavPage } from "../pages/nav.page.js";

test.describe("Navigation", () => {
  let nav: NavPage;

  test.beforeEach(async ({ page }) => {
    nav = new NavPage(page);
    await nav.goto();
  });

  test("shows brand text", async () => {
    await expect(nav.brand).toHaveText("mtrack");
  });

  test("shows all nav links", async () => {
    await expect(nav.dashboardLink).toBeVisible();
    await expect(nav.configLink).toBeVisible();
    await expect(nav.songsLink).toBeVisible();
    await expect(nav.playlistsLink).toBeVisible();
    await expect(nav.statusLink).toBeVisible();
  });

  test("dashboard link is active by default", async () => {
    await expect(nav.activeLink()).toHaveAttribute("href", "#/");
  });

  test("highlights active page on navigation", async () => {
    await nav.songsLink.click();
    await expect(nav.activeLink()).toHaveAttribute("href", "#/songs");

    await nav.playlistsLink.click();
    await expect(nav.activeLink()).toHaveAttribute("href", "#/playlists");

    await nav.configLink.click();
    await expect(nav.activeLink()).toHaveAttribute("href", "#/config");

    await nav.statusLink.click();
    await expect(nav.activeLink()).toHaveAttribute("href", "#/status");

    await nav.dashboardLink.click();
    await expect(nav.activeLink()).toHaveAttribute("href", "#/");
  });

  test("page title updates on navigation", async ({ page }) => {
    await expect(page).toHaveTitle(/Dashboard - mtrack/);

    await nav.songsLink.click();
    await expect(page).toHaveTitle(/Songs - mtrack/);

    await nav.playlistsLink.click();
    await expect(page).toHaveTitle(/Playlists - mtrack/);

    await nav.configLink.click();
    await expect(page).toHaveTitle(/Config - mtrack/);

    await nav.statusLink.click();
    await expect(page).toHaveTitle(/Status - mtrack/);
  });

  test("unknown hash shows Not Found", async ({ page }) => {
    await page.goto("/#/unknown-route");
    await expect(page.getByText("Not Found")).toBeVisible();
  });

  test("WebSocket status indicator shows connected", async () => {
    await expect(nav.statusIndicator).toBeVisible();
    await expect(nav.statusIndicator).toHaveClass(/connected/);
  });

  test("lock toggle button is visible", async () => {
    await expect(nav.lockToggle).toBeVisible();
  });

  test("now-playing shows song name from WebSocket", async () => {
    await expect(nav.nowPlayingSong).toBeVisible();
    await expect(nav.nowPlayingSong).toHaveText("Test Song Alpha");
  });
});
