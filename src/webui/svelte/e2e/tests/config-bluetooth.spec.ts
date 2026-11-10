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

// The Bluetooth controller's card: pairing a phone from the web UI. The mock
// keeps the window and the phones per test, named by a request header.

let counter = 0;

async function openBluetoothCard(page: Page): Promise<void> {
  await page.goto("/#/config");
  await page.locator(".profile-row", { hasText: "test-host" }).click();
  await page.locator(".tab", { hasText: "Controllers" }).click();
  await page.getByRole("button", { name: "Enable Controllers" }).click();
  await page.getByRole("button", { name: "Add Bluetooth" }).click();
  await expect(page.getByTestId("bluetooth-pairing")).toBeVisible();
}

test.describe("Bluetooth pairing", () => {
  test("pairing opens with a countdown, stops, and a phone is forgotten", async ({
    page,
  }) => {
    await page.setExtraHTTPHeaders({
      "x-mock-session": `bluetooth-${test.info().parallelIndex}-${++counter}-${Date.now()}`,
    });
    await openBluetoothCard(page);
    const card = page.getByTestId("bluetooth-pairing");

    await expect(card.getByText("Pairing closed")).toBeVisible();
    await card.getByRole("button", { name: "Pair a phone" }).click();
    await expect(card.getByText(/Pairing open — 1:\d\d left/)).toBeVisible();
    await expect(card.getByText(/connect to the name/)).toBeVisible();

    // The countdown moves on its own.
    const first = await card.locator(".status").first().innerText();
    await expect
      .poll(async () => card.locator(".status").first().innerText())
      .not.toBe(first);

    await card.getByRole("button", { name: "Stop pairing" }).click();
    await expect(card.getByText("Pairing closed")).toBeVisible();
    await expect(
      card.getByRole("button", { name: "Pair a phone" }),
    ).toBeVisible();

    // The paired phones: a name, or the address when there is none.
    const pixel = card.locator(".phone", { hasText: "Pixel" });
    await expect(pixel.getByText("connected")).toBeVisible();
    await expect(
      card.locator(".phone", { hasText: "AA:BB:CC:DD:EE:02" }),
    ).toBeVisible();

    await pixel.getByRole("button", { name: "Forget" }).click();
    const dialog = page.getByRole("dialog");
    await expect(dialog).toContainText("Forget Pixel?");
    await dialog.getByRole("button", { name: "Forget" }).click();
    await expect(pixel).toHaveCount(0);
    await expect(card.locator(".phone")).toHaveCount(1);
  });

  test("a controller not saved yet says to save the configuration", async ({
    page,
  }) => {
    await page.route("**/api/bluetooth", (route) =>
      route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({
          configured: false,
          running: false,
          error: null,
          pairing: { open: false, seconds_left: 0 },
          devices: [],
        }),
      }),
    );
    await openBluetoothCard(page);
    const card = page.getByTestId("bluetooth-pairing");
    await expect(
      card.getByText("Save the configuration to start Bluetooth."),
    ).toBeVisible();
    await expect(
      card.getByRole("button", { name: "Pair a phone" }),
    ).toHaveCount(0);
  });
});
