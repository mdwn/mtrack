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
//! Fixture types, as the patch form reads what the parser accepts.

use crate::lighting::parser::*;

#[test]
fn a_comment_marker_inside_a_quoted_channel_name_is_part_of_it() {
    // #656: `channel_name` and `special_case` skipped comments inside
    // their quotes, as a show's name did. (`#` is refused in a channel
    // name by its own rule, so a `//` stands for both markers there.)
    let content = "fixture_type \"Par\" {\n    channels: 2\n    channel_map: {\n        \
                   \"dimmer\": 1,\n        \"red // main\": 2\n    }\n    \
                   special_cases: [\"RGB #2\", \"Dimmer\"]\n}\n";
    let types = parse_fixture_types(content).expect("parses");
    let channels = types["Par"].channels();
    assert_eq!(channels.get("dimmer"), Some(&1));
    assert_eq!(channels.get("red // main"), Some(&2));
    // The form, which does not write special cases, sees no comment in
    // the channel map.
    let mapped = content.replace("    special_cases: [\"RGB #2\", \"Dimmer\"]\n", "");
    assert!(
        crate::lighting::fixture_patch::form_expresses(&mapped, "Par"),
        "{mapped}"
    );
    assert!(!crate::lighting::fixture_patch::form_expresses(
        &mapped.replace("\"red // main\": 2", "\"red\": 2 // main"),
        "Par"
    ));
}
