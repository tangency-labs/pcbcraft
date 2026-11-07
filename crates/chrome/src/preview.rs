//! Engine-independent background preview cache, adapted from SolidCraft's thumbnail lifecycle.
//! A removed or superseded request keeps its worker slot until it exits; invalidation cannot
//! accumulate detached workers. Dropping the cache never joins a possibly stalled disk read.

use std::{
    collections::BTreeMap,
    fmt,
    sync::mpsc::{self, Receiver, TryRecvError},
};

const WORKERS: usize = 2;
type Task<T> = Box<dyn FnOnce() -> Option<T> + Send>;

enum Entry<T> {
    Queued(Task<T>),
    Loading(u64),
    Ready(Option<T>),
}

struct Pending<K, T> {
    key: K,
    generation: u64,
    receiver: Receiver<Option<T>>,
}

/// A bounded cache of app-supplied preview data. It owns scheduling and stale-result rejection,
/// but knows nothing about documents, file formats, renderers, or electrical state.
pub struct PreviewCache<K, T> {
    entries: BTreeMap<K, Entry<T>>,
    pending: Vec<Pending<K, T>>,
    generation: u64,
    capacity: usize,
}

impl<K, T> fmt::Debug for PreviewCache<K, T> {
    /// Report cache size without requiring callbacks or application data to implement Debug.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PreviewCache").field("entries", &self.entries.len()).field("workers", &self.pending.len()).finish()
    }
}

impl<K: Ord + Clone, T: Send + 'static> PreviewCache<K, T> {
    /// Keep at most `capacity` entries and two active workers. Callers retain their visible keys
    /// before enqueueing replacements; queue saturation refuses new keys rather than growing.
    pub fn new(capacity: usize) -> Self {
        Self { entries: BTreeMap::new(), pending: Vec::new(), generation: 0, capacity }
    }

    /// Whether a request is already queued, running, ready or known to be unavailable.
    pub fn contains(&self, key: &K) -> bool {
        self.entries.contains_key(key)
    }

    /// Install or supersede a request. An old worker's result cannot overwrite this task.
    /// Returning false means a new key was refused because the cache is full.
    pub fn enqueue(&mut self, key: K, make: impl FnOnce() -> Option<T> + Send + 'static) -> bool {
        if !self.entries.contains_key(&key) && self.entries.len() >= self.capacity {
            return false;
        }
        self.entries.insert(key, Entry::Queued(Box::new(make)));
        true
    }

    /// Forget results and queued work for invisible keys. Running jobs retain their slots until
    /// they finish, even after clear/reopen cycles, and their removed results are discarded.
    pub fn retain(&mut self, mut keep: impl FnMut(&K) -> bool) {
        self.entries.retain(|key, _| keep(key));
    }

    /// `None` means queued/loading; `Some(None)` is a terminal unavailable preview, retried only
    /// when the caller explicitly enqueues a fresh source. Data is borrowed without cloning.
    pub fn get(&self, key: &K) -> Option<Option<&T>> {
        match self.entries.get(key) {
            Some(Entry::Ready(data)) => Some(data.as_ref()),
            _ => None,
        }
    }

    /// Adopt completed jobs without blocking, then fill free worker slots. Completion wakes the
    /// UI even if a task unwinds; spawn failures and disconnected workers become unavailable.
    /// File reads run only inside the caller's task, never here on the UI thread.
    pub fn poll(&mut self, ctx: &egui::Context) {
        for index in (0..self.pending.len()).rev() {
            let result = match self.pending[index].receiver.try_recv() {
                Ok(data) => data,
                Err(TryRecvError::Empty) => continue,
                Err(TryRecvError::Disconnected) => None,
            };
            let pending = self.pending.swap_remove(index);
            if matches!(self.entries.get(&pending.key), Some(Entry::Loading(generation)) if *generation == pending.generation) {
                self.entries.insert(pending.key, Entry::Ready(result));
            }
        }
        while self.pending.len() < WORKERS {
            let Some(key) = self.entries.iter().find_map(|(key, entry)| matches!(entry, Entry::Queued(_)).then(|| key.clone())) else { break };
            self.generation = self.generation.wrapping_add(1);
            let generation = self.generation;
            let Some(Entry::Queued(make)) = self.entries.insert(key.clone(), Entry::Loading(generation)) else { continue };
            let (send, receiver) = mpsc::sync_channel(1);
            let wake = RepaintOnDrop(ctx.clone());
            match std::thread::Builder::new().name("craft-preview".to_owned()).spawn(move || {
                let _wake = wake;
                let _ = send.send(make());
            }) {
                Ok(_) => self.pending.push(Pending { key, generation, receiver }),
                Err(_) => {
                    self.entries.insert(key, Entry::Ready(None));
                }
            }
        }
    }
}

struct RepaintOnDrop(egui::Context);
impl Drop for RepaintOnDrop {
    /// Wake the owner on normal completion or unwinding, without waiting for another UI event.
    fn drop(&mut self) {
        self.0.request_repaint();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// Poll actual callbacks with a deadline; no GUI loop or file system is needed.
    fn wait(cache: &mut PreviewCache<u8, u8>, key: u8) {
        let until = Instant::now() + Duration::from_secs(5);
        while cache.get(&key).is_none() && Instant::now() < until {
            cache.poll(&egui::Context::default());
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(cache.get(&key).is_some());
    }

    /// Removing busy keys cannot release worker slots early, while newer requests win over
    /// obsolete results. Refused queue additions and failed tasks stay bounded and terminal.
    #[test]
    fn invalidation_retains_worker_slots_and_rejects_stale_results() {
        let mut cache = PreviewCache::new(2);
        let (first, first_rx) = mpsc::channel();
        let (second, second_rx) = mpsc::channel();
        cache.enqueue(1, move || first_rx.recv().ok());
        cache.enqueue(2, move || second_rx.recv().ok());
        assert!(!cache.enqueue(3, || Some(3)));
        cache.poll(&egui::Context::default());
        assert_eq!(cache.pending.len(), 2);
        cache.retain(|key| *key == 1);
        cache.enqueue(1, || Some(9));
        cache.enqueue(3, || None);
        cache.poll(&egui::Context::default());
        assert_eq!(cache.pending.len(), 2);
        assert!(cache.get(&1).is_none());
        first.send(1).unwrap();
        second.send(2).unwrap();
        wait(&mut cache, 1);
        wait(&mut cache, 3);
        assert_eq!(cache.get(&1), Some(Some(&9)));
        assert_eq!(cache.get(&3), Some(None));
        assert!(!cache.contains(&2));
    }

    /// A disconnected task cannot keep a loading entry or hold up a later callback.
    #[test]
    fn disconnected_workers_release_slots() {
        let mut cache = PreviewCache::<u8, u8>::new(2);
        let (send, receiver) = mpsc::channel();
        drop(send);
        cache.entries.insert(1, Entry::Loading(7));
        cache.pending.push(Pending { key: 1, generation: 7, receiver });
        cache.enqueue(2, || Some(2));
        wait(&mut cache, 2);
        assert_eq!(cache.get(&1), Some(None));
        assert_eq!(cache.get(&2), Some(Some(&2)));
    }
}
