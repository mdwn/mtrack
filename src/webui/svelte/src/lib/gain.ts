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

import { playerClient } from "./grpc/client";

export const GAIN_MIN = -60;
export const GAIN_MAX = 12;
export const GAIN_STEP = 0.5;

/** Throttle interval for slider drags. */
const THROTTLE_MS = 75;

const timers = new Map<string, ReturnType<typeof setTimeout>>();
const pending = new Map<string, number>();

/** Formats a dB value for display; GAIN_MIN and below shows as -inf. */
export function formatDb(db: number): string {
  if (db <= GAIN_MIN) return "-∞";
  return `${db > 0 ? "+" : ""}${db.toFixed(1)} dB`;
}

/** Immediately sends a track gain, cancelling any pending throttled send. */
export async function sendTrackGain(
  track: string,
  gainDb: number,
): Promise<void> {
  const timer = timers.get(track);
  if (timer !== undefined) {
    clearTimeout(timer);
    timers.delete(track);
  }
  pending.delete(track);
  try {
    await playerClient.setTrackGain({ track, gainDb });
  } catch (e) {
    console.error(`Failed to set gain for track "${track}":`, e);
  }
}

/** Mutes or unmutes a track without touching its gain value. */
export async function sendTrackMute(
  track: string,
  muted: boolean,
): Promise<void> {
  try {
    await playerClient.setTrackMute({ track, muted });
  } catch (e) {
    console.error(`Failed to set mute for track "${track}":`, e);
  }
}

/**
 * Trailing-throttled gain send for use while dragging, keyed per track so
 * concurrent sliders never starve each other. The final value should be
 * flushed with `sendTrackGain` on commit (change/pointerup).
 */
export function sendTrackGainThrottled(track: string, gainDb: number): void {
  pending.set(track, gainDb);
  if (timers.has(track)) return;
  timers.set(
    track,
    setTimeout(() => {
      timers.delete(track);
      const value = pending.get(track);
      pending.delete(track);
      if (value !== undefined) {
        void sendTrackGain(track, value);
      }
    }, THROTTLE_MS),
  );
}
