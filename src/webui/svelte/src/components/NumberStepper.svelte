<!-- *     * Copyright (C) 2026 Michael Wilson <mike@mdwn.dev>
     *
     * This program is free software: you can redistribute it and/or modify it under
     * the terms of the GNU General Public License as published by the Free Software
     * Foundation, version 3.
     *
     * This program is distributed in the hope that it will be useful, but WITHOUT
     * ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS
     * FOR A PARTICULAR PURPOSE. See the GNU General Public License for more details.
     *
     * You should have received a copy of the GNU General Public License along with
     * this program. If not, see <https://www.gnu.org/licenses/>.
     *
     * -->
<script lang="ts">
  interface Props {
    value: number;
    onchange: (value: number) => void;
    min?: number;
    max?: number;
    /** Increment per tap. Holding a button auto-repeats and, after a while,
     * steps by 10x for fast travel. */
    step?: number;
    /** When set, +/- move through this fixed list instead of adding step
     * (e.g. time-signature denominators: [1, 2, 4, 8, 16, 32]). */
    values?: number[];
    /** Digits shown after the decimal point (default: as few as possible). */
    decimals?: number;
    /** Rendered after the value inside the field, e.g. a unit. */
    suffix?: string;
    ariaLabel?: string;
  }

  let {
    value,
    onchange,
    min = -Infinity,
    max = Infinity,
    step = 1,
    values,
    decimals,
    suffix = "",
    ariaLabel,
  }: Props = $props();

  const REPEAT_DELAY_MS = 450;
  const REPEAT_INTERVAL_MS = 70;
  const FAST_AFTER_REPEATS = 14;

  let repeatTimer: ReturnType<typeof setTimeout> | null = null;
  let repeatCount = 0;

  function clamp(v: number): number {
    return Math.min(max, Math.max(min, v));
  }

  function roundToStep(v: number): number {
    // Avoid float noise like 120.30000000000001 on fractional steps.
    const places = Math.max(
      decimals ?? 0,
      (step.toString().split(".")[1] ?? "").length,
    );
    return parseFloat(v.toFixed(places));
  }

  function bump(direction: 1 | -1, fast: boolean) {
    if (values && values.length > 0) {
      // Move through the fixed list.
      let idx = values.findIndex((v) => v >= value);
      if (idx === -1) idx = values.length - 1;
      else if (values[idx] !== value && direction === -1) idx -= 1;
      const next =
        values[Math.min(values.length - 1, Math.max(0, idx + direction))];
      if (next !== undefined && next !== value) onchange(next);
      return;
    }
    const amount = step * (fast ? 10 : 1) * direction;
    const next = clamp(roundToStep(value + amount));
    if (next !== value) onchange(next);
  }

  function startRepeat(direction: 1 | -1) {
    stopRepeat();
    bump(direction, false);
    repeatCount = 0;
    const tick = () => {
      repeatCount += 1;
      bump(direction, repeatCount > FAST_AFTER_REPEATS);
      repeatTimer = setTimeout(tick, REPEAT_INTERVAL_MS);
    };
    repeatTimer = setTimeout(tick, REPEAT_DELAY_MS);
  }

  function stopRepeat() {
    if (repeatTimer !== null) {
      clearTimeout(repeatTimer);
      repeatTimer = null;
    }
  }

  function onInputChange(e: Event) {
    const raw = (e.target as HTMLInputElement).value.replace(",", ".");
    const parsed = parseFloat(raw);
    if (!isNaN(parsed)) {
      onchange(clamp(parsed));
    } else {
      // Reset the field to the current value on unparsable input.
      (e.target as HTMLInputElement).value = display;
    }
  }

  let display = $derived(
    decimals !== undefined ? value.toFixed(decimals) : value.toString(),
  );
</script>

<div class="stepper" role="group" aria-label={ariaLabel}>
  <button
    type="button"
    class="stepper-btn"
    aria-label="decrease"
    disabled={values ? value <= values[0] : value <= min}
    onpointerdown={(e) => {
      e.preventDefault();
      startRepeat(-1);
    }}
    onpointerup={stopRepeat}
    onpointerleave={stopRepeat}
    onpointercancel={stopRepeat}
    oncontextmenu={(e) => e.preventDefault()}>−</button
  >
  <div class="stepper-field">
    <input
      class="stepper-value"
      type="text"
      inputmode="decimal"
      value={display}
      aria-label={ariaLabel}
      onchange={onInputChange}
      onfocus={(e) => (e.target as HTMLInputElement).select()}
    />
    {#if suffix}
      <span class="stepper-suffix">{suffix}</span>
    {/if}
  </div>
  <button
    type="button"
    class="stepper-btn"
    aria-label="increase"
    disabled={values ? value >= values[values.length - 1] : value >= max}
    onpointerdown={(e) => {
      e.preventDefault();
      startRepeat(1);
    }}
    onpointerup={stopRepeat}
    onpointerleave={stopRepeat}
    onpointercancel={stopRepeat}
    oncontextmenu={(e) => e.preventDefault()}>+</button
  >
</div>

<style>
  .stepper {
    display: inline-flex;
    align-items: stretch;
    border: 1px solid var(--border);
    border-radius: 10px;
    background: var(--bg-input);
    overflow: hidden;
  }
  .stepper-btn {
    width: 44px;
    min-height: 44px;
    border: none;
    background: var(--bg-raised);
    color: var(--text);
    font-size: 22px;
    font-weight: 600;
    cursor: pointer;
    touch-action: manipulation;
    user-select: none;
    -webkit-user-select: none;
    transition: background 0.08s;
  }
  .stepper-btn:first-child {
    border-right: 1px solid var(--border);
  }
  .stepper-btn:last-child {
    border-left: 1px solid var(--border);
  }
  .stepper-btn:active:not(:disabled) {
    background: var(--accent);
    color: var(--bg);
  }
  .stepper-btn:disabled {
    opacity: 0.35;
    cursor: default;
  }
  .stepper-field {
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 0 4px;
    min-width: 58px;
    justify-content: center;
  }
  .stepper-value {
    width: 100%;
    min-width: 40px;
    border: none;
    background: transparent;
    color: var(--text);
    font-size: 17px;
    font-weight: 600;
    text-align: center;
    outline: none;
    font-variant-numeric: tabular-nums;
  }
  .stepper-suffix {
    font-size: 12px;
    color: var(--text-muted);
    flex-shrink: 0;
  }
</style>
