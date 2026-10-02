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

import * as fs from "node:fs";
import * as path from "node:path";
import { api, expect, FIRST_RUN_CONFIG, synthGdtf, test } from "./harness";

// A new install's first lighting, through the UI, with a DMX engine and a
// config that names no lighting directories: what the Lighting area makes
// is what the engine loads. Found on a freshly imaged Raspberry Pi, where
// the venue was "not found" until a directories setting was added by hand.

test.use({ config: FIRST_RUN_CONFIG });

interface Status {
  hardware: { lighting_venue: { name: string | null; status: string } };
}
interface Readiness {
  venue: { name: string; fixtures: number } | null;
  venue_error: unknown;
}

test("a GDTF, a venue, made current on Groups: the engine drives it, and again after a restart", async ({
  page,
  project,
}) => {
  // Import the GDTF.
  await page.goto("/#/lighting/fixtures");
  await page.locator('input[type="file"]').setInputFiles({
    name: "synth.gdtf",
    mimeType: "application/octet-stream",
    buffer: synthGdtf(),
  });
  await expect(page.getByTestId("gdtf-report")).toContainText(
    "Imported Synth Brick",
  );

  // A venue with one of it.
  await page.goto("/#/lighting/venues");
  await page.getByRole("button", { name: "New Venue" }).click();
  await page.locator("#venue-name").fill("rig");
  await page.getByRole("button", { name: "Add Fixture" }).click();
  await expect(
    page.getByTestId("venue-fixture-row").nth(0).getByTestId("venue-row-mode"),
  ).toHaveValue("8: RGBS");
  await page
    .locator(".editor-form")
    .getByRole("button", { name: "Save" })
    .click();
  await expect(page.locator(".editor-form")).toHaveCount(0);
  expect(project.exists("lighting/venues/rig.venue")).toBe(true);

  // Made current the way a user does it: the Groups page.
  await page.goto("/#/lighting/groups");
  // The config's one profile names no host: choose it, as the page asks.
  await page.locator("#groups-profile").selectOption({ index: 1 });
  await page.locator("#lighting-venue").selectOption("rig");
  await page.getByRole("button", { name: "Save", exact: true }).click();
  await expect(page.getByText("Saved", { exact: true })).toBeVisible();

  // No directory was written (the config's writer spells an unset one
  // `directories: ~`): the defaults are where it is.
  const named = /directories:(?![ \t]*~)/;
  expect(project.read("mtrack.yaml")).not.toMatch(named);
  for (const file of fsProfiles(project.dir)) expect(file).not.toMatch(named);

  const driven = async () => {
    await expect
      .poll(
        async () => {
          const s = await api<Status>(project, "/status");
          return [
            s.hardware.lighting_venue.name,
            s.hardware.lighting_venue.status,
          ];
        },
        { timeout: 15_000 },
      )
      .toEqual(["rig", "ok"]);
    const ready = await api<Readiness>(project, "/lighting/readiness");
    expect(ready.venue_error).toBeNull();
    expect(ready.venue?.name).toBe("rig");
    expect(ready.venue?.fixtures).toBe(1);
  };
  await driven();
  // A cold boot reads it all from the same places.
  await project.restart();
  await driven();
});

/** The text of every profile file the save may have written. */
function fsProfiles(dir: string): string[] {
  const profiles = path.join(dir, "profiles");
  if (!fs.existsSync(profiles)) return [];
  return fs
    .readdirSync(profiles)
    .map((f) => fs.readFileSync(path.join(profiles, f), "utf8"));
}
