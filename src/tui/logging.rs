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

        {
            let mut buffer = self.buffer.lock();
            if buffer.len() >= self.capacity {
                buffer.pop_front();
            }
            buffer.push_back(line);
        }
    }
}
