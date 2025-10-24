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

use std::error::Error;

/// Trait for OLA client functionality
pub trait OlaClient: Send + Sync {
    /// Send DMX data to a universe
    fn send_dmx(&mut self, universe: u32, buffer: &ola::DmxBuffer) -> Result<(), Box<dyn Error>>;
}

/// Real OLA client implementation
pub struct RealOlaClient {
    client: ola::StreamingClient<std::net::TcpStream>,
}

impl RealOlaClient {
    pub fn new(client: ola::StreamingClient<std::net::TcpStream>) -> Self {
        Self { client }
    }
}

impl OlaClient for RealOlaClient {
    fn send_dmx(&mut self, universe: u32, buffer: &ola::DmxBuffer) -> Result<(), Box<dyn Error>> {
        self.client.send_dmx(universe, buffer)?;
        Ok(())
    }
}

#[cfg(test)]
/// Mock OLA client for testing
pub struct MockOlaClient {
    pub sent_messages: std::sync::Arc<std::sync::Mutex<Vec<DmxMessage>>>,
    pub should_fail: bool,
}

#[cfg(test)]
#[derive(Debug, Clone)]
pub struct DmxMessage {
    pub universe: u32,
    pub buffer: ola::DmxBuffer,
}

#[cfg(test)]
impl MockOlaClient {
    pub fn new() -> Self {
        Self {
            sent_messages: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
            should_fail: false,
        }
    }

    /// Get the number of messages sent
    pub fn message_count(&self) -> usize {
        self.sent_messages.lock().unwrap().len()
    }

    /// Get the last sent message
    pub fn get_last_message(&self) -> Option<DmxMessage> {
        self.sent_messages.lock().unwrap().last().cloned()
    }

    /// Clear all sent messages
    pub fn clear_messages(&self) {
        self.sent_messages.lock().unwrap().clear();
    }

    /// Get messages for a specific universe
    pub fn get_messages_for_universe(&self, universe: u32) -> Vec<DmxMessage> {
        self.sent_messages
            .lock()
            .unwrap()
            .iter()
            .filter(|msg| msg.universe == universe)
            .cloned()
            .collect()
    }

    /// Get the DMX buffer for a specific universe from the last message
    pub fn get_buffer_for_universe(&self, universe: u32) -> Option<ola::DmxBuffer> {
        self.get_messages_for_universe(universe)
            .last()
            .map(|msg| msg.buffer.clone())
    }
}

#[cfg(test)]
impl OlaClient for MockOlaClient {
    fn send_dmx(&mut self, universe: u32, buffer: &ola::DmxBuffer) -> Result<(), Box<dyn Error>> {
        if self.should_fail {
            return Err("Mock OLA client failure".into());
        }

        let message = DmxMessage {
            universe,
            buffer: buffer.clone(),
        };
        self.sent_messages.lock().unwrap().push(message);
        Ok(())
    }
}

/// Factory for creating OLA clients
pub struct OlaClientFactory;

impl OlaClientFactory {
    /// Create a real OLA client (requires OLA to be running)
    pub fn create_real_client(
        config: ola::client::StreamingClientConfig,
    ) -> Result<Box<dyn OlaClient>, Box<dyn Error>> {
        let client = ola::connect_with_config(config)?;
        Ok(Box::new(RealOlaClient::new(client)))
    }

    #[cfg(test)]
    /// Create a mock OLA client for testing
    pub fn create_mock_client() -> Box<dyn OlaClient> {
        Box::new(MockOlaClient::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mock_ola_client_dmx_verification() {
        let mut mock_client = MockOlaClient::new();

        // Create a test DMX buffer
        let buffer = ola::DmxBuffer::new();

        // Send DMX data
        let result = mock_client.send_dmx(1, &buffer);
        assert!(result.is_ok());

        // Verify message was captured
        assert_eq!(mock_client.message_count(), 1);

        // Verify universe
        let last_message = mock_client.get_last_message().unwrap();
        assert_eq!(last_message.universe, 1);

        // Test universe filtering
        let universe_1_messages = mock_client.get_messages_for_universe(1);
        assert_eq!(universe_1_messages.len(), 1);

        let universe_2_messages = mock_client.get_messages_for_universe(2);
        assert_eq!(universe_2_messages.len(), 0);

        // Test buffer retrieval
        let retrieved_buffer = mock_client.get_buffer_for_universe(1).unwrap();
        assert_eq!(retrieved_buffer.len(), 512); // DMX buffer should be 512 channels
    }

    #[test]
    fn test_mock_ola_client_multiple_messages() {
        let mut mock_client = MockOlaClient::new();

        // Send first message
        let buffer1 = ola::DmxBuffer::new();
        mock_client.send_dmx(1, &buffer1).unwrap();

        // Send second message
        let buffer2 = ola::DmxBuffer::new();
        mock_client.send_dmx(1, &buffer2).unwrap();

        // Send message to different universe
        let buffer3 = ola::DmxBuffer::new();
        mock_client.send_dmx(2, &buffer3).unwrap();

        // Verify total messages
        assert_eq!(mock_client.message_count(), 3);

        // Verify universe-specific messages
        let universe_1_messages = mock_client.get_messages_for_universe(1);
        assert_eq!(universe_1_messages.len(), 2);

        let universe_2_messages = mock_client.get_messages_for_universe(2);
        assert_eq!(universe_2_messages.len(), 1);

        // Verify last message (should be universe 2)
        let last_message = mock_client.get_last_message().unwrap();
        assert_eq!(last_message.universe, 2);
    }

    #[test]
    fn test_mock_ola_client_clear_messages() {
        let mut mock_client = MockOlaClient::new();

        // Send a message
        let buffer = ola::DmxBuffer::new();
        mock_client.send_dmx(1, &buffer).unwrap();

        assert_eq!(mock_client.message_count(), 1);

        // Clear messages
        mock_client.clear_messages();
        assert_eq!(mock_client.message_count(), 0);
    }
}
