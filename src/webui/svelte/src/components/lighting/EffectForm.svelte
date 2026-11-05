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
  import type { CueEffect, EffectType } from "../../lib/lighting/types";
  import {
    EFFECT_TYPES,
    EASINGS,
    LAYERS,
    BLEND_MODES,
    CURVES,
    CYCLE_DIRECTIONS,
    CHASE_DIRECTIONS,
    CHASE_PATTERNS,
  } from "../../lib/lighting/types";
  import ColorInput from "./ColorInput.svelte";
  import type { EffectEdit, MoveParam } from "../../gen/rules/mtrack_rules";
  import { rules } from "../../lib/rules";
  import { venueStore } from "../../lib/ws/stores";

  /** The effect with a form's edit made, the value written by the
   *  player's writer (`rules.effect_edit`): only the edited text changes. */
  function edit(change: EffectEdit) {
    onchange(rules.effect_edit(effect, change) ?? effect);
  }

  /** A `move` aims (and starts) at a focus point or at angles, never
   *  both: setting one clears the other, so the form always shows what
   *  will be written. */
  function updateMove(key: MoveParam, value?: string) {
    edit({ op: "move", key, value });
  }

  /** The current venue's focus points, offered as `move` targets. */
  let focusPointNames = $derived(
    Object.keys($venueStore?.focus_points ?? {}).sort(),
  );

  interface Props {
    effect: CueEffect;
    groups: string[];
    onchange: (effect: CueEffect) => void;
    ondelete: () => void;
  }

  let { effect, groups = [], onchange, ondelete }: Props = $props();

  /** The parameters by name, the last of each as written, for the
   *  controls (a `to:` reads as `focus`; a focus point without its
   *  quotes), and the colours: `rules.effect_view`. */
  let view = $derived(rules.effect_view(effect.effect));
  let p = $derived<Record<string, string | undefined>>(view?.values ?? {});
  let colors = $derived(view?.colors ?? []);

  let expanded = $state(false);

  let groupText = $derived(effect.groups.join(", "));

  function updateGroups(text: string) {
    const newGroups = text
      .split(",")
      .map((g) => g.trim())
      .filter((g) => g);
    onchange({ ...effect, groups: newGroups });
  }

  /** What each effect type actually reads, mirroring the engine's own arms. */
  const USED_PARAMS: Record<EffectType, string[]> = {
    static: [
      "dimmer",
      "intensity",
      "red",
      "green",
      "blue",
      "white",
      "duration",
    ],
    cycle: ["speed", "direction", "transition", "duration"],
    strobe: ["frequency", "duration"],
    move: [
      "focus",
      "to",
      "from",
      "from_pan",
      "from_tilt",
      "pan",
      "tilt",
      "easing",
      "duration",
    ],
    pulse: [
      "base_level",
      "pulse_amplitude",
      "intensity",
      "frequency",
      "duration",
    ],
    chase: ["pattern", "speed", "direction", "transition", "duration"],
    dimmer: ["start_level", "end_level", "curve", "duration"],
    rainbow: ["speed", "saturation", "brightness", "duration"],
  };

  /** Whether a type reads colours at all. */
  const USES_COLOR: EffectType[] = ["static", "cycle"];

  function updateType(type: EffectType) {
    // Changing the type used to carry every field across. A cycle with speed,
    // direction, transition and a colour became a strobe still carrying all of
    // them — parameters the strobe ignores, written to the file on the next
    // save. Only what the new type reads survives, with the layer, blend
    // and timing parameters every type takes.
    const keep = [
      ...(USED_PARAMS[type] ?? []),
      ...(USES_COLOR.includes(type) ? ["color"] : []),
      "layer",
      "blend_mode",
      "up_time",
      "hold_time",
      "down_time",
    ];
    onchange({
      ...effect,
      effect: {
        ...effect.effect,
        type,
        params: effect.effect.params.filter((entry) =>
          keep.includes(entry.name),
        ),
      },
    });
  }

  function updateParam(key: string, value: unknown) {
    const text =
      value === undefined || value === "" ? undefined : String(value);
    edit({ op: "set", name: key, value: text });
  }

  function updateColors(colors: string[]) {
    edit({ op: "colors", colors });
  }

  // Only the effect types that actually use a colour. `EffectType::Strobe`,
  // `Chase`, `Pulse` and `Dimmer` have no colour field: the parser accepts a
  // `color:` on them and drops it, so offering the control wrote a setting that
  // never did anything — the same defect as the rainbow direction control
  // removed in #398, which is how these were found.
  let showsColor = $derived(
    effect.effect.type === "static" || effect.effect.type === "cycle",
  );

  let isMultiColor = $derived(
    effect.effect.type === "cycle" || effect.effect.type === "chase",
  );

  // Build a compact summary of key params for the collapsed view
  let paramSummary = $derived.by(() => {
    const parts: string[] = [];
    // Only where the effect reads them, or the collapsed row advertises a value
    // that has no control to edit it.
    if (p.dimmer && effect.effect.type === "static") parts.push(p.dimmer);
    if (
      p.intensity !== undefined &&
      (effect.effect.type === "static" || effect.effect.type === "pulse")
    )
      parts.push(p.intensity);
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

  // Deduplicated group options: known venue groups + "all"
  let allGroupOptions = $derived.by(() => {
    const seen: Record<string, boolean> = {};
    for (const g of groups) seen[g] = true;
    seen["all"] = true;
    return Object.keys(seen).sort();
  });
</script>

<div class="effect-form" class:expanded>
  <!-- Compact single-line row -->
  <div class="effect-row">
    <button
      class="expand-toggle"
      onclick={() => (expanded = !expanded)}
      title={expanded ? $t("effect.collapse") : $t("effect.expand")}
    >
      {expanded ? "\u25BC" : "\u25B6"}
    </button>

    <input
      type="text"
      class="inline-input group-input"
      value={groupText}
      onchange={(e) => updateGroups((e.target as HTMLInputElement).value)}
      placeholder={$t("effect.group")}
      list="group-suggestions"
    />
    <datalist id="group-suggestions">
      {#each allGroupOptions as g (g)}
        <option value={g}></option>
      {/each}
    </datalist>

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
    {#if showsColor && colors.length > 0}
      <span class="color-swatches">
        {#each colors.slice(0, 4) as c, ci (ci)}
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
      title={$t("effect.removeEffect")}
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
          <span class="detail-label">{$t("effect.colors")}</span>
          <ColorInput {colors} onchange={updateColors} multi={isMultiColor} />
        </div>
      {/if}

      <div class="param-grid">
        {#if effect.effect.type === "static"}
          <label class="param">
            <span class="param-label">{$t("effect.intensity")}</span>
            <input
              type="number"
              class="param-input"
              min="0"
              max="1"
              step="0.1"
              value={p.intensity ?? ""}
              onchange={(e) => {
                const v = (e.target as HTMLInputElement).value;
                updateParam("intensity", v || undefined);
              }}
            />
          </label>
          <label class="param">
            <span class="param-label">{$t("effect.dimmer")}</span>
            <input
              type="text"
              class="param-input"
              placeholder="100%"
              value={p.dimmer ?? ""}
              onchange={(e) => {
                const v = (e.target as HTMLInputElement).value;
                updateParam("dimmer", v || undefined);
              }}
            />
          </label>
          <label class="param"
            ><span class="param-label">{$t("effect.duration")}</span><input
              type="text"
              class="param-input"
              placeholder="5s"
              value={p.duration ?? ""}
              onchange={(e) =>
                updateParam(
                  "duration",
                  (e.target as HTMLInputElement).value || undefined,
                )}
            /></label
          >
        {:else if effect.effect.type === "cycle"}
          <label class="param"
            ><span class="param-label">{$t("effect.speed")}</span><input
              type="text"
              class="param-input"
              placeholder="1.0"
              value={p.speed ?? ""}
              onchange={(e) =>
                updateParam(
                  "speed",
                  (e.target as HTMLInputElement).value || undefined,
                )}
            /></label
          >
          <label class="param"
            ><span class="param-label">{$t("effect.duration")}</span><input
              type="text"
              class="param-input"
              placeholder="2s"
              value={p.duration ?? ""}
              onchange={(e) =>
                updateParam(
                  "duration",
                  (e.target as HTMLInputElement).value || undefined,
                )}
            /></label
          >
          <label class="param"
            ><span class="param-label">{$t("effect.direction")}</span>
            <select
              class="param-input"
              value={p.direction ?? ""}
              onchange={(e) =>
                updateParam(
                  "direction",
                  (e.target as HTMLSelectElement).value || undefined,
                )}
            >
              <option value="">--</option>
              {#each CYCLE_DIRECTIONS as d (d)}<option value={d}>{d}</option
                >{/each}
            </select>
          </label>
        {:else if effect.effect.type === "strobe"}
          <label class="param"
            ><span class="param-label">{$t("effect.frequency")}</span><input
              type="text"
              class="param-input"
              placeholder="8"
              value={p.frequency ?? ""}
              onchange={(e) =>
                updateParam(
                  "frequency",
                  (e.target as HTMLInputElement).value || undefined,
                )}
            /></label
          >
          <label class="param"
            ><span class="param-label">{$t("effect.duration")}</span><input
              type="text"
              class="param-input"
              placeholder="4s"
              value={p.duration ?? ""}
              onchange={(e) =>
                updateParam(
                  "duration",
                  (e.target as HTMLInputElement).value || undefined,
                )}
            /></label
          >
          <!-- No intensity or duty cycle: `EffectType::Strobe` carries only a
               frequency and a duration, so the parser accepted both and dropped
               them. -->
        {:else if effect.effect.type === "pulse"}
          <label class="param"
            ><span class="param-label">{$t("effect.frequency")}</span><input
              type="text"
              class="param-input"
              placeholder="4"
              value={p.frequency ?? ""}
              onchange={(e) =>
                updateParam(
                  "frequency",
                  (e.target as HTMLInputElement).value || undefined,
                )}
            /></label
          >
          <label class="param"
            ><span class="param-label">{$t("effect.intensity")}</span><input
              type="number"
              class="param-input"
              min="0"
              max="1"
              step="0.1"
              value={p.intensity ?? ""}
              onchange={(e) => {
                const v = (e.target as HTMLInputElement).value;
                updateParam("intensity", v || undefined);
              }}
            /></label
          >
          <label class="param"
            ><span class="param-label">{$t("effect.base")}</span><input
              type="number"
              class="param-input"
              min="0"
              max="1"
              step="0.1"
              value={p.base_level ?? ""}
              onchange={(e) => {
                const v = (e.target as HTMLInputElement).value;
                updateParam("base_level", v || undefined);
              }}
            /></label
          >
          <label class="param"
            ><span class="param-label">{$t("effect.amplitude")}</span><input
              type="number"
              class="param-input"
              min="0"
              max="1"
              step="0.1"
              value={p.pulse_amplitude ?? ""}
              onchange={(e) => {
                const v = (e.target as HTMLInputElement).value;
                updateParam("pulse_amplitude", v || undefined);
              }}
            /></label
          >
          <label class="param"
            ><span class="param-label">{$t("effect.duration")}</span><input
              type="text"
              class="param-input"
              placeholder="500ms"
              value={p.duration ?? ""}
              onchange={(e) =>
                updateParam(
                  "duration",
                  (e.target as HTMLInputElement).value || undefined,
                )}
            /></label
          >
        {:else if effect.effect.type === "chase"}
          <label class="param"
            ><span class="param-label">{$t("effect.speed")}</span><input
              type="text"
              class="param-input"
              placeholder="2.0"
              value={p.speed ?? ""}
              onchange={(e) =>
                updateParam(
                  "speed",
                  (e.target as HTMLInputElement).value || undefined,
                )}
            /></label
          >
          <label class="param"
            ><span class="param-label">{$t("effect.pattern")}</span>
            <select
              class="param-input"
              value={p.pattern ?? ""}
              onchange={(e) =>
                updateParam(
                  "pattern",
                  (e.target as HTMLSelectElement).value || undefined,
                )}
            >
              <option value="">--</option>
              {#each CHASE_PATTERNS as p (p)}<option value={p}>{p}</option
                >{/each}
            </select>
          </label>
          <label class="param"
            ><span class="param-label">{$t("effect.duration")}</span><input
              type="text"
              class="param-input"
              placeholder="1s"
              value={p.duration ?? ""}
              onchange={(e) =>
                updateParam(
                  "duration",
                  (e.target as HTMLInputElement).value || undefined,
                )}
            /></label
          >
          <label class="param"
            ><span class="param-label">{$t("effect.direction")}</span>
            <select
              class="param-input"
              value={p.direction ?? ""}
              onchange={(e) =>
                updateParam(
                  "direction",
                  (e.target as HTMLSelectElement).value || undefined,
                )}
            >
              <option value="">--</option
              >{#each CHASE_DIRECTIONS as d (d)}<option value={d}>{d}</option
                >{/each}
            </select>
          </label>
        {:else if effect.effect.type === "dimmer"}
          <label class="param"
            ><span class="param-label">{$t("effect.start")}</span><input
              type="number"
              class="param-input"
              min="0"
              max="1"
              step="0.1"
              value={p.start_level ?? ""}
              onchange={(e) => {
                const v = (e.target as HTMLInputElement).value;
                updateParam("start_level", v || undefined);
              }}
            /></label
          >
          <label class="param"
            ><span class="param-label">{$t("effect.end")}</span><input
              type="number"
              class="param-input"
              min="0"
              max="1"
              step="0.1"
              value={p.end_level ?? ""}
              onchange={(e) => {
                const v = (e.target as HTMLInputElement).value;
                updateParam("end_level", v || undefined);
              }}
            /></label
          >
          <label class="param"
            ><span class="param-label">{$t("effect.duration")}</span><input
              type="text"
              class="param-input"
              placeholder="0.5s"
              value={p.duration ?? ""}
              onchange={(e) =>
                updateParam(
                  "duration",
                  (e.target as HTMLInputElement).value || undefined,
                )}
            /></label
          >
          <label class="param"
            ><span class="param-label">{$t("effect.curve")}</span>
            <select
              class="param-input"
              value={p.curve ?? ""}
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
        {:else if effect.effect.type === "move"}
          <label class="param"
            ><span class="param-label">{$t("effect.focus")}</span>
            <input
              type="text"
              class="param-input"
              list="focus-points"
              placeholder="drummer"
              value={p.focus ?? ""}
              onchange={(e) =>
                updateMove(
                  "focus",
                  (e.target as HTMLInputElement).value || undefined,
                )}
            />
          </label>
          <datalist id="focus-points">
            {#each focusPointNames as name (name)}
              <option value={name}></option>
            {/each}
          </datalist>
          <label class="param"
            ><span class="param-label">{$t("effect.from")}</span>
            <input
              type="text"
              class="param-input"
              list="focus-points"
              placeholder="center-stage"
              value={p.from ?? ""}
              onchange={(e) =>
                updateMove(
                  "from",
                  (e.target as HTMLInputElement).value || undefined,
                )}
            />
          </label>
          <label class="param"
            ><span class="param-label">{$t("effect.pan")}</span><input
              type="text"
              class="param-input"
              placeholder="45deg"
              value={p.pan ?? ""}
              onchange={(e) =>
                updateMove(
                  "pan",
                  (e.target as HTMLInputElement).value || undefined,
                )}
            /></label
          >
          <label class="param"
            ><span class="param-label">{$t("effect.tilt")}</span><input
              type="text"
              class="param-input"
              placeholder="-20deg"
              value={p.tilt ?? ""}
              onchange={(e) =>
                updateMove(
                  "tilt",
                  (e.target as HTMLInputElement).value || undefined,
                )}
            /></label
          >
          <label class="param"
            ><span class="param-label">{$t("effect.fromPan")}</span><input
              type="text"
              class="param-input"
              placeholder="0deg"
              value={p.from_pan ?? ""}
              onchange={(e) =>
                updateMove(
                  "from_pan",
                  (e.target as HTMLInputElement).value || undefined,
                )}
            /></label
          >
          <label class="param"
            ><span class="param-label">{$t("effect.fromTilt")}</span><input
              type="text"
              class="param-input"
              placeholder="0deg"
              value={p.from_tilt ?? ""}
              onchange={(e) =>
                updateMove(
                  "from_tilt",
                  (e.target as HTMLInputElement).value || undefined,
                )}
            /></label
          >
          <label class="param"
            ><span class="param-label">{$t("effect.easing")}</span>
            <select
              class="param-input"
              value={p.easing ?? ""}
              onchange={(e) =>
                updateParam(
                  "easing",
                  (e.target as HTMLSelectElement).value || undefined,
                )}
            >
              <option value="">--</option>{#each EASINGS as z (z)}<option
                  value={z}>{z}</option
                >{/each}
            </select>
          </label>
          <label class="param"
            ><span class="param-label">{$t("effect.duration")}</span><input
              type="text"
              class="param-input"
              placeholder="2s"
              value={p.duration ?? ""}
              onchange={(e) =>
                updateParam(
                  "duration",
                  (e.target as HTMLInputElement).value || undefined,
                )}
            /></label
          >
          <div class="param-hint">{$t("effect.focusOrAngles")}</div>
        {:else if effect.effect.type === "rainbow"}
          <label class="param"
            ><span class="param-label">{$t("effect.speed")}</span><input
              type="text"
              class="param-input"
              placeholder="2.0"
              value={p.speed ?? ""}
              onchange={(e) =>
                updateParam(
                  "speed",
                  (e.target as HTMLInputElement).value || undefined,
                )}
            /></label
          >
          <label class="param"
            ><span class="param-label">{$t("effect.saturation")}</span><input
              type="number"
              class="param-input"
              min="0"
              max="1"
              step="0.1"
              value={p.saturation ?? ""}
              onchange={(e) => {
                const v = (e.target as HTMLInputElement).value;
                updateParam("saturation", v || undefined);
              }}
            /></label
          >
          <label class="param"
            ><span class="param-label">{$t("effect.brightness")}</span><input
              type="number"
              class="param-input"
              min="0"
              max="1"
              step="0.1"
              value={p.brightness ?? ""}
              onchange={(e) => {
                const v = (e.target as HTMLInputElement).value;
                updateParam("brightness", v || undefined);
              }}
            /></label
          >
          <label class="param"
            ><span class="param-label">{$t("effect.duration")}</span><input
              type="text"
              class="param-input"
              placeholder="3s"
              value={p.duration ?? ""}
              onchange={(e) =>
                updateParam(
                  "duration",
                  (e.target as HTMLInputElement).value || undefined,
                )}
            /></label
          >
        {/if}

        <!-- Layer & blend mode -->
        <label class="param"
          ><span class="param-label">{$t("effect.layer")}</span>
          <select
            class="param-input"
            value={p.layer ?? ""}
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
          ><span class="param-label">{$t("effect.blend")}</span>
          <select
            class="param-input"
            value={p.blend_mode ?? ""}
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
          ><span class="param-label">{$t("effect.up")}</span><input
            type="text"
            class="param-input"
            placeholder="fade in"
            value={p.up_time ?? ""}
            onchange={(e) =>
              updateParam(
                "up_time",
                (e.target as HTMLInputElement).value || undefined,
              )}
          /></label
        >
        <label class="param"
          ><span class="param-label">{$t("effect.hold")}</span><input
            type="text"
            class="param-input"
            placeholder="hold"
            value={p.hold_time ?? ""}
            onchange={(e) =>
              updateParam(
                "hold_time",
                (e.target as HTMLInputElement).value || undefined,
              )}
          /></label
        >
        <label class="param"
          ><span class="param-label">{$t("effect.down")}</span><input
            type="text"
            class="param-input"
            placeholder="fade out"
            value={p.down_time ?? ""}
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
  .param-hint {
    grid-column: 1 / -1;
    font-size: 11px;
    color: var(--text-dim);
  }
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
    font-size: 10px;
    padding: 2px;
    width: 14px;
    flex-shrink: 0;
  }
  .inline-input {
    background: transparent;
    border: 1px solid transparent;
    border-radius: 3px;
    color: var(--text);
    font-size: 13px;
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
    cursor: pointer;
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
    font-size: 12px;
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
    font-size: 12px;
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
    font-size: 11px;
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
    font-size: 11px;
    color: var(--text-muted);
    white-space: nowrap;
  }
  .param-input {
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 3px;
    color: var(--text);
    font-size: 12px;
    padding: 2px 4px;
    width: 64px;
  }
  .param-input:focus {
    border-color: var(--border-focus);
    outline: none;
  }
</style>
