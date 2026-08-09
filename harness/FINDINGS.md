# Hardware e2e findings

Defects found by `mtrack-harness` running against real hardware. Each has a
failing check, so an entry can be deleted once that check passes.

Reproduce with `./scripts/hardware-test.sh`.

All three reproduce on **two independent machines** (recorded 2026-08-08,
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

---

## 3. `active_playlist` does not survive a restart

**Tracked as [#359](https://github.com/mdwn/mtrack/issues/359).**

**Check:** `active_playlist_persists_across_restart` (area `persistence`)

Switching to `all_songs`, then restarting, reports `playlist` again. The field
is documented in `config/player.rs:98` as *"The active playlist name (persisted
across restarts)"*, so either the persistence is broken or the comment is wrong.

Reproduces independently of finding 2 — different project, different server, no
config mutation RPC involved.

---

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
