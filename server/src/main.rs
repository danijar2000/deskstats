// On Windows the server runs without a console window.
#![cfg_attr(windows, windows_subsystem = "windows")]

mod adb;
mod crypto;
mod http;
mod lan;
mod load;
mod pair;
mod ping;
mod power;
mod procs;
mod settings;
#[cfg_attr(target_os = "macos", path = "sys/mac/mod.rs")]
#[cfg_attr(target_os = "linux", path = "sys/linux/mod.rs")]
#[cfg_attr(windows, path = "sys/windows/mod.rs")]
mod sys;
mod util;
mod weather;

use std::fs;
use std::net::TcpListener;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};
use util::{json_escape, log, now_ms};

const PORT: u16 = 8787;
/// The release this was built for (the workflow passes the tag); a build by hand has the version of the crate.
pub const VERSION: &str = match option_env!("DESKSTATS_VERSION") {
    Some(version) if !version.is_empty() => version,
    _ => env!("CARGO_PKG_VERSION"),
};

type Counters = (Vec<(u64, u64)>, std::collections::HashMap<String, (u64, u64)>, Option<(u64, u64)>, Instant);

/// How often the free space of the disk and the CPU temperature are read: they change slowly.
const DISK_EVERY: Duration = Duration::from_secs(60);
const TEMP_EVERY: Duration = Duration::from_secs(5);

use sys::data_dir;

fn load_token() -> std::io::Result<String> {
    let dir = data_dir();
    let path = format!("{dir}/token");
    if let Ok(token) = fs::read_to_string(&path) {
        let token = token.trim().to_string();
        if (32..=128).contains(&token.len()) && token.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Ok(token);
        }
    }
    fs::create_dir_all(&dir)?;
    sys::private_dir(&dir)?;
    let token = crypto::random_hex(24);
    let mut file = sys::private_file(&path)?;
    std::io::Write::write_all(&mut file, token.as_bytes())?;
    log("token: created a new one; the phone gets it the next time it is connected over USB");
    Ok(token)
}

/// Whether a phone, or the window on this computer, asked for numbers lately.
fn someone_looks(shared: &http::Shared, settings: &settings::Settings) -> bool {
    let last = shared.last_request_ms.load(Ordering::Relaxed).max(shared.last_window_ms.load(Ordering::Relaxed));
    last != 0 && now_ms().saturating_sub(last) <= settings.idle_after().as_millis() as u64
}

fn spawn(name: &str, work: impl FnOnce() + Send + 'static) -> bool {
    let started = std::thread::Builder::new().name(name.into()).spawn(work).is_ok();
    if !started {
        log(&format!("cannot start the {name} thread"));
    }
    started
}

fn nap(time: Duration) {
    let until = Instant::now() + time;
    while !power::stopping() {
        let left = until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break;
        }
        std::thread::sleep(left.min(Duration::from_millis(200)));
    }
}

struct Load<'a> {
    ts: u64,
    interval_ms: u128,
    mac: &'a str,
    name: &'a str,
    cpu_name: &'a str,
    cpu_pct: f64,
    gpu: Option<(f64, u64)>,
    ram_used: u64,
    ram_total: u64,
    down_bps: f64,
    up_bps: f64,
    /// The disk's size and free space, and how fast it is read and written.
    disk: Option<(u64, u64)>,
    disk_rates: Option<(f64, f64)>,
    uptime: Option<u64>,
    temp: Option<f64>,
    ping: Option<u32>,
    batteries: &'a str,
}

fn opt<T: std::fmt::Display>(value: Option<T>) -> String {
    value.map_or_else(|| String::from("null"), |v| v.to_string())
}

fn disk_json(l: &Load) -> String {
    let Some((total, free)) = l.disk else { return String::from("null") };
    let (read, write) = match l.disk_rates {
        Some((r, w)) => (format!("{r:.0}"), format!("{w:.0}")),
        None => (String::from("null"), String::from("null")),
    };
    format!("{{\"total\":{total},\"free\":{free},\"read_bps\":{read},\"write_bps\":{write}}}")
}

fn head_json(l: &Load) -> String {
    let gpu = match l.gpu {
        Some((pct, mem)) => format!("{{\"pct\":{pct:.0},\"mem_used\":{mem}}}"),
        None => String::from("null"),
    };
    let ram_pct = if l.ram_total > 0 { l.ram_used as f64 * 100.0 / l.ram_total as f64 } else { 0.0 };
    format!(
        "{{\"ts\":{},\"interval_ms\":{},\"mac\":\"{}\",\"os\":\"{}\",\"name\":\"{}\",\"cpu\":{{\"name\":\"{}\",\"pct\":{:.1}}},\"gpu\":{gpu},\"ram\":{{\"used\":{},\"total\":{},\"pct\":{ram_pct:.1}}},\"net\":{{\"down_bps\":{:.0},\"up_bps\":{:.0}}},\"disk\":{},\"uptime\":{},\"temp\":{},\"ping\":{},\"batteries\":{},",
        l.ts, l.interval_ms, l.mac, sys::OS, l.name, l.cpu_name, l.cpu_pct, l.ram_used, l.ram_total, l.down_bps, l.up_bps,
        disk_json(l), opt(l.uptime), opt(l.temp.map(|t| format!("{t:.0}"))), opt(l.ping), l.batteries,
    )
}

fn main() {
    #[cfg(target_os = "macos")]
    if std::env::args().nth(1).as_deref() == Some("--ax-check") {
        println!("{}", if sys::alerts::accessibility_granted() { "yes" } else { "no" });
        return;
    }
    let token = match load_token() {
        Ok(t) => t,
        Err(e) => {
            log(&format!("cannot read or create the token: {e}"));
            std::process::exit(1);
        }
    };
    let listener = match TcpListener::bind(("0.0.0.0", PORT)) {
        Ok(l) => l,
        Err(e) => {
            log(&format!("cannot listen on port {PORT}: {e}"));
            std::process::exit(1);
        }
    };
    let settings = Arc::new(settings::Settings::load(format!("{}/settings", data_dir())));
    log(&format!("listening on port {PORT}"));

    power::start();
    let weather = Arc::new(weather::Weather::new(Duration::from_secs(settings.weather_min.load(Ordering::Relaxed) * 60)));
    let computer = sys::computer_name();
    let about = lan::about_json(&crypto::id_of(&token), &computer, &sys::model_name(), PORT);
    let mut shared = http::Shared::new(token.clone(), PORT, weather.clone(), settings.clone());
    shared.about = about.clone();
    let shared = Arc::new(shared);
    {
        let shared = shared.clone();
        if !spawn("accept", move || http::run(listener, shared)) {
            std::process::exit(1);
        }
    }
    {
        let keys = crypto::Keys::from_token(&token);
        let settings = settings.clone();
        spawn("beacon", move || lan::run_beacon(keys, PORT, about, settings));
    }
    spawn("weather", move || weather::run(weather));
    match adb::find() {
        Some(path) => {
            shared.adb.store(true, Ordering::Relaxed);
            let shared = shared.clone();
            spawn("adb", move || adb::run(path, data_dir(), shared));
        }
        None => log("adb: not found; the phone connects over Wi-Fi only"),
    }

    let machine = sys::Host::new();
    let cpu_name = json_escape(&machine.cpu_name());
    let name = json_escape(&computer);
    let ram_total = machine.ram_total();
    let gpu = sys::Gpu::new();
    let mut batteries = sys::Batteries::new();
    let mut notifications = sys::Notifications::new();
    let mut thermal = sys::extra::Thermal::new();
    let (mut disk, mut disk_at): (Option<(u64, u64)>, Option<Instant>) = (None, None);
    let (mut temp, mut temp_at): (Option<f64>, Option<Instant>) = (None, None);
    {
        let (shared, settings) = (shared.clone(), settings.clone());
        spawn("ping", move || {
            ping::run(move || someone_looks(&shared, &settings))
        });
    }

    let mut prev: Option<Counters> = None;
    loop {
        if power::stopping() {
            log("stopping: telling the phone the computer goes away");
            // Under launchd nothing keeps what the server writes; who stopped it is worth keeping.
            #[cfg(target_os = "macos")]
            {
                let pid = sys::power::stop_sender();
                let who = util::run("/bin/ps", &["-o", "command=", "-p", &pid.to_string()], Duration::from_secs(2)).map(|(_, out)| out.trim().to_string()).unwrap_or_default();
                let line = format!("{} stopped by pid {pid}: {who}\n", now_ms());
                let path = format!("{}/stops.log", data_dir());
                let _ = fs::OpenOptions::new().create(true).append(true).open(&path).and_then(|mut f| std::io::Write::write_all(&mut f, line.as_bytes()));
            }
            power::wait_for_poll(now_ms());
            std::process::exit(0);
        }
        shared.procs.forget_when_idle();
        let interval = settings.interval();
        let active = someone_looks(&shared, &settings);
        if !active {
            // The sample is dropped so that the next poll gets 503, not numbers from before the pause.
            if prev.take().is_some() {
                *shared.sample.lock().unwrap_or_else(|e| e.into_inner()) = None;
            }
            // Asleep: nothing is sampled until a phone asks again. The second-long timeout only checks for a stop.
            shared.wait_for_request(Duration::from_secs(1));
            continue;
        }
        let Some((prev_cpu, prev_net, prev_disk, prev_at)) = prev.take() else {
            prev = Some((machine.cpu_ticks(), sys::net_counters(), sys::extra::disk_counters(), Instant::now()));
            nap(Duration::from_millis(250));
            continue;
        };

        let cpu = machine.cpu_ticks();
        let net = sys::net_counters();
        let disk_now = sys::extra::disk_counters();
        let at = Instant::now();
        if disk_at.is_none_or(|t| t.elapsed() >= DISK_EVERY) {
            disk = sys::extra::disk();
            disk_at = Some(at);
        }
        if temp_at.is_none_or(|t| t.elapsed() >= TEMP_EVERY) {
            temp = thermal.celsius();
            temp_at = Some(at);
        }
        let seconds = at.duration_since(prev_at).as_secs_f64().max(0.001);
        let (rx, tx) = load::net_delta(&prev_net, &net);
        let head = head_json(&Load {
            ts: now_ms(),
            interval_ms: interval.as_millis(),
            mac: power::state(),
            name: &name,
            cpu_name: &cpu_name,
            cpu_pct: load::cpu_percent(&prev_cpu, &cpu),
            gpu: gpu.sample(),
            ram_used: machine.ram_used(),
            ram_total,
            down_bps: rx as f64 / seconds,
            up_bps: tx as f64 / seconds,
            disk,
            disk_rates: match (prev_disk, disk_now) {
                (Some((r0, w0)), Some((r1, w1))) => Some((r1.saturating_sub(r0) as f64 / seconds, w1.saturating_sub(w0) as f64 / seconds)),
                _ => None,
            },
            uptime: sys::extra::uptime(),
            temp,
            ping: ping::last(),
            batteries: batteries.refresh(),
        });
        let (short, full) = if settings.alerts.load(Ordering::Relaxed) { notifications.json() } else { (String::new(), String::new()) };
        *shared.sample.lock().unwrap_or_else(|e| e.into_inner()) = Some(http::Sample { head, alerts: short, alerts_full: full });
        prev = Some((cpu, net, disk_now, at));
        nap(interval.saturating_sub(at.elapsed()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_json_has_the_documented_shape() {
        let load = Load {
            ts: 1700000000000,
            interval_ms: 1000,
            mac: "awake",
            name: "Studio",
            cpu_name: "Apple M4",
            cpu_pct: 12.34,
            gpu: Some((9.0, 1048576)),
            ram_used: 8,
            ram_total: 16,
            down_bps: 1500.4,
            up_bps: 0.0,
            disk: Some((100, 40)),
            disk_rates: Some((2048.4, 0.0)),
            uptime: Some(3600),
            temp: Some(61.6),
            ping: None,
            batteries: "[]",
        };
        let alerts = "\"alerts\":[{\"id\":7}],";
        let sample = http::Sample { head: head_json(&load), alerts: alerts.into(), alerts_full: "\"alerts\":null,".into() };
        assert_eq!(
            sample.json(false, "null", "[]"),
            format!(
                "{{\"ts\":1700000000000,\"interval_ms\":1000,\"mac\":\"awake\",\"os\":\"{}\",\"name\":\"Studio\",\"cpu\":{{\"name\":\"Apple M4\",\"pct\":12.3}},\"gpu\":{{\"pct\":9,\"mem_used\":1048576}},\
                 \"ram\":{{\"used\":8,\"total\":16,\"pct\":50.0}},\"net\":{{\"down_bps\":1500,\"up_bps\":0}},\
                 \"disk\":{{\"total\":100,\"free\":40,\"read_bps\":2048,\"write_bps\":0}},\"uptime\":3600,\"temp\":62,\"ping\":null,\"batteries\":[],\
                 \"alerts\":[{{\"id\":7}}],\"pushed\":[],\"weather\":null}}",
                sys::OS
            )
        );
        assert!(sample.json(true, "{}", "[]").ends_with("\"batteries\":[],\"alerts\":null,\"pushed\":[],\"weather\":{}}"));
        let none = Load { gpu: None, ram_total: 0, disk: None, uptime: None, temp: None, ..load };
        assert!(head_json(&none).contains("\"gpu\":null,\"ram\":{\"used\":8,\"total\":0,\"pct\":0.0}"));
        assert!(head_json(&none).contains("\"disk\":null,\"uptime\":null,\"temp\":null,\"ping\":null,"));
    }
}
