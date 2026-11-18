//! Small helpers shared by the modules: JSON text, time, logging.

use crate::ffi::{localtime_r, Tm};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::fs::MetadataExt;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// Escapes a string for a JSON string literal; line breaks become spaces, other control characters go.
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

/// Cuts a string to `limit` characters, adding an ellipsis when it was longer.
pub fn truncate(s: &str, limit: usize) -> String {
    if s.chars().count() <= limit {
        return s.to_string();
    }
    let mut cut: String = s.chars().take(limit).collect();
    cut.push('…');
    cut
}

/// Lower case with runs of whitespace collapsed, for comparing texts from different sources.
pub fn normalize(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// Writes one line to stderr, which launchd sends to ~/Library/Logs/deskstats.log.
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
    // A failed write (a full disk) must not take the server down: eprintln! would panic.
    let _ = writeln!(std::io::stderr().lock(), "{stamp} {message}");
}

/// Runs a program and returns whether it succeeded and what it printed. One that does not finish
/// within `timeout` is killed, so a hung tool never stops the thread that called it.
pub fn run(program: &str, args: &[&str], timeout: Duration) -> Option<(bool, String)> {
    let mut child = Command::new(program).args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
    let stdout = child.stdout.take()?;
    // The output is read on the side: a full pipe would block the program, and a program
    // that hangs must not block the reader's caller either.
    let (send, receive) = std::sync::mpsc::channel();
    let reader = std::thread::Builder::new().name("output".into()).stack_size(64 * 1024).spawn(move || {
        let mut out = Vec::new();
        // No tool here prints more than this; a longer output counts as a failure.
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

const O_APPEND: i32 = 0x8;
const F_GETFL: i32 = 3;

extern "C" {
    fn fcntl(fd: i32, command: i32, ...) -> i32;
}

/// Keeps the log from growing without end: when the file launchd appends stderr to is larger
/// than `limit`, only its last quarter stays. Does nothing when stderr is a terminal or a pipe.
pub fn trim_log(path: &str, limit: u64) {
    let Ok(meta) = std::fs::metadata(path) else { return };
    let stderr = std::mem::ManuallyDrop::new(unsafe { <std::fs::File as std::os::fd::FromRawFd>::from_raw_fd(2) });
    let Ok(own) = stderr.metadata() else { return };
    // Cutting a file that is not written in append mode would leave a hole up to the old offset.
    let appending = unsafe { fcntl(2, F_GETFL) } & O_APPEND != 0;
    if meta.len() <= limit || (own.dev(), own.ino()) != (meta.dev(), meta.ino()) || !appending {
        return;
    }
    let Ok(mut file) = std::fs::OpenOptions::new().read(true).write(true).open(path) else { return };
    let mut tail = Vec::new();
    if file.seek(SeekFrom::End(-((limit / 4) as i64))).is_err() || file.read_to_end(&mut tail).is_err() {
        return;
    }
    let start = tail.iter().position(|b| *b == b'\n').map_or(0, |i| i + 1);
    if file.set_len(0).is_ok() && file.seek(SeekFrom::Start(0)).is_ok() {
        let _ = file.write_all(&tail[start..]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_escape_quotes_and_controls() {
        assert_eq!(json_escape("a\"b\\c\nd\u{1}e"), "a\\\"b\\\\c de");
        assert_eq!(json_escape("Привет"), "Привет");
    }

    #[test]
    fn truncate_counts_characters() {
        assert_eq!(truncate("абвгд", 3), "абв…");
        assert_eq!(truncate("абв", 3), "абв");
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
        assert_eq!(normalize("  Зачислено:\n 8.75   KGS "), "зачислено: 8.75 kgs");
    }
}
