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

// Plain helpers for the playlist editor's ambient light pickers and an
// entry's transitions summary. No Svelte, no i18n store: the caller passes
// its translator, so these can be checked as plain values.

import type { Translate } from "./i18n";
import type {
  Ambient,
  EntryFades,
  FadeConfig,
} from "../gen/rules/mtrack_rules";
import type { AutoPlay, PlaylistAmbient } from "./api/config";
import { formatFadeLength } from "./util/format";
import { rules } from "./rules";

/** The ambient picker's choice: inherited (null), none (false), or its own. */
export type AmbientChoice = "inherit" | "off" | "custom";

export function ambientModeOf(ambient: PlaylistAmbient): AmbientChoice {
  return ambient === null ? "inherit" : ambient === false ? "off" : "custom";
}

export function ambientForMode(mode: AmbientChoice): PlaylistAmbient {
  return mode === "inherit" ? null : mode === "off" ? false : {};
}

/** The settings with one field set, or removed when empty (to inherit). */
export function withAmbientField(
  ambient: PlaylistAmbient,
  key: keyof Ambient,
  value: string,
): Ambient {
  const settings: Ambient = { ...(ambient || {}) };
  if (value) settings[key] = value;
  else delete settings[key];
  return settings;
}

/** The timing fields after the show, with their labels' i18n keys. */
export const ambientTimings: { key: keyof Ambient; label: string }[] = [
  { key: "delay", label: "lighting.ambientDelay" },
  { key: "fade_in", label: "lighting.ambientFadeIn" },
  { key: "fade_out", label: "lighting.ambientFadeOut" },
];

function fadeLength(fade: FadeConfig | null | undefined): string | null {
  if (fade === null || fade === undefined) return null;
  return formatFadeLength(typeof fade === "string" ? fade : fade.length);
}

/** Everything set on an entry, in short, for its closed transitions
 *  button: its fades, its auto play gap (when not zero) and its ambient
 *  light. Empty when the entry sets nothing. */
export function entrySummary(
  entry: {
    fades?: EntryFades;
    autoPlay?: AutoPlay;
    ambient?: PlaylistAmbient;
  },
  tr: Translate,
): string {
  const out = fadeLength(entry.fades?.fade_out);
  const fadeIn = fadeLength(entry.fades?.fade_in);
  // The gap as the player reads it (`rules.auto_play_read`); a gap of none
  // is no gap, and text the player refuses says so.
  const gapText =
    typeof entry.autoPlay === "string" ? entry.autoPlay.trim() : "";
  const read = gapText ? rules.auto_play_read(gapText) : null;
  const gap =
    read?.kind === "invalid"
      ? tr("playlists.autoPlayGapInvalid", { values: { gap: gapText } })
      : read?.kind === "gap" && read.ms > 0
        ? tr("playlists.autoPlayGapSummary", {
            values: { gap: read.text },
          })
        : null;
  const ambient = entry.ambient ?? null;
  return [
    out && tr("playlists.fadeOutSummary", { values: { length: out } }),
    fadeIn && tr("playlists.fadeInSummary", { values: { length: fadeIn } }),
    gap,
    ambient === false && tr("playlists.entryAmbientOffSummary"),
    ambient && tr("playlists.entryAmbientOwnSummary"),
  ]
    .filter(Boolean)
    .join(" · ");
}
