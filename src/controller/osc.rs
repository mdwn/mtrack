// Copyright (C) 2025 Michael Wilson <mike@mdwn.dev>
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

use std::{error::Error, io, net::SocketAddr, sync::Arc};

use rosc::{
    address::{Matcher, OscAddress},
    OscMessage, OscPacket,
};
use tokio::{net::UdpSocket, task::JoinHandle};
use tracing::{info, span, Level};

use crate::{config, player::Player};

/// A controller that controls a player using OSC.
pub struct Driver {
    /// The player.
    player: Arc<Player>,
    /// The socket address to host the OSC server on.
    addr: SocketAddr,
    /// OSC events.
    osc_events: Arc<OscEvents>,
}

struct OscEvents {
    /// The OSC address to look for to play the current song in the playlist.
    play: Option<Matcher>,
    /// The OSC address to look for to move the playlist to the previous item.
    prev: Option<Matcher>,
    /// The OSC address to look for to move the playlist to the next item.
    next: Option<Matcher>,
    /// The OSC address to look for to stop playback.
    stop: Option<Matcher>,
    /// The OSC address to look for to switch from the current playlist to an all songs playlist.
    all_songs: Option<Matcher>,
    /// The OSC address to look for to switch back to the current playlist.
    playlist: Option<Matcher>,
}

impl Driver {
    pub fn new(
        config: config::OscController,
        player: Arc<Player>,
    ) -> Result<Arc<Self>, Box<dyn Error>> {
        let addr: SocketAddr = format!("0.0.0.0:{}", config.port()).parse()?;

        Ok(Arc::new(Driver {
            player,
            addr,
            osc_events: Arc::new(OscEvents {
                play: to_matcher(config.play())?,
                prev: to_matcher(config.prev())?,
                next: to_matcher(config.next())?,
                stop: to_matcher(config.stop())?,
                all_songs: to_matcher(config.all_songs())?,
                playlist: to_matcher(config.playlist())?,
            }),
        }))
    }
}

impl super::Driver for Driver {
    fn monitor_events(&self) -> JoinHandle<Result<(), std::io::Error>> {
        let addr = self.addr;
        let player = self.player.clone();
        let osc_events = self.osc_events.clone();

        tokio::spawn(async move {
            let span = span!(Level::INFO, "OSC Driver");
            let _enter = span.enter();

            info!("OSC driver started.");
            let socket = UdpSocket::bind(addr).await?;
            let mut buf = [0u8; rosc::decoder::MTU];

            loop {
                let (size, _) = socket.recv_from(&mut buf).await?;
                let (_, packet) = rosc::decoder::decode_udp(&buf[..size])
                    .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

                Self::handle_packet(&player, &osc_events, &packet)
                    .await
                    .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;
            }
        })
    }
}

impl Driver {
    async fn handle_packet(
        player: &Arc<Player>,
        osc_events: &Arc<OscEvents>,
        packet: &OscPacket,
    ) -> Result<(), Box<dyn Error>> {
        match packet {
            OscPacket::Message(osc_message) => {
                Box::pin(Self::handle_message(player, osc_events, osc_message)).await
            }
            OscPacket::Bundle(osc_bundle) => {
                for packet in &osc_bundle.content {
                    Box::pin(Self::handle_packet(player, osc_events, packet)).await?
                }

                Ok(())
            }
        }
    }
    async fn handle_message(
        player: &Arc<Player>,
        osc_events: &Arc<OscEvents>,
        msg: &OscMessage,
    ) -> Result<(), Box<dyn Error>> {
        let address = OscAddress::new(msg.addr.clone())?;
        if match_address(osc_events.play.as_ref(), &address) {
            player.play().await;
        } else if match_address(osc_events.prev.as_ref(), &address) {
            player.prev().await;
        } else if match_address(osc_events.next.as_ref(), &address) {
            player.next().await;
        } else if match_address(osc_events.stop.as_ref(), &address) {
            player.stop().await;
        } else if match_address(osc_events.all_songs.as_ref(), &address) {
            player.switch_to_all_songs().await;
        } else if match_address(osc_events.playlist.as_ref(), &address) {
            player.switch_to_playlist().await;
        }

        Ok(())
    }
}

/// Converts a string to a Matcher.
fn to_matcher(address: Option<String>) -> Result<Option<Matcher>, Box<dyn Error>> {
    Ok(match address {
        Some(address) => Some(Matcher::new(address.as_str())?),
        None => None,
    })
}

/// Matches the given matcher against the address if it exists.
fn match_address(matcher: Option<&Matcher>, address: &OscAddress) -> bool {
    matcher.is_some_and(|matcher| matcher.match_address(address))
}
