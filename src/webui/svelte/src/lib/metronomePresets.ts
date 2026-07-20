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

import type { ClickSoundConfig } from "./api/songs";

export interface MetronomeSoundSet {
  accent?: ClickSoundConfig;
  half?: ClickSoundConfig;
  normal?: ClickSoundConfig;
  sub?: ClickSoundConfig;
}

/** Built-in synthesized defaults, used to seed a fresh sound override. */
export const SOUND_DEFAULTS: Record<
  "accent" | "half" | "normal" | "sub",
  { freq: number; volume: number }
> = {
  accent: { freq: 1600, volume: 1.0 },
  half: { freq: 1400, volume: 0.9 },
  normal: { freq: 1200, volume: 0.8 },
  sub: { freq: 1000, volume: 0.45 },
};

/** Typical click sound schemes; applying one overwrites all four sounds.
 * Shared by the song metronome panel and the config-page defaults. */
export const METRONOME_PRESETS: {
  key: string;
  sounds: MetronomeSoundSet;
}[] = [
  {
    key: "classic",
    sounds: {
      accent: { freq: 1125, volume: 1.0 },
      half: { freq: 1125, volume: 1.0 },
      normal: { freq: 1125, volume: 1.0 },
      sub: { freq: 1125, volume: 0.5 },
    },
  },
  {
    key: "hilo",
    sounds: {
      accent: { freq: 1600, volume: 1.0 },
      half: { freq: 1400, volume: 0.9 },
      normal: { freq: 1200, volume: 0.8 },
      sub: { freq: 1000, volume: 0.45 },
    },
  },
  {
    key: "sharp",
    sounds: {
      accent: { freq: 2000, volume: 1.0 },
      half: { freq: 1750, volume: 0.85 },
      normal: { freq: 1500, volume: 0.75 },
      sub: { freq: 1200, volume: 0.4 },
    },
  },
  {
    key: "low",
    sounds: {
      accent: { freq: 880, volume: 1.0 },
      half: { freq: 770, volume: 0.9 },
      normal: { freq: 660, volume: 0.8 },
      sub: { freq: 550, volume: 0.45 },
    },
  },
];
