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
  import { Code, ConnectError } from "@connectrpc/connect";
  import { playbackStore } from "../../lib/ws/stores";
  import { playerClient } from "../../lib/grpc/client";
  import BeatIndicator from "./BeatIndicator.svelte";
  import { formatMs } from "../../lib/util/format";
  import { t } from "svelte-i18n";
  import { get } from "svelte/store";

  let errorMsg = $state("");
  let errorTimer: ReturnType<typeof setTimeout> | null = null;
  let loading = $state(false);

  function showError(msg: string) {
    errorMsg = msg;
    if (errorTimer) clearTimeout(errorTimer);
    errorTimer = setTimeout(() => (errorMsg = ""), 8000);
  }

  function dismissError() {
    if (errorTimer) clearTimeout(errorTimer);
    errorMsg = "";
  }

  async function play() {
    loading = true;
    try {
      await playerClient.play({});
    } catch (e) {
      console.error("play failed:", e);
      showError(get(t)("playback.error.play"));
    } finally {
      loading = false;
    }
  }

  async function stop() {
    loading = true;
    try {
      await playerClient.stop({});
    } catch (e) {
      console.error("stop failed:", e);
      showError(get(t)("playback.error.stop"));
    } finally {
      loading = false;
    }
  }

  async function next() {
    loading = true;
    try {
      await playerClient.next({});
    } catch (e) {
      if (e instanceof ConnectError && e.code === Code.OutOfRange) {
        // Already at end — silent.
      } else {
        console.error("next failed:", e);
        showError(get(t)("playback.error.next"));
      }
    } finally {
      loading = false;
    }
  }

  async function previous() {
    loading = true;
    try {
      await playerClient.previous({});
    } catch (e) {
      if (e instanceof ConnectError && e.code === Code.OutOfRange) {
        // Already at start — silent.
      } else {
        console.error("previous failed:", e);
        showError(get(t)("playback.error.prev"));
      }
    } finally {
      loading = false;
    }
  }

  async function loopSection(name: string) {
    try {
      await playerClient.loopSection({ sectionName: name });
    } catch (e) {
      console.error("loop section failed:", e);
      showError(get(t)("playback.error.loopSection"));
    }
  }

  async function seekToMs(ms: number) {
    const duration = $playbackStore.song_duration_ms;
    if (duration <= 0) return;
    const clamped = Math.max(0, Math.min(ms, duration));
    try {
      await playerClient.seek({
        position: {
          seconds: BigInt(Math.floor(clamped / 1000)),
          nanos: Math.round((clamped % 1000) * 1e6),
        },
      });
    } catch (e) {
      console.error("seek failed:", e);
      showError(get(t)("playback.error.seek"));
    }
  }

  async function seekToSection(name: string) {
    try {
      await playerClient.seekToSection({ sectionName: name });
    } catch (e) {
      console.error("seek to section failed:", e);
      showError(get(t)("playback.error.seek"));
    }
  }

  function onScrubClick(e: MouseEvent) {
    if ($playbackStore.song_duration_ms <= 0) return;
    const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
    if (rect.width <= 0) return;
    const frac = (e.clientX - rect.left) / rect.width;
    seekToMs(frac * $playbackStore.song_duration_ms);
  }

  function onScrubKeydown(e: KeyboardEvent) {
    if ($playbackStore.song_duration_ms <= 0) return;
    if (e.code !== "ArrowLeft" && e.code !== "ArrowRight") return;
    e.preventDefault();
    // Keep the global next/prev shortcuts from also firing.
    e.stopPropagation();
    const base = $playbackStore.is_playing
      ? $playbackStore.elapsed_ms
      : ($playbackStore.pending_start_ms ?? $playbackStore.elapsed_ms);
    seekToMs(base + (e.code === "ArrowLeft" ? -5000 : 5000));
  }

  // A pilot hint is surfaced from shortly before its audio/anchor until it
  // has passed; it renders highlighted while its audio window is live.
  const HINT_LEAD_MS = 5000;
  let currentHint = $derived.by(() => {
    const hints = $playbackStore.pilot_hints;
    if (hints.length === 0) return null;
    const elapsed = $playbackStore.elapsed_ms;
    let best = null;
    for (const hint of hints) {
      const visibleFrom = Math.min(hint.start_ms, hint.at_ms - HINT_LEAD_MS);
      const visibleTo = Math.max(hint.end_ms, hint.at_ms);
      if (elapsed >= visibleFrom && elapsed <= visibleTo) {
        best = hint;
      }
    }
    return best;
  });
  let hintLive = $derived(
    currentHint != null &&
      $playbackStore.elapsed_ms >= currentHint.start_ms &&
      $playbackStore.elapsed_ms <=
        Math.max(currentHint.end_ms, currentHint.at_ms),
  );

  let pendingStartPct = $derived(
    !$playbackStore.is_playing &&
      $playbackStore.pending_start_ms != null &&
      $playbackStore.song_duration_ms > 0
      ? ($playbackStore.pending_start_ms / $playbackStore.song_duration_ms) *
          100
      : null,
  );

  async function stopSectionLoop() {
    try {
      await playerClient.stopSectionLoop({});
    } catch (e) {
      console.error("stop section loop failed:", e);
      showError(get(t)("playback.error.stopSectionLoop"));
    }
  }

  function handleKeydown(e: KeyboardEvent) {
    if (
      window.location.hash !== "#/" &&
      window.location.hash !== "" &&
      window.location.hash !== "#"
    )
      return;
    const tag = (e.target as HTMLElement)?.tagName;
    if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT") return;

    switch (e.code) {
      case "Space":
        e.preventDefault();
        if ($playbackStore.is_playing) stop();
        else play();
        break;
      case "ArrowRight":
        e.preventDefault();
        next();
        break;
      case "ArrowLeft":
        e.preventDefault();
        previous();
        break;
    }
  }

  let canPrev = $derived(
    $playbackStore.playlist_songs.length > 0 &&
      $playbackStore.playlist_position > 0,
  );
  let canNext = $derived(
    $playbackStore.playlist_songs.length > 0 &&
      $playbackStore.playlist_position <
        $playbackStore.playlist_songs.length - 1,
  );

  let progressPct = $derived(
    $playbackStore.song_duration_ms > 0
      ? ($playbackStore.elapsed_ms / $playbackStore.song_duration_ms) * 100
      : 0,
  );

  const SECTION_COLORS = [
    "94, 202, 234",
    "239, 96, 163",
    "242, 181, 68",
    "77, 192, 138",
    "139, 92, 246",
    "249, 115, 22",
  ];

  function measureToMs(
    grid: { beats: number[]; measure_starts: number[] },
    measure: number,
    durationMs: number,
  ): number {
    const idx = measure - 1;
    if (idx < 0) return 0;
    if (idx >= grid.measure_starts.length) return durationMs;
    return grid.beats[grid.measure_starts[idx]] * 1000;
  }

  let sectionRegions = $derived.by(() => {
    const grid = $playbackStore.beat_grid;
    const dur = $playbackStore.song_duration_ms;
    const sections = $playbackStore.available_sections;
    const active = $playbackStore.active_section;
    if (!grid || dur <= 0 || sections.length === 0) return [];

    return sections.map((s, i) => {
      const startPct = (measureToMs(grid, s.start_measure, dur) / dur) * 100;
      const endPct = (measureToMs(grid, s.end_measure + 1, dur) / dur) * 100;
      const isActive = active?.name === s.name;
      const rgb = SECTION_COLORS[i % SECTION_COLORS.length];
      return {
        name: s.name,
        startPct,
        widthPct: endPct - startPct,
        rgb,
        isActive,
      };
    });
  });

  let currentBeatInfo = $derived.by(() => {
    const grid = $playbackStore.beat_grid;
    if (!grid || grid.beats.length === 0) return null;
    const elapsed = $playbackStore.elapsed_ms / 1000;

    let beatIdx = 0;
    for (let i = grid.beats.length - 1; i >= 0; i--) {
      if (grid.beats[i] <= elapsed) {
        beatIdx = i;
        break;
      }
    }

    let measureIdx = 0;
    for (let i = grid.measure_starts.length - 1; i >= 0; i--) {
      if (grid.measure_starts[i] <= beatIdx) {
        measureIdx = i;
        break;
      }
    }

    const beatInMeasure = beatIdx - grid.measure_starts[measureIdx] + 1;
    return { measure: measureIdx + 1, beat: beatInMeasure };
  });
</script>

<svelte:window onkeydown={handleKeydown} />

<section
  class="card card--hero playback-card"
  aria-label={$t("playback.title")}
>
  <div class="playback-card__corner" aria-hidden="true">
    <span class="pixeq__cell pixeq__cell--cyan"></span>
    <span class="pixeq__cell"></span>
    <span class="pixeq__cell pixeq__cell--pink"></span>
    <span class="pixeq__cell pixeq__cell--cyan"></span>
    <span class="pixeq__cell pixeq__cell--pink"></span>
    <span class="pixeq__cell"></span>
    <span class="pixeq__cell"></span>
    <span class="pixeq__cell pixeq__cell--cyan"></span>
    <span class="pixeq__cell pixeq__cell--pink"></span>
    <span class="pixeq__cell"></span>
    <span class="pixeq__cell pixeq__cell--cyan"></span>
    <span class="pixeq__cell"></span>
  </div>

  <div class="playback-card__body">
    <div
      class="overline playback-card__state"
      class:playback-card__state--playing={$playbackStore.is_playing}
    >
      <span class="playback-card__dot" aria-hidden="true">
        {#if $playbackStore.is_playing}●{:else}■{/if}
      </span>
      {#if $playbackStore.is_playing}
        {$t("playback.playing")}
      {:else}
        {$t("playback.stopped")}
      {/if}
      {#if $playbackStore.looping}
        <span class="badge badge--ctrl">LOOP</span>
      {/if}
    </div>

    <div class="playback-card__head">
      <div class="playback-card__heading">
        <h2 class="playback-card__title">
          {$playbackStore.song_name || $t("playback.noSong")}
        </h2>
        <div class="mono playback-card__meta">
          {#if $playbackStore.tracks.length > 0}
            {$playbackStore.tracks.length} tracks
          {/if}
          {#if $playbackStore.tempo}
            · {Math.round($playbackStore.tempo.bpm)} BPM {$playbackStore.tempo
              .time_signature[0]}/{$playbackStore.tempo.time_signature[1]}
          {/if}
          {#if currentBeatInfo}
            · beat {currentBeatInfo.beat} / measure {currentBeatInfo.measure}
          {/if}
          <BeatIndicator />
        </div>
      </div>
      <div class="playback-card__transport">
        <button
          class="btn-icon-circle"
          onclick={previous}
          disabled={($playbackStore.is_playing && !$playbackStore.looping) ||
            loading ||
            !canPrev}
          title={$t("playback.prevTooltip")}
          aria-label={$t("playback.prev")}
        >
          <svg width="16" height="16" viewBox="0 0 24 24" fill="currentColor"
            ><path d="M6 5h2v14H6zM20 5L9 12l11 7V5z" /></svg
          >
        </button>
        <button
          class="btn-icon-circle"
          onclick={stop}
          disabled={loading || !$playbackStore.is_playing}
          title={$t("playback.stopTooltip")}
          aria-label={$t("playback.stop")}
        >
          <svg width="16" height="16" viewBox="0 0 24 24" fill="currentColor"
            ><rect x="6" y="6" width="12" height="12" rx="1" /></svg
          >
        </button>
        <button
          class="btn-play"
          class:btn-play--playing={$playbackStore.is_playing}
          onclick={$playbackStore.is_playing ? stop : play}
          disabled={loading}
          title={$playbackStore.is_playing
            ? $t("playback.pauseTooltip")
            : $t("playback.playTooltip")}
          aria-label={$playbackStore.is_playing
            ? $t("playback.pause")
            : $t("playback.play")}
        >
          {#if $playbackStore.is_playing}
            <svg width="20" height="20" viewBox="0 0 24 24" fill="currentColor"
              ><rect x="6" y="5" width="4" height="14" rx="1" /><rect
                x="14"
                y="5"
                width="4"
                height="14"
                rx="1"
              /></svg
            >
          {:else}
            <svg width="20" height="20" viewBox="0 0 24 24" fill="currentColor"
              ><path d="M8 5v14l11-7z" /></svg
            >
          {/if}
        </button>
        <button
          class="btn-icon-circle"
          onclick={next}
          disabled={($playbackStore.is_playing && !$playbackStore.looping) ||
            loading ||
            !canNext}
          title={$t("playback.nextTooltip")}
          aria-label={$t("playback.next")}
        >
          <svg width="16" height="16" viewBox="0 0 24 24" fill="currentColor"
            ><path d="M16 5h2v14h-2zM4 5l11 7L4 19V5z" /></svg
          >
        </button>
      </div>
    </div>

    <div class="playback-card__progress">
      <span class="mono playback-card__time"
        >{formatMs($playbackStore.elapsed_ms)}</span
      >
      <div
        class="scrub scrub--seekable"
        class:scrub--playing={$playbackStore.is_playing}
        role="slider"
        tabindex="0"
        aria-valuenow={progressPct}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-label={$t("playback.songProgress")}
        title={$t("playback.seekTooltip")}
        onclick={onScrubClick}
        onkeydown={onScrubKeydown}
      >
        {#each sectionRegions as region (region.name)}
          <div
            class="scrub__region"
            class:scrub__region--active={region.isActive}
            style:left="{region.startPct}%"
            style:width="{region.widthPct}%"
            style:--section-rgb={region.rgb}
            title={region.name}
          ></div>
        {/each}
        {#if $playbackStore.song_duration_ms > 0}
          {#each $playbackStore.pilot_hints as hint (hint.label + hint.at_ms)}
            <div
              class="scrub__hint"
              style:left="{(hint.at_ms / $playbackStore.song_duration_ms) *
                100}%"
              title={hint.label}
            ></div>
          {/each}
        {/if}
        <div class="scrub__fill" style:width="{progressPct}%"></div>
        {#if pendingStartPct != null}
          <div
            class="scrub__pending"
            style:left="{pendingStartPct}%"
            title={formatMs($playbackStore.pending_start_ms ?? 0)}
          ></div>
        {/if}
      </div>
      <span class="mono playback-card__time"
        >{formatMs($playbackStore.song_duration_ms)}</span
      >
    </div>

    {#if currentHint && $playbackStore.is_playing}
      <div
        class="mono playback-card__hint"
        class:playback-card__hint--live={hintLive}
      >
        <span aria-hidden="true">🎙</span>
        {currentHint.label}
      </div>
    {/if}

    {#if $playbackStore.available_sections.length > 0}
      <div class="playback-card__sections">
        <span class="overline">{$t("playback.sections")}</span>
        {#if $playbackStore.active_section}
          <button
            class="badge badge--pill badge--active section-chip"
            onclick={stopSectionLoop}
            title={$t("playback.stopLoop")}
          >
            <span aria-hidden="true">↻</span>
            {$playbackStore.active_section.name}
            <span aria-hidden="true">×</span>
          </button>
        {/if}
        {#each $playbackStore.available_sections as section (section.name)}
          {#if !$playbackStore.active_section || $playbackStore.active_section.name !== section.name}
            <span class="section-chip-group">
              <button
                class="badge badge--pill section-chip section-chip--grouped"
                onclick={() => seekToSection(section.name)}
                title="m{section.start_measure}-{section.end_measure} — {$t(
                  'playback.seekToSection',
                )}"
              >
                {section.name}
              </button>
              <button
                class="section-chip-loop"
                onclick={() => loopSection(section.name)}
                disabled={!$playbackStore.is_playing}
                title={$t("playback.loopSectionTooltip")}
                aria-label="{$t('playback.loopSectionTooltip')}: {section.name}"
              >
                ↻
              </button>
            </span>
          {/if}
        {/each}
      </div>
    {/if}

    {#if errorMsg}
      <div class="playback-card__error" role="alert">
        <span>{errorMsg}</span>
        <button
          class="error-dismiss-btn"
          onclick={dismissError}
          aria-label={$t("common.dismiss")}>×</button
        >
      </div>
    {/if}
  </div>
</section>

<style>
  .playback-card {
    position: relative;
    overflow: hidden;
    padding: 0;
    margin-bottom: 24px;
  }
  .playback-card__corner {
    position: absolute;
    top: 14px;
    right: 18px;
    display: grid;
    grid-template-columns: repeat(4, 6px);
    grid-template-rows: repeat(3, 6px);
    gap: 2px;
    opacity: 0.6;
    pointer-events: none;
  }
  .playback-card__corner :global(.pixeq__cell) {
    width: 6px;
    height: 6px;
    border-radius: 1px;
    background: var(--nc-gray-300);
  }
  :global(.nc--dark) .playback-card__corner :global(.pixeq__cell) {
    background: var(--nc-gray-700);
  }
  .playback-card__body {
    padding: 28px;
  }
  .playback-card__state {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    color: var(--nc-fg-3);
  }
  .playback-card__state--playing {
    color: var(--nc-pink-600);
  }
  :global(.nc--dark) .playback-card__state--playing {
    color: var(--nc-pink-300);
  }
  .playback-card__dot {
    font-size: 9px;
    line-height: 1;
  }
  .playback-card__head {
    display: flex;
    align-items: flex-end;
    justify-content: space-between;
    margin-top: 6px;
    gap: 24px;
    flex-wrap: wrap;
  }
  .playback-card__heading {
    flex: 1;
    min-width: 0;
  }
  .playback-card__title {
    font-family: var(--nc-font-display);
    font-weight: 800;
    font-size: 36px;
    line-height: 1.05;
    letter-spacing: -0.02em;
    color: var(--nc-fg-1);
    margin: 0;
    overflow-wrap: break-word;
    word-break: break-word;
  }
  .playback-card__meta {
    margin-top: 8px;
    color: var(--nc-fg-3);
  }
  .playback-card__transport {
    display: flex;
    gap: 8px;
    align-items: center;
  }
  .btn-icon-circle {
    width: 36px;
    height: 36px;
    border-radius: 999px;
    border: 1px solid var(--nc-border-2);
    background: var(--card-bg);
    color: var(--nc-fg-1);
    cursor: pointer;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    transition:
      background var(--nc-dur-fast) var(--nc-ease),
      border-color var(--nc-dur-fast) var(--nc-ease);
  }
  .btn-icon-circle:hover:not(:disabled) {
    background: var(--nc-bg-2);
    border-color: var(--nc-fg-3);
  }
  .btn-icon-circle:disabled {
    opacity: 0.35;
    cursor: not-allowed;
  }
  .btn-play {
    width: 48px;
    height: 48px;
    border-radius: 999px;
    border: 1px solid var(--nc-cyan-500);
    background: var(--nc-cyan-400);
    color: var(--nc-ink);
    cursor: pointer;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    transition:
      background var(--nc-dur-fast) var(--nc-ease),
      border-color var(--nc-dur-fast) var(--nc-ease),
      transform var(--nc-dur-fast) var(--nc-ease);
  }
  .btn-play:hover:not(:disabled) {
    background: var(--nc-cyan-500);
  }
  .btn-play--playing {
    background: var(--nc-pink-400);
    border-color: var(--nc-pink-500);
  }
  .btn-play--playing:hover:not(:disabled) {
    background: var(--nc-pink-500);
  }
  .btn-play:disabled {
    opacity: 0.45;
    cursor: not-allowed;
  }

  .playback-card__progress {
    display: flex;
    align-items: center;
    gap: 12px;
    margin-top: 24px;
  }
  .playback-card__time {
    font-family: var(--nc-font-mono);
    font-size: 12px;
    color: var(--nc-fg-3);
    min-width: 44px;
  }
  .playback-card__time:last-child {
    text-align: right;
  }
  .scrub {
    flex: 1;
    position: relative;
    height: 8px;
    background: var(--nc-bg-3);
    border-radius: 999px;
    overflow: hidden;
  }
  .scrub__fill {
    position: absolute;
    inset: 0;
    background: var(--nc-cyan-400);
    width: 0%;
    border-radius: 999px;
    transition: width 0.2s linear;
    z-index: 2;
  }
  .scrub--playing .scrub__fill {
    background: var(--nc-pink-400);
  }
  .scrub__region {
    position: absolute;
    top: 0;
    height: 100%;
    z-index: 1;
    background: rgba(var(--section-rgb), 0.18);
    border-left: 1px solid rgba(var(--section-rgb), 0.4);
    border-right: 1px solid rgba(var(--section-rgb), 0.4);
  }
  .scrub__region--active {
    background: rgba(var(--section-rgb), 0.4);
    border-left-color: rgba(var(--section-rgb), 0.7);
    border-right-color: rgba(var(--section-rgb), 0.7);
  }
  .scrub--seekable {
    cursor: pointer;
  }
  .scrub--seekable:focus-visible {
    outline: 2px solid var(--nc-cyan-400);
    outline-offset: 2px;
  }
  .scrub__pending {
    position: absolute;
    top: -2px;
    bottom: -2px;
    width: 3px;
    margin-left: -1px;
    background: var(--nc-cyan-400);
    border-radius: 1px;
    z-index: 3;
  }
  .scrub__hint {
    position: absolute;
    top: -3px;
    width: 2px;
    height: 6px;
    margin-left: -1px;
    background: var(--nc-amber-400, #f2b544);
    border-radius: 1px;
    z-index: 3;
    pointer-events: none;
  }
  .playback-card__hint {
    margin-top: 10px;
    font-size: 13px;
    color: var(--nc-fg-3);
    transition: color var(--nc-dur-fast) var(--nc-ease);
  }
  .playback-card__hint--live {
    color: var(--nc-amber-400, #f2b544);
    font-weight: 600;
  }

  .playback-card__sections {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-top: 18px;
    flex-wrap: wrap;
  }
  .section-chip {
    cursor: pointer;
    border: 1px solid var(--card-border);
    background: var(--nc-bg-2);
    color: var(--nc-fg-2);
    transition:
      background var(--nc-dur-fast) var(--nc-ease),
      color var(--nc-dur-fast) var(--nc-ease),
      border-color var(--nc-dur-fast) var(--nc-ease);
  }
  .section-chip:hover:not(:disabled) {
    background: var(--nc-bg-3);
    color: var(--nc-fg-1);
  }
  .section-chip:disabled {
    cursor: not-allowed;
    opacity: 0.55;
  }
  .section-chip-group {
    display: inline-flex;
    align-items: stretch;
  }
  .section-chip--grouped {
    border-top-right-radius: 0;
    border-bottom-right-radius: 0;
    border-right: none;
  }
  .section-chip-loop {
    cursor: pointer;
    border: 1px solid var(--card-border);
    border-top-right-radius: 999px;
    border-bottom-right-radius: 999px;
    background: var(--nc-bg-2);
    color: var(--nc-fg-2);
    font-size: 12px;
    line-height: 1;
    padding: 0 8px 0 6px;
    transition:
      background var(--nc-dur-fast) var(--nc-ease),
      color var(--nc-dur-fast) var(--nc-ease);
  }
  .section-chip-loop:hover:not(:disabled) {
    background: var(--nc-bg-3);
    color: var(--nc-fg-1);
  }
  .section-chip-loop:disabled {
    cursor: not-allowed;
    opacity: 0.55;
  }
  .badge--pill.badge--active.section-chip {
    background: var(--nc-cyan-400);
    border-color: var(--nc-cyan-500);
    color: var(--nc-ink);
  }
  .badge--pill.badge--active.section-chip:hover:not(:disabled) {
    background: var(--nc-cyan-500);
    color: var(--nc-ink);
  }

  .playback-card__error {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    margin-top: 16px;
    font-size: 13px;
    color: var(--nc-error);
    background: rgba(232, 75, 75, 0.12);
    border: 1px solid rgba(232, 75, 75, 0.4);
    border-radius: var(--nc-radius-sm);
    padding: 8px 12px;
  }
  .error-dismiss-btn {
    background: none;
    border: none;
    color: inherit;
    cursor: pointer;
    font-size: 18px;
    line-height: 1;
    padding: 0 4px;
    opacity: 0.7;
  }
  .error-dismiss-btn:hover {
    opacity: 1;
  }

  @media (max-width: 720px) {
    .playback-card__body {
      padding: 20px;
    }
    .playback-card__title {
      font-size: 26px;
    }
    .playback-card__head {
      gap: 16px;
    }
    .playback-card__transport {
      width: 100%;
      justify-content: flex-end;
    }
  }
</style>
