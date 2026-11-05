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

import type {
  Ambient,
  EntryFades,
  FadeConfig,
} from "../gen/rules/mtrack_rules";
import type { AutoPlay, PlaylistAmbient } from "./api/config";
import { formatFadeLength, isZeroLength } from "./util/format";

/** The ambient picker's choice: inherited (null), none (false), or its own. */
export type AmbientMode = "inherit" | "off" | "custom";

export function ambientModeOf(ambient: PlaylistAmbient): AmbientMode {
  return ambient === null ? "inherit" : ambient === false ? "off" : "custom";
}

export function ambientForMode(mode: AmbientMode): PlaylistAmbient {
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

export type Translate = (
  key: string,
  values?: Record<string, string>,
) => string;

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
  const gap =
    typeof entry.autoPlay === "string" &&
    entry.autoPlay.trim() &&
    !isZeroLength(entry.autoPlay)
      ? formatFadeLength(entry.autoPlay)
      : null;
  const ambient = entry.ambient ?? null;
  return [
    out && tr("playlists.fadeOutSummary", { length: out }),
    fadeIn && tr("playlists.fadeInSummary", { length: fadeIn }),
    gap && tr("playlists.autoPlayGapSummary", { gap }),
    ambient === false && tr("playlists.entryAmbientOffSummary"),
    ambient && tr("playlists.entryAmbientOwnSummary"),
  ]
    .filter(Boolean)
    .join(" · ");
}
