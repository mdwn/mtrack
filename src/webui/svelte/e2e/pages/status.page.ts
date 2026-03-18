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

import type { Page, Locator } from "@playwright/test";

export class StatusPageModel {
  readonly heading: Locator;
  readonly refreshButton: Locator;
  readonly restartButton: Locator;
  readonly errorBanner: Locator;
  readonly cards: Locator;
  readonly buildInfoCard: Locator;
  readonly hardwareCard: Locator;
  readonly controllersCard: Locator;
  readonly subsystemRows: Locator;
  readonly version: Locator;

  constructor(private page: Page) {
    this.heading = page.getByRole("heading", { name: "Status" });
    this.refreshButton = page.getByRole("button", { name: "Refresh" });
    this.restartButton = page.getByRole("button", { name: "Restart" });
    this.errorBanner = page.locator(".error-banner");
    this.cards = page.locator(".cards .card");
    this.buildInfoCard = page
      .locator(".card")
      .filter({ hasText: "Build Info" });
    this.hardwareCard = page.locator(".card").filter({ hasText: "Hardware" });
    this.controllersCard = page
      .locator(".card")
      .filter({ hasText: "Controllers" });
    this.subsystemRows = page.locator(".subsystem-row");
    this.version = page.locator(".info-value").first();
  }

  async goto() {
    await this.page.goto("/#/status");
  }

  subsystemRow(label: string): Locator {
    return this.page.locator(".subsystem-row", { hasText: label });
  }
}
