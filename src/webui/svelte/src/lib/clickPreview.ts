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

import { rules } from "./rules";

let ctx: AudioContext | null = null;

/** Plays the player's synthesized click (`rules.click_samples`) locally
 *  (browser output, not the player). */
export function previewClick(freq: number, volume: number) {
  ctx ??= new AudioContext();
  // Some browsers suspend fresh contexts until a user gesture completes.
  if (ctx.state === "suspended") void ctx.resume();

  const samples = rules.click_samples(freq, volume, ctx.sampleRate);
  if (!samples) return;
  const buffer = ctx.createBuffer(1, samples.length, ctx.sampleRate);
  buffer.getChannelData(0).set(samples);
  const source = ctx.createBufferSource();
  source.buffer = buffer;
  source.connect(ctx.destination);
  source.start();
}
