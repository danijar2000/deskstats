use crate::ffi::{localtime_r, Tm};
use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

pub fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' | '\r' | '\t' => out.push(' '),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

pub fn truncate(s: &str, limit: usize) -> String {
    if s.chars().count() <= limit {
        return s.to_string();
    }
    let mut cut: String = s.chars().take(limit).collect();
    cut.push('…');
    cut
}

pub fn normalize(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

pub fn log(message: &str) {
    let secs = (now_ms() / 1000) as i64;
    let mut tm: Tm = unsafe { std::mem::zeroed() };
    let stamp = if unsafe { localtime_r(&secs, &mut tm) }.is_null() {
        secs.to_string()
    } else {
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            tm.tm_year + 1900,
            tm.tm_mon + 1,
            tm.tm_mday,
            tm.tm_hour,
            tm.tm_min,
            tm.tm_sec
        )
    };
    let _ = writeln!(std::io::stderr().lock(), "{stamp} {message}");
}

pub fn run(program: &str, args: &[&str], timeout: Duration) -> Option<(bool, String)> {
    let mut child = Command::new(program).args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
    let stdout = child.stdout.take()?;
    let (send, receive) = std::sync::mpsc::channel();
    let reader = std::thread::Builder::new().name("output".into()).stack_size(64 * 1024).spawn(move || {
        let mut out = Vec::new();
        let _ = stdout.take(1 << 20).read_to_end(&mut out);
        let _ = send.send(out);
    });
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if reader.is_ok() && started.elapsed() < timeout => std::thread::sleep(Duration::from_millis(5)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };
    let out = receive.recv_timeout(Duration::from_secs(1)).ok()?;
    Some((status?.success(), String::from_utf8_lossy(&out).into_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_escape_quotes_and_controls() {
        assert_eq!(json_escape("a\"b\\c\nd\u{1}e"), "a\\\"b\\\\c de");
        assert_eq!(json_escape("Grüße ✓"), "Grüße ✓");
    }

    #[test]
    fn truncate_counts_characters() {
        assert_eq!(truncate("äöüßé", 3), "äöü…");
        assert_eq!(truncate("äöü", 3), "äöü");
    }

    #[test]
    fn run_returns_output_and_kills_what_hangs() {
        assert_eq!(run("/bin/echo", &["hi"], Duration::from_secs(5)), Some((true, "hi\n".into())));
        assert_eq!(run("/usr/bin/false", &[], Duration::from_secs(5)), Some((false, String::new())));
        let started = Instant::now();
        assert_eq!(run("/bin/sleep", &["30"], Duration::from_millis(200)), None);
        assert!(started.elapsed() < Duration::from_secs(5));
        assert_eq!(run("/nonexistent", &[], Duration::from_secs(1)), None);
    }

    #[test]
    fn normalize_collapses_whitespace() {
        assert_eq!(normalize("  Credited:\n 8.75   KGS "), "credited: 8.75 kgs");
    }
}
