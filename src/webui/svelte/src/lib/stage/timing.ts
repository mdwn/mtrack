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
// Stage-state arrival timing, for judging how smooth a stage feed is. Pure:
// no DOM, no clock of its own (the caller gives each timestamp). Part of the
// gRPC stage feed spike (see grpc-feed.ts).

/** The most arrivals kept; older ones are overwritten. */
export const TIMING_CAPACITY = 20_000;

/** An inter-arrival longer than this is a gap, in ms. */
export const GAP_MS = 100;

export interface TimingSummary {
  /** Which feed these arrivals came from. */
  feed: string;
  /** Arrivals held (at most the capacity). */
  count: number;
  /** Arrivals recorded since the last reset, including overwritten ones. */
  total: number;
  /** First arrival to last, in ms. */
  duration_ms: number;
  /** Arrivals a second over the duration. */
  mean_rate_hz: number;
  /** Inter-arrival percentiles and maximum, in ms. */
  p50_ms: number;
  p95_ms: number;
  p99_ms: number;
  max_ms: number;
  /** Inter-arrivals longer than GAP_MS. */
  gaps_over_100ms: number;
}

/** Nearest-rank percentile of an ascending array; 0 for none. */
export function percentile(sorted: number[], p: number): number {
  if (sorted.length === 0) return 0;
  const rank = Math.ceil((p / 100) * sorted.length);
  return sorted[Math.min(sorted.length, Math.max(1, rank)) - 1];
}

/** A ring buffer of arrival timestamps (ms) and the statistics over them. */
export class TimingRecorder {
  private readonly times: Float64Array;
  private next = 0;
  private held = 0;
  private recorded = 0;
  feed: string;

  constructor(feed = "ws", capacity = TIMING_CAPACITY) {
    this.feed = feed;
    this.times = new Float64Array(capacity);
  }

  record(ms: number): void {
    this.times[this.next] = ms;
    this.next = (this.next + 1) % this.times.length;
    if (this.held < this.times.length) this.held++;
    this.recorded++;
  }

  reset(): void {
    this.next = 0;
    this.held = 0;
    this.recorded = 0;
  }

  /** The held timestamps, oldest first. */
  private ordered(): number[] {
    const out: number[] = [];
    const start = this.held < this.times.length ? 0 : this.next;
    for (let i = 0; i < this.held; i++) {
      out.push(this.times[(start + i) % this.times.length]);
    }
    return out;
  }

  summary(): TimingSummary {
    const at = this.ordered();
    const gaps: number[] = [];
    for (let i = 1; i < at.length; i++) gaps.push(at[i] - at[i - 1]);
    gaps.sort((a, b) => a - b);
    const duration = at.length > 1 ? at[at.length - 1] - at[0] : 0;
    return {
      feed: this.feed,
      count: at.length,
      total: this.recorded,
      duration_ms: duration,
      mean_rate_hz: duration > 0 ? ((at.length - 1) * 1000) / duration : 0,
      p50_ms: percentile(gaps, 50),
      p95_ms: percentile(gaps, 95),
      p99_ms: percentile(gaps, 99),
      max_ms: gaps.length ? gaps[gaps.length - 1] : 0,
      gaps_over_100ms: gaps.filter((g) => g > GAP_MS).length,
    };
  }
}
