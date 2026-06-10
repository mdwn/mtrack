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
  import { GAIN_MIN, GAIN_MAX, GAIN_STEP, formatDb } from "../lib/gain";
  import { t } from "svelte-i18n";

  interface Props {
    /** Server-pushed gain value in dB. */
    value?: number;
    /** Accessible label for the slider. */
    label: string;
    /** Called (throttled upstream) while dragging. */
    oninput: (db: number) => void;
    /** Called with the final value on commit (change/reset). */
    oncommit: (db: number) => void;
  }

  let { value = 0, label, oninput, oncommit }: Props = $props();

  // Optimistic local state: show the local value while dragging and until
  // the server echoes (within epsilon) or a timeout passes, so the 5 Hz
  // playback poller can't snap the thumb backwards mid-interaction.
  const ECHO_EPSILON = 0.01;
  const ECHO_TIMEOUT_MS = 1500;

  let dragging = $state(false);
  let localDb = $state(0);
  let lastSent: number | null = $state(null);
  let holdUntil = $state(0);

  const shown = $derived(
    dragging ||
      (lastSent !== null &&
        Date.now() < holdUntil &&
        Math.abs(value - lastSent) > ECHO_EPSILON)
      ? localDb
      : value,
  );
  const muted = $derived(shown <= GAIN_MIN);
  const fillPct = $derived(((shown - GAIN_MIN) / (GAIN_MAX - GAIN_MIN)) * 100);

  function beginDrag() {
    if (!dragging) {
      localDb = shown;
      dragging = true;
    }
  }

  function handleInput(e: Event) {
    beginDrag();
    localDb = Number((e.currentTarget as HTMLInputElement).value);
    oninput(localDb);
  }

  function commit(db: number) {
    localDb = db;
    lastSent = db;
    holdUntil = Date.now() + ECHO_TIMEOUT_MS;
    dragging = false;
    oncommit(db);
  }

  function handleChange(e: Event) {
    commit(Number((e.currentTarget as HTMLInputElement).value));
  }

  function reset() {
    commit(0);
  }
</script>

<div class="gain-slider">
  <input
    type="range"
    class="gain-slider__input"
    min={GAIN_MIN}
    max={GAIN_MAX}
    step={GAIN_STEP}
    value={shown}
    style="--fill: {fillPct}%"
    aria-label={label}
    aria-valuetext={formatDb(shown)}
    onpointerdown={beginDrag}
    oninput={handleInput}
    onchange={handleChange}
    ondblclick={reset}
  />
  <button
    type="button"
    class="gain-slider__db mono"
    class:gain-slider__db--muted={muted}
    title={$t("tracks.gainReset")}
    aria-label={$t("tracks.gainReset")}
    onclick={reset}
  >
    {formatDb(shown)}
  </button>
</div>

<style>
  .gain-slider {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
  }
  .gain-slider__input {
    flex: 1;
    min-width: 0;
    appearance: none;
    -webkit-appearance: none;
    height: 28px;
    padding: 12px 0;
    margin: 0;
    background: transparent;
    cursor: pointer;
  }
  .gain-slider__input::-webkit-slider-runnable-track {
    height: 4px;
    border-radius: 2px;
    background: linear-gradient(
      to right,
      var(--nc-cyan-500) var(--fill),
      var(--card-border) var(--fill)
    );
  }
  .gain-slider__input::-moz-range-track {
    height: 4px;
    border-radius: 2px;
    background: linear-gradient(
      to right,
      var(--nc-cyan-500) var(--fill),
      var(--card-border) var(--fill)
    );
  }
  .gain-slider__input::-webkit-slider-thumb {
    -webkit-appearance: none;
    appearance: none;
    width: 14px;
    height: 14px;
    margin-top: -5px;
    border-radius: 50%;
    border: none;
    background: var(--nc-cyan-400);
  }
  .gain-slider__input::-moz-range-thumb {
    width: 14px;
    height: 14px;
    border-radius: 50%;
    border: none;
    background: var(--nc-cyan-400);
  }
  .gain-slider__input:focus-visible {
    outline: none;
  }
  .gain-slider__input:focus-visible::-webkit-slider-thumb {
    box-shadow: var(--nc-glow-cyan);
  }
  .gain-slider__input:focus-visible::-moz-range-thumb {
    box-shadow: var(--nc-glow-cyan);
  }
  .gain-slider__db {
    flex: 0 0 64px;
    text-align: right;
    font-size: 11px;
    color: var(--nc-fg-2);
    background: none;
    border: none;
    padding: 4px 0;
    cursor: pointer;
  }
  .gain-slider__db:hover {
    color: var(--nc-cyan-400);
  }
  .gain-slider__db--muted {
    color: var(--nc-fg-3);
  }
</style>
