//! A blocking priority queue for loading tasks.
//!
//! Higher priority first; tasks of equal priority run in the order they were queued. Pending
//! tasks can be re-prioritised or cancelled by asset id, which is what lets the streamer keep
//! the most important textures at the front while the camera moves.

use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::sync::{Condvar, Mutex, PoisonError};

use crate::handle::AssetId;

/// How urgently a load is needed. Larger values run first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Priority(pub u8);

impl Priority {
    /// Background work nobody waits for.
    pub const LOW: Priority = Priority(32);
    /// The default.
    pub const NORMAL: Priority = Priority(128);
    /// Needed soon (visible, close to the camera).
    pub const HIGH: Priority = Priority(192);
    /// Needed right now.
    pub const URGENT: Priority = Priority(255);
}

impl Default for Priority {
    fn default() -> Self {
        Self::NORMAL
    }
}

struct Entry<T> {
    priority: Priority,
    seq: u64,
    id: AssetId,
    task: T,
}

impl<T> PartialEq for Entry<T> {
    fn eq(&self, other: &Self) -> bool {
        (self.priority, self.seq) == (other.priority, other.seq)
    }
}

impl<T> Eq for Entry<T> {}

impl<T> PartialOrd for Entry<T> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<T> Ord for Entry<T> {
    fn cmp(&self, other: &Self) -> Ordering {
        // BinaryHeap pops the greatest: highest priority, then the oldest request.
        self.priority
            .cmp(&other.priority)
            .then_with(|| other.seq.cmp(&self.seq))
    }
}

struct State<T> {
    heap: BinaryHeap<Entry<T>>,
    next_seq: u64,
    closed: bool,
}

/// Blocking priority queue shared between the manager and its worker threads.
pub struct TaskQueue<T> {
    state: Mutex<State<T>>,
    ready: Condvar,
}

impl<T> Default for TaskQueue<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> TaskQueue<T> {
    /// Creates an empty, open queue.
    pub fn new() -> Self {
        Self {
            state: Mutex::new(State {
                heap: BinaryHeap::new(),
                next_seq: 0,
                closed: false,
            }),
            ready: Condvar::new(),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State<T>> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Queues `task` for asset `id`.
    pub fn push(&self, priority: Priority, id: AssetId, task: T) {
        let mut state = self.lock();
        let seq = state.next_seq;
        state.next_seq += 1;
        state.heap.push(Entry {
            priority,
            seq,
            id,
            task,
        });
        drop(state);
        self.ready.notify_one();
    }

    /// Takes the most urgent task, waiting while the queue is empty. `None` once the queue is
    /// closed and drained.
    pub fn pop(&self) -> Option<T> {
        let mut state = self.lock();
        loop {
            if let Some(entry) = state.heap.pop() {
                return Some(entry.task);
            }
            if state.closed {
                return None;
            }
            state = self
                .ready
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }

    /// Changes the priority of every pending task of `id`. Returns whether any was found.
    pub fn set_priority(&self, id: AssetId, priority: Priority) -> bool {
        let mut state = self.lock();
        let mut entries = std::mem::take(&mut state.heap).into_vec();
        let mut found = false;
        for entry in &mut entries {
            if entry.id == id {
                entry.priority = priority;
                found = true;
            }
        }
        state.heap = BinaryHeap::from(entries);
        found
    }

    /// Removes the pending tasks of `id` (a task a worker already took is not affected).
    /// Returns how many were removed.
    pub fn cancel(&self, id: AssetId) -> usize {
        let mut state = self.lock();
        let before = state.heap.len();
        let mut entries = std::mem::take(&mut state.heap).into_vec();
        entries.retain(|e| e.id != id);
        let removed = before - entries.len();
        state.heap = BinaryHeap::from(entries);
        removed
    }

    /// Number of pending tasks.
    pub fn len(&self) -> usize {
        self.lock().heap.len()
    }

    /// Whether nothing is pending.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Wakes all waiting workers; once drained, [`Self::pop`] returns `None`.
    pub fn close(&self) {
        self.lock().closed = true;
        self.ready.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use super::*;

    fn id(n: u64) -> AssetId {
        AssetId(n)
    }

    fn drain(queue: &TaskQueue<u64>) -> Vec<u64> {
        queue.close();
        std::iter::from_fn(|| queue.pop()).collect()
    }

    #[test]
    fn higher_priority_first_and_fifo_within_a_priority() {
        let q = TaskQueue::new();
        q.push(Priority::NORMAL, id(1), 1);
        q.push(Priority::LOW, id(2), 2);
        q.push(Priority::URGENT, id(3), 3);
        q.push(Priority::NORMAL, id(4), 4);
        q.push(Priority::HIGH, id(5), 5);
        assert_eq!(drain(&q), [3, 5, 1, 4, 2]);
    }

    #[test]
    fn pending_tasks_can_be_reprioritised_and_cancelled() {
        let q = TaskQueue::new();
        for n in 1..=4 {
            q.push(Priority::NORMAL, id(n), n);
        }
        assert!(q.set_priority(id(4), Priority::URGENT));
        assert!(q.set_priority(id(1), Priority::LOW));
        assert!(!q.set_priority(id(99), Priority::HIGH));
        assert_eq!(q.cancel(id(2)), 1);
        assert_eq!(q.cancel(id(2)), 0);
        assert_eq!(q.len(), 3);
        assert_eq!(drain(&q), [4, 3, 1]);
    }

    #[test]
    fn pop_blocks_until_a_task_arrives_and_close_releases_waiters() {
        let q = Arc::new(TaskQueue::new());
        let worker = {
            let q = q.clone();
            std::thread::spawn(move || {
                let first = q.pop();
                let second = q.pop();
                (first, second)
            })
        };
        std::thread::sleep(Duration::from_millis(30));
        q.push(Priority::NORMAL, id(1), 7u64);
        std::thread::sleep(Duration::from_millis(30));
        q.close();
        assert_eq!(worker.join().unwrap(), (Some(7), None));
    }

    #[test]
    fn many_workers_and_random_priorities_lose_nothing() {
        let q = Arc::new(TaskQueue::new());
        let total = 2000u64;
        let mut state = 12345u32;
        for n in 0..total {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            q.push(Priority((state >> 24) as u8), id(n), n);
        }
        let workers: Vec<_> = (0..4)
            .map(|_| {
                let q = q.clone();
                std::thread::spawn(move || std::iter::from_fn(|| q.pop()).collect::<Vec<u64>>())
            })
            .collect();
        std::thread::sleep(Duration::from_millis(20));
        q.close();
        let mut all: Vec<u64> = workers
            .into_iter()
            .flat_map(|w| w.join().unwrap())
            .collect();
        all.sort_unstable();
        assert_eq!(all, (0..total).collect::<Vec<_>>());
    }
}
