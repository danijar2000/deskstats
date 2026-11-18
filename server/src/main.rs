//! deskstats: serves the load of this Mac (CPU, GPU, RAM, network), the battery level of its
//! wireless mouse and keyboard, the weather and the recent notifications as JSON for a phone
//! on the desk.
//!
//! A phone on USB reaches the server through `adb reverse` (set up by the server while nobody
//! polls). Its requests and the answers are sealed with keys from a token (see http.rs); while
//! it shows the list of applications it also asks for them (see procs.rs).
//! The token lives in ~/Library/Application Support/deskstats/token (created on the first
//! start) and reaches the phone over USB, once. With `--lan` the server also answers on Wi-Fi,
//! and a signed UDP beacon tells the phone the Mac's address. Android before 5.0 has no
//! `adb reverse` and can only use Wi-Fi.

mod adb;
mod alerts;
mod crypto;
mod ffi;
mod http;
mod lan;
mod power;
mod procs;
mod sampler;
mod util;
mod weather;

use std::fs;
use std::io::Read;
use std::net::TcpListener;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};
use util::{json_escape, log, now_ms};

/// CPU ticks, network counters and when they were read.
type Counters = (Vec<(u64, u64)>, std::collections::HashMap<String, (u64, u64)>, Instant);

/// The log launchd writes; cut at startup when it has grown past this.
const LOG_LIMIT: u64 = 1024 * 1024;

struct Options {
    interval: Duration,
    port: u16,
    city: Option<String>,
    weather_every: Duration,
    alerts: bool,
    lan: bool,
    adb: bool,
    ax_dump: bool,
    check: bool,
}

const USAGE: &str = "\
usage: deskstats [--interval <seconds>] [--port <port>] [--city <name>] [--weather-hours <hours>]
                 [--no-alerts] [--lan] [--no-adb] [--ax-dump] [--check]
  --interval       how often to sample the load, in seconds (0.2 to 3600, default 1)
  --port           port to listen on (default 8787)
  --city           the weather shown until a city is picked on the phone, e.g. \"Bishkek\"
  --weather-hours  how often to ask for the weather, in hours (0.25 to 168, default 1)
  --no-alerts      do not serve notifications
  --lan            also answer on Wi-Fi and announce this Mac to the phone
  --no-adb         do not set up `adb reverse` for a phone on USB
  --ax-dump        write the Notification Center element tree (it holds notification texts)
                   to ~/Library/Logs/deskstats-ax.log
  --check          only check the options and exit
  --help           show this";

fn usage(problem: &str) -> ! {
    eprintln!("deskstats: {problem}");
    eprintln!("{USAGE}");
    std::process::exit(2);
}

fn parse_options() -> Options {
    let mut o = Options {
        interval: Duration::from_secs(1),
        port: 8787,
        city: None,
        weather_every: Duration::from_secs(3600),
        alerts: true,
        lan: false,
        adb: true,
        ax_dump: false,
        check: false,
    };
    let mut args = std::env::args().skip(1);
    let seconds = |value: Option<String>, min: f64, max: f64, name: &str| -> Duration {
        let v: f64 = value.and_then(|v| v.parse().ok()).unwrap_or_else(|| usage(&format!("{name} needs a number")));
        if !(min..=max).contains(&v) {
            usage(&format!("{name} must be between {min} and {max}"));
        }
        Duration::from_secs_f64(v)
    };
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--interval" => o.interval = seconds(args.next(), 0.2, 3600.0, "--interval"),
            "--weather-hours" => o.weather_every = seconds(args.next(), 0.25, 168.0, "--weather-hours") * 3600,
            "--port" => o.port = args.next().and_then(|v| v.parse().ok()).filter(|p| *p > 0).unwrap_or_else(|| usage("--port needs a port number")),
            // A name that starts with "--" is the next option: the name was forgotten.
            "--city" => {
                let valid = |v: &String| !v.trim().is_empty() && !v.contains('\0') && !v.starts_with("--");
                o.city = Some(args.next().filter(valid).unwrap_or_else(|| usage("--city needs a name")))
            }
            "--no-alerts" => o.alerts = false,
            "--lan" => o.lan = true,
            "--no-adb" => o.adb = false,
            "--ax-dump" => o.ax_dump = true,
            "--check" => o.check = true,
            "--help" | "-h" => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            other => usage(&format!("unknown option {other}")),
        }
    }
    o
}

/// The folder of the token, which only the owner can enter.
fn data_dir() -> String {
    format!("{}/Library/Application Support/deskstats", std::env::var("HOME").unwrap_or_default())
}

/// The shared secret of the server and the phone, created once with 0600 permissions.
fn load_token() -> std::io::Result<String> {
    let dir = data_dir();
    let path = format!("{dir}/token");
    if let Ok(token) = fs::read_to_string(&path) {
        let token = token.trim().to_string();
        // The phone takes 32 to 128 characters.
        if (32..=128).contains(&token.len()) && token.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Ok(token);
        }
    }
    fs::create_dir_all(&dir)?;
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
    let mut random = [0u8; 24];
    fs::File::open("/dev/urandom")?.read_exact(&mut random)?;
    let token: String = random.iter().map(|b| format!("{b:02x}")).collect();
    let mut file = fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&path)?;
    std::io::Write::write_all(&mut file, token.as_bytes())?;
    log("token: created a new one; the phone gets it the next time it is connected over USB");
    Ok(token)
}

/// Starts a named thread; false (and a log line) when the system refuses.
fn spawn(name: &str, work: impl FnOnce() + Send + 'static) -> bool {
    let started = std::thread::Builder::new().name(name.into()).spawn(work).is_ok();
    if !started {
        log(&format!("cannot start the {name} thread"));
    }
    started
}

/// Sleeps in short steps, so a request to stop is noticed at once however long the interval.
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

/// What one sample measured, besides the notifications.
struct Load<'a> {
    ts: u64,
    interval_ms: u128,
    mac: &'a str,
    /// Already escaped for JSON.
    cpu_name: &'a str,
    cpu_pct: f64,
    gpu: Option<(f64, u64)>,
    ram_used: u64,
    ram_total: u64,
    down_bps: f64,
    up_bps: f64,
    /// A JSON array.
    batteries: &'a str,
}

/// The sample's JSON up to the notifications (see http::Sample).
fn head_json(l: &Load) -> String {
    let gpu = match l.gpu {
        Some((pct, mem)) => format!("{{\"pct\":{pct:.0},\"mem_used\":{mem}}}"),
        None => String::from("null"),
    };
    let ram_pct = if l.ram_total > 0 { l.ram_used as f64 * 100.0 / l.ram_total as f64 } else { 0.0 };
    format!(
        "{{\"ts\":{},\"interval_ms\":{},\"mac\":\"{}\",\"cpu\":{{\"name\":\"{}\",\"pct\":{:.1}}},\"gpu\":{gpu},\"ram\":{{\"used\":{},\"total\":{},\"pct\":{ram_pct:.1}}},\"net\":{{\"down_bps\":{:.0},\"up_bps\":{:.0}}},\"batteries\":{},",
        l.ts, l.interval_ms, l.mac, l.cpu_name, l.cpu_pct, l.ram_used, l.ram_total, l.down_bps, l.up_bps, l.batteries,
    )
}

/// `"alerts":[…],` with at most `count` notifications; `"alerts":null,` when no source can be read.
fn alerts_json(list: Option<&[alerts::Item]>, count: usize) -> String {
    match list {
        Some(items) => format!("\"alerts\":[{}],", items.iter().take(count).map(alerts::Item::json).collect::<Vec<_>>().join(",")),
        None => String::from("\"alerts\":null,"),
    }
}

fn main() {
    // Not an option for people: the server asks a fresh copy of itself (see alerts.rs).
    if std::env::args().nth(1).as_deref() == Some("--ax-check") {
        println!("{}", if alerts::accessibility_granted() { "yes" } else { "no" });
        return;
    }
    let o = parse_options();
    if o.check {
        println!("ok");
        return;
    }
    util::trim_log(&format!("{}/Library/Logs/deskstats.log", std::env::var("HOME").unwrap_or_default()), LOG_LIMIT);
    let token = match load_token() {
        Ok(t) => t,
        Err(e) => {
            log(&format!("cannot read or create the token: {e}"));
            std::process::exit(1);
        }
    };
    // With --lan the socket is open to every interface; which connections are served is
    // decided as they arrive (see http::Shared::arrived_on_own_network).
    let host = if o.lan { "0.0.0.0" } else { "127.0.0.1" };
    let listener = match TcpListener::bind((host, o.port)) {
        Ok(l) => l,
        Err(e) => {
            log(&format!("cannot listen on {host}:{}: {e}", o.port));
            std::process::exit(1);
        }
    };
    log(&format!(
        "listening on {host}:{}, sampling every {:.1} s{}",
        o.port,
        o.interval.as_secs_f64(),
        o.city.as_ref().map(|c| format!(", weather for \"{c}\"")).unwrap_or_default()
    ));

    power::start();
    let weather = Arc::new(weather::Weather::new(o.weather_every));
    let shared = Arc::new(http::Shared::new(token.clone(), o.port, weather.clone()));
    {
        let shared = shared.clone();
        // Without this thread there is no server.
        if !spawn("accept", move || http::run(listener, shared)) {
            std::process::exit(1);
        }
    }
    if o.lan {
        let keys = crypto::Keys::from_token(&token);
        let port = o.port;
        spawn("beacon", move || lan::run_beacon(keys, port));
    }
    {
        // Also without --city: the phone may pick a place of its own.
        let city = o.city.clone();
        spawn("weather", move || weather::run(city, weather));
    }
    // Nobody polling for this long means the phone is gone: stop sampling, look for it instead.
    let idle_after = (o.interval * 3).max(Duration::from_secs(5));
    if o.adb {
        match adb::find() {
            Some(path) => {
                let shared = shared.clone();
                spawn("adb", move || adb::run(path, data_dir(), shared, idle_after));
            }
            None => log("adb: not found, the phone has to be connected by hand"),
        }
    }

    let machine = sampler::Host::new();
    let cpu_name = json_escape(&sampler::sysctl_string("machdep.cpu.brand_string"));
    let ram_total = sampler::sysctl_u64("hw.memsize");
    let page_size = sampler::sysctl_u64("hw.pagesize").max(4096);
    let gpu = sampler::Gpu::new();
    let mut batteries = sampler::Batteries::new();
    let serve_alerts = o.alerts;
    let mut database = alerts::Database::new();
    let mut screen = alerts::Screen::new(o.ax_dump);

    let mut prev: Option<Counters> = None;
    loop {
        if power::stopping() {
            // Shutdown, logout or launchctl stop: let the phone see "sleep" before going away.
            log("stopping: telling the phone the Mac goes away");
            power::wait_for_poll(now_ms());
            std::process::exit(0);
        }
        shared.procs.forget_when_idle();
        let last = shared.last_request_ms.load(Ordering::Relaxed);
        let active = last != 0 && now_ms().saturating_sub(last) <= idle_after.as_millis() as u64;
        if !active {
            // Paused while nobody polls. The sample is dropped, so the next poll gets "starting"
            // (503) instead of stale numbers, and the next sample is measured from fresh counters
            // rather than averaged over the whole pause.
            if prev.take().is_some() {
                *shared.sample.lock().unwrap_or_else(|e| e.into_inner()) = None;
            }
            // Short naps, so the first numbers are there soon after the phone comes back.
            nap(o.interval.min(Duration::from_millis(200)));
            continue;
        }
        let Some((prev_cpu, prev_net, prev_at)) = prev.take() else {
            prev = Some((machine.cpu_ticks(), sampler::net_counters(), Instant::now()));
            nap(Duration::from_millis(250));
            continue;
        };

        let cpu = machine.cpu_ticks();
        let net = sampler::net_counters();
        let at = Instant::now();
        let seconds = at.duration_since(prev_at).as_secs_f64().max(0.001);
        let (rx, tx) = sampler::net_delta(&prev_net, &net);
        let head = head_json(&Load {
            ts: now_ms(),
            interval_ms: o.interval.as_millis(),
            mac: power::state(),
            cpu_name: &cpu_name,
            cpu_pct: sampler::cpu_percent(&prev_cpu, &cpu),
            gpu: gpu.sample(),
            ram_used: machine.ram_used(page_size),
            ram_total,
            down_bps: rx as f64 / seconds,
            up_bps: tx as f64 / seconds,
            batteries: batteries.refresh(),
        });
        // "alerts" is absent when they are switched off and null when no source can be read.
        // Both lengths are kept: which one goes out is the request's choice.
        let (short, full) = if serve_alerts {
            let on_screen = screen.refresh();
            let list = match database.refresh() {
                Some(rows) => Some(alerts::merge(rows, on_screen, now_ms())),
                None if !on_screen.is_empty() => Some(alerts::merge(&[], on_screen, now_ms())),
                None => None,
            };
            (alerts_json(list.as_deref(), alerts::SHORT), alerts_json(list.as_deref(), alerts::FULL))
        } else {
            (String::new(), String::new())
        };
        *shared.sample.lock().unwrap_or_else(|e| e.into_inner()) = Some(http::Sample { head, alerts: short, alerts_full: full });
        prev = Some((cpu, net, at));
        nap(o.interval.saturating_sub(at.elapsed()));
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
            cpu_name: "Apple M4",
            cpu_pct: 12.34,
            gpu: Some((9.0, 1048576)),
            ram_used: 8,
            ram_total: 16,
            down_bps: 1500.4,
            up_bps: 0.0,
            batteries: "[]",
        };
        let item = alerts::Item { id: 7, ts: 5, app: "Mail".into(), title: "a \"b\"".into(), subtitle: String::new(), body: "c\nd".into() };
        let sample = http::Sample { head: head_json(&load), alerts: alerts_json(Some(std::slice::from_ref(&item)), 6), alerts_full: alerts_json(None, 30) };
        assert_eq!(
            sample.json(false, "null"),
            "{\"ts\":1700000000000,\"interval_ms\":1000,\"mac\":\"awake\",\"cpu\":{\"name\":\"Apple M4\",\"pct\":12.3},\"gpu\":{\"pct\":9,\"mem_used\":1048576},\
             \"ram\":{\"used\":8,\"total\":16,\"pct\":50.0},\"net\":{\"down_bps\":1500,\"up_bps\":0},\"batteries\":[],\
             \"alerts\":[{\"id\":7,\"ts\":5,\"app\":\"Mail\",\"title\":\"a \\\"b\\\"\",\"subtitle\":\"\",\"body\":\"c d\"}],\"weather\":null}"
        );
        assert!(sample.json(true, "{}").ends_with("\"batteries\":[],\"alerts\":null,\"weather\":{}}"));
        let none = Load { gpu: None, ram_total: 0, ..load };
        assert!(head_json(&none).contains("\"gpu\":null,\"ram\":{\"used\":8,\"total\":0,\"pct\":0.0}"));
        assert_eq!(alerts_json(Some(&[item.clone(), item]), 1).matches("\"id\"").count(), 1);
    }
}
