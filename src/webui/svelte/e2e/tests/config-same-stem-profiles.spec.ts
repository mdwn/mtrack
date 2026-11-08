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
// groups page; a link naming no file says so and opens none, not even the
// running profile. A new profile is saved as a create (`If-None-Match: *`),
// is a draft under the name typed, and once saved is selected by the file
// the save made, so nothing re-selects it and clears the "Saved" notice.

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
  error: null,
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
  info("stage.yaml", "test-host"),
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
      await expect(page).toHaveURL((url) => url.hash === `#/config/${file}`);
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

  test("a typed full name beside a same-stem file stays its own draft", async ({
    page,
  }) => {
    const listed = [info("stage.yaml", "pi-a")];
    const reads = await serveProfiles(page, () => listed);
    const puts: string[] = [];
    await page.route("**/api/profiles/stage.yml", async (route) => {
      if (route.request().method() !== "PUT") {
        await route.fallback();
        return;
      }
      puts.push(route.request().headers()["if-none-match"] ?? "");
      listed.push(info("stage.yml", "pi-y"));
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({ status: "saved", version: "v1" }),
      });
    });

    await page.goto("/#/config");
    await page.getByRole("button", { name: "Add Profile" }).click();
    const dialog = page.locator('[role="dialog"]');
    await dialog.locator("input").fill("stage.yml");
    await dialog.getByRole("button", { name: "Confirm" }).click();
    await expect(page).toHaveURL((url) => url.hash === "#/config/stage.yml");
    await page.locator("#profile-hostname").fill("pi-y");
    // Still the draft: `stage.yaml` was not opened over it.
    await expect(page.locator("#profile-hostname")).toHaveValue("pi-y");
    expect(reads).toEqual([]);

    await page.getByRole("button", { name: "Save", exact: true }).click();
    await expect.poll(() => puts).toEqual(["*"]);
    await expect(page).toHaveURL((url) => url.hash === "#/config/stage.yml");
    await expect(page.locator(".save-msg")).toHaveText("Saved");
    expect(reads).toEqual([]);
  });

  test("a new profile whose name reaches a file stays a draft and is refused", async ({
    page,
  }) => {
    const reads = await serveProfiles(page, () => [info("stage.yml", "pi-a")]);
    const puts: string[] = [];
    await page.route("**/api/profiles/stage", async (route) => {
      if (route.request().method() !== "PUT") {
        await route.fallback();
        return;
      }
      puts.push(route.request().headers()["if-none-match"] ?? "");
      await route.fulfill({
        status: 409,
        contentType: "application/json",
        body: JSON.stringify({
          error: "a profile file `stage.yml` already exists",
        }),
      });
    });

    await page.goto("/#/config");
    await page.getByRole("button", { name: "Add Profile" }).click();
    const dialog = page.locator('[role="dialog"]');
    await dialog.locator("input").fill("stage");
    await dialog.getByRole("button", { name: "Confirm" }).click();
    await page.locator("#profile-hostname").fill("pi-z");
    await page.getByRole("button", { name: "Save", exact: true }).click();

    await expect.poll(() => puts).toEqual(["*"]);
    await expect(page.locator(".save-error")).toContainText(
      "a profile file `stage.yml` already exists",
    );
    // `stage.yml` was never opened over the draft.
    await expect(page.locator("#profile-hostname")).toHaveValue("pi-z");
    expect(reads).toEqual([]);
  });

  test("a file that does not parse is shown with why, and not opened", async ({
    page,
  }) => {
    const reads = await serveProfiles(page, () => [
      { ...info("broken.yaml", null), error: "invalid type: sequence" },
    ]);
    await page.goto("/#/config");
    const row = page.locator(".profile-file-row");
    await expect(row.locator(".pf-error")).toHaveText("invalid type: sequence");
    await expect(row).toBeDisabled();
    expect(reads).toEqual([]);
  });

  test("a link naming no profile file says so and opens none", async ({
    page,
  }) => {
    const reads = await serveProfiles(page, () => THREE);
    await page.goto("/#/config/gone.yaml");
    await expect(page.locator(".route-missing")).toHaveText(
      'Profile file "gone.yaml" not found.',
    );
    await page.goto("/#/config/Stage.yml");
    await expect(page.locator(".route-missing")).toHaveText(
      'Profile file "Stage.yml" not found; "stage.yml" differs from it only in case.',
    );
    // The editor opened no file for either.
    expect(reads).toEqual([]);

    // The groups page picks no profile in the missing one's place: not the
    // running one (`stage.yaml`, for test-host). The lighting area reads
    // that one's lighting block for itself, so reads are not the measure;
    // the picker's selection is.
    await page.goto("/#/lighting/groups?profile=gone");
    await expect(page.locator(".param-missing")).toHaveText(
      'Profile file "gone" not found.',
    );
    await expect(page.locator("#groups-profile")).toHaveValue("");
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
      puts.push(route.request().headers()["if-none-match"] ?? "");
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

    // Sent as a create: never merged into a file already there.
    await expect.poll(() => puts).toEqual(["*"]);
    // The listed file is selected, so the route names it ...
    await expect(page).toHaveURL((url) => url.hash === "#/config/newhost.yaml");
    // ... the notice stays, and the file is not opened again over it.
    await expect(page.locator(".save-msg")).toHaveText("Saved");
    expect(reads).toEqual([]);
  });
});
