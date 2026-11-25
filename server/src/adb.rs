use crate::http::Shared;
use crate::util::{log, now_ms};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

const CONFIG_ACTIVITY: &str = "app.deskstats/.ConfigActivity";
const COMMAND_TIMEOUT: Duration = Duration::from_secs(5);
/// How often to look at USB without any news from the system: plugging in and pulling out wake the thread at once.
const FALLBACK: Duration = Duration::from_secs(300);
/// After a USB device comes, adb sees it a moment later, and a phone may still ask whether to allow debugging.
const AFTER_PLUG: [u64; 6] = [1, 2, 4, 8, 15, 30];

pub fn find() -> Option<String> {
    crate::sys::adb::candidates().into_iter().find(|p| std::path::Path::new(p).is_file())
}

struct Adb {
    path: String,
    dir: String,
}

impl Adb {
    fn run(&self, args: &[&str], input: Option<&str>) -> Option<String> {
        crate::sys::adb::run(&self.path, &self.dir, args, input, COMMAND_TIMEOUT)
    }

    fn devices(&self) -> Vec<String> {
        self.run(&["devices"], None).map(|out| parse_devices(&out)).unwrap_or_default()
    }
}

pub fn parse_devices(out: &str) -> Vec<String> {
    out.lines().filter_map(|line| line.split_once('\t')).filter(|(_, state)| state.trim() == "device").map(|(serial, _)| serial.trim().to_string()).collect()
}

static USB: (Mutex<bool>, Condvar) = (Mutex::new(false), Condvar::new());

fn usb_changed() {
    *USB.0.lock().unwrap_or_else(|e| e.into_inner()) = true;
    USB.1.notify_all();
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
    if !crate::sys::adb::watch_usb(usb_changed) {
        log("adb: cannot watch USB; looking for the phone every few minutes instead");
    }
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

    #[cfg(unix)]
    #[test]
    fn programs_run_with_input_and_leave_nothing_behind() {
        let dir = std::env::temp_dir().join(format!("deskstats-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let adb = Adb { path: "/bin/cat".into(), dir: dir.to_string_lossy().into_owned() };
        assert_eq!(adb.run(&[], Some("hello\n")), Some("hello\n".into()));
        let failing = Adb { path: "/usr/bin/false".into(), dir: adb.dir.clone() };
        assert_eq!(failing.run(&[], None), None);
        let started = std::time::Instant::now();
        assert_eq!(crate::sys::adb::run("/bin/sleep", &adb.dir, &["30"], None, Duration::from_millis(200)), None);
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(std::fs::read_dir(&dir).unwrap().next().is_none());
        std::fs::remove_dir(&dir).unwrap();
    }
}
