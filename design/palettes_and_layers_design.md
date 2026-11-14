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
  primary: "#c01020"
  secondary: "orange"
  accent: rgb(255,240,200)
}

show "Main" {
  @00:00.000
  front_wash: static color: palette.primary, duration: 8measures
  back_truss: cycle color: palette.primary, color: palette.secondary, …
  @16/1
  sequence hit(color: palette.accent)
}
```

- `palette { … }` is a top-level block, beside `tempo` and `sequence`.
  Each slot takes any colour the grammar already accepts. There's at most
  one block per file.
- `palette.<slot>` works anywhere a colour works: `color:` (a `cycle`
  repeats it), a step's `color:`, and as a sequence argument or parameter
  default (bound like any other argument, so a sequence body still says
  `$color`). Hex values are quoted, because a bare `#` starts a comment.
- An included file may carry a palette: its slots serve its own sequences
  and merge into the including file, whose colour wins for a slot both
  declare (P1, #760).
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
- §3.4's brightness rule lands first, on the three-layer enum (phase B1),
  so it can be backported. L1 then carries it over to indexed layers
  without changing output.
- Status and the web UI's live state report layers by name.

### 3.4 Brightness: one model for every fixture

A fixture's hardware decides how mtrack drives it, not what a show means.
Today brightness means different things depending on whether a fixture has
a dimmer channel:

| | Dimmer-channel fixture | RGB-only fixture |
|---|---|---|
| Colour `replace` on a higher layer, lower layer dimmed | Still dimmed: colour writes only RGB, and the dimmer channel blends separately | Not dimmed on foreground, dimmed on midground (`state.rs:262`) |
| Dimmer `replace` on a higher layer, lower layer dimmed | Replaces the lower level (normal channel blending) | Multiplies with it: the blend mode is ignored and every brightness effect becomes a multiplier (`fixture.rs:291`) |

The new model, the same on every fixture. **The principle: dimming on a
layer applies to that layer and the layers beneath it, never to the layers
above it.**

1. **Each layer has its own colour and its own intensity.** Colour effects
   set the layer's colour; dimmer, pulse, chase and strobe effects set its
   intensity, each with its own blend mode, the same way any channel
   blends within a layer.
2. **Layers are composited bottom up, as light.** Start dark. For each
   layer in order: blend its colour into the light so far (with the
   colour effect's blend mode), then apply its intensity to the result.
   So a layer's intensity dims its own colour and everything beneath, and
   nothing above. This covers every blend mode:
   - A colour `replace` on layer *k* starts a new look: what was beneath,
     and its dimming, is gone. While the colour crossfades in, the lower
     light (already dimmed) fades out under it, so there's no jump.
   - A colour `add` (or `screen`, `overlay`) on layer *k* sits on top of
     the lower light *as it was dimmed*; the lower dimmer doesn't touch
     the added colour.
   - A colour `multiply` filters the dimmed light beneath.
3. **A dimmer's blend mode decides what it does to the dimmers below.**
   This holds for dimmer, pulse, chase and strobe alike. `multiply`
   scales the light so far. `replace`, the default for every effect,
   sets its level: the dimmers on layers below it no longer apply (as if
   at full), and the light so far is set to its level. That's what a
   dimmer channel does today on a fixture that has one.
4. **A fixture with no colour (a plain dimmer)** is treated as full white
   at the bottom, so the rules above reduce to blending its intensity
   layer by layer, exactly as its dimmer channel does today.
5. **Output depends on the hardware.** The composite is a colour of light
   (colour × level). On an RGB-only fixture it goes straight to RGB. On
   a fixture with a dimmer channel it's split into a level and a colour:
   the dimmer gets the highest level among the layers that contribute
   colour (`replace`, `add`, `screen`, `overlay`; a `multiply` only
   filters what's beneath and contributes none), and each layer's colour is scaled by its own level relative to
   that before blending, so the light out is the same. When one layer
   provides all the colour, which is the common case, the dimmer is that
   layer's level and RGB is its colour unchanged, so the DMX values match
   what the show wrote. CMY and colour-wheel fixtures go through the same
   split.

6. **Colour with nothing dimming it is full brightness**, on every
   fixture. A dimmer-channel fixture showing a colour gets its dimmer
   written (at full when nothing dims it) instead of keeping whatever
   byte was last sent. Decided by Mike.
7. **A level with no colour beneath it is white**, on every fixture: a
   strobe, dimmer, pulse or chase on a colour fixture with no colour
   effect shows white at that level, rather than whatever colour bytes
   were last sent (dimmer-channel fixtures) or nothing (RGB-only
   fixtures under a dimmer or pulse). Decided by Mike.

Strobe on a fixture with no strobe channel becomes an intensity
contribution on every fixture (today: the dimmer on dimmer-channel
fixtures, RGB on RGB-only ones), so it flashes the colour beneath it, or
white when there is none.

#### 3.4.1 In the engine

- Effects stop choosing between the dimmer channel and a multiplier.
  Dimmer, pulse, chase (gating modes) and strobe (without a strobe
  channel) all emit one internal intensity contribution with the effect's
  layer and blend mode. Colour effects emit colour channels as now; a
  chase in a lifting mode (`add`/`screen`) still emits colour.
- `FixtureState::blend_with` keeps colour channels (`red`, `green`,
  `blue`, `white`) and intensity contributions as an ordered list of
  operations, each with its layer, instead of blending them into channels.
  Other channels blend as they do now.
- At the end of the frame, before anything reads the states, each fixture
  is **resolved**: its operations are composited by §3.4 rules 2–3 (per
  layer, colour operations in order, then intensity operations in order),
  and the result is written back as plain `red/green/blue/white/dimmer`
  channels. Everything downstream (DMX output, CMY flags, snapshots, the
  live views, song crossfades) reads plain channels and loses its
  multiplier handling.
- The composite runs on three values per colour channel: the light so
  far, the same light with every dimmer treated as full (what a dimmer
  `replace` restores), and the peak level (rule 5). A crossfading
  `replace` lerps all three.
- Values a MIDI DMX show wrote to the fixture are the bottom of the
  stack: their colour (and dimmer) seed the composite, so DSL effects
  build on them exactly as they blend over them now.
- **Cells** (design §17.3): a cell's operations are merged with its
  fixture's by layer (the fixture's first within a layer) and resolved
  per cell. A fixture-level dimmer shared by cells gets the highest peak
  among its cells, and each cell's colour is scaled against it, so every
  cell's light is exact.
- The engine's multiplier clean-up for completed effects goes away: no
  state outlives the frame.

**Output that changes for existing shows** (all on purpose, listed in the
changelog):

- On dimmer-channel fixtures, a colour `replace` on midground or
  foreground is no longer dimmed by background/midground dimmers. This is
  the biggest change: a show that sets brightness on background and colour
  on a higher layer will play brighter. The fix in the show is to put the
  level on the colour's layer (`static color: red, dimmer: 30%`) or to
  blend instead of replace.
- On RGB-only fixtures, a midground colour `replace` is no longer dimmed
  by background dimmers.
- On every fixture, colour that a higher layer adds (`add`, `screen`,
  `overlay`) is no longer dimmed by dimmers beneath it; the lower colour
  still is.
- On dimmer-channel fixtures, a colour with no dimmer effect running is
  at full instead of the last dimmer byte sent; most visibly, a fade-out
  that ends while its colour runs on no longer stays dark (rule 6).
- A strobe, dimmer, pulse or chase with no colour beneath it shows white
  on every fixture (rule 7). On dimmer-channel fixtures it no longer
  reuses the last colour sent; on RGB-only fixtures a dimmer or pulse
  over no colour now shows white instead of nothing.
- On RGB-only fixtures, a strobe over a colour flashes that colour
  instead of white.
- On RGB-only fixtures, a higher-layer dimmer `replace` sets the level
  instead of multiplying with the levels below.

Before B1 lands, lint gains a check that names each place in a show where
the new rules change what's seen, and the prod shows are run through it:

- colour on a layer above one that dims the same fixtures at the same
  time (rules 2–3);
- a dimmer `replace` above a lower dimmer on the same fixtures (rule 3);
- on dimmer-channel fixtures, colour running while no intensity effect
  covers it, after one has ended (rule 6);
- an intensity effect on colour fixtures with no colour beneath it
  (rule 7).

Worked example, layers A (bottom), B, C (top):

| Show | Light | RGB-only fixture | Dimmer-channel fixture | Today, dimmer-channel |
|---|---|---|---|---|
| A red, A dimmer 50%; C `multiply` white | 50% red, as A alone | RGB 50% red | dimmer 50%, red | the same |
| A red, A dimmer 50%; C `replace` blue | full blue | RGB blue | dimmer 100%, blue | 50% blue |
| A red, A dimmer 50%; B `replace` blue | full blue | RGB blue (today 50% blue) | dimmer 100%, blue | 50% blue |
| A red; C `replace` blue, C dimmer 50% | 50% blue | RGB 50% blue | dimmer 50%, blue | the same |

### 3.5 Surfaces

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
  load; clear/freeze/master on a configured layer.
- Brightness (§3.4): every row of the §3.4 table, on a dimmer-channel and
  an RGB-only fixture, gives the same light on both; a higher-layer `add`
  over a dimmed lower layer, with the lower dimmer touching only the lower
  colour; a dimmer `replace` over lower dimmers; one-layer looks give the
  same DMX as today on dimmer-channel fixtures; a crossfading colour
  `replace` fades the lower intensity out with no jump; a plain dimmer
  fixture is unchanged; cells follow the same rule. Each golden and
  visual-consistency diff B1 causes is checked against the list of
  intended changes, and nothing else may move.
- Web UI: journeys for editing a slot in the show, overriding it in a
  playlist entry, and a show on a custom layer drawing its lane.

## 5. Phases

| # | What | Size |
|---|---|---|
| B1 | One brightness model (§3.4) on today's three layers: per-layer colour and intensity composited bottom up, dimming never reaches a layer above, blend modes honoured on every fixture, split to dimmer + colour by hardware; the lint check, run on the prod shows first; changelog. Backported (§5.1) | Medium–large |
| L1 | Layers in the engine: `EffectLayer(u8)`, `LayerSet`, per-layer intensity indexed; default list only; no output change from B1 | Medium |
| L2 | Layers configurable: `dmx.lighting.layers`, parser checks names, status/gRPC report them, web UI lanes and dropdown, settings editor, docs | Medium |
| P1 | Palettes in the DSL and engine: `palette {}` block, `palette.<slot>`, `ColorRef`, resolve before first frame, defaults only; writer round-trip; docs | Medium |
| P2 | Overrides: song YAML and playlist entry fields, resolution order at play, warnings | Small |
| P3 | Palette UI: show editor panel and slot pickers, song editor overrides, playlist editor overrides and colour strip | Medium |
| P4 | Over gRPC and in the app: `EntrySettings.palette`, entry sheet | Small; after phone-remote phase 3 P1 |

B1 comes first and is its own PR, because it changes output on purpose and
is backported. The layer and palette tracks don't depend on each other.
L1 and P1 can be built side by side.

### 5.1 Backport

**Today's layer behaviour is a bug, and B1 is its fix**, decided by
Mike. Dimming reaches layers above it: on dimmer-channel fixtures always,
on RGB-only fixtures everywhere but a foreground `replace`; and the two
kinds of fixture disagree. Both release lines have the same three-layer
code, so B1 is written against the enum as it is and is backported to
`release/0.18` and `release/0.17`, with its lint check, so anyone
upgrading can find what will look different in their shows.

The fixed count of three layers is not part of the bug. Configurable
layers and palettes are features and stay on main.

Because a patch release will change how some shows look, the changelog
entry for each line goes under **Fixed** and says so plainly, names the case that plays brighter
(brightness on background, colour `replace` above it, on dimmer-channel
fixtures), the fix in the show, and the lint command that finds it.

## 6. Decisions

1. **Palettes are fixed for a song's whole play, never changed live.**
   Decided.
2. **Where colours come from: show default, then song override, then
   playlist-entry override.** Decided.
3. **Layers are a named, ordered list in the player config, defaulting to
   the current three, at most 16.** Decided.
4. **Brightness is one model on every fixture: dimming on a layer applies
   to that layer and those beneath it, never to those above** (§3.4).
   Decided.
   Hardware decides how a fixture is driven, not what the show means.
   Backported to 0.18 and 0.17 (§5.1).
5. **Slot reference spelling: `palette.primary`** (§2.2). Decided.
6. **A show must give every slot a default** (§2.2). Recommended: yes, so
   a show always plays and previews without a song or playlist.
