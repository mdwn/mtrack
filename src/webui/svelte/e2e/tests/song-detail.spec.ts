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

import { test, expect } from "@playwright/test";

test.describe("Song Detail", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/#/songs/Test%20Song%20Alpha");
  });

  test("shows back link", async ({ page }) => {
    await expect(page.locator(".back-link")).toBeVisible();
    await expect(page.locator(".back-link")).toContainText("All Songs");
  });

  test("shows song name", async ({ page }) => {
    await expect(page.locator(".song-title, h2")).toContainText(
      "Test Song Alpha",
    );
  });

  test("shows tab bar with all tabs", async ({ page }) => {
    await expect(page.locator(".tab-bar")).toBeVisible();
    await expect(page.locator(".tab", { hasText: "Tracks" })).toBeVisible();
    await expect(page.locator(".tab", { hasText: "MIDI" })).toBeVisible();
    await expect(page.locator(".tab", { hasText: "Samples" })).toBeVisible();
    await expect(page.locator(".tab", { hasText: "Lighting" })).toBeVisible();
    await expect(page.locator(".tab", { hasText: "Config" })).toBeVisible();
  });

  test("tracks tab is active by default", async ({ page }) => {
    await expect(page.locator(".tab.active")).toContainText("Tracks");
  });

  test("clicking MIDI tab changes active tab", async ({ page }) => {
    await page.locator(".tab", { hasText: "MIDI" }).click();
    await expect(page.locator(".tab.active")).toContainText("MIDI");
    await expect(page).toHaveURL(/.*#\/songs\/Test%20Song%20Alpha\/midi/);
  });

  test("clicking Lighting tab changes active tab", async ({ page }) => {
    await page.locator(".tab", { hasText: "Lighting" }).click();
    await expect(page.locator(".tab.active")).toContainText("Lighting");
    await expect(page).toHaveURL(/.*#\/songs\/Test%20Song%20Alpha\/lighting/);
  });

  test("clicking Config tab shows config editor", async ({ page }) => {
    await page.locator(".tab", { hasText: "Config" }).click();
    await expect(page.locator(".tab.active")).toContainText("Config");
    await expect(page.locator(".config-editor")).toBeVisible();
  });

  test("config tab shows YAML content", async ({ page }) => {
    await page.locator(".tab", { hasText: "Config" }).click();
    const editor = page.locator(".config-editor");
    await expect(editor).toBeVisible();
    const value = await editor.inputValue();
    expect(value).toContain("name:");
  });

  test("back link navigates to song list", async ({ page }) => {
    await page.locator(".back-link").click();
    await expect(page).toHaveURL(/.*#\/songs$/);
  });

  test("shows song metadata", async ({ page }) => {
    // Should show duration and track info somewhere in the detail header
    await expect(page.getByText("3:00")).toBeVisible();
  });

  test("shows MIDI badge for song with MIDI", async ({ page }) => {
    await expect(page.locator(".badge.midi")).toBeVisible();
  });

  test("shows lighting badge for song with lighting", async ({ page }) => {
    await expect(page.locator(".badge.lighting, .badge.light")).toBeVisible();
  });

  test("clicking Samples tab shows samples content", async ({ page }) => {
    await page.locator(".tab", { hasText: "Samples" }).click();
    await expect(page.locator(".tab.active")).toContainText("Samples");
    await expect(page).toHaveURL(/.*#\/songs\/Test%20Song%20Alpha\/samples/);
  });

  test("Samples tab shows empty state", async ({ page }) => {
    await page.locator(".tab", { hasText: "Samples" }).click();
    await expect(page.getByText(/per-song sample overrides/i)).toBeVisible();
  });

  test("Samples tab shows Add Sample button", async ({ page }) => {
    await page.locator(".tab", { hasText: "Samples" }).click();
    await expect(
      page.getByRole("button", { name: "Add Sample" }),
    ).toBeVisible();
  });
});

test.describe("Song Detail - Loop Playback", () => {
  test("does not show LOOP badge for non-looping song", async ({ page }) => {
    await page.goto("/#/songs/Test%20Song%20Alpha");
    await expect(page.locator(".badge.loop")).not.toBeVisible();
  });

  test("shows LOOP badge for looping song", async ({ page }) => {
    await page.goto("/#/songs/Test%20Song%20Beta");
    await expect(page.locator(".badge.loop")).toBeVisible();
  });

  test("loop checkbox is unchecked for non-looping song", async ({ page }) => {
    await page.goto("/#/songs/Test%20Song%20Alpha");
    const checkbox = page.locator("#loop-playback");
    await expect(checkbox).toBeVisible();
    await expect(checkbox).not.toBeChecked();
  });

  test("loop checkbox is checked for looping song", async ({ page }) => {
    await page.goto("/#/songs/Test%20Song%20Beta");
    const checkbox = page.locator("#loop-playback");
    await expect(checkbox).toBeVisible();
    await expect(checkbox).toBeChecked();
  });

  test("toggling loop checkbox marks config as dirty", async ({ page }) => {
    await page.goto("/#/songs/Test%20Song%20Alpha");
    await page.locator("#loop-playback").check();
    await expect(page.locator(".unsaved")).toBeVisible();
  });
});

test.describe("Song Detail - MIDI Event Editor", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/#/songs/Test%20Song%20Alpha");
    await page.locator(".tab", { hasText: "MIDI" }).click();
  });

  test("shows Song Select Event section", async ({ page }) => {
    await expect(page.getByText(/Song Select Event/i)).toBeVisible();
  });

  test("shows Add Event button when no event configured", async ({ page }) => {
    await expect(page.getByRole("button", { name: "Add Event" })).toBeVisible();
  });

  test("adding event shows type dropdown and fields", async ({ page }) => {
    await page.getByRole("button", { name: "Add Event" }).click();
    await expect(page.locator("#song-midi-event-type")).toBeVisible();
    await expect(page.locator("#song-midi-event-channel")).toBeVisible();
    // Default type is program_change, so program field should be visible
    await expect(page.locator("#song-midi-event-program")).toBeVisible();
  });

  test("changing type to note_on shows key and velocity fields", async ({
    page,
  }) => {
    await page.getByRole("button", { name: "Add Event" }).click();
    await page.locator("#song-midi-event-type").selectOption("note_on");
    await expect(page.locator("#song-midi-event-key")).toBeVisible();
    await expect(page.locator("#song-midi-event-velocity")).toBeVisible();
  });

  test("changing type to control_change shows controller and value fields", async ({
    page,
  }) => {
    await page.getByRole("button", { name: "Add Event" }).click();
    await page.locator("#song-midi-event-type").selectOption("control_change");
    await expect(page.locator("#song-midi-event-controller")).toBeVisible();
    await expect(page.locator("#song-midi-event-value")).toBeVisible();
  });

  test("Remove button clears the event", async ({ page }) => {
    await page.getByRole("button", { name: "Add Event" }).click();
    await expect(page.locator("#song-midi-event-type")).toBeVisible();
    await page.getByRole("button", { name: "Remove", exact: true }).click();
    await expect(page.locator("#song-midi-event-type")).not.toBeVisible();
    await expect(page.getByRole("button", { name: "Add Event" })).toBeVisible();
  });

  test("adding event marks config as dirty", async ({ page }) => {
    await page.getByRole("button", { name: "Add Event" }).click();
    await expect(page.locator(".unsaved")).toBeVisible();
  });
});

test.describe("Song Detail - Exclude MIDI Channels", () => {
  test.beforeEach(async ({ page }) => {
    // Test Song Alpha has midi_playback with exclude_midi_channels: [10]
    await page.goto("/#/songs/Test%20Song%20Alpha");
    await page.locator(".tab", { hasText: "MIDI" }).click();
  });

  test("shows channel grid when MIDI file is present", async ({ page }) => {
    await expect(page.locator(".channel-grid")).toBeVisible();
    await expect(page.getByText(/Exclude MIDI Channels/i)).toBeVisible();
  });

  test("shows 16 channel toggles", async ({ page }) => {
    await expect(page.locator(".channel-toggle")).toHaveCount(16);
  });

  test("channel 10 is pre-selected as excluded", async ({ page }) => {
    const ch10 = page.locator(".channel-toggle").nth(9);
    await expect(ch10).toHaveClass(/excluded/);
  });

  test("toggling a channel marks config as dirty", async ({ page }) => {
    // Click channel 1 to exclude it
    await page.locator(".channel-toggle").first().click();
    await expect(page.locator(".unsaved")).toBeVisible();
  });
});

test.describe("Song Detail - No MIDI File", () => {
  test("does not show channel grid for song without MIDI file", async ({
    page,
  }) => {
    // Test Song Beta has no MIDI — navigate directly without visiting Alpha first
    await page.goto("/#/songs/Test%20Song%20Beta");
    await page.locator(".tab", { hasText: "MIDI" }).click();
    await expect(page.locator(".channel-grid")).not.toBeVisible();
  });
});

test.describe("Song Detail - Notifications Tab", () => {
  test("shows Notifications tab", async ({ page }) => {
    await page.goto("/#/songs/Test%20Song%20Alpha");
    await expect(
      page.locator(".tab", { hasText: "Notifications" }),
    ).toBeVisible();
  });

  test("clicking Notifications tab shows notification fields", async ({
    page,
  }) => {
    await page.goto("/#/songs/Test%20Song%20Alpha");
    await page.locator(".tab", { hasText: "Notifications" }).click();
    await expect(page.locator(".tab.active")).toContainText("Notifications");
    await expect(page.locator("#notif-loop_armed")).toBeVisible();
  });

  test("navigates to notifications tab via URL", async ({ page }) => {
    await page.goto("/#/songs/Test%20Song%20Alpha/notifications");
    await expect(page.locator(".tab.active")).toContainText("Notifications");
    await expect(page.locator("#notif-loop_armed")).toBeVisible();
  });

  test("editing notification field marks config as dirty", async ({ page }) => {
    await page.goto("/#/songs/Test%20Song%20Alpha/notifications");
    await page.locator("#notif-loop_armed").fill("armed.wav");
    await page.locator("#notif-loop_armed").dispatchEvent("change");
    await expect(page.locator(".unsaved")).toBeVisible();
  });

  test("shows browse buttons on notification fields", async ({ page }) => {
    await page.goto("/#/songs/Test%20Song%20Alpha/notifications");
    await expect(page.locator(".browse-btn").first()).toBeVisible();
  });

  test("shows file upload drop zone", async ({ page }) => {
    await page.goto("/#/songs/Test%20Song%20Alpha/notifications");
    await expect(page.locator(".drop-zone")).toBeVisible();
  });

  test("section override shows datalist with section names", async ({
    page,
  }) => {
    // Test Song Beta has sections defined
    await page.goto("/#/songs/Test%20Song%20Beta/notifications");
    // Add a per-section override
    await page
      .locator(".sections-area")
      .getByRole("button", { name: "Add" })
      .click();
    await expect(page.locator(".section-row")).toHaveCount(1);
    // The datalist should be rendered with section names from the song
    const datalist = page.locator("#notif-section-names");
    await expect(datalist).toBeAttached();
    const options = datalist.locator("option");
    await expect(options).toHaveCount(2);
  });
});

test.describe("Song Detail - Section Editor", () => {
  test("shows Sections tab", async ({ page }) => {
    await page.goto("/#/songs/Test%20Song%20Alpha");
    await expect(page.locator(".tab", { hasText: "Sections" })).toBeVisible();
  });

  test("clicking Sections tab shows visual timeline editor", async ({
    page,
  }) => {
    await page.goto("/#/songs/Test%20Song%20Alpha");
    await page.locator(".tab", { hasText: "Sections" }).click();
    await expect(page.locator(".tab.active")).toContainText("Sections");
    await expect(page.locator(".section-timeline-editor")).toBeVisible();
  });

  test("sections tab shows zoom controls", async ({ page }) => {
    await page.goto("/#/songs/Test%20Song%20Alpha");
    await page.locator(".tab", { hasText: "Sections" }).click();
    await expect(page.getByRole("button", { name: "Fit" })).toBeVisible();
  });

  test("sections tab shows section bar", async ({ page }) => {
    await page.goto("/#/songs/Test%20Song%20Alpha");
    await page.locator(".tab", { hasText: "Sections" }).click();
    await expect(page.locator(".section-bar")).toBeVisible();
  });

  test("song with sections shows section chips", async ({ page }) => {
    await page.goto("/#/songs/Test%20Song%20Beta");
    await page.locator(".tab", { hasText: "Sections" }).click();
    await expect(page.locator(".section-chip")).toHaveCount(2);
  });

  test("song with sections shows section blocks in bar", async ({ page }) => {
    await page.goto("/#/songs/Test%20Song%20Beta");
    await page.locator(".tab", { hasText: "Sections" }).click();
    await expect(page.locator(".section-block")).toHaveCount(2);
  });

  test("drag on empty area creates a new section", async ({ page }) => {
    await page.goto("/#/songs/Test%20Song%20Beta");
    await page.locator(".tab", { hasText: "Sections" }).click();
    await expect(page.locator(".section-block")).toHaveCount(2);

    // Drag on the bar-content area past the existing sections.
    const barContent = page.locator(".bar-content");
    const box = await barContent.boundingBox();
    if (!box) throw new Error("bar-content not found");

    // Beat grid: 16 measures spanning 0-32s of a 240s song.
    // Existing sections cover m1-8 (0-16s). Drag in m9-16 range (16-32s = ~7%-13% of bar).
    const startX = box.x + box.width * 0.08;
    const endX = box.x + box.width * 0.12;
    const y = box.y + box.height / 2;

    await page.mouse.move(startX, y);
    await page.mouse.down();
    await page.mouse.move(endX, y, { steps: 5 });
    await page.mouse.up();

    await expect(page.locator(".section-block")).toHaveCount(3);
    await expect(page.locator(".section-chip")).toHaveCount(3);
  });

  test("selecting a section and pressing Delete removes it", async ({
    page,
  }) => {
    await page.goto("/#/songs/Test%20Song%20Beta");
    await page.locator(".tab", { hasText: "Sections" }).click();
    await expect(page.locator(".section-block")).toHaveCount(2);

    // Click on the first section block to select it.
    const firstBlock = page.locator(".section-block").first();
    const blockBox = await firstBlock.boundingBox();
    if (!blockBox) throw new Error("section block not found");

    // Click the bar-content at the block's center (blocks have pointer-events: none).
    const barContent = page.locator(".bar-content");
    const barBox = await barContent.boundingBox();
    if (!barBox) throw new Error("bar-content not found");
    await page.mouse.click(
      blockBox.x + blockBox.width / 2,
      blockBox.y + blockBox.height / 2,
    );

    await expect(page.locator(".section-block.selected")).toHaveCount(1);
    await page.keyboard.press("Delete");

    await expect(page.locator(".section-block")).toHaveCount(1);
    await expect(page.locator(".section-chip")).toHaveCount(1);
  });

  test("double-clicking a section allows renaming", async ({ page }) => {
    await page.goto("/#/songs/Test%20Song%20Beta");
    await page.locator(".tab", { hasText: "Sections" }).click();
    await expect(page.locator(".section-block")).toHaveCount(2);

    // Double-click on the first section block to edit its name.
    const firstBlock = page.locator(".section-block").first();
    const blockBox = await firstBlock.boundingBox();
    if (!blockBox) throw new Error("section block not found");

    await page.mouse.dblclick(
      blockBox.x + blockBox.width / 2,
      blockBox.y + blockBox.height / 2,
    );

    // An inline input should appear.
    const input = page.locator(".section-name-input");
    await expect(input).toBeVisible();

    // Clear and type a new name.
    await input.fill("intro");
    await input.press("Enter");

    // The input should disappear and the name should be updated.
    await expect(input).not.toBeVisible();
    await expect(page.locator(".section-chip").first()).toContainText("intro");
  });

  test("section editing marks config as dirty", async ({ page }) => {
    await page.goto("/#/songs/Test%20Song%20Beta");
    await page.locator(".tab", { hasText: "Sections" }).click();

    // Delete a section to trigger a change.
    const firstBlock = page.locator(".section-block").first();
    const blockBox = await firstBlock.boundingBox();
    if (!blockBox) throw new Error("section block not found");
    await page.mouse.click(
      blockBox.x + blockBox.width / 2,
      blockBox.y + blockBox.height / 2,
    );
    await page.keyboard.press("Delete");

    await expect(page.locator(".unsaved")).toBeVisible();
  });
});
