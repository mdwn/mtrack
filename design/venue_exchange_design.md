# Venue Exchange: GDTF/MVR Import, the Rich Fixture Model, and Physical-Unit Movement

*Design doc, draft 5 — 2026-08-19. Draft 5 revises the file-format story
after implementation review: the extension identifies the DSL generation (v1
`.light` files are not renamed or migrated), the intermediary model is
machine-only, the DSL is scoped to the datasheet-typable subset, and every
DSL construct ships in the same phase as its consumer.*

> **Terminology:** today there is exactly one fixture/venue definition
> syntax — the `.light` DSL. This document calls it "v1" only to contrast it
> with the **planned** extended syntax ("v2") that phases P1a–P1c will
> introduce under the `.fixture`/`.venue` extensions; until those phases
> ship, no v2 DSL exists. Show files are neither versioned nor touched by
> this design. Feature scope is named by phase (P0–P2), never by version
> number.

mtrack shows already target roles — tags and logical groups — rather than fixtures, and a
show plays across multiple venues today. What doesn't scale is everything underneath: every
fixture definition and venue patch is hand-transcribed, capabilities are inferred from
channel-name strings, and movement has no physical vocabulary. This design fills that in:
fixture definitions sourced from manufacturer GDTF files, venue patches imported from MVR,
movement authored in degrees and stage coordinates instead of raw DMX, and a pre-show lint
that answers "will my show work there?" before the van leaves.

## 1. Goals & non-goals

**Goals.**

- Import a manufacturer `.gdtf` file and get a correct, reviewable mtrack fixture_type
  without reading a manual. (Proven exact for the Astera PixelBrick: the distilled file
  matched the hand-written one byte-for-byte, including strobe DMX offset and Hz range.)
- Import a venue's `.mvr` file and get a playable venue: patch, positions, fixture types.
- Author movement in physical units (`pan: 130deg`, `focus: "drummer"`) so shows transfer
  between fixtures and venues.
- 16-bit channel support, required for smooth movement.
- A venue visualizer grounded in real positions; full 3D lighting simulation in phase 2.
- Lint that reports capability gaps, unresolvable groups/focus points, and infeasible moves
  per venue.

**Non-goals (initial scope).** GDTF/MVR *writing* (deferred; the model is designed to be exportable).
Wheels, gobos, matrix/pixel modes, RDM (skipped loudly on import). Full geometry-tree
kinematics (tier-3 fidelity visualizers need; simple spherical pointing math suffices — §8).
OFL import (possible later addition for hobbyist gear; not in scope here).

## 2. Settled decisions

| Decision | Choice | Why |
|---|---|---|
| GDTF parser | Own implementation (quick-xml) | Spec is public and well-specified (DIN SPEC 15800). gdtf-rs is read-only, single-maintainer, 1-star; we'd own it either way. We control subset, leniency, and error reporting. |
| MVR parser | Own implementation | No Rust crate exists. Simpler format; reuses our GDTF machinery for embedded fixtures. |
| Fixture sourcing | GDTF-sourced fixtures are *referential*: a thin `.fixture` file names the GDTF + mode (+ overrides); the expanded channel table is an ephemeral cache, never committed | Fixture data is a manufacturer fact — a fat distilled copy can only drift from its source. The cache pattern (hash-keyed, regenerated on change) is the waveform-cache model mtrack already has. |
| Venue sourcing | MVR import *seeds* an owned `.venue` file | Venues are authored, not derived: tags, focus points, and position tweaks are human judgment layered on the import. |
| File extensions | `.fixture`/`.venue` will identify the planned v2 DSL as P1a–P1c introduce its constructs; existing definitions stay in `.light` files, valid beside them | The extension *is* the version marker (versions mark breakages, not expansions). This design renames, migrates, and deprecates nothing, and there is no in-file version field. No v1 removal is scheduled — retiring v1 someday is a legitimate future decision, but it would be its own design, with its own migration story. |
| GDTF/MVR export | Deferred (phase 2+) | Nothing in the initial phases depends on it; model stays exportable. |
| Visualizer | 2D top-down in phase 1; real 3D simulation in phase 2 | Positions/orientations/beam data and glTF assets are retained from import day one so 3D is additive, not a re-import. |
| Position abstraction | Named focus points, bound per-venue | The positional analog of tags: shows say `focus "drummer"`; venues supply coordinates. |
| Legacy path | MIDI-to-DMX layer untouched | It's the working fallback while this lands. |
| Intermediary | The serde model and its cache are **machine-only** — never hand-edited, never a user-facing format | GDTF and the DSL are the two sources; both compile into the intermediary. Letting users hand-craft a compilation target forfeits its derived/rebuildable property. |
| DSL scope | The DSL models the **datasheet-typable** subset of what the engine consumes | Asset-class data — 3D meshes, geometry trees, gobo images, spectral/emitter data — is GDTF-only; no text format can carry it, and everything downstream degrades gracefully without it (generic body, cone from beam angle, sRGB-ish color). |
| Syntax delivery | Every DSL construct ships in the same phase as its engine consumer | Defined-but-inert syntax invites files that look configured but aren't, and freezes grammar shape before a consumer exists to pressure-test it. |

## 3. Architecture

The load-bearing structural claim: **untrusted zip/XML is parsed by one hardened path, once
per new or changed source file — never at show time.** MVR import seeds owned `.venue`
files. GDTF fixtures stay referential: a `.fixture` file pins the source archive and mode,
and the distiller expands it into a hash-keyed cache (the waveform-cache pattern — filled at
import or prewarm, invalidated when the GDTF or the distiller version changes). Show time
reads only native files and a warm cache; a cold cache at startup fills with a loud log
line, never silently at a cue.

```mermaid
flowchart TB
    A["venue .mvr · fixture .gdtf<br/>(zip + XML, untrusted)"]
    B["Importer / distiller<br/>parse · validate · warn loudly"]
    C["asset cache<br/>(glTF, thumbnails — kept for 3D, P2)"]
    D[".fixture refs · .venue files<br/>+ expansion cache (hash-keyed, ephemeral)"]
    E["Effect engine<br/>authored in deg · Hz · stage xyz"]
    F["DMX frames @ 44 Hz<br/>per-fixture function resolution · 16-bit fanout"]
    A -->|unzip · size caps · schema checks| B
    B -.->|kept for 3D| C
    B -->|seeds .venue · pins .fixture refs · fills cache| D
    D -->|loaded at startup| E
    E -->|interpolation · slew limits| F
```

Trust boundary: everything above the `.fixture`/`.venue` + cache line is **parse time**
(import / prewarm — the only place untrusted input is touched); everything below is **show
time** (native files + warm cache only).

## 4. Data model

### 4.1 Fixture types: `.fixture` files

Two forms, one runtime model. The common path is a **referential** fixture: the GDTF archive
is the source of truth, the file pins it and carries only human additions. The distiller's
expansion (full channel table, function ranges) lives in the hash-keyed cache, never on disk
as an editable file — so it cannot drift from its source, and a distiller improvement
reaches every fixture on next prewarm.

```
# spots/robe-esprite.fixture — referential (GDTF is the source of truth)
fixture_type "Robe-Esprite"
  from gdtf("library/Robe@Esprite@V1.1.gdtf", mode "Mode 1")
{
  # overrides and additions only — not in GDTF (§8)
  movement { max_pan_speed: 240deg/s  max_tilt_speed: 200deg/s }
}
```

The **native** form is the escape hatch for gear with no usable GDTF, and is what the
expanded model looks like — structured channels with fine bytes, physical ranges, and
DMX-range functions. Capability derivation moves from channel-name string matching to
declared data in both forms. Grammar decision: **hybrid** — a channel is a name-keyed
one-liner, taking a block only when it has structure, so simple fixtures stay as terse as v1.

```
# native (hand-authored) — also the shape of a cached expansion
fixture_type "House-Blinder" {
  channels: 4
  channel "dimmer" @ 1 fine 2
  channel "red"    @ 3
  channel "strobe" @ 4 {
    functions: { "off": 0..7, "strobe": 64..255 -> 0.3hz..25.0hz }
  }
}
```

Canonical channel names (`red`, `dimmer`, `pan`, `ct`, …) are produced by the distiller from
GDTF's standardized attributes (`ColorAdd_R`, `Dimmer`, `Pan`, `CTC`), making the distiller
the normalizer — multi-user configs stop diverging on spelling. Debuggability:
`mtrack lighting expand <fixture>` (and the webui detail view) dumps the resolved model,
since a referential fixture's runtime truth isn't otherwise a text file you can read.
Existing `.light` fixture files (`channel_map` + three strobe fields) stay valid,
unrenamed and unmigrated, and will load beside v2 files once those exist; internally
both normalize through one conversion point (`From<FixtureTypeV1>`). "Detach to
native" renders control data as DSL and is lossy w.r.t. GDTF asset data (which has no
textual form) — it says so loudly. **None of the syntax shown in this section exists
yet**: it is the v2 target, and each construct lands with its consumer (§13), never
ahead of it.

### 4.2 Venues: `.venue` files and focus points

Venues are authored, so MVR import *seeds* a `.venue` file you then own — tags, focus
points, and position corrections are yours, and re-importing a revised MVR diffs against
your file rather than replacing it.

```
# kellys-basement.venue
venue "kellys-basement" {
  # mtrack stage convention: meters, right-handed Z-up, origin at
  # downstage-center on the deck · +x stage-left · +y upstage · +z up
  fixture "Spot1" "Robe-Esprite" @ 1:1
    tags ["spot", "rear"]
    position (-2.0, 3.5, 4.2)  rotation (0, 0, 180)

  focus "drummer"      (0.0, 2.8, 1.4)
  focus "center-stage" (0.0, 1.5, 1.7)
}
```

Tags remain the role abstraction; **focus points are the positional equivalent**. Shows
reference focus names, venues bind coordinates. Position/rotation are optional — a venue
without them still plays; it just can't resolve positional effects or draw a meaningful
stage view, and lint says so.

**Seeded, not referential (settled in P1b).** "Isn't the MVR effectively the venue?" —
it is the venue's *rig facts*, not the venue: tags, focus names and the stage origin are
the band's, and every fixture line needs them, so a `from mvr(...)` reference would not be
thin the way `from gdtf(...)` is, and would buy runtime overlay semantics for nothing.
Provenance is therefore a body statement, `imported from mvr("lighting/library/x.mvr")
origin (x, y, z)`, that the loader ignores. The MVR is copied into the library so re-import
can tell three things apart: a fixture in both MVRs (rig facts updated, tags kept), one the
venue removed (dropped, reported with its tags), and one only the `.venue` has (a hand
addition, kept). Focus points merge by name, or by position when the band renamed the
console's name. Hand-written venues of the same name are never overwritten. MVR focus
point objects are read and seeded; the console's names are there to rename.

## 5. GDTF parser (owned)

quick-xml over the extracted `description.xml`, into a spec-shaped object model, then
distillation. We read the subset the distiller needs and skip the rest *loudly* — every skip
is a named warning in the import report, per the harness's assume-everything/skip-loudly
stance.

| Read | Skip (warn) |
|---|---|
| FixtureType metadata; AttributeDefinitions; DMXModes → DMXChannels → LogicalChannels → ChannelFunctions → ChannelSets (offsets, fine bytes, DMX ranges, physical from/to); Geometries (enough to resolve channel→geometry references and skip virtual channels); beam data (angles); Revisions (provenance) | Wheels & gobo resources; matrix/pixel template channels; FTPresets; Protocols; RDM; emitters/filters/CRI. Models (glTF) aren't parsed but are copied to the asset cache for phase 2. |

Known wrinkles, all hit in the PixelBrick experiment: **virtual channels** (no DMX offset —
excluded from footprint), **multi-function channels** (pick dominant per canonical name,
keep function table), **mode selection is human input** (importer lists modes with
footprints; `--mode` or UI picker required).

> **Security posture:** an MVR is a zip a stranger emails the band, parsed by the machine
> that runs the show. The extraction layer enforces: no path traversal (zip-slip), no
> symlinks, per-entry and total decompression caps, entry-count caps, XML depth/size limits,
> no DTD/entity expansion (quick-xml default — keep it that way). Both parsers are
> cargo-fuzz targets from day one.

## 6. MVR parser (owned)

Zip containing `GeneralSceneDescription.xml` plus embedded GDTF files. We read: layers →
fixtures (name, `GDTFSpec` reference, `GDTFMode`, address, universe, 3D transform), and hand
each embedded GDTF to the §5 pipeline. Output: a seeded `.venue` file with positions, plus
`.fixture` refs for every referenced fixture. Fixtures whose GDTF is missing or unparseable
become explicit `TODO` entries in the venue file — the import never silently drops a patched
fixture.

**Coordinates:** MVR is right-handed Z-up in millimeters with an *author-chosen* origin —
the spec fixes no stage origin. Import converts to mtrack's stage convention (meters, origin
downstage-center; §4.2) and includes a re-origin step: the user picks a reference point or
fixture, since every venue's file will be offset differently.

**Mode matching:** the spec requires `GDTFMode` to name a mode in the GDTF exactly, but real
console exports drift (truncation, re-punctuation). Fallback chain: exact match → normalized
match (case/whitespace/punctuation; warn) → unique DMX-footprint match (only one mode has
the patched channel count; warn) → hard error listing candidate modes.

## 7. Import pipeline & library management

- **Entry points:** `mtrack lighting import-gdtf <file> --mode <name>`,
  `mtrack lighting import-mvr <file>`, and webui upload with a mode picker. Both produce the
  same import report (what was read, what was skipped, what needs human input). Import =
  validate the archive, copy it into the library, write the `.fixture` ref (and seed the
  `.venue`), fill the cache.
- **Layout:** `.fixture`/`.venue` files where fixture_types/venues live today. GDTF archives
  under `lighting/library/` — *committed*, since they're now the source of truth a
  referential fixture resolves against. Expansions and extracted assets (glTF, thumbnails)
  under `lighting/.cache/` — gitignored, rebuildable.
- **Invalidation:** cache key = GDTF content hash + mode + distiller version + override
  hash. A changed archive or upgraded distiller regenerates on prewarm; the import report
  and lint both surface when a resolved fixture changed since last run, so an upgrade never
  silently reshapes a working rig on gig day.
- **Editing:** the webui shows referential fixtures as a read-only resolved view (provenance
  banner: archive, mode, revision) plus an editable overrides pane; native fixtures get the
  full editor (fine bytes, ranges, functions, movement speeds). "Detach to native" copies
  the expansion into an editable file for the rare full-tweak case.

## 8. Engine: the physical-value pipeline

Movement is what breaks the write-a-value-to-a-named-channel model. The engine gains one
layer:

1. **Effects emit physical intents** — pan/tilt in degrees (or a focus-point target), strobe
   in Hz, color as today. Color/dimmer effects keep their existing semantics; nothing about
   the explicit-durations effect model changes.
2. **Per-fixture resolution** maps intents through the resolved fixture model: degrees →
   channel-function DMX range interpolation; focus targets → pan/tilt via pointing math
   (below); one logical value → coarse+fine bytes (16-bit fanout) in `to_dmx_commands`.
   Color lives here too: shows keep `red`/`green`/`blue` parameters exactly as today,
   CCT/white channels are handled in this layer, and physical color params
   (`color_temp: 3200K`) are purely additive.
3. **Interpolation & slew:** movement interpolates in physical space at the 44 Hz tick
   (44 Hz is ample; 8-bit quantization was the real smoothness problem). Configured
   `max_pan_speed` clamps output; lint flags cues that demand more than the fixture can do.

Positions also give chase *direction* real meaning: `left_to_right` used to
order fixtures by their position in the resolved group list, which has no
spatial (or cross-universe) significance. Since P1b, when the venue places
every fixture in the group, chase ordering resolves from positions — seen
from the audience, so `left_to_right` is stage-right to stage-left,
`top_to_bottom` upstage to downstage, `clockwise` around the centroid from
upstage — with list order as the fallback for position-less (or partially
placed) groups.

**Pointing math (tier 2, not tier 3):** fixture position + mounting rotation + pan/tilt
ranges → spherical solve for "aim at (x,y,z)". No geometry-tree kinematics; a page of
trigonometry, property-tested (§12). Fixtures with unattainable targets (out of range) clamp
and warn.

**Effect language sketch:**

```
effect "verse-sweep" {
  target: group("spots")
  focus: "center-stage" -> "drummer"   # physical, venue-resolved
  duration: 2 bars
  easing: smooth
}
```

## 9. Visualization

**Phase 1 — positional 2D.** StageView stops faking layout from tags: top-down stage plot
from venue positions, orientation ticks, beam-direction cones for movers (from live pan/tilt
state + beam angle), live color/intensity overlay from the existing 20 Hz snapshots
(snapshots gain position + pointing data). Fixtures without positions fall back to today's
tag layout, visually marked. Focus points are draggable pins — this is also the focus-point
editing UI.

**Phase 2 — 3D simulation.** Real 3D pre-viz: venue space, fixture bodies from the cached
glTF models, beam rendering, "play the show against a venue you've never seen." Everything
phase 2 needs (transforms, beam data, models) is captured and stored in phase 1 — 3D is a
rendering project, not a data-model project.

## 10. Surfaces

- **webui API:** upload endpoints for `.gdtf`/`.mvr` (import report as the response), mode
  listing, focus-point CRUD, fixture-type editing CRUD.
- **MCP:** `list_fixture_types` gains capabilities/ranges/provenance; venue tools gain
  positions and focus points; `evaluate_show` gains the new lint classes. The import flow
  gets first-class tools — `list_gdtf_modes`, `import_gdtf`, `import_mvr`, import-report
  retrieval — so an AI agent can close the loop end-to-end: fetch the GDTF from the
  manufacturer's site, pick the mode against the patch sheet, import, and read back the
  warnings. "Get the file from the manufacturer" stops being a chore when an agent can do it.
- **Docs:** new import + touring-workflow guide (sourcing GDTFs from the manufacturer or
  GDTF-Share is the documented user path — mtrack never fetches them itself); regenerate
  screenshots (StageView changes substantially).

## 11. Lint & pre-show analysis

The tool a touring user runs when the venue's file arrives. New checks on top of the
existing group-resolution lint:

- Capability coverage: show uses strobe/movement/CT in a group whose venue fixtures can't do
  it (with the degradation the profile will apply, stated).
- Focus points referenced by the show but unbound in the venue.
- Movement feasibility: cue requires more than `max_*_speed`, or target outside pan/tilt
  range from a fixture's position.
- Positional effects against a venue without positions.
- Universe coverage: the venue patches fixtures on universes the active
  profile configures no output for (today this is reported loudly at venue
  registration and on first drop; lint makes it a pre-show answer).
- Import hygiene: fixture_types whose source GDTF has a newer revision in the library.

## 12. Testing

- **Golden corpus, two tiers:** (1) synthetic GDTF/MVR files we author — committed, full
  spec-feature coverage (virtual channels, 16-bit, multi-function channels, matrix modes
  that must skip loudly, sloppy MVR mode strings); this tier is the CI backbone. (2) Real
  manufacturer files as a *bring-your-own local corpus*: a gitignored `tests/gdtf-corpus/`
  directory a developer fills (e.g. the Astera library), run via a manual/`#[ignore]`d test
  target, with a checksummed manifest recording which file versions produced the committed
  snapshots. No CI fetches from manufacturer sites — those URLs rot and block non-browser
  clients, and a red build from a vendor's CDN is noise, not signal.
- **Fuzzing:** cargo-fuzz targets for the zip layer and both XML parsers; malformed-archive
  regression suite (zip-slip, bombs, truncations).
- **Property tests:** pointing math round-trips (aim → pan/tilt → direction), 16-bit fanout
  monotonicity (no coarse-byte jumps across fine rollover), physical-range interpolation
  against channel-function tables.
- **Equivalence:** parsing a v1-DSL definition into the internal model is lossless —
  identical channel maps and strobe parameters — and existing configs produce
  byte-identical DMX. A referential PixelBrick (`from gdtf(...)`) resolves identically
  to its native-form equivalent.
- **Cache correctness:** expansion regenerates on archive hash, mode, distiller-version, or
  override change — and only then; cold-cache startup fills loudly and deterministically.
- **Harness:** a DMX frame-capture sink joins the audio loopback — hardware e2e checks
  assert emitted frames: strobe function offsets, 16-bit continuity during a slow sweep,
  slew clamping, focus resolution on a venue with known geometry.
- **webui e2e:** import flow (upload → mode pick → report → files exist), focus-point
  editing, positional StageView.

## 13. Phasing

| Phase | Scope | Exit criterion | Size |
|---|---|---|---|
| P0 | Internal only: rich fixture model (the v1-DSL view derived, `From<FixtureTypeV1>` conversion) + expansion cache plumbing; no grammar, no new extensions, no user-facing surface | Existing configs produce byte-identical DMX; cache fill/hit/corruption covered | S |
| P1a | GDTF parser + distiller, CLI + webui import, corpus + fuzzing, security hardening. **Introduces** the `.fixture` extension and referential syntax (`from gdtf(...)`, `movement`) — born working | PixelBrick distills byte-identical; a 16-ch+ mover distills with only expected warnings | M |
| P1b | MVR import, positions, focus points, positional 2D StageView, lint expansion. **Introduces** the `.venue` extension and `position`/`rotation`/`focus` syntax | A real venue MVR imports to a playable venue; show lint runs against it | M |
| P1c | Physical-value pipeline, 16-bit fanout, movement effects, slew model, harness DMX sink. **Introduces** rich channel syntax (`fine`, `range:`, function tables) for hand-authored fixtures | A movement show authored on one venue plays correctly on a second imported venue | L |
| P2 | 3D simulation (glTF, beam rendering); GDTF/MVR export; optionally OFL import | Show playable against a 3D venue never visited | L–XL |

Each phase ships independently; P1a is already useful alone (import replaces hand
transcription). Grammar follows the same rule as everything else here: a DSL construct
is introduced by the phase whose engine work consumes it, never earlier — so at no
point does syntax exist that parses but does nothing. The legacy MIDI-to-DMX path
stays untouched throughout as the working fallback.

## 14. Resolved questions

Open in draft 2; all resolved by draft 4.

1. **Coordinate convention** — stage-relative: meters, right-handed Z-up, origin at
   downstage-center on the deck, +x stage-left, +y upstage. MVR (right-handed Z-up,
   *millimeters*, author-chosen origin) is converted at import with a re-origin step (§6).
2. **v2 DSL grammar (planned, not shipped)** — hybrid: name-keyed channel one-liners,
   optional block only where a channel has structure (§4.1); constructs ship with their
   consumers (P1a–P1c). Simple fixtures stay as terse as v1. The v1 grammar is frozen,
   not deprecated: `.light` fixture/venue files remain valid, and no removal is
   scheduled. Retiring v1 is out of scope here — if it ever happens, it arrives as its
   own design with its own migration story.
3. **Corpus licensing** — sidestepped via the two-tier corpus (§12): committed synthetic
   files we author are the CI backbone; real manufacturer files are a bring-your-own local
   corpus, never fetched by CI (vendor URLs rot and block non-browser clients — brittle by
   construction). Users source their own GDTFs from the manufacturer; the MCP import tools
   (§10) let an AI agent do that legwork.
4. **Slew defaults** — no public database exists (verified; datasheet travel times like
   "540° in 2.2s" are the only common source). So: one conservative shipped default (order
   100°/s), per-fixture override, lint-only consequences (a wrong value is a noisy warning,
   never wrong DMX). The fixture editor accepts datasheet notation directly
   (`540deg / 2.2s`). Calibration is a *guided webui flow* first, CLI second: pick fixture →
   it sweeps full travel → tap when it stops → value written to the override; no extra
   hardware.
5. **Color model scope** — CCT/white handling enters the initial engine work (P1c) in the resolution layer only; show
   DSL parameters unchanged (RGB-first as today), physical color params additive (§8).
   Spectral/calibrated cross-fixture matching deferred to P2.
6. **Mode identity in MVR** — fallback chain: exact → normalized (warn) → unique-footprint
   (warn) → error with candidates (§6).
7. **Cache scope** — per-project `lighting/.cache/`.
8. **Format versioning** — no in-file version field. The extension is the version marker,
   and versions mark breakages, not expansions: additive syntax never mints a new
   generation, and changing the meaning of existing syntax is forbidden (new meaning
   requires new syntax). A v3 would arrive as a new extension, if it ever exists.

## 15. P1c in detail: the physical pipeline (draft 1, 2026-09-18)

P1a and P1b built the sources (GDTF, MVR), the model (rich channels, positions, focus
points) and the picture (stage plot). P1c is where the engine starts to *use* the model:
values in degrees and hertz, resolved per fixture into the bytes that fixture wants, and a
`move` effect that aims at the focus points the venue binds. This section fixes what §8
sketched, and splits it into shippable slices under the same rule as before: syntax ships
with its consumer, and every slice leaves existing shows producing byte-identical DMX.

### 15.1 What exists to build on

- `FixtureType` already carries `ChannelDef { offset, fine, range, functions }` with
  `PhysicalRange { from, to, unit: Degrees | Hertz }` per function, and the GDTF distiller
  fills it (16-bit as `fine`, Pan/Tilt ranges, the variable-strobe function). The engine
  ignores all of it: `FixtureInfo.channels` is still the flat name→offset map, and
  `to_dmx_commands` writes `(value × 255) as u8` per named channel.
- Strobe already reaches DMX as a frequency, through the three legacy fields
  (`max_strobe_frequency`, `min_strobe_frequency`, `strobe_dmx_offset`) that P0 reconciles
  with the strobe function. The normalization lives in `apply_strobe`'s caller.
- Venues carry `position`/`rotation` per fixture and named focus points; `FixtureInfo`
  carries them since P1b. Fixture types carry `movement { max_pan_speed, max_tilt_speed }`
  since P1a, consumed by nothing yet.
- The effect engine is stateless per tick — every effect computes its contribution from
  `elapsed` — except `last_merged_states`, the previous tick's merged output.

### 15.2 The resolution layer

`FixtureState` keeps its per-channel normalized values (0..1, layered and blended exactly as
today) and gains a **physical intent** alongside them:

```
PhysicalState { pan: Option<Degrees>, tilt: Option<Degrees>, strobe: Option<Hertz> }
```

Layering and blending of physical intents is *replace-by-layer*: the highest active layer's
intent wins per parameter; there is no meaningful "add" of two pan angles. Color and dimmer
are untouched.

Resolution happens where DMX is produced, `to_dmx_commands`, which gains the fixture's
`ChannelDef`s (carried on `FixtureInfo` as `channel_defs`, beside the flat map that every
existing caller keeps using):

1. **Physical → DMX**: a degree or hertz value is mapped through the channel's range (a
   `range:` on the channel, or the function whose `physical` covers it; the variable-strobe
   function is the existing case) by linear interpolation into that function's DMX
   sub-range. Out-of-range values clamp, and the clamp is reported once per effect
   (§15.5). A fixture whose channel has no physical range (a hand-written `channel_map`
   mover, or a GDTF that omitted it) resolves degrees over the full DMX range as if the
   range were the type's declared `movement` travel or, failing that, 0..540 pan / 0..270
   tilt with a lint warning. Nothing is refused: a venue with a thin fixture definition
   still moves, just less precisely.
2. **16-bit fanout**: a channel with `fine` emits two bytes from one 16-bit value, coarse
   from the high byte and fine from the low. Normalized (0..1) values on 16-bit channels
   fan out too, so a `static pan: 50%` on a 16-bit mover no longer leaves the fine byte
   at whatever it was. Monotonic by construction; property-tested (§12).
3. **Strobe in hertz** goes through the strobe function's range. The three legacy fields
   stay as the *derived* v1 view they already are; the caller-side normalization in the
   effects processor moves into resolution, so one code path serves `.light` fixtures
   with the three fields and `.fixture` ones with a function table.

Existing shows never produce a physical intent, so their DMX is byte-identical: the flat
channel path is unchanged, and the fanout only differs for channels that have a `fine`
byte, which no v1 fixture declares. The equivalence suite (§12) pins this.

### 15.3 Pointing math

A focus point is a stage-space target `(x, y, z)`. For a fixture at position `p` with
mounting rotation `R = Rz·Ry·Rx` (the P1b convention, degrees about X, Y, Z in that order):

```
d_world = normalize(target − p)
d_local = Rᵀ · d_world                 # into the fixture's mounting frame
pan     = atan2(d_local.x, d_local.y)  # 0° faces the fixture's local +y
tilt    = atan2(d_local.z, hypot(d_local.x, d_local.y))   # 0° level, +90° straight up
```

The fixture's "home" (pan 0, tilt 0) is its local +y axis, level. That is the same
convention P1b's stage plot draws the orientation tick with, so a fixture drawn facing the
drummer really does have the drummer at pan 0. GDTF's own rest pose (beam down −Z) is not
assumed; the mounting rotation in the venue is the whole story, which is what a venue
author can actually check against the room. Pan has a second solution 360° away wherever
the range allows; the solver picks the one nearest the fixture's current pan (pose memory,
§15.4), which is what a desk does and what stops a mover flipping through 500° to reach a
point 10° away. Unattainable targets (outside the pan or tilt range) clamp to the nearest
edge and are reported once per effect.

Property tests: `aim → (pan, tilt) → direction` round-trips to the target direction for
random poses and targets within range; the nearest-solution rule never chooses a pan more
than 180° from the current one when a nearer one exists.

### 15.4 The `move` effect and pose memory

The show-side syntax, in the cue grammar every other effect uses:

```
@00:12.000
spots: move focus: "drummer", duration: 2s, easing: smooth
spots: move pan: 45deg, tilt: -20deg, duration: 1s
spots: move from: "center-stage", to: "drummer", duration: 2measures
```

- `focus: "name"` (or `to:`) aims at a venue focus point; `pan:`/`tilt:` in degrees aim
  explicitly; the two forms do not mix in one cue.
- `from:` names the starting focus point (or `from_pan:`/`from_tilt:`). Without it the move
  starts from the fixture's **current pose**, which is the point of pose memory: the engine
  keeps the last resolved `(pan, tilt)` per fixture as engine state (initialized from the
  first move's target, so a show's first cue is a snap, not a sweep from an unknown place).
- `duration` is the travel time and is required, as every duration is. **When the travel
  ends, the pose holds.** A mover that has arrived does not snap anywhere when its effect's
  duration expires; the engine goes on emitting the held pose from pose memory until another
  move (or a `clear`) changes it. This is the one place the explicit-durations model
  reads differently — the effect's *contribution* is bounded, the fixture's *state*
  persists, exactly as a real fixture's does — and it is the only sane behaviour: the
  alternative writes pan/tilt to nothing, and the fixture sits wherever OLA's last frame
  left it, which is the same thing with no model behind it.
- `easing: linear | smooth` (smooth = ease-in-out); default `smooth`.
- Movement interpolates in **physical space** (degrees) at the tick, then resolves — never
  in DMX space, which is what made 8-bit chases look stepped.
- **Slew**: when the fixture type declares `movement { max_pan_speed }`, the tick clamps
  the per-tick delta to it, so a cue that asks for more than the fixture can do arrives late
  rather than commanding a jump the hardware would smear anyway. Undeclared = unclamped (the
  fixture applies its own limit); lint still warns using the conservative default (§14.4).

Chase, static and the color effects gain nothing; `static pan: 50%` keeps meaning a
normalized channel write, as today. Physical *intent* comes only from `move`, so no
existing cue changes meaning.

### 15.5 Lint

The P1b list (§11) items that waited for show-side syntax arrive with `move`:

- **Unbound focus point**: `focus: "x"` with no `focus "x"` in the current venue.
- **Positional effect against a venue without positions**: a `move focus:` on a group with
  a fixture that has no `position`.
- **Capability**: a `move` on a group with no pan/tilt channels (`capability-gap` grows a
  case).
- **Feasibility**: travel required ÷ duration exceeds the fixture's `max_*_speed` (or the
  default when undeclared — as a weaker "may arrive late" note); target outside range.
- **Precision**: a `move` on a fixture whose pan/tilt has no physical range (§15.2 fallback).

### 15.6 Rich channel syntax for hand-authored fixtures

The P0 grammar that was reverted returns, in `.fixture` files only, now that the resolver
consumes it. `.fixture` = the v2 DSL, whether referential (`from gdtf`) or native:

```
fixture_type "Cheap Mover" {
  channel "pan"  @ 1 fine 2 range -270deg..270deg
  channel "tilt" @ 3 fine 4 range -135deg..135deg
  channel "dimmer" @ 5
  channel "strobe" @ 6 {
    function "open"   0..15
    function "strobe" 16..255 0.5hz..20hz
  }
  channel "red" @ 7
  channel "green" @ 8
  channel "blue" @ 9
  movement { max_pan_speed: 240deg/s }
}
```

`channel_map` stays valid forever (§2 "coexist forever"). A `.fixture` may use either
form; a `.light` fixture file keeps the v1 grammar. `Display` renders whichever form the
file used, so the web UI's future fixture editor round-trips both.

### 15.7 Harness

The rig runs `olad`; the harness already probes it and gates DMX checks on it. The sink is
OLA itself: the harness reads universe state through OLA's client (`GetDmx`) at the tick
rate during a check and asserts on the frames. Checks: strobe function offsets on the
PixelBrick, 16-bit continuity through a slow sweep on a synthetic mover (no coarse-byte
jump across a fine rollover), slew clamp holding a sweep to the declared speed, and focus
resolution on a venue of known geometry (two fixtures, one focus point, expected pan/tilt
computed independently in the check). Playback stays on the loopback: DMX checks run beside
audio, not instead of it.

### 15.8 Slices

| Slice | Contents | Exit |
|---|---|---|
| P1c-1 (internal) | `channel_defs` + movement limits on `FixtureInfo`; `PhysicalState`; resolution in `to_dmx_commands` (range interpolation, 16-bit fanout, strobe through the function); strobe normalization moved out of the processor; equivalence tests | Existing shows byte-identical; a 16-bit synthetic mover fans out monotonically |
| P1c-2 | Pointing math + pose memory + `move` grammar/parser/effect + easing + slew clamp; lint (§15.5); timeline editor `move` form | A movement show authored on one venue plays correctly on a second imported venue (the P1c exit criterion) |
| P1c-3 | Rich channel syntax in `.fixture` + `Display` round-trip + docs | Hand-written 16-bit mover moves smoothly |
| P1c-4 | Harness DMX sink + four checks; stage view beam-direction ticks from live pan/tilt | 41+4 blessed on the rig |

### 15.9 Decisions (settled 2026-09-18)

1. **Pose memory**: hold after arrival. A mover that has arrived keeps its pose until the
   next move or a `clear`.
2. **Home convention**: pan 0 / tilt 0 is the mounting frame's +y axis, level — the stage
   plot's orientation tick. GDTF's rest pose is not assumed; the venue's mounting rotation
   carries the difference.
3. **`move` spelling**: `focus:` / `to:` / `from:` and `pan:` / `tilt:` in `deg`. No arrow
   form.
4. **Undeclared slew**: never clamped by the engine — the fixture applies its own limit —
   and lint warns against the conservative default. A declared `movement { max_*_speed }`
   is clamped.
5. **Rich channel syntax** lands in `.fixture` only; `.light` stays on the frozen v1
   grammar.
