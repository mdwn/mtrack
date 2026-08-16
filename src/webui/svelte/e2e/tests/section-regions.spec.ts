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
// Where the transport draws a section, in percentages of the song.
//
// Two things the drawing has to agree with the player about, neither of which
// any other test pins down:
//
//   - `end_measure` is *exclusive* (`config::Section`). Treating it as
//     inclusive draws every region a measure too long.
//   - `start_beat`/`end_beat` offset the boundary within its measure. Ignoring
//     them puts the region up to a measure away from where the active-section
//     highlight actually flips, because the player resolves the boundary
//     through `grid.beat_time(measure, beat)`.
//
// The numbers below are chosen so the two mistakes give visibly different
// answers from the correct one, rather than merely approximate ones.

import { test, expect } from "@playwright/test";

let testCounter = 0;

async function sendWsMessage(
  page: import("@playwright/test").Page,
  wsId: string,
  msg: object,
) {
  await page.request.post("http://127.0.0.1:3111/test/send-ws", {
    data: { ...msg, _wsId: wsId },
  });
}

/**
 * 120bpm 4/4 over five measures: a beat every 0.5s, a bar every 2s.
 *
 * Against a 10s song that puts bar N at (N-1) * 20% of the width, so every
 * expected percentage below is a round number.
 */
function fourFourGrid() {
  const beats: number[] = [];
  const measure_starts: number[] = [];
  for (let m = 0; m < 5; m++) {
    measure_starts.push(beats.length);
    for (let b = 0; b < 4; b++) {
      beats.push((m * 4 + b) * 0.5);
    }
  }
  return { beats, measure_starts };
}

function playbackState(overrides: Record<string, unknown> = {}) {
  return {
    type: "playback",
    is_playing: false,
    elapsed_ms: 0,
    song_name: "Test Song Beta",
    song_duration_ms: 10000,
    playlist_name: "setlist",
    playlist_position: 1,
    playlist_songs: ["Test Song Alpha", "Test Song Beta"],
    tracks: [],
    available_playlists: ["all_songs", "setlist"],
    persisted_playlist_name: "setlist",
    locked: false,
    beat_grid: fourFourGrid(),
    available_sections: [],
    active_section: null,
    ...overrides,
  };
}

/** The inline percentage the component wrote, as a number. */
async function pct(
  region: import("@playwright/test").Locator,
  property: "left" | "width",
): Promise<number> {
  const raw = await region.evaluate(
    (el, prop) => (el as HTMLElement).style.getPropertyValue(prop),
    property,
  );
  return Number.parseFloat(raw.replace("%", ""));
}

test.describe("Section regions on the transport", () => {
  let wsId: string;

  test.beforeEach(async ({ page }) => {
    wsId = `section-regions-${test.info().parallelIndex}-${++testCounter}-${Date.now()}`;
    await page.goto(`/?wsId=${wsId}#/`);
    await expect(page.locator(".playback-card__title")).toBeVisible();
  });

  test("a measure-aligned section ends where the exclusive bound says", async ({
    page,
  }) => {
    await sendWsMessage(
      page,
      wsId,
      playbackState({
        available_sections: [
          // Bars 1 and 2 only: the bound is exclusive, so this ends at bar 3
          // (4.0s of 10s), not bar 4.
          { name: "intro", start_measure: 1, end_measure: 3 },
        ],
      }),
    );

    const region = page.locator(".scrub__region").first();
    await expect(region).toBeVisible();

    expect(await pct(region, "left")).toBeCloseTo(0, 1);
    // 40%, not the 60% an inclusive reading of `end_measure` produces.
    expect(await pct(region, "width")).toBeCloseTo(40, 1);
  });

  test("beat offsets move the boundary within its measure", async ({
    page,
  }) => {
    await sendWsMessage(
      page,
      wsId,
      playbackState({
        available_sections: [
          // Bar 2 beat 3 (3.0s) to bar 4 beat 1 (6.0s) of a 10s song.
          {
            name: "verse",
            start_measure: 2,
            start_beat: 3,
            end_measure: 4,
            end_beat: 1,
          },
        ],
      }),
    );

    const region = page.locator(".scrub__region").first();
    await expect(region).toBeVisible();

    // 30%, not the 20% that ignoring `start_beat` gives.
    expect(await pct(region, "left")).toBeCloseTo(30, 1);
    // 30% wide, not the 60% that ignoring the beat *and* reading the bound as
    // inclusive gives.
    expect(await pct(region, "width")).toBeCloseTo(30, 1);
  });

  test("a fractional beat interpolates between grid beats", async ({
    page,
  }) => {
    await sendWsMessage(
      page,
      wsId,
      playbackState({
        available_sections: [
          // Bar 1 beat 2.5 is halfway between beat 2 (0.5s) and beat 3 (1.0s),
          // so 0.75s -> 7.5%. This is the interpolation `BeatGrid::beat_time`
          // performs, and the editor has to agree with it.
          {
            name: "pickup",
            start_measure: 1,
            start_beat: 2.5,
            end_measure: 2,
          },
        ],
      }),
    );

    const region = page.locator(".scrub__region").first();
    await expect(region).toBeVisible();
    expect(await pct(region, "left")).toBeCloseTo(7.5, 1);
    // Ends at bar 2 = 2.0s = 20%.
    expect(await pct(region, "width")).toBeCloseTo(12.5, 1);
  });
});
