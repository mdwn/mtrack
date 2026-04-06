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
  import { formatMs } from "../../lib/util/format";
  import { t } from "svelte-i18n";
  import { get } from "svelte/store";

  let errorMsg = $state("");
  let errorTimer: ReturnType<typeof setTimeout> | null = null;

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
        // Already at end of playlist — not an error.
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
        // Already at beginning of playlist — not an error.
      } else {
        console.error("previous failed:", e);
        showError(get(t)("playback.error.prev"));
      }
    } finally {
      loading = false;
    }
  }

  let loading = $state(false);
  let sectionMenuOpen = $state(false);

  function toggleSectionMenu() {
    sectionMenuOpen = !sectionMenuOpen;
  }

  function closeSectionMenu() {
    sectionMenuOpen = false;
  }

  async function loopSection(name: string) {
    sectionMenuOpen = false;
    try {
      await playerClient.loopSection({ sectionName: name });
    } catch (e) {
      console.error("loop section failed:", e);
      showError(get(t)("playback.error.loopSection"));
    }
  }

  async function stopSectionLoop() {
    sectionMenuOpen = false;
    try {
      await playerClient.stopSectionLoop({});
    } catch (e) {
      console.error("stop section loop failed:", e);
      showError(get(t)("playback.error.stopSectionLoop"));
    }
  }

  function handleKeydown(e: KeyboardEvent) {
    // Only handle shortcuts on the dashboard page to avoid accidental
    // playback triggers when interacting with forms on other pages.
    if (
      window.location.hash !== "#/" &&
      window.location.hash !== "" &&
      window.location.hash !== "#"
    )
      return;

    // Don't intercept when typing in form fields
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
    "139, 92, 246",
    "234, 179, 8",
    "239, 96, 163",
    "34, 197, 94",
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
      // end_measure is inclusive, so the region extends to the start of the next measure
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

    // Find current beat index.
    let beatIdx = 0;
    for (let i = grid.beats.length - 1; i >= 0; i--) {
      if (grid.beats[i] <= elapsed) {
        beatIdx = i;
        break;
      }
    }

    // Find current measure.
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

<div class="card card-full">
  <div class="card-header">
    <span class="card-title">{$t("playback.title")}</span>
    {#if $playbackStore.is_playing && $playbackStore.available_sections.length > 0}
      <div class="section-menu-anchor">
        <button
          class="btn btn-sm"
          class:btn-loop-active={$playbackStore.active_section != null}
          onclick={toggleSectionMenu}
          title={$playbackStore.active_section
            ? `Looping: ${$playbackStore.active_section.name}`
            : $t("playback.sections")}
        >
          {#if $playbackStore.active_section}
            <span class="loop-icon">&#x21BB;</span>
            {$playbackStore.active_section.name}
          {:else}
            {$t("playback.sections")}
          {/if}
        </button>
        {#if sectionMenuOpen}
          <button class="section-menu-backdrop" onclick={closeSectionMenu}
          ></button>
          <div class="section-menu">
            {#if $playbackStore.active_section}
              <button
                class="section-menu-item section-menu-stop"
                onclick={stopSectionLoop}>{$t("playback.stopLoop")}</button
              >
            {/if}
            {#each $playbackStore.available_sections as section (section.name)}
              <button
                class="section-menu-item"
                class:section-menu-item-active={$playbackStore.active_section
                  ?.name === section.name}
                onclick={() => loopSection(section.name)}
                title="m{section.start_measure}-{section.end_measure}"
                >{section.name}</button
              >
            {/each}
          </div>
        {/if}
      </div>
    {/if}
  </div>
  <div class="transport">
    <div class="transport-info">
      <div class="playback-song">
        {$playbackStore.song_name || $t("playback.noSong")}
      </div>
      <div class="playback-status">
        {#if $playbackStore.is_playing}
          <span class="playing">{$t("playback.playing")}</span>
        {:else}
          <span class="stopped">{$t("playback.stopped")}</span>
        {/if}
        {#if $playbackStore.looping}
          <span class="loop-badge">LOOP</span>
        {/if}
        {#if currentBeatInfo}
          <span class="beat-info"
            >m{currentBeatInfo.measure} b{currentBeatInfo.beat}</span
          >
        {/if}
      </div>
    </div>
    <div class="transport-progress">
      <div
        class="progress-bar"
        role="progressbar"
        aria-valuenow={progressPct}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-label={$t("playback.songProgress")}
      >
        {#each sectionRegions as region (region.name)}
          <div
            class="section-region"
            class:section-active-region={region.isActive}
            style:left="{region.startPct}%"
            style:width="{region.widthPct}%"
            style:--section-rgb={region.rgb}
            title={region.name}
          ></div>
        {/each}
        <div class="progress-fill" style:width="{progressPct}%"></div>
      </div>
      <div class="progress-time">
        <span>{formatMs($playbackStore.elapsed_ms)}</span>
        <span>{formatMs($playbackStore.song_duration_ms)}</span>
      </div>
    </div>
    <div class="controls">
      <button
        class="btn"
        onclick={previous}
        disabled={($playbackStore.is_playing && !$playbackStore.looping) ||
          loading ||
          !canPrev}
        title={$t("playback.prevTooltip")}>{$t("playback.prev")}</button
      >
      {#if $playbackStore.is_playing}
        <button
          class="btn btn-primary"
          onclick={stop}
          disabled={loading}
          title={$t("playback.stopTooltip")}>{$t("playback.stop")}</button
        >
      {:else}
        <button
          class="btn btn-primary"
          onclick={play}
          disabled={loading}
          title={$t("playback.playTooltip")}>{$t("playback.play")}</button
        >
      {/if}
      <button
        class="btn"
        onclick={next}
        disabled={($playbackStore.is_playing && !$playbackStore.looping) ||
          loading ||
          !canNext}
        title={$t("playback.nextTooltip")}>{$t("playback.next")}</button
      >
    </div>
  </div>
  {#if errorMsg}
    <div class="playback-error" role="alert">
      <span>{errorMsg}</span>
      <button
        class="error-dismiss-btn"
        onclick={dismissError}
        aria-label={$t("common.dismiss")}>&times;</button
      >
    </div>
  {/if}
</div>

<style>
  .card-header {
    min-height: 28px;
  }
  .transport {
    display: grid;
    grid-template-columns: auto 1fr auto;
    gap: 8px 16px;
    align-items: center;
  }
  .transport-info {
    min-width: 120px;
  }
  .transport-progress {
    min-width: 0;
  }
  .playback-song {
    font-size: var(--text-lg);
    font-weight: 600;
    color: var(--text);
  }
  .playback-status {
    font-size: 14px;
    color: var(--text-muted);
  }
  .playback-status .playing {
    color: var(--green);
  }
  .playback-status .stopped {
    color: var(--text-dim);
  }
  .beat-info {
    margin-left: 8px;
    font-family: var(--mono);
    color: var(--text-dim);
  }
  .section-menu-anchor {
    position: relative;
  }
  .section-menu-backdrop {
    position: fixed;
    inset: 0;
    background: transparent;
    border: none;
    z-index: 99;
    cursor: default;
  }
  .section-menu {
    position: absolute;
    right: 0;
    top: calc(100% + 4px);
    z-index: 100;
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    box-shadow: 0 4px 12px rgba(0, 0, 0, 0.25);
    min-width: 120px;
    padding: 4px;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .section-menu-item {
    background: none;
    border: none;
    color: var(--text);
    font-size: 13px;
    padding: 6px 12px;
    border-radius: var(--radius);
    cursor: pointer;
    text-align: left;
    white-space: nowrap;
  }
  .section-menu-item:hover {
    background: var(--bg-hover);
  }
  .section-menu-item-active {
    color: var(--accent);
    font-weight: 600;
  }
  .section-menu-stop {
    color: var(--red);
    border-bottom: 1px solid var(--border);
    border-radius: var(--radius) var(--radius) 0 0;
    padding-bottom: 8px;
    margin-bottom: 2px;
  }
  .btn-loop-active {
    background: var(--accent);
    color: var(--bg);
    border-color: var(--accent);
  }
  .btn-loop-active:hover {
    opacity: 0.85;
  }
  .loop-icon {
    font-size: 14px;
  }
  .loop-badge {
    margin-left: 8px;
    font-size: var(--text-xs);
    font-weight: 600;
    padding: 2px 8px;
    border-radius: var(--radius);
    background: var(--accent);
    color: var(--bg);
  }
  .progress-bar {
    position: relative;
    height: 10px;
    background: var(--border);
    border-radius: 5px;
    overflow: hidden;
    margin-bottom: 6px;
  }
  .progress-fill {
    position: relative;
    z-index: 2;
    height: 100%;
    background: var(--accent);
    border-radius: 5px;
    transition: width 0.2s linear;
  }
  .section-region {
    position: absolute;
    top: 0;
    height: 100%;
    z-index: 1;
    background: rgba(var(--section-rgb), 0.18);
    border-left: 1px solid rgba(var(--section-rgb), 0.4);
    border-right: 1px solid rgba(var(--section-rgb), 0.4);
  }
  .section-active-region {
    background: rgba(var(--section-rgb), 0.35);
    border-left: 1px solid rgba(var(--section-rgb), 0.7);
    border-right: 1px solid rgba(var(--section-rgb), 0.7);
  }
  .progress-time {
    display: flex;
    justify-content: space-between;
    font-family: var(--mono);
    font-size: 12px;
    color: var(--text-dim);
  }
  .playback-error {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    font-size: 12px;
    color: var(--red);
    background: var(--red-subtle);
    border-radius: var(--radius);
    padding: 6px 12px;
    margin-top: 8px;
  }
  .error-dismiss-btn {
    background: none;
    border: none;
    color: inherit;
    cursor: pointer;
    font-size: 16px;
    padding: 0 4px;
    opacity: 0.7;
    line-height: 1;
  }
  .error-dismiss-btn:hover {
    opacity: 1;
  }
  .controls {
    display: flex;
    gap: 8px;
    flex-shrink: 0;
  }
</style>
