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
use parking_lot::Mutex;
use std::collections::VecDeque;
use std::fmt;
use std::sync::{Arc, OnceLock};
use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::Context;
use tracing_subscriber::Layer;

/// Global log buffer shared between the TUI layer and the TUI renderer.
static LOG_BUFFER: OnceLock<Arc<Mutex<VecDeque<String>>>> = OnceLock::new();

/// Returns the global log buffer for the TUI to read from.
pub fn get_log_buffer() -> Option<Arc<Mutex<VecDeque<String>>>> {
    LOG_BUFFER.get().cloned()
}

/// Initializes the TUI logging layer with the given ring buffer capacity.
/// Returns the layer to be installed into the tracing subscriber.
pub fn init_tui_logging(capacity: usize) -> TuiLogLayer {
    let buffer = Arc::new(Mutex::new(VecDeque::with_capacity(capacity)));
    LOG_BUFFER
        .set(buffer.clone())
        .expect("TUI logging already initialized");
    TuiLogLayer { buffer, capacity }
}

/// A tracing subscriber layer that captures log events into a ring buffer.
pub struct TuiLogLayer {
    buffer: Arc<Mutex<VecDeque<String>>>,
    capacity: usize,
}

/// Visitor that extracts the message field from a tracing event.
struct MessageVisitor {
    message: String,
}

impl MessageVisitor {
    fn new() -> Self {
        Self {
            message: String::new(),
        }
    }
}

impl Visit for MessageVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        if field.name() == "message" {
            self.message = format!("{:?}", value);
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message = value.to_string();
        }
    }
}

impl<S: Subscriber> Layer<S> for TuiLogLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let metadata = event.metadata();
        let level = metadata.level();
        let target = metadata.target();

        let mut visitor = MessageVisitor::new();
        event.record(&mut visitor);

        let line = format!("{} {}: {}", level, target, visitor.message);

        push_to_ring_buffer(&self.buffer, self.capacity, line);
    }
}

/// Pushes a line into the ring buffer, evicting the oldest entry if at capacity.
fn push_to_ring_buffer(buffer: &Arc<Mutex<VecDeque<String>>>, capacity: usize, line: String) {
    let mut buffer = buffer.lock();
    if buffer.len() >= capacity {
        buffer.pop_front();
    }
    buffer.push_back(line);
}

#[cfg(test)]
mod tests {
    use super::*;

    mod ring_buffer_tests {
        use super::*;

        fn make_buffer() -> Arc<Mutex<VecDeque<String>>> {
            Arc::new(Mutex::new(VecDeque::new()))
        }

        #[test]
        fn push_to_empty_buffer() {
            let buffer = make_buffer();
            push_to_ring_buffer(&buffer, 5, "line 1".to_string());
            let buf = buffer.lock();
            assert_eq!(buf.len(), 1);
            assert_eq!(buf[0], "line 1");
        }

        #[test]
        fn push_multiple_within_capacity() {
            let buffer = make_buffer();
            push_to_ring_buffer(&buffer, 3, "a".to_string());
            push_to_ring_buffer(&buffer, 3, "b".to_string());
            push_to_ring_buffer(&buffer, 3, "c".to_string());
            let buf = buffer.lock();
            assert_eq!(buf.len(), 3);
            assert_eq!(buf[0], "a");
            assert_eq!(buf[2], "c");
        }

        #[test]
        fn evicts_oldest_when_at_capacity() {
            let buffer = make_buffer();
            push_to_ring_buffer(&buffer, 2, "first".to_string());
            push_to_ring_buffer(&buffer, 2, "second".to_string());
            push_to_ring_buffer(&buffer, 2, "third".to_string());
            let buf = buffer.lock();
            assert_eq!(buf.len(), 2);
            assert_eq!(buf[0], "second");
            assert_eq!(buf[1], "third");
        }

        #[test]
        fn capacity_one_always_has_latest() {
            let buffer = make_buffer();
            push_to_ring_buffer(&buffer, 1, "a".to_string());
            push_to_ring_buffer(&buffer, 1, "b".to_string());
            push_to_ring_buffer(&buffer, 1, "c".to_string());
            let buf = buffer.lock();
            assert_eq!(buf.len(), 1);
            assert_eq!(buf[0], "c");
        }

        #[test]
        fn many_items_evict_correctly() {
            let buffer = make_buffer();
            for i in 0..100 {
                push_to_ring_buffer(&buffer, 5, format!("line {}", i));
            }
            let buf = buffer.lock();
            assert_eq!(buf.len(), 5);
            assert_eq!(buf[0], "line 95");
            assert_eq!(buf[4], "line 99");
        }
    }

    mod message_visitor_tests {
        use super::*;

        #[test]
        fn new_visitor_has_empty_message() {
            let visitor = MessageVisitor::new();
            assert!(visitor.message.is_empty());
        }
    }

    mod tui_log_layer_tests {
        use super::*;
        use tracing_subscriber::layer::SubscriberExt;
        use tracing_subscriber::Registry;

        #[test]
        fn layer_captures_info_event() {
            let buffer = Arc::new(Mutex::new(VecDeque::with_capacity(10)));
            let layer = TuiLogLayer {
                buffer: buffer.clone(),
                capacity: 10,
            };
            let subscriber = Registry::default().with(layer);
            let _guard = tracing::subscriber::set_default(subscriber);

            tracing::info!("hello world");

            let buf = buffer.lock();
            assert_eq!(buf.len(), 1);
            assert!(buf[0].contains("hello world"), "got: {}", buf[0]);
        }

        #[test]
        fn layer_captures_multiple_levels() {
            let buffer = Arc::new(Mutex::new(VecDeque::with_capacity(10)));
            let layer = TuiLogLayer {
                buffer: buffer.clone(),
                capacity: 10,
            };
            let subscriber = Registry::default().with(layer);
            let _guard = tracing::subscriber::set_default(subscriber);

            tracing::warn!("warning message");
            tracing::error!("error message");

            let buf = buffer.lock();
            assert_eq!(buf.len(), 2);
            assert!(buf[0].contains("WARN"), "got: {}", buf[0]);
            assert!(buf[1].contains("ERROR"), "got: {}", buf[1]);
        }

        #[test]
        fn layer_respects_capacity() {
            let buffer = Arc::new(Mutex::new(VecDeque::with_capacity(2)));
            let layer = TuiLogLayer {
                buffer: buffer.clone(),
                capacity: 2,
            };
            let subscriber = Registry::default().with(layer);
            let _guard = tracing::subscriber::set_default(subscriber);

            tracing::info!("first");
            tracing::info!("second");
            tracing::info!("third");

            let buf = buffer.lock();
            assert_eq!(buf.len(), 2);
            assert!(buf[0].contains("second"), "got: {}", buf[0]);
            assert!(buf[1].contains("third"), "got: {}", buf[1]);
        }

        #[test]
        fn layer_includes_target() {
            let buffer = Arc::new(Mutex::new(VecDeque::with_capacity(10)));
            let layer = TuiLogLayer {
                buffer: buffer.clone(),
                capacity: 10,
            };
            let subscriber = Registry::default().with(layer);
            let _guard = tracing::subscriber::set_default(subscriber);

            tracing::info!("test");

            let buf = buffer.lock();
            assert_eq!(buf.len(), 1);
            // Should contain the module target
            assert!(buf[0].contains("mtrack"), "got: {}", buf[0]);
        }
    }
}
