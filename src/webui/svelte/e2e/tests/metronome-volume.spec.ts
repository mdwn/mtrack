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
// Where the master click volume lives.
//
// Click levels are a property of the rig, so `metronome.volume` in
// `mtrack.yaml` is the level every song plays at; a song overrides it by
// setting its own. The two halves of that are only visible on the wire: the
// Config page has to send the player-wide volume, and a song that hasn't
// overridden it must not write a `volume:` key at all — an unset song volume
// and an explicit `1.0` mean different things once the player sets 0.8.

import { test, expect, type Page } from "@playwright/test";
import { parse } from "yaml";

const CONFIG_YAML = `songs: songs
profiles:
  - hostname: test-host
    audio:
      device: default
samples: {}
metronome:
  enabled: true
  volume: 0.8
`;

const SONG = "Test Song Beta";
const ENC = encodeURIComponent(SONG);

/** A song with a metronome block that does not set a volume. */
const SONG_YAML = `name: ${SONG}
tracks:
  - name: guitar
    file: guitar.wav
tempo:
  bpm: 120
  time_signature: 4/4
metronome:
  track: metronome
`;

interface MetronomeDefaults {
  enabled?: boolean;
  volume?: number;
  sounds?: Record<string, unknown>;
}

async function openConfig(page: Page) {
  await page.route("**/api/config/store", async (route) => {
    const res = await route.fetch();
    const data = await res.json();
    await route.fulfill({
      response: res,
      json: { ...data, yaml: CONFIG_YAML },
    });
  });
  await page.goto("/#/config");
  await expect(page.locator(".metronome-defaults")).toBeVisible();
}

async function openSong(page: Page) {
  await page.route(`**/api/songs/${ENC}`, async (route) => {
    if (route.request().method() !== "GET") {
      await route.fulfill({ status: 200, body: "{}" });
      return;
    }
    await route.fulfill({
      status: 200,
      contentType: "text/yaml",
      body: SONG_YAML,
    });
  });
  // The panel lives under the timeline, with the rest of the click editing.
  await page.goto(`/#/songs/${ENC}/sections`);
  await expect(page.locator(".metronome-editor")).toBeVisible();
}

/** The `metronome` block of the song YAML the UI PUTs when Save is clicked. */
async function saveAndReadMetronome(
  page: Page,
): Promise<Record<string, unknown> | undefined> {
  const request = page.waitForRequest(
    (req) => req.url().includes(`/api/songs/${ENC}`) && req.method() === "PUT",
  );
  await page.getByRole("button", { name: "Save" }).first().click();
  const body = (await request).postData() ?? "";
  return (parse(body) as { metronome?: Record<string, unknown> }).metronome;
}

test.describe("Master click volume", () => {
  test("the Config page loads and saves the player-wide volume", async ({
    page,
  }) => {
    await openConfig(page);
    const master = page
      .locator(".metronome-defaults .master-volume")
      .getByRole("textbox");
    await expect(master).toHaveValue("0.80");

    await master.fill("0.5");
    await master.dispatchEvent("change");

    const request = page.waitForRequest(
      (req) =>
        req.url().includes("/api/config/metronome") && req.method() === "PUT",
    );
    await page.getByRole("button", { name: /Save Metronome/i }).click();
    const body = JSON.parse((await request).postData() ?? "{}") as {
      metronome: MetronomeDefaults | null;
    };
    expect(body.metronome?.volume).toBe(0.5);
    // Still default-on: the volume rides along with the rest of the block.
    expect(body.metronome?.enabled).toBe(true);
  });

  test("a song inherits the volume until it overrides it", async ({ page }) => {
    await openSong(page);
    const level = page.locator(".metronome-editor .level-field");
    // Nothing to show but the note: the song has no volume of its own.
    await expect(level.locator(".inherited-note")).toBeVisible();
    await expect(level.locator(".slider-stepper")).toHaveCount(0);

    // Overriding stores the level even at 1x, which is the whole point: the
    // player is at 0.8 and this song wants full.
    await level.getByRole("checkbox").check();
    await expect(level.locator(".slider-stepper")).toBeVisible();
    expect(await saveAndReadMetronome(page)).toMatchObject({ volume: 1 });

    // And going back to inherited drops the key rather than pinning 1x.
    await level.getByRole("checkbox").uncheck();
    await expect(level.locator(".inherited-note")).toBeVisible();
    expect(await saveAndReadMetronome(page)).not.toHaveProperty("volume");
  });
});
