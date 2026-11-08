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

//! A loopback TCP port nothing listens on, held for as long as a test needs
//! it: bound, so no other test or server can take it, and never listening,
//! so a connection to it is refused. A port picked by binding one and letting
//! it go could be taken by something else before the test used it (#668); a
//! server under test binds port 0 instead, and says which port it got.

/// A loopback TCP port that refuses connections until this is dropped.
pub struct ClosedPort {
    /// Bound, not listening: what holds the port.
    _socket: tokio::net::TcpSocket,
    port: u16,
}

impl ClosedPort {
    pub fn bind() -> ClosedPort {
        let socket = tokio::net::TcpSocket::new_v4().expect("socket");
        socket
            .bind((std::net::Ipv4Addr::LOCALHOST, 0).into())
            .expect("bind");
        let port = socket.local_addr().expect("local_addr").port();
        ClosedPort {
            _socket: socket,
            port,
        }
    }

    pub fn port(&self) -> u16 {
        self.port
    }
}
