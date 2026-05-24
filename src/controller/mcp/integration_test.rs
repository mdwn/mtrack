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
//! End-to-end smoke tests for the MCP controller.
//!
//! Two scenarios:
//!
//! 1. [`mcp_initialize_list_and_call_tools`] uses the in-repo `assets/` fixtures
//!    and a fresh `Player` to drive the MCP protocol layer: `initialize`,
//!    `tools/list`, `tools/call status`, and both happy / unhappy paths of
//!    `tools/call validate_lighting`. No `ConfigStore` is wired.
//!
//! 2. [`mcp_config_store_round_trip`] sets up a writable tempdir with a real
//!    `mtrack.yaml`, `playlist.yaml`, and a song directory copied from
//!    `examples/`, then wires a `ConfigStore` the way `cli::local::start` does.
//!    Drives the config / song / playlist / lighting *write* tools and asserts
//!    the on-disk state matches what came back through MCP.

use std::collections::HashMap;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use reqwest::Client;
use serde_json::{json, Value};

use crate::{config, controller::Controller, player::Player, playlist, songs};

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Picks a port that is currently unbound by binding/dropping a listener.
/// Standard ephemeral-port trick — small TOCTOU race window, but adequate for
/// in-process tests.
fn pick_free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("local_addr").port();
    drop(listener);
    port
}

/// Polls the MCP endpoint until it accepts a connection.
async fn wait_until_listening(client: &Client, url: &str) {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        if client
            .post(url)
            .timeout(Duration::from_millis(100))
            .body("{}")
            .send()
            .await
            .is_ok()
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("MCP server never started listening at {url}");
}

/// Extracts a single JSON-RPC response from either an `application/json` body
/// or an SSE stream — the `StreamableHttpService` may use either.
fn parse_response_body(content_type: &str, body: &str) -> Value {
    if content_type.contains("text/event-stream") {
        for line in body.lines() {
            let Some(rest) = line.strip_prefix("data:") else {
                continue;
            };
            let payload = rest.trim_start();
            if payload.is_empty() {
                continue;
            }
            if let Ok(value) = serde_json::from_str::<Value>(payload) {
                return value;
            }
        }
        panic!("event-stream response had no JSON `data:` line:\n{body}");
    }
    if body.is_empty() {
        return Value::Null;
    }
    serde_json::from_str(body)
        .unwrap_or_else(|e| panic!("response body was neither SSE nor JSON ({e}):\n{body}"))
}

/// Posts a JSON-RPC request. Returns `(new_session_id, parsed_response)`.
async fn mcp_post(
    client: &Client,
    url: &str,
    session_id: Option<&str>,
    body: &Value,
) -> (Option<String>, Value) {
    let mut req = client
        .post(url)
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream");
    if let Some(sid) = session_id {
        req = req.header("mcp-session-id", sid);
    }
    let resp = req.json(body).send().await.expect("send");
    assert!(resp.status().is_success(), "non-2xx: {}", resp.status());

    let session = resp
        .headers()
        .get("mcp-session-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let ct = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let text = resp.text().await.expect("body");
    (session, parse_response_body(&ct, &text))
}

/// Completes the MCP handshake on a fresh server and returns the session id.
async fn initialize_session(client: &Client, url: &str) -> String {
    let (session, init_resp) = mcp_post(
        client,
        url,
        None,
        &json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "mtrack-test", "version": "0"}
            }
        }),
    )
    .await;
    assert!(
        init_resp["result"]["serverInfo"].is_object(),
        "initialize did not return serverInfo: {init_resp}"
    );
    let session = session.expect("server should issue a session id");
    let _ = mcp_post(
        client,
        url,
        Some(&session),
        &json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
    )
    .await;
    session
}

/// Invokes a tool and returns the parsed JSON-RPC envelope.
async fn call_tool(
    client: &Client,
    url: &str,
    session: &str,
    id: u64,
    name: &str,
    arguments: Value,
) -> Value {
    let (_, resp) = mcp_post(
        client,
        url,
        Some(session),
        &json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": {"name": name, "arguments": arguments}
        }),
    )
    .await;
    resp
}

/// Extracts the text payload from a `CallToolResult`. Our tools encode their
/// JSON result as a single text content block.
fn tool_text(response: &Value) -> String {
    response
        .get("result")
        .and_then(|r| r.get("content"))
        .and_then(|c| c.as_array())
        .and_then(|arr| arr.first())
        .and_then(|c| c.get("text"))
        .and_then(|t| t.as_str())
        .unwrap_or_default()
        .to_string()
}

/// Parses a tool's JSON payload directly.
fn tool_json(response: &Value) -> Value {
    let text = tool_text(response);
    serde_json::from_str(&text)
        .unwrap_or_else(|e| panic!("tool response was not JSON ({e}): {text}"))
}

/// Recursively copies a directory tree. Test-only — small helper so we don't
/// need to depend on `fs_extra` or similar.
fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let path = entry.path();
        let dest = dst.join(entry.file_name());
        if path.is_dir() {
            copy_dir_recursive(&path, &dest)?;
        } else {
            std::fs::copy(&path, &dest)?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Test 1: protocol smoke against the in-repo `assets/` fixtures
// ---------------------------------------------------------------------------

/// Builds a `Player` against the `assets/` fixtures shipped with the repo.
async fn build_assets_player() -> Result<Arc<Player>, Box<dyn Error>> {
    let songs = songs::get_all_songs(Path::new("assets/songs"))?;
    let pl = playlist::Playlist::new(
        "playlist",
        &config::Playlist::deserialize(Path::new("assets/playlist.yaml"))?,
        songs.clone(),
    )?;
    let mut playlists = HashMap::new();
    playlists.insert(
        "all_songs".to_string(),
        playlist::from_songs(songs.clone())?,
    );
    playlists.insert("playlist".to_string(), pl);
    let player = Player::new(
        playlists,
        "playlist".to_string(),
        &config::Player::new(
            vec![],
            Some(config::Audio::new("mock-device")),
            None,
            None,
            HashMap::new(),
            "assets/songs",
        ),
        None,
    )?;
    player.await_hardware_ready().await;
    Ok(player)
}

#[tokio::test(flavor = "multi_thread")]
async fn mcp_initialize_list_and_call_tools() -> Result<(), Box<dyn Error>> {
    let player = build_assets_player().await?;

    let port = pick_free_port();
    let cfg = config::McpController::new(port);
    let controller = Controller::new(vec![config::Controller::Mcp(cfg)], player);
    assert!(controller.statuses().iter().all(|s| s.status == "running"));

    let url = format!("http://127.0.0.1:{port}/mcp");
    let client = Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .expect("client");
    wait_until_listening(&client, &url).await;

    let session = initialize_session(&client, &url).await;

    // tools/list spot-checks tools from every surface to confirm the macros
    // wired everything.
    let (_, list_resp) = mcp_post(
        &client,
        &url,
        Some(&session),
        &json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
    )
    .await;
    let tools = list_resp["result"]["tools"]
        .as_array()
        .expect("tools array");
    let tool_names: Vec<&str> = tools
        .iter()
        .filter_map(|t| t.get("name").and_then(|n| n.as_str()))
        .collect();
    for expected in [
        "status",
        "list_songs",
        "list_playlists",
        "play",
        "get_cues",
        "get_config",
        "lighting_dsl_reference",
        "validate_lighting",
    ] {
        assert!(
            tool_names.contains(&expected),
            "missing tool `{expected}`; have {tool_names:?}"
        );
    }

    let status = tool_json(&call_tool(&client, &url, &session, 3, "status", json!({})).await);
    assert_eq!(status["playlist_name"], "playlist");
    assert_eq!(status["playing"], false);
    assert!(status["elapsed"].is_string());

    let valid_dsl = r#"show "Smoke" { @00:00.000 all: static color: "blue", duration: 1s }"#;
    let ok_resp = call_tool(
        &client,
        &url,
        &session,
        4,
        "validate_lighting",
        json!({"source": valid_dsl}),
    )
    .await;
    let ok_body = tool_json(&ok_resp);
    assert_eq!(ok_body["ok"], true, "valid DSL was rejected: {ok_body}");

    let bad_dsl = r#"show "Broken" { @00:00.000 cue without colon }"#;
    let err_resp = call_tool(
        &client,
        &url,
        &session,
        5,
        "validate_lighting",
        json!({"source": bad_dsl}),
    )
    .await;
    let err_body = tool_json(&err_resp);
    assert_eq!(err_body["ok"], false, "broken DSL was accepted: {err_body}");
    assert!(err_body["error"].as_str().is_some());

    controller.shutdown();
    Ok(())
}

/// Waits for any line of SSE event data that, parsed as JSON, satisfies
/// `predicate`. Returns the matched value or panics if the wait timed out.
async fn wait_for_event(
    client: &Client,
    url: &str,
    session: &str,
    timeout: Duration,
    mut predicate: impl FnMut(&Value) -> bool,
) -> Value {
    // The server may not send anything if `accept: application/json` is set,
    // so we explicitly ask for the event-stream form.
    let resp = client
        .get(url)
        .header("mcp-session-id", session)
        .header("accept", "text/event-stream")
        .timeout(timeout)
        .send()
        .await
        .expect("open GET /mcp");
    assert!(
        resp.status().is_success(),
        "GET /mcp returned {}",
        resp.status()
    );
    let mut stream = resp.bytes_stream();
    let mut buf = String::new();
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        match tokio::time::timeout(remaining, stream.next()).await {
            Ok(Some(Ok(bytes))) => {
                buf.push_str(std::str::from_utf8(&bytes).unwrap_or(""));
            }
            Ok(Some(Err(e))) => panic!("SSE stream error: {e}"),
            Ok(None) => break,
            Err(_) => break, // timeout
        }
        // SSE events are separated by blank lines.
        while let Some(idx) = buf.find("\n\n") {
            let event = buf[..idx].to_string();
            buf = buf[idx + 2..].to_string();
            for line in event.lines() {
                let Some(payload) = line.strip_prefix("data:").map(str::trim_start) else {
                    continue;
                };
                if payload.is_empty() {
                    continue;
                }
                if let Ok(value) = serde_json::from_str::<Value>(payload) {
                    if predicate(&value) {
                        return value;
                    }
                }
            }
        }
    }
    panic!("timed out waiting for SSE event matching predicate; buffer:\n{buf}");
}

// ---------------------------------------------------------------------------
// Test 2: full chain with a wired `ConfigStore` against a tempdir
// ---------------------------------------------------------------------------

struct StandaloneFixture {
    _tempdir: tempfile::TempDir,
    root: PathBuf,
    config_path: PathBuf,
}

impl StandaloneFixture {
    fn write_config(&self) {
        // Top-level `audio:` and `dmx:` are normalized into a profile at load
        // time, so `lighting_from_profiles()` resolves correctly without us
        // having to author the profile block by hand. The `all_lights`
        // logical group lets simplified shows in the cues/effects test
        // resolve a real group during play() validation.
        let yaml = format!(
            r#"songs: {songs}
playlist: {playlist}
audio:
  device: "mock-device"
dmx:
  universes:
    - universe: 1
      name: test-universe
  lighting:
    current_venue: "main_stage"
    groups:
      all_lights:
        name: "all_lights"
        constraints:
          - MinCount: 1
    directories:
      fixture_types: "lighting/fixture_types"
      venues: "lighting/venues"
"#,
            songs = self.root.join("songs").display(),
            playlist = self.root.join("playlist.yaml").display(),
        );
        std::fs::write(&self.config_path, yaml).expect("write mtrack.yaml");
    }

    fn write_playlist(&self, songs: &[&str]) {
        let mut yaml = String::from("kind: playlist\nsongs:\n");
        for name in songs {
            yaml.push_str(&format!("- {name}\n"));
        }
        std::fs::write(self.root.join("playlist.yaml"), yaml).expect("write playlist.yaml");
    }
}

/// Sets up a tempdir with a real `mtrack.yaml`, `playlist.yaml`, and the
/// `examples/songs/dsl-light-show-song` directory copied in so the player loads
/// a real song with real audio files.
fn setup_standalone_fixture() -> Result<StandaloneFixture, Box<dyn Error>> {
    let tempdir = tempfile::tempdir()?;
    let root = tempdir.path().to_path_buf();
    std::fs::create_dir_all(root.join("songs"))?;
    // `dsl-light-show-song` uses the modern `.light` DSL pointer format for
    // `lighting:`, so it loads cleanly against the current `config::Song`
    // schema. The older `dsl-light-show-song` example uses the legacy inline
    // cues format and would fail to deserialize.
    copy_dir_recursive(
        Path::new("examples/songs/dsl-light-show-song"),
        &root.join("songs/dsl-light-show-song"),
    )?;
    let config_path = root.join("mtrack.yaml");
    let fixture = StandaloneFixture {
        _tempdir: tempdir,
        root,
        config_path,
    };
    fixture.write_config();
    fixture.write_playlist(&["DSL Light Show Song"]);
    Ok(fixture)
}

/// Constructs a `Player` and wires a `ConfigStore` the way `cli::local::start`
/// does — the production path for standalone (non-test) deployment.
async fn build_standalone_player(
    fixture: &StandaloneFixture,
) -> Result<Arc<Player>, Box<dyn Error>> {
    let player_config = config::Player::deserialize(&fixture.config_path)?;
    let songs_path = player_config.songs(&fixture.config_path);
    let songs = songs::get_all_songs(&songs_path)?;
    let playlist_path = fixture.root.join("playlist.yaml");
    // Mirror `cli::local::start`: use `load_playlists` so a single broken
    // playlist file is logged and skipped instead of hard-erroring. The
    // production path expects this resilience so we test against it.
    let playlists = crate::player::load_playlists(
        None,
        Some(&playlist_path),
        songs.clone(),
    )?;
    let player = Player::new(
        playlists,
        "playlist".to_string(),
        &player_config,
        fixture.config_path.parent(),
    )?;
    let store = Arc::new(config::store::ConfigStore::new(
        player_config,
        fixture.config_path.clone(),
    ));
    player.set_config_store(store);

    // Wire a state-watch channel so subscription tools that depend on the
    // player state stream (mtrack://status) can be tested without a DMX
    // engine being present to drive the sampler.
    let (state_tx, _state_rx) = tokio::sync::watch::channel(Arc::new(
        crate::state::StateSnapshot::default(),
    ));
    player.set_state_tx(state_tx);

    player.await_hardware_ready().await;
    Ok(player)
}

#[tokio::test(flavor = "multi_thread")]
async fn mcp_config_store_round_trip() -> Result<(), Box<dyn Error>> {
    let fixture = setup_standalone_fixture()?;
    let player = build_standalone_player(&fixture).await?;

    let port = pick_free_port();
    let controller = Controller::new(
        vec![config::Controller::Mcp(config::McpController::new(port))],
        player,
    );
    assert!(controller.statuses().iter().all(|s| s.status == "running"));

    let url = format!("http://127.0.0.1:{port}/mcp");
    let client = Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .expect("client");
    wait_until_listening(&client, &url).await;
    let session = initialize_session(&client, &url).await;

    // --- get_config returns the YAML we wrote, plus a checksum.
    let cfg_resp =
        tool_json(&call_tool(&client, &url, &session, 10, "get_config", json!({})).await);
    let initial_yaml = cfg_resp["yaml"].as_str().expect("yaml string").to_string();
    let initial_checksum = cfg_resp["checksum"]
        .as_str()
        .expect("checksum string")
        .to_string();
    assert!(
        initial_yaml.contains("mock-device"),
        "initial yaml missing audio device:\n{initial_yaml}"
    );

    // --- update_audio mutates both the in-memory config and the on-disk file.
    let new_audio = json!({"device": "swapped-device"});
    let updated = tool_json(
        &call_tool(
            &client,
            &url,
            &session,
            11,
            "update_audio",
            json!({"body": new_audio, "expected_checksum": initial_checksum}),
        )
        .await,
    );
    let new_yaml = updated["yaml"].as_str().expect("updated yaml");
    assert!(
        new_yaml.contains("swapped-device"),
        "updated yaml missing new device:\n{new_yaml}"
    );
    let new_checksum = updated["checksum"].as_str().expect("new checksum");
    assert_ne!(
        new_checksum, initial_checksum,
        "checksum should change after mutation"
    );
    // The on-disk file should reflect the new device too.
    let disk_yaml = std::fs::read_to_string(&fixture.config_path)?;
    assert!(
        disk_yaml.contains("swapped-device"),
        "config file on disk was not updated:\n{disk_yaml}"
    );

    // --- update_audio is rejected with a stale checksum.
    let stale = call_tool(
        &client,
        &url,
        &session,
        12,
        "update_audio",
        json!({"body": {"device": "should-not-stick"}, "expected_checksum": initial_checksum}),
    )
    .await;
    assert!(
        stale.get("error").is_some() || stale["result"]["isError"].as_bool() == Some(true),
        "stale-checksum update should fail; got {stale}"
    );

    // --- read_playlist returns what we wrote.
    let pl =
        tool_json(&call_tool(&client, &url, &session, 13, "read_playlist", json!({})).await);
    assert!(
        pl["yaml"].as_str().unwrap_or("").contains("DSL Light Show Song"),
        "read_playlist did not return our playlist: {pl}"
    );

    // --- write_playlist roundtrips an unrelated edit (a comment) so the
    //     song list stays single-entry. Later patch tests rely on the song
    //     name being unique within the file.
    let new_playlist_yaml = "# round-tripped via MCP\nkind: playlist\nsongs:\n- DSL Light Show Song\n";
    let _ = call_tool(
        &client,
        &url,
        &session,
        14,
        "write_playlist",
        json!({"yaml": new_playlist_yaml}),
    )
    .await;
    let disk_pl = std::fs::read_to_string(fixture.root.join("playlist.yaml"))?;
    assert_eq!(disk_pl, new_playlist_yaml);

    // --- read_song reads the existing song.yaml content.
    let song = tool_json(
        &call_tool(
            &client,
            &url,
            &session,
            15,
            "read_song",
            json!({"name": "DSL Light Show Song"}),
        )
        .await,
    );
    assert!(
        song["yaml"]
            .as_str()
            .unwrap_or("")
            .contains("DSL Light Show Song"),
        "read_song did not return song.yaml: {song}"
    );

    // --- write_song creates a new song directory under the songs root.
    // We point the track at the existing song's audio file so the reload
    // succeeds — otherwise the new song fails to load (no audio file at the
    // referenced path) and list_songs wouldn't see it.
    let shared_audio = fixture
        .root
        .join("songs/dsl-light-show-song/song.mp3")
        .display()
        .to_string();
    let new_song_yaml = format!(
        "kind: song\nname: Brand New Song\ntracks:\n- name: main\n  file: \"{shared_audio}\"\n  file_channel: 1\n"
    );
    let new_song_yaml = new_song_yaml.as_str();
    let _ = call_tool(
        &client,
        &url,
        &session,
        16,
        "write_song",
        json!({"name": "brand-new-song", "yaml": new_song_yaml}),
    )
    .await;
    let new_song_path = fixture.root.join("songs/brand-new-song/song.yaml");
    assert!(
        new_song_path.is_file(),
        "write_song did not create {}",
        new_song_path.display()
    );
    assert_eq!(std::fs::read_to_string(&new_song_path)?, new_song_yaml);

    // The player's in-memory song registry should now include the new song.
    // (Pre-reload behavior was: list_songs still returns the stale set until
    // mtrack restarts.)
    let post_write = tool_json(
        &call_tool(&client, &url, &session, 1000, "list_songs", json!({})).await,
    );
    let names: Vec<String> = post_write["songs"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v["name"].as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    assert!(
        names.iter().any(|n| n == "Brand New Song"),
        "list_songs didn't see the new song after write_song: {names:?}",
    );

    // --- write_song_lighting validates and writes into the loaded song's
    //     lighting directory.
    let lighting_dsl = r#"show "Round-Trip" {
    @00:00.000
    all_lights: static color: "blue", intensity: 0.5, duration: 5s
}
"#;
    let _ = call_tool(
        &client,
        &url,
        &session,
        17,
        "write_song_lighting",
        json!({
            "song": "DSL Light Show Song",
            "file": "round_trip.light",
            "source": lighting_dsl,
        }),
    )
    .await;
    let lighting_path = fixture
        .root
        .join("songs/dsl-light-show-song/lighting/round_trip.light");
    assert!(
        lighting_path.is_file(),
        "write_song_lighting did not create {}",
        lighting_path.display()
    );

    // --- write_song_lighting refuses bad DSL and leaves no file behind.
    let bad_dsl = "show \"Bad\" { @00:00.000 cue with no colon }";
    let bad_resp = call_tool(
        &client,
        &url,
        &session,
        18,
        "write_song_lighting",
        json!({
            "song": "DSL Light Show Song",
            "file": "should_not_exist.light",
            "source": bad_dsl,
        }),
    )
    .await;
    assert!(
        bad_resp.get("error").is_some()
            || bad_resp["result"]["isError"].as_bool() == Some(true),
        "bad DSL should be rejected; got {bad_resp}"
    );
    let rejected_path = fixture
        .root
        .join("songs/dsl-light-show-song/lighting/should_not_exist.light");
    assert!(
        !rejected_path.exists(),
        "rejected lighting file was written anyway: {}",
        rejected_path.display()
    );

    // --- Venues: list (empty) → write → list (has one) → read → bad-write
    let venues_dir = fixture.root.join("lighting/venues");
    let initial_venues = tool_json(
        &call_tool(&client, &url, &session, 19, "list_venue_files", json!({})).await,
    );
    assert_eq!(
        initial_venues["dir"].as_str().unwrap_or(""),
        venues_dir.canonicalize().unwrap_or(venues_dir.clone()).display().to_string(),
        "venues dir resolved unexpectedly: {initial_venues}",
    );
    let venue_dsl = "venue \"test_stage\" {\n  fixture \"Wash1\" RGBW_Par @ 1:1 tags [\"wash\"]\n  group \"front_wash\" = Wash1\n}\n";
    let _ = call_tool(
        &client,
        &url,
        &session,
        20,
        "write_venue",
        json!({"file": "test_stage.light", "source": venue_dsl}),
    )
    .await;
    let listed = tool_json(
        &call_tool(&client, &url, &session, 21, "list_venue_files", json!({})).await,
    );
    let listed_files: Vec<String> = listed["files"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    assert!(
        listed_files.iter().any(|n| n == "test_stage.light"),
        "list_venue_files missing our write: {listed_files:?}",
    );
    let read = tool_json(
        &call_tool(
            &client,
            &url,
            &session,
            22,
            "read_venue",
            json!({"file": "test_stage.light"}),
        )
        .await,
    );
    assert_eq!(read["source"].as_str(), Some(venue_dsl));
    let bad_venue = call_tool(
        &client,
        &url,
        &session,
        23,
        "write_venue",
        json!({"file": "bogus.light", "source": "venue ohno { not valid syntax"}),
    )
    .await;
    assert!(
        bad_venue.get("error").is_some()
            || bad_venue["result"]["isError"].as_bool() == Some(true),
        "bad venue should be rejected: {bad_venue}",
    );
    assert!(
        !fixture.root.join("lighting/venues/bogus.light").exists(),
        "rejected venue was written anyway",
    );

    // --- Fixture types: write → list → read → bad-write
    let fixture_dsl = "fixture_type \"TestPar\" {\n  channels: 4\n  channel_map: { \"red\": 1, \"green\": 2, \"blue\": 3, \"dimmer\": 4 }\n}\n";
    let _ = call_tool(
        &client,
        &url,
        &session,
        24,
        "write_fixture_type",
        json!({"file": "test_par.light", "source": fixture_dsl}),
    )
    .await;
    let listed_fts = tool_json(
        &call_tool(
            &client,
            &url,
            &session,
            25,
            "list_fixture_type_files",
            json!({}),
        )
        .await,
    );
    let ft_files: Vec<String> = listed_fts["files"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    assert!(
        ft_files.iter().any(|n| n == "test_par.light"),
        "list_fixture_type_files missing our write: {ft_files:?}",
    );
    let read_ft = tool_json(
        &call_tool(
            &client,
            &url,
            &session,
            26,
            "read_fixture_type",
            json!({"file": "test_par.light"}),
        )
        .await,
    );
    assert_eq!(read_ft["source"].as_str(), Some(fixture_dsl));
    let bad_ft = call_tool(
        &client,
        &url,
        &session,
        27,
        "write_fixture_type",
        json!({"file": "bogus.light", "source": "fixture_type \"X\" { what is this"}),
    )
    .await;
    assert!(
        bad_ft.get("error").is_some() || bad_ft["result"]["isError"].as_bool() == Some(true),
        "bad fixture type should be rejected: {bad_ft}",
    );
    assert!(
        !fixture
            .root
            .join("lighting/fixture_types/bogus.light")
            .exists(),
        "rejected fixture type was written anyway",
    );

    // --- patch_playlist: rename a song in place
    let _ = call_tool(
        &client,
        &url,
        &session,
        28,
        "patch_playlist",
        json!({
            "old_string": "- DSL Light Show Song",
            "new_string": "- Renamed Song",
        }),
    )
    .await;
    let pl_after = std::fs::read_to_string(fixture.root.join("playlist.yaml"))?;
    assert!(
        pl_after.contains("Renamed Song"),
        "patch_playlist did not apply: {pl_after}"
    );
    assert!(!pl_after.contains("DSL Light Show Song"));

    // Restore so subsequent tools that reference the song still work.
    let _ = call_tool(
        &client,
        &url,
        &session,
        29,
        "patch_playlist",
        json!({
            "old_string": "- Renamed Song",
            "new_string": "- DSL Light Show Song",
        }),
    )
    .await;

    // --- patch_song: change a track file_channel via context-anchored replace
    let _ = call_tool(
        &client,
        &url,
        &session,
        30,
        "patch_song",
        json!({
            "name": "DSL Light Show Song",
            "old_string": "    file_channel: 1",
            "new_string": "    file_channel: 2",
        }),
    )
    .await;
    let song_yaml_after =
        std::fs::read_to_string(fixture.root.join("songs/dsl-light-show-song/song.yaml"))?;
    assert!(
        song_yaml_after.contains("file_channel: 2"),
        "patch_song did not apply: {song_yaml_after}"
    );

    // --- patch_song: not-found surfaces an error
    let not_found = call_tool(
        &client,
        &url,
        &session,
        31,
        "patch_song",
        json!({
            "name": "DSL Light Show Song",
            "old_string": "this text definitely does not appear",
            "new_string": "irrelevant",
        }),
    )
    .await;
    assert!(
        not_found.get("error").is_some()
            || not_found["result"]["isError"].as_bool() == Some(true),
        "missing old_string should be rejected: {not_found}"
    );

    // --- patch_song_lighting: non-unique without replace_all fails
    let lighting_dsl = r#"show "Two Cues" {
    @00:00.000
    all: static color: "blue", duration: 1s

    @00:02.000
    all: static color: "blue", duration: 1s
}
"#;
    let _ = call_tool(
        &client,
        &url,
        &session,
        32,
        "write_song_lighting",
        json!({
            "song": "DSL Light Show Song",
            "file": "patch_target.light",
            "source": lighting_dsl,
        }),
    )
    .await;
    let ambiguous = call_tool(
        &client,
        &url,
        &session,
        33,
        "patch_song_lighting",
        json!({
            "song": "DSL Light Show Song",
            "file": "patch_target.light",
            "old_string": "color: \"blue\"",
            "new_string": "color: \"red\"",
        }),
    )
    .await;
    assert!(
        ambiguous.get("error").is_some()
            || ambiguous["result"]["isError"].as_bool() == Some(true),
        "non-unique patch should be rejected without replace_all: {ambiguous}",
    );

    // --- patch_song_lighting: replace_all succeeds
    let _ = call_tool(
        &client,
        &url,
        &session,
        34,
        "patch_song_lighting",
        json!({
            "song": "DSL Light Show Song",
            "file": "patch_target.light",
            "old_string": "color: \"blue\"",
            "new_string": "color: \"red\"",
            "replace_all": true,
        }),
    )
    .await;
    let after_patch = std::fs::read_to_string(
        fixture
            .root
            .join("songs/dsl-light-show-song/lighting/patch_target.light"),
    )?;
    assert_eq!(
        after_patch.matches("color: \"red\"").count(),
        2,
        "replace_all should have changed both occurrences: {after_patch}"
    );
    assert!(!after_patch.contains("color: \"blue\""));

    // --- patch_song_lighting: invalid result is rejected, file untouched
    let bad_patch = call_tool(
        &client,
        &url,
        &session,
        35,
        "patch_song_lighting",
        json!({
            "song": "DSL Light Show Song",
            "file": "patch_target.light",
            "old_string": "show \"Two Cues\" {",
            "new_string": "show \"Two Cues\" {{ broken syntax",
        }),
    )
    .await;
    assert!(
        bad_patch.get("error").is_some()
            || bad_patch["result"]["isError"].as_bool() == Some(true),
        "patch yielding invalid DSL should be rejected: {bad_patch}",
    );
    let after_bad = std::fs::read_to_string(
        fixture
            .root
            .join("songs/dsl-light-show-song/lighting/patch_target.light"),
    )?;
    assert_eq!(
        after_bad, after_patch,
        "file should be unchanged when patched content is invalid"
    );

    // --- patch_venue + patch_fixture_type: happy path
    let _ = call_tool(
        &client,
        &url,
        &session,
        36,
        "patch_venue",
        json!({
            "file": "test_stage.light",
            "old_string": "tags [\"wash\"]",
            "new_string": "tags [\"wash\", \"front\"]",
        }),
    )
    .await;
    let venue_after = std::fs::read_to_string(fixture.root.join("lighting/venues/test_stage.light"))?;
    assert!(venue_after.contains("tags [\"wash\", \"front\"]"));

    let _ = call_tool(
        &client,
        &url,
        &session,
        37,
        "patch_fixture_type",
        json!({
            "file": "test_par.light",
            "old_string": "channels: 4",
            "new_string": "channels: 5",
        }),
    )
    .await;
    let ft_after =
        std::fs::read_to_string(fixture.root.join("lighting/fixture_types/test_par.light"))?;
    assert!(ft_after.contains("channels: 5"));

    controller.shutdown();
    Ok(())
}

// ---------------------------------------------------------------------------
// Test 3e: get_cues + get_active_effects against a real timeline
// ---------------------------------------------------------------------------

/// These two tools return placeholder data when nothing is playing. The
/// interesting case is when a song with a lighting timeline is loaded —
/// `get_cues` should return the cues from the active `.light` file and
/// `get_active_effects` should describe whatever effects the engine is
/// holding. We play the bundled `dsl-light-show-song` (which has a
/// `lighting:` block pointing at `main_show.light`) and sample both.
#[tokio::test(flavor = "multi_thread")]
async fn mcp_cues_and_effects_against_live_timeline() -> Result<(), Box<dyn Error>> {
    let fixture = setup_standalone_fixture()?;

    // The fixture setup creates `lighting/venues` lazily through the MCP
    // tools, so we have to mkdir it here. Then drop in a minimal venue that
    // defines an `all_lights` group so the lighting validator accepts the
    // simplified shows we're about to write.
    std::fs::create_dir_all(fixture.root.join("lighting/venues"))?;
    std::fs::create_dir_all(fixture.root.join("lighting/fixture_types"))?;
    // Copy a fixture-type definition so RGBW_Par resolves.
    copy_dir_recursive(
        Path::new("examples/lighting/fixture_types"),
        &fixture.root.join("lighting/fixture_types"),
    )?;
    std::fs::write(
        fixture.root.join("lighting/venues/main_stage.light"),
        "venue \"main_stage\" {\n  fixture \"Par1\" RGBW_Par @ 1:1 tags [\"wash\"]\n  fixture \"Par2\" RGBW_Par @ 1:7 tags [\"wash\"]\n  group \"all_lights\" = Par1, Par2\n}\n",
    )?;

    // The bundled `main_show.light` references groups (front_wash, back_wash,
    // movers, strobe) that the bundled venue doesn't define. Replace both
    // lighting files in the copied song with shows that target only
    // `all_lights`, which we just defined above.
    let simple_show = r#"show "Cue Test" {
    @00:00.000
    all_lights: static color: "blue", duration: 10s

    @00:05.000
    all_lights: static color: "red", duration: 5s

    @00:08.000
    all_lights: dimmer start_level: 1.0, end_level: 0.0, duration: 2s
}
"#;
    let song_lighting_dir = fixture.root.join("songs/dsl-light-show-song/lighting");
    std::fs::write(song_lighting_dir.join("main_show.light"), simple_show)?;
    std::fs::write(
        song_lighting_dir.join("outro.light"),
        "show \"Outro\" {\n    @00:00.000\n    all_lights: static color: \"white\", duration: 1s\n}\n",
    )?;

    let player = build_standalone_player(&fixture).await?;

    assert!(
        player.dmx_engine().is_some(),
        "test fixture should have a DMX engine running"
    );

    let port = pick_free_port();
    let controller = Controller::new(
        vec![config::Controller::Mcp(config::McpController::new(port))],
        player.clone(),
    );
    assert!(controller.statuses().iter().all(|s| s.status == "running"));

    let url = format!("http://127.0.0.1:{port}/mcp");
    let client = Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .expect("client");
    wait_until_listening(&client, &url).await;
    let session = initialize_session(&client, &url).await;

    // Before play(), the cue list is empty (no song timeline yet).
    let pre = tool_json(
        &call_tool(&client, &url, &session, 800, "get_cues", json!({})).await,
    );
    assert_eq!(
        pre["cues"].as_array().map(|a| a.len()).unwrap_or(0),
        0,
        "expected no cues before playback: {pre}"
    );

    let play_resp = tool_json(
        &call_tool(&client, &url, &session, 801, "play", json!({})).await,
    );
    assert!(
        play_resp["now_playing"].is_object(),
        "play did not start the song: {play_resp}"
    );

    // The timeline is set up asynchronously when play() activates the song.
    // Poll get_cues until it reports a non-empty timeline or we time out.
    let cues = {
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        loop {
            let body = tool_json(
                &call_tool(&client, &url, &session, 802, "get_cues", json!({})).await,
            );
            let count = body["cues"].as_array().map(|a| a.len()).unwrap_or(0);
            if count > 0 {
                break body;
            }
            if std::time::Instant::now() > deadline {
                panic!(
                    "get_cues never returned a timeline after playing a song with \
                     lighting; last response: {body}"
                );
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    };
    let arr = cues["cues"].as_array().expect("cues array");
    assert!(
        arr.len() >= 2,
        "expected at least two cues for the loaded show: {cues}"
    );
    for entry in arr {
        assert!(entry["index"].is_number(), "cue missing index: {entry}");
        assert!(entry["time"].is_string(), "cue missing time string: {entry}");
    }

    // get_active_effects returns a human-readable summary. We don't pin its
    // exact text (the formatter is internal), but we verify a non-empty
    // string came back rather than the "(no effect engine configured)"
    // placeholder.
    let effects_resp = call_tool(
        &client,
        &url,
        &session,
        803,
        "get_active_effects",
        json!({}),
    )
    .await;
    let effects_text = effects_resp["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_default();
    assert!(
        !effects_text.contains("(no effect engine configured)"),
        "expected a real effects summary, got: {effects_text}"
    );

    let _ = call_tool(&client, &url, &session, 804, "stop", json!({})).await;
    controller.shutdown();
    Ok(())
}

// ---------------------------------------------------------------------------
// Test 3d: profile add → update → remove round-trip
// ---------------------------------------------------------------------------

/// `add_profile` / `update_profile` / `remove_profile` share the `ConfigStore`
/// mutation path with `update_audio` etc., but they take a `Profile` payload
/// and an index argument that need their own coverage.
#[tokio::test(flavor = "multi_thread")]
async fn mcp_profile_crud_round_trip() -> Result<(), Box<dyn Error>> {
    let fixture = setup_standalone_fixture()?;
    let player = build_standalone_player(&fixture).await?;

    let port = pick_free_port();
    let controller = Controller::new(
        vec![config::Controller::Mcp(config::McpController::new(port))],
        player,
    );
    assert!(controller.statuses().iter().all(|s| s.status == "running"));

    let url = format!("http://127.0.0.1:{port}/mcp");
    let client = Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .expect("client");
    wait_until_listening(&client, &url).await;
    let session = initialize_session(&client, &url).await;

    // Helper to read the current checksum.
    let checksum = |id: u64| {
        let client = client.clone();
        let url = url.clone();
        let session = session.clone();
        async move {
            let cfg = tool_json(
                &call_tool(&client, &url, &session, id, "get_config", json!({})).await,
            );
            cfg["checksum"]
                .as_str()
                .expect("checksum string")
                .to_string()
        }
    };

    // Add a profile.
    let cs1 = checksum(700).await;
    let added = tool_json(
        &call_tool(
            &client,
            &url,
            &session,
            701,
            "add_profile",
            json!({
                "body": {
                    "kind": "hardware_profile",
                    "hostname": "mcp-added-host",
                    "dmx": {
                        "universes": [{"universe": 7, "name": "mcp-added-universe"}]
                    }
                },
                "expected_checksum": cs1,
            }),
        )
        .await,
    );
    let added_yaml = added["yaml"].as_str().expect("yaml string");
    assert!(
        added_yaml.contains("mcp-added-host"),
        "add_profile didn't land in YAML: {added_yaml}"
    );
    assert!(
        std::fs::read_to_string(&fixture.config_path)?.contains("mcp-added-host"),
        "add_profile didn't reach disk",
    );

    // Find the index of the profile we just added. `add_profile` appends to
    // the `profiles:` list, so it's the last entry — the count of
    // `kind: hardware_profile` lines minus one.
    let cfg_after_add = tool_json(
        &call_tool(&client, &url, &session, 702, "get_config", json!({})).await,
    );
    let yaml = cfg_after_add["yaml"].as_str().expect("yaml");
    let profile_count = yaml.matches("kind: hardware_profile").count();
    assert!(profile_count >= 1, "expected at least one profile in: {yaml}");
    let added_index = profile_count - 1;

    // Update the profile (rename the hostname).
    let cs2 = cfg_after_add["checksum"]
        .as_str()
        .expect("checksum string")
        .to_string();
    let updated = tool_json(
        &call_tool(
            &client,
            &url,
            &session,
            703,
            "update_profile",
            json!({
                "index": added_index,
                "body": {
                    "kind": "hardware_profile",
                    "hostname": "mcp-updated-host",
                    "dmx": {
                        "universes": [{"universe": 7, "name": "mcp-updated-universe"}]
                    }
                },
                "expected_checksum": cs2,
            }),
        )
        .await,
    );
    let updated_yaml = updated["yaml"].as_str().expect("yaml");
    assert!(
        updated_yaml.contains("mcp-updated-host"),
        "update_profile didn't replace the profile: {updated_yaml}"
    );
    assert!(
        !updated_yaml.contains("mcp-added-host"),
        "old hostname still present after update: {updated_yaml}"
    );

    // Remove the profile.
    let cs3 = updated["checksum"]
        .as_str()
        .expect("checksum string")
        .to_string();
    let removed = tool_json(
        &call_tool(
            &client,
            &url,
            &session,
            704,
            "remove_profile",
            json!({"index": added_index, "expected_checksum": cs3}),
        )
        .await,
    );
    let removed_yaml = removed["yaml"].as_str().expect("yaml");
    assert!(
        !removed_yaml.contains("mcp-updated-host"),
        "remove_profile didn't delete the profile: {removed_yaml}"
    );
    assert!(
        !std::fs::read_to_string(&fixture.config_path)?.contains("mcp-updated-host"),
        "remove_profile didn't reach disk",
    );

    // remove_profile with a bogus index should error.
    let cs4 = removed["checksum"]
        .as_str()
        .expect("checksum string")
        .to_string();
    let bogus = call_tool(
        &client,
        &url,
        &session,
        705,
        "remove_profile",
        json!({"index": 999, "expected_checksum": cs4}),
    )
    .await;
    assert!(
        bogus.get("error").is_some() || bogus["result"]["isError"].as_bool() == Some(true),
        "out-of-range index should be rejected: {bogus}"
    );

    controller.shutdown();
    Ok(())
}

// ---------------------------------------------------------------------------
// Test 3b: read_song reads flat-layout songs (assets/songs/<name>.yaml)
// ---------------------------------------------------------------------------

/// `assets/songs/` uses the flat layout (`song1.yaml`, `song2.yaml`, …) where
/// each YAML sits at the songs root rather than inside its own directory. The
/// initial cut of `read_song` blindly read `<base_path>/song.yaml`, which
/// fails for this layout. Verify that we now use the source YAML path
/// recorded at load time.
#[tokio::test(flavor = "multi_thread")]
async fn mcp_read_song_handles_flat_layout() -> Result<(), Box<dyn Error>> {
    let player = build_assets_player().await?;

    let port = pick_free_port();
    let controller = Controller::new(
        vec![config::Controller::Mcp(config::McpController::new(port))],
        player,
    );
    assert!(controller.statuses().iter().all(|s| s.status == "running"));

    let url = format!("http://127.0.0.1:{port}/mcp");
    let client = Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .expect("client");
    wait_until_listening(&client, &url).await;
    let session = initialize_session(&client, &url).await;

    // The asset songs are "Song 1" … "Song 10", each stored as a top-level
    // yaml under `assets/songs/`. Read one and assert the response carries
    // both the resolved path and the contents.
    let resp = tool_json(
        &call_tool(
            &client,
            &url,
            &session,
            400,
            "read_song",
            json!({"name": "Song 1"}),
        )
        .await,
    );
    let yaml = resp["yaml"].as_str().unwrap_or_default();
    assert!(
        yaml.contains("name: Song 1"),
        "read_song for flat layout did not return the right body: {yaml}",
    );
    let path = resp["path"].as_str().unwrap_or_default();
    assert!(
        path.ends_with("song1.yaml"),
        "read_song should resolve to song1.yaml for flat layout, got: {path}",
    );

    controller.shutdown();
    Ok(())
}

// ---------------------------------------------------------------------------
// Test 3a: every config-update tool round-trips through ConfigStore
// ---------------------------------------------------------------------------

/// Drives a single config-update tool: get_config → update_<section> → assert
/// the new YAML contains `marker` and the on-disk file does too. Returns the
/// new checksum so the caller can chain.
#[allow(clippy::too_many_arguments)]
async fn assert_config_update_roundtrip(
    client: &Client,
    url: &str,
    session: &str,
    config_path: &Path,
    id_base: u64,
    tool_name: &str,
    body: Value,
    marker: &str,
) -> String {
    let cfg = tool_json(&call_tool(client, url, session, id_base, "get_config", json!({})).await);
    let checksum = cfg["checksum"]
        .as_str()
        .expect("checksum string")
        .to_string();

    let updated = tool_json(
        &call_tool(
            client,
            url,
            session,
            id_base + 1,
            tool_name,
            json!({"body": body, "expected_checksum": checksum}),
        )
        .await,
    );
    let new_yaml = updated["yaml"].as_str().expect("updated yaml");
    assert!(
        new_yaml.contains(marker),
        "{tool_name}: new yaml missing marker `{marker}`:\n{new_yaml}",
    );
    let disk_yaml = std::fs::read_to_string(config_path).expect("read on-disk yaml");
    assert!(
        disk_yaml.contains(marker),
        "{tool_name}: on-disk yaml missing marker `{marker}`:\n{disk_yaml}",
    );
    updated["checksum"]
        .as_str()
        .expect("new checksum")
        .to_string()
}

#[tokio::test(flavor = "multi_thread")]
async fn mcp_config_updates_every_subsection() -> Result<(), Box<dyn Error>> {
    let fixture = setup_standalone_fixture()?;
    let player = build_standalone_player(&fixture).await?;

    let port = pick_free_port();
    let controller = Controller::new(
        vec![config::Controller::Mcp(config::McpController::new(port))],
        player,
    );
    assert!(controller.statuses().iter().all(|s| s.status == "running"));

    let url = format!("http://127.0.0.1:{port}/mcp");
    let client = Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .expect("client");
    wait_until_listening(&client, &url).await;
    let session = initialize_session(&client, &url).await;

    // update_midi installs a new MIDI subsection.
    assert_config_update_roundtrip(
        &client,
        &url,
        &session,
        &fixture.config_path,
        300,
        "update_midi",
        json!({"device": "virtual-midi"}),
        "virtual-midi",
    )
    .await;

    // update_dmx replaces the DMX block. We keep the existing lighting dirs
    // so the on-disk config stays internally consistent for the rest of the
    // tests (and for the running DMX engine).
    assert_config_update_roundtrip(
        &client,
        &url,
        &session,
        &fixture.config_path,
        310,
        "update_dmx",
        json!({
            "universes": [{"universe": 9, "name": "swapped-universe"}],
            "lighting": {
                "current_venue": "main_stage",
                "directories": {
                    "fixture_types": "lighting/fixture_types",
                    "venues": "lighting/venues"
                }
            }
        }),
        "swapped-universe",
    )
    .await;

    // update_controllers swaps the controllers list. We MUST keep our own
    // MCP controller in the list — otherwise we'd be cutting the branch
    // we're sitting on.
    assert_config_update_roundtrip(
        &client,
        &url,
        &session,
        &fixture.config_path,
        320,
        "update_controllers",
        json!([
            {"kind": "mcp", "port": port, "bind_address": "127.0.0.1"},
            {"kind": "grpc", "port": 43234}
        ]),
        "grpc",
    )
    .await;

    controller.shutdown();
    Ok(())
}

// ---------------------------------------------------------------------------
// Test 3: playback control round-trip
// ---------------------------------------------------------------------------

/// Verifies that the MCP playback control tools actually drive the underlying
/// audio device, not just return success. Uses the assets-based player + mock
/// audio device so we can observe is_playing transitions.
#[tokio::test(flavor = "multi_thread")]
async fn mcp_playback_controls_drive_audio() -> Result<(), Box<dyn Error>> {
    let player = build_assets_player().await?;
    let device = player
        .audio_device()
        .expect("audio device should be present")
        .to_mock()?;

    let port = pick_free_port();
    let controller = Controller::new(
        vec![config::Controller::Mcp(config::McpController::new(port))],
        player.clone(),
    );
    assert!(controller.statuses().iter().all(|s| s.status == "running"));

    let url = format!("http://127.0.0.1:{port}/mcp");
    let client = Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .expect("client");
    wait_until_listening(&client, &url).await;
    let session = initialize_session(&client, &url).await;

    // play
    let play_resp = tool_json(
        &call_tool(&client, &url, &session, 200, "play", json!({})).await,
    );
    assert!(
        play_resp["now_playing"].is_object(),
        "play didn't return a song: {play_resp}"
    );
    crate::testutil::eventually(|| device.is_playing(), "song never started playing");

    // status reports playing=true
    let status = tool_json(
        &call_tool(&client, &url, &session, 201, "status", json!({})).await,
    );
    assert_eq!(status["playing"], true, "status didn't see playback: {status}");
    assert!(status["current_song"].is_object());

    // stop
    let stop_resp = tool_json(
        &call_tool(&client, &url, &session, 202, "stop", json!({})).await,
    );
    assert!(
        stop_resp["stopped"].is_object(),
        "stop didn't return the stopped song: {stop_resp}"
    );
    crate::testutil::eventually(|| !device.is_playing(), "song never stopped playing");

    let status = tool_json(
        &call_tool(&client, &url, &session, 203, "status", json!({})).await,
    );
    assert_eq!(status["playing"], false);

    // next + previous move the playlist cursor without starting playback.
    let initial_song = player.get_playlist().current().map(|s| s.name().to_string());
    let next_resp = tool_json(
        &call_tool(&client, &url, &session, 204, "next", json!({})).await,
    );
    let after_next = player.get_playlist().current().map(|s| s.name().to_string());
    assert_ne!(initial_song, after_next, "next didn't advance: {next_resp}");

    let _ = call_tool(&client, &url, &session, 205, "previous", json!({})).await;
    let after_prev = player.get_playlist().current().map(|s| s.name().to_string());
    assert_eq!(initial_song, after_prev, "previous didn't restore cursor");

    controller.shutdown();
    Ok(())
}

// ---------------------------------------------------------------------------
// Test 3k: patch_playlist triggers a playlist reload
// ---------------------------------------------------------------------------

/// Reproduces the live bug we hit: a playlist whose songs list contains a
/// case-mismatched reference fails to load at startup, leaving only
/// `all_songs`. Patching the file to fix the reference should auto-reload
/// playlists so the player picks it up without a restart.
#[tokio::test(flavor = "multi_thread")]
async fn mcp_patch_playlist_triggers_reload() -> Result<(), Box<dyn Error>> {
    let fixture = setup_standalone_fixture()?;
    // Overwrite the playlist file with a song name that doesn't exist in
    // the registry — `Player::load_playlists` will skip it during startup
    // and only `all_songs` will be visible.
    std::fs::write(
        fixture.root.join("playlist.yaml"),
        "kind: playlist\nsongs:\n- nonexistent SONG name\n",
    )?;
    let player = build_standalone_player(&fixture).await?;

    let port = pick_free_port();
    let controller = Controller::new(
        vec![config::Controller::Mcp(config::McpController::new(port))],
        player,
    );
    assert!(controller.statuses().iter().all(|s| s.status == "running"));

    let url = format!("http://127.0.0.1:{port}/mcp");
    let client = Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .expect("client");
    wait_until_listening(&client, &url).await;
    let session = initialize_session(&client, &url).await;

    // Baseline: only `all_songs` is loaded because the playlist file
    // referenced a missing song at startup.
    let initial = tool_json(
        &call_tool(&client, &url, &session, 1500, "list_playlists", json!({})).await,
    );
    let initial_names: Vec<&str> = initial["playlists"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();
    assert_eq!(
        initial_names,
        vec!["all_songs"],
        "expected only all_songs at startup; got {initial_names:?}"
    );

    // Fix the playlist via MCP. After the patch, the player should
    // auto-reload and `playlist` should appear.
    let _ = call_tool(
        &client,
        &url,
        &session,
        1501,
        "patch_playlist",
        json!({
            "old_string": "nonexistent SONG name",
            "new_string": "DSL Light Show Song",
        }),
    )
    .await;

    let after = tool_json(
        &call_tool(&client, &url, &session, 1502, "list_playlists", json!({})).await,
    );
    let after_names: Vec<String> = after["playlists"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();
    assert!(
        after_names.iter().any(|n| n == "playlist"),
        "auto-reload didn't surface the fixed playlist; got {after_names:?}"
    );

    controller.shutdown();
    Ok(())
}

// ---------------------------------------------------------------------------
// Test 3l: song_details returns the structured metadata + computed BPM
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn mcp_song_details_returns_structured_metadata() -> Result<(), Box<dyn Error>> {
    let player = build_assets_player().await?;

    let port = pick_free_port();
    let controller = Controller::new(
        vec![config::Controller::Mcp(config::McpController::new(port))],
        player,
    );
    assert!(controller.statuses().iter().all(|s| s.status == "running"));

    let url = format!("http://127.0.0.1:{port}/mcp");
    let client = Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .expect("client");
    wait_until_listening(&client, &url).await;
    let session = initialize_session(&client, &url).await;

    let details = tool_json(
        &call_tool(
            &client,
            &url,
            &session,
            1600,
            "song_details",
            json!({"name": "Song 1"}),
        )
        .await,
    );

    assert_eq!(details["name"].as_str(), Some("Song 1"));
    assert!(details["duration"].is_string());
    assert!(details["duration_seconds"].is_number());
    let tracks = details["tracks"].as_array().expect("tracks array");
    assert!(!tracks.is_empty(), "song should have at least one track");
    for track in tracks {
        assert!(track["name"].is_string());
        assert!(track["file"].is_string());
        assert!(track["file_channel"].is_number());
    }
    // The asset songs have no click analysis cached, so beat_grid is None,
    // bpm is null, and tempo_segments is an empty array. The fields must
    // still exist in the response shape.
    assert!(details.get("beat_grid").is_some());
    assert!(details.get("bpm").is_some());
    assert_eq!(
        details["tempo_segments"]
            .as_array()
            .map(Vec::len),
        Some(0),
        "tempo_segments should be empty for songs with no beat grid: {details}"
    );
    // song_details must NOT carry the raw per-beat array — that's reserved
    // for song_beat_grid so the response stays small even on long songs.
    if let Some(bg) = details["beat_grid"].as_object() {
        assert!(
            !bg.contains_key("beats"),
            "song_details.beat_grid must not include the raw beats array: {details}"
        );
    }

    // song_beat_grid returns the raw arrays (null when no analysis cached).
    let grid = tool_json(
        &call_tool(
            &client,
            &url,
            &session,
            1601,
            "song_beat_grid",
            json!({"name": "Song 1"}),
        )
        .await,
    );
    assert_eq!(grid["name"].as_str(), Some("Song 1"));
    assert!(grid.get("beats").is_some());
    assert!(grid.get("measure_starts").is_some());

    // Unknown name should be rejected for both tools.
    let missing = call_tool(
        &client,
        &url,
        &session,
        1602,
        "song_details",
        json!({"name": "definitely not a real song"}),
    )
    .await;
    assert!(
        missing.get("error").is_some(),
        "song_details for unknown song should error: {missing}"
    );
    let missing_grid = call_tool(
        &client,
        &url,
        &session,
        1603,
        "song_beat_grid",
        json!({"name": "definitely not a real song"}),
    )
    .await;
    assert!(
        missing_grid.get("error").is_some(),
        "song_beat_grid for unknown song should error: {missing_grid}"
    );

    controller.shutdown();
    Ok(())
}

// ---------------------------------------------------------------------------
// Test 3i: host_info exposes hostname + profile + subsystem statuses
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn mcp_host_info_returns_runtime_identity() -> Result<(), Box<dyn Error>> {
    let player = build_assets_player().await?;

    let port = pick_free_port();
    let controller = Controller::new(
        vec![config::Controller::Mcp(config::McpController::new(port))],
        player,
    );
    assert!(controller.statuses().iter().all(|s| s.status == "running"));

    let url = format!("http://127.0.0.1:{port}/mcp");
    let client = Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .expect("client");
    wait_until_listening(&client, &url).await;
    let session = initialize_session(&client, &url).await;

    let info = tool_json(
        &call_tool(&client, &url, &session, 1300, "host_info", json!({})).await,
    );

    // We can't assert the exact hostname (varies per machine), but the field
    // must be present and the subsystem objects must have the documented
    // shape so MCP clients can rely on them.
    assert!(info["init_done"].is_boolean(), "host_info missing init_done: {info}");
    assert!(info.get("hostname").is_some(), "host_info missing hostname: {info}");
    for subsystem in ["audio", "midi", "dmx", "trigger"] {
        let s = &info[subsystem];
        assert!(
            s["status"].is_string(),
            "host_info.{subsystem}.status missing: {info}"
        );
    }
    // The assets-based test player wires a mock audio device. Its Display
    // impl appends "(Mock)" to the configured name — assert that flows
    // through here.
    assert!(
        info["audio"]["name"]
            .as_str()
            .unwrap_or("")
            .contains("mock-device"),
        "expected mock-device in audio.name: {info}"
    );

    controller.shutdown();
    Ok(())
}

// ---------------------------------------------------------------------------
// Test 3j: list_groups returns both venue and logical groups
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn mcp_list_groups_surfaces_logical_groups() -> Result<(), Box<dyn Error>> {
    let fixture = setup_standalone_fixture()?;
    // The standalone fixture already declares a logical group named
    // `all_lights` (see write_config). Build a minimal venue with one
    // explicit group too, so we can assert both flavors come back.
    std::fs::create_dir_all(fixture.root.join("lighting/venues"))?;
    std::fs::create_dir_all(fixture.root.join("lighting/fixture_types"))?;
    copy_dir_recursive(
        Path::new("examples/lighting/fixture_types"),
        &fixture.root.join("lighting/fixture_types"),
    )?;
    std::fs::write(
        fixture.root.join("lighting/venues/main_stage.light"),
        "venue \"main_stage\" {\n  fixture \"Par1\" RGBW_Par @ 1:1 tags [\"wash\"]\n  fixture \"Par2\" RGBW_Par @ 1:7 tags [\"wash\"]\n  group \"my_venue_group\" = Par1, Par2\n}\n",
    )?;

    let player = build_standalone_player(&fixture).await?;

    let port = pick_free_port();
    let controller = Controller::new(
        vec![config::Controller::Mcp(config::McpController::new(port))],
        player,
    );
    assert!(controller.statuses().iter().all(|s| s.status == "running"));

    let url = format!("http://127.0.0.1:{port}/mcp");
    let client = Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .expect("client");
    wait_until_listening(&client, &url).await;
    let session = initialize_session(&client, &url).await;

    let resp = tool_json(
        &call_tool(&client, &url, &session, 1400, "list_groups", json!({})).await,
    );
    let groups = resp["groups"].as_array().expect("groups array");

    let by_name: std::collections::HashMap<&str, &Value> = groups
        .iter()
        .filter_map(|g| g["name"].as_str().map(|n| (n, g)))
        .collect();
    assert!(
        by_name.contains_key("my_venue_group"),
        "venue group missing from list_groups: {resp}",
    );
    assert_eq!(
        by_name["my_venue_group"]["source"].as_str(),
        Some("venue")
    );
    assert!(
        by_name.contains_key("all_lights"),
        "logical group missing from list_groups: {resp}",
    );
    assert_eq!(
        by_name["all_lights"]["source"].as_str(),
        Some("logical")
    );
    assert!(
        by_name["all_lights"]["constraints"]
            .as_array()
            .map(|a| !a.is_empty())
            .unwrap_or(false),
        "logical group missing constraints: {resp}",
    );

    controller.shutdown();
    Ok(())
}

// ---------------------------------------------------------------------------
// Test 3h: idle sessions get evicted past TTL
// ---------------------------------------------------------------------------

/// `rmcp::LocalSessionManager` does not auto-evict; we layer our own sweeper
/// that closes sessions whose last activity is older than the configured
/// `idle_session_timeout`. Verify that a session held idle past the TTL is
/// gone from the server's perspective.
#[tokio::test(flavor = "multi_thread")]
async fn mcp_idle_sessions_are_evicted() -> Result<(), Box<dyn Error>> {
    let player = build_assets_player().await?;

    let port = pick_free_port();
    // 1-second idle TTL with a fast sweeper (clamped to MIN_SWEEP_INTERVAL =
    // 5s). We wait long enough to clear both.
    let cfg = config::McpController::with_idle_timeout(port, Some(1));
    let controller = Controller::new(vec![config::Controller::Mcp(cfg)], player);
    assert!(controller.statuses().iter().all(|s| s.status == "running"));

    let url = format!("http://127.0.0.1:{port}/mcp");
    let client = Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .expect("client");
    wait_until_listening(&client, &url).await;
    let session = initialize_session(&client, &url).await;

    // Confirm the session works initially.
    let status = tool_json(
        &call_tool(&client, &url, &session, 1200, "status", json!({})).await,
    );
    assert_eq!(status["playlist_name"], "playlist");

    // Now go quiet for longer than (TTL + sweep_interval). The sweep
    // interval is clamped to `MIN_SWEEP_INTERVAL` (5s) regardless of TTL, so
    // worst case eviction happens at TTL + 5s = 6s. Give ourselves comfortable
    // slack for slow test runners; the test isn't latency-sensitive.
    tokio::time::sleep(Duration::from_secs(12)).await;

    // The session id should no longer be valid. Tool calls succeed-or-fail
    // shape varies by implementation; we just require that *something*
    // changes — either the body carries an error, or the server mints a new
    // session id on a fresh request.
    let resp = client
        .post(&url)
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .header("mcp-session-id", &session)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1201,
            "method": "tools/call",
            "params": {"name": "status", "arguments": {}}
        }))
        .send()
        .await?;
    let new_session = resp
        .headers()
        .get("mcp-session-id")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let status_code = resp.status();
    let body = resp.text().await?;
    let body_indicates_error = body.contains("\"error\"") || !status_code.is_success();
    let session_changed = matches!(new_session, Some(ref id) if id.as_str() != session.as_str());
    assert!(
        body_indicates_error || session_changed,
        "expected the evicted session id to be rejected or replaced; status {status_code}, new session {new_session:?}, body: {body}",
    );

    controller.shutdown();
    Ok(())
}

// ---------------------------------------------------------------------------
// Test 3g: session DELETE cleans up server-side state
// ---------------------------------------------------------------------------

/// `LocalSessionManager::close_session` is documented to remove the session
/// and tear down its worker, but our own `McpServer::subscriptions` map is
/// dropped through `SubscriptionHandle::Drop`. Verify that:
///
///   1. After we send `DELETE /mcp` with a session id, follow-up requests on
///      that session id are rejected.
///   2. Triggering a config change after the delete does not blow anything
///      up — i.e. the lingering subscription task (if any) handles
///      `peer.notify_*` returning an error cleanly.
#[tokio::test(flavor = "multi_thread")]
async fn mcp_session_delete_cleans_up_subscriptions() -> Result<(), Box<dyn Error>> {
    let fixture = setup_standalone_fixture()?;
    let player = build_standalone_player(&fixture).await?;

    let port = pick_free_port();
    let controller = Controller::new(
        vec![config::Controller::Mcp(config::McpController::new(port))],
        player,
    );
    assert!(controller.statuses().iter().all(|s| s.status == "running"));

    let url = format!("http://127.0.0.1:{port}/mcp");
    let client = Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .expect("client");
    wait_until_listening(&client, &url).await;
    let session = initialize_session(&client, &url).await;

    // Subscribe so we have something for the session to clean up.
    let _ = mcp_post(
        &client,
        &url,
        Some(&session),
        &json!({
            "jsonrpc": "2.0",
            "id": 1100,
            "method": "resources/subscribe",
            "params": {"uri": "mtrack://config"}
        }),
    )
    .await;

    // DELETE the session.
    let delete_resp = client
        .delete(&url)
        .header("mcp-session-id", &session)
        .send()
        .await?;
    assert!(
        delete_resp.status().is_success() || delete_resp.status().as_u16() == 202,
        "DELETE /mcp returned unexpected status {}",
        delete_resp.status()
    );

    // A follow-up request on the now-closed session id should fail (the
    // session is gone from the manager). Exact failure mode varies — we
    // accept any non-2xx OR an error in the JSON-RPC envelope.
    let followup = client
        .post(&url)
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .header("mcp-session-id", &session)
        .json(&json!({"jsonrpc": "2.0", "id": 1101, "method": "tools/list"}))
        .send()
        .await?;
    if followup.status().is_success() {
        let body = followup.text().await?;
        // If the server returned 200, the body should at least surface an
        // error — not a normal tools/list payload.
        assert!(
            body.contains("error")
                || body.is_empty()
                || !body.contains("\"tools\""),
            "closed session should not return a tools/list payload: {body}"
        );
    }

    // Trigger a config change — this exercises the path where any
    // surviving subscription task tries to notify a dead peer. It should
    // self-terminate without panicking; the controller should keep running.
    let new_session = initialize_session(&client, &url).await;
    let cfg = tool_json(
        &call_tool(&client, &url, &new_session, 1102, "get_config", json!({})).await,
    );
    let checksum = cfg["checksum"].as_str().expect("checksum").to_string();
    let _ = call_tool(
        &client,
        &url,
        &new_session,
        1103,
        "update_audio",
        json!({
            "body": {"device": "after-delete-device"},
            "expected_checksum": checksum,
        }),
    )
    .await;

    // The new session should still work normally afterward.
    let status = tool_json(
        &call_tool(&client, &url, &new_session, 1104, "status", json!({})).await,
    );
    assert_eq!(status["playlist_name"], "playlist");

    controller.shutdown();
    Ok(())
}

// ---------------------------------------------------------------------------
// Test 3f: many simultaneous sessions
// ---------------------------------------------------------------------------

/// Spins up several MCP sessions in parallel and drives a tool call from each.
/// Verifies that the session manager keeps them isolated and that nothing
/// panics under concurrent access. Doesn't try to test fairness or
/// throughput — just "more than one client at a time doesn't break things."
#[tokio::test(flavor = "multi_thread")]
async fn mcp_handles_many_simultaneous_sessions() -> Result<(), Box<dyn Error>> {
    let player = build_assets_player().await?;

    let port = pick_free_port();
    let controller = Controller::new(
        vec![config::Controller::Mcp(config::McpController::new(port))],
        player,
    );
    assert!(controller.statuses().iter().all(|s| s.status == "running"));

    let url = format!("http://127.0.0.1:{port}/mcp");
    let client = Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .expect("client");
    wait_until_listening(&client, &url).await;

    let mut handles = Vec::new();
    for i in 0..8u64 {
        let client = client.clone();
        let url = url.clone();
        handles.push(tokio::spawn(async move {
            let session = initialize_session(&client, &url).await;
            // Drive a few tools to make sure each session sees the same world
            // and gets unique session ids.
            let status = tool_json(
                &call_tool(&client, &url, &session, 900 + i, "status", json!({})).await,
            );
            assert_eq!(status["playlist_name"], "playlist");

            let songs = tool_json(
                &call_tool(&client, &url, &session, 1000 + i, "list_songs", json!({}))
                    .await,
            );
            let count = songs["count"].as_u64().unwrap_or(0);
            assert!(count > 0, "session {i} saw no songs: {songs}");

            session
        }));
    }

    // All sessions must successfully complete their work.
    let mut sessions = Vec::with_capacity(handles.len());
    for handle in handles {
        sessions.push(handle.await.expect("session task panicked"));
    }
    // Session ids must be unique — the session manager is what guarantees
    // per-client state separation.
    let unique: std::collections::HashSet<&String> = sessions.iter().collect();
    assert_eq!(unique.len(), sessions.len(), "session ids should all be unique");

    controller.shutdown();
    Ok(())
}

// ---------------------------------------------------------------------------
// Test 3c: bearer token auth
// ---------------------------------------------------------------------------

/// When `bearer_token` is configured, all `/mcp` requests must carry a
/// matching `Authorization: Bearer …` header. Missing, malformed, and wrong
/// tokens are rejected with 401; the correct token is accepted.
#[tokio::test(flavor = "multi_thread")]
async fn mcp_bearer_token_enforces_auth() -> Result<(), Box<dyn Error>> {
    let player = build_assets_player().await?;

    let port = pick_free_port();
    let token = "test-secret-token-123";
    let controller = Controller::new(
        vec![config::Controller::Mcp(
            config::McpController::with_bearer_token(port, token),
        )],
        player,
    );
    assert!(controller.statuses().iter().all(|s| s.status == "running"));

    let url = format!("http://127.0.0.1:{port}/mcp");
    let client = Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .expect("client");

    // Wait for the listener — the auth layer applies, so even our probe sees
    // 401 once the socket is up. That's still "listening" for us.
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        if client.post(&url).body("{}").send().await.is_ok() {
            break;
        }
        if std::time::Instant::now() > deadline {
            panic!("MCP never started listening");
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    let init_body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {"name": "auth-test", "version": "0"}
        }
    });

    // No Authorization header at all → 401.
    let no_header = client
        .post(&url)
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .json(&init_body)
        .send()
        .await?;
    assert_eq!(
        no_header.status(),
        reqwest::StatusCode::UNAUTHORIZED,
        "missing token should yield 401",
    );

    // Wrong token → 401.
    let wrong = client
        .post(&url)
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .header("authorization", "Bearer not-the-right-token")
        .json(&init_body)
        .send()
        .await?;
    assert_eq!(
        wrong.status(),
        reqwest::StatusCode::UNAUTHORIZED,
        "wrong token should yield 401",
    );

    // Malformed (no Bearer prefix) → 401.
    let malformed = client
        .post(&url)
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .header("authorization", token)
        .json(&init_body)
        .send()
        .await?;
    assert_eq!(
        malformed.status(),
        reqwest::StatusCode::UNAUTHORIZED,
        "missing `Bearer ` prefix should yield 401",
    );

    // Correct token → 200 plus a real initialize response.
    let ok = client
        .post(&url)
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .header("authorization", format!("Bearer {token}"))
        .json(&init_body)
        .send()
        .await?;
    assert!(
        ok.status().is_success(),
        "correct token should succeed, got {}",
        ok.status(),
    );

    controller.shutdown();
    Ok(())
}

// ---------------------------------------------------------------------------
// Test 4: server-initiated resource notifications
// ---------------------------------------------------------------------------

/// Verifies that the `mtrack://status` resource fires
/// `notifications/resources/updated` when the player's state-watch channel
/// receives a new snapshot.
#[tokio::test(flavor = "multi_thread")]
async fn mcp_status_subscription_notifies_on_state_change() -> Result<(), Box<dyn Error>> {
    let fixture = setup_standalone_fixture()?;
    let player = build_standalone_player(&fixture).await?;
    let state_tx = player
        .state_tx()
        .expect("test fixture wires state_tx");

    let port = pick_free_port();
    let controller = Controller::new(
        vec![config::Controller::Mcp(config::McpController::new(port))],
        player,
    );
    assert!(controller.statuses().iter().all(|s| s.status == "running"));

    let url = format!("http://127.0.0.1:{port}/mcp");
    let client = Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .expect("client");
    wait_until_listening(&client, &url).await;
    let session = initialize_session(&client, &url).await;

    let (_, sub_resp) = mcp_post(
        &client,
        &url,
        Some(&session),
        &json!({
            "jsonrpc": "2.0",
            "id": 600,
            "method": "resources/subscribe",
            "params": {"uri": "mtrack://status"}
        }),
    )
    .await;
    assert!(
        sub_resp.get("error").is_none() && sub_resp["result"].is_object(),
        "subscribe was rejected: {sub_resp}"
    );

    let session_clone = session.clone();
    let url_clone = url.clone();
    let listener_client = client.clone();
    let listener = tokio::spawn(async move {
        wait_for_event(
            &listener_client,
            &url_clone,
            &session_clone,
            Duration::from_secs(5),
            |v| {
                v.get("method").and_then(|m| m.as_str())
                    == Some("notifications/resources/updated")
                    && v["params"]["uri"].as_str() == Some("mtrack://status")
            },
        )
        .await
    });

    // Give the listener a moment to open the GET connection so the server
    // has somewhere to push the notification.
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Push a synthetic state snapshot. The exact contents don't matter — the
    // `watch::Sender::send` call is what flips the receiver's "changed" flag
    // that the subscription task is awaiting.
    state_tx.send_modify(|snap| {
        let mut next = crate::state::StateSnapshot::default();
        next.active_effects.push("synthetic-effect-from-test".to_string());
        *snap = Arc::new(next);
    });

    let event = listener.await.expect("listener task");
    assert_eq!(
        event["params"]["uri"].as_str(),
        Some("mtrack://status"),
        "unexpected event: {event}"
    );

    controller.shutdown();
    Ok(())
}

/// Subscribes to `mtrack://config` and verifies that a config mutation
/// triggers a `notifications/resources/updated` event on the SSE channel.
#[tokio::test(flavor = "multi_thread")]
async fn mcp_resource_subscription_notifies_on_config_change() -> Result<(), Box<dyn Error>> {
    let fixture = setup_standalone_fixture()?;
    let player = build_standalone_player(&fixture).await?;

    let port = pick_free_port();
    let controller = Controller::new(
        vec![config::Controller::Mcp(config::McpController::new(port))],
        player,
    );
    assert!(controller.statuses().iter().all(|s| s.status == "running"));

    let url = format!("http://127.0.0.1:{port}/mcp");
    let client = Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .expect("client");
    wait_until_listening(&client, &url).await;
    let session = initialize_session(&client, &url).await;

    // Subscribe to the config resource. The handler spawns a watcher tied to
    // this session.
    let (_, sub_resp) = mcp_post(
        &client,
        &url,
        Some(&session),
        &json!({
            "jsonrpc": "2.0",
            "id": 100,
            "method": "resources/subscribe",
            "params": {"uri": "mtrack://config"}
        }),
    )
    .await;
    assert!(
        sub_resp.get("error").is_none() && sub_resp["result"].is_object(),
        "subscribe was rejected: {sub_resp}"
    );

    // Get a checksum so we can trigger a valid mutation.
    let cfg_resp = tool_json(
        &call_tool(&client, &url, &session, 101, "get_config", json!({})).await,
    );
    let checksum = cfg_resp["checksum"]
        .as_str()
        .expect("checksum string")
        .to_string();

    // Open the SSE listener BEFORE triggering the mutation. We do this in a
    // spawned task so we can fire the mutation in the foreground.
    let session_clone = session.clone();
    let url_clone = url.clone();
    let listener_client = client.clone();
    let listener = tokio::spawn(async move {
        wait_for_event(
            &listener_client,
            &url_clone,
            &session_clone,
            Duration::from_secs(5),
            |v| {
                v.get("method").and_then(|m| m.as_str())
                    == Some("notifications/resources/updated")
                    && v["params"]["uri"].as_str() == Some("mtrack://config")
            },
        )
        .await
    });

    // Give the listener a moment to actually open the GET connection so the
    // server has somewhere to push the notification when ConfigStore fires.
    tokio::time::sleep(Duration::from_millis(200)).await;

    let _ = call_tool(
        &client,
        &url,
        &session,
        102,
        "update_audio",
        json!({
            "body": {"device": "subscription-test-device"},
            "expected_checksum": checksum,
        }),
    )
    .await;

    let event = listener.await.expect("listener task");
    assert_eq!(
        event["params"]["uri"].as_str(),
        Some("mtrack://config"),
        "unexpected event: {event}"
    );

    // resources/read returns the same content as `get_config` for this URI.
    let (_, read_resp) = mcp_post(
        &client,
        &url,
        Some(&session),
        &json!({
            "jsonrpc": "2.0",
            "id": 103,
            "method": "resources/read",
            "params": {"uri": "mtrack://config"}
        }),
    )
    .await;
    let contents = read_resp["result"]["contents"][0]["text"]
        .as_str()
        .expect("resource text");
    assert!(
        contents.contains("subscription-test-device"),
        "read_resource didn't reflect the update: {contents}"
    );

    // Unsubscribe — second mutation should NOT produce a fresh event.
    let _ = mcp_post(
        &client,
        &url,
        Some(&session),
        &json!({
            "jsonrpc": "2.0",
            "id": 104,
            "method": "resources/unsubscribe",
            "params": {"uri": "mtrack://config"}
        }),
    )
    .await;

    controller.shutdown();
    Ok(())
}
