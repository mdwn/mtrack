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
  import type { CueEffect, EffectType } from "../../lib/lighting/types";
  import {
    EFFECT_TYPES,
    LAYERS,
    BLEND_MODES,
    CURVES,
    DIRECTIONS,
  } from "../../lib/lighting/types";
  import ColorInput from "./ColorInput.svelte";

  interface Props {
    effect: CueEffect;
    groups: string[];
    onchange: (effect: CueEffect) => void;
    ondelete: () => void;
  }

  let { effect, groups = [], onchange, ondelete }: Props = $props();

  let expanded = $state(false);

  let groupText = $derived(effect.groups.join(", "));

  function updateGroups(text: string) {
    const newGroups = text
      .split(",")
      .map((g) => g.trim())
      .filter((g) => g);
    onchange({ ...effect, groups: newGroups });
  }

  function updateType(type: EffectType) {
    onchange({
      ...effect,
      effect: {
        ...effect.effect,
        type,
        colors: effect.effect.colors,
        extra: effect.effect.extra,
      },
    });
  }

  function updateParam(key: string, value: unknown) {
    onchange({
      ...effect,
      effect: { ...effect.effect, [key]: value },
    });
  }

  function updateColors(colors: string[]) {
    onchange({
      ...effect,
      effect: { ...effect.effect, colors },
    });
  }

  let showsColor = $derived(
    effect.effect.type === "static" ||
      effect.effect.type === "cycle" ||
      effect.effect.type === "chase" ||
      effect.effect.type === "strobe" ||
      effect.effect.type === "pulse",
  );

  let isMultiColor = $derived(
    effect.effect.type === "cycle" || effect.effect.type === "chase",
  );

  // Build a compact summary of key params for the collapsed view
  let paramSummary = $derived.by(() => {
    const p = effect.effect;
    const parts: string[] = [];
    if (p.colors.length > 0) {
      // Shown as swatches, skip text
    }
    if (p.dimmer) parts.push(p.dimmer);
    if (p.intensity !== undefined)
      parts.push(`${Math.round(p.intensity * 100)}%`);
    if (p.speed) parts.push(`spd:${p.speed}`);
    if (p.frequency) parts.push(`freq:${p.frequency}`);
    if (p.duration) parts.push(p.duration);
    if (p.direction) parts.push(p.direction);
    if (p.curve) parts.push(p.curve);
    if (p.pattern) parts.push(p.pattern);
    if (p.start_level !== undefined || p.end_level !== undefined)
      parts.push(`${p.start_level ?? "?"}→${p.end_level ?? "?"}`);
    if (p.loop) parts.push(p.loop);
    if (p.layer) parts.push(p.layer);
    return parts.join("  ");
  });

  // Filtered group suggestions
  let filteredGroups = $derived(
    groups.filter(
      (g) =>
        !effect.groups.includes(g) &&
        g
          .toLowerCase()
          .includes((groupText.split(",").pop() ?? "").trim().toLowerCase()),
    ),
  );
</script>

<div class="effect-form" class:expanded>
  <!-- Compact single-line row -->
  <div class="effect-row">
    <button
      class="expand-toggle"
      onclick={() => (expanded = !expanded)}
      title={expanded ? "Collapse" : "Expand"}
    >
      {expanded ? "\u25BC" : "\u25B6"}
    </button>

    <input
      type="text"
      class="inline-input group-input"
      value={groupText}
      onchange={(e) => updateGroups((e.target as HTMLInputElement).value)}
      placeholder="groups"
      list="group-suggestions-{effect.effect.type}"
    />
    {#if filteredGroups.length > 0}
      <datalist id="group-suggestions-{effect.effect.type}">
        {#each filteredGroups as g (g)}
          <option value={g}></option>
        {/each}
      </datalist>
    {/if}

    <select
      class="inline-input type-select"
      value={effect.effect.type}
      onchange={(e) =>
        updateType((e.target as HTMLSelectElement).value as EffectType)}
    >
      {#each EFFECT_TYPES as t (t)}
        <option value={t}>{t}</option>
      {/each}
    </select>

    <!-- Color swatches inline -->
    {#if showsColor && effect.effect.colors.length > 0}
      <span class="color-swatches">
        {#each effect.effect.colors.slice(0, 4) as c, ci (ci)}
          <span
            class="color-swatch"
            style:background={c.startsWith("#") || c.startsWith("rgb")
              ? c
              : "var(--text-dim)"}
            title={c}
          ></span>
        {/each}
      </span>
    {/if}

    <span class="param-summary">{paramSummary}</span>

    <button
      class="btn-icon delete-btn"
      title="Remove effect"
      onclick={ondelete}
    >
      &#10005;
    </button>
  </div>

  <!-- Expanded detail section -->
  {#if expanded}
    <div class="effect-detail">
      {#if showsColor}
        <div class="detail-row">
          <span class="detail-label">Color{isMultiColor ? "s" : ""}</span>
          <ColorInput
            colors={effect.effect.colors}
            onchange={updateColors}
            multi={isMultiColor}
          />
        </div>
      {/if}

      <div class="param-grid">
        {#if effect.effect.type === "static"}
          <label class="param">
            <span class="param-label">Intensity</span>
            <input
              type="number"
              class="param-input"
              min="0"
              max="1"
              step="0.1"
              value={effect.effect.intensity ?? ""}
              onchange={(e) => {
                const v = (e.target as HTMLInputElement).value;
                updateParam("intensity", v ? parseFloat(v) : undefined);
              }}
            />
          </label>
          <label class="param">
            <span class="param-label">Dimmer</span>
            <input
              type="text"
              class="param-input"
              placeholder="100%"
              value={effect.effect.dimmer ?? ""}
              onchange={(e) => {
                const v = (e.target as HTMLInputElement).value;
                updateParam("dimmer", v || undefined);
              }}
            />
          </label>
        {:else if effect.effect.type === "cycle"}
          <label class="param"
            ><span class="param-label">Speed</span><input
              type="text"
              class="param-input"
              placeholder="1.0"
              value={effect.effect.speed ?? ""}
              onchange={(e) =>
                updateParam(
                  "speed",
                  (e.target as HTMLInputElement).value || undefined,
                )}
            /></label
          >
          <label class="param"
            ><span class="param-label">Duration</span><input
              type="text"
              class="param-input"
              placeholder="2s"
              value={effect.effect.duration ?? ""}
              onchange={(e) =>
                updateParam(
                  "duration",
                  (e.target as HTMLInputElement).value || undefined,
                )}
            /></label
          >
          <label class="param"
            ><span class="param-label">Dimmer</span><input
              type="text"
              class="param-input"
              placeholder="100%"
              value={effect.effect.dimmer ?? ""}
              onchange={(e) =>
                updateParam(
                  "dimmer",
                  (e.target as HTMLInputElement).value || undefined,
                )}
            /></label
          >
          <label class="param"
            ><span class="param-label">Direction</span>
            <select
              class="param-input"
              value={effect.effect.direction ?? ""}
              onchange={(e) =>
                updateParam(
                  "direction",
                  (e.target as HTMLSelectElement).value || undefined,
                )}
            >
              <option value="">--</option>
              {#each DIRECTIONS as d (d)}<option value={d}>{d}</option>{/each}
            </select>
          </label>
          <label class="param"
            ><span class="param-label">Loop</span>
            <select
              class="param-input"
              value={effect.effect.loop ?? ""}
              onchange={(e) =>
                updateParam(
                  "loop",
                  (e.target as HTMLSelectElement).value || undefined,
                )}
            >
              <option value="">--</option><option value="once">once</option
              ><option value="loop">loop</option><option value="pingpong"
                >pingpong</option
              ><option value="random">random</option>
            </select>
          </label>
        {:else if effect.effect.type === "strobe"}
          <label class="param"
            ><span class="param-label">Frequency</span><input
              type="text"
              class="param-input"
              placeholder="8"
              value={effect.effect.frequency ?? ""}
              onchange={(e) =>
                updateParam(
                  "frequency",
                  (e.target as HTMLInputElement).value || undefined,
                )}
            /></label
          >
          <label class="param"
            ><span class="param-label">Intensity</span><input
              type="number"
              class="param-input"
              min="0"
              max="1"
              step="0.1"
              value={effect.effect.intensity ?? ""}
              onchange={(e) => {
                const v = (e.target as HTMLInputElement).value;
                updateParam("intensity", v ? parseFloat(v) : undefined);
              }}
            /></label
          >
          <label class="param"
            ><span class="param-label">Duration</span><input
              type="text"
              class="param-input"
              placeholder="4s"
              value={effect.effect.duration ?? ""}
              onchange={(e) =>
                updateParam(
                  "duration",
                  (e.target as HTMLInputElement).value || undefined,
                )}
            /></label
          >
          <label class="param"
            ><span class="param-label">Duty</span><input
              type="text"
              class="param-input"
              placeholder="50%"
              value={effect.effect.duty_cycle ?? ""}
              onchange={(e) =>
                updateParam(
                  "duty_cycle",
                  (e.target as HTMLInputElement).value || undefined,
                )}
            /></label
          >
        {:else if effect.effect.type === "pulse"}
          <label class="param"
            ><span class="param-label">Frequency</span><input
              type="text"
              class="param-input"
              placeholder="4"
              value={effect.effect.frequency ?? ""}
              onchange={(e) =>
                updateParam(
                  "frequency",
                  (e.target as HTMLInputElement).value || undefined,
                )}
            /></label
          >
          <label class="param"
            ><span class="param-label">Intensity</span><input
              type="number"
              class="param-input"
              min="0"
              max="1"
              step="0.1"
              value={effect.effect.intensity ?? ""}
              onchange={(e) => {
                const v = (e.target as HTMLInputElement).value;
                updateParam("intensity", v ? parseFloat(v) : undefined);
              }}
            /></label
          >
          <label class="param"
            ><span class="param-label">Base</span><input
              type="number"
              class="param-input"
              min="0"
              max="1"
              step="0.1"
              value={effect.effect.base_level ?? ""}
              onchange={(e) => {
                const v = (e.target as HTMLInputElement).value;
                updateParam("base_level", v ? parseFloat(v) : undefined);
              }}
            /></label
          >
          <label class="param"
            ><span class="param-label">Amplitude</span><input
              type="number"
              class="param-input"
              min="0"
              max="1"
              step="0.1"
              value={effect.effect.pulse_amplitude ?? ""}
              onchange={(e) => {
                const v = (e.target as HTMLInputElement).value;
                updateParam("pulse_amplitude", v ? parseFloat(v) : undefined);
              }}
            /></label
          >
          <label class="param"
            ><span class="param-label">Duration</span><input
              type="text"
              class="param-input"
              placeholder="500ms"
              value={effect.effect.duration ?? ""}
              onchange={(e) =>
                updateParam(
                  "duration",
                  (e.target as HTMLInputElement).value || undefined,
                )}
            /></label
          >
          <label class="param"
            ><span class="param-label">Dimmer</span><input
              type="text"
              class="param-input"
              placeholder="80%"
              value={effect.effect.dimmer ?? ""}
              onchange={(e) =>
                updateParam(
                  "dimmer",
                  (e.target as HTMLInputElement).value || undefined,
                )}
            /></label
          >
        {:else if effect.effect.type === "chase"}
          <label class="param"
            ><span class="param-label">Speed</span><input
              type="text"
              class="param-input"
              placeholder="2.0"
              value={effect.effect.speed ?? ""}
              onchange={(e) =>
                updateParam(
                  "speed",
                  (e.target as HTMLInputElement).value || undefined,
                )}
            /></label
          >
          <label class="param"
            ><span class="param-label">Pattern</span><input
              type="text"
              class="param-input"
              placeholder="linear"
              value={effect.effect.pattern ?? ""}
              onchange={(e) =>
                updateParam(
                  "pattern",
                  (e.target as HTMLInputElement).value || undefined,
                )}
            /></label
          >
          <label class="param"
            ><span class="param-label">Duration</span><input
              type="text"
              class="param-input"
              placeholder="1s"
              value={effect.effect.duration ?? ""}
              onchange={(e) =>
                updateParam(
                  "duration",
                  (e.target as HTMLInputElement).value || undefined,
                )}
            /></label
          >
          <label class="param"
            ><span class="param-label">Direction</span>
            <select
              class="param-input"
              value={effect.effect.direction ?? ""}
              onchange={(e) =>
                updateParam(
                  "direction",
                  (e.target as HTMLSelectElement).value || undefined,
                )}
            >
              <option value="">--</option>{#each DIRECTIONS as d (d)}<option
                  value={d}>{d}</option
                >{/each}
            </select>
          </label>
          <label class="param"
            ><span class="param-label">Loop</span>
            <select
              class="param-input"
              value={effect.effect.loop ?? ""}
              onchange={(e) =>
                updateParam(
                  "loop",
                  (e.target as HTMLSelectElement).value || undefined,
                )}
            >
              <option value="">--</option><option value="once">once</option
              ><option value="loop">loop</option><option value="pingpong"
                >pingpong</option
              ><option value="random">random</option>
            </select>
          </label>
        {:else if effect.effect.type === "dimmer"}
          <label class="param"
            ><span class="param-label">Start</span><input
              type="number"
              class="param-input"
              min="0"
              max="1"
              step="0.1"
              value={effect.effect.start_level ?? ""}
              onchange={(e) => {
                const v = (e.target as HTMLInputElement).value;
                updateParam("start_level", v ? parseFloat(v) : undefined);
              }}
            /></label
          >
          <label class="param"
            ><span class="param-label">End</span><input
              type="number"
              class="param-input"
              min="0"
              max="1"
              step="0.1"
              value={effect.effect.end_level ?? ""}
              onchange={(e) => {
                const v = (e.target as HTMLInputElement).value;
                updateParam("end_level", v ? parseFloat(v) : undefined);
              }}
            /></label
          >
          <label class="param"
            ><span class="param-label">Duration</span><input
              type="text"
              class="param-input"
              placeholder="0.5s"
              value={effect.effect.duration ?? ""}
              onchange={(e) =>
                updateParam(
                  "duration",
                  (e.target as HTMLInputElement).value || undefined,
                )}
            /></label
          >
          <label class="param"
            ><span class="param-label">Curve</span>
            <select
              class="param-input"
              value={effect.effect.curve ?? ""}
              onchange={(e) =>
                updateParam(
                  "curve",
                  (e.target as HTMLSelectElement).value || undefined,
                )}
            >
              <option value="">--</option>{#each CURVES as c (c)}<option
                  value={c}>{c}</option
                >{/each}
            </select>
          </label>
        {:else if effect.effect.type === "rainbow"}
          <label class="param"
            ><span class="param-label">Speed</span><input
              type="text"
              class="param-input"
              placeholder="2.0"
              value={effect.effect.speed ?? ""}
              onchange={(e) =>
                updateParam(
                  "speed",
                  (e.target as HTMLInputElement).value || undefined,
                )}
            /></label
          >
          <label class="param"
            ><span class="param-label">Saturation</span><input
              type="number"
              class="param-input"
              min="0"
              max="1"
              step="0.1"
              value={effect.effect.saturation ?? ""}
              onchange={(e) => {
                const v = (e.target as HTMLInputElement).value;
                updateParam("saturation", v ? parseFloat(v) : undefined);
              }}
            /></label
          >
          <label class="param"
            ><span class="param-label">Brightness</span><input
              type="number"
              class="param-input"
              min="0"
              max="1"
              step="0.1"
              value={effect.effect.brightness ?? ""}
              onchange={(e) => {
                const v = (e.target as HTMLInputElement).value;
                updateParam("brightness", v ? parseFloat(v) : undefined);
              }}
            /></label
          >
          <label class="param"
            ><span class="param-label">Direction</span>
            <select
              class="param-input"
              value={effect.effect.direction ?? ""}
              onchange={(e) =>
                updateParam(
                  "direction",
                  (e.target as HTMLSelectElement).value || undefined,
                )}
            >
              <option value="">--</option>{#each DIRECTIONS as d (d)}<option
                  value={d}>{d}</option
                >{/each}
            </select>
          </label>
          <label class="param"
            ><span class="param-label">Duration</span><input
              type="text"
              class="param-input"
              placeholder="3s"
              value={effect.effect.duration ?? ""}
              onchange={(e) =>
                updateParam(
                  "duration",
                  (e.target as HTMLInputElement).value || undefined,
                )}
            /></label
          >
          <label class="param"
            ><span class="param-label">Loop</span>
            <select
              class="param-input"
              value={effect.effect.loop ?? ""}
              onchange={(e) =>
                updateParam(
                  "loop",
                  (e.target as HTMLSelectElement).value || undefined,
                )}
            >
              <option value="">--</option><option value="once">once</option
              ><option value="loop">loop</option>
            </select>
          </label>
        {/if}

        <!-- Layer & blend mode -->
        <label class="param"
          ><span class="param-label">Layer</span>
          <select
            class="param-input"
            value={effect.effect.layer ?? ""}
            onchange={(e) =>
              updateParam(
                "layer",
                (e.target as HTMLSelectElement).value || undefined,
              )}
          >
            <option value="">--</option>{#each LAYERS as l (l)}<option value={l}
                >{l}</option
              >{/each}
          </select>
        </label>
        <label class="param"
          ><span class="param-label">Blend</span>
          <select
            class="param-input"
            value={effect.effect.blend_mode ?? ""}
            onchange={(e) =>
              updateParam(
                "blend_mode",
                (e.target as HTMLSelectElement).value || undefined,
              )}
          >
            <option value="">--</option>{#each BLEND_MODES as b (b)}<option
                value={b}>{b}</option
              >{/each}
          </select>
        </label>

        <!-- Timing -->
        <label class="param"
          ><span class="param-label">Up</span><input
            type="text"
            class="param-input"
            placeholder="fade in"
            value={effect.effect.up_time ?? ""}
            onchange={(e) =>
              updateParam(
                "up_time",
                (e.target as HTMLInputElement).value || undefined,
              )}
          /></label
        >
        <label class="param"
          ><span class="param-label">Hold</span><input
            type="text"
            class="param-input"
            placeholder="hold"
            value={effect.effect.hold_time ?? ""}
            onchange={(e) =>
              updateParam(
                "hold_time",
                (e.target as HTMLInputElement).value || undefined,
              )}
          /></label
        >
        <label class="param"
          ><span class="param-label">Down</span><input
            type="text"
            class="param-input"
            placeholder="fade out"
            value={effect.effect.down_time ?? ""}
            onchange={(e) =>
              updateParam(
                "down_time",
                (e.target as HTMLInputElement).value || undefined,
              )}
          /></label
        >
      </div>
    </div>
  {/if}
</div>

<style>
  .effect-form {
    background: var(--bg-input);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    overflow: hidden;
  }
  .effect-row {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 8px;
    min-height: 28px;
  }
  .expand-toggle {
    background: none;
    border: none;
    color: var(--text-dim);
    cursor: pointer;
    font-size: 8px;
    padding: 2px;
    width: 14px;
    flex-shrink: 0;
  }
  .inline-input {
    background: transparent;
    border: 1px solid transparent;
    border-radius: 3px;
    color: var(--text);
    font-size: 12px;
    padding: 2px 4px;
  }
  .inline-input:hover,
  .inline-input:focus {
    border-color: var(--border);
    background: var(--bg-card);
    outline: none;
  }
  .group-input {
    width: 120px;
    min-width: 60px;
    flex-shrink: 1;
  }
  .type-select {
    width: 72px;
    flex-shrink: 0;
    cursor: pointer;
  }
  .color-swatches {
    display: flex;
    gap: 2px;
    flex-shrink: 0;
  }
  .color-swatch {
    width: 12px;
    height: 12px;
    border-radius: 2px;
    border: 1px solid rgba(255, 255, 255, 0.15);
  }
  .param-summary {
    font-size: 11px;
    color: var(--text-dim);
    font-family: var(--mono);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    flex: 1;
    min-width: 0;
  }
  .delete-btn {
    background: none;
    border: none;
    color: var(--text-dim);
    cursor: pointer;
    font-size: 11px;
    padding: 2px 4px;
    border-radius: 3px;
    flex-shrink: 0;
    opacity: 0.5;
  }
  .effect-form:hover .delete-btn {
    opacity: 1;
  }
  .delete-btn:hover {
    background: rgba(239, 68, 68, 0.15);
    color: var(--red);
  }

  /* Expanded detail */
  .effect-detail {
    border-top: 1px solid var(--border);
    padding: 8px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .detail-row {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .detail-label {
    font-size: 10px;
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: 0.5px;
    width: 50px;
    flex-shrink: 0;
  }
  .param-grid {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .param {
    display: flex;
    align-items: center;
    gap: 4px;
  }
  .param-label {
    font-size: 10px;
    color: var(--text-muted);
    white-space: nowrap;
  }
  .param-input {
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 3px;
    color: var(--text);
    font-size: 11px;
    padding: 2px 4px;
    width: 64px;
  }
  .param-input:focus {
    border-color: var(--border-focus);
    outline: none;
  }
</style>
