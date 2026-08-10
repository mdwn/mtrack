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
// Where the config editor sends MIDI settings.
//
// `Player::normalize` discards the top-level `midi:` block the moment
// `profiles:` is present, so a MIDI setting written there comes back from the
// API, appears in the served config, and is thrown away on the next restart
// (#358). The editor is supposed to write into the profile that owns the
// setting instead. That is a claim about which endpoint it calls, which is
// invisible to a test that only reads the rendered form back.
//
// `persist_tempo` is the setting under test because it is new, but nothing here
// is specific to it: any MIDI field would do.

import { test, expect, type Page } from "@playwright/test";

/// The MIDI fields these tests read back off the wire.
interface MidiConfig {
  device?: string;
  beat_clock?: boolean;
  persist_tempo?: boolean;
}

/// A profile as the editor sends it.
interface ProfilePayload {
  midi?: MidiConfig;
}

/// The inline-profile endpoint nests the profile under `profile` and carries a
/// checksum; the file endpoint sends the profile on its own.
interface ProfileUpdate {
  expected_checksum?: string;
  profile?: ProfilePayload;
}

/// A config whose profiles live in a directory rather than inline. The editor
/// branches on `profiles_dir` and saves through the file API in that case.
const PROFILES_DIR_YAML = `songs: songs
profiles_dir: profiles
samples: {}
`;

/// Records every config-writing request the page makes, so a test can assert
/// what was *not* called as well as what was.
///
/// Registered last and deferring with `fallback`, not `continue`: Playwright
/// matches routes in reverse registration order, so a catch-all that continues
/// swallows every specific handler registered before it and sends the request
/// to the network instead.
async function recordConfigWrites(page: Page): Promise<string[]> {
  const writes: string[] = [];
  await page.route("**/api/**", async (route) => {
    const request = route.request();
    const method = request.method();
    if (method === "PUT" || method === "POST" || method === "DELETE") {
      writes.push(`${method} ${new URL(request.url()).pathname}`);
    }
    await route.fallback();
  });
  return writes;
}

/// Opens the MIDI tab of the first profile with the section enabled.
async function openMidiSection(page: Page, profileRow: string) {
  await page.goto("/#/config");
  await page.locator(profileRow).first().click();
  await expect(page.getByRole("button", { name: "Back" })).toBeVisible();
  await page.locator(".tab", { hasText: "MIDI" }).click();
  await expect(page.locator(".tab.active")).toContainText("MIDI");
  await page.getByRole("button", { name: "Enable MIDI" }).click();
}

test.describe("Config editor - persist_tempo", () => {
  test("persist tempo is disabled until beat clock is on", async ({ page }) => {
    await openMidiSection(page, ".profile-row");

    // The option only means anything with the beat clock running, and the UI
    // says so by refusing the click rather than accepting a setting that would
    // do nothing.
    const persistTempo = page.locator("#midi-persist-tempo");
    await expect(persistTempo).toBeVisible();
    await expect(persistTempo).toBeDisabled();

    await page.locator("#midi-beat-clock").check();
    await expect(persistTempo).toBeEnabled();
  });

  test("saves persist_tempo into the profile, not the top-level midi block", async ({
    page,
  }) => {
    let profileBody: ProfileUpdate | null = null;
    await page.route("**/api/config/profiles/0", async (route) => {
      if (route.request().method() === "PUT") {
        profileBody = JSON.parse(route.request().postData() || "{}");
        await route.fulfill({
          status: 200,
          contentType: "application/json",
          body: JSON.stringify({
            yaml: "songs: songs\nprofiles:\n  - hostname: test-host\nsamples: {}\n",
            checksum: "updated-checksum",
          }),
        });
        return;
      }
      await route.continue();
    });

    const writes = await recordConfigWrites(page);
    await openMidiSection(page, ".profile-row");

    await page.locator("#midi-device").fill("Test MIDI Device");
    await page.locator("#midi-beat-clock").check();
    await page.locator("#midi-persist-tempo").check();
    await page.getByRole("button", { name: "Save" }).click();

    await expect.poll(() => profileBody, { timeout: 10000 }).not.toBeNull();

    // The setting has to arrive inside the profile. Landing anywhere else is
    // the #358 defect: accepted, echoed back, and gone after a restart.
    const midi = profileBody?.profile?.midi;
    if (!midi) {
      throw new Error(`no midi block in ${JSON.stringify(profileBody)}`);
    }
    expect(midi.persist_tempo).toBe(true);
    expect(midi.beat_clock).toBe(true);
    expect(midi.device).toBe("Test MIDI Device");

    // And the discarded endpoint must not be touched at all.
    expect(writes).not.toContain("PUT /api/config/midi");
    expect(writes).toContain("PUT /api/config/profiles/0");
  });

  test("unchecking persist_tempo removes it rather than sending false", async ({
    page,
  }) => {
    let profileBody: ProfileUpdate | null = null;
    await page.route("**/api/config/profiles/0", async (route) => {
      if (route.request().method() === "PUT") {
        profileBody = JSON.parse(route.request().postData() || "{}");
        await route.fulfill({
          status: 200,
          contentType: "application/json",
          body: JSON.stringify({
            yaml: "songs: songs\nprofiles:\n  - hostname: test-host\nsamples: {}\n",
            checksum: "updated-checksum",
          }),
        });
        return;
      }
      await route.continue();
    });

    await openMidiSection(page, ".profile-row");
    await page.locator("#midi-device").fill("Test MIDI Device");
    await page.locator("#midi-beat-clock").check();
    await page.locator("#midi-persist-tempo").check();
    await page.locator("#midi-persist-tempo").uncheck();
    await page.getByRole("button", { name: "Save" }).click();

    await expect.poll(() => profileBody, { timeout: 10000 }).not.toBeNull();

    // Off is the default, so the key is dropped instead of being written as an
    // explicit false. Config files stay as small as what the operator changed.
    expect(profileBody?.profile?.midi?.persist_tempo).toBeUndefined();
    expect(profileBody?.profile?.midi?.beat_clock).toBe(true);
  });

  test("on a profiles_dir layout the edit goes to the owning profile file", async ({
    page,
  }) => {
    // Serve a directory-based config. The editor must then write through the
    // file API -- writing the main config instead is what inlined every profile
    // into mtrack.yaml and collapsed the layout.
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

    let fileBody: ProfilePayload | null = null;
    await page.route("**/api/profiles/test-host.yaml", async (route) => {
      if (route.request().method() === "PUT") {
        fileBody = JSON.parse(route.request().postData() || "{}");
        await route.fulfill({
          status: 200,
          contentType: "application/json",
          body: JSON.stringify({ status: "saved" }),
        });
        return;
      }
      await route.continue();
    });

    const writes = await recordConfigWrites(page);
    await openMidiSection(page, ".profile-file-row");

    await page.locator("#midi-device").fill("Test MIDI Device");
    await page.locator("#midi-beat-clock").check();
    await page.locator("#midi-persist-tempo").check();
    await page.getByRole("button", { name: "Save" }).click();

    await expect.poll(() => fileBody, { timeout: 10000 }).not.toBeNull();
    // The file API takes the profile unwrapped, unlike the inline-profile
    // endpoint which nests it under `profile`.
    const midi = fileBody?.midi;
    if (!midi) {
      throw new Error(`no midi block in ${JSON.stringify(fileBody)}`);
    }
    expect(midi.persist_tempo).toBe(true);
    expect(midi.beat_clock).toBe(true);

    // Neither the main config nor the inline-profile endpoint may be written:
    // in this layout the directory is the source of truth.
    expect(writes).not.toContain("PUT /api/config/midi");
    expect(writes).not.toContain("PUT /api/config/profiles/0");
    expect(writes).toContain("PUT /api/profiles/test-host.yaml");
  });
});
