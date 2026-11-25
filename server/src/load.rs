//! Turning two readings of the counters into a load; the same on every system.

use std::collections::HashMap;

pub fn cpu_percent(prev: &[(u64, u64)], cur: &[(u64, u64)]) -> f64 {
    let (mut busy, mut total) = (0u64, 0u64);
    for (i, (b, t)) in cur.iter().enumerate() {
        let Some((pb, pt)) = prev.get(i) else { continue };
        busy += b.wrapping_sub(*pb) & 0xffff_ffff;
        total += t.wrapping_sub(*pt) & 0xffff_ffff;
    }
    if total > 0 {
        busy as f64 * 100.0 / total as f64
    } else {
        0.0
    }
}

pub fn net_delta(prev: &HashMap<String, (u64, u64)>, cur: &HashMap<String, (u64, u64)>) -> (u64, u64) {
    let (mut rx, mut tx) = (0u64, 0u64);
    for (name, (r, t)) in cur {
        if let Some((pr, pt)) = prev.get(name) {
            rx += r.saturating_sub(*pr);
            tx += t.saturating_sub(*pt);
        }
    }
    (rx, tx)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_percent_handles_wrap_and_missing_cores() {
        let prev = vec![(u32::MAX as u64 - 10, u32::MAX as u64 - 20)];
        let cur = vec![(30, 80), (5, 10)];
        let pct = cpu_percent(&prev, &cur);
        assert!((pct - 41.0 * 100.0 / 101.0).abs() < 1e-9);
        assert_eq!(cpu_percent(&[], &cur), 0.0);
    }

    #[test]
    fn net_delta_ignores_interfaces_that_come_and_go() {
        let prev = HashMap::from([("en0".to_string(), (1000u64, 500u64)), ("en5".to_string(), (9_000_000, 9_000_000))]);
        let cur = HashMap::from([("en0".to_string(), (1500u64, 700u64)), ("en7".to_string(), (4_000_000, 4_000_000))]);
        assert_eq!(net_delta(&prev, &cur), (500, 200));
    }

    #[test]
    fn net_delta_counts_past_four_gigabytes_and_ignores_a_reset() {
        let prev = HashMap::from([("en0".to_string(), (u32::MAX as u64 - 99, 5000u64))]);
        let cur = HashMap::from([("en0".to_string(), (u32::MAX as u64 + 2_500_000_000, 10u64))]);
        assert_eq!(net_delta(&prev, &cur), (2_500_000_099, 0));
    }
}
