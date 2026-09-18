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

//! Reads DMX back from olad.
//!
//! mtrack streams frames into olad; nothing on this machine reads them
//! off the wire. But olad itself knows what it is outputting, and its web
//! server answers `GET /get_dmx?u=<universe>` with the universe's current
//! values. Polled at the effects loop's rate, that is a frame-capture sink
//! for the DMX checks (venue-exchange design §15.7): what mtrack *said* it
//! sent, as olad *received* it, one hop from the wire.

use std::time::{Duration, Instant};

use crate::outcome::CheckError;

/// One universe as olad reported it.
#[derive(Clone, Debug)]
pub struct Frame {
    pub at: Instant,
    /// 512 channel values; a universe olad has not been streamed yet reads
    /// as empty.
    pub data: Vec<u8>,
}

impl Frame {
    /// A channel's value, 1-based as the venue patches it.
    pub fn channel(&self, channel: u16) -> u8 {
        self.data
            .get(usize::from(channel).saturating_sub(1))
            .copied()
            .unwrap_or(0)
    }

    /// The 16-bit value of a coarse/fine pair.
    pub fn value16(&self, coarse: u16, fine: u16) -> u16 {
        u16::from_be_bytes([self.channel(coarse), self.channel(fine)])
    }
}

/// The Dummy Device's first output port in `ola_dev_info` output, as
/// `(device id, port id)`.
fn dummy_output_port(listing: &str) -> Option<(u32, u32)> {
    let mut device: Option<u32> = None;
    for line in listing.lines() {
        if let Some(rest) = line.strip_prefix("Device ") {
            let (id, name) = rest.split_once(':')?;
            device = if name.trim().starts_with("Dummy") {
                id.trim().parse().ok()
            } else {
                None
            };
            continue;
        }
        if let (Some(dev), Some(rest)) = (device, line.trim().strip_prefix("port ")) {
            let (port, kind) = rest.split_once(',')?;
            if kind.trim().starts_with("OUT") {
                return Some((dev, port.trim().parse().ok()?));
            }
        }
    }
    None
}

/// The readback client.
pub struct DmxSink {
    http: reqwest::Client,
    base: String,
}

impl DmxSink {
    /// A sink on olad's web server port.
    pub fn new(http_port: u16) -> DmxSink {
        DmxSink {
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(2))
                .build()
                .expect("reqwest client"),
            base: format!("http://127.0.0.1:{http_port}"),
        }
    }

    /// One frame of a universe.
    pub async fn read(&self, universe: u16) -> Result<Frame, CheckError> {
        #[derive(serde::Deserialize)]
        struct GetDmx {
            #[serde(default)]
            dmx: Vec<u8>,
            #[serde(default)]
            error: Option<String>,
        }
        let response = self
            .http
            .get(format!("{}/get_dmx?u={universe}", self.base))
            .send()
            .await
            .map_err(|e| CheckError::before_assertion(format!("olad readback: {e}")))?;
        let body: GetDmx = response
            .json()
            .await
            .map_err(|e| CheckError::before_assertion(format!("olad readback JSON: {e}")))?;
        // A live universe answers with `"error": ""`; only a non-empty
        // error means anything.
        if let Some(error) = body.error.filter(|e| !e.is_empty()) {
            // "Universe doesn't exist" until mtrack's first frame reaches it.
            if body.dmx.is_empty() {
                return Ok(Frame {
                    at: Instant::now(),
                    data: Vec::new(),
                });
            }
            return Err(CheckError::before_assertion(format!(
                "olad readback: {error}"
            )));
        }
        Ok(Frame {
            at: Instant::now(),
            data: body.dmx,
        })
    }

    /// Makes sure olad has the universe at all.
    ///
    /// olad drops streamed frames for a universe nobody has created — the
    /// streaming path never creates one — and a fresh olad has none, so
    /// mtrack's output would silently go nowhere and every readback would
    /// say "Universe doesn't exist". The Dummy plugin's output port exists
    /// for exactly this: patching it to the universe creates the universe,
    /// olad then keeps what is streamed, and nothing physical is driven.
    /// The patch is recorded, since it changes olad's state for the run.
    pub async fn ensure_universe(&self, universe: u16) -> Result<(), CheckError> {
        if !self.read(universe).await?.data.is_empty() {
            return Ok(());
        }
        let devices = std::process::Command::new("ola_dev_info")
            .output()
            .map_err(|e| {
                CheckError::before_assertion(format!(
                    "universe {universe} does not exist in olad and ola_dev_info is not \
                     runnable to patch one: {e}"
                ))
            })?;
        let listing = String::from_utf8_lossy(&devices.stdout);
        let (device, port) = dummy_output_port(&listing).ok_or_else(|| {
            CheckError::before_assertion(format!(
                "universe {universe} does not exist in olad and no Dummy Device output port \
                 is available to patch it to; patch a port to universe {universe} \
                 (ola_patch) and rerun.\n--- ola_dev_info ---\n{listing}"
            ))
        })?;
        let status = std::process::Command::new("ola_patch")
            .args([
                "-d",
                &device.to_string(),
                "-p",
                &port.to_string(),
                "-u",
                &universe.to_string(),
            ])
            .status()
            .map_err(|e| CheckError::before_assertion(format!("ola_patch: {e}")))?;
        if !status.success() {
            return Err(CheckError::before_assertion(format!(
                "ola_patch -d {device} -p {port} -u {universe} failed: {status}"
            )));
        }
        crate::outcome::record(format!(
            "caveat: olad had no universe {universe}, so the Dummy Device port {device}/{port} \
             was patched to it for this run — an unpatched olad drops mtrack's frames silently"
        ));
        // olad creates the universe on the patch; give it a moment.
        tokio::time::sleep(Duration::from_millis(200)).await;
        Ok(())
    }

    /// Frames of a universe for `window`, one every `period`.
    pub async fn record(
        &self,
        universe: u16,
        window: Duration,
        period: Duration,
    ) -> Result<Vec<Frame>, CheckError> {
        let started = Instant::now();
        let mut frames = Vec::new();
        while started.elapsed() < window {
            let frame = self.read(universe).await?;
            frames.push(frame);
            tokio::time::sleep(period).await;
        }
        Ok(frames)
    }

    /// Polls until the universe carries a nonzero byte at `channel`, or
    /// gives up.
    pub async fn wait_for_nonzero(
        &self,
        universe: u16,
        channel: u16,
        timeout: Duration,
    ) -> Result<Frame, CheckError> {
        let started = Instant::now();
        loop {
            let frame = self.read(universe).await?;
            if frame.channel(channel) != 0 {
                return Ok(frame);
            }
            if started.elapsed() > timeout {
                return Err(CheckError::assertion(format!(
                    "universe {universe} channel {channel} stayed at 0 for {timeout:?}; \
                     last frame had {} channels",
                    frame.data.len()
                )));
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dummy_output_port_is_found_in_a_device_listing() {
        let listing = "Device 1: Dummy Device\n  port 0, OUT Dummy Port, RDM supported\n\
                       Device 2: ArtNet [192.168.1.216]\n  port 0, IN, priority 100\n  port 0, OUT\n";
        assert_eq!(dummy_output_port(listing), Some((1, 0)));
        let no_dummy = "Device 2: ArtNet [192.168.1.216]\n  port 0, OUT\n";
        assert_eq!(dummy_output_port(no_dummy), None);
    }
}
