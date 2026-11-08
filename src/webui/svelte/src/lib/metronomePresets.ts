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

import type { MetronomeSounds } from "../gen/rules/mtrack_rules";
import { rules } from "./rules";

/** The click sound roles, in the order the editors list them. */
export const SOUND_ROLES = ["accent", "half", "normal", "sub"] as const;
export type SoundRole = (typeof SOUND_ROLES)[number];

/** The player's built-in synthesized click of each role
 *  (`rules.metronome_sounds`), which seeds a fresh sound override. */
export function builtinSounds(): Record<
  SoundRole,
  { freq: number; volume: number }
> {
  const sounds = rules.metronome_sounds();
  return Object.fromEntries(
    SOUND_ROLES.map((role) => [
      role,
      { freq: sounds[role]?.freq ?? 0, volume: sounds[role]?.volume ?? 0 },
    ]),
  ) as Record<SoundRole, { freq: number; volume: number }>;
}

/** Typical click sound schemes; applying one overwrites all four sounds.
 * Shared by the song metronome panel and the config-page defaults. The
 * player's built-in clicks are the "hilo" scheme. */
export function metronomePresets(): { key: string; sounds: MetronomeSounds }[] {
  return [
    PRESETS[0],
    { key: "hilo", sounds: rules.metronome_sounds() },
    ...PRESETS.slice(1),
  ];
}

const PRESETS: { key: string; sounds: MetronomeSounds }[] = [
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
