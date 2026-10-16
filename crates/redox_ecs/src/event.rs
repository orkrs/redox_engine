//! Double-buffered events with per-reader cursors.
//!
//! [`Events<T>`] assigns a monotonically increasing id to every event. Events
//! live for **two** [`Events::update`] calls (current frame + next frame), so
//! readers created this frame still see last frame's events. Each
//! [`EventReader`] tracks its own cursor and only yields events it has not
//! seen — multiple readers never duplicate or lose events.
//!
//! Call [`Events::update`] once per frame (the engine does this automatically
//! after the dispatcher runs).

/// Internal storage for events of type `T`.
pub struct Events<T: 'static + Send + Sync> {
    /// `(id, event)` pairs in send order.
    events: Vec<(u64, T)>,
    /// Id assigned to the next event.
    next_id: u64,
    /// Oldest id still retained. Set to `next_id` by each `update()`.
    cutoff: u64,
}

impl<T: 'static + Send + Sync> Events<T> {
    /// Creates a new event queue.
    pub fn new() -> Self {
        Self {
            events: Vec::with_capacity(128),
            next_id: 0,
            cutoff: 0,
        }
    }

    /// Ages the queue: events from before the previous `update` are dropped.
    ///
    /// Should be called once per frame.
    pub fn update(&mut self) {
        self.events.retain(|(id, _)| *id >= self.cutoff);
        self.cutoff = self.next_id;
    }

    /// Sends an event into the current frame's window.
    pub fn send(&mut self, event: T) {
        self.events.push((self.next_id, event));
        self.next_id += 1;
    }

    /// Returns the id of the oldest retained event (cursor start for new readers).
    pub fn oldest(&self) -> u64 {
        self.cutoff
    }

    /// Returns an iterator over all retained events (ignores reader cursors).
    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.events.iter().map(|(_, e)| e)
    }

    /// Returns `true` if no events are retained.
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Number of retained events.
    pub fn len(&self) -> usize {
        self.events.len()
    }
}

impl<T: 'static + Send + Sync> Default for Events<T> {
    fn default() -> Self {
        Self::new()
    }
}

/// Reader for events of type `T`.
///
/// Tracks its own read cursor. Store readers as resources (they are cheap and
/// borrow nothing) and call [`EventReader::iter`] with the events resource each
/// frame — every event is delivered exactly once per reader.
#[derive(Debug)]
pub struct EventReader<T: 'static + Send + Sync> {
    next_id: u64,
    _marker: std::marker::PhantomData<T>,
}

impl<T: 'static + Send + Sync> EventReader<T> {
    /// Creates a reader that starts at the oldest currently retained event.
    pub fn new() -> Self {
        Self {
            next_id: 0,
            _marker: std::marker::PhantomData,
        }
    }

    /// Creates a reader positioned at `start_id` (events with smaller ids are skipped).
    pub fn starting_at(start_id: u64) -> Self {
        Self {
            next_id: start_id,
            _marker: std::marker::PhantomData,
        }
    }

    /// Returns an iterator over events this reader has not seen yet.
    ///
    /// Advances the reader's cursor to the newest event.
    pub fn iter<'a>(&mut self, events: &'a Events<T>) -> impl Iterator<Item = &'a T> {
        let next_id = &mut self.next_id;
        let start = *next_id;
        events
            .events
            .iter()
            .filter(move |(id, _)| {
                // Advance cursor past everything up to and including the last
                // yielded event; events are sorted by id.
                if *id >= start {
                    *next_id = *id + 1;
                    true
                } else {
                    false
                }
            })
            .map(|(_, e)| e)
    }
}

impl<T: 'static + Send + Sync> Default for EventReader<T> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reader_sees_each_event_exactly_once() {
        let mut events = Events::new();
        let mut reader = EventReader::new();

        events.send(1);
        events.send(2);
        let seen: Vec<i32> = reader.iter(&events).copied().collect();
        assert_eq!(seen, vec![1, 2]);

        // No new events → nothing yielded again.
        assert!(reader.iter(&events).next().is_none());

        events.send(3);
        let seen: Vec<i32> = reader.iter(&events).copied().collect();
        assert_eq!(seen, vec![3]);
    }

    #[test]
    fn update_drops_events_older_than_two_frames() {
        let mut events = Events::new();
        let mut reader = EventReader::new();

        events.send(1);
        events.update(); // frame boundary: event 1 still retained
        events.update(); // event 1 now expired

        assert!(reader.iter(&events).next().is_none());
    }

    #[test]
    fn independent_readers_do_not_interfere() {
        let mut events = Events::new();
        let mut r1 = EventReader::new();
        let mut r2 = EventReader::new();

        events.send(10);
        let _: Vec<i32> = r1.iter(&events).copied().collect();
        let _: Vec<i32> = r2.iter(&events).copied().collect();

        events.send(20);
        events.update();
        assert_eq!(r1.iter(&events).copied().collect::<Vec<i32>>(), vec![20]);
        assert_eq!(r2.iter(&events).copied().collect::<Vec<i32>>(), vec![20]);
    }
}
