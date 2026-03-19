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

export class ConfigEditorPage {
  readonly heading: Locator;
  readonly addProfileButton: Locator;
  readonly profileList: Locator;
  readonly profileRows: Locator;
  readonly backButton: Locator;
  readonly saveButton: Locator;
  readonly deleteButton: Locator;
  readonly emptyState: Locator;
  readonly samplesHeading: Locator;

  constructor(private page: Page) {
    this.heading = page.getByRole("heading", { name: "Hardware Profiles" });
    this.addProfileButton = page.getByRole("button", { name: "Add Profile" });
    this.profileList = page.locator(".profile-list");
    // Inline profiles use .profile-row (ProfileCard component)
    this.profileRows = page.locator(".profile-row");
    this.backButton = page.getByRole("button", { name: "Back" });
    this.saveButton = page.getByRole("button", { name: "Save" });
    this.deleteButton = page.getByRole("button", { name: "Delete" });
    this.emptyState = page.locator(".empty-state");
    this.samplesHeading = page.getByRole("heading", { name: "Samples" });
  }

  async goto() {
    await this.page.goto("/#/config");
  }

  profileByName(name: string): Locator {
    return this.page.locator(".profile-row", { hasText: name });
  }
}
