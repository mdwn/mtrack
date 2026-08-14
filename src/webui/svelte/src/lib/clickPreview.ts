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

// Mirrors the backend's synthesized click (audio/metronome.rs): a sine
// burst with a 1ms attack and exponential decay, 50ms long.
const CLICK_SECS = 0.05;
const ATTACK_SECS = 0.001;
const DECAY_TAU_SECS = 0.012;

let ctx: AudioContext | null = null;

/** Plays a synthesized click locally (browser output, not the player). */
export function previewClick(freq: number, volume: number) {
  ctx ??= new AudioContext();
  // Some browsers suspend fresh contexts until a user gesture completes.
  if (ctx.state === "suspended") void ctx.resume();

  const rate = ctx.sampleRate;
  const total = Math.max(1, Math.round(rate * CLICK_SECS));
  const attack = Math.max(1, Math.round(rate * ATTACK_SECS));
  const buffer = ctx.createBuffer(1, total, rate);
  const data = buffer.getChannelData(0);
  for (let i = 0; i < total; i++) {
    const t = i / rate;
    const envelope =
      i < attack
        ? i / attack
        : Math.exp(-((i - attack) / rate) / DECAY_TAU_SECS);
    data[i] = Math.sin(2 * Math.PI * freq * t) * envelope * volume;
  }
  const source = ctx.createBufferSource();
  source.buffer = buffer;
  source.connect(ctx.destination);
  source.start();
}
