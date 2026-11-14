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
import {
  ConfigService,
  PlayerService,
} from "../../src/gen/player/v1/player_pb";
import {
  getConfigReply,
  profileFileSavedReply,
  savedReply,
} from "../support/config-rpc";
import { mockRpc } from "../support/rpc";

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

/// The inline-profile call carries the profile as JSON beside a checksum;
/// the file endpoint sends the profile on its own.

/// A config whose profiles live in a directory rather than inline. The editor
/// branches on `profiles_dir` and saves through the file API in that case.
const PROFILES_DIR_YAML = `songs: songs
profiles_dir: profiles
samples: {}
`;

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
    const rpc = await mockRpc(page);
    rpc.unary(
      PlayerService.method.updateProfile,
      savedReply({
        yaml: "songs: songs\nprofiles:\n  - hostname: test-host\nsamples: {}\n",
        checksum: "updated-checksum",
      }),
    );

    await openMidiSection(page, ".profile-row");

    await page.locator("#midi-device").fill("Test MIDI Device");
    await page.locator("#midi-beat-clock").check();
    await page.locator("#midi-persist-tempo").check();
    await page.getByRole("button", { name: "Save" }).click();

    await expect
      .poll(() => rpc.calls(PlayerService.method.updateProfile).length, {
        timeout: 10000,
      })
      .toBe(1);
    const [update] = rpc.calls(PlayerService.method.updateProfile);
    const profileBody: ProfilePayload = JSON.parse(update.profileJson);

    // The setting has to arrive inside the profile. Landing anywhere else is
    // the #358 defect: accepted, echoed back, and gone after a restart.
    const midi = profileBody.midi;
    if (!midi) {
      throw new Error(`no midi block in ${JSON.stringify(profileBody)}`);
    }
    expect(midi.persist_tempo).toBe(true);
    expect(midi.beat_clock).toBe(true);
    expect(midi.device).toBe("Test MIDI Device");

    // And the discarded section must not be touched at all.
    expect(rpc.calls(PlayerService.method.updateMidi)).toHaveLength(0);
    expect(update.index).toBe(0);
  });

  test("unchecking persist_tempo removes it rather than sending false", async ({
    page,
  }) => {
    const rpc = await mockRpc(page);
    rpc.unary(
      PlayerService.method.updateProfile,
      savedReply({
        yaml: "songs: songs\nprofiles:\n  - hostname: test-host\nsamples: {}\n",
        checksum: "updated-checksum",
      }),
    );

    await openMidiSection(page, ".profile-row");
    await page.locator("#midi-device").fill("Test MIDI Device");
    await page.locator("#midi-beat-clock").check();
    await page.locator("#midi-persist-tempo").check();
    await page.locator("#midi-persist-tempo").uncheck();
    await page.getByRole("button", { name: "Save" }).click();

    await expect
      .poll(() => rpc.calls(PlayerService.method.updateProfile).length, {
        timeout: 10000,
      })
      .toBe(1);
    const profileBody: ProfilePayload = JSON.parse(
      rpc.calls(PlayerService.method.updateProfile)[0].profileJson,
    );

    // Off is the default, so the key is dropped instead of being written as an
    // explicit false. Config files stay as small as what the operator changed.
    expect(profileBody.midi?.persist_tempo).toBeUndefined();
    expect(profileBody.midi?.beat_clock).toBe(true);
  });

  test("on a profiles_dir layout the edit goes to the owning profile file", async ({
    page,
  }) => {
    // Serve a directory-based config. The editor must then write through the
    // file API -- writing the main config instead is what inlined every profile
    // into mtrack.yaml and collapsed the layout.
    const rpc = await mockRpc(page);
    rpc.unary(
      PlayerService.method.getConfig,
      getConfigReply({ yaml: PROFILES_DIR_YAML, checksum: "dir-checksum" }),
    );

    rpc.unary(
      ConfigService.method.saveProfileFile,
      profileFileSavedReply({ filename: "test-host.yaml" }),
    );

    await openMidiSection(page, ".profile-file-row");

    await page.locator("#midi-device").fill("Test MIDI Device");
    await page.locator("#midi-beat-clock").check();
    await page.locator("#midi-persist-tempo").check();
    await page.getByRole("button", { name: "Save" }).click();

    await expect
      .poll(() => rpc.calls(ConfigService.method.saveProfileFile).length, {
        timeout: 10000,
      })
      .toBe(1);
    const [save] = rpc.calls(ConfigService.method.saveProfileFile);
    expect(save.filename).toBe("test-host.yaml");
    // The file call takes the profile unwrapped, unlike the inline-profile
    // call which names the profile's index beside it.
    const fileBody: ProfilePayload = JSON.parse(save.profileJson);
    const midi = fileBody.midi;
    if (!midi) {
      throw new Error(`no midi block in ${save.profileJson}`);
    }
    expect(midi.persist_tempo).toBe(true);
    expect(midi.beat_clock).toBe(true);

    // Neither the main config nor an inline profile may be written: in this
    // layout the directory is the source of truth.
    expect(rpc.calls(PlayerService.method.updateMidi)).toHaveLength(0);
    expect(rpc.calls(PlayerService.method.updateProfile)).toHaveLength(0);
    expect(rpc.calls(PlayerService.method.addProfile)).toHaveLength(0);
  });
});
