# Song palettes and configurable layers

*Design doc, draft 1 — 2026-11-13. Mike asked for two lighting features:
per-song palettes, so a song keeps its look while its colours change, and a
player-configured number of lighting layers, with today's three as the
default. This doc says how each works, what it touches, the order to build
it in, and what is his to decide (§6).*

## 1. Where things stand

| | Today | Missing |
|---|---|---|
| Colours | Written into each effect as a literal: `#ff0000`, `rgb(…)` or one of 11 names (`effects/color.rs`). A sequence can take a colour as a `$name` argument | A name for a colour that the show uses and something outside the show can change |
| Layers | `EffectLayer` is a fixed enum of `Background`, `Midground`, `Foreground` (`effects/types.rs:357`) | Any other number of layers |

Places that assume exactly three layers:

- The grammar: `layer_parameter = { "background" | "midground" | "foreground" }`.
- The multiplier channels: `_dimmer_mult_{bg,mid,fg}`, and the same for
  `_pulse_` and `_chase_` (`effects/fixture.rs:26`), multiplied together
  by hand in `effects/state.rs:252`.
- The foreground rule (`effects/state.rs:262`): a foreground `replace`
  effect is dimmed only by foreground multipliers, so a strobe on top
  isn't dimmed by a background dimmer.
- The web UI timeline draws one effect lane per layer, labelled by
  initials (`ShowGroup.svelte:81`), with the i18n keys `BG`/`MG`/`FG`.
- The MCP DSL reference and the docs book.

Layer clocks, masters, freeze and clear are already keyed by layer
(`engine/layers.rs`), so they don't change.

## 2. Palettes

### 2.1 What a palette is

A palette is a set of named colour **slots**. A show says which slots it
uses and gives each a default colour; its effects refer to a slot instead
of a colour. What colour a slot ends up as is decided **before playback
starts**, in this order (the later one wins):

1. **The show's default** — in the `.light` file. This is how the song
   looks, and it's what the editor previews.
2. **The song's override** — in the song YAML. This is mostly for
   testing: recolour a song without editing its show, and check that
   overrides work without building a playlist.
3. **The playlist entry's override** — in the playlist file, for this
   song in this playlist only. This is the main use: a set full of red
   songs gets some of them recoloured in the playlist.

Nothing changes while a song is playing. There's no live palette switching
and no gRPC/OSC command to recolour a song as it plays. A palette edit
applies the next time the song starts.

### 2.2 In the show

```
palette {
  primary: #c01020
  secondary: "orange"
  accent: rgb(255, 240, 200)
}

show "Main" {
  @0.000
  front_wash: static color: palette.primary, duration: 8measures
  back_truss: cycle colors: [palette.primary, palette.secondary], …
  @16/1
  sequence hit(color: palette.accent)
}
```

- `palette { … }` is a top-level block, beside `tempo` and `sequence`.
  Each slot takes any colour the grammar already accepts. There's at most
  one block per file.
- `palette.<slot>` works anywhere a colour works: `color:`, a colour list,
  a step's `color:`, and as a sequence argument (bound like any other
  argument, so a sequence body still says `$color`).
- `@` is already taken by times and DMX addresses, and `$` by sequence
  arguments, so the reference is spelled out as `palette.`.
- Referring to a slot the file doesn't declare is a parse error, with the
  slot name and position, like an unknown sequence argument. Every slot
  must have a default, so a show always plays on its own.
- A song with several `.light` files: each file declares its own slots.
  An override names a slot, not a file, so it recolours that slot in
  every show of the song that declares it.

### 2.3 Overrides

Song YAML:

```yaml
lighting:
  - file: main.light
palette:
  primary: "#2040ff"
```

Playlist entry (beside `fade_out`, `auto_play`, `ambient`):

```yaml
songs:
  - song: Red Song
    palette:
      primary: "#2040ff"
      secondary: cyan
```

- Both are a map of slot to colour string, parsed with the DSL's colour
  parser, so `"#2040ff"`, `"rgb(…)"` and the names work the same in both.
  A bad colour is a config error at load.
- An override for a slot that none of the song's shows declare is a
  **warning**, not an error, so changing a show doesn't stop a playlist
  from loading. The web UI flags these (§2.5).
- Overrides replace one slot at a time. A slot that isn't named keeps its
  default.

### 2.4 In the player

The parser keeps a slot reference rather than a colour: colour values in
effects become `ColorRef = Literal(Color) | Slot(String)`, and the parsed
show carries its `palette` of defaults. Song loading and validation stay
as they are, and the parsed song is shared across playlists as it is
today.

When a song is about to play, the player works out its palette (defaults,
then the song's overrides, then the current playlist entry's) and gives
the resolved map to the timeline when the engine takes it. The timeline
resolves every `Slot` to a `Color` once, before the first frame. From
then on the engine only ever sees literal colours, so the render path,
seek replay and hue transitions don't change.

Everything else that builds a timeline without a playlist entry (lint,
readiness, analyze, MCP's evaluate, the editor preview) uses the defaults
plus the song's override, and can take an explicit palette where a
caller wants to preview one.

### 2.5 Surfaces

- **Web UI, show editor:** a palette panel to add/rename/recolour slots
  (reusing `ColorInput.svelte`). The colour inputs in the effect forms
  offer "palette slot" as well as a literal. Renaming a slot rewrites its
  references, through the parser's document edit layer.
- **Web UI, song editor:** the song's slots (from all its shows) with
  their defaults, and an override picker for each.
- **Web UI, playlist editor:** per entry, the same swatches with override
  pickers, plus a playlist-wide strip showing each song's resolved colours
  side by side. That strip is what shows a set is "heavy on one colour".
  Overrides naming a missing slot get a warning badge.
- **gRPC / app:** playlist entry settings (phase 3 §2, `EntrySettings`)
  gain `map<string, string> palette`. The app shows and edits it in the
  entry sheet. No new RPC.
- **MCP:** the DSL reference documents `palette`, and the song/playlist
  tools carry the field.

## 3. Layers

### 3.1 Configuration

In the player's lighting config (`dmx.lighting`), bottom to top:

```yaml
dmx:
  lighting:
    layers: [background, midground, foreground, accents, blinders]
```

- When it's left out, it's `[background, midground, foreground]`, so every
  existing show and config behaves exactly as it does now.
- Names are identifiers, unique, between one and 16 of them.
- A show's default layer (when an effect names none) stays the **bottom**
  layer.

### 3.2 In the DSL

`layer: <name>` takes any identifier. The grammar stops listing layer
names; the parser checks the name against the player's layer list. So a
`.light` file is parsed against a layer set, the same way it's already
checked against the venue's groups:

- A show naming a layer the player doesn't have **fails to load**, with
  the layer name, its position, and the player's layers. The web UI shows
  this in the editor as it shows other parse errors.
- `clear(layer: …)`, `freeze`, `unfreeze` and `master` take the same names.

### 3.3 In the engine

- `EffectLayer` becomes an index into the configured list:
  `struct EffectLayer(u8)`, ordered by index. The name lives in a
  `LayerSet` the engine and parser share.
- Multiplier channels are named by index (`_dimmer_mult_0`, `_1`, …) in
  place of `_bg/_mid/_fg`. These channels are internal and never written
  to files. `is_multiplier_channel` matches the new form.
- The hand-written triple product in `state.rs:252` becomes a loop over
  the layers.
- **The foreground rule becomes the top-layer rule** (§6.4): a `replace`
  effect on the top layer is dimmed only by the top layer's multipliers;
  everything else is dimmed by all of them. With the default three layers
  that's exactly today's behaviour. The alternative is a general rule: a
  `replace` on layer *k* is dimmed only by layers *k* and above. That
  changes midground today (a midground replace would stop following a
  background dimmer), so it isn't recommended.
- Status and the web UI's live state report layers by name.

### 3.4 Surfaces

- The player reports its layer list (in the existing lighting status/config
  REST response, and a `repeated string layers` on the gRPC call the app
  already uses for the light view).
- The web UI timeline draws one lane per configured layer, top on top,
  labelled by name (shortened when the lane is narrow). The `BG/MG/FG`
  i18n keys go away. The layer field in effect forms becomes a dropdown
  of the player's layers.
- The layer list is edited in the web UI's player/lighting settings, like
  the venue choice. Removing a layer that a show uses is refused, with the
  shows that use it listed.
- The docs book and the MCP DSL reference describe layers as configured,
  with the default three.

## 4. Tests

- Palettes: parse (declare, refer, undeclared slot, slot as sequence
  argument, colour lists, steps); resolution order (default → song →
  entry); warnings for unknown overrides; a golden show rendered with and
  without overrides differs only in colour; seek into an overridden song
  replays resolved colours.
- Layers: the default list reproduces today's output on the golden and
  visual-consistency suites exactly (no output change is the bar for
  L1); a five-layer config with effects on each; unknown layer refused at
  load; clear/freeze/master on a configured layer; the dimming rule
  (§3.3) on every layer.
- Web UI: journeys for editing a slot in the show, overriding it in a
  playlist entry, and a show on a custom layer drawing its lane.

## 5. Phases

| # | What | Size |
|---|---|---|
| L1 | Layers in the engine: `EffectLayer(u8)`, `LayerSet`, index multiplier channels, loop in `state.rs`; default list only; no output change | Medium |
| L2 | Layers configurable: `dmx.lighting.layers`, parser checks names, status/gRPC report them, web UI lanes and dropdown, settings editor, docs | Medium |
| P1 | Palettes in the DSL and engine: `palette {}` block, `palette.<slot>`, `ColorRef`, resolve before first frame, defaults only; writer round-trip; docs | Medium |
| P2 | Overrides: song YAML and playlist entry fields, resolution order at play, warnings | Small |
| P3 | Palette UI: show editor panel and slot pickers, song editor overrides, playlist editor overrides and colour strip | Medium |
| P4 | Over gRPC and in the app: `EntrySettings.palette`, entry sheet | Small; after phone-remote phase 3 P1 |

The layer and palette tracks don't depend on each other. L1 and P1 can be
built side by side.

## 6. Decisions

1. **Palettes are fixed for a song's whole play, never changed live.**
   Decided.
2. **Where colours come from: show default, then song override, then
   playlist-entry override.** Decided.
3. **Layers are a named, ordered list in the player config, defaulting to
   the current three, at most 16.** Decided.
4. **Dimming of a `replace` effect with more than three layers** (§3.3).
   Open. Recommended: only the top layer escapes lower multipliers, which
   keeps today's behaviour exactly and is the smaller change; layer-*k*
   generalisation can come later if a show needs it.
5. **Slot reference spelling: `palette.primary`** (§2.2). Decided.
6. **A show must give every slot a default** (§2.2). Recommended: yes, so
   a show always plays and previews without a song or playlist.
