//! How long the way to the internet takes: a TCP connection to a public resolver every few seconds, while a phone looks.
//! A connection rather than ICMP, which ordinary programs may not send.

use std::net::{SocketAddr, TcpStream};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

const TARGETS: [&str; 2] = ["1.1.1.1:443", "8.8.8.8:443"];
const EVERY: Duration = Duration::from_secs(10);
const TIMEOUT: Duration = Duration::from_secs(2);
const NONE: u32 = u32::MAX;

static LAST_MS: AtomicU32 = AtomicU32::new(NONE);

/// The last time it took in milliseconds; None when the internet did not answer.
pub fn last() -> Option<u32> {
    let ms = LAST_MS.load(Ordering::Relaxed);
    (ms != NONE).then_some(ms)
}

fn once() -> Option<u32> {
    for target in TARGETS {
        let address: SocketAddr = target.parse().ok()?;
        let started = Instant::now();
        if TcpStream::connect_timeout(&address, TIMEOUT).is_ok() {
            return Some(started.elapsed().as_millis().min(NONE as u128 - 1) as u32);
        }
    }
    None
}

/// Measures while [wanted] says someone looks, and sleeps otherwise.
pub fn run(wanted: impl Fn() -> bool) {
    let mut last: Option<Instant> = None;
    loop {
        if !wanted() {
            LAST_MS.store(NONE, Ordering::Relaxed);
            last = None;
        } else if last.is_none_or(|t| t.elapsed() >= EVERY) {
            // At once when a phone starts to look, then every few seconds.
            LAST_MS.store(once().unwrap_or(NONE), Ordering::Relaxed);
            last = Some(Instant::now());
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}
