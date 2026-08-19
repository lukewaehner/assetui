//! Duplicate-suppression for in-flight detail fetches.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

type Key = (&'static str, String);

/// The set of fetches currently running, shared with the tasks themselves.
#[derive(Clone, Default)]
pub struct Inflight {
    keys: Arc<Mutex<HashSet<Key>>>,
}

impl Inflight {
    /// Claims `(kind, symbol)`, returning a guard that releases it on drop.
    pub fn begin(&self, kind: &'static str, symbol: &str) -> Option<InflightGuard> {
        let key: Key = (kind, symbol.to_string());
        let mut keys = self.lock();
        if !keys.insert(key.clone()) {
            return None;
        }
        drop(keys);
        Some(InflightGuard {
            keys: Arc::clone(&self.keys),
            key: Some(key),
        })
    }

    /// The shared set.
    fn lock(&self) -> std::sync::MutexGuard<'_, HashSet<Key>> {
        self.keys.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Releases its key from the [`Inflight`] set when dropped.
pub struct InflightGuard {
    keys: Arc<Mutex<HashSet<Key>>>,
    /// `Option` only so `Drop` can take ownership of the key.
    key: Option<Key>,
}

impl Drop for InflightGuard {
    fn drop(&mut self) {
        if let Some(key) = self.key.take() {
            let mut keys = self.keys.lock().unwrap_or_else(|e| e.into_inner());
            keys.remove(&key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn begin_refuses_a_duplicate_while_the_first_is_in_flight() {
        let inflight = Inflight::default();
        let _first = inflight
            .begin("analysis", "AAPL")
            .expect("first should claim");

        assert!(inflight.begin("analysis", "AAPL").is_none());
    }

    /// Once the fetch finishes, the same request can be made again.
    #[test]
    fn begin_allows_a_repeat_after_the_guard_drops() {
        let inflight = Inflight::default();
        let first = inflight
            .begin("analysis", "AAPL")
            .expect("first should claim");

        drop(first);

        assert!(inflight.begin("analysis", "AAPL").is_some());
    }

    #[test]
    fn begin_treats_kinds_independently() {
        let inflight = Inflight::default();
        let _analysis = inflight
            .begin("analysis", "AAPL")
            .expect("first should claim");

        assert!(inflight.begin("chart", "AAPL").is_some());
    }

    #[test]
    fn begin_treats_symbols_independently() {
        let inflight = Inflight::default();
        let _aapl = inflight
            .begin("analysis", "AAPL")
            .expect("first should claim");

        assert!(inflight.begin("analysis", "MSFT").is_some());
    }

    #[test]
    fn a_key_is_released_even_when_its_task_panics() {
        let inflight = Inflight::default();
        let shared = inflight.clone();

        let panicked = std::thread::spawn(move || {
            let _guard = shared.begin("analysis", "AAPL").expect("should claim");
            panic!("simulated task panic");
        })
        .join();

        assert!(panicked.is_err(), "the thread should have panicked");
        assert!(
            inflight.begin("analysis", "AAPL").is_some(),
            "the key should have been released, allowing the fetch again"
        );
    }
}
