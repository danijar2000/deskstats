//! Linux: everything the server asks of the system it runs on. Nearly all of it is read from /proc and /sys.

#[path = "../portable.rs"]
mod portable;

pub use portable::{crypto, ec};
pub mod extra;
use std::collections::HashMap;
use std::ffi::CStr;
use std::net::Ipv4Addr;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub const OS: &str = "linux";
pub const CURL: &str = "curl";

fn read(path: impl AsRef<Path>) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

fn read_trimmed(path: impl AsRef<Path>) -> Option<String> {
    read(path).map(|text| text.trim().to_string()).filter(|text| !text.is_empty())
}

pub fn data_dir() -> String {
    let base = std::env::var("XDG_CONFIG_HOME").ok().filter(|dir| dir.starts_with('/'));
    let base = base.unwrap_or_else(|| format!("{}/.config", std::env::var("HOME").unwrap_or_default()));
    format!("{base}/deskstats")
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
    let time = secs as _;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    if unsafe { libc::localtime_r(&time, &mut tm) }.is_null() {
        return None;
    }
    Some([tm.tm_year + 1900, tm.tm_mon + 1, tm.tm_mday, tm.tm_hour, tm.tm_min, tm.tm_sec])
}

pub fn log_line(line: &str) {
    use std::io::Write;
    let _ = writeln!(std::io::stderr().lock(), "{line}");
}

/// The full path of a program found through PATH.
pub fn find_in_path(program: &str) -> Option<String> {
    let path = std::env::var("PATH").unwrap_or_else(|_| String::from("/usr/local/bin:/usr/bin:/bin"));
    path.split(':').filter(|dir| !dir.is_empty()).map(|dir| format!("{dir}/{program}")).find(|file| Path::new(file).is_file())
}

/// What a program needs to open a window, as "NAME=value". A service started at login may run before the desktop
/// is up and so not have these itself; the desktop tells the systemd user manager, which is asked then.
fn desktop_environment() -> (Vec<String>, bool) {
    const NAMES: [&str; 5] = ["DISPLAY", "WAYLAND_DISPLAY", "XAUTHORITY", "XDG_RUNTIME_DIR", "DBUS_SESSION_BUS_ADDRESS"];
    let own = |name: &str| std::env::var(name).ok().filter(|value| !value.is_empty());
    if own("DISPLAY").is_some() || own("WAYLAND_DISPLAY").is_some() {
        return (Vec::new(), true);
    }
    let shown = crate::util::run("systemctl", &["--user", "show-environment"], Duration::from_secs(5)).map(|(_, text)| text).unwrap_or_default();
    let found = parse_environment(&shown, &NAMES);
    let display = found.iter().any(|line| line.starts_with("DISPLAY=") || line.starts_with("WAYLAND_DISPLAY="));
    (found, display)
}

pub fn parse_environment(shown: &str, names: &[&str]) -> Vec<String> {
    shown.lines().filter(|line| line.split_once('=').is_some_and(|(name, value)| names.contains(&name) && !value.is_empty() && !value.starts_with("$'"))).map(String::from).collect()
}

/// Shows a dialog with two buttons and waits for the answer; true only for a click on [allow].
/// Uses zenity (GNOME and most desktops) or kdialog (KDE); without either nobody can be asked, so the answer is no.
pub fn ask(text: &str, deny: &str, allow: &str, wait: Duration) -> bool {
    let limit = wait + Duration::from_secs(10);
    let (environment, display) = desktop_environment();
    if !display {
        crate::util::log("pair: no display is known to this service, so the question may not show");
    }
    // "env NAME=value… program arguments…" runs the program with these added to its environment.
    let run = |program: &str, args: &[&str]| {
        let mut all: Vec<&str> = environment.iter().map(String::as_str).collect();
        all.push(program);
        all.extend_from_slice(args);
        crate::util::run("env", &all, limit).is_some_and(|(ok, _)| ok)
    };
    if let Some(zenity) = find_in_path("zenity") {
        // zenity reads the text as markup, and the name of the phone comes from the network.
        let plain = text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
        let seconds = wait.as_secs().to_string();
        return run(&zenity, &["--question", "--title", "Deskstats", "--text", &plain, "--ok-label", allow, "--cancel-label", deny, "--default-cancel", "--timeout", &seconds]);
    }
    if let Some(kdialog) = find_in_path("kdialog") {
        return run(&kdialog, &["--title", "Deskstats", "--warningyesno", &without_markup(text), "--yes-label", allow, "--no-label", deny]);
    }
    crate::util::log("pair: cannot ask whether to let the phone in; install zenity or kdialog");
    false
}

/// kdialog decides by the look of the text whether it is markup, and the name of the phone comes from the network:
/// a name with tags could hide the code. Without these three characters nothing in the text can be a tag.
pub fn without_markup(text: &str) -> String {
    text.chars().map(|c| match c { '<' => '‹', '>' => '›', '&' => '＆', other => other }).collect()
}

/// Notifications of other programs are not read on Linux: the answer simply has no "alerts".
pub struct Notifications;

impl Notifications {
    pub fn new() -> Self {
        Notifications
    }

    pub fn json(&mut self) -> (String, String) {
        (String::new(), String::new())
    }
}

pub struct Host;

impl Host {
    pub fn new() -> Self {
        Host
    }

    pub fn cpu_name(&self) -> String {
        read("/proc/cpuinfo").and_then(|text| parse_cpu_name(&text)).unwrap_or_else(|| std::env::consts::ARCH.to_string())
    }

    pub fn ram_total(&self) -> u64 {
        read("/proc/meminfo").and_then(|text| parse_meminfo(&text, "MemTotal")).unwrap_or(0)
    }

    pub fn ram_used(&self) -> u64 {
        let Some(text) = read("/proc/meminfo") else { return 0 };
        let total = parse_meminfo(&text, "MemTotal").unwrap_or(0);
        // MemAvailable is what programs could still get without swapping; the rest is in use.
        let available = parse_meminfo(&text, "MemAvailable").or_else(|| parse_meminfo(&text, "MemFree")).unwrap_or(0);
        total.saturating_sub(available)
    }

    /// Busy and total ticks of every core.
    pub fn cpu_ticks(&self) -> Vec<(u64, u64)> {
        read("/proc/stat").map(|text| parse_stat(&text)).unwrap_or_default()
    }
}

pub fn parse_cpu_name(cpuinfo: &str) -> Option<String> {
    ["model name", "Model", "Hardware", "cpu model"].iter().find_map(|key| {
        cpuinfo.lines().find_map(|line| {
            let (name, value) = line.split_once(':')?;
            (name.trim() == *key && !value.trim().is_empty()).then(|| value.split_whitespace().collect::<Vec<_>>().join(" "))
        })
    })
}

/// A value of /proc/meminfo, in bytes.
pub fn parse_meminfo(meminfo: &str, key: &str) -> Option<u64> {
    meminfo.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        if name.trim() != key {
            return None;
        }
        value.split_whitespace().next()?.parse::<u64>().ok().map(|kb| kb.saturating_mul(1024))
    })
}

/// The per-core lines of /proc/stat as (busy, total).
pub fn parse_stat(stat: &str) -> Vec<(u64, u64)> {
    let mut cores = Vec::new();
    for line in stat.lines() {
        let mut fields = line.split_whitespace();
        let Some(name) = fields.next() else { continue };
        if !name.starts_with("cpu") || name.len() == 3 || !name[3..].bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        // user nice system idle iowait irq softirq steal; guest time is already inside user.
        let values: Vec<u64> = fields.take(8).map(|field| field.parse().unwrap_or(0)).collect();
        let at = |i: usize| values.get(i).copied().unwrap_or(0);
        let busy = at(0) + at(1) + at(2) + at(5) + at(6) + at(7);
        cores.push((busy, busy + at(3) + at(4)));
    }
    cores
}

/// The interfaces of /proc/net/dev with the bytes received and sent.
pub fn parse_net_dev(text: &str) -> Vec<(String, u64, u64)> {
    text.lines()
        .filter_map(|line| {
            let (name, counters) = line.split_once(':')?;
            let fields: Vec<&str> = counters.split_whitespace().collect();
            Some((name.trim().to_string(), fields.first()?.parse().ok()?, fields.get(8)?.parse().ok()?))
        })
        .collect()
}

/// Network cards have a device behind them; bridges, containers and tunnels do not.
fn is_physical(name: &str) -> bool {
    Path::new(&format!("/sys/class/net/{name}/device")).exists()
}

fn is_virtual_name(name: &str) -> bool {
    name == "lo" || ["docker", "veth", "br-", "virbr", "tun", "tap", "lxc", "cni", "flannel", "zt", "tailscale", "wg"].iter().any(|prefix| name.starts_with(prefix))
}

/// Keeps the network cards; on a system that shows none (some containers and virtual machines), whatever is not plainly virtual.
fn only_cards<T>(all: Vec<(String, T)>) -> Vec<(String, T)> {
    if all.iter().any(|(name, _)| is_physical(name)) {
        all.into_iter().filter(|(name, _)| is_physical(name)).collect()
    } else {
        all.into_iter().filter(|(name, _)| !is_virtual_name(name)).collect()
    }
}

pub fn net_counters() -> HashMap<String, (u64, u64)> {
    let all = read("/proc/net/dev").map(|text| parse_net_dev(&text)).unwrap_or_default();
    only_cards(all.into_iter().map(|(name, rx, tx)| (name, (rx, tx))).collect()).into_iter().collect()
}

/// The address and the broadcast address of every network card that is up.
pub fn interfaces() -> Vec<(Ipv4Addr, Ipv4Addr)> {
    let mut all: Vec<(String, (Ipv4Addr, Ipv4Addr))> = Vec::new();
    unsafe {
        let mut list: *mut libc::ifaddrs = std::ptr::null_mut();
        if libc::getifaddrs(&mut list) != 0 {
            return Vec::new();
        }
        let mut cur = list;
        while !cur.is_null() {
            let ifa = &*cur;
            cur = ifa.ifa_next;
            let flags = ifa.ifa_flags as libc::c_int;
            let wanted = libc::IFF_UP | libc::IFF_RUNNING | libc::IFF_BROADCAST;
            if ifa.ifa_addr.is_null() || ifa.ifa_ifu.is_null() || ifa.ifa_name.is_null() || flags & wanted != wanted || flags & libc::IFF_LOOPBACK != 0 {
                continue;
            }
            if (*ifa.ifa_addr).sa_family as libc::c_int != libc::AF_INET || (*ifa.ifa_ifu).sa_family as libc::c_int != libc::AF_INET {
                continue;
            }
            let address = |socket: *const libc::sockaddr| Ipv4Addr::from(u32::from_be((*(socket as *const libc::sockaddr_in)).sin_addr.s_addr));
            let name = CStr::from_ptr(ifa.ifa_name).to_string_lossy().into_owned();
            all.push((name, (address(ifa.ifa_addr), address(ifa.ifa_ifu))));
        }
        libc::freeifaddrs(list);
    }
    only_cards(all).into_iter().map(|(_, addresses)| addresses).collect()
}

pub fn computer_name() -> String {
    read_trimmed("/proc/sys/kernel/hostname").or_else(|| std::env::var("HOSTNAME").ok().filter(|name| !name.is_empty())).unwrap_or_else(|| String::from("Linux"))
}

/// What the maker calls this computer, such as "ThinkPad X1 Carbon Gen 9" or "Raspberry Pi 4 Model B".
pub fn model_name() -> String {
    let placeholder = |name: &str| {
        let lower = name.to_lowercase();
        ["system product name", "to be filled", "default string", "not specified", "o.e.m", "unknown"].iter().any(|junk| lower.contains(junk))
    };
    ["/sys/class/dmi/id/product_name", "/sys/class/dmi/id/board_name", "/sys/firmware/devicetree/base/model"]
        .iter()
        .filter_map(|path| read(path).map(|text| text.trim_matches(|c: char| c == '\0' || c.is_whitespace()).to_string()))
        .find(|name| !name.is_empty() && !placeholder(name))
        .unwrap_or_else(|| String::from("Linux PC"))
}

/// The load of the graphics card, where the driver publishes it in /sys (AMD does; others are not read).
pub struct Gpu {
    cards: Vec<PathBuf>,
}

impl Gpu {
    pub fn new() -> Self {
        let mut cards = Vec::new();
        if let Ok(entries) = std::fs::read_dir("/sys/class/drm") {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                let device = entry.path().join("device");
                if name.starts_with("card") && name[4..].bytes().all(|b| b.is_ascii_digit()) && device.join("gpu_busy_percent").is_file() {
                    cards.push(device);
                }
            }
        }
        cards.sort();
        Gpu { cards }
    }

    /// Percent busy and bytes of video memory in use, of the busiest card.
    pub fn sample(&self) -> Option<(f64, u64)> {
        let number = |path: PathBuf| read_trimmed(path).and_then(|text| text.parse::<u64>().ok());
        self.cards
            .iter()
            .filter_map(|card| Some((number(card.join("gpu_busy_percent"))?.min(100) as f64, number(card.join("mem_info_vram_used")).unwrap_or(0))))
            .max_by(|a, b| a.0.total_cmp(&b.0))
    }
}

/// The batteries of wireless mice, keyboards and the like, as the kernel reports them.
pub struct Batteries {
    json: String,
    read_at: Option<Instant>,
}

impl Batteries {
    pub fn new() -> Self {
        Batteries { json: String::from("[]"), read_at: None }
    }

    pub fn refresh(&mut self) -> &str {
        if self.read_at.is_none_or(|t| t.elapsed() >= Duration::from_secs(60)) {
            self.read_at = Some(Instant::now());
            self.json = format!("[{}]", read_batteries(Path::new("/sys/class/power_supply")).join(","));
        }
        &self.json
    }
}

pub fn battery_kind(name: &str) -> &'static str {
    let lower = name.to_lowercase();
    if lower.contains("mouse") || lower.contains("mx master") || lower.contains("mx anywhere") {
        "mouse"
    } else if lower.contains("keyboard") || lower.contains("keys") {
        "keyboard"
    } else if lower.contains("trackpad") || lower.contains("touchpad") {
        "trackpad"
    } else {
        ""
    }
}

fn read_batteries(dir: &Path) -> Vec<String> {
    let mut items = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else { return items };
    let mut supplies: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).collect();
    supplies.sort();
    for supply in supplies {
        // "Device" marks the battery of something plugged in or paired; the laptop's own battery says "System" or nothing.
        if read_trimmed(supply.join("scope")).as_deref() != Some("Device") {
            continue;
        }
        let Some(pct) = read_trimmed(supply.join("capacity")).and_then(|text| text.parse::<u32>().ok()).filter(|pct| *pct <= 100) else { continue };
        let name = read_trimmed(supply.join("model_name")).unwrap_or_default();
        items.push(format!("{{\"kind\":\"{}\",\"name\":\"{}\",\"pct\":{pct}}}", battery_kind(&name), crate::util::json_escape(&name)));
    }
    items
}

pub mod power {
    use std::sync::atomic::{AtomicBool, Ordering};

    static STOPPING: AtomicBool = AtomicBool::new(false);

    extern "C" fn on_signal(_: libc::c_int) {
        STOPPING.store(true, Ordering::SeqCst);
    }

    /// Notes when the service is being stopped, so that the phone is told before the server is gone.
    pub fn start() {
        let handler = on_signal as extern "C" fn(libc::c_int) as libc::sighandler_t;
        unsafe {
            libc::signal(libc::SIGTERM, handler);
            libc::signal(libc::SIGINT, handler);
        }
    }

    pub fn state() -> &'static str {
        if stopping() {
            "sleep"
        } else {
            "awake"
        }
    }

    pub fn stopping() -> bool {
        STOPPING.load(Ordering::SeqCst)
    }
}

pub mod procs {
    use crate::procs::Counters;
    use std::collections::HashMap;

    /// Fills in every process from /proc; None when it cannot be read. Per-process network use is not known, hence false.
    pub fn read(pids: &mut HashMap<i32, Counters>) -> Option<bool> {
        let ticks = unsafe { libc::sysconf(libc::_SC_CLK_TCK) }.max(1) as u64;
        let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) }.max(1) as u64;
        for entry in std::fs::read_dir("/proc").ok()?.flatten() {
            let Some(pid) = entry.file_name().to_str().and_then(|name| name.parse::<i32>().ok()) else { continue };
            // A process may end between the listing and the reading; it is simply left out.
            let Some(time) = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok().and_then(|stat| parse_cpu_ticks(&stat)) else { continue };
            let resident = std::fs::read_to_string(format!("/proc/{pid}/statm")).ok().and_then(|statm| statm.split_whitespace().nth(1)?.parse::<u64>().ok()).unwrap_or(0);
            pids.insert(pid, Counters { cpu_cs: time.saturating_mul(100) / ticks, ram: resident.saturating_mul(page), ..Counters::default() });
        }
        (!pids.is_empty()).then_some(false)
    }

    /// User plus system time from a /proc/<pid>/stat line. The name in brackets may itself hold spaces and brackets.
    pub fn parse_cpu_ticks(stat: &str) -> Option<u64> {
        let after_name = &stat[stat.rfind(')')? + 1..];
        let mut fields = after_name.split_whitespace().skip(11);
        Some(fields.next()?.parse::<u64>().ok()? + fields.next()?.parse::<u64>().ok()?)
    }

    pub fn app_name(pid: i32) -> String {
        if let Ok(exe) = std::fs::read_link(format!("/proc/{pid}/exe")) {
            if let Some(name) = exe.file_name().and_then(|name| name.to_str()) {
                return name.trim_end_matches(" (deleted)").to_string();
            }
        }
        // Threads of the kernel have no command line; they are shown together.
        if std::fs::read(format!("/proc/{pid}/cmdline")).is_ok_and(|line| line.is_empty()) {
            return String::from("kernel");
        }
        std::fs::read_to_string(format!("/proc/{pid}/comm")).map(|name| name.trim().to_string()).unwrap_or_default()
    }
}

pub mod adb {
    use std::time::Duration;

    pub fn candidates() -> Vec<String> {
        let home = std::env::var("HOME").unwrap_or_default();
        // The installer names the adb it found in the user's shell: the service has another PATH.
        let mut list: Vec<String> = std::env::var("DESKSTATS_ADB").ok().filter(|path| path.starts_with('/')).into_iter().collect();
        list.extend(super::find_in_path("adb"));
        list.extend([format!("{home}/Android/Sdk/platform-tools/adb"), String::from("/usr/bin/adb"), String::from("/usr/local/bin/adb")]);
        list
    }

    /// Runs adb with [input] on its standard input; its output when it ended well in time.
    pub fn run(path: &str, _dir: &str, args: &[&str], input: Option<&str>, timeout: Duration) -> Option<String> {
        let (ok, out) = crate::util::run_with_input(path, args, input, timeout)?;
        ok.then_some(out)
    }

    fn usb_devices() -> Vec<std::ffi::OsString> {
        let mut names: Vec<_> = std::fs::read_dir("/sys/bus/usb/devices").map(|entries| entries.flatten().map(|entry| entry.file_name()).collect()).unwrap_or_default();
        names.sort();
        names
    }

    /// Says when a USB device comes or goes. The kernel lists them in /sys; reading that list every two seconds
    /// costs next to nothing and needs neither udev nor a netlink socket.
    pub fn watch_usb(changed: fn()) -> bool {
        if !std::path::Path::new("/sys/bus/usb/devices").is_dir() {
            return false;
        }
        std::thread::Builder::new()
            .name("usb".into())
            .stack_size(64 * 1024)
            .spawn(move || {
                let mut last = usb_devices();
                loop {
                    std::thread::sleep(Duration::from_secs(2));
                    let now = usb_devices();
                    if now != last {
                        last = now;
                        changed();
                    }
                }
            })
            .is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_cannot_be_markup() {
        assert_eq!(without_markup("<b><!-- A&B\ncode 123456"), "‹b›‹!-- A＆B\ncode 123456");
    }

    #[test]
    fn stat_lines_become_busy_and_total() {
        let stat = "cpu  100 2 30 400 5 6 7 8 9 0\ncpu0 10 1 3 40 5 0 1 2 0 0\ncpu1 x\nintr 5\ncpufreq 1 2 3\n";
        assert_eq!(parse_stat(stat), vec![(17, 62), (0, 0)]);
        assert!(!Host::new().cpu_ticks().is_empty());
    }

    #[test]
    fn meminfo_and_cpu_name() {
        let meminfo = "MemTotal:       16000 kB\nMemFree:  1000 kB\nMemAvailable:    6000 kB\n";
        assert_eq!(parse_meminfo(meminfo, "MemTotal"), Some(16000 * 1024));
        assert_eq!(parse_meminfo(meminfo, "MemAvailable"), Some(6000 * 1024));
        assert_eq!(parse_meminfo(meminfo, "Swap"), None);
        assert_eq!(parse_cpu_name("processor\t: 0\nmodel name\t: AMD  Ryzen 7 5800X 8-Core Processor\n"), Some("AMD Ryzen 7 5800X 8-Core Processor".into()));
        assert_eq!(parse_cpu_name("processor\t: 0\nModel\t\t: Raspberry Pi 4 Model B Rev 1.4\n"), Some("Raspberry Pi 4 Model B Rev 1.4".into()));
        assert_eq!(parse_cpu_name("processor\t: 0\n"), None);
        let host = Host::new();
        assert!(host.ram_total() > 0 && host.ram_used() > 0 && host.ram_used() <= host.ram_total());
        assert!(!host.cpu_name().is_empty());
    }

    #[test]
    fn net_dev_lines() {
        let text = "Inter-|   Receive                                                |  Transmit\n face |bytes    packets errs drop fifo frame compressed multicast|bytes    packets errs drop fifo colls carrier compressed\n    lo: 100 1 0 0 0 0 0 0 100 1 0 0 0 0 0 0\n  eth0: 5000 10 0 0 0 0 0 0 7000 12 0 0 0 0 0 0\n";
        assert_eq!(parse_net_dev(text), vec![("lo".to_string(), 100, 100), ("eth0".to_string(), 5000, 7000)]);
        assert!(!net_counters().contains_key("lo"));
        assert!(is_virtual_name("docker0") && is_virtual_name("lo") && !is_virtual_name("enp3s0") && !is_virtual_name("wlan0"));
    }

    #[test]
    fn the_system_answers() {
        assert!(!computer_name().is_empty());
        assert!(!model_name().is_empty());
        assert!(local_time(0).is_some());
        assert!(data_dir().ends_with("/deskstats"));
        for (ip, broadcast) in interfaces() {
            assert!(!ip.is_loopback() && !broadcast.is_unspecified());
        }
        let _ = Gpu::new().sample();
        let mut batteries = Batteries::new();
        assert!(batteries.refresh().starts_with('['));
        assert_eq!(Notifications::new().json(), (String::new(), String::new()));
    }

    #[test]
    fn batteries_of_devices_only() {
        let dir = std::env::temp_dir().join(format!("deskstats-power-{}", std::process::id()));
        for (name, scope, capacity, model) in [("hidpp_battery_0", "Device", "73", "MX Master 3"), ("BAT0", "System", "50", "laptop"), ("hid-1", "Device", "200", "broken")] {
            let supply = dir.join(name);
            std::fs::create_dir_all(&supply).unwrap();
            std::fs::write(supply.join("scope"), format!("{scope}\n")).unwrap();
            std::fs::write(supply.join("capacity"), format!("{capacity}\n")).unwrap();
            std::fs::write(supply.join("model_name"), format!("{model}\n")).unwrap();
        }
        assert_eq!(read_batteries(&dir), vec![r#"{"kind":"mouse","name":"MX Master 3","pct":73}"#.to_string()]);
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!((battery_kind("Logitech K380 Keyboard"), battery_kind("Magic Trackpad"), battery_kind("Pen")), ("keyboard", "trackpad", ""));
    }

    #[test]
    fn processes_are_read() {
        assert_eq!(procs::parse_cpu_ticks("42 (a b) c) S 1 42 42 0 -1 4194560 100 0 0 0 7 5 0 0 20 0 1 0 100 1000 50"), Some(12));
        assert_eq!(procs::parse_cpu_ticks("nonsense"), None);
        let mut pids = HashMap::new();
        assert_eq!(procs::read(&mut pids), Some(false));
        let me = std::process::id() as i32;
        assert!(pids[&me].ram > 0);
        assert!(!procs::app_name(me).is_empty());
        assert_eq!(procs::app_name(-5), "");
    }

    #[test]
    fn desktop_variables_are_picked() {
        let shown = "HOME=/home/u\nDISPLAY=:0\nWAYLAND_DISPLAY=wayland-0\nXAUTHORITY=$'/run/a b'\nLANG=C\nXDG_RUNTIME_DIR=\n";
        assert_eq!(parse_environment(shown, &["DISPLAY", "WAYLAND_DISPLAY", "XAUTHORITY", "XDG_RUNTIME_DIR"]), vec!["DISPLAY=:0", "WAYLAND_DISPLAY=wayland-0"]);
        assert!(parse_environment("", &["DISPLAY"]).is_empty());
    }

    #[test]
    fn programs_are_found_in_path() {
        assert!(find_in_path("sh").is_some());
        assert_eq!(find_in_path("no-such-program-here"), None);
    }
}
