# Hardware e2e findings

Defects found by `mtrack-harness` running against real hardware. Each has a
failing check, so an entry can be deleted once that check passes.

Reproduce with `./scripts/hardware-test.sh`.

Both reproduce on **two independent machines** (recorded 2026-08-08,
both against `3a92ceb`):

- `mtrack-harness` (Pi, test rig): MAT USB interface 8 out / 16 in, outputs
  1-2 patched to inputs 3-4, ALSA `Midi Through` loopback, no olad.
- `btrackplayer` (Pi 5, production): Behringer WING console via
  `plughw:CARD=WING`, Roland UM-ONE, live olad on ArtNet/ShowNet.

---

## 1. `/api/devices/audio` advertises devices the player cannot open

**Tracked as [#357](https://github.com/mdwn/mtrack/issues/357).**

**Check:** `advertised_devices_are_openable` (area `devices`)

mtrack enumerates audio devices through two paths that do not agree:

| Path | Used by | Devices found |
|---|---|---|
| `audio::list_device_info()` | `GET /api/devices/audio`, i.e. the web UI picker | 19 |
| `audio::list_devices()` | `mtrack devices`, and `Device::get` when opening | 8 |

The 11 in the gap fail at playback with `no device found with name ...`:

```
alsa:default:CARD=MAT          alsa:plughw:CARD=3,DEV=0
alsa:dmix:CARD=Headphones,DEV=0  alsa:plughw:CARD=MAT,DEV=0
alsa:dmix:CARD=MAT,DEV=0       alsa:surround40:CARD=MAT,DEV=0
alsa:front:CARD=MAT,DEV=0      alsa:surround71:CARD=MAT,DEV=0
alsa:hw:CARD=3,DEV=0           alsa:sysdefault:CARD=MAT
alsa:iec958:CARD=MAT,DEV=0
```

**Impact.** Every alias of the USB interface *except* `alsa:hw:CARD=MAT,DEV=0`
is unopenable, so a user selecting their interface from the web UI dropdown has
a good chance of selecting one that cannot be opened. Note `alsa:hw:CARD=3,DEV=0`
is in the broken set while `alsa:hw:CARD=MAT,DEV=0` — the same device by index
rather than name — works.

**Suspected cause (unconfirmed).** The two functions are near-identical, except
`Device::list_cpal_devices()` calls `device.supported_output_configs()` *twice*
(once to test for `Err`, then again to iterate) while `list_device_info()` calls
it once. For an exclusive ALSA device the second call can yield an empty
iterator, leaving `max_channels == 0` so the device is silently dropped. This
would also explain why only one alias per card survives. Not isolated — worth
confirming before fixing.

---

## 2. `UpdateMidi` persists to a location the loader discards

**Tracked as [#358](https://github.com/mdwn/mtrack/issues/358).**

**Check:** `midi_beat_clock_persists` (area `midi-config`)

On a project using `profiles_dir`, enabling beat clock over gRPC rewrites
`mtrack.yaml` with `midi:` at the **top level**. But `config/player.rs:333`
discards top-level `midi`/`midi_device` when `profiles:` is present. The value
is visible in the served config and has no effect after restart:

```
midi beat_clock did not survive the round trip to disk:
  - 'beat_clock: true' did not reach 01-e2e.yaml, the profile file that owns this setting
  - the reloaded config discarded the written value:
      WARN mtrack::config::player: top-level 'midi'/'midi_device' ignored when 'profiles' is present
```

The same write is destructive in two further ways:

- **`profiles_dir` is dropped and the profiles are inlined into `mtrack.yaml`**,
  collapsing a multi-file profile layout into one file. The `profiles_dir:` key
  is gone from the rewritten config.
- Every unset `Option` is expanded to an explicit `~`, so the file is rewritten
  far beyond the field that changed.

`store.rs` does have `profiles_dir` write-back handling (it resolves the owning
profile file), so `update_midi` appears not to route through it. Likely also
affects `update_audio` and `update_dmx`, which were not exercised.

**Scope: the programmatic path only, not the web UI.** `MidiSection.svelte` is
rendered inside `ProfileEditor.svelte`, and `ConfigEditor.svelte` branches on
`profiles_dir` -- `saveProfileFile()` for a directory layout, `updateProfile()`
otherwise. Nothing in the Svelte API layer calls `PUT /config/midi`. So MIDI
settings edited in the UI, `persist_tempo` included, reach the profile that owns
them. What is broken is `update_midi` itself: gRPC `UpdateMidi`, the MCP tool,
and `PUT /api/config/midi`.

---

---

## 3. WITHDRAWN -- `active_playlist` (was #359)

Not a defect. The check switched to `all_songs`, which the player treats as
session-only by design (`player/navigation.rs`: "Switching to \"all_songs\" is
session-only (not persisted to config)"), so it asserted the opposite of
intended behaviour and failed on every rig -- which is why it appeared to
"reproduce on two independent machines".

The check now switches to a second generated playlist and passes. Issue #359
was closed as invalid. What misled the original report was
`config/player.rs`'s comment, which promised persistence without mentioning
the exception; that comment has been corrected.

## Verifying the checks themselves

A check that cannot fail is worse than no check, and three have shipped here:
`bogus_*_device_degrades_gracefully` read a status that was `initializing`
whatever mtrack did; `active_playlist_persists_across_restart` asserted the
opposite of documented behaviour; and `stop_halts_playback` used a four-second
song that ended on its own inside the ten-second stop deadline, so it passed
whether or not Stop did anything. All three survived several reviews.

```
./scripts/hardware-test.sh --self-test      # or: mtrack-harness --self-test
```

Every check names one thing it depends on and asks `sabotage::` whether to
break it. `--self-test` runs them all with the flag set and requires each to
report a defect **from its own assertion**. **26/26 on the MAT rig; 25/26 on
the WING**, where routing is not exercised because that console has no
loopback.

**What the score cannot tell you.** The self-test proves a check's assertion
fires; it cannot prove the assertion is *positioned* correctly. Where a control
substitutes the value the assertion reads (a "predicate-level" control, listed
in `checks.rs`) rather than breaking the world the assertion observes, it would
still score a pass if the check were later made vacuous the way
`bogus_*_device_degrades_gracefully` was -- reading something insensitive to
what mtrack does. Those controls are deliberate trade-offs, usually because the
world-level version killed startup before the assertion ran, and `--self-test`
prints how many of its passes rest on them. Read that number alongside the
total.

**Proof is opt-in.** Only the assertion macros (`check!`, `check_eq!`, `fail!`)
and constructors named `*_assertion` mark a failure as coming from a check's
own assertion; shared helpers -- log checks, readiness waits, RPC conversions,
`inconclusive!` -- produce pre-assertion values. The flag is private to
`outcome`, so it cannot be set by struct literal, which is how `inconclusive!`
set it wrongly for three rounds. That closes the accidental case. It does not
close the deliberate one: the constructors are public, and
`Client::wait_for_status` takes an `assertion` parameter by design. A helper
claiming proof must now say so in its own source.

Only an assertion counts. Failures carry a `from_assertion` flag, because a
control that dies inside `Server::start` reports `Failed` and would otherwise
be indistinguishable from one that drove its assertion to failure -- the same
"reading it cannot tell you" problem, one level up. Counting any non-pass as
proof initially hid four broken controls behind a green 26/26.

The control lives beside the assertion it guards, so refactoring cannot orphan
it, and completeness enforces itself: a check with no break point, or an
ineffective one, simply passes the self-test and is reported as a defect. That
is how the two lighting checks and a rig-dependent control were caught — the
self-test found them, not review.

An external source-mutating version was tried first and removed. Its mutations
were pinned to exact source strings, so it rotted whenever a check was edited,
and it did not know the registry held checks it never covered: it scored itself
16/16 while ignoring ten.

Three lessons from building it, all bugs in the controls rather than the checks:

- Profiles load in **filename** order (`config/player.rs`), so reordering the
  vec passed to `.profiles()` is a no-op sabotage.
- A *bogus* device stops the player booting, so the check fails at startup
  without reaching the assertion under test. A valid-but-different device is
  the correct control.
- A control must not depend on the rig supplying the failure condition. The
  plug-device check could not fail on the WING, where no raw device exists at
  all -- the very case it was written to tolerate.

## Non-defects worth knowing

These are working as designed but surprised the harness, and cost time:

- **Top-level `controllers:` is ignored when `profiles` is present.** It warns
  and continues, so the gRPC port simply never opens. Controllers must be in
  each profile, and only the *first* profile is active.
- **`hardware.profile` in `/api/status` is the profile's `hostname` field**, or
  `"default"` — there is no profile-name concept. Check the claimed device to
  confirm which profile was applied.
- **`hardware.<subsystem>.name` is the display form**
  (`alsa:hw:CARD=MAT,DEV=0 (Channels=8) (Alsa)`), not the config name.
- **ALSA plug devices report a fictional 32 channels.** `plughw`, `sysdefault`,
  `default` and `pulse` advertise 32 regardless of the hardware behind them, so
  ranking candidate devices by channel count picks a plug node over the real
  interface.
