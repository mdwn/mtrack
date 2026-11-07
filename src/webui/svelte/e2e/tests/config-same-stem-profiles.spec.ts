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
// Profile files are addressed by their full file names. Two files sharing a
// stem (`stage.yaml`, `stage.yml`) both load, since a profile is matched by
// the `hostname` inside it, and a third may be named the first one's name
// (`stage.yaml.yaml`). The config editor lists each, keyed and linked by its
// full file name (a key on the stem threw `each_key_duplicate`), and each
// row opens its own file. An old link by stem opens the file the server's
// rule gives it (`.yaml`, else `.yml`), in the editor and on the lighting
// groups page. A new profile, once saved, is selected by the file the save
// made, so nothing re-selects it and clears the "Saved" notice.

import { test, expect, type Page } from "@playwright/test";
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
  filename: file.replace(/\.(yaml|yml)$/, ""),
  addressable: true,
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

/** Serves a directory-based config holding `files`, and each file's
 *  profile; returns the files each GET read, in order. */
async function serveProfiles(
  page: Page,
  files: () => ProfileFileInfo[],
): Promise<string[]> {
  await page.route("**/api/config/store", async (route) => {
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({ yaml: PROFILES_DIR_YAML, checksum: "dir" }),
    });
  });
  await page.route("**/api/profiles", async (route) => {
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(files() satisfies ProfileFileInfo[]),
    });
  });
  const reads: string[] = [];
  await page.route("**/api/profiles/*", async (route) => {
    const file = decodeURIComponent(
      new URL(route.request().url()).pathname.split("/").pop() ?? "",
    );
    if (route.request().method() !== "GET") {
      await route.fallback();
      return;
    }
    reads.push(file);
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(detail(`dev-${file}`) satisfies ProfileFile),
    });
  });
  return reads;
}

const THREE = [
  info("stage.yaml", "pi-a"),
  info("stage.yaml.yaml", "pi-b"),
  info("stage.yml", null),
];

test.describe("Profile files by full file name", () => {
  test("each row opens its own file", async ({ page }) => {
    const errors: string[] = [];
    page.on("pageerror", (e) => errors.push(e.message));
    const reads = await serveProfiles(page, () => THREE);

    await page.goto("/#/config");
    const rows = page.locator(".profile-file-row");
    await expect(rows).toHaveCount(3);
    for (const [i, file] of THREE.map((f) => f.file).entries()) {
      // Each label is the full file name: every stem here could be taken
      // for another file.
      await expect(rows.nth(i).locator(".pf-name")).toHaveText(file);
      await rows.nth(i).click();
      await expect(page).toHaveURL(
        new RegExp(`#/config/${file.replace(/\./g, "\\.")}$`),
      );
      await expect.poll(() => reads.at(-1)).toBe(file);
      await page.getByRole("button", { name: "Back" }).click();
      await expect(rows).toHaveCount(3);
    }
    expect(errors).toEqual([]);
  });

  test("an old stem link opens the stem's .yaml", async ({ page }) => {
    const reads = await serveProfiles(page, () => THREE);
    await page.goto("/#/config/stage");
    await expect(page.getByRole("button", { name: "Back" })).toBeVisible();
    await expect.poll(() => reads).toContain("stage.yaml");
    expect(reads).not.toContain("stage.yml");
    expect(reads).not.toContain("stage.yaml.yaml");
  });

  test("an old stem link on the groups page picks the stem's .yaml", async ({
    page,
  }) => {
    const reads = await serveProfiles(page, () => THREE);
    await page.goto("/#/lighting/groups?profile=stage");
    await expect(page.locator("#groups-profile")).toHaveValue("stage.yaml");
    await expect.poll(() => reads).toContain("stage.yaml");
    expect(reads).not.toContain("stage.yml");
  });

  test("a new profile stays selected, with its Saved notice", async ({
    page,
  }) => {
    let listed: ProfileFileInfo[] = [];
    const reads = await serveProfiles(page, () => listed);
    const puts: string[] = [];
    await page.route("**/api/profiles/newhost", async (route) => {
      if (route.request().method() !== "PUT") {
        await route.fallback();
        return;
      }
      puts.push("newhost");
      listed = [info("newhost.yaml", "newhost")];
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({ status: "saved", version: "v1" }),
      });
    });

    await page.goto("/#/config");
    await page.getByRole("button", { name: "Add Profile" }).click();
    const dialog = page.locator('[role="dialog"]');
    await dialog.locator("input").fill("newhost");
    await dialog.getByRole("button", { name: "Confirm" }).click();
    await page.locator("#profile-hostname").fill("newhost");
    await page.getByRole("button", { name: "Save", exact: true }).click();

    await expect.poll(() => puts).toEqual(["newhost"]);
    // The listed file is selected, so the route names it ...
    await expect(page).toHaveURL(/#\/config\/newhost\.yaml$/);
    // ... the notice stays, and the file is not opened again over it.
    await expect(page.locator(".save-msg")).toHaveText("Saved");
    expect(reads).toEqual([]);
  });
});
