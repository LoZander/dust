use std::collections::VecDeque;

/// A simple queue with fixed capacity.
///
/// When there are fewer elements in the [`Queue`] than the `capacity`,
/// elements can be pushed as a normal [`VecDeque`], but when at `capacity`,
/// pushing another item to the back of the queue results in in
/// item at the front being automatically popped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Queue<T> {
    capacity: usize,
    items: VecDeque<T>,
}

impl<T> Queue<T> {
    /// Creates an empty [`Queue`] with a given capacity.
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            items: VecDeque::new(),
        }
    }

    /// Pushes an element to the back of the queue.
    ///
    /// If the queue is full, the element at the front is popped
    /// and returned. Otherwise, [`None`] is returned.
    pub fn push(&mut self, item: T) -> Option<T> {
        self.items.push_back(item);

        if self.items.len() > self.capacity {
            self.items.pop_front()
        } else {
            None
        }
    }
}

impl<T: PartialEq> Queue<T> {
    /// Returns `true` if the queue contains the given item and `false` otherwise.
    pub fn contains(&self, item: &T) -> bool {
        self.items.contains(item)
    }
}
