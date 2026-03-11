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

use crate::config;
use crate::lighting::parser::utils::parse_time_string;
use crate::proto::player::v1::player_service_client::PlayerServiceClient;
use crate::proto::player::v1::{
    GetActiveEffectsRequest, GetCuesRequest, NextRequest, PlayFromRequest, PlayRequest,
    PreviousRequest, Song, StatusRequest, StopRequest, SwitchToPlaylistRequest,
};
use crate::util;
use std::error::Error;
use std::time::Duration;
use tonic::transport::Channel;
use tonic::Request;

async fn connect(
    host_port: Option<String>,
) -> Result<PlayerServiceClient<Channel>, Box<dyn Error>> {
    // Use 127.0.0.1 for local client; 0.0.0.0 is a bind address, not valid for connect.
    let addr = host_port.unwrap_or_else(|| format!("127.0.0.1:{}", config::DEFAULT_GRPC_PORT));
    Ok(PlayerServiceClient::connect(format!("http://{}", addr)).await?)
}

fn print_song(song: Option<Song>) -> Result<(), Box<dyn Error>> {
    if let Some(song) = song {
        println!("Name: {}", song.name);
        println!(
            "Duration: {}",
            util::duration_minutes_seconds(Duration::try_from(song.duration.unwrap_or_default())?)
        );
        println!("Tracks:");
        for track in song.tracks {
            println!("  - {}", track);
        }
    }

    Ok(())
}

pub async fn play(host_port: Option<String>, from: Option<String>) -> Result<(), Box<dyn Error>> {
    let mut client = connect(host_port).await?;
    if let Some(from_str) = from {
        // Parse the time string
        let start_time = parse_time_string(&from_str)?;
        let start_duration = prost_types::Duration::try_from(start_time)
            .map_err(|e| format!("Failed to convert duration: {}", e))?;

        let response = client
            .play_from(Request::new(PlayFromRequest {
                start_time: Some(start_duration),
            }))
            .await?;
        println!("Playing the song from {}:", from_str);
        print_song(response.into_inner().song)?;
    } else {
        let response = client.play(Request::new(PlayRequest {})).await?;
        println!("Playing the song:");
        print_song(response.into_inner().song)?;
    }
    Ok(())
}

pub async fn previous(host_port: Option<String>) -> Result<(), Box<dyn Error>> {
    let mut client = connect(host_port).await?;
    let response = client.previous(Request::new(PreviousRequest {})).await?;
    println!("Moved to previous song:");
    print_song(response.into_inner().song)?;
    Ok(())
}

pub async fn next(host_port: Option<String>) -> Result<(), Box<dyn Error>> {
    let mut client = connect(host_port).await?;
    let response = client.next(Request::new(NextRequest {})).await?;
    println!("Moved to next song:");
    print_song(response.into_inner().song)?;
    Ok(())
}

pub async fn stop(host_port: Option<String>) -> Result<(), Box<dyn Error>> {
    let mut client = connect(host_port).await?;
    let response = client.stop(Request::new(StopRequest {})).await?;
    println!("The song was stopped:");
    print_song(response.into_inner().song)?;
    Ok(())
}

pub async fn switch_to_playlist(
    host_port: Option<String>,
    playlist_name: &str,
) -> Result<(), Box<dyn Error>> {
    let mut client = connect(host_port).await?;
    let _ = client
        .switch_to_playlist(Request::new(SwitchToPlaylistRequest {
            playlist_name: playlist_name.to_string(),
        }))
        .await?;
    println!("Switched to playlist {}", playlist_name);
    Ok(())
}

pub async fn status(host_port: Option<String>) -> Result<(), Box<dyn Error>> {
    let mut client = connect(host_port).await?;
    let response = client
        .status(Request::new(StatusRequest {}))
        .await?
        .into_inner();
    if let Some(song) = response.current_song {
        println!("Current song: {}", song.name);
        let song_duration =
            util::duration_minutes_seconds(Duration::try_from(song.duration.unwrap_or_default())?);
        let elapsed = Duration::try_from(response.elapsed.unwrap_or_default())
            .map(util::duration_minutes_seconds)?;
        println!("Elapsed: {}/{}", elapsed, song_duration);
    }
    println!("Playing: {}", response.playing);
    println!("Playlist name: {}", response.playlist_name);
    Ok(())
}

pub async fn active_effects(host_port: Option<String>) -> Result<(), Box<dyn Error>> {
    let mut client = connect(host_port).await?;
    let response = client
        .get_active_effects(Request::new(GetActiveEffectsRequest {}))
        .await?;
    println!("{}", response.into_inner().active_effects);
    Ok(())
}

pub async fn cues(host_port: Option<String>) -> Result<(), Box<dyn Error>> {
    let mut client = connect(host_port).await?;
    let response = client
        .get_cues(Request::new(GetCuesRequest {}))
        .await?
        .into_inner();

    if response.cues.is_empty() {
        println!("No cues found in the current song.");
    } else {
        println!("Cues in current song ({} total):", response.cues.len());
        for cue in response.cues {
            let time = cue
                .time
                .and_then(|d| Duration::try_from(d).ok())
                .map(util::duration_minutes_seconds)
                .unwrap_or_else(|| "unknown".to_string());
            println!("  {}: {} (index {})", cue.index, time, cue.index);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    mod print_song_tests {
        use super::*;

        #[test]
        fn none_song_is_ok() {
            assert!(print_song(None).is_ok());
        }

        #[test]
        fn song_with_fields() {
            let song = Song {
                name: "Test Song".to_string(),
                duration: Some(prost_types::Duration {
                    seconds: 180,
                    nanos: 0,
                }),
                tracks: vec!["guitar".to_string(), "bass".to_string()],
            };
            assert!(print_song(Some(song)).is_ok());
        }

        #[test]
        fn song_with_no_duration() {
            let song = Song {
                name: "No Duration".to_string(),
                duration: None,
                tracks: vec![],
            };
            assert!(print_song(Some(song)).is_ok());
        }

        #[test]
        fn song_with_empty_tracks() {
            let song = Song {
                name: "Empty".to_string(),
                duration: Some(prost_types::Duration {
                    seconds: 0,
                    nanos: 0,
                }),
                tracks: vec![],
            };
            assert!(print_song(Some(song)).is_ok());
        }
    }

    /// Returns an address with an OS-assigned ephemeral port that nothing is listening on.
    fn unused_addr() -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        addr.to_string()
    }

    mod connect_tests {
        use super::*;

        #[tokio::test]
        async fn connect_fails_when_no_server() {
            let result = connect(Some(unused_addr())).await;
            assert!(result.is_err());
        }
    }

    mod remote_command_tests {
        use super::*;

        #[tokio::test]
        async fn play_fails_without_server() {
            let result = play(Some(unused_addr()), None).await;
            assert!(result.is_err());
        }

        #[tokio::test]
        async fn play_from_fails_without_server() {
            let result = play(Some(unused_addr()), Some("0:30".to_string())).await;
            assert!(result.is_err());
        }

        #[tokio::test]
        async fn play_from_with_minutes_seconds_format() {
            let result = play(Some(unused_addr()), Some("1:23.456".to_string())).await;
            assert!(result.is_err());
        }

        #[tokio::test]
        async fn play_from_with_seconds_only_format() {
            let result = play(Some(unused_addr()), Some("45.5s".to_string())).await;
            assert!(result.is_err());
        }

        #[tokio::test]
        async fn next_fails_without_server() {
            let result = next(Some(unused_addr())).await;
            assert!(result.is_err());
        }

        #[tokio::test]
        async fn previous_fails_without_server() {
            let result = previous(Some(unused_addr())).await;
            assert!(result.is_err());
        }

        #[tokio::test]
        async fn stop_fails_without_server() {
            let result = stop(Some(unused_addr())).await;
            assert!(result.is_err());
        }

        #[tokio::test]
        async fn status_fails_without_server() {
            let result = status(Some(unused_addr())).await;
            assert!(result.is_err());
        }

        #[tokio::test]
        async fn switch_to_playlist_fails_without_server() {
            let result = switch_to_playlist(Some(unused_addr()), "all_songs").await;
            assert!(result.is_err());
        }

        #[tokio::test]
        async fn switch_to_playlist_with_different_name() {
            let result = switch_to_playlist(Some(unused_addr()), "playlist").await;
            assert!(result.is_err());
        }

        #[tokio::test]
        async fn active_effects_fails_without_server() {
            let result = active_effects(Some(unused_addr())).await;
            assert!(result.is_err());
        }

        #[tokio::test]
        async fn cues_fails_without_server() {
            let result = cues(Some(unused_addr())).await;
            assert!(result.is_err());
        }
    }

    mod print_song_edge_cases {
        use super::*;

        #[test]
        fn song_with_many_tracks() {
            let song = Song {
                name: "Multi Track Song".to_string(),
                duration: Some(prost_types::Duration {
                    seconds: 300,
                    nanos: 500_000_000,
                }),
                tracks: vec![
                    "guitar".to_string(),
                    "bass".to_string(),
                    "drums".to_string(),
                    "vocals".to_string(),
                    "keys".to_string(),
                ],
            };
            assert!(print_song(Some(song)).is_ok());
        }

        #[test]
        fn song_with_zero_duration() {
            let song = Song {
                name: "Zero Duration".to_string(),
                duration: Some(prost_types::Duration {
                    seconds: 0,
                    nanos: 0,
                }),
                tracks: vec!["track".to_string()],
            };
            assert!(print_song(Some(song)).is_ok());
        }

        #[test]
        fn song_with_sub_second_duration() {
            let song = Song {
                name: "Short".to_string(),
                duration: Some(prost_types::Duration {
                    seconds: 0,
                    nanos: 500_000_000,
                }),
                tracks: vec![],
            };
            assert!(print_song(Some(song)).is_ok());
        }

        #[test]
        fn song_with_long_duration() {
            let song = Song {
                name: "Long Song".to_string(),
                duration: Some(prost_types::Duration {
                    seconds: 3600,
                    nanos: 0,
                }),
                tracks: vec!["ambient".to_string()],
            };
            assert!(print_song(Some(song)).is_ok());
        }

        #[test]
        fn song_with_empty_name() {
            let song = Song {
                name: "".to_string(),
                duration: Some(prost_types::Duration {
                    seconds: 60,
                    nanos: 0,
                }),
                tracks: vec![],
            };
            assert!(print_song(Some(song)).is_ok());
        }
    }
}
