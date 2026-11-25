//! Whether the computer is awake, and telling the phone before it goes away.

pub use crate::sys::power::{start, state, stopping};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

const GRACE: Duration = Duration::from_secs(3);

pub static LAST_POLL_MS: AtomicU64 = AtomicU64::new(0);

/// Gives a phone that polled lately a moment to poll once more and learn that the computer goes to sleep or stops.
pub fn wait_for_poll(since: u64) {
    if since.saturating_sub(LAST_POLL_MS.load(Ordering::Relaxed)) > 5000 {
        return;
    }
    let start = Instant::now();
    while start.elapsed() < GRACE && LAST_POLL_MS.load(Ordering::Relaxed) <= since {
        std::thread::sleep(Duration::from_millis(100));
    }
}
