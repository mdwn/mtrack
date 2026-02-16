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
