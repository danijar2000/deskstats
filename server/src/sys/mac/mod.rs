//! macOS: everything the server asks of the system it runs on.

pub mod adb;
pub mod alerts;
pub mod crypto;
pub mod ec;
pub mod ffi;
mod net;
pub mod power;
pub mod procs;
mod sampler;

pub use net::interfaces;
pub use sampler::{computer_name, model_name, net_counters, Batteries, Gpu, Host};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::time::Duration;

pub const OS: &str = "macos";
pub const CURL: &str = "/usr/bin/curl";

pub fn data_dir() -> String {
    format!("{}/Library/Application Support/deskstats", std::env::var("HOME").unwrap_or_default())
}

/// Makes the folder of the token readable by this user only.
pub fn private_dir(dir: &str) -> std::io::Result<()> {
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
}

/// Creates or empties a file that only this user can read.
pub fn private_file(path: &str) -> std::io::Result<std::fs::File> {
    std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(path)
}

/// Year, month, day, hour, minute and second on the wall clock.
pub fn local_time(secs: i64) -> Option<[i32; 6]> {
    let mut tm: ffi::Tm = unsafe { std::mem::zeroed() };
    if unsafe { ffi::localtime_r(&secs, &mut tm) }.is_null() {
        return None;
    }
    Some([tm.tm_year + 1900, tm.tm_mon + 1, tm.tm_mday, tm.tm_hour, tm.tm_min, tm.tm_sec])
}

pub fn log_line(line: &str) {
    use std::io::Write;
    let _ = writeln!(std::io::stderr().lock(), "{line}");
}

/// Shows a dialog with two buttons and waits for the answer; true only for a click on [allow].
pub fn ask(text: &str, deny: &str, allow: &str, wait: Duration) -> bool {
    let script = [
        "on run argv",
        "set r to display dialog (item 1 of argv) with title \"Deskstats\" buttons {item 2 of argv, item 3 of argv} default button 2 with icon caution giving up after (item 4 of argv as integer)",
        "if gave up of r then return \"\"",
        "return button returned of r",
        "end run",
    ];
    let mut args: Vec<&str> = Vec::new();
    for line in &script {
        args.push("-e");
        args.push(line);
    }
    let seconds = wait.as_secs().to_string();
    args.extend_from_slice(&["--", text, deny, allow, &seconds]);
    let answer = crate::util::run("/usr/bin/osascript", &args, wait + Duration::from_secs(10));
    answer.as_ref().is_some_and(|(ok, out)| *ok && out.trim() == allow)
}

/// The notifications of the Mac and of the iPhone that mirrors to it.
pub struct Notifications {
    database: alerts::Database,
    screen: alerts::Screen,
}

impl Notifications {
    pub fn new() -> Self {
        Notifications { database: alerts::Database::new(), screen: alerts::Screen::new() }
    }

    /// The "alerts" member of the answer, short and full; null when the system does not let the server read them.
    pub fn json(&mut self) -> (String, String) {
        let on_screen = self.screen.refresh();
        let now = crate::util::now_ms();
        let list = match self.database.refresh() {
            Some(rows) => Some(alerts::merge(rows, on_screen, now)),
            None if !on_screen.is_empty() => Some(alerts::merge(&[], on_screen, now)),
            None => None,
        };
        (alerts::list_json(list.as_deref(), alerts::SHORT), alerts::list_json(list.as_deref(), alerts::FULL))
    }
}
