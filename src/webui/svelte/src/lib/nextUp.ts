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

import type { FadeInfo, NextUp } from "./ws/stores";
import { formatFadeLength, formatGapMs, isZeroLength } from "./util/format";

type Translate = (
  key: string,
  opts?: { values?: Record<string, string> },
) => string;

/** The dashboard's "next up" line: what comes after the current song and
 *  how it will arrive. Empty when there is nothing to say. */
export function describeNext(
  next: NextUp | null | undefined,
  t: Translate,
): string {
  if (!next) return "";
  if (next.song === null) {
    const fade = loopEndFade(next);
    return fade
      ? t("playback.nextEndsLoop", {
          values: { length: formatFadeLength(fade.length) },
        })
      : t("playback.nextEnd");
  }
  const song = next.song;
  if (next.breaks_loop) {
    const out = next.fade_out;
    const into =
      out && !isZeroLength(out.length)
        ? t("playback.nextBreakFade", {
            values: { song, length: formatFadeLength(out.length) },
          })
        : t("playback.nextBreakCut", { values: { song } });
    const fadeIn = next.fade_in;
    if (!fadeIn || isZeroLength(fadeIn.length)) return into;
    return (
      into +
      t("playback.nextBreakFadeIn", {
        values: { length: formatFadeLength(fadeIn.length) },
      })
    );
  }
  if (next.auto_play_ms == null) {
    return t("playback.nextWaits", { values: { song } });
  }
  if (next.auto_play_ms === 0) {
    return t("playback.nextAutoNow", { values: { song } });
  }
  return t("playback.nextAuto", {
    values: { song, gap: formatGapMs(next.auto_play_ms) },
  });
}

/** The fade a looping song playing ends over when Next on the last entry,
 *  or Previous on the first, ends it, or null when such a press would cut
 *  it (no `fade_out`) and so is not offered. */
export function loopEndFade(next: NextUp | null | undefined): FadeInfo | null {
  if (!next || !(next.breaks_loop || next.ends_loop)) return null;
  const out = next.fade_out;
  return out && !isZeroLength(out.length) ? out : null;
}
