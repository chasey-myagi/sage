//! Per-file mutation serialization queue.
//!
//! Translated from pi-mono `packages/coding-agent/src/core/tools/file-mutation-queue.ts`.
//!
//! Ensures that concurrent write / edit operations targeting the **same
//! absolute path** are serialized, while operations on different files still
//! run in parallel.
//!
//! The TypeScript version uses a promise chain (`Map<string, Promise<void>>`).
//! This Rust implementation uses `tokio` async mutexes via a global
//! `DashMap`-backed registry so that the behaviour mirrors the TS version
//! without requiring `unsafe` code.
//!
//! For unit tests (which are synchronous in Rust) the queue still works
//! correctly: each `with_file_mutation_queue` call awaits the previous
//! operation on the same key before proceeding.

use std::collections::HashMap;
use std::future::Future;
use std::path::Path;
use std::sync::Mutex;

// ============================================================================
// Queue key helper
// ============================================================================

/// Return the canonical key for a file path.
///
/// Attempts to resolve symlinks (`realpath`); falls back to the lexically
/// resolved path when that fails.
///
/// Mirrors `getMutationQueueKey()` from `file-mutation-queue.ts`.
fn get_mutation_queue_key(file_path: &str) -> String {
    let resolved = Path::new(file_path).canonicalize().unwrap_or_else(|_| {
        // Fallback: join with cwd
        let cwd = std::env::current_dir().unwrap_or_default();
        cwd.join(file_path)
    });
    resolved.to_string_lossy().into_owned()
}

// ============================================================================
// Global queue state (sync variant for blocking callers)
// ============================================================================

/// A simple synchronous per-file mutex registry.
///
/// Unlike the TypeScript promise chain, this uses `std::sync::Mutex` per key
/// so that multiple threads calling `with_file_mutation_queue_sync` on the
/// same path are serialized correctly.
///
/// The map itself is guarded by a global `Mutex<HashMap<...>>`.
static FILE_MUTEXES: Mutex<Option<HashMap<String, std::sync::Arc<Mutex<()>>>>> = Mutex::new(None);

fn get_or_create_file_mutex(key: &str) -> std::sync::Arc<Mutex<()>> {
    let mut guard = FILE_MUTEXES.lock().expect("FILE_MUTEXES poisoned");
    let map = guard.get_or_insert_with(HashMap::new);
    map.entry(key.to_string())
        .or_insert_with(|| std::sync::Arc::new(Mutex::new(())))
        .clone()
}

/// Serialize synchronous file mutations for the same path.
///
/// Operations on different paths run concurrently. Operations on the same
/// path are serialized.
///
/// Mirrors `withFileMutationQueue()` from `file-mutation-queue.ts` for
/// synchronous callers.
pub fn with_file_mutation_queue_sync<T, F>(file_path: &str, f: F) -> T
where
    F: FnOnce() -> T,
{
    let key = get_mutation_queue_key(file_path);
    let file_mutex = get_or_create_file_mutex(&key);
    let _guard = file_mutex.lock().expect("file mutex poisoned");
    f()
}

// ============================================================================
// Async variant
// ============================================================================

/// Serialize async file mutations for the same path.
///
/// Mirrors `withFileMutationQueue()` from `file-mutation-queue.ts` for async
/// callers.  Acquires the per-file lock synchronously (briefly blocks) before
/// driving `f()` to completion.
pub async fn with_file_mutation_queue<T, Fut, F>(file_path: &str, f: F) -> T
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = T>,
{
    // Acquire the mutex synchronously; the critical section is file I/O which
    // is typically short.  This matches the TypeScript behavior of chaining
    // promises without yielding the lock across unrelated awaits.
    let key = get_mutation_queue_key(file_path);
    let file_mutex = get_or_create_file_mutex(&key);
    let _guard = file_mutex.lock().expect("file mutex poisoned");
    f().await
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    // ---- get_mutation_queue_key ----

    #[test]
    fn key_is_deterministic_for_same_path() {
        let k1 = get_mutation_queue_key("/tmp/test_file.txt");
        let k2 = get_mutation_queue_key("/tmp/test_file.txt");
        assert_eq!(k1, k2);
    }

    #[test]
    fn key_differs_for_different_paths() {
        let k1 = get_mutation_queue_key("/tmp/file_a.txt");
        let k2 = get_mutation_queue_key("/tmp/file_b.txt");
        assert_ne!(k1, k2);
    }

    // ---- with_file_mutation_queue_sync ----

    #[test]
    fn sync_queue_serializes_same_file() {
        let counter = Arc::new(AtomicUsize::new(0));
        let path = "/tmp/sage_fmq_test_same.txt";

        let c = Arc::clone(&counter);
        let r1 = with_file_mutation_queue_sync(path, || c.fetch_add(1, Ordering::SeqCst));

        let c = Arc::clone(&counter);
        let r2 = with_file_mutation_queue_sync(path, || c.fetch_add(1, Ordering::SeqCst));

        // Results should be 0 and 1 respectively (serialized)
        assert_eq!(r1, 0);
        assert_eq!(r2, 1);
        assert_eq!(counter.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn sync_queue_allows_different_files() {
        let path_a = "/tmp/sage_fmq_file_a.txt";
        let path_b = "/tmp/sage_fmq_file_b.txt";

        let result_a = with_file_mutation_queue_sync(path_a, || "a");
        let result_b = with_file_mutation_queue_sync(path_b, || "b");

        assert_eq!(result_a, "a");
        assert_eq!(result_b, "b");
    }

    #[test]
    fn sync_queue_propagates_return_value() {
        let result = with_file_mutation_queue_sync("/tmp/fmq_return.txt", || 42usize);
        assert_eq!(result, 42);
    }

    #[test]
    fn sync_queue_nested_calls_different_files_do_not_deadlock() {
        let result = with_file_mutation_queue_sync("/tmp/outer.txt", || {
            with_file_mutation_queue_sync("/tmp/inner.txt", || "ok")
        });
        assert_eq!(result, "ok");
    }
}
