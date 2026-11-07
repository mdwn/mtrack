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
// Two profile files sharing a stem (`stage.yaml` and `stage.yml`) both load,
// since a profile is matched by the `hostname` inside it. The config editor
// lists both, keyed by their full file names (a key on the stem threw
// `each_key_duplicate`), and each row opens its own file: the `.yml` row used
// to open the `.yaml`.

import { test, expect } from "@playwright/test";
import type {
  ProfileFile,
  ProfileFileInfo,
} from "../../src/gen/rules/mtrack_rules";

const PROFILES_DIR_YAML = `songs: songs
profiles_dir: profiles
samples: {}
`;

const info = (file: string, hostname: string | null): ProfileFileInfo => ({
  file,
  filename: "stage",
  hostname,
  has_audio: true,
  has_midi: false,
  has_dmx: false,
  has_trigger: false,
  has_controllers: false,
});

const detail = (device: string): ProfileFile => ({
  profile: { audio: { device } },
  yaml: `audio:\n  device: ${device}\n`,
  version: `v-${device}`,
});

test.describe("Profile files sharing a stem", () => {
  test("both are listed, and each row opens its own file", async ({ page }) => {
    const errors: string[] = [];
    page.on("pageerror", (e) => errors.push(e.message));

    await page.route("**/api/config/store", async (route) => {
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({
          yaml: PROFILES_DIR_YAML,
          checksum: "dir-checksum",
        }),
      });
    });
    await page.route("**/api/profiles", async (route) => {
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify([
          info("stage.yaml", "pi-a"),
          info("stage.yml", null),
        ] satisfies ProfileFileInfo[]),
      });
    });
    const reads: string[] = [];
    for (const [file, device] of [
      ["stage.yaml", "dev-a"],
      ["stage.yml", "catch-all"],
    ]) {
      await page.route(`**/api/profiles/${file}`, async (route) => {
        reads.push(file);
        await route.fulfill({
          status: 200,
          contentType: "application/json",
          body: JSON.stringify(detail(device) satisfies ProfileFile),
        });
      });
    }

    await page.goto("/#/config");
    const rows = page.locator(".profile-file-row");
    await expect(rows).toHaveCount(2);
    await expect(rows.nth(0).locator(".pf-name")).toHaveText("stage.yaml");
    await expect(rows.nth(1).locator(".pf-name")).toHaveText("stage.yml");

    await rows.nth(1).click();
    await expect(page.locator(".detail-title")).toContainText("stage.yml");
    await expect.poll(() => reads.includes("stage.yml")).toBe(true);
    expect(reads).not.toContain("stage.yaml");
    await expect(page).toHaveURL(/stage\.yml/);

    await page.getByRole("button", { name: "Back" }).click();
    await expect(rows).toHaveCount(2);
    await rows.nth(0).click();
    await expect(page.locator(".detail-title")).toContainText("stage.yaml");
    await expect.poll(() => reads.includes("stage.yaml")).toBe(true);

    expect(errors).toEqual([]);
  });
});
