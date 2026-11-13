//! Independent liveness bounds for the opt-in native diagnostic, including a stalled event loop.
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

/// A native diagnostic watchdog whose worker never depends on graphics/event-loop callbacks.
/// Drop cancels it without joining; the caller owns the failure action (the example exits).
pub struct Watchdog {
    origin: Instant,
    deadline_ms: Arc<AtomicU64>,
    stopped: Arc<AtomicBool>,
}

impl Watchdog {
    /// Start one worker with a monotonic deadline. The failure callback runs at most once;
    /// it must not wait for the UI, which may be the component that has stopped responding.
    pub fn start(timeout: Duration, failed: impl FnOnce() + Send + 'static) -> std::io::Result<Self> {
        let watchdog = Self { origin: Instant::now(), deadline_ms: Arc::new(AtomicU64::new(0)), stopped: Arc::new(AtomicBool::new(false)) };
        watchdog.renew(timeout);
        let origin = watchdog.origin;
        let deadline = watchdog.deadline_ms.clone();
        let stopped = watchdog.stopped.clone();
        std::thread::Builder::new().name("pcbcraft-perf-watchdog".into()).spawn(move || {
            while !stopped.load(Ordering::Acquire) {
                if origin.elapsed().as_millis() >= u128::from(deadline.load(Ordering::Acquire)) {
                    if !stopped.swap(true, Ordering::AcqRel) {
                        failed();
                    }
                    return;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        })?;
        Ok(watchdog)
    }

    /// Renew only on accepted UI progress or an intentional idle hold, never just from
    /// an external timer. This keeps an absent UI from disguising a lost frame callback.
    pub fn renew(&self, timeout: Duration) {
        let deadline = self.origin.elapsed().saturating_add(timeout).as_millis().min(u128::from(u64::MAX)) as u64;
        self.deadline_ms.store(deadline, Ordering::Release);
    }
}

impl Drop for Watchdog {
    /// Teardown/success disarms the independent worker without waiting for its polling interval.
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    /// An event loop that never calls again still expires, while explicit teardown cancels.
    #[test]
    fn missing_ui_progress_expires_independently_and_drop_cancels() {
        let (send, receive) = std::sync::mpsc::sync_channel(1);
        let watchdog = Watchdog::start(Duration::from_secs(60), move || {
            let _ = send.try_send(());
        })
        .unwrap();
        watchdog.renew(Duration::from_secs(120));
        assert!(watchdog.deadline_ms.load(Ordering::Acquire) >= 120_000);
        watchdog.deadline_ms.store(0, Ordering::Release);
        assert!(receive.recv_timeout(Duration::from_secs(5)).is_ok());
        let (send, receive) = std::sync::mpsc::sync_channel(1);
        let watchdog = Watchdog::start(Duration::from_secs(60), move || {
            let _ = send.try_send(());
        })
        .unwrap();
        drop(watchdog);
        assert!(matches!(receive.recv_timeout(Duration::from_secs(5)), Err(std::sync::mpsc::RecvTimeoutError::Disconnected)));
    }
}
