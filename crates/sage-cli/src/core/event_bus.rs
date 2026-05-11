//! Simple in-process event bus.
//!
//! Translated from pi-mono `packages/coding-agent/src/core/event-bus.ts`.
//!
//! The TypeScript version wraps Node.js `EventEmitter`. In Rust we use a
//! `HashMap<channel, Vec<handler>>` protected by a `Mutex`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde_json::Value;

// ============================================================================
// Types
// ============================================================================

pub type ChannelHandler = Box<dyn Fn(&Value) + Send + Sync>;

/// A simple publish/subscribe event bus.
pub trait EventBus: Send + Sync {
    /// Publish `data` to `channel`. All registered handlers are called.
    fn emit(&self, channel: &str, data: &Value);

    /// Subscribe to `channel`. Returns an unsubscribe ID.
    fn on(&self, channel: &str, handler: ChannelHandler) -> u64;

    /// Unsubscribe a previously registered handler.
    fn off(&self, channel: &str, id: u64);
}

pub trait EventBusController: EventBus {
    /// Remove all registered handlers.
    fn clear(&self);
}

// ============================================================================
// Default implementation
// ============================================================================

struct HandlerEntry {
    id: u64,
    handler: ChannelHandler,
}

struct Inner {
    handlers: HashMap<String, Vec<HandlerEntry>>,
    next_id: u64,
}

/// Default event bus implementation — wraps a `Mutex<Inner>`.
pub struct DefaultEventBus {
    inner: Mutex<Inner>,
}

impl DefaultEventBus {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(Inner {
                handlers: HashMap::new(),
                next_id: 0,
            }),
        })
    }
}

impl Default for DefaultEventBus {
    fn default() -> Self {
        Self {
            inner: Mutex::new(Inner {
                handlers: HashMap::new(),
                next_id: 0,
            }),
        }
    }
}

impl EventBus for DefaultEventBus {
    fn emit(&self, channel: &str, data: &Value) {
        let inner = self.inner.lock().unwrap();
        if let Some(entries) = inner.handlers.get(channel) {
            for entry in entries {
                // Errors in handlers are swallowed (matching TS behaviour)
                (entry.handler)(data);
            }
        }
    }

    fn on(&self, channel: &str, handler: ChannelHandler) -> u64 {
        let mut inner = self.inner.lock().unwrap();
        let id = inner.next_id;
        inner.next_id += 1;
        inner
            .handlers
            .entry(channel.to_string())
            .or_default()
            .push(HandlerEntry { id, handler });
        id
    }

    fn off(&self, channel: &str, id: u64) {
        let mut inner = self.inner.lock().unwrap();
        if let Some(entries) = inner.handlers.get_mut(channel) {
            entries.retain(|e| e.id != id);
        }
    }
}

impl EventBusController for DefaultEventBus {
    fn clear(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.handlers.clear();
    }
}

/// Convenience constructor — mirrors `createEventBus()` from TypeScript.
pub fn create_event_bus() -> Arc<DefaultEventBus> {
    DefaultEventBus::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn emit_calls_handler() {
        let bus = create_event_bus();
        let count = Arc::new(AtomicUsize::new(0));
        let count_clone = Arc::clone(&count);

        bus.on(
            "test",
            Box::new(move |_| {
                count_clone.fetch_add(1, Ordering::SeqCst);
            }),
        );

        bus.emit("test", &Value::Null);
        bus.emit("test", &Value::Null);

        assert_eq!(count.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn off_removes_handler() {
        let bus = create_event_bus();
        let count = Arc::new(AtomicUsize::new(0));
        let count_clone = Arc::clone(&count);

        let id = bus.on(
            "test",
            Box::new(move |_| {
                count_clone.fetch_add(1, Ordering::SeqCst);
            }),
        );

        bus.emit("test", &Value::Null);
        bus.off("test", id);
        bus.emit("test", &Value::Null);

        assert_eq!(count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn clear_removes_all_handlers() {
        let bus = create_event_bus();
        let count = Arc::new(AtomicUsize::new(0));
        let count_clone = Arc::clone(&count);

        bus.on(
            "test",
            Box::new(move |_| {
                count_clone.fetch_add(1, Ordering::SeqCst);
            }),
        );

        bus.clear();
        bus.emit("test", &Value::Null);

        assert_eq!(count.load(Ordering::SeqCst), 0);
    }
}
