//! Keeps a phone on USB connected: `adb reverse` for the port and, when the phone shows up and
//! is not polling, opens the panel with the port and token. Runs in its own thread with
//! timeouts, so a hung adb never stops the sampling.
//!
//! adb is not part of macOS: it comes from a folder the user's programs can write to. So it is
//! started "disclaimed" — responsible for itself in the eyes of the privacy system — and does
//! not inherit this server's Full Disk Access. The token reaches it through a file only the
//! owner can read, not through its arguments, which every user of the Mac could list.

use crate::http::Shared;
use crate::util::{log, now_ms};
use std::ffi::{c_char, c_int, c_void, CString};
use std::os::unix::fs::OpenOptionsExt;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Receives the port and token (it requires the DUMP permission, which only adb's shell has).
const CONFIG_ACTIVITY: &str = "app.deskstats/.ConfigActivity";
const COMMAND_TIMEOUT: Duration = Duration::from_secs(5);
const RETRY_MIN: Duration = Duration::from_secs(10);
const RETRY_MAX: Duration = Duration::from_secs(120);

const O_RDONLY: c_int = 0;
const O_WRONLY_CREATE_TRUNCATE: c_int = 0x1 | 0x200 | 0x400;
const WNOHANG: c_int = 1;
const SIGKILL: c_int = 9;

extern "C" {
    fn posix_spawnattr_init(attr: *mut *mut c_void) -> c_int;
    fn posix_spawnattr_destroy(attr: *mut *mut c_void) -> c_int;
    fn posix_spawn_file_actions_init(actions: *mut *mut c_void) -> c_int;
    fn posix_spawn_file_actions_destroy(actions: *mut *mut c_void) -> c_int;
    fn posix_spawn_file_actions_addopen(actions: *mut *mut c_void, fd: c_int, path: *const c_char, flags: c_int, mode: u16) -> c_int;
    fn posix_spawn(
        pid: *mut c_int,
        path: *const c_char,
        actions: *const *mut c_void,
        attr: *const *mut c_void,
        argv: *const *const c_char,
        envp: *const *const c_char,
    ) -> c_int;
    /// Makes the child its own "responsible process", so it gets none of the parent's privacy
    /// permissions. Not in the public headers, but exported by libSystem since macOS 10.14.
    fn responsibility_spawnattrs_setdisclaim(attr: *mut *mut c_void, disclaim: c_int) -> c_int;
    fn _NSGetEnviron() -> *const *const *const c_char;
    fn waitpid(pid: c_int, status: *mut c_int, options: c_int) -> c_int;
    fn kill(pid: c_int, signal: c_int) -> c_int;
}

pub fn find() -> Option<String> {
    let home = std::env::var("HOME").unwrap_or_default();
    [
        format!("{home}/Library/Android/sdk/platform-tools/adb"),
        String::from("/opt/homebrew/bin/adb"),
        String::from("/usr/local/bin/adb"),
    ]
    .into_iter()
    .find(|p| std::path::Path::new(p).is_file())
}

/// Runs a program disclaimed, with its input and output in files, and waits at most `timeout`
/// (then it is killed). Some(true) when it exited with success.
fn spawn_disclaimed(program: &str, args: &[&str], stdin: &str, stdout: &str, timeout: Duration) -> Option<bool> {
    let c = |s: &str| CString::new(s).ok();
    let (program, stdin, stdout, null) = (c(program)?, c(stdin)?, c(stdout)?, c("/dev/null")?);
    let args: Vec<CString> = args.iter().map(|a| c(a)).collect::<Option<_>>()?;
    let mut argv: Vec<*const c_char> = std::iter::once(program.as_ptr()).chain(args.iter().map(|a| a.as_ptr())).collect();
    argv.push(std::ptr::null());
    let mut pid: c_int = 0;
    let started = unsafe {
        let mut attr: *mut c_void = std::ptr::null_mut();
        let mut actions: *mut c_void = std::ptr::null_mut();
        if posix_spawnattr_init(&mut attr) != 0 {
            return None;
        }
        if posix_spawn_file_actions_init(&mut actions) != 0 {
            posix_spawnattr_destroy(&mut attr);
            return None;
        }
        let ready = responsibility_spawnattrs_setdisclaim(&mut attr, 1) == 0
            && posix_spawn_file_actions_addopen(&mut actions, 0, stdin.as_ptr(), O_RDONLY, 0) == 0
            && posix_spawn_file_actions_addopen(&mut actions, 1, stdout.as_ptr(), O_WRONLY_CREATE_TRUNCATE, 0o600) == 0
            && posix_spawn_file_actions_addopen(&mut actions, 2, null.as_ptr(), O_WRONLY_CREATE_TRUNCATE, 0o600) == 0;
        let rc = if ready { posix_spawn(&mut pid, program.as_ptr(), &actions, &attr, argv.as_ptr(), *_NSGetEnviron()) } else { -1 };
        posix_spawn_file_actions_destroy(&mut actions);
        posix_spawnattr_destroy(&mut attr);
        rc == 0
    };
    if !started {
        return None;
    }
    let begun = Instant::now();
    let mut status: c_int = 0;
    loop {
        match unsafe { waitpid(pid, &mut status, WNOHANG) } {
            0 if begun.elapsed() < timeout => std::thread::sleep(Duration::from_millis(50)),
            0 => unsafe {
                kill(pid, SIGKILL);
                waitpid(pid, &mut status, 0);
                return None;
            },
            // Exited normally (no signal in the low bits) with code 0.
            p if p == pid => return Some(status & 0x7f == 0 && (status >> 8) & 0xff == 0),
            _ => return None,
        }
    }
}

struct Adb {
    path: String,
    /// A folder only the owner can enter (the one that holds the token), for adb's input and output.
    dir: String,
}

impl Adb {
    /// Runs adb; `input` is what it reads (a script for `adb shell`). Returns its output on success.
    fn run(&self, args: &[&str], input: Option<&str>) -> Option<String> {
        let (stdin, stdout) = (format!("{}/adb-in", self.dir), format!("{}/adb-out", self.dir));
        if let Some(text) = input {
            // Made anew each time: a link planted in its place is never followed.
            let _ = std::fs::remove_file(&stdin);
            let mut file = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&stdin).ok()?;
            std::io::Write::write_all(&mut file, text.as_bytes()).ok()?;
        }
        let done = spawn_disclaimed(&self.path, args, if input.is_some() { &stdin } else { "/dev/null" }, &stdout, COMMAND_TIMEOUT);
        let out = std::fs::read_to_string(&stdout).unwrap_or_default();
        let _ = std::fs::remove_file(&stdin);
        let _ = std::fs::remove_file(&stdout);
        (done == Some(true)).then_some(out)
    }

    /// Serial numbers of the phones and emulators that are connected and authorised.
    fn devices(&self) -> Vec<String> {
        self.run(&["devices"], None).map(|out| parse_devices(&out)).unwrap_or_default()
    }
}

/// The serials from the output of `adb devices`: lines `<serial>\tdevice` after the heading.
pub fn parse_devices(out: &str) -> Vec<String> {
    out.lines().filter_map(|line| line.split_once('\t')).filter(|(_, state)| state.trim() == "device").map(|(serial, _)| serial.trim().to_string()).collect()
}

/// While the phone does not poll over USB for `idle_after`, sets up `adb reverse` on every
/// connected device: every 10 seconds at first, backing off to every 2 minutes while no phone
/// answers. This also runs while the phone polls over Wi-Fi, so plugging the cable in moves it
/// back to USB. When a phone answers and nobody polls at all, the panel is opened (again every
/// 5 minutes while it stays so).
pub fn run(path: String, dir: String, shared: Arc<Shared>, idle_after: Duration) {
    let adb = Adb { path, dir };
    let port = format!("tcp:{}", shared.port);
    let quiet = |stamp: &std::sync::atomic::AtomicU64| now_ms().saturating_sub(stamp.load(Ordering::Relaxed)) >= idle_after.as_millis() as u64;
    let mut wait = RETRY_MIN;
    let mut reachable: Option<bool> = None;
    let mut opened: Option<Instant> = None;
    loop {
        std::thread::sleep(Duration::from_secs(2));
        if !quiet(&shared.last_local_ms) {
            reachable = Some(true);
            opened = None;
            wait = RETRY_MIN;
            continue;
        }
        // With two devices adb refuses a command that does not name one.
        let phones: Vec<String> = adb.devices().into_iter().filter(|serial| adb.run(&["-s", serial, "reverse", &port, &port], None).is_some()).collect();
        let ok = !phones.is_empty();
        if reachable != Some(ok) {
            log(if ok { "adb: phone connected" } else { "adb: no phone on USB" });
        }
        reachable = Some(ok);
        if !quiet(&shared.last_request_ms) {
            opened = None;
        } else if ok && opened.is_none_or(|t| t.elapsed() >= Duration::from_secs(300)) {
            opened = Some(Instant::now());
            let script = format!("am start -n {CONFIG_ACTIVITY} --ei port {} --es token {}\n", shared.port, shared.token);
            for serial in &phones {
                adb.run(&["-s", serial, "shell"], Some(&script));
            }
        }
        wait = if ok { RETRY_MIN } else { (wait * 2).min(RETRY_MAX) };
        std::thread::sleep(wait.saturating_sub(Duration::from_secs(2)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn devices_are_parsed() {
        let out = "List of devices attached\n52033ae6e27a34c5\tdevice\nemulator-5554\toffline\nR58M\tunauthorized\n10.0.0.5:5555\tdevice\n\n";
        assert_eq!(parse_devices(out), vec!["52033ae6e27a34c5", "10.0.0.5:5555"]);
        assert!(parse_devices("").is_empty());
    }

    #[test]
    fn disclaimed_programs_run_with_files_for_input_and_output() {
        let dir = std::env::temp_dir().join(format!("deskstats-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let adb = Adb { path: "/bin/cat".into(), dir: dir.to_string_lossy().into_owned() };
        assert_eq!(adb.run(&[], Some("hello\n")), Some("hello\n".into()));
        let failing = Adb { path: "/usr/bin/false".into(), dir: adb.dir.clone() };
        assert_eq!(failing.run(&[], None), None);
        let started = Instant::now();
        assert_eq!(spawn_disclaimed("/bin/sleep", &["30"], "/dev/null", "/dev/null", Duration::from_millis(200)), None);
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(std::fs::read_dir(&dir).unwrap().next().is_none());
        std::fs::remove_dir(&dir).unwrap();
    }
}
