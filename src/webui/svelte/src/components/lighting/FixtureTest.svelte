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
   * "Test this fixture": does this light answer when mtrack sends to it,
   * and if not, why not. Pick the mode, the universe and the address,
   * turn Send on, and the fixture goes to full white; swatches, sliders and
   * raw channels follow. What is sent is resolved by the server exactly as
   * a show would be, held a few seconds and kept alive from here, and
   * released when Send goes off, the place changes, or the page goes.
   */
  import { onDestroy, onMount, untrack } from "svelte";
  import { t } from "svelte-i18n";
  import { get } from "svelte/store";
  import { playbackStore } from "../../lib/ws/stores";
  import { statusStore } from "../../lib/ws/status";
  import { setLocked } from "../../lib/api/config";
  import { showConfirm } from "../../lib/dialog.svelte";
  import {
    fetchTestOptions,
    frameChannels,
    lastAddress,
    releaseTest,
    sendTest,
    startingControls,
    testBody,
    TestRefused,
    Throttle,
    type TestAnswer,
    type TestControls,
    type TestOptions,
    type TestUnavailable,
  } from "../../lib/lighting/fixtureTest";

  interface Props {
    fixtureType: string;
    dir?: string;
    venuesDir?: string;
    /** What is being sent, for the page's 3D view; null when nothing is. */
    onframe?: (
      live: {
        channels: Record<string, number>;
        pose: { pan: number; tilt: number } | null;
      } | null,
    ) => void;
  }

  let { fixtureType, dir, venuesDir, onframe }: Props = $props();

  /** The window event the nav's Stop raises, so a page that is sending
   *  stops too rather than lighting the fixture again on its heartbeat. */
  const STOP_EVENT = "mtrack:fixture-test-stop";
  const HEARTBEAT_MS = 2000;
  const SWATCHES: [string, string][] = [
    ["red", "#ff0000"],
    ["green", "#00ff00"],
    ["blue", "#0000ff"],
    ["white", "#ffffff"],
    ["off", "#000000"],
  ];

  let open = $state(false);
  let options = $state<TestOptions | null>(null);
  let optionsError = $state("");
  let modeName = $state<string | null>(null);
  let universe = $state<number | null>(null);
  let address = $state(1);
  let live = $state(false);
  let controls = $state<TestControls>(startingControls(null));
  let raw = $state<Record<number, number>>({});
  let answer = $state<TestAnswer | null>(null);
  let refused = $state<{ message: string; reason: string | null } | null>(null);

  const mode = $derived(
    options
      ? options.gdtf
        ? (options.modes.find((m) => m.name === modeName && m.drivable) ?? null)
        : (options.modes[0] ?? null)
      : null,
  );
  const footprint = $derived(mode?.footprint ?? 1);
  const last = $derived(lastAddress(address, footprint));
  const chosenUniverse = $derived(
    options?.universes.find((u) => u.universe === universe) ?? null,
  );
  /** Why testing cannot happen now; the player's own state first. */
  const unavailable = $derived.by<TestUnavailable | null>(() => {
    if ($playbackStore.locked) return "locked";
    if ($playbackStore.is_playing) return "playing";
    const why = options?.unavailable ?? null;
    return why === "no_dmx" || why === "no_universes" ? why : null;
  });
  const ready = $derived(
    !!mode && universe !== null && address >= 1 && !unavailable,
  );
  const warnings = $derived(answer?.warnings ?? []);
  const oladDown = $derived(
    options?.olad?.reachable === false ||
      warnings.some((w) => w.kind === "olad_unreachable"),
  );
  const unpatched = $derived(
    chosenUniverse?.patched === false ||
      warnings.some((w) => w.kind === "universe_unpatched"),
  );
  const oladUrl = $derived(
    `http://${typeof window === "undefined" ? "localhost" : window.location.hostname}:${options?.olad?.port ?? 9090}`,
  );
  const sending = $derived(
    answer
      ? [...answer.frame]
          .sort((a, b) => a.offset - b.offset)
          .map((c) => c.value)
          .join(", ")
      : "",
  );
  const frameByOffset = $derived(
    new Map((answer?.frame ?? []).map((c) => [c.offset, c.value])),
  );
  const otherWarnings = $derived(
    warnings.filter(
      (w) => w.kind !== "olad_unreachable" && w.kind !== "universe_unpatched",
    ),
  );
  const profileName = $derived($statusStore?.hardware.profile ?? null);

  // --- Loading the options when the section opens, or the type changes.
  let loadedFor = "";
  async function loadOptions() {
    const key = `${fixtureType}\u0000${dir ?? ""}`;
    loadedFor = key;
    optionsError = "";
    try {
      const got = await fetchTestOptions(fixtureType, dir, venuesDir);
      if (loadedFor !== key) return;
      options = got;
      if (got.gdtf) {
        const keep = got.modes.find((m) => m.name === modeName && m.drivable);
        if (!keep)
          modeName =
            got.modes.find((m) => m.name === got.default_mode && m.drivable)
              ?.name ??
            got.modes.find((m) => m.drivable)?.name ??
            null;
      }
      if (!got.universes.some((u) => u.universe === universe))
        universe = got.universes[0]?.universe ?? null;
    } catch (e: unknown) {
      if (loadedFor !== key) return;
      options = null;
      optionsError = e instanceof Error ? e.message : String(e);
    }
  }
  $effect(() => {
    void fixtureType;
    void dir;
    if (!open) return;
    untrack(() => void loadOptions());
  });

  // A new mode starts again at full white, with no manual channels.
  $effect(() => {
    const m = mode;
    untrack(() => {
      controls = startingControls(m);
      raw = {};
    });
  });

  // --- Sending. Every request goes through one chain, so a release always
  // lands after the send before it, and the old addresses never stay lit.
  let chain: Promise<void> = Promise.resolve();
  function serial(job: () => Promise<void>): Promise<void> {
    chain = chain.then(job).catch(() => {});
    return chain;
  }

  async function sendNow() {
    if (!live || !mode || universe === null) return;
    const body = testBody(fixtureType, mode, universe, address, controls, raw);
    try {
      answer = await sendTest(body, dir, venuesDir);
      refused = null;
    } catch (e: unknown) {
      if (e instanceof TestRefused) {
        refused = { message: e.message, reason: e.reason };
        stopLocally();
        void loadOptions();
      } else {
        refused = {
          message: e instanceof Error ? e.message : String(e),
          reason: null,
        };
      }
    }
  }

  const throttle = new Throttle(() => serial(sendNow), 50);

  /** Something changed: send it (rate-limited) when live. */
  function changed() {
    if (live) throttle.request();
  }

  function stopLocally() {
    live = false;
    throttle.cancel();
    answer = null;
  }

  async function stop() {
    stopLocally();
    await serial(() => releaseTest());
  }

  async function toggleSend() {
    if (live) {
      await stop();
      return;
    }
    if (!ready) return;
    refused = null;
    live = true;
    throttle.request();
  }

  // The place changed while live: release the old addresses first.
  let place = "";
  $effect(() => {
    const now = `${fixtureType}|${modeName}|${universe}|${address}`;
    untrack(() => {
      const before = place;
      place = now;
      if (!before || before === now || !live) return;
      void serial(() => releaseTest());
      throttle.request();
    });
  });

  // Testing stops being possible (locked, a song started): stop here too;
  // the server has already let go.
  $effect(() => {
    if (unavailable && untrack(() => live)) stopLocally();
  });

  // Keep the output alive while Send is on and the page is in view.
  onMount(() => {
    const beat = setInterval(() => {
      if (live && document.visibilityState === "visible") throttle.request();
    }, HEARTBEAT_MS);
    const onStop = () => stopLocally();
    const onLeave = () => {
      if (live) void releaseTest(true);
    };
    window.addEventListener(STOP_EVENT, onStop);
    window.addEventListener("pagehide", onLeave);
    return () => {
      clearInterval(beat);
      window.removeEventListener(STOP_EVENT, onStop);
      window.removeEventListener("pagehide", onLeave);
    };
  });

  onDestroy(() => {
    if (live) {
      throttle.cancel();
      live = false;
      void releaseTest(true);
    }
    onframe?.(null);
  });

  // The page's 3D view shows what is sent.
  $effect(() => {
    const frame = answer?.frame;
    const m = mode;
    const c = controls;
    if (!live || !frame) {
      onframe?.(null);
      return;
    }
    onframe?.({
      channels: frameChannels(frame),
      pose:
        m?.controls.pan || m?.controls.tilt
          ? { pan: c.pan, tilt: c.tilt }
          : null,
    });
  });

  // --- Controls.
  function setColor(hex: string) {
    controls.color = hex;
    changed();
  }
  function setValue(key: "dimmer" | "pan" | "tilt", value: number) {
    controls[key] = value;
    changed();
  }
  function setStrobe(value: number | null) {
    controls.strobe = value;
    changed();
  }
  function setLevel(name: string, value: number) {
    controls.levels[name] = value;
    changed();
  }
  function setRaw(offset: number, value: number) {
    raw[offset] = Math.max(0, Math.min(255, Math.round(value)));
    changed();
  }
  function resetRaw(offset: number) {
    delete raw[offset];
    changed();
  }
  function blackout() {
    const levels: Record<string, number> = {};
    for (const name of Object.keys(controls.levels)) levels[name] = 0;
    controls = {
      ...controls,
      color: "#000000",
      dimmer: 0,
      strobe: null,
      levels,
    };
    raw = {};
    changed();
  }

  async function unlock() {
    const ok = await showConfirm(get(t)("nav.live.unlockPrompt"), {
      danger: true,
    });
    if (!ok) return;
    try {
      await setLocked(false);
    } catch (e) {
      console.error("Failed to unlock:", e);
    }
  }

  const strobeMid = (s: { min_hz: number; max_hz: number }) =>
    Math.round(Math.min(s.max_hz, Math.max(s.min_hz, 5)) * 10) / 10;
</script>

<details
  class="ftest"
  data-testid="fixture-test"
  bind:open
  aria-label={$t("lighting.test.title")}
>
  <summary class="ftest__summary">
    <span class="ftest__title">{$t("lighting.test.title")}</span>
    {#if live}
      <span class="badge ftest__badge" data-testid="ftest-live-badge"
        >{$t("lighting.test.liveBadge")}</span
      >
    {/if}
  </summary>

  <div class="ftest__body">
    <p class="ftest__intro">{$t("lighting.test.intro")}</p>

    {#if optionsError}
      <p class="ftest__error" role="alert">{optionsError}</p>
    {:else if !options}
      <p class="ftest__quiet">{$t("common.loading")}</p>
    {:else}
      {#if unavailable}
        <div
          class="ftest__unavailable"
          role="status"
          data-testid="ftest-unavailable"
          data-reason={unavailable}
        >
          <p>{$t(`lighting.test.unavailable.${unavailable}`)}</p>
          {#if unavailable === "locked"}
            <button class="btn btn-sm" type="button" onclick={unlock}
              >{$t("lighting.test.unlock")}</button
            >
          {:else if unavailable === "no_dmx" || unavailable === "no_universes"}
            <a
              class="btn btn-sm"
              href={profileName
                ? `#/config/${encodeURIComponent(profileName)}/lighting`
                : "#/config"}>{$t("lighting.test.setUpDmx")}</a
            >
          {/if}
        </div>
      {/if}

      <!-- 1. Where the fixture is. -->
      <div class="ftest__setup">
        {#if options.gdtf}
          <div class="field">
            <label for="ftest-mode">{$t("lighting.test.mode")}</label>
            <select
              id="ftest-mode"
              class="input"
              bind:value={modeName}
              data-testid="ftest-mode"
            >
              {#each options.modes as m (m.name)}
                <option value={m.name} disabled={!m.drivable}
                  >{m.name} — {$t("lighting.test.channelCount", {
                    values: { count: m.footprint },
                  })}{m.drivable
                    ? ""
                    : ` (${$t("lighting.test.cannotDrive")})`}</option
                >
              {/each}
            </select>
          </div>
        {/if}
        <div class="field">
          <label for="ftest-universe">{$t("lighting.test.universe")}</label>
          {#if options.universes.length > 0}
            <select
              id="ftest-universe"
              class="input"
              bind:value={universe}
              data-testid="ftest-universe"
            >
              {#each options.universes as u (u.universe)}
                <option value={u.universe}
                  >{u.universe} · {u.name}{u.patched === false
                    ? ` — ${$t("lighting.test.unpatchedShort")}`
                    : ""}</option
                >
              {/each}
            </select>
          {:else}
            <p class="ftest__quiet">{$t("lighting.test.noUniverses")}</p>
          {/if}
          {#if chosenUniverse?.patched === false}
            <span class="field-hint ftest__warn" data-testid="ftest-unpatched"
              >{$t("lighting.test.unpatched")}</span
            >
          {/if}
        </div>
        <div class="field">
          <label for="ftest-address">{$t("lighting.test.address")}</label>
          <input
            id="ftest-address"
            class="input ftest__address"
            type="number"
            min="1"
            max="512"
            value={address}
            data-testid="ftest-address"
            onchange={(e) => {
              const v = Number((e.currentTarget as HTMLInputElement).value);
              if (Number.isFinite(v) && v >= 1) address = Math.round(v);
            }}
          />
          <span class="field-hint" data-testid="ftest-span"
            >{footprint > 1
              ? $t("lighting.test.span", {
                  values: { from: address, to: last },
                })
              : $t("lighting.test.spanOne", { values: { at: address } })}</span
          >
        </div>
      </div>

      <!-- 2. Send. -->
      <div class="ftest__send">
        <label class="ftest__toggle">
          <input
            type="checkbox"
            role="switch"
            checked={live}
            disabled={!live && !ready}
            data-testid="ftest-send"
            onchange={toggleSend}
          />
          <span>{$t("lighting.test.send")}</span>
        </label>
        {#if live}
          <span class="ftest__live" role="status" data-testid="ftest-live"
            >{$t("lighting.test.live", {
              values: { universe: universe ?? "", from: address, to: last },
            })}</span
          >
          <button
            class="btn btn-sm btn-danger"
            type="button"
            data-testid="ftest-stop"
            onclick={stop}>{$t("lighting.test.stop")}</button
          >
        {:else if ready}
          <span class="field-hint">{$t("lighting.test.sendHint")}</span>
        {/if}
      </div>
      {#if refused}
        <p class="ftest__error" role="alert" data-testid="ftest-refused">
          {refused.message}
        </p>
      {/if}

      {#if mode}
        <!-- 3. Is it alive: swatches, then sliders. -->
        <div class="ftest__controls" data-testid="ftest-controls">
          <div
            class="ftest__swatches"
            role="group"
            aria-label={$t("lighting.test.colour")}
          >
            {#each SWATCHES as [key, hex] (key)}
              <button
                class="btn btn-sm ftest__swatch"
                type="button"
                disabled={!mode.controls.color &&
                  key !== "off" &&
                  key !== "white"}
                data-testid="ftest-swatch-{key}"
                style:--swatch={hex}
                onclick={() => {
                  if (key === "off") blackout();
                  else {
                    if (!mode?.controls.color) setValue("dimmer", 1);
                    else setColor(hex);
                  }
                }}>{$t(`lighting.test.swatch.${key}`)}</button
              >
            {/each}
            <button
              class="btn btn-sm"
              type="button"
              data-testid="ftest-blackout"
              onclick={blackout}>{$t("lighting.test.blackout")}</button
            >
          </div>

          <div class="ftest__row" class:ftest__row--off={!mode.controls.color}>
            <label for="ftest-colour">{$t("lighting.test.colour")}</label>
            {#if mode.controls.color}
              <input
                id="ftest-colour"
                type="color"
                value={controls.color}
                data-testid="ftest-colour"
                oninput={(e) =>
                  setColor((e.currentTarget as HTMLInputElement).value)}
              />
            {:else}
              <span class="ftest__na">{$t("lighting.test.notInMode")}</span>
            {/if}
          </div>

          <div class="ftest__row" class:ftest__row--off={!mode.controls.dimmer}>
            <label for="ftest-dimmer">{$t("lighting.test.dimmer")}</label>
            {#if mode.controls.dimmer}
              <input
                id="ftest-dimmer"
                type="range"
                min="0"
                max="100"
                value={Math.round(controls.dimmer * 100)}
                data-testid="ftest-dimmer"
                oninput={(e) =>
                  setValue(
                    "dimmer",
                    Number((e.currentTarget as HTMLInputElement).value) / 100,
                  )}
              />
              <span class="ftest__value"
                >{Math.round(controls.dimmer * 100)}%</span
              >
              {#if mode.controls.dimmer === "folded"}
                <span class="field-hint"
                  >{$t("lighting.test.dimmerFolded")}</span
                >
              {/if}
            {:else}
              <span class="ftest__na">{$t("lighting.test.notInMode")}</span>
            {/if}
          </div>

          <div
            class="ftest__row"
            class:ftest__row--off={!mode.controls.strobe}
            data-testid="ftest-strobe-row"
          >
            <label for="ftest-strobe">{$t("lighting.test.strobe")}</label>
            {#if mode.controls.strobe}
              {@const s = mode.controls.strobe}
              <label class="ftest__inline">
                <input
                  type="checkbox"
                  checked={controls.strobe !== null}
                  data-testid="ftest-strobe-on"
                  onchange={(e) =>
                    setStrobe(
                      (e.currentTarget as HTMLInputElement).checked
                        ? strobeMid(s)
                        : null,
                    )}
                />
                {$t("lighting.test.strobeOn")}
              </label>
              <input
                id="ftest-strobe"
                type="range"
                min={s.min_hz}
                max={s.max_hz}
                step="0.1"
                value={controls.strobe ?? strobeMid(s)}
                disabled={controls.strobe === null}
                data-testid="ftest-strobe"
                oninput={(e) =>
                  setStrobe(
                    Number((e.currentTarget as HTMLInputElement).value),
                  )}
              />
              <span class="ftest__value"
                >{controls.strobe === null
                  ? $t("lighting.test.strobeOff")
                  : $t("lighting.test.hz", {
                      values: { hz: controls.strobe },
                    })}</span
              >
            {:else}
              <span class="ftest__na" data-testid="ftest-strobe-na"
                >{$t("lighting.test.notInMode")}</span
              >
            {/if}
          </div>

          {#each mode.controls.levels as level (level)}
            <div class="ftest__row">
              <label for="ftest-level-{level}">{level}</label>
              <input
                id="ftest-level-{level}"
                type="range"
                min="0"
                max="100"
                value={Math.round((controls.levels[level] ?? 0) * 100)}
                data-testid="ftest-level-{level}"
                oninput={(e) =>
                  setLevel(
                    level,
                    Number((e.currentTarget as HTMLInputElement).value) / 100,
                  )}
              />
              <span class="ftest__value"
                >{Math.round((controls.levels[level] ?? 0) * 100)}%</span
              >
            </div>
          {/each}

          {#each [["pan", mode.controls.pan], ["tilt", mode.controls.tilt]] as const as [axis, range] (axis)}
            <div class="ftest__row" class:ftest__row--off={!range}>
              <label for="ftest-{axis}">{$t(`lighting.test.${axis}`)}</label>
              {#if range}
                <input
                  id="ftest-{axis}"
                  type="range"
                  min={range.min}
                  max={range.max}
                  step="1"
                  value={controls[axis]}
                  data-testid="ftest-{axis}"
                  oninput={(e) =>
                    setValue(
                      axis,
                      Number((e.currentTarget as HTMLInputElement).value),
                    )}
                />
                <span class="ftest__value">{controls[axis]}°</span>
              {:else}
                <span class="ftest__na">{$t("lighting.test.notInMode")}</span>
              {/if}
            </div>
          {/each}
        </div>

        <!-- 4. Nothing happened? -->
        {#if live && answer}
          <section class="ftest__help" data-testid="ftest-help">
            <p class="ftest__sending" data-testid="ftest-sending">
              {$t("lighting.test.sending", {
                values: {
                  values: sending,
                  universe: answer.universe,
                  from: answer.address,
                  to: lastAddress(answer.address, answer.footprint),
                },
              })}
            </p>
            {#each otherWarnings as w (w.kind + (w.control ?? ""))}
              <p class="ftest__note" data-testid="ftest-warning-{w.kind}">
                {w.message}
              </p>
            {/each}
            <h5 class="ftest__help-title">{$t("lighting.test.help.title")}</h5>
            <ol class="ftest__causes">
              {#if oladDown}
                <li data-testid="ftest-cause-olad">
                  {$t("lighting.test.help.olad")}
                </li>
              {/if}
              {#if unpatched}
                <li data-testid="ftest-cause-unpatched">
                  {$t("lighting.test.help.unpatched", {
                    values: { universe: universe ?? "" },
                  })}
                  <a href={oladUrl} target="_blank" rel="noopener"
                    >{$t("lighting.test.help.openOlad")}</a
                  >
                </li>
              {/if}
              <li data-testid="ftest-cause-address">
                {$t("lighting.test.help.address", {
                  values: { address },
                })}
              </li>
              <li data-testid="ftest-cause-mode">
                {mode.name
                  ? $t("lighting.test.help.mode", {
                      values: { mode: mode.name, count: mode.footprint },
                    })
                  : $t("lighting.test.help.channels", {
                      values: { count: mode.footprint },
                    })}
              </li>
              <li>{$t("lighting.test.help.cable")}</li>
            </ol>
          </section>
        {/if}

        <!-- 5. Every channel, by hand. -->
        <details class="ftest__raw" data-testid="ftest-raw">
          <summary>{$t("lighting.test.raw.title")}</summary>
          <p class="field-hint">{$t("lighting.test.raw.hint")}</p>
          <table class="ftest__table">
            <thead>
              <tr>
                <th>{$t("lighting.test.raw.address")}</th>
                <th>{$t("lighting.test.raw.channel")}</th>
                <th>{$t("lighting.test.raw.value")}</th>
                <th></th>
              </tr>
            </thead>
            <tbody>
              {#each mode.channels as ch (ch.offset)}
                {@const manual = ch.offset in raw}
                {@const value = manual
                  ? raw[ch.offset]
                  : (frameByOffset.get(ch.offset) ?? 0)}
                <tr
                  data-testid="ftest-raw-row"
                  data-offset={ch.offset}
                  class:ftest__manual={manual}
                >
                  <td class="ftest__mono">{address + ch.offset - 1}</td>
                  <td>{ch.name}</td>
                  <td class="ftest__rawcell">
                    <input
                      type="range"
                      min="0"
                      max="255"
                      {value}
                      aria-label={ch.name}
                      data-testid="ftest-raw-slider"
                      oninput={(e) =>
                        setRaw(
                          ch.offset,
                          Number((e.currentTarget as HTMLInputElement).value),
                        )}
                    />
                    <input
                      class="input ftest__rawnum"
                      type="number"
                      min="0"
                      max="255"
                      {value}
                      aria-label={ch.name}
                      data-testid="ftest-raw-number"
                      onchange={(e) =>
                        setRaw(
                          ch.offset,
                          Number((e.currentTarget as HTMLInputElement).value),
                        )}
                    />
                  </td>
                  <td>
                    {#if manual}
                      <span class="badge" data-testid="ftest-raw-manual"
                        >{$t("lighting.test.raw.manual")}</span
                      >
                      <button
                        class="btn btn-sm"
                        type="button"
                        data-testid="ftest-raw-reset"
                        onclick={() => resetRaw(ch.offset)}
                        >{$t("lighting.test.raw.reset")}</button
                      >
                    {/if}
                  </td>
                </tr>
              {/each}
            </tbody>
          </table>
        </details>
      {/if}
    {/if}
  </div>
</details>

<style>
  .ftest {
    border: 1px solid var(--border);
    border-radius: var(--radius, 8px);
    background: var(--bg-card);
    min-width: 0;
  }
  .ftest__summary {
    cursor: pointer;
    padding: 12px 16px;
    display: flex;
    align-items: center;
    gap: 10px;
    flex-wrap: wrap;
  }
  .ftest__title {
    font-weight: 600;
  }
  .ftest__badge {
    background: rgba(232, 75, 75, 0.15);
    border-color: rgba(232, 75, 75, 0.5);
    color: var(--nc-error, #c33);
  }
  .ftest__body {
    padding: 0 16px 16px;
    display: flex;
    flex-direction: column;
    gap: 14px;
    min-width: 0;
  }
  .ftest__intro,
  .ftest__quiet {
    margin: 0;
    color: var(--text-muted, var(--text));
    font-size: 13px;
  }
  .ftest__error {
    margin: 0;
    color: var(--text-danger, #c33);
    font-size: 13px;
  }
  .ftest__unavailable {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px 12px;
    padding: 10px 12px;
    border-radius: var(--radius, 8px);
    border: 1px solid var(--border);
    background: var(--inset-bg, transparent);
    font-size: 13px;
  }
  .ftest__unavailable p {
    margin: 0;
    flex: 1 1 240px;
  }
  .ftest__setup {
    display: flex;
    flex-wrap: wrap;
    gap: 12px 16px;
  }
  .ftest__setup .field {
    flex: 1 1 160px;
    min-width: 0;
  }
  .ftest__address {
    max-width: 120px;
  }
  .ftest__warn {
    color: var(--nc-warning, #b7791f);
  }
  .ftest__send {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px 12px;
  }
  .ftest__toggle {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    font-weight: 600;
  }
  .ftest__live {
    color: var(--nc-error, #c33);
    font-weight: 600;
    font-size: 13px;
  }
  .ftest__controls {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .ftest__swatches {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .ftest__swatch {
    box-shadow:
      inset 12px 0 0 var(--swatch),
      inset 13px 0 0 var(--border);
    padding-left: 20px;
  }
  .ftest__row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px 12px;
    font-size: 13px;
  }
  .ftest__row > label:first-child {
    flex: 0 0 90px;
    font-weight: 500;
    text-transform: capitalize;
  }
  .ftest__row input[type="range"] {
    flex: 1 1 140px;
    min-width: 0;
  }
  .ftest__row--off {
    opacity: 0.6;
  }
  .ftest__inline {
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }
  .ftest__value {
    min-width: 52px;
    font-variant-numeric: tabular-nums;
  }
  .ftest__na {
    font-style: italic;
    color: var(--text-muted, var(--text));
  }
  .ftest__help {
    border-left: 3px solid var(--accent);
    padding: 4px 0 4px 12px;
    font-size: 13px;
  }
  .ftest__sending {
    margin: 0 0 8px;
    font-family: var(--nc-font-mono, monospace);
    overflow-wrap: anywhere;
  }
  .ftest__note {
    margin: 0 0 6px;
    color: var(--nc-warning, #b7791f);
  }
  .ftest__help-title {
    margin: 8px 0 4px;
    font-size: 13px;
  }
  .ftest__causes {
    margin: 0;
    padding-left: 20px;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .ftest__causes a {
    color: var(--accent);
    font-weight: 600;
  }
  .ftest__raw summary {
    cursor: pointer;
    font-weight: 500;
  }
  .ftest__table {
    width: 100%;
    border-collapse: collapse;
    font-size: 13px;
    table-layout: fixed;
  }
  .ftest__table th,
  .ftest__table td {
    text-align: left;
    padding: 4px 6px;
    border-bottom: 1px solid var(--border);
    overflow-wrap: anywhere;
  }
  .ftest__table th:nth-child(1) {
    width: 76px;
  }
  .ftest__table th:nth-child(2) {
    width: 24%;
  }
  .ftest__table th:nth-child(4) {
    width: 96px;
  }
  .ftest__rawcell {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
  }
  .ftest__rawcell input[type="range"] {
    flex: 1 1 80px;
    min-width: 0;
  }
  .ftest__rawnum {
    width: 80px;
    flex: 0 0 auto;
  }
  .ftest__mono {
    font-family: var(--nc-font-mono, monospace);
  }
  .ftest__manual td {
    background: var(--accent-subtle, transparent);
  }
  @media (max-width: 600px) {
    .ftest__table th:nth-child(4) {
      width: 72px;
    }
    .ftest__row > label:first-child {
      flex-basis: 100%;
    }
  }
</style>
