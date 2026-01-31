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

use std::{
    collections::HashSet,
    error::Error,
    io,
    net::{AddrParseError, Ipv4Addr, SocketAddr, SocketAddrV4},
    sync::Arc,
    time::Duration,
};

use rosc::{
    address::{Matcher, OscAddress},
    OscMessage, OscPacket, OscType,
};
use tokio::{
    net::UdpSocket,
    select,
    sync::{
        mpsc::{self, Receiver, Sender},
        Mutex,
    },
    task::JoinHandle,
};
use tracing::{error, info, span, Level};

use crate::{config, player::Player, util};

/// This is the all hosts multicast address.
const BROADCAST_SLEEP_DURATION: Duration = Duration::from_millis(500);

/// Player status strings.
const STATUS_STOPPED: &str = "Stopped";
const STATUS_PLAYING: &str = "Playing";

/// A controller that controls a player using OSC.
pub struct Driver {
    /// The player.
    player: Arc<Player>,
    /// The socket address to host the OSC server on.
    addr: SocketAddr,
    /// The addresses to broadcast OSC status to.
    broadcast_addresses: Vec<SocketAddr>,
    /// OSC events.
    osc_events: Arc<OscEvents>,
}

pub(super) struct OscEvents {
    /// The OSC address to look for to play the current song in the playlist.
    play: Matcher,
    /// The OSC address to look for to move the playlist to the previous item.
    prev: Matcher,
    /// The OSC address to look for to move the playlist to the next item.
    next: Matcher,
    /// The OSC address to look for to stop playback.
    stop: Matcher,
    /// The OSC address to look for to switch from the current playlist to an all songs playlist.
    all_songs: Matcher,
    /// The OSC address to look for to switch back to the current playlist.
    playlist: Matcher,
    /// The OSC address to look for to stop all triggered samples.
    stop_samples: Matcher,
    /// The OSC address to use to broadcast the player status.
    status: String,
    /// The OSC address to use to broadcast the current playlist.
    playlist_current: String,
    /// The OSC address to use to broadcast the currently playing song.
    playlist_current_song: String,
    /// The OSC address to use to broadcast the currently playing song elapsed time.
    playlist_current_song_elapsed: String,
}

impl Driver {
    pub fn new(
        config: Box<config::OscController>,
        player: Arc<Player>,
    ) -> Result<Arc<Self>, Box<dyn Error>> {
        let addr: SocketAddr =
            SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, config.port()));
        let broadcast_addresses: Vec<SocketAddr> = config
            .broadcast_addresses()
            .iter()
            .map(|addr| addr.parse())
            .collect::<Result<Vec<SocketAddr>, AddrParseError>>()?;

        Ok(Arc::new(Driver {
            player,
            addr,
            broadcast_addresses,
            osc_events: Arc::new(OscEvents {
                play: Matcher::new(config.play().as_str())?,
                prev: Matcher::new(config.prev().as_str())?,
                next: Matcher::new(config.next().as_str())?,
                stop: Matcher::new(config.stop().as_str())?,
                all_songs: Matcher::new(config.all_songs().as_str())?,
                playlist: Matcher::new(config.playlist().as_str())?,
                stop_samples: Matcher::new(config.stop_samples().as_str())?,
                status: config.status(),
                playlist_current: config.playlist_current(),
                playlist_current_song: config.playlist_current_song(),
                playlist_current_song_elapsed: config.playlist_current_song_elapsed(),
            }),
        }))
    }
}

impl super::Driver for Driver {
    fn monitor_events(&self) -> JoinHandle<Result<(), std::io::Error>> {
        let addr = self.addr;
        let broadcast_addresses = self.broadcast_addresses.clone();
        let player = self.player.clone();
        let osc_events = self.osc_events.clone();

        tokio::spawn(async move {
            let span = span!(Level::INFO, "OSC Driver");
            let _enter = span.enter();

            info!("OSC driver started.");
            let socket = UdpSocket::bind(addr).await?;

            // We're allowed to send broadcast messages.
            socket.set_broadcast(true)?;
            for broadcast_addr in broadcast_addresses.iter() {
                let ip = broadcast_addr.ip();
                if ip.is_multicast() {
                    match ip {
                        std::net::IpAddr::V4(ipv4_addr) => {
                            socket.join_multicast_v4(ipv4_addr, Ipv4Addr::UNSPECIFIED)?
                        }
                        std::net::IpAddr::V6(ipv6_addr) => {
                            socket.join_multicast_v6(&ipv6_addr, 0)?
                        }
                    }
                }
            }
            let (rx_sender, mut rx_receiver) = mpsc::channel::<OscPacket>(10);
            let (tx_sender, tx_receiver) = mpsc::channel::<OscPacket>(10);
            let connected_clients: Arc<Mutex<HashSet<SocketAddr>>> =
                Arc::new(Mutex::new(HashSet::new()));

            tokio::spawn(Self::handle_udp_comms(
                socket,
                broadcast_addresses,
                connected_clients.clone(),
                rx_sender,
                tx_receiver,
            ));

            // Start the broadcast async task.
            {
                let player = player.clone();
                let tx_sender = tx_sender.clone();
                let osc_events = osc_events.clone();

                info!("Starting broadcast loop");
                tokio::spawn(async move {
                    loop {
                        if let Err(e) = Self::broadcast(&player, &osc_events, &tx_sender).await {
                            error!(err = e, "Error broadcasting player status");
                        }
                        tokio::time::sleep(BROADCAST_SLEEP_DURATION).await;
                    }
                });
            }

            loop {
                let packet = rx_receiver.recv().await;
                let tx_sender = tx_sender.clone();

                if let Some(packet) = packet {
                    if Self::handle_packet(&player, &osc_events, &packet)
                        .await
                        .map_err(|e| io::Error::other(e.to_string()))?
                    {
                        if let Err(e) = Self::broadcast(&player, &osc_events, &tx_sender).await {
                            error!(err = e, "Error broadcasting player status");
                        };
                    }
                }
            }
        })
    }
}

impl Driver {
    /// Handles UDP sending/receiving.
    pub(super) async fn handle_udp_comms(
        socket: UdpSocket,
        broadcast_addresses: Vec<SocketAddr>,
        connected_clients: Arc<Mutex<HashSet<SocketAddr>>>,
        rx_sender: Sender<OscPacket>,
        mut tx_receiver: Receiver<OscPacket>,
    ) {
        let mut buf = [0u8; rosc::decoder::MTU];

        // Handle all UDP communication in this loop. We want to be pretty resilient here,
        // as we don't want the program to fail if we run into spurious errors.
        loop {
            select! {
                result = socket.recv_from(&mut buf) => {
                    match result {
                        Ok((size, sender_addr)) => {
                            // Add the sender to the list of connected clients
                            {
                                let mut clients = connected_clients.lock().await;
                                clients.insert(sender_addr);
                            }
                            match rosc::decoder::decode_udp(&buf[..size]) {
                                Ok((_, packet)) => {
                                    if let Err(e) = rx_sender.send(packet).await {
                                        error!(err = e.to_string(), "Error sending packet on channel.");
                                    }
                                }
                                Err(e) => error!(err = e.to_string(), "Error decoding OSC message"),
                            }
                        },
                        Err(e) => error!(err = e.to_string(), "Error receiving UDP."),
                    }
                }
                packet = tx_receiver.recv() => {
                    if let Some(packet) = packet {
                        match rosc::encoder::encode(&packet) {
                            Ok(buf) => {
                                // Send to configured broadcast addresses
                                for addr in broadcast_addresses.iter() {
                                    if let Err(e) = socket.send_to(&buf, addr).await {
                                        error!(err = e.to_string(), "Error sending UDP data.");
                                    }
                                }
                                // Send to all connected clients
                                let clients = connected_clients.lock().await;
                                for addr in clients.iter() {
                                    if let Err(e) = socket.send_to(&buf, addr).await {
                                        error!(err = e.to_string(), "Error sending UDP data to client.");
                                    }
                                }
                            }
                            Err(e) => error!(err = e.to_string(), "Error encoding OSC message"),
                        };
                    }
                }
            };
        }
    }

    /// Broadcasts various information to OSC clients.
    pub(super) async fn broadcast(
        player: &Arc<Player>,
        osc_events: &Arc<OscEvents>,
        tx_sender: &Sender<OscPacket>,
    ) -> Result<(), Box<dyn Error>> {
        let playlist = player.get_playlist();
        let song = playlist.current();
        let song_name = song.name();

        // Output the current song.
        let current_song_packet = OscPacket::Message(OscMessage {
            addr: osc_events.playlist_current_song.clone(),
            args: vec![OscType::String(song_name.to_string())],
        });
        tx_sender.send(current_song_packet).await?;

        // Output the current playing status.
        let elapsed = player.elapsed().await?;
        let status_string = match elapsed {
            Some(_) => STATUS_PLAYING,
            None => STATUS_STOPPED,
        };
        tx_sender
            .send(OscPacket::Message(OscMessage {
                addr: osc_events.status.clone(),
                args: vec![OscType::String(status_string.to_string())],
            }))
            .await?;

        let duration_string = format!(
            "{}/{}",
            util::duration_minutes_seconds(elapsed.unwrap_or_default()),
            util::duration_minutes_seconds(song.duration())
        );
        tx_sender
            .send(OscPacket::Message(OscMessage {
                addr: osc_events.playlist_current_song_elapsed.clone(),
                args: vec![OscType::String(duration_string)],
            }))
            .await?;

        // Output the actual current playlist contents.
        let playlist_content: String = playlist
            .songs()
            .iter()
            .enumerate()
            .map(|(i, song)| format!("{}. {}", i + 1, song))
            .collect::<Vec<String>>()
            .join("\n");
        tx_sender
            .send(OscPacket::Message(OscMessage {
                addr: osc_events.playlist_current.clone(),
                args: vec![OscType::String(playlist_content)],
            }))
            .await?;

        Ok(())
    }

    /// Handles incoming OSC packets. Meant for responding to things like player
    /// commands (play, previous, next, stop).
    pub(super) async fn handle_packet(
        player: &Arc<Player>,
        osc_events: &Arc<OscEvents>,
        packet: &OscPacket,
    ) -> Result<bool, Box<dyn Error>> {
        match packet {
            OscPacket::Message(osc_message) => {
                Box::pin(Self::handle_message(player, osc_events, osc_message)).await
            }
            OscPacket::Bundle(osc_bundle) => {
                let mut recognized_event = false;
                for packet in &osc_bundle.content {
                    recognized_event = recognized_event
                        || Box::pin(Self::handle_packet(player, osc_events, packet)).await?;
                }

                Ok(recognized_event)
            }
        }
    }

    /// Handles individual OSC messages.
    pub(super) async fn handle_message(
        player: &Arc<Player>,
        osc_events: &Arc<OscEvents>,
        msg: &OscMessage,
    ) -> Result<bool, Box<dyn Error>> {
        let address = OscAddress::new(msg.addr.clone())?;
        let mut recognized_event = false;
        if osc_events.play.match_address(&address) {
            if let Err(e) = player.play().await {
                error!(err = e.as_ref(), "Failed to play song: {}", e);
            }
            recognized_event = true;
        } else if osc_events.prev.match_address(&address) {
            player.prev().await;
            recognized_event = true;
        } else if osc_events.next.match_address(&address) {
            player.next().await;
            recognized_event = true;
        } else if osc_events.stop.match_address(&address) {
            player.stop().await;
            recognized_event = true;
        } else if osc_events.all_songs.match_address(&address) {
            player.switch_to_all_songs().await;
            recognized_event = true;
        } else if osc_events.playlist.match_address(&address) {
            player.switch_to_playlist().await;
            recognized_event = true;
        } else if osc_events.stop_samples.match_address(&address) {
            player.stop_samples();
            recognized_event = true;
        }

        Ok(recognized_event)
    }
}

#[cfg(test)]
mod test {
    use std::{
        collections::HashMap, error::Error, net::SocketAddr, path::Path, sync::Arc, time::Duration,
    };

    use rosc::{OscMessage, OscPacket, OscType};
    use tokio::{net::UdpSocket, sync::mpsc, time::timeout};

    use crate::{
        config,
        controller::osc::{Driver, STATUS_PLAYING, STATUS_STOPPED},
        playlist::Playlist,
        songs,
        testutil::{eventually, eventually_async},
    };

    use super::Player;

    #[tokio::test(flavor = "multi_thread")]
    async fn test_osc() -> Result<(), Box<dyn Error>> {
        let songs = songs::get_all_songs(Path::new("assets/songs"))?;
        let player = Arc::new(Player::new(
            songs.clone(),
            Playlist::new(
                "playlist",
                &config::Playlist::deserialize(Path::new("assets/playlist.yaml"))?,
                songs,
            )?,
            &config::Player::new(
                vec![],
                config::Audio::new("mock-device"),
                Some(config::Midi::new("mock-midi-device", None)),
                None,
                HashMap::new(),
                "assets/songs",
            ),
            None,
        )?);
        let binding = player.audio_device();
        let device = binding.to_mock()?;

        let driver = Driver::new(Box::new(config::OscController::new()), player.clone())?;
        let next = driver.osc_events.next.pattern.clone();
        let prev = driver.osc_events.prev.pattern.clone();
        let play = driver.osc_events.play.pattern.clone();
        let stop = driver.osc_events.stop.pattern.clone();
        let all_songs = driver.osc_events.all_songs.pattern.clone();
        let playlist = driver.osc_events.playlist.pattern.clone();

        let osc_packet = |address| async {
            let packet = osc_event(address);
            Driver::handle_packet(&player, &driver.osc_events, &packet).await
        };

        // Direct the player.
        println!("Playlist -> Song 1");
        assert_eq!(player.get_playlist().current().name(), "Song 1");

        osc_packet(next.clone()).await?;
        println!("Playlist -> Song 3");
        assert_eq!(player.get_playlist().current().name(), "Song 3");

        osc_packet(prev.clone()).await?;
        println!("Playlist -> Song 1");
        assert_eq!(player.get_playlist().current().name(), "Song 1");

        println!("Switch to AllSongs");
        osc_packet(all_songs.clone()).await?;
        assert_eq!(player.get_playlist().current().name(), "Song 1");

        osc_packet(next.clone()).await?;
        println!("AllSongs -> Song 10");
        assert_eq!(player.get_playlist().current().name(), "Song 10");

        osc_packet(next.clone()).await?;
        println!("AllSongs -> Song 2");
        assert_eq!(player.get_playlist().current().name(), "Song 2");

        osc_packet(next.clone()).await?;
        println!("AllSongs -> Song 3");
        assert_eq!(player.get_playlist().current().name(), "Song 3");

        osc_packet(playlist.clone()).await?;
        println!("Switch to Playlist");
        assert_eq!(player.get_playlist().current().name(), "Song 1");

        osc_packet(next.clone()).await?;
        println!("Playlist -> Song 3");
        assert_eq!(player.get_playlist().current().name(), "Song 3");

        osc_packet(play.clone()).await?;

        // Playlist should have moved to next song.
        eventually(
            || player.get_playlist().current().name() == "Song 5",
            format!(
                "Song never moved to next, on song {}",
                player.get_playlist().current().name()
            )
            .as_str(),
        );

        // Play a song and cancel it.
        osc_packet(play.clone()).await?;
        println!("Play Song 5.");
        eventually(|| device.is_playing(), "Song never started playing");

        let (tx_sender, mut tx_receiver) = mpsc::channel::<OscPacket>(10);
        Driver::broadcast(&player, &driver.osc_events, &tx_sender).await?;

        let mut buf: Vec<OscPacket> = Vec::new();
        tx_receiver.recv_many(&mut buf, 10).await;

        assert_eq!(
            buf[1],
            OscPacket::Message(OscMessage {
                addr: driver.osc_events.status.clone(),
                args: vec![OscType::String(STATUS_PLAYING.to_string())],
            })
        );

        osc_packet(stop.clone()).await?;
        eventually(|| !device.is_playing(), "Song never stopped playing");

        // Wait for player's internal state to update as well
        eventually_async(
            || async { player.elapsed().await.map(|e| e.is_none()).unwrap_or(false) },
            "Player state never updated to stopped",
        )
        .await;

        // Player should not have moved to the next song.
        assert_eq!(player.get_playlist().current().name(), "Song 5");

        Driver::broadcast(&player, &driver.osc_events, &tx_sender).await?;

        let mut buf: Vec<OscPacket> = Vec::new();
        tx_receiver.recv_many(&mut buf, 10).await;

        assert_eq!(buf.len(), 4);
        assert_eq!(
            buf[0],
            OscPacket::Message(OscMessage {
                addr: driver.osc_events.playlist_current_song.clone(),
                args: vec![OscType::String("Song 5".to_string())],
            })
        );
        assert_eq!(
            buf[1],
            OscPacket::Message(OscMessage {
                addr: driver.osc_events.status.clone(),
                args: vec![OscType::String(STATUS_STOPPED.to_string())],
            })
        );
        assert_eq!(
            buf[2],
            OscPacket::Message(OscMessage {
                addr: driver.osc_events.playlist_current_song_elapsed.clone(),
                args: vec![OscType::String("0:00/0:20".to_string())],
            })
        );
        assert_eq!(
            buf[3],
            OscPacket::Message(OscMessage {
                addr: driver.osc_events.playlist_current.clone(),
                args: vec![OscType::String(
                    "1. Song 1\n2. Song 3\n3. Song 5\n4. Song 7\n5. Song 9".to_string()
                )],
            })
        );

        Ok(())
    }

    fn osc_event(addr: String) -> OscPacket {
        OscPacket::Message(OscMessage { addr, args: vec![] })
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_osc_client_tracking() -> Result<(), Box<dyn Error>> {
        // Set up a player (not used directly, but needed for Driver initialization)
        let songs = songs::get_all_songs(Path::new("assets/songs"))?;
        let _player = Arc::new(Player::new(
            songs.clone(),
            Playlist::new(
                "playlist",
                &config::Playlist::deserialize(Path::new("assets/playlist.yaml"))?,
                songs,
            )?,
            &config::Player::new(
                vec![],
                config::Audio::new("mock-device"),
                Some(config::Midi::new("mock-midi-device", None)),
                None,
                HashMap::new(),
                "assets/songs",
            ),
            None,
        )?);

        // Test the client tracking by directly testing the handle_udp_comms logic
        // with a controlled setup

        // Create test UDP sockets
        let server_socket = UdpSocket::bind("127.0.0.1:0").await?;
        let server_addr = server_socket.local_addr()?;

        let client_socket = UdpSocket::bind("127.0.0.1:0").await?;
        let client_addr = client_socket.local_addr()?;

        // Set up channels
        let (rx_sender, _rx_receiver) = mpsc::channel::<OscPacket>(10);
        let (tx_sender, tx_receiver) = mpsc::channel::<OscPacket>(10);

        // Create connected clients tracker
        let connected_clients: Arc<tokio::sync::Mutex<std::collections::HashSet<SocketAddr>>> =
            Arc::new(tokio::sync::Mutex::new(std::collections::HashSet::new()));

        // Spawn the UDP handler
        let handler_clients = connected_clients.clone();
        let handler_socket = server_socket;
        let handler_task = tokio::spawn(async move {
            Driver::handle_udp_comms(
                handler_socket,
                vec![], // No broadcast addresses for this test
                handler_clients,
                rx_sender,
                tx_receiver,
            )
            .await;
        });

        // Send a message from the client to the server
        let test_message = OscPacket::Message(OscMessage {
            addr: "/test".to_string(),
            args: vec![],
        });
        let encoded = rosc::encoder::encode(&test_message)?;
        client_socket.send_to(&encoded, server_addr).await?;

        // Wait a bit for the message to be processed
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Verify the client was added to the connected clients list
        let clients = connected_clients.lock().await;
        assert!(
            clients.contains(&client_addr),
            "Client address should be in connected clients list"
        );
        drop(clients);

        // Send a broadcast packet
        let broadcast_packet = OscPacket::Message(OscMessage {
            addr: "/status".to_string(),
            args: vec![OscType::String("test".to_string())],
        });
        tx_sender.send(broadcast_packet).await?;

        // Wait a bit for the broadcast to be sent
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Try to receive the broadcast on the client socket
        let mut recv_buf = [0u8; 1024];
        match timeout(
            Duration::from_secs(1),
            client_socket.recv_from(&mut recv_buf),
        )
        .await
        {
            Ok(Ok((size, _))) => {
                // Decode the received packet
                match rosc::decoder::decode_udp(&recv_buf[..size]) {
                    Ok((_, packet)) => {
                        // Verify it's the status message we sent
                        if let OscPacket::Message(msg) = packet {
                            assert_eq!(msg.addr, "/status");
                            assert_eq!(msg.args.len(), 1);
                            if let OscType::String(s) = &msg.args[0] {
                                assert_eq!(s, "test");
                            } else {
                                panic!("Expected string argument");
                            }
                        } else {
                            panic!("Expected OSC message");
                        }
                    }
                    Err(e) => panic!("Failed to decode received packet: {}", e),
                }
            }
            Ok(Err(e)) => panic!("Failed to receive broadcast: {}", e),
            Err(_) => panic!("Timeout waiting for broadcast message"),
        }

        // Clean up
        handler_task.abort();

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_osc_multiple_clients() -> Result<(), Box<dyn Error>> {
        // Test that multiple clients can connect and all receive broadcasts
        let songs = songs::get_all_songs(Path::new("assets/songs"))?;
        let _player = Arc::new(Player::new(
            songs.clone(),
            Playlist::new(
                "playlist",
                &config::Playlist::deserialize(Path::new("assets/playlist.yaml"))?,
                songs,
            )?,
            &config::Player::new(
                vec![],
                config::Audio::new("mock-device"),
                Some(config::Midi::new("mock-midi-device", None)),
                None,
                HashMap::new(),
                "assets/songs",
            ),
            None,
        )?);

        // Create test UDP sockets
        let server_socket = UdpSocket::bind("127.0.0.1:0").await?;
        let server_addr = server_socket.local_addr()?;

        let client1_socket = UdpSocket::bind("127.0.0.1:0").await?;
        let client1_addr = client1_socket.local_addr()?;

        let client2_socket = UdpSocket::bind("127.0.0.1:0").await?;
        let client2_addr = client2_socket.local_addr()?;

        // Set up channels
        let (rx_sender, _rx_receiver) = mpsc::channel::<OscPacket>(10);
        let (tx_sender, tx_receiver) = mpsc::channel::<OscPacket>(10);

        // Create connected clients tracker
        let connected_clients: Arc<tokio::sync::Mutex<std::collections::HashSet<SocketAddr>>> =
            Arc::new(tokio::sync::Mutex::new(std::collections::HashSet::new()));

        // Spawn the UDP handler
        let handler_clients = connected_clients.clone();
        let handler_socket = server_socket;
        let handler_task = tokio::spawn(async move {
            Driver::handle_udp_comms(
                handler_socket,
                vec![],
                handler_clients,
                rx_sender,
                tx_receiver,
            )
            .await;
        });

        // Send messages from both clients
        let test_message = OscPacket::Message(OscMessage {
            addr: "/test".to_string(),
            args: vec![],
        });
        let encoded = rosc::encoder::encode(&test_message)?;
        client1_socket.send_to(&encoded, server_addr).await?;
        client2_socket.send_to(&encoded, server_addr).await?;

        // Wait for messages to be processed
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Verify both clients are in the list
        let clients = connected_clients.lock().await;
        assert!(
            clients.contains(&client1_addr),
            "Client 1 should be in connected clients list"
        );
        assert!(
            clients.contains(&client2_addr),
            "Client 2 should be in connected clients list"
        );
        assert_eq!(clients.len(), 2, "Should have exactly 2 clients");
        drop(clients);

        // Send a broadcast packet
        let broadcast_packet = OscPacket::Message(OscMessage {
            addr: "/broadcast".to_string(),
            args: vec![OscType::String("test".to_string())],
        });
        tx_sender.send(broadcast_packet).await?;

        // Wait for broadcasts to be sent
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Both clients should receive the broadcast
        let mut recv_buf1 = [0u8; 1024];
        let mut recv_buf2 = [0u8; 1024];

        // Client 1 should receive it
        match timeout(
            Duration::from_secs(1),
            client1_socket.recv_from(&mut recv_buf1),
        )
        .await
        {
            Ok(Ok((size, _))) => {
                let decoded = rosc::decoder::decode_udp(&recv_buf1[..size])?;
                if let OscPacket::Message(msg) = decoded.1 {
                    assert_eq!(msg.addr, "/broadcast");
                }
            }
            _ => panic!("Client 1 should receive broadcast"),
        }

        // Client 2 should also receive it
        match timeout(
            Duration::from_secs(1),
            client2_socket.recv_from(&mut recv_buf2),
        )
        .await
        {
            Ok(Ok((size, _))) => {
                let decoded = rosc::decoder::decode_udp(&recv_buf2[..size])?;
                if let OscPacket::Message(msg) = decoded.1 {
                    assert_eq!(msg.addr, "/broadcast");
                }
            }
            _ => panic!("Client 2 should receive broadcast"),
        }

        // Clean up
        handler_task.abort();

        Ok(())
    }
}
