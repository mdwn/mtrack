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
import { describe, expect, it } from "vitest";
import { percentile, TimingRecorder } from "./timing";

describe("percentile", () => {
  it("is nearest-rank and 0 for none", () => {
    expect(percentile([], 50)).toBe(0);
    const v = Array.from({ length: 100 }, (_, i) => i + 1);
    expect(percentile(v, 50)).toBe(50);
    expect(percentile(v, 95)).toBe(95);
    expect(percentile(v, 99)).toBe(99);
    expect(percentile(v, 100)).toBe(100);
  });
});

describe("TimingRecorder", () => {
  it("summarises an empty and a one-sample recorder as zeros", () => {
    const r = new TimingRecorder("grpc");
    expect(r.summary()).toMatchObject({ feed: "grpc", count: 0, max_ms: 0 });
    r.record(5);
    expect(r.summary()).toMatchObject({
      count: 1,
      duration_ms: 0,
      mean_rate_hz: 0,
      max_ms: 0,
    });
  });

  it("computes rate, percentiles, max and gaps", () => {
    const r = new TimingRecorder();
    let t = 1000;
    r.record(t);
    for (let i = 0; i < 98; i++) r.record((t += 50)); // 98 steps of 50 ms
    r.record(t + 150); // one gap
    const s = r.summary();
    expect(s.count).toBe(100);
    expect(s.duration_ms).toBe(98 * 50 + 150);
    expect(s.mean_rate_hz).toBeCloseTo((99 * 1000) / 5050, 6);
    expect(s.p50_ms).toBe(50);
    expect(s.p95_ms).toBe(50);
    expect(s.p99_ms).toBe(150);
    expect(s.max_ms).toBe(150);
    expect(s.gaps_over_100ms).toBe(1);
  });

  it("keeps the newest samples when the ring wraps, in order", () => {
    const r = new TimingRecorder("grpc", 4);
    for (let i = 0; i < 10; i++) r.record(i * 10);
    const s = r.summary();
    expect(s.count).toBe(4);
    expect(s.total).toBe(10);
    expect(s.duration_ms).toBe(30);
    expect(s.max_ms).toBe(10);
  });

  it("resets", () => {
    const r = new TimingRecorder();
    r.record(1);
    r.record(2);
    r.reset();
    expect(r.summary().count).toBe(0);
    expect(r.summary().total).toBe(0);
  });
});
