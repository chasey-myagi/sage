/// Ring buffer for Emacs-style kill/yank operations.
///
/// Tracks killed (deleted) text entries. Consecutive kills can accumulate
/// into a single entry. Supports yank (paste most recent) and yank-pop
/// (cycle through older entries).

use std::collections::VecDeque;

#[derive(Default)]
pub struct KillRing {
    ring: VecDeque<String>,
}

impl KillRing {
    pub fn new() -> Self {
        Self { ring: VecDeque::new() }
    }

    /// Add text to the kill ring.
    ///
    /// - `prepend`: if accumulating, prepend (backward deletion) or append (forward deletion)
    /// - `accumulate`: merge with the most recent entry instead of creating a new one
    pub fn push(&mut self, text: &str, prepend: bool, accumulate: bool) {
        if text.is_empty() {
            return;
        }
        if accumulate && !self.ring.is_empty() {
            let last = self.ring.pop_back().unwrap();
            if prepend {
                self.ring.push_back(format!("{text}{last}"));
            } else {
                self.ring.push_back(format!("{last}{text}"));
            }
        } else {
            self.ring.push_back(text.to_string());
        }
    }

    /// Get most recent entry without modifying the ring.
    pub fn peek(&self) -> Option<&str> {
        self.ring.back().map(|s| s.as_str())
    }

    /// Move last entry to front (for yank-pop cycling).
    pub fn rotate(&mut self) {
        if let Some(last) = self.ring.pop_back() {
            self.ring.push_front(last);
        }
    }

    pub fn len(&self) -> usize {
        self.ring.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ring.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_push_and_peek() {
        let mut kr = KillRing::new();
        kr.push("hello", false, false);
        assert_eq!(kr.peek(), Some("hello"));
    }

    #[test]
    fn test_push_empty_is_noop() {
        let mut kr = KillRing::new();
        kr.push("", false, false);
        assert_eq!(kr.len(), 0);
    }

    #[test]
    fn test_accumulate_append() {
        let mut kr = KillRing::new();
        kr.push("hello", false, false);
        kr.push(" world", false, true);
        assert_eq!(kr.peek(), Some("hello world"));
        assert_eq!(kr.len(), 1);
    }

    #[test]
    fn test_accumulate_prepend() {
        let mut kr = KillRing::new();
        kr.push("world", false, false);
        kr.push("hello ", true, true);
        assert_eq!(kr.peek(), Some("hello world"));
        assert_eq!(kr.len(), 1);
    }

    #[test]
    fn test_no_accumulate_creates_new_entry() {
        let mut kr = KillRing::new();
        kr.push("hello", false, false);
        kr.push("world", false, false);
        assert_eq!(kr.len(), 2);
        assert_eq!(kr.peek(), Some("world"));
    }

    #[test]
    fn test_rotate() {
        let mut kr = KillRing::new();
        kr.push("a", false, false);
        kr.push("b", false, false);
        kr.push("c", false, false);
        // before rotate: ["a", "b", "c"], peek = "c"
        kr.rotate();
        // after rotate: ["c", "a", "b"], peek = "b"
        assert_eq!(kr.peek(), Some("b"));
    }

    #[test]
    fn test_rotate_single_entry_noop() {
        let mut kr = KillRing::new();
        kr.push("only", false, false);
        kr.rotate();
        assert_eq!(kr.peek(), Some("only"));
    }

    #[test]
    fn test_peek_empty() {
        let kr = KillRing::new();
        assert_eq!(kr.peek(), None);
    }
}
