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

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn truncate(s: &str, limit: usize) -> String {
    if s.chars().count() <= limit {
        return s.to_string();
    }
    let mut cut: String = s.chars().take(limit).collect();
    cut.push('…');
    cut
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn normalize(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

pub fn log(message: &str) {
    let secs = (now_ms() / 1000) as i64;
    let stamp = match crate::sys::local_time(secs) {
        Some([year, month, day, hour, minute, second]) => format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02}"),
        None => secs.to_string(),
    };
    crate::sys::log_line(&format!("{stamp} {message}"));
}

/// A command that runs without a window of its own.
fn command(program: &str) -> Command {
    #[allow(unused_mut)]
    let mut command = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    command
}

pub fn run(program: &str, args: &[&str], timeout: Duration) -> Option<(bool, String)> {
    run_with_input(program, args, None, timeout)
}

/// Runs a program, optionally with [input] on its standard input; whether it ended well and what it printed.
/// None when it cannot start or does not end in time, in which case it is killed.
pub fn run_with_input(program: &str, args: &[&str], input: Option<&str>, timeout: Duration) -> Option<(bool, String)> {
    let stdin = if input.is_some() { Stdio::piped() } else { Stdio::null() };
    let mut child = command(program).args(args).stdin(stdin).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
    if let (Some(text), Some(mut pipe)) = (input, child.stdin.take()) {
        // Short enough to fit the pipe; dropping the pipe ends the input.
        let _ = pipe.write_all(text.as_bytes());
    }
    let Some(stdout) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return None;
    };
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

    #[cfg(unix)]
    #[test]
    fn run_returns_output_and_kills_what_hangs() {
        assert_eq!(run_with_input("/bin/cat", &[], Some("in\n"), Duration::from_secs(5)), Some((true, "in\n".into())));
        assert_eq!(run("/bin/echo", &["hi"], Duration::from_secs(5)), Some((true, "hi\n".into())));
        assert_eq!(run("/usr/bin/false", &[], Duration::from_secs(5)), Some((false, String::new())));
        let started = Instant::now();
        assert_eq!(run("/bin/sleep", &["30"], Duration::from_millis(200)), None);
        assert!(started.elapsed() < Duration::from_secs(5));
        assert_eq!(run("/nonexistent", &[], Duration::from_secs(1)), None);
    }

    #[cfg(windows)]
    #[test]
    fn run_returns_output_and_kills_what_hangs() {
        assert_eq!(run("cmd.exe", &["/C", "echo hi"], Duration::from_secs(10)), Some((true, "hi\r\n".into())));
        assert_eq!(run("cmd.exe", &["/C", "exit 3"], Duration::from_secs(10)), Some((false, String::new())));
        assert_eq!(run_with_input("findstr.exe", &["x"], Some("axb\r\n"), Duration::from_secs(10)), Some((true, "axb\r\n".into())));
        let started = Instant::now();
        assert_eq!(run("ping.exe", &["-n", "30", "127.0.0.1"], Duration::from_millis(300)), None);
        assert!(started.elapsed() < Duration::from_secs(5));
        assert_eq!(run("no-such-program-here.exe", &[], Duration::from_secs(1)), None);
    }

    #[test]
    fn normalize_collapses_whitespace() {
        assert_eq!(normalize("  Credited:\n 8.75   KGS "), "credited: 8.75 kgs");
    }
}
