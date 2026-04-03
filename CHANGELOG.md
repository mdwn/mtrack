# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **MIDI controller configuration in web UI**: The controllers section now supports full
  editing of MIDI controller event mappings (play, prev, next, stop, all_songs, playlist)
  with optional section_ack and stop_section_loop events. Includes an "Add MIDI" button
  alongside existing gRPC and OSC controller options.
- **Reusable MIDI event editor component**: Extracted a shared `MidiEventEditor` component
  used by both the MIDI controller configuration and the song MIDI event editor. Displays
  the event type dropdown and contextual parameter fields in a compact horizontal layout.
- **Song exclude MIDI channels**: The MIDI tab in the song editor now shows a 16-channel
  toggle grid when a MIDI file is configured, allowing users to exclude specific channels
  from playback. Commonly used to skip drums (channel 10) or lighting data channels.
- **Notification audio configuration in web UI**: New Notifications tab in the hardware
  profile editor allows configuring custom audio files for loop armed, break requested,
  loop exited, and section entering events, plus per-section-name overrides. Includes
  filesystem browse and drag-and-drop upload.
- **Per-song notification overrides**: New Notifications tab in the song editor allows
  overriding profile-level notification sounds for individual songs. Per-section override
  names autocomplete from the song's defined sections.
- **Global max sample voices setting**: The samples section in the config editor now
  includes a max sample voices field (default 32) controlling the global polyphony limit.
- **Status events in hardware profile**: Status events (MIDI events emitted on player
  state changes for off/idling/playing) are now configured per-profile in the new Status
  Events tab. Each state supports a list of MIDI events. Legacy top-level `status_events`
  is automatically normalized into the profile.

- **MIDI-to-DMX editor in web UI**: The MIDI section now has a full editor for MIDI-to-DMX
  passthrough mappings, replacing the previous read-only note. Each mapping configures a
  MIDI channel routed to a DMX universe, with optional Note Mapper and CC Mapper
  transformers that remap note/CC numbers before output.

### Improved

- **Web UI design system and accessibility overhaul**: Comprehensive design review and
  redesign pass across all pages, establishing a stronger design language and improving
  accessibility for assistive technologies.

  **Design system foundations:**
  - Added type scale tokens (`--text-xs` through `--text-xl`) replacing 7+ ad-hoc font sizes.
  - Added semantic color tokens (`--accent-subtle`, `--red-subtle`, `--yellow-subtle`,
    `--blue-subtle`, `--green-subtle`) — hardcoded `rgba()` values throughout components now
    reference design tokens.
  - Added `--bg-surface` token (was referenced but undefined).
  - Added `.sr-only` utility class, `.badge` global component, `.checkbox-row` form utility,
    and shared form layout classes (`.section-fields`, `.field`, `.field-row`, `.field-header`).
  - Added `prefers-reduced-motion` media query respecting user motion preferences.

  **Accessibility (WCAG compliance):**
  - Replaced `all: unset` on playlist buttons with explicit resets so focus indicators work.
  - Added `aria-expanded` to hamburger menu and sample collapsible headers.
  - Added `aria-pressed` to log level filter toggles, `role="log"` to log container.
  - Added `role="img"` and `aria-label` to canvas waveforms.
  - Added `role="tabpanel"` with `aria-labelledby` to all tab content panels.
  - Fixed song delete button from inaccessible `<span tabindex="-1">` to proper `<button>`.
  - Fixed song row from nested `<button>` (invalid HTML) to `<div role="link">`.
  - Fixed sample header from suppressed-a11y div to `role="button"` with keyboard support.
  - Added keyboard focus handlers to tooltips (`onfocus`/`onblur`).
  - Added WAI-ARIA arrow-key navigation to profile editor tab bar.
  - Added `aria-label` to status page subsystem dots.
  - Scoped SectionBar keyboard handler to focused container — prevents Delete key from
    destroying sections while typing in other inputs.

  **Visual polish:**
  - Replaced all emoji/Unicode icons in nav (play/pause, lock/unlock) with inline SVGs.
  - Thickened playback progress bar from 6px to 10px for better touch targets.
  - Improved loop badge sizing and error message treatment (8s timeout, dismiss button,
    colored background).
  - Added left-border accent to current playlist song for clearer visual indication.
  - Added music icon to dashboard empty state.
  - Added dirty indicator (`*` in yellow) to profile editor title when unsaved.
  - Improved cue block visibility in timeline (raised base opacity, stronger hover).
  - Widened cue color strip from 3px to 4px.
  - Added pulsing animation to disconnected status indicator.
  - Added tab overflow gradient fade on profile editor tab bar.

  **Layout optimization:**
  - Dashboard card-pair uses flexible height (`min-height`/`max-height`) instead of rigid 280px.
  - Effects card uses flexible width instead of fixed 280px.
  - Status page widened from 700px to 1000px with 2-column grid layout.
  - Timeline lane labels widened from 80px to 100px across all lane types.
  - Timeline bottom panel is now collapsible with toggle button.

  **UX improvements:**
  - Fixed playlist drag-and-drop with stable slot IDs (was using fragile `song + i` key).
  - Waveform canvas now applies DPR scaling for crisp rendering on HiDPI/Retina displays.
  - Transport uses CSS Grid layout with section controls spanning full width.
  - New sequence cue references auto-select the first available sequence definition.

- **Web UI UX overhaul**: Comprehensive usability pass across all pages, focused on
  reducing clicks, preventing data loss, and improving visual consistency.

  **Data loss prevention:**
  - Playlist editor warns before switching playlists or leaving the page with unsaved changes.
  - Sample deletion now requires confirmation.
  - "Remove section" confirmation in profile editor describes what will be lost (e.g.,
    "This will delete 5 track mappings and all audio settings.").
  - Ctrl+S / Cmd+S keyboard shortcut for saving in the config editor.

  **Live performance usability:**
  - Dashboard playlist songs are now clickable to jump directly to a song during playback.
  - Full-width disconnection banner when WebSocket connection drops.
  - Improved text contrast for dim/muted text (WCAG AA compliant).
  - Hardcoded English strings in playback card moved to i18n.

  **User flow improvements:**
  - Song detail tabs reduced from 7 to 5: MIDI merged into Tracks, Notifications merged
    into Config (both as collapsible sections).
  - File browser now starts in the song's directory instead of filesystem root.
  - "Import from Filesystem" is now the primary button in the song list; "New Song" is secondary.
  - Song list search query persists when navigating back from a song detail view.

  **Visual design consistency:**
  - Added missing CSS variables (`--bg-hover`, `--bg-danger`, `--text-danger`, z-index tokens).
  - Extracted shared `.error-banner`, `.panel`, `.panel-header`, `.btn-icon` classes from
    duplicated component styles into global app.css.
  - Standardized border-radius across all cards and panels.
  - Unified z-index layering with CSS variable tokens.

  **Polish:**
  - Status page auto-refreshes every 5 seconds with "Updated Xs ago" indicator; build info
    moved to the bottom; distinguishes "Not Configured" from "Not Connected."
  - Playlist editor shows position numbers, drag feedback, and filters out the `all_songs`
    system playlist.
  - Dashboard shows a consolidated empty state with action links when no playlist is loaded;
    hides empty cards (tracks, effects, stage view) to reduce noise.
  - Log card has level filter pills (TRACE/DEBUG/INFO/WARN/ERROR), defaulting to INFO+.
  - Loading spinners replace plain "Loading..." text across all pages.
  - Sample rename is now discoverable via a pencil icon (not just double-click).
  - Channel mapping inputs validate and show inline errors for non-numeric values.
  - Controller "Add" buttons have descriptive tooltips explaining each type.
  - Device refresh buttons show loading state during enumeration.
  - NotFound page has a "Back to Dashboard" button.
  - Lock button tooltip explains the consequence of locking/unlocking.
  - Stage view fixture drag positions persist to localStorage across page reloads.
  - Nav bar song name truncation relaxed (300px desktop, 150px mobile); mobile nav shows
    current page name next to the brand.
  - Tab hover states have subtle background highlight.
  - Aria-labels added to all icon-only buttons for screen reader accessibility.

### Fixed

- **Missing OSC path overrides**: Added section_ack, stop_section_loop, and loop_section
  to the OSC controller advanced path overrides panel.
- **Flaky save test**: Fixed race condition in song config save test by replacing a
  synchronous boolean flag with `waitForRequest`.
- **Config editor URL rewrite on refresh**: Navigating to a deep-linked profile tab URL
  (e.g. `#/config/profile-name/midi`) no longer rewrites to the bare profile URL on load,
  so refreshing the page preserves the active tab.

- **Song looping with crossfade**: Songs can now be configured to loop indefinitely by
  setting `loop_playback: true` in song.yaml. Audio crossfades seamlessly at loop boundaries
  (100ms linear fade), MIDI restarts from the beginning, and the lighting/DMX timeline resets
  cleanly. During a looping song, pressing Play or Next breaks out of the loop, advances the
  playlist, and auto-plays the next song. Stop cancels everything as usual.
- **Beat grid detection from click tracks**: Audio click tracks (tracks named "click") are
  analyzed offline to detect beat positions and measure boundaries. The result is a `BeatGrid`
  with absolute beat times and accented-beat indices, stored in a per-song disk cache
  (`.mtrack-cache.json`) alongside waveform peaks. Accent classification uses a pluggable
  `AccentClassifier` trait — the default `ZcrClassifier` separates click sounds by
  zero-crossing rate (timbral differences), with `AmplitudeClassifier` as an alternative.
  Beat grid data is exposed via gRPC proto and displayed in the web UI (measure/beat position
  during playback, beat/measure counts in song detail).
- **Song analysis disk cache**: Computed song data (waveform peaks, beat grids) is now persisted
  to `.mtrack-cache.json` in each song's directory. The cache uses file mtime+size for
  invalidation — if an audio file changes, its cached data is recomputed on next access.
  This eliminates redundant waveform computation on restarts.
- **Audio crossfade primitives**: New `CrossfadeCurve` enum (Linear, EqualPower) and
  `GainEnvelope` struct for applying time-varying gain to audio sources. The mixer's
  `ActiveSource` now supports an optional gain envelope, applied per-block during mixing.
  Sources with a completed fade-out envelope are automatically finished. These primitives
  support both song looping and future song-to-song crossfade transitions.
- **Morningstar SysEx integration**: mtrack can now automatically push the current song name
  to a Morningstar MIDI controller (MC3, MC6, MC8, MC6 Pro, MC8 Pro, MC4 Pro) via SysEx
  when songs change. Configured via an optional `morningstar` block on the MIDI controller,
  this eliminates the need for hand-maintained per-song program change mappings. Supports
  short/long preset names, configurable preset slots, save-to-flash, and custom model IDs.
- **Morningstar configuration in web UI**: The MIDI controller section in the hardware profile
  editor now includes a Morningstar preset naming panel with model selection, preset number,
  name type, and save-to-flash options.
- **Song change notifier system**: New `SongChangeNotifier` trait on the player enables
  pluggable reactions to song changes. The Morningstar integration is the first consumer;
  the trait is generic and supports multiple concurrent notifiers.
- **Section looping**: Named sections of a song (defined by measure ranges in song.yaml)
  can be activated during playback via gRPC `LoopSection`/`StopSectionLoop` RPCs. Audio
  crossfades at section boundaries (same 100ms linear fade as whole-song looping), MIDI
  restarts from section start with hard cut, and DMX/lighting timelines reset to the
  section's start time. Elapsed time reporting accounts for accumulated loop iterations
  via a `loop_time_consumed` accumulator. Section activation is rejected if playback has
  already passed the section end. A confirmation tone (1kHz, 50ms, -12dB sine with fade
  envelope) plays through the `mtrack:looping` track mapping when a section loop activates.
- **Visual section editor**: The Sections tab in the song detail view now shows a
  canvas-based timeline with all track waveforms, beat grid measure lines, and interactive
  section creation/editing. Sections can be created by dragging on empty space (snaps to
  measure boundaries), resized by dragging edges, moved by dragging the body, renamed by
  double-clicking, and deleted with the Delete key. Measure label density and snap
  granularity adapt to zoom level using power-of-2 stride thinning. Zoom controls include
  +/-, Fit, and Ctrl+scroll wheel with anchor-point zooming.
- **Section loop UI controls**: The PlaybackCard shows section buttons when playing a song
  with defined sections. Clicking a section button activates the loop; an active loop shows
  the section name and a "Stop Loop" button. Next/Prev navigation is allowed during looping.
- **Section config validation**: Song validation now checks section constraints (name not
  empty, start_measure >= 1, end_measure > start_measure).
- **Visual lighting timeline: duration-based blocks**: Effect blocks in the visual editor
  now display their actual duration as block width (previously all blocks were a fixed 500ms
  width). A right-edge drag handle allows resizing effects directly on the timeline, which
  updates the effect's `duration` parameter. New effects created via double-click default to
  `duration: 5s`.
- **Visual lighting timeline: per-layer lanes**: The single "effects" lane is replaced by
  three layer lanes — Foreground, Midground, Background — each showing only effects assigned
  to that layer. The show name appears in its own header row above the lanes. Layer lanes are
  derived from the `LAYERS` array for future configurability.
- **Visual lighting timeline: sequence expansion**: Sequence references are now expanded
  inline into the layer lanes, showing each iteration's effects at their correct timeline
  positions. Sequence-originated blocks are visually distinct with dashed borders and a pink
  tint. The sequences lane shows sequence references as blocks spanning their full expanded
  duration (all loop iterations), and dragging the right edge adjusts the loop count, snapping
  to whole iterations.
- **Tempo map detection from MIDI**: When a song has a MIDI file, the lighting
  tempo editor can extract an authoritative tempo map directly from MIDI
  `SetTempo` and `TimeSignature` meta events. Consecutive monotonic BPM changes
  (ritardandos/accelerandos) are automatically collapsed into single transitions.
  Falls back to beat grid estimation when no MIDI file is available. The beat
  grid's start offset (first audible beat) is used in both cases.
- **Tempo map estimation from beat grid**: Songs with a click track but no MIDI
  file can estimate a tempo map from the detected beat grid. Finds stable tempo
  sections, detects time signature changes from measure boundary spacing, and
  identifies discrete tempo changes. Results are snapped to measure boundaries.
  Displayed with an "estimated from beat grid" badge to indicate it's a guess.
- **Beat grid refinement**: Beat positions within stable tempo sections are
  snapped to an ideal grid after onset detection, removing ±5-15ms jitter from
  the onset detector. This improves tempo calculation accuracy at section
  boundaries.
- **Tempo editor in visual timeline**: The tempo map editor is now accessible
  by clicking the tempo lane in the visual timeline. Includes controls for BPM,
  time signature, start offset, and tempo changes. A "Detect from MIDI" or
  "Guess from beat grid" button populates the tempo map automatically.
- **Snap subdivisions**: The snap resolution in both the main timeline and the
  sequence editor now includes 1/2, 1/4, 1/8, and 1/16 beat subdivisions in
  addition to beat and measure snapping.
- **MIDI alignment quality warning**: After detecting a tempo map from MIDI,
  the lighting editor computes a beat-alignment RMSE between MIDI-predicted
  beat positions and click-track detections. If the error exceeds 15 ms, a
  warning badge is shown indicating the MIDI file may not match the recording.
  The `alignment_rms_ms` field is included in the `GuessedTempo` API response.
- **Compound meter beat stepping**: Tempo detection from MIDI now correctly
  handles 6/8, 9/8, and 12/8 time signatures by stepping by dotted-quarter
  note pulses (three eighth notes) rather than quarter notes, matching the
  natural click-track pulse for compound meters.
- **Lighting file management in song editor**: Light show files (`.light`) can
  now be added and removed directly from the song detail lighting tab. Each file
  is listed with a remove button; adding or removing files updates the song.yaml
  `lighting:` array. File creation and deletion are deferred until Save, so
  navigating away without saving leaves the disk untouched. Both operations
  respect the player lock.
- **Delete lighting file API**: New `DELETE /api/lighting/:name` endpoint for
  removing `.light` files from disk, with SafePath validation and `.light`
  extension enforcement.
- **Implicit lighting file on editor load**: Opening the lighting tab for a song
  with no `.light` files automatically creates an implicit file with a default
  "Main" show, so effect lanes are immediately visible. The file is only written
  to disk when the user saves.
- **Resize snap to grid**: Dragging an effect's resize handle now snaps the
  duration to the nearest beat or measure boundary (matching the timeline's snap
  resolution setting). Hold Ctrl/Cmd while releasing to bypass snap for
  free-form sizing.
- **Measure-based duration output**: Effect durations produced by resize and
  other UI operations now prefer measure/beat units (e.g. `1measure`, `2beats`)
  over time units when the duration aligns cleanly to the tempo grid. Falls back
  to `ms`/`s` for non-aligned values.
- **Double-click creates effect on layer lanes**: Double-clicking on a
  foreground/midground/background lane now creates a default static effect
  assigned to the correct layer, with a `1measure` duration when tempo is
  available. Previously, double-click only worked on the combined "effects" lane.

### Fixed

- **Effect block width uses max duration**: CueBlock width now reflects the
  maximum duration across all effects in the cue, rather than only the first
  effect's duration.
- **Effect resize was non-functional**: The `oneffectresize` callback was never
  wired from `TimelineEditor` to `ShowGroup`, so dragging the resize handle had
  no effect. Now connected with a handler that updates all effects in the cue.
- **CuePropertiesPanel not showing for layer lanes**: Clicking an effect on a
  layer lane (e.g. `effects:foreground`) didn't show the properties panel because
  the tab matching required an exact `"effects"` string. Now normalizes
  `"effects:*"` sub-lane types to `"effects"` for tab selection.
- **Tempo lost when adding light files**: Setting tempo on a song with no
  existing `.light` files, then adding a file, would lose the tempo because it
  was only stored in the merged state and never persisted to a file. Now
  auto-creates a backing file for tempo-only changes and inherits the current
  tempo when creating new files.
- **WebSocket connection banner**: The "Not connected to server" banner in the
  lighting editor used a manual store subscription pattern that could miss
  updates. Switched to the reactive `$wsConnected` store syntax.
- **song.yaml lighting key handling**: `buildYaml()` now explicitly manages the
  `lighting` key — non-empty arrays are preserved, empty arrays clean up the key
  entirely.
- **Flaky playlist save test**: The playlist mutations "save calls API" test
  checked a boolean synchronously after clicking Save, racing against the async
  fetch. Replaced with `page.waitForRequest()`.

### Changed

- **Lighting effects: explicit durations required (breaking change)**: The lighting engine no
  longer supports perpetual or permanent effects. Every effect must have an explicit `duration`
  or `hold_time` parameter. Effects that previously ran indefinitely until replaced (e.g.,
  `static color: "red"` with no duration) are no longer valid — the parser will reject them
  with a clear error message. This is a breaking change to the lighting show file format; old
  show files must be updated to include durations on all effects.

### Removed

- **Effect replacement semantics**: Effects no longer automatically kill conflicting effects on
  the same layer. Multiple effects can now coexist on the same layer simultaneously, with the
  blend mode determining how overlapping effects combine. This simplifies the mental model:
  effects are independent, finite blocks on a timeline.
- **Persistent fixture state**: The engine no longer preserves an effect's final state after it
  completes. When an effect's duration expires, its contribution to the output is gone. Dimmer
  effects no longer persist their final brightness level. This includes removal of the
  `fixture_states` store, channel locking, and the `is_permanent()` concept.

### Changed

- **Player method refactoring**: Converted several static `Player` methods (`emit_midi_event`,
  `prev_and_emit`, `next_and_emit`, `report_status`) to instance methods, reducing parameter
  passing and simplifying call sites. Playlist navigation now uses a `PlaylistDirection` enum
  instead of function pointers.
- **Consistent `parking_lot::Mutex` usage**: All `std::sync::Mutex` instances across the
  codebase have been converted to `parking_lot::Mutex` for consistency (no poisoning, simpler
  API). The only exception was already using `parking_lot::Condvar`.

### Fixed

- **WebSocket test isolation**: Playwright e2e tests now use namespace-based WebSocket routing
  (unique `wsId` per test) to prevent cross-test message contamination via the shared mock
  server.

## [0.11.2] - 2026-03-21

### Fixed

- **Trigger input stream recovery**: The trigger engine now automatically recovers
  from ALSA backend errors (e.g. POLLERR) by recreating the input stream, matching
  the existing recovery pattern used by the audio output stream. Previously, a single
  backend error would flood the logs with repeated error messages and leave the trigger
  engine non-functional.
- **Trigger input thread priority**: The trigger input callback thread is now promoted
  to real-time priority (SCHED_FIFO), matching the audio output and MIDI beat clock
  threads. This reduces scheduling jitter for trigger detection.

## [0.11.1] - 2026-03-20

### Fixed

- **Controller startup timing**: MIDI controllers (and any controller that depends on
  hardware devices) failed to start because controllers were initialized before async
  hardware discovery completed. Controllers are now started at the end of hardware
  initialization, after all devices are ready. This also affects hardware reloads —
  controllers are re-created with each reload cycle.
- **`Player::new()` returns `Arc<Player>`**: Since the player spawns async init tasks
  that require `Arc` access, `new()` now returns `Arc<Player>` directly instead of
  requiring callers to wrap it.

## [0.11.0] - 2026-03-20

### Added

- **Zero-config start**: `mtrack start mtrack.yaml` now works even when the config file doesn't
  exist. A default config is created automatically, the songs directory is created if missing,
  and the player starts idle with the web UI and gRPC available. No playlist file is required —
  the player falls back to an alphabetized all-songs playlist (which may be empty).
- **Empty profile fallback**: When no hardware profiles match the current hostname, the player
  starts with a synthetic empty profile (no audio/MIDI/DMX) instead of exiting with an error.
- **`config::Player` Default impl**: Produces a minimal config (`songs: songs`) suitable for
  bootstrapping a new installation.
- **Project directory mode**: `mtrack start` now accepts a project directory instead of a config
  file path. When given a directory, it looks for `mtrack.yaml` inside it. When given a file
  (or a path with a `.yaml`/`.yml` extension), it uses it directly for backwards compatibility.
  The default is `.` (current directory), so bare `mtrack start` works as a zero-config entry point.
- **Song upload API**: New REST endpoints for uploading track files to songs:
  - `PUT /api/songs/{name}/tracks/{filename}` — upload a single file (binary body)
  - `POST /api/songs/{name}/tracks` — upload multiple files via multipart form
  - Uploading to a new song name auto-creates the song directory and generates `song.yaml`
  - Subsequent uploads preserve existing `song.yaml` (track names, lighting config, etc.)
- **Song creation API**: `POST /api/songs/{name}` creates a new song with a user-provided
  config YAML. This allows setting track names, lighting shows, and MIDI config before
  uploading any audio files. Returns 409 Conflict if the song already exists.
- **Broader audio format support**: Song auto-discovery and track uploads now accept all
  audio formats supported by symphonia (FLAC, MP3, OGG, AAC, M4A, AIFF) in addition to WAV.
- **Lighting file uploads**: `.light` DSL files can be uploaded alongside audio and MIDI
  files. `Song::initialize()` now auto-discovers `.light` files and includes them in the
  generated `song.yaml`.
- **Hardware profile editor UI**: The web UI config page now includes a full hardware profile
  editor. Profiles can be created, edited, and deleted with dedicated sections for audio
  (device, sample rate, format, buffer size, track mappings), MIDI (device, beat clock),
  DMX (OLA host/port, universe mappings), and controllers (gRPC, OSC). Changes are saved
  with optimistic concurrency and trigger automatic hardware reinitialization.
- **Song browser UI**: A new song management page in the web UI provides:
  - Song list with create/delete
  - Song detail view with track editor (rename tracks, assign lighting shows)
  - Per-track waveform visualization
  - File upload (drag-and-drop or file picker) for audio, MIDI, and lighting files
  - Server-side file browser for importing songs from the local filesystem
  - Bulk song import from a directory
- **Non-blocking hardware initialization**: Audio, MIDI, and DMX devices are now discovered
  asynchronously in the background at startup. The player, web UI, and gRPC server become
  available immediately while hardware init retries perpetually until devices are found.
  This eliminates startup delays when hardware is slow to enumerate or temporarily
  unavailable (e.g. USB devices not yet plugged in).
- **Hardware hot-reload on profile save**: Saving a hardware profile through the config
  editor web UI now automatically reinitializes all hardware from the updated configuration.
  The old hardware is torn down and new devices are discovered asynchronously, just like at
  startup. Hardware reload is rejected during active playback (returns 409 Conflict).
- **Samples configuration UI**: The config editor now includes a Samples section for managing
  the `samples` map in the player configuration. Each sample can be configured with file path,
  output channels, output track, release behavior, retrigger mode, max voices, fade time, and
  velocity settings (ignore/scale/layers with per-layer configuration). Samples are saved with
  the same optimistic concurrency as hardware profiles.
- **Sample file upload and browse**: Sample audio files can be uploaded via drag-and-drop or
  file picker, or imported from the server filesystem using the file browser. Uploaded files
  are stored in a `samples/` directory alongside the config file. Supports WAV, FLAC, MP3,
  OGG, AAC, M4A, AIFF formats. New REST endpoints: `PUT /api/config/samples` for updating
  sample definitions, `PUT /api/samples/upload/{filename}` for uploading sample files.
- **Trigger configuration UI**: The profile editor now includes a Triggers tab for configuring
  audio and MIDI trigger inputs. Each input can be configured with device, channel, threshold,
  gain, scan/retrigger timing, velocity curve, sample assignment, and release groups. Audio
  inputs support calibration directly from the UI.
- **Lighting configuration UI**: Fixture types and venues can now be created, edited, renamed,
  and deleted directly from the web UI without a text editor. The profile editor's Lighting tab
  provides three sub-views:
  - **Fixture Types**: Visual editor for channel maps (name + DMX offset) and strobe settings
  - **Venues**: Visual editor for fixtures (type, universe, channel, tags) with fixture type
    dropdowns populated from loaded definitions
  - **Profile Settings**: Directory overrides, current venue selection, inline fixtures, and
    logical group configuration with constraint editors (AllOf, AnyOf, Prefer, MinCount,
    MaxCount, FallbackTo, AllowEmpty)
- **Lighting REST API**: Ten new endpoints for fixture type and venue CRUD:
  `GET/PUT/DELETE /api/lighting/fixture-types/{name}`,
  `GET /api/lighting/fixture-types`,
  `GET/PUT/DELETE /api/lighting/venues/{name}`,
  `GET /api/lighting/venues`. All endpoints accept an optional `?dir=` parameter for
  directory overrides. PUT endpoints accept structured JSON (converted to `.light` DSL) or
  raw DSL text, and validate by parsing before writing.
- **Tag input component**: A reusable chip-style tag editor used in venue fixture tags and
  lighting group constraints. Supports typing (Enter/comma to add), backspace to remove,
  paste with auto-split, and sanitizes input to `[a-z0-9_-]` to prevent syntax errors.
- **Profile editor tab layout**: The hardware profile editor now uses horizontal tabs
  (Audio, MIDI, DMX, Lighting, Triggers, Controllers) instead of stacked collapsible
  sections, reducing visual clutter. Each tab shows a green dot when that section is enabled.
- **Lighting timeline editor**: A DAW-style visual editor for lighting cue authoring,
  replacing the previous form-based editor. The editor is song-driven — the left panel lists
  songs, and selecting one loads its waveform and all associated `.light` files into a
  horizontal scrollable timeline. Features include:
  - Time ruler with absolute timestamps and measure/beat grid (when tempo is defined)
  - Song waveform reference track
  - Per-show cue lanes with color-coded draggable cue blocks
  - Click to select, double-click to add, drag to reposition with snap-to-grid
  - Compact single-line effect display that expands for full editing
  - Sequence editing in a dedicated modal with its own timeline
  - Legacy MIDI DMX file management (upload and import from server filesystem)
  - Zoom, fit-to-view, and beat/measure snap controls
- **Song file import API**: New `POST /api/songs/{name}/import` endpoint copies a file from
  the server filesystem into a song directory. Source paths are validated against the project
  root to prevent path traversal.
- **Song creation UI**: Songs can now be created from the web UI song browser with a name
  and optional configuration.
- **Multiple playlist support**: The player now supports multiple user-defined playlists
  stored as individual YAML files in a `playlists/` directory (default: `{config_dir}/playlists/`).
  Playlists are named after their filename stem. The `all_songs` playlist remains always
  present and auto-generated. Switching to `all_songs` is session-only (not persisted across
  restarts), acting as a temporary escape hatch. The persisted active playlist is stored in
  `mtrack.yaml` via the `active_playlist` field (defaults to `"playlist"` for backward
  compatibility). MIDI/OSC `Playlist` action returns to the persisted active playlist,
  regardless of its name.
- **Playlist REST API**: Five new endpoints for playlist CRUD:
  `GET /api/playlists` (list all with song count and active status),
  `GET /api/playlists/{name}` (songs + available songs),
  `PUT /api/playlists/{name}` (create or update),
  `DELETE /api/playlists/{name}` (delete, refuses `all_songs`),
  `POST /api/playlists/{name}/activate` (switch active playlist).
  Legacy `/api/playlist` GET/PUT endpoints remain as backward-compatible aliases.
- **Playlist editor UI**: The web UI playlist page is now a full editor with a left panel
  for browsing, creating, and deleting playlists, and a right panel for editing song order
  (reorder, add, remove) with a searchable available-songs list. Playlists can be activated
  directly from the editor.
- **Dashboard playlist selector**: The dashboard playlist card now shows a dropdown of all
  available playlists instead of a hardcoded Playlist/All Songs toggle. The available
  playlists and active playlist name are broadcast via WebSocket.
- **Timeline playback from editor**: The lighting timeline editor now supports full audio
  playback with synchronized lighting preview. Users can click the ruler to set a play
  position, then play from that point to hear audio and see lighting effects update in
  real time. Features include:
  - Full transport controls: skip to start, stop (reset), play/pause toggle, skip to end
  - Spacebar play/pause toggle, Home/End keyboard shortcuts for skip
  - Green playhead line across ruler and all show lanes during playback
  - Dashed green cursor marker showing where playback will start
  - Pause remembers playhead position for resume; Stop resets to beginning
  - Auto-save of dirty lighting files before playback starts
  - Client-side `requestAnimationFrame` interpolation for smooth playhead movement
  - Toolbar time display: green during playback, dimmed at play cursor position
- **Stage preview in timeline editor**: A compact stage visualization is displayed alongside
  the cue properties panel at the bottom of the timeline editor. Shows real-time fixture
  RGB output with glow and strobe animation, plus active effect names. Fixtures can be
  rearranged by dragging, same as the dashboard stage view.
- **`PlaySongFrom` gRPC endpoint**: New `PlaySongFrom` RPC accepts a song name and start
  time, switches to the `all_songs` playlist, navigates to the song, and begins playback.
  This enables the timeline editor to play any song from any position in a single call.
- **`Playlist::navigate_to(name)`**: New method sets the playlist position to the song
  matching the given name, returning the song if found.
- **Bulk song import**: New `POST /api/browse/bulk-import` endpoint recursively scans
  subdirectories of a given path and creates `song.yaml` in each one that contains
  audio files and doesn't already have a `song.yaml`. Nested structures (artist/album/song)
  are handled automatically. The song browser's import UI shows an "Import All
  Subdirectories" button with a results summary showing created, skipped, and failed imports.
- **Song deletion**: New `DELETE /api/songs/{name}` endpoint removes a song by deleting its
  `song.yaml`. Audio and other files are preserved. The song is automatically removed from
  any playlists that reference it. Cannot delete a song that is currently playing.
- **Nested song creation**: `POST /api/songs/{name}` now accepts paths with slashes
  (e.g. `Artist/Album/Song`), creating nested directory structures automatically.
- **Lock mode**: The player starts in locked mode by default, blocking all state-altering
  operations (song edits, playlist changes, config updates, file uploads) via a middleware
  layer. Playback controls always work. Toggle via `PUT /api/lock` or the lock icon in
  the web UI nav bar. Lock state is broadcast via WebSocket.
- **SafePath module**: Centralized path verification (`src/webui/safe_path.rs`) with
  `SafePath`, `VerifiedRoot`, and `SafePathError` types. All REST API handlers that touch
  the filesystem now use SafePath for canonicalize + starts_with containment verification,
  replacing bespoke per-handler implementations.

### Changed

- **Binary uses library crate**: `main.rs` now imports from `mtrack::` instead of
  re-declaring all modules. This eliminates double-compilation and ensures the binary
  runs the same code that tests verify.
- **Song path resolution via registry**: `put_song`, `upload_track_single`,
  `upload_tracks_multipart`, and `import_file_to_song` now look up the song's actual
  directory from the player registry before falling back to a flat `songs_path/name` join.
  This correctly handles songs in nested subdirectories.
- **Zero-config songs path**: Fresh `mtrack.yaml` files created during zero-config startup
  now default to `songs: .` (project root) instead of conditionally choosing between
  `songs` and `.`. This ensures bulk-imported songs are always discoverable.

- `Playlist::current()`, `next()`, and `prev()` now return `Option<Arc<Song>>` instead of
  `Arc<Song>`, returning `None` when the playlist is empty. All callers (player, controllers,
  web UI, TUI) handle the empty case gracefully — controllers return appropriate error statuses,
  fire-and-forget handlers skip silently, and the web UI sends minimal "no song" state.
- The `start` command no longer requires a playlist file to be specified. When absent, it falls
  back to an all-songs playlist.
- The `start` command's positional argument has been renamed from `player_path` to `path` and
  now defaults to `.` (current directory). Existing usage with a config file path continues to
  work unchanged.
- `PUT /api/songs/{name}` now locates songs by directory name rather than requiring audio files
  to be present. Songs created via `POST /api/songs/{name}` (config-only, no audio yet) can
  be updated immediately.
- The systemd service template now uses `$MTRACK_PATH` instead of `$MTRACK_CONFIG` to reflect
  the new directory-or-file semantics.
- The `Player` struct now uses a `HashMap<String, Arc<Playlist>>` internally instead of
  separate `playlist` / `all_songs` / `use_all_songs` fields. `switch_to_playlist()` accepts
  any playlist name (not just `"playlist"` or `"all_songs"`). The gRPC `SwitchToPlaylist` RPC
  accepts arbitrary playlist names — invalid names return `NOT_FOUND` instead of
  `UNIMPLEMENTED`.

- **Mutable configuration store**: A new `ConfigStore` wraps the player configuration in a
  `tokio::sync::RwLock`, enabling runtime config mutations with optimistic concurrency control
  via whole-config checksums. Mutations are persisted atomically to the YAML config file on
  every change. The store is accessible from the `Player` via `player.config_store()`.
- **Config gRPC RPCs**: Eight new RPCs on `PlayerService` for reading and mutating configuration
  at runtime: `GetConfig`, `UpdateAudio`, `UpdateMidi`, `UpdateDmx`, `UpdateControllers`,
  `AddProfile`, `UpdateProfile`, `RemoveProfile`. Stale checksums return `FAILED_PRECONDITION`.
- **Config REST endpoints**: New REST API endpoints for config mutations:
  `GET /api/config/store`, `PUT /api/config/{audio,midi,dmx,controllers}`,
  `POST /api/config/profiles`, `PUT /api/config/profiles/:index`,
  `DELETE /api/config/profiles/:index`. Stale checksums return 409 Conflict.
- **Config change notifications**: WebSocket clients receive `config_changed` messages when
  the configuration is mutated, enabling real-time UI updates without polling.
- **Config round-trip test**: A serialize/deserialize round-trip test validates that
  `config::Player` survives YAML serialization via `yaml-rust2` without data loss.
- Config store checksums use SHA-256 (via `sha2` crate) instead of hand-rolled FNV-1a for
  clarity and industry-standard collision resistance.
- `config::Player` and `config::TrackMappings` now derive `Clone`, enabling the config store
  to snapshot the full configuration.
- `config::Player` gained setter methods (`set_audio`, `set_midi`, `set_dmx`,
  `set_controllers`, `profiles_mut`) for structured config mutations.

### Fixed

- **Playback elapsed time accuracy**: `play_start_time` is now set inside `play_files`
  immediately after `clock.start()`, rather than in `play_from` before subsystem setup.
  Previously, the variable setup time (loading audio buffers, seeking, DMX timeline
  reconstruction) was included in the reported elapsed time, causing lighting and audio
  to drift apart when seeking to mid-song positions.
- **Timeline zoom stability**: Toolbar zoom (+/- buttons) now correctly anchors on the
  center of the content area, accounting for the 80px label column. Previously, each zoom
  step shifted the center by a fraction of 40px, causing compounding drift that could move
  the view by many measures when zooming deeply.
- **Timeline zoom during rapid scrolling**: Zoom operations now read `scrollLeft` directly
  from the DOM and suppress the RAF-debounced scroll handler during zoom, preventing stale
  values from corrupting the anchor point during rapid zoom-in/out.
- **Playback status reporting**: OSC broadcast and gRPC Status RPC now use
  `player.is_playing()` (join handle check) instead of `elapsed().is_some()` to determine
  playing state. This prevents a brief false "Stopped" report during the startup window
  between `play_from` returning and `clock.start()` firing.
- **Play rejected during initialization**: `play_from()` now returns an error if hardware
  hasn't finished initializing, preventing silent no-output playback.
- **PlaySongFrom playlist switch is session-only**: The editor's play-from-position switches
  to `all_songs` for the duration of playback (required for correct WebSocket state
  broadcasting) but the switch is not persisted — the user's real playlist is restored
  on restart.
- **Playhead interpolation drift cap**: Client-side playhead interpolation is capped at 2
  seconds of drift, preventing large backward jumps when the WebSocket reconnects after a
  long disconnect.
- **Deleting playing song blocked**: `DELETE /api/songs/{name}` returns 409 Conflict if the
  song is currently playing.
- **Playlist cleanup on song deletion**: When a song is deleted, all playlist YAML files are
  updated to remove the deleted song, preventing broken references on restart.
- **DSL serializer trailing commas**: The lighting DSL serializer no longer emits trailing
  commas in tempo change lists, which caused parse errors on re-read.
- **DSL serializer empty groups**: Effects with empty group names are skipped during
  serialization instead of producing invalid DSL like `": static"`.
- **DSL serializer dimmer separator**: All effect types now use comma-separated parameters.
  Previously, dimmer effects incorrectly used space separation.
- **Default effect group**: New effects created from the timeline editor default to group
  "all" instead of an empty string.
- **Upload error visibility**: MIDI upload error messages are now displayed on the MIDI tab
  (previously only shown on the Tracks tab).
- **Upload body size limit removed**: The axum default 2MB body limit is disabled for API
  routes, allowing upload of large audio files.
- **File replacement confirmation**: Uploading a file that already exists prompts for
  confirmation and shows "Replaced" instead of "Uploaded" in the success message.
- **WebSocket disconnect indicator**: The lighting editor shows a yellow warning banner when
  the WebSocket connection is lost.
- **Chase direction/pattern dropdowns**: Chase effect direction uses the correct spatial
  directions (not "forward"/"backward"), and pattern is now a dropdown instead of free text.
- **Effect group dropdown**: The effect group field is now a text input with a datalist
  populated from the venue's fixture groups, supporting both selection and free-text entry.
- **Sample upload path injection**: The sample file upload endpoint now canonicalizes the
  project root before constructing filesystem paths, preventing path traversal via crafted
  filenames.
- Web UI asset tests now discover embedded filenames dynamically instead of hardcoding
  hashed filenames that change with every Svelte build.

## [0.10.2] - 2026-03-14

### Fixed

- **Songs without MIDI freeze playback**: When a MIDI device was configured but a song had no
  MIDI file, the MIDI subsystem returned without signaling readiness, preventing the playback
  clock from starting. All subsystems (audio, DMX, MIDI) would spin indefinitely waiting for
  the clock, resulting in no audio or lighting output.

## [0.10.1] - 2026-03-11

### Fixed

- **`cargo publish` includes web UI assets**: The published crate now includes the pre-built
  Svelte frontend so that `cargo install mtrack` works without Node.js. The build script no
  longer creates files in the source tree during packaging.
- **Publish workflow builds frontend**: The GitHub Actions publish workflow now builds the
  Svelte frontend before `cargo publish`, using devbox for consistent tooling.
- **Remote client tests no longer flaky**: gRPC remote client tests now use OS-assigned
  ephemeral ports instead of the default gRPC port, so they pass even when a local mtrack
  server is running.

## [0.10.0] - 2026-03-11

### Added

- **Web UI**: A new web-based interface for controlling and monitoring mtrack from a browser.
  The web UI is always available when running `mtrack start`, served on `http://0.0.0.0:8080`
  by default. Use `--web-port` and `--web-address` to customize. Features include:
  - Real-time playback control (play/stop/next/prev) with progress bar
  - Playlist management with song selection
  - Per-track waveform visualization
  - Interactive stage view with fixture positions and real-time DMX state
  - Active effects display
  - Streaming log panel
  - REST API for config, playlist, song, and lighting file management
  - WebSocket streaming for real-time updates
  - The lighting simulator is now integrated into the web UI (replaces the previous
    `--simulator` / `--simulator-port` flags)
- **MIDI beat clock output**: mtrack can now send MIDI beat clock (24 ppqn) to synchronize
  external gear to song tempo. Enable with `beat_clock: true` in the MIDI configuration. When
  enabled, mtrack sends Start (0xFA), Timing Clock (0xF8), and Stop (0xFC) messages derived
  from the MIDI file's tempo map. Beat clock is only emitted for songs whose MIDI files contain
  explicit tempo change events — songs without a tempo map do not emit beat clock, leaving
  musicians free to control their own tempo. The beat clock runs on a dedicated real-time
  priority thread with `spin_sleep` precision and supports mid-song tempo changes and seeking
  (Continue 0xFB). Thread priority can be tuned with `MTRACK_THREAD_PRIORITY` (0–99, default
  70) or disabled with `MTRACK_DISABLE_RT_AUDIO=1`.
- **Unified playback clock**: A new `PlaybackClock` abstraction synchronizes MIDI and DMX
  timing to the audio interface's hardware sample counter, eliminating drift between subsystems
  over long songs. When no audio device is present, the clock falls back to `Instant::now()`
  (system monotonic clock) with no behavioral change. The clock is always passed through to
  MIDI and DMX regardless of audio presence.
- **Makefile**: A new Makefile provides convenient build targets for the full project
  (Rust + Svelte frontend), including `build`, `test`, `lint`, `fmt`, `check`, `dev-ui`,
  and `clean`.

### Changed

- **Replaced barrier synchronization with clock-based coordination**: Playback subsystems
  (audio, MIDI, DMX) now signal readiness via channels and wait for the `PlaybackClock` to
  start as the "go" signal, replacing the counted `Barrier` mechanism. This eliminates the
  fragile barrier count computation, removes dummy barrier threads from the DMX engine, and
  simplifies the overall synchronization model.
- **TUI is now opt-in**: The terminal UI is no longer launched automatically when stdin is a
  TTY. Use `--tui` to enable it. The `--no-tui` flag has been removed.
- **Waveform generation speedup**: Waveform peak data generation for the web UI is now
  significantly faster.

### Fixed

- **MacOS build fixes**: Resolved build issues on macOS.
- **MacOS SCHED_FIFO thread priority**: Added clamping of the thread priority to the valid
  macOS SCHED_FIFO range (15–47) so the default of 70 works on both platforms. SCHED_FIFO
  failures on macOS CoreAudio threads are now logged at debug level since these threads
  already use Mach real-time scheduling.

### Internal

- **Massive test expansion**: Comprehensive test coverage added across all modules including
  player, audio mixer, sample engine, DMX engine, MIDI, controllers (gRPC, OSC, MIDI), TUI,
  web UI, triggers, config parsing, and CLI.
- **Thread priority module**: Extracted `thread_priority` from `audio/` into a shared
  top-level module, reused by both the audio callback and MIDI beat clock threads.
- **Documentation**: Migrated README content into an mdBook documentation site for easier
  maintenance and hosting.
- **CI improvements**: cargo tarpaulin now uses a separate cache to avoid conflicts with
  regular builds.

## [0.9.2] - 2026-03-01

### Fixed

- **Buffered audio near-livelock**: Fixed a near-livelock in `BufferedSampleSource` that could
  cause audio playback to degrade to sparse blips (~3% real-time speed) while lighting continued
  normally. When the ring buffer underran, the audio callback would acquire the inner source mutex
  while holding the buffer state mutex, starving the background fill task and creating a
  self-reinforcing stall. The audio callback no longer acquires the inner source mutex; underruns
  produce brief silence while the fill task catches up. A new `is_exhausted()` trait method lets
  the mixer distinguish transient buffer underruns from true end-of-source.

## [0.9.1] - 2026-02-28

### Fixed

- **DMX universe deadlock**: Fixed an ABBA deadlock between the effects loop and universe
  threads caused by reversed lock ordering of the `target` and `rates` RwLocks in
  `approach_target()`. This could cause lighting to freeze mid-song, after which audio
  playback and lighting would no longer function until restart. The deadlock was timing-
  dependent and could occur on any song type (legacy MIDI or DSL).

- **Effects loop resilience**: The persistent effects loop is now wrapped in `catch_unwind`
  so that a panic in a single tick cannot kill the loop thread. Panics are logged and the
  loop continues on the next tick.

- **Effects loop heartbeat monitoring**: Barrier threads now monitor the effects loop via an
  atomic heartbeat counter instead of waiting indefinitely. If the heartbeat is stale for 10
  seconds, the timeline is force-finished and the song completes gracefully rather than
  hanging forever.

- **Lost condvar notifications**: `CancelHandle::notify()` now acquires the mutex before
  signaling, preventing a race where notifications could be lost between a waiter's predicate
  check and its condvar sleep.

- **Blocking mutexes in async code**: The state sampler and TUI tick now acquire
  `parking_lot::Mutex` locks via `spawn_blocking` instead of blocking the tokio runtime
  directly. The TUI log buffer was also migrated from `std::sync::Mutex` to
  `parking_lot::Mutex` for consistency with the rest of the codebase.

## [0.9.0] - 2026-02-26

### Added

- **Audio trigger input**: Piezoelectric drum triggers (or any transient audio signal) can now
  trigger sample playback via a standard audio interface input. Configure per-channel threshold,
  scan window, retrigger lockout, gain, and velocity curve (linear, logarithmic, or fixed).
  Trigger inputs can fire samples or release voice groups (e.g. cymbal choke). No MIDI hardware
  required — plug piezos directly into any cpal-supported audio interface. Advanced options
  include a high-pass filter (`highpass_freq`) to reject stage rumble and bass bleed, dynamic
  threshold decay (`dynamic_threshold_decay_ms`) that raises the threshold after a hit to reject
  ringing, and crosstalk suppression (`crosstalk_window_ms` / `crosstalk_threshold`) to prevent
  a single hit from firing multiple channels.

- **Unified trigger config with `kind` discriminator**: Audio and MIDI trigger inputs now coexist
  in a single `trigger.inputs` list using a `kind` field (`audio` or `midi`). MIDI triggers
  defined this way replace the top-level `sample_triggers` section. The `device` field is now
  optional (only required when audio inputs are present). Legacy `sample_triggers` are
  automatically converted to `kind: midi` trigger inputs at startup.

- **Trigger config in profiles**: The `trigger` section can now be placed inside a hardware
  profile, alongside `audio`, `midi`, and `dmx`. This allows different hosts to use different
  trigger configurations. Top-level `trigger` is still supported and automatically normalized
  into the profile.

- **`calibrate-triggers` CLI command**: A new `mtrack calibrate-triggers <device>` command
  measures the noise floor and hit characteristics of a connected audio input device, then
  generates a ready-to-paste YAML trigger configuration with calibrated thresholds, scan windows,
  retrigger lockout times, and optional high-pass filter settings.

- **`output_track` for samples**: Sample definitions now support an `output_track` field that
  references a track mapping name from the active hardware profile's `track_mappings`. This allows
  a single sample definition to work across different hardware profiles with different channel
  assignments. The existing `output_channels` field continues to work unchanged. If both are set,
  `output_track` takes precedence.

- **Lighting simulator**: A new web-based UI for visualizing lighting effects in real-time without
  physical DMX hardware. Start with `mtrack start --simulator` (optionally `--simulator-port`).
  The simulator shows per-fixture color/strobe state, processes layer commands, and supports
  hot-reload of `.light` DSL files during playback. When the OLA daemon is unavailable, the DMX
  engine falls back to a null client so the effects engine can still run.

- **Legacy MIDI-to-DMX integration with effect engine**: Legacy MIDI light shows now feed their
  interpolated DMX values into the DSL effect engine via a lockless atomic store
  (`LegacyDmxStore`). This allows MIDI-driven fixtures to appear in the lighting simulator and
  coexist with DSL effects on the same universes. DSL effects take priority over legacy values.

- **Fixture strobe frequency range**: Fixture type definitions now support `min_strobe_frequency`
  and `strobe_dmx_offset` fields, allowing accurate strobe DMX calculation for fixtures whose
  strobe channel starts above DMX value 0 (e.g. Astera PixelBrick: offset 7, range 0.4–25 Hz).

- **Configurable FFT resampling**: A new `resampler` option in the audio configuration allows
  choosing between `sinc` (default, high-quality sinc interpolation) and `fft` (FFT-based
  resampling, considerably faster for fixed-ratio resampling). The FFT mode can significantly
  reduce CPU usage on low-power hardware like the Raspberry Pi when source and output sample
  rates differ. Set `resampler: fft` in the audio config to enable it.

- **Terminal UI (TUI)**: When `mtrack start` is run from an interactive terminal, it now
  launches a full-screen terminal UI built with ratatui/crossterm. The TUI shows the playlist,
  now-playing status with a progress bar, real-time fixture colors from the lighting engine,
  active effects, and an inline log panel. Keyboard shortcuts provide play/stop, prev/next,
  playlist switching, and quit. When stdin is not a TTY (e.g. systemd, pipes), mtrack runs in
  headless mode as before. Use `--no-tui` to force headless mode from an interactive terminal.
  All configured controllers (gRPC, OSC, MIDI) continue to run alongside the TUI.

### Changed

- **Custom MIDI playback engine replaces nodi**: The `nodi` dependency has been removed and replaced
  with a bespoke MIDI playback engine built on `midly`. MIDI events are pre-computed into absolute-
  timestamped event streams in a single pass, with wall-clock timing via `spin_sleep` for precise
  hardware MIDI output. Legacy MIDI-to-DMX light shows are now driven from the effects loop using
  cursor-based playback rather than spawning separate threads with nodi players, reducing thread
  count and improving timing consistency.

- **Strobe DMX normalization uses period-linear interpolation**: Strobe frequency-to-DMX mapping
  now interpolates in period-space (1/frequency) rather than frequency-space, matching how most
  LED fixtures actually scale their strobe channel. This produces significantly higher DMX values
  for high strobe frequencies (e.g. 10 Hz on a PixelBrick now sends DMX 248 instead of 103).

- **Sequence tempo rescaling**: Sequences referenced from shows with different tempos now have
  their cue timing correctly rescaled from the sequence's internal BPM to the expansion-point
  tempo. Previously, sequences always played at their own BPM regardless of the calling context.

- **`note_off` renamed to `release_behavior`**: The sample configuration field `note_off` has been
  renamed to `release_behavior` to better reflect its source-agnostic purpose. The old `note_off`
  key is still accepted for backwards compatibility.

- **Legacy field warnings**: When `profiles` is present in the player config, any top-level
  `audio`, `midi`, `dmx`, `trigger`, `track_mappings`, or `sample_triggers` fields now produce
  a warning that they are being ignored, making misconfiguration easier to diagnose.

- **Shared state sampler**: Lighting fixture state and active effect information are now
  broadcast via a `watch` channel from a shared 20 Hz sampler (`state.rs`). Both the TUI and
  the lighting simulator subscribe to the same channel, replacing the simulator's previous
  dedicated polling. This reduces lock contention on the effect engine.

### Fixed

- **Looped sequence effects compounding**: Fixed a bug where effects in looped lighting sequences
  would accumulate across iterations (e.g. multiply chases compounding). Each loop iteration now
  stops the previous iteration's effects before starting its own.

- **Lighting simulator layer commands**: The lighting simulator now processes layer commands
  (clear, release, freeze, unfreeze, master) and sequence stop commands, matching the behavior of
  the player's DMX engine.

- **Seeking past layer clears**: When using play-from to seek into a song, layer `clear()`
  commands in the timeline history now correctly purge effects that were started before the clear.
  Previously, seeking would accumulate all perpetual effects from the entire history, ignoring
  intermediate clears.

- **OLA connection resilience**: The OLA client now supports automatic reconnection after
  connection failures, with backoff retry logic. A single failed DMX send no longer drops the
  entire connection. The DMX engine also avoids inadvertently starting the OLA daemon.

- **Stale lighting state between songs**: Fixed a bug where DSL songs without tempo blocks would
  inherit stale tempo maps from previous songs. Tempo maps, timelines, and legacy MIDI values are
  now properly cleared on song transitions.

- **DMX timeline completion race**: Fixed a race condition where the effects loop could mark the
  timeline as finished before the first song had set up its lighting state. The `timeline_finished`
  flag now initializes to `true` and is only reset when a song begins, preventing premature
  completion signals.

## [0.8.0] - 2026-02-20

### Added

MIDI-triggered sample playback has been added. Samples can be configured globally or per-song
with the following features:

- **Velocity handling**: Configurable velocity sensitivity with optional fixed velocity override
- **Note Off behavior**: Samples can play to completion, stop immediately, or fade out on Note Off
- **Retrigger behavior**: Polyphonic mode allows layering, cut mode stops previous instances
- **Voice limits**: Global and per-sample voice limits with oldest-voice stealing
- **Output routing**: Samples can be routed to specific output channels via track mappings
- **In-memory preloading**: Samples are decoded and cached in memory for low-latency playback

Sample triggering uses a fixed-latency scheduling system that ensures consistent trigger-to-audio
latency with zero jitter. At 256 sample buffer size (44.1kHz), latency is approximately 11.6ms
(~5.8ms scheduled delay + ~5.8ms output buffer). Cut transitions are sample-accurate - old
samples stop at exactly the same sample the new one starts, eliminating gaps.

The audio engine has been refactored for lower latency and stability:

Audio is now fully optional. mtrack can run as a pure MIDI or DMX player without any
audio device configured. This enables dedicated lighting-only or MIDI-only nodes.

A warning is displayed when playing songs that have tracks that are not configured to
be output in the current player configuration. This will not cause an error, but will be
logged so that it's easier to diagnose a misconfiguration.

Hardware profiles allow multiple complete host configurations in a single config file:

- **Unified profiles** (`profiles`): Define complete per-host configurations in a single list.
  Each profile specifies audio (optional), MIDI (optional), and DMX (optional) for one host.
  All three subsystems are optional — a profile can define any combination (e.g. MIDI-only,
  DMX-only, audio + MIDI, etc.). Subsystem presence in profile = required, absence = skipped.
- **Hostname filtering**: Each profile can optionally specify a `hostname` constraint so that
  different hosts sharing the same config file use different devices and channel mappings.
  Set the `MTRACK_HOSTNAME` environment variable to override the system hostname.
- **Per-host optionality**: All subsystems (audio, MIDI, DMX) can be required on some hosts
  (present in profile) and optional on others (absent from profile), enabling flexible
  multi-host setups such as dedicated lighting-only or MIDI-only nodes.
- **External profiles directory** (`profiles_dir`): Load profiles from individual YAML files in a
  directory instead of (or in addition to) defining them inline. Each file defines one profile.
  Files are sorted by filename for deterministic ordering. Directory profiles are prepended
  before inline profiles, making them useful for host-specific configs with inline fallbacks.
- **Backwards compatible**: Existing configs using `audio:`, `midi:`, `dmx:`, and `track_mappings:`
  are automatically normalized into a single profile at startup.

- **Direct callback mode**: The CPAL callback now calls the mixer directly, eliminating the
  intermediate ring buffer. This follows the pattern used by professional audio systems
  (ASIO, CoreAudio, JACK) for lowest possible latency.
- Lock-free voice cancellation using atomic flags
- Channel-based source addition to decouple sample engine from mixer locks
- Inline cleanup of finished sources during mixing (simpler, no separate cleanup pass)
- Bounded source channel (capacity 64) to prevent unbounded memory growth
- Precomputed channel mappings at sample load time (no allocations during trigger)
- Song playback is buffered to reduce buffer underrun.
- **Audio stress test** (`examples/audio_stress`): A standalone stress test binary for
  verifying real-time audio performance against real hardware. Sweeps buffer sizes, sample
  rates, and formats; ramps source counts; and churns source creation/destruction. Reports
  pass/fail based on callback budget headroom. Useful for validating hardware configurations
  before live use.

### Changed

- **cpal 0.15.3 to 0.17.1**: Updated for improved ALSA handling. (Breaking) Device names
  may have changed. Run `mtrack devices` to check.
- **Symphonia replaces hound**: Supports flac, ogg, vorbis, mp3, alac, aac, and anything
  else Symphonia supports, not just wav.
- **Security**: mtrack now supports running as a dedicated `mtrack` user instead of root
  via systemd, with updated service definitions and documentation.
- **Error handling**: Improved error reporting and panic aversion across the player, DMX
  engine, sample loader, and playlist handling.
- **Lighting cleanup**: A pass of cleanup on the lighting system, improving code quality
  across the effect engine, parser, timeline, and layering.

### Fixed

- **Stop during playback**: Fixed a bug where stopping too fast after playing could produce
  a hang. This is unlikely to have happened in a live scenario.
- **Clean device shutdown**: The CPAL output stream now shuts down promptly when the device
  is dropped, fixing a bug where the stream thread would block indefinitely on join.
- **Lighting bug fixes**: Fixed bugs in the lighting system discovered during cleanup.

## [0.7.0]

A major resampling bug discovered in 0.6.0 was fixed, apologies to everyone who encountered
it. 0.6.0 was pulled for this.

A new lighting engine has been added that mimics grandMA style lighting consoles. There are
multiple layers and multiple effects. It exists entirely differently from the MIDI to DMX
conversion that was written before, but it lives along side the previous functionality.
In general, I found the MIDI to DMX engine to be too cumbersome so I'm hoping this new
engine is easier to deal with. It has its own DSL as well. Feedback on all of this would be
appreciated.

The play-direct command was removed, as I think it's outlived its purpose, which was largely
for testing.

OSC commands now broadcast back to any connecting clients. Right now these clients are never
forgotten, so it's possible to DDoS mtrack, so be responsible with your network security!

## [0.6.0]

**Note**: You can now explicitly specify the sample rate, sample format, and bits per
sample for your audio device. This may be a breaking change depending on the audio files
you have been using. Please test before playing live!

Transcoding has been added to mtrack. This allows audio files and audio devices to have
mismatched sample rates and formats, making it easier to deal with files from multiple
sources. This also adds the ability to configure the target output format for an audio
device.

A new way of reading files has been added: SampleSources, which will hopefully allow us
to build different types of input sources in the future. This will make it easier to
introduce other input file types like FLAC, Ogg, MP3, etc. Along with this was a good
bit of performance tuning. The transcoding introduces increased performance requirements,
but this was reduced as much as possible.

Finally, audio is now played in a constant stream as opposed to opening up a new stream
per file. Combined with the transcoding work, this should make CPU usage very consistent.
In my testing, it seems that the biggest performance hit is from libasound's
speex_resampler_process. The rubato method is extremely efficient, so I recommend targeting
your audio device's native sample rate/format.

## [0.5.0]

Initialization of a songs directory is now easier, as mtrack can be given an `--init` flag
when using the `songs` subcommand. This will establish a baseline YAML file in each song
directory.

Live MIDI can be routed to the DMX engine. This will allow live controlling of lights through
mtrack.

The playlist can now be specified through the mtrack configuration YAML rather than specified
separately when using the start command.

Simple MIDI to DMX transformations are now supported. This allows for a simple MIDI event to
be expanded into multiple, which can then be fed into the DMX engine. This allows for things
like simple MIDI controllers to control multiple lights.

## [0.4.1]

Fix: Audio interfaces with spaces at the end can now be selected.

## [0.4.0] - Refactoring of config parsing, gRPC/OSC.

A gRPC server has been added to mtrack along with several utility subcommands that allow
for control of the player from the command line. This should be useful for creating
external player clients.

An OSC server has been added to mtrack. This will allow communication with mtrack over any
OSC protocol (UDP only at the moment). This is handy for using clients like TouchOSC to
control mtrack. This includes reporting, so that OSC clients can display information about
currently playing songs, track durations, etc.

The keyboard controller has been fixed -- it wasn't trimming off the newlines at the end
of keyboard input.

(Breaking Change) The `play` subcommand has been renamed `play-direct` so that the `play`
subcommand could be used to control the gRPC server.

(Breaking Change) mtrack no longer supports multiple song definitions in one file. This
is because mtrack has shed `serde_yaml`, which has been deprecated, and now uses `config-rs`
to parse config files, and config-rs does not support YAML documents in one file.
Other than lessening the maintenance burden, one advantage to doing this is that mtrack
can now support multiple file types. As of the time of writing, this includes:

- JSON
- TOML
- YAML
- INI
- RON
- JSON5

Note that I still personally test with YAML, so I haven't had an opportunity to exercise
all of the different file types.

## [0.3.0] - Configurable playback delays and refactoring.

Configurable playback delays have been added for audio, MIDI, and DMX playback.

DMX will now only use one OLA client instead of one per universe.

A fairly large refactor has been done to the config logic. The motivation is to
keep (most) of the instantiation of the various pieces of business logic out of
the config package while simultaneously trying to simplify the configuration of
the player and its various components.

Finally, the nodi dependency was updated. This is of interested as it has a corrected
timer, so mtrack has been updated to use it. Initial testing seems to indicate
that it works well.

## [0.2.1] - Repeatedly attempt to connect to OLA on startup.

Repeatedly attempt to connect to OLA, which should make connecting to DMX on startup
more reliable.

## [0.2.0] - Fix hard coded DMX universe.

The DMX universe was inadvertently hard coded to OLA universe 1. This
has been corrected.

## [0.1.9] - Better cancellation.

The expiration mechanism for cancellation had an unintended side effect
of preventing cancellation if one component of the song completed ahead
of time. In other words, if a MIDI file finished playing but there is
still audio to play, the song is no longer cancellable. Additionally, it
would be possible, in some circumstances, for the completion of one
aspect of a song to cancel others unexpectedly.

The "expiration" concept was introduced to allow cancellation while
still allowing a song to finish normally. This has been replaced with a
simple concept of an atomic bool that indicates whether a song component
(MIDI, DMX, or audio) has finished and, when used in combination with
the new "notify" function, will allow a cancel\_handle.wait() call to
return without an actual cancellation happening.

## [0.1.8] - Better MacOS support.

MacOS support is improved. It's not super thoroughly tested, but has been tested
against several audio interfaces.

`mtrack -V` will report the correct mtrack version.

## [0.1.7] - Fix dependencies.

Dependencies for mtrack have all been updated. This should hopefully resolve the issue
with `cargo install` not working properly.

## [0.1.6] - DMX engine with dimming support, MIDI channel exclusions.

DMX engine using OLA has been added. Contains a built in dimming engine.

Added the ability to exclude MIDI channels from MIDI playback.

## [0.1.5] - Initial DMX engine, fixed MIDI clock timing.

Initial DMX engine implemented. This is not quite ready for prime time.

Fixed the MIDI cancelable clock. Not sure what I was thinking when I implemented that.

## [0.1.4] - Track mapping update, status reporting, stopping fix.

(Breaking Change) Track mappings now support mapping to multiple channels.

Status reporting is now configurable.

Address stopping not working for songs with sparse MIDI files.

## [0.1.3] - MIDI tuning, accuracy.

MIDI playback is now more accurate and has been tuned to be more in time with audio
playback.

## [0.1.2] - Player level channel mapping, merging of channels.

Channel mappings have been removed from individual song files and will now be
maintained as part of the player configuration.

Channels can now be merged. That is, tracks can target the same output channel.

## [0.1.1] - Minor dependency update.

Removal of unneeded ringbuffer dependency.

## [0.1.0] - Initial release.

### Added

- Initial release.
