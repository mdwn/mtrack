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
//! The playback-position examples the MCP tools, the CLI help and the
//! docs quote: each one reads as a position (`parse_position`).

use crate::util::parse_position;

/// Each text that describes a position, by name. A description is a line
/// quoting `1:23.456`, the example every copy gives.
const SOURCES: [(&str, &str); 3] = [
    (
        "src/controller/mcp/service.rs",
        include_str!("../controller/mcp/service.rs"),
    ),
    ("src/cli.rs", include_str!("../cli.rs")),
    (
        "docs/src/interfaces/grpc.md",
        include_str!("../../docs/src/interfaces/grpc.md"),
    ),
];

/// The quoted words of `line` (in backticks or double quotes) that start
/// like a position: a digit or the `@` of a cue time.
fn examples(line: &str) -> Vec<&str> {
    let mut out = Vec::new();
    for quote in ['`', '"'] {
        out.extend(
            line.split(quote)
                .skip(1)
                .step_by(2)
                .filter(|w| w.starts_with(|c: char| c.is_ascii_digit() || c == '@')),
        );
    }
    out
}

#[test]
fn every_quoted_position_example_parses() {
    let mut descriptions = 0;
    for (name, text) in SOURCES {
        for (n, line) in text.lines().enumerate() {
            if !line.contains("1:23.456") {
                continue;
            }
            descriptions += 1;
            let quoted = examples(line);
            assert!(!quoted.is_empty(), "{name}:{}", n + 1);
            for example in quoted {
                assert!(
                    parse_position(example).is_ok(),
                    "{name}:{}: {example} is not a position",
                    n + 1
                );
            }
        }
    }
    // The MCP arguments and tool texts, the CLI help, the gRPC page.
    assert!(descriptions >= 9, "found {descriptions}");
}

/// Each MCP argument taking a position names every form it reads.
#[test]
fn the_mcp_arguments_name_every_form() {
    let service = SOURCES[0].1;
    let forms = "`mm:ss.mmm`, `Ns`, or a bare number of seconds";
    let arguments = service
        .lines()
        .filter(|l| l.trim_start().starts_with("///") && l.contains("1:23.456"))
        .filter(|l| !l.contains("Parses"))
        .collect::<Vec<_>>();
    assert_eq!(arguments.len(), 4, "{arguments:#?}");
    for line in arguments {
        assert!(line.contains(forms), "{line}");
    }
}
