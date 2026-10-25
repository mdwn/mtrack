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
//! `include "file.light"`: sequences shared between files (#540).

use crate::lighting::parser::*;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Writes `files` (relative path, content) under a fresh directory.
fn tree(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for (path, content) in files {
        let path = dir.path().join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }
    dir
}

fn parse_at(
    dir: &Path,
    file: &str,
) -> Result<std::collections::HashMap<String, LightShow>, String> {
    let path: PathBuf = dir.join(file);
    let content = std::fs::read_to_string(&path).unwrap();
    parse_light_file(&content, Some(&path), None).map_err(|e| e.to_string())
}

const SONG_TEMPO: &str = "tempo {\n    start: 0.0s\n    bpm: 120\n    time_signature: 4/4\n}\n";

#[test]
fn included_sequences_play_under_the_including_files_tempo() {
    let show = format!(
        "include \"../common/looks.light\"\n{SONG_TEMPO}\nshow \"Song\" {{\n    @2/1\n    sequence \"strike\"(hit: \"#FF0000\"), loop: 2\n}}\n"
    );
    let dir = tree(&[
        (
            "common/looks.light",
            "sequence \"strike\"(hit, bed: \"#0000FF\"), length: 1measure {\n    @1/1\n    front_wash: static, color: $hit, duration: 1beat\n}\n",
        ),
        ("song/show.light", &show),
    ]);
    let shows = parse_at(dir.path(), "song/show.light").expect("parses");
    let times: Vec<Duration> = shows["Song"].cues.iter().map(|c| c.time).collect();
    // 120 BPM, 4/4: bar 2 at 2 s, the second pass a measure later.
    assert_eq!(times, vec![Duration::from_secs(2), Duration::from_secs(4)]);
    assert_eq!(
        shows["Song"].cues[0].effects[0].effect_type.duration(),
        Duration::from_millis(500)
    );
}

#[test]
fn nested_includes_resolve_from_their_own_folder_and_load_once() {
    let dir = tree(&[
        (
            "lib/all.light",
            "include \"parts/a.light\"\ninclude \"parts/b.light\"\n",
        ),
        (
            "lib/parts/a.light",
            "include \"base.light\"\nsequence \"a\" {\n    @0.000\n    sequence \"base\"\n}\n",
        ),
        (
            "lib/parts/b.light",
            "include \"base.light\"\nsequence \"b\" {\n    @0.000\n    sequence \"base\"\n}\n",
        ),
        (
            "lib/parts/base.light",
            "sequence \"base\" {\n    @0.000\n    front_wash: static, color: \"red\", duration: 1s\n}\n",
        ),
        (
            "show.light",
            "include \"lib/all.light\"\nshow \"Song\" {\n    @0.000\n    sequence \"a\"\n    @2.000\n    sequence \"b\"\n}\n",
        ),
    ]);
    let shows = parse_at(dir.path(), "show.light").expect("a diamond include is not a duplicate");
    assert_eq!(shows["Song"].cues.len(), 2);
}

#[test]
fn an_include_cycle_ends() {
    let dir = tree(&[
        (
            "a.light",
            "include \"b.light\"\nshow \"Song\" {\n    @0.000\n    sequence \"from_b\"\n}\n",
        ),
        (
            "b.light",
            "include \"a.light\"\nsequence \"from_b\" {\n    @0.000\n    front_wash: static, color: \"red\", duration: 1s\n}\n",
        ),
    ]);
    // b including a back would bring in a show, which an included file may
    // not define -- but a is the file being parsed, already read.
    parse_at(dir.path(), "a.light").expect("parses");
}

#[test]
fn a_sequence_defined_in_two_files_is_an_error() {
    let seq = "sequence \"wash\" {\n    @0.000\n    front_wash: static, color: \"red\", duration: 1s\n}\n";
    let dir = tree(&[
        ("common.light", seq),
        (
            "show.light",
            &format!("include \"common.light\"\n{seq}show \"Song\" {{\n    @0.000\n    sequence \"wash\"\n}}\n"),
        ),
    ]);
    let err = parse_at(dir.path(), "show.light").expect_err("duplicate");
    assert!(
        err.contains("'wash'") && err.contains("common.light") && err.contains("this file"),
        "{err}"
    );
}

#[test]
fn an_included_file_defines_only_sequences() {
    let dir = tree(&[
        (
            "common.light",
            "sequence \"wash\" {\n    @0.000\n    front_wash: static, color: \"red\", duration: 1s\n}\n\nshow \"Other\" {\n    @0.000\n    sequence \"wash\"\n}\n",
        ),
        ("show.light", "include \"common.light\"\nshow \"Song\" {\n    @0.000\n    sequence \"wash\"\n}\n"),
    ]);
    let err = parse_at(dir.path(), "show.light").expect_err("a show in an included file");
    assert!(
        err.contains("common.light") && err.contains("line 6") && err.contains("a show"),
        "{err}"
    );
}

#[test]
fn errors_in_an_included_sequence_name_its_file() {
    let dir = tree(&[
        (
            "common.light",
            "sequence \"wash\"(colour: \"#X\") {\n    @0.000\n    front_wash: static, color: $colour, duration: 1s\n}\n",
        ),
        ("show.light", "include \"common.light\"\n"),
    ]);
    let err = parse_at(dir.path(), "show.light").expect_err("bad default");
    assert!(
        err.contains("common.light: line 3") && err.contains("#X"),
        "{err}"
    );
}

#[test]
fn bad_includes_are_errors_at_their_line() {
    let dir = tree(&[
        ("notes.txt", "hello"),
        ("show.light", "\ninclude \"missing.light\"\n"),
        ("other.light", "include \"notes.txt\"\n"),
    ]);
    let err = parse_at(dir.path(), "show.light").expect_err("missing file");
    assert!(
        err.contains("line 2") && err.contains("missing.light"),
        "{err}"
    );

    let err = parse_at(dir.path(), "other.light").expect_err("not a .light file");
    assert!(err.contains("only .light files"), "{err}");

    // Source that is not a file has no folder to resolve against.
    let err = parse_light_shows("include \"common.light\"\n")
        .expect_err("no path")
        .to_string();
    assert!(err.contains("not a file"), "{err}");
}

#[test]
fn included_files_lists_every_file_reached() {
    let dir = tree(&[
        ("show.light", "include \"lib/a.light\"\n"),
        ("lib/a.light", "include \"b.light\"\n"),
        ("lib/b.light", "include \"a.light\"\n"),
    ]);
    let found = included_files(&dir.path().join("show.light"));
    let mut names: Vec<String> = found
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(names, vec!["a.light", "b.light"]);
}
