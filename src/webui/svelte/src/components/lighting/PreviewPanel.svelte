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
  /**
   * The 3D view's Preview mode (lighting UI design, section 12.1): pick a
   * song, scrub through it, and see what its show does at that moment.
   * The show is evaluated offline; this panel hands the scene the same
   * three maps the live state message carries and never touches the
   * engine, so a playing song plays on.
   */
  import { onDestroy, onMount } from "svelte";
  import { t } from "svelte-i18n";
  import { BadAnswerError } from "../../lib/lighting/answer";
  import { fetchSongs, type SongSummary } from "../../lib/api/songs";
  import {
    clock,
    evaluatePreview,
    groupActivity,
    sectionSegments,
    timelineLink,
    type PreviewEvaluation,
    type PreviewFrame,
  } from "../../lib/lighting/preview";
  import { playbackStore } from "../../lib/ws/stores";

  interface Props {
    /** Song to open on, from the address. */
    initialSong?: string | null;
    /** Time to open at, seconds, from the address. */
    initialTime?: number | null;
    /** Called with what the scene should draw; null when there is nothing. */
    onfeed: (frame: PreviewFrame | null) => void;
    /** Called with each moment evaluated, so the host can keep it in the
     *  address. */
    onmoment?: (song: string, time: number) => void;
  }

  let {
    initialSong = null,
    initialTime = null,
    onfeed,
    onmoment,
  }: Props = $props();

  /** How long scrubbing rests before the show is evaluated. */
  const DEBOUNCE_MS = 120;

  let songs = $state<SongSummary[]>([]);
  let loaded = $state(false);
  let songName = $state("");
  let time = $state(0);
  let evaluation = $state<PreviewEvaluation | null>(null);
  let untouched = $state<string[]>([]);
  let error = $state<string | null>(null);
  let busy = $state(false);

  const song = $derived(songs.find((s) => s.name === songName) ?? null);
  const duration = $derived(song ? song.duration_ms / 1000 : 0);
  const segments = $derived(song ? sectionSegments(song) : []);
  const activity = $derived(groupActivity(evaluation?.active_effects ?? []));

  let timer: ReturnType<typeof setTimeout> | undefined;
  /** Answers arriving after a newer request went out are stale. */
  let sequence = 0;

  onMount(async () => {
    try {
      const { songs: all } = await fetchSongs();
      songs = all
        .filter((s) => s.has_lighting)
        .sort((a, b) => a.name.localeCompare(b.name));
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    }
    loaded = true;
    const playing = $playbackStore.song_name;
    const pick =
      songs.find((s) => s.name === initialSong) ??
      songs.find((s) => s.name === playing) ??
      songs[0];
    if (pick) {
      songName = pick.name;
      time = Math.min(initialTime ?? 0, pick.duration_ms / 1000);
      void evaluateNow();
    }
  });

  onDestroy(() => {
    clearTimeout(timer);
    sequence++;
    onfeed(null);
  });

  async function evaluateNow() {
    clearTimeout(timer);
    if (!songName) return;
    const mine = ++sequence;
    busy = true;
    try {
      const result = await evaluatePreview(songName, [time]);
      if (mine !== sequence) return;
      const at = result.evaluations[0];
      onmoment?.(songName, time);
      evaluation = at ?? null;
      untouched = result.untouched;
      error = null;
      onfeed(
        at ? { fixtures: at.fixtures, poses: at.poses, cells: at.cells } : null,
      );
    } catch (e) {
      if (mine !== sequence) return;
      evaluation = null;
      untouched = [];
      error =
        e instanceof BadAnswerError
          ? $t("lighting.badAnswer")
          : e instanceof Error
            ? e.message
            : String(e);
      onfeed(null);
    } finally {
      if (mine === sequence) busy = false;
    }
  }

  function scrub(e: Event) {
    time = Number((e.currentTarget as HTMLInputElement).value);
    clearTimeout(timer);
    timer = setTimeout(evaluateNow, DEBOUNCE_MS);
  }

  function chooseSong(e: Event) {
    songName = (e.currentTarget as HTMLSelectElement).value;
    time = 0;
    evaluation = null;
    void evaluateNow();
  }
</script>

<div class="preview" data-testid="preview-panel">
  {#if loaded && songs.length === 0 && !error}
    <p class="preview__quiet" data-testid="preview-no-songs">
      {$t("preview.noSongs")}
    </p>
  {:else}
    <div class="preview__controls">
      <div class="preview__song">
        <label for="preview-song">{$t("preview.song")}</label>
        <select
          id="preview-song"
          class="input"
          value={songName}
          onchange={chooseSong}
          disabled={songs.length === 0}
        >
          {#each songs as s (s.name)}
            <option value={s.name}>{s.name}</option>
          {/each}
        </select>
      </div>

      <div class="preview__scrub">
        <label for="preview-time">{$t("preview.time")}</label>
        <div class="preview__scrubline">
          <div class="preview__scrubber">
            {#if segments.length > 0}
              <div
                class="preview__sections"
                data-testid="preview-sections"
                role="img"
                aria-label={$t("preview.sections", {
                  values: { names: segments.map((s) => s.name).join(", ") },
                })}
              >
                {#each segments as seg (seg.name + seg.start)}
                  <span
                    class="preview__section"
                    style:left="{seg.start}%"
                    style:width="{seg.width}%"
                    style:--section={seg.color}
                    title={seg.name}>{seg.name}</span
                  >
                {/each}
              </div>
            {/if}
            <input
              id="preview-time"
              type="range"
              class="preview__range"
              min="0"
              max={duration}
              step="0.1"
              value={time}
              disabled={!song}
              aria-valuetext={clock(time, true)}
              oninput={scrub}
            />
          </div>
          <output
            class="preview__clock"
            for="preview-time"
            data-testid="preview-clock"
            >{clock(time, true)} / {clock(duration)}</output
          >
        </div>
      </div>
    </div>

    {#if error}
      <p class="preview__error" role="alert" data-testid="preview-error">
        {error}
      </p>
    {/if}

    {#if evaluation}
      <div class="preview__panels" aria-busy={busy}>
        <section class="preview__panel" aria-labelledby="preview-now">
          <h3 id="preview-now" class="preview__heading">
            {$t("preview.now")}
          </h3>
          {#if activity.length === 0}
            <p class="preview__quiet" data-testid="preview-idle">
              {$t("preview.nothingRunning")}
            </p>
          {:else}
            <ul class="preview__activity" data-testid="preview-activity">
              {#each activity as row (row.group)}
                <li>
                  <strong>{row.group}</strong
                  >{#each row.effects as effect, i (i)}
                    <span class="preview__effect"
                      >{i === 0 ? ": " : "; "}{$t("preview.effect", {
                        values: {
                          kind: $t(`preview.kind.${effect.kind}`, {
                            default: effect.kind.toLowerCase(),
                          }),
                          percent: effect.percent,
                          elapsed: effect.elapsed.toFixed(1),
                          duration: effect.duration.toFixed(1),
                        },
                      })}</span
                    >
                  {/each}
                </li>
              {/each}
            </ul>
          {/if}
        </section>

        <section class="preview__panel" aria-labelledby="preview-untouched">
          <h3 id="preview-untouched" class="preview__heading">
            {$t("preview.untouched")}
          </h3>
          {#if untouched.length === 0}
            <p class="preview__quiet" data-testid="preview-untouched-count">
              {$t("preview.allTargeted")}
            </p>
          {:else}
            <details>
              <summary data-testid="preview-untouched-count"
                >{$t("preview.untouchedCount", {
                  values: { count: untouched.length },
                })}</summary
              >
              <p class="preview__names" data-testid="preview-untouched-names">
                {untouched.join(", ")}
              </p>
            </details>
            <p class="preview__quiet">{$t("preview.untouchedHint")}</p>
          {/if}
        </section>

        <a
          class="btn btn-sm preview__open"
          data-testid="preview-open-timeline"
          href={timelineLink(songName, time)}>{$t("preview.openTimeline")}</a
        >
      </div>
    {/if}
  {/if}
</div>

<style>
  .preview {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .preview__controls {
    display: grid;
    grid-template-columns: minmax(160px, 260px) minmax(0, 1fr);
    gap: 12px 16px;
    align-items: end;
  }
  .preview__song,
  .preview__scrub {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
  }
  .preview label {
    font-size: 12px;
    color: var(--text-muted);
  }
  .preview__scrubline {
    display: flex;
    align-items: center;
    gap: 12px;
  }
  .preview__scrubber {
    position: relative;
    flex: 1;
    min-width: 0;
  }
  .preview__sections {
    position: relative;
    height: 18px;
    margin: 0 8px 2px;
    border-radius: 3px;
    overflow: hidden;
    background: var(--bg-card);
  }
  .preview__section {
    position: absolute;
    top: 0;
    bottom: 0;
    padding: 0 4px;
    font-size: 11px;
    line-height: 18px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    color: var(--text);
    background: color-mix(in srgb, var(--section) 35%, transparent);
    border-left: 2px solid var(--section);
    box-sizing: border-box;
  }
  .preview__range {
    display: block;
    width: 100%;
    margin: 0;
    accent-color: var(--accent);
  }
  .preview__clock {
    font-variant-numeric: tabular-nums;
    font-size: 13px;
    white-space: nowrap;
  }
  .preview__panels {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 12px;
    align-items: start;
  }
  .preview__panel {
    border: 1px solid var(--border);
    border-radius: var(--radius);
    padding: 10px 12px;
    min-width: 0;
  }
  .preview__heading {
    margin: 0 0 6px;
    font-size: 13px;
    font-weight: 700;
  }
  .preview__activity {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: 13px;
  }
  .preview__quiet {
    margin: 0;
    color: var(--text-muted);
    font-size: 13px;
  }
  .preview__names {
    margin: 6px 0;
    font-size: 13px;
    overflow-wrap: anywhere;
  }
  .preview__error {
    margin: 0;
    padding: 10px 14px;
    border: 1px solid var(--border-danger);
    background: var(--bg-danger);
    border-radius: var(--radius);
    font-size: 13px;
    white-space: pre-wrap;
  }
  .preview__open {
    grid-column: 1 / -1;
    justify-self: start;
  }
  @media (max-width: 640px) {
    .preview__controls,
    .preview__panels {
      grid-template-columns: minmax(0, 1fr);
    }
    .preview__scrubline {
      flex-wrap: wrap;
    }
  }
</style>
