#!/usr/bin/env python3
# Copyright (C) 2026 Michael Wilson <mike@mdwn.dev>
#
# This program is free software: you can redistribute it and/or modify it under
# the terms of the GNU General Public License as published by the Free Software
# Foundation, version 3.
#
# This program is distributed in the hope that it will be useful, but WITHOUT
# ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS
# FOR A PARTICULAR PURPOSE. See the GNU General Public License for more details.
#
# You should have received a copy of the GNU General Public License along with
# this program. If not, see <https://www.gnu.org/licenses/>.
"""Negative-control sweep: prove each check is capable of failing.

Run from anywhere:  python3 harness/negative-control.py [check-name-substring]

A check that cannot fail is worse than no check, and this harness has shipped
two of them: `bogus_*_device_degrades_gracefully` read a status that was
"initializing" whatever mtrack did, and `active_playlist_persists_across_restart`
asserted the opposite of documented behaviour. Both survived multiple reviews.
This sweep is how that class of defect gets caught mechanically.

For each entry it breaks something the check depends on, runs that check alone,
and requires the outcome to stop being PASS. Two strengths are distinguished:

  world      the *input* is broken, so the check must notice a changed reality.
             Strong evidence: it exercises the observable the check reads.
  predicate  the assertion itself is inverted. Proves the assertion is reached
             and its message is produced, but NOT that the observable it reads
             is sensitive to real misbehaviour. A vacuous check passes this.

Replacements are scoped to the function's own span, because most anchors repeat
across checks.
"""
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CHECKS = ROOT / "harness/src/checks"

# (check, file, find, replace, expect-in-failure, kind)
SWEEP = [
    ("malformed_show_is_rejected", "lighting.rs",
     'let broken = "show \\"Broken\\" { @00:00.000 this is not a valid effect line ".to_string();',
     'let broken = LightingSpec::simple("V", "v.light", LIGHTING_GROUP).source;',
     "malformed light show was accepted", "world"),

    ("generated_show_passes_validation", "lighting.rs",
     "show.source.clone()", '"definitely not a light show".to_string()',
     "did not validate", "world"),

    ("song_lighting_produces_cues", "lighting.rs",
     "!cues.is_empty()", "cues.is_empty()",
     "produced no cues", "predicate"),

    ("show_written_via_api_is_readable", "lighting.rs",
     "readback.contains(LIGHTING_GROUP)", "readback.contains(\"absent-marker-xyz\")",
     "does not contain what was written", "predicate"),

    ("absent_midi_is_skipped_not_fatal", "subsystems.rs",
     "profile.midi = Subsystem::Absent;", "profile.midi = Subsystem::Detected;",
     "not_connected", "world"),

    ("absent_dmx_is_skipped_not_fatal", "subsystems.rs",
     "profile.dmx = Subsystem::Absent;", "profile.dmx = Subsystem::Detected;",
     "not_connected", "world"),

    # A *valid but different* device, not a bogus one: a bogus device stops the
    # player booting, which fails the check for the wrong reason and never
    # exercises the assertion under test.
    ("first_profile_wins", "subsystems.rs",
     'let mut second = ProfileSpec::detected("02-decoy");\n'
     '    second.audio = Subsystem::Bogus("e2e-decoy-device".to_string());',
     'let mut second = ProfileSpec::detected("00-decoy");\n'
     '    second.audio = Subsystem::Named("alsa:hw:CARD=Headphones,DEV=0".to_string());',
     "to be claimed", "world"),

    ("bogus_midi_device_degrades_gracefully", "subsystems.rs",
     'profile.midi = Subsystem::Bogus("e2e-nonexistent-midi-device".to_string());',
     "profile.midi = Subsystem::Detected;",
     "reported as connected", "world"),

    ("bogus_audio_device_degrades_gracefully", "subsystems.rs",
     'profile.audio = Subsystem::Bogus("e2e-nonexistent-audio-device".to_string());',
     "profile.audio = Subsystem::Detected;",
     "reported as connected", "world"),

    ("stale_checksum_is_rejected", "persistence.rs",
     "stale.is_err()", "stale.is_ok()",
     "stale checksum", "predicate"),

    ("generated_project_loads_all_songs", "startup.rs",
     ".unwrap_or(0);", ".unwrap_or(0) + 1;",
     "generated songs to load", "world"),

    ("playlist_navigation_moves_between_songs", "playback.rs",
     "first != second", "first == second",
     "next did not change", "predicate"),

    ("beat_clock_is_silent_when_disabled", "midi_output.rs",
     "let project = midi_project(false)?;", "let project = midi_project(true)?;",
     "timing clock pulses were transmitted", "world"),

    ("song_midi_notes_are_transmitted", "midi_output.rs",
     "(0..expected_notes).map(|b| 60 + (b as u8 % 12)).collect()",
     "(0..expected_notes).map(|b| 90 + (b as u8 % 12)).collect()",
     "do not match the song's MIDI file", "world"),

    ("configured_midi_device_transmits", "midi_output.rs",
     'vec![ProfileSpec::detected("01-e2e")]',
     'vec![{ let mut p = ProfileSpec::detected("01-e2e"); '
     'p.midi = Subsystem::Bogus("e2e-nope".to_string()); p }]',
     "was not claimed", "world"),

    ("player_starts_against_detected_hardware", "startup.rs",
     "claimed.starts_with(&configured)", "claimed.starts_with(\"absent-device-xyz\")",
     "generated profile named", "predicate"),
]


def fn_span(text, name):
    """Byte range of `pub async fn <name>`'s body, by brace matching."""
    m = re.search(rf"pub async fn {re.escape(name)}\b", text)
    if not m:
        return None
    i = text.index("{", m.end())
    depth, j = 0, i
    in_str = in_chr = False
    while j < len(text):
        c = text[j]
        if (in_str or in_chr) and c == "\\":
            j += 2               # skip the escaped character
            continue
        if in_str:
            in_str = c != '"'
        elif in_chr:
            in_chr = c != "'"
        elif c == '"':
            in_str = True
        elif c == "'":
            in_chr = True
        elif c == "{":
            depth += 1
        elif c == "}":
            depth -= 1
            if depth == 0:
                return (m.start(), j + 1)
        j += 1
    return None


def run(cmd, **kw):
    return subprocess.run(cmd, shell=True, cwd=ROOT, capture_output=True, text=True, **kw)


def main():
    only = sys.argv[1] if len(sys.argv) > 1 else None
    results = []

    for check, fname, find, repl, expect, kind in SWEEP:
        if only and only not in check:
            continue
        path = CHECKS / fname
        original = path.read_text()

        span = fn_span(original, check)
        if not span:
            results.append((check, kind, "NO-SPAN", "could not locate the function"))
            continue
        start, end = span
        body = original[start:end]
        if body.count(find) != 1:
            results.append((check, kind, "ANCHOR",
                            f"anchor appears {body.count(find)}x in the function"))
            continue

        path.write_text(original[:start] + body.replace(find, repl, 1) + original[end:])
        try:
            build = run("cargo build -p mtrack-harness 2>&1 | tail -3")
            if "error" in build.stdout:
                results.append((check, kind, "BUILD", build.stdout.strip()[:120]))
                continue
            out = run(f"./target/debug/mtrack-harness --only {check} 2>&1").stdout
            line = next((l for l in out.splitlines()
                         if check in l and any(o in l for o in
                                               ("PASS", "FAIL", "SKIP", "INCONCLUSIVE",
                                                "HARNESS-ERROR", "BLOCKED"))), "")
            outcome = next((o for o in ("FAIL", "INCONCLUSIVE", "HARNESS-ERROR", "BLOCKED",
                                        "SKIP", "PASS") if o in line), "?")
            if outcome == "PASS":
                verdict = "*** STILL PASSED — cannot fail ***"
            elif expect.lower() in out.lower():
                verdict = f"failed as expected ({outcome})"
            else:
                verdict = f"failed ({outcome}) but not with the expected reason"
            results.append((check, kind, outcome, verdict))
        finally:
            path.write_text(original)

    run("cargo build -p mtrack-harness 2>&1 | tail -1")

    print("\n" + "=" * 78)
    print("  NEGATIVE-CONTROL SWEEP")
    print("=" * 78)
    for check, kind, outcome, verdict in results:
        flag = "  " if outcome not in ("PASS", "NO-SPAN", "ANCHOR", "BUILD") else "!!"
        print(f"{flag} {kind:<9} {check:<46} {verdict}")
    bad = [r for r in results if r[2] in ("PASS", "NO-SPAN", "ANCHOR", "BUILD")]
    print("=" * 78)
    print(f"  {len(results) - len(bad)}/{len(results)} proved capable of failing")
    if bad:
        print(f"  {len(bad)} need attention: {', '.join(r[0] for r in bad)}")
    print("=" * 78)


if __name__ == "__main__":
    main()
