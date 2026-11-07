// Copyright (C) 2026 Michael Wilson <mike@mdwn.dev>
//
// This program is free software: you can redistribute it and/or modify it under
// the terms of the GNU General Public License as published by the Free Software
// Foundation, version 3.
//
// This program is distributed in the hope that it will be useful, but WITHOUT
// ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS
// FOR A PARTICULAR PURPOSE. See the GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License along with
// this program. If not, see <https://www.gnu.org/licenses/>.
//


// The file pickers' `accept=` lists and the MIDI test, from the player's
// one rule for a file's kind (`songs::FileKind`), as plain values (no
// browser).

import { test, expect } from "@playwright/test";
import { audioAccept, isMidiFile, midiAccept } from "../../src/lib/util/paths";
import { loadRules } from "./rules-node";

test.beforeAll(loadRules);

test("audio pickers accept every audio extension the player reads", () => {
  expect(audioAccept()).toBe(".wav,.flac,.mp3,.ogg,.aac,.m4a,.mp4,.aiff,.aif");
});

test("MIDI pickers accept .mid only, as the upload does", () => {
  expect(midiAccept()).toBe(".mid");
});

test("a MIDI file is one in any case, and .midi is not one", () => {
  expect(isMidiFile("show.mid")).toBe(true);
  expect(isMidiFile("Show.MID")).toBe(true);
  expect(isMidiFile("show.midi")).toBe(false);
  expect(isMidiFile("show.wav")).toBe(false);
  expect(isMidiFile("mid")).toBe(false);
});
