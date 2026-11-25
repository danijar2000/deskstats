use crate::ffi::{kCFRunLoopDefaultMode, CFRunLoopAddSource, CFRunLoopGetCurrent, CFRunLoopRun, IOIteratorNext, IONotificationPortCreate, IONotificationPortGetRunLoopSource, IOObjectRelease, IOServiceAddMatchingNotification, IOServiceMatching};
use crate::http::Shared;
use crate::util::{log, now_ms};
use std::ffi::{c_char, c_int, c_void, CString};
use std::os::unix::fs::OpenOptionsExt;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

const CONFIG_ACTIVITY: &str = "app.deskstats/.ConfigActivity";
const COMMAND_TIMEOUT: Duration = Duration::from_secs(5);
/// How often to look at USB without any news from macOS: plugging in and pulling out wake the thread at once.
const FALLBACK: Duration = Duration::from_secs(300);
/// After a USB device comes, adb sees it a moment later, and a phone may still ask whether to allow debugging.
const AFTER_PLUG: [u64; 6] = [1, 2, 4, 8, 15, 30];

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
            p if p == pid => return Some(status & 0x7f == 0 && (status >> 8) & 0xff == 0),
            _ => return None,
        }
    }
}

struct Adb {
    path: String,
    dir: String,
}

impl Adb {
    fn run(&self, args: &[&str], input: Option<&str>) -> Option<String> {
        let (stdin, stdout) = (format!("{}/adb-in", self.dir), format!("{}/adb-out", self.dir));
        if let Some(text) = input {
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

    fn devices(&self) -> Vec<String> {
        self.run(&["devices"], None).map(|out| parse_devices(&out)).unwrap_or_default()
    }
}

pub fn parse_devices(out: &str) -> Vec<String> {
    out.lines().filter_map(|line| line.split_once('\t')).filter(|(_, state)| state.trim() == "device").map(|(serial, _)| serial.trim().to_string()).collect()
}

static USB: (Mutex<bool>, Condvar) = (Mutex::new(false), Condvar::new());

extern "C" fn on_usb(_: *mut c_void, iterator: u32) {
    unsafe { drain(iterator) };
    *USB.0.lock().unwrap_or_else(|e| e.into_inner()) = true;
    USB.1.notify_all();
}

unsafe fn drain(iterator: u32) {
    loop {
        let device = IOIteratorNext(iterator);
        if device == 0 {
            break;
        }
        IOObjectRelease(device);
    }
}

/// Asks macOS to say when a USB device comes or goes, so that nothing has to look for the phone while none is plugged in.
fn watch_usb() -> bool {
    std::thread::Builder::new()
        .name("usb".into())
        .stack_size(64 * 1024)
        .spawn(|| unsafe {
            let port = IONotificationPortCreate(0);
            if port.is_null() {
                log("adb: cannot watch USB; looking for the phone every few minutes instead");
                return;
            }
            for kind in [c"IOServiceFirstMatch", c"IOServiceTerminate"] {
                let mut iterator = 0u32;
                let matching = IOServiceMatching(c"IOUSBHostDevice".as_ptr());
                if IOServiceAddMatchingNotification(port, kind.as_ptr(), matching, on_usb, std::ptr::null_mut(), &mut iterator) == 0 {
                    drain(iterator);
                }
            }
            CFRunLoopAddSource(CFRunLoopGetCurrent(), IONotificationPortGetRunLoopSource(port), kCFRunLoopDefaultMode);
            CFRunLoopRun();
        })
        .is_ok()
}

/// Sleeps until USB changes or [most] passes; true when USB changed.
fn wait_for_usb(most: Duration) -> bool {
    let guard = USB.0.lock().unwrap_or_else(|e| e.into_inner());
    let (mut changed, _) = USB.1.wait_timeout_while(guard, most, |changed| !*changed).unwrap_or_else(|e| e.into_inner());
    std::mem::take(&mut *changed)
}

pub fn run(path: String, dir: String, shared: Arc<Shared>) {
    let adb = Adb { path, dir };
    let port = format!("tcp:{}", shared.port);
    let quiet = |stamp: &std::sync::atomic::AtomicU64| now_ms().saturating_sub(stamp.load(Ordering::Relaxed)) >= shared.settings.idle_after().as_millis() as u64;
    watch_usb();
    let mut known: Vec<String> = Vec::new();
    let mut reachable: Option<bool> = None;
    let mut plugged = true;
    loop {
        // Right after a change, look a few times while adb and the phone catch up; otherwise once.
        let looks: &[u64] = if plugged { &AFTER_PLUG } else { &[0] };
        for wait in looks {
            std::thread::sleep(Duration::from_secs(*wait));
            let phones: Vec<String> = adb.devices().into_iter().filter(|serial| adb.run(&["-s", serial, "reverse", &port, &port], None).is_some()).collect();
            let ok = !phones.is_empty();
            if reachable != Some(ok) {
                log(if ok { "adb: phone connected" } else { "adb: no phone on USB" });
            }
            reachable = Some(ok);
            // A phone that was just plugged in gets the token once, unless it is already showing this Mac.
            let new: Vec<&String> = phones.iter().filter(|serial| !known.contains(serial)).collect();
            if !new.is_empty() && quiet(&shared.last_request_ms) {
                let script = format!("am start -n {CONFIG_ACTIVITY} --ei port {} --es token {}\n", shared.port, shared.token);
                for serial in &new {
                    adb.run(&["-s", serial, "shell"], Some(&script));
                }
            }
            let settled = !new.is_empty();
            known = phones;
            if settled {
                break;
            }
        }
        plugged = wait_for_usb(FALLBACK);
        // A phone that reads this Mac over the cable is fine as it is: nothing to do until USB changes.
        while !plugged && !quiet(&shared.last_local_ms) {
            plugged = wait_for_usb(FALLBACK);
        }
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
