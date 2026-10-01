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
import { STATUS } from "../mock-server/test-data";

// A venue that does not load lights nothing, so it must be impossible to
// miss: a banner on every page while it stands, and a red health dot. Each
// test answers /api/status itself (page.route); the shared mock is untouched.

const REASON =
  'fixture "Brick3": mode "13: DIM" of fixture type "Astera-PixelBrick" did not load (venue "house", lighting/venues/house.venue)';

async function statusWith(
  page: Page,
  lighting_venue: { name: string | null; status: string; error: string | null },
) {
  const body = structuredClone(STATUS);
  (body.hardware as Record<string, unknown>).lighting_venue = lighting_venue;
  await page.route("**/api/status", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(body),
    }),
  );
}

test("a venue that did not load is a banner on the dashboard and in Lighting", async ({
  page,
}) => {
  await statusWith(page, { name: "house", status: "failed", error: REASON });
  await page.goto("/#/");
  const banner = page.getByTestId("venue-failed-banner");
  await expect(banner).toBeVisible();
  await expect(banner).toContainText('Venue "house" did not load:');
  await expect(banner).toContainText('fixture "Brick3"');
  await expect(banner).toContainText("no fixtures will light");
  await expect(banner).toHaveAttribute("role", "alert");
  // Not dismissible: no button to close it.
  await expect(banner.getByRole("button")).toHaveCount(0);
  await expect(page.locator(".topnav__conn")).toHaveClass(
    /topnav__conn--error/,
  );

  await banner.getByRole("link", { name: "Open the venue" }).click();
  await expect(page).toHaveURL(/#\/lighting\/venues/);
  await expect(page.getByTestId("venue-failed-banner")).toBeVisible();
});

test("a venue that loads, or none at all, shows no banner", async ({
  page,
}) => {
  await statusWith(page, { name: "house", status: "ok", error: null });
  await page.goto("/#/");
  await expect(page.locator(".topnav__conn")).toBeVisible();
  await expect(page.getByTestId("venue-failed-banner")).toHaveCount(0);
});
