//! Generic undo stack with clone-on-push semantics.
//!
//! Stores clones of state snapshots. Popped snapshots are returned
//! directly (no re-cloning) since they are already detached.

pub struct UndoStack<S> {
    stack: Vec<S>,
}

impl<S: Clone> UndoStack<S> {
    pub fn new() -> Self {
        Self { stack: Vec::new() }
    }

    /// Push a clone of the given state onto the stack.
    pub fn push(&mut self, state: &S) {
        self.stack.push(state.clone());
    }

    /// Pop and return the most recent snapshot, or None if empty.
    pub fn pop(&mut self) -> Option<S> {
        self.stack.pop()
    }

    /// Remove all snapshots.
    pub fn clear(&mut self) {
        self.stack.clear();
    }

    pub fn len(&self) -> usize {
        self.stack.len()
    }

    pub fn is_empty(&self) -> bool {
        self.stack.is_empty()
    }
}

impl<S: Clone> Default for UndoStack<S> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_push_and_pop() {
        let mut stack: UndoStack<String> = UndoStack::new();
        stack.push(&"hello".to_string());
        assert_eq!(stack.pop(), Some("hello".to_string()));
    }

    #[test]
    fn test_pop_empty() {
        let mut stack: UndoStack<String> = UndoStack::new();
        assert_eq!(stack.pop(), None);
    }

    #[test]
    fn test_clone_on_push() {
        let mut stack: UndoStack<Vec<i32>> = UndoStack::new();
        let mut state = vec![1, 2, 3];
        stack.push(&state);
        state.push(4);
        // Snapshot should still be [1, 2, 3]
        assert_eq!(stack.pop(), Some(vec![1, 2, 3]));
    }

    #[test]
    fn test_multiple_push_pop() {
        let mut stack: UndoStack<i32> = UndoStack::new();
        stack.push(&1);
        stack.push(&2);
        stack.push(&3);
        assert_eq!(stack.pop(), Some(3));
        assert_eq!(stack.pop(), Some(2));
        assert_eq!(stack.pop(), Some(1));
        assert_eq!(stack.pop(), None);
    }

    #[test]
    fn test_clear() {
        let mut stack: UndoStack<i32> = UndoStack::new();
        stack.push(&1);
        stack.push(&2);
        stack.clear();
        assert_eq!(stack.len(), 0);
        assert_eq!(stack.pop(), None);
    }

    #[test]
    fn test_len() {
        let mut stack: UndoStack<i32> = UndoStack::new();
        assert_eq!(stack.len(), 0);
        stack.push(&1);
        assert_eq!(stack.len(), 1);
        stack.push(&2);
        assert_eq!(stack.len(), 2);
        stack.pop();
        assert_eq!(stack.len(), 1);
    }
}
