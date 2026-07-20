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
  import { t } from "svelte-i18n";

  interface Props {
    value: number;
    onchange: (value: number) => void;
    min: number;
    max: number;
    /** Increment for the +/- buttons (hold to repeat). */
    step?: number;
    /** Digits shown after the decimal point. */
    decimals?: number;
    /** Rendered after the value, e.g. a unit. */
    suffix?: string;
    /** Logarithmic slider travel — for ranges like frequencies where the
     * low end deserves most of the resolution. */
    log?: boolean;
    /** Double-clicking the slider resets to this value. */
    defaultValue?: number;
    ariaLabel?: string;
  }

  let {
    value,
    onchange,
    min,
    max,
    step = 1,
    decimals,
    suffix = "",
    log = false,
    defaultValue,
    ariaLabel,
  }: Props = $props();

  const REPEAT_DELAY_MS = 450;
  const REPEAT_INTERVAL_MS = 70;
  const SLIDER_RESOLUTION = 1000;

  let repeatTimer: ReturnType<typeof setTimeout> | null = null;

  function clamp(v: number): number {
    return Math.min(max, Math.max(min, v));
  }

  function roundToStep(v: number): number {
    const places = Math.max(
      decimals ?? 0,
      (step.toString().split(".")[1] ?? "").length,
    );
    return parseFloat(v.toFixed(places));
  }

  // Slider position <-> value, linear or logarithmic.
  let sliderPos = $derived.by(() => {
    if (log) {
      const ratio = Math.log(clamp(value) / min) / Math.log(max / min);
      return Math.round(ratio * SLIDER_RESOLUTION);
    }
    return Math.round(((clamp(value) - min) / (max - min)) * SLIDER_RESOLUTION);
  });

  function posToValue(pos: number): number {
    const ratio = pos / SLIDER_RESOLUTION;
    if (log) {
      // Whole units on a log slider; the buttons do fine steps.
      return Math.round(min * Math.pow(max / min, ratio));
    }
    return roundToStep(min + ratio * (max - min));
  }

  let fillPct = $derived((sliderPos / SLIDER_RESOLUTION) * 100);

  function handleSlider(e: Event) {
    onchange(clamp(posToValue(Number((e.target as HTMLInputElement).value))));
  }

  function resetToDefault() {
    if (defaultValue !== undefined) onchange(clamp(defaultValue));
  }

  function bump(direction: 1 | -1) {
    const next = clamp(roundToStep(value + step * direction));
    if (next !== value) onchange(next);
  }

  function startRepeat(direction: 1 | -1) {
    stopRepeat();
    bump(direction);
    const tick = () => {
      bump(direction);
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
      (e.target as HTMLInputElement).value = display;
    }
  }

  let display = $derived(
    decimals !== undefined ? value.toFixed(decimals) : value.toString(),
  );
</script>

<div class="slider-stepper" role="group" aria-label={ariaLabel}>
  <button
    type="button"
    class="ss-btn"
    aria-label="decrease"
    disabled={value <= min}
    onpointerdown={(e) => {
      e.preventDefault();
      startRepeat(-1);
    }}
    onpointerup={stopRepeat}
    onpointerleave={stopRepeat}
    onpointercancel={stopRepeat}
    oncontextmenu={(e) => e.preventDefault()}>−</button
  >
  <input
    type="range"
    class="ss-slider"
    min="0"
    max={SLIDER_RESOLUTION}
    step="1"
    value={sliderPos}
    style="--fill: {fillPct}%"
    aria-label={ariaLabel}
    aria-valuetext={display + suffix}
    title={defaultValue !== undefined
      ? $t("slider.resetHint", { values: { value: defaultValue } })
      : undefined}
    oninput={handleSlider}
    ondblclick={resetToDefault}
  />
  <button
    type="button"
    class="ss-btn"
    aria-label="increase"
    disabled={value >= max}
    onpointerdown={(e) => {
      e.preventDefault();
      startRepeat(1);
    }}
    onpointerup={stopRepeat}
    onpointerleave={stopRepeat}
    onpointercancel={stopRepeat}
    oncontextmenu={(e) => e.preventDefault()}>+</button
  >
  <div class="ss-field">
    <input
      class="ss-value"
      type="text"
      inputmode="decimal"
      value={display}
      aria-label={ariaLabel}
      onchange={onInputChange}
      onfocus={(e) => (e.target as HTMLInputElement).select()}
    />
    {#if suffix}
      <span class="ss-suffix">{suffix}</span>
    {/if}
  </div>
</div>

<style>
  .slider-stepper {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
  }
  .ss-btn {
    flex-shrink: 0;
    width: 40px;
    height: 40px;
    border: 1px solid var(--border);
    border-radius: 10px;
    background: var(--bg-raised);
    color: var(--text);
    font-size: 20px;
    font-weight: 600;
    cursor: pointer;
    touch-action: manipulation;
    user-select: none;
    -webkit-user-select: none;
    transition: background 0.08s;
  }
  .ss-btn:active:not(:disabled) {
    background: var(--accent);
    color: var(--bg);
  }
  .ss-btn:disabled {
    opacity: 0.35;
    cursor: default;
  }
  .ss-slider {
    flex: 1;
    min-width: 40px;
    appearance: none;
    -webkit-appearance: none;
    height: 40px;
    padding: 18px 0;
    margin: 0;
    background: transparent;
    cursor: pointer;
    touch-action: pan-y;
  }
  .ss-slider::-webkit-slider-runnable-track {
    height: 4px;
    border-radius: 2px;
    background: linear-gradient(
      to right,
      var(--accent) var(--fill),
      var(--border) var(--fill)
    );
  }
  .ss-slider::-moz-range-track {
    height: 4px;
    border-radius: 2px;
    background: linear-gradient(
      to right,
      var(--accent) var(--fill),
      var(--border) var(--fill)
    );
  }
  .ss-slider::-webkit-slider-thumb {
    -webkit-appearance: none;
    appearance: none;
    width: 16px;
    height: 16px;
    margin-top: -6px;
    border-radius: 50%;
    border: none;
    background: var(--accent);
  }
  .ss-slider::-moz-range-thumb {
    width: 16px;
    height: 16px;
    border-radius: 50%;
    border: none;
    background: var(--accent);
  }
  .ss-slider:focus-visible {
    outline: none;
  }
  .ss-field {
    flex-shrink: 0;
    display: flex;
    align-items: baseline;
    gap: 2px;
  }
  .ss-value {
    width: 52px;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--bg-input);
    color: var(--text);
    font-size: 14px;
    font-weight: 600;
    text-align: center;
    padding: 6px 4px;
    outline: none;
    font-variant-numeric: tabular-nums;
  }
  .ss-suffix {
    font-size: 11px;
    color: var(--text-muted);
  }
</style>
