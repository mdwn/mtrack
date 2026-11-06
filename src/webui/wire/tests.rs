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

//! What the UI reads of a payload whose JSON a hand-written `Serialize`
//! decides, where the generated type is stated rather than derived.

use serde_json::json;

#[test]
fn a_playlists_ambient_and_auto_play_are_what_their_ts_types_say() {
    use crate::config::ambient::PlaylistAmbient;
    use crate::config::AutoPlayConfig;
    // `false | Ambient | null`, `boolean | string`.
    assert_eq!(json!(PlaylistAmbient::Off), json!(false));
    assert_eq!(json!(Some(AutoPlayConfig::On(true))), json!(true));
    assert_eq!(json!(AutoPlayConfig::Gap("2s".to_string())), json!("2s"));
}

#[test]
fn a_stream_buffer_size_is_a_keyword_or_a_number() {
    use crate::config::StreamBufferSize;
    assert_eq!(json!(StreamBufferSize::Min), json!("min"));
    assert_eq!(json!(StreamBufferSize::Fixed(256)), json!(256));
}
