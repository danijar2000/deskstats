mod adb;
mod alerts;
mod crypto;
mod ffi;
mod http;
mod lan;
mod pair;
mod power;
mod procs;
mod sampler;
mod settings;
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

const PORT: u16 = 8787;

type Counters = (Vec<(u64, u64)>, std::collections::HashMap<String, (u64, u64)>, Instant);

fn data_dir() -> String {
    format!("{}/Library/Application Support/deskstats", std::env::var("HOME").unwrap_or_default())
}

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
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
    let mut random = [0u8; 24];
    fs::File::open("/dev/urandom")?.read_exact(&mut random)?;
    let token: String = random.iter().map(|b| format!("{b:02x}")).collect();
    let mut file = fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&path)?;
    std::io::Write::write_all(&mut file, token.as_bytes())?;
    log("token: created a new one; the phone gets it the next time it is connected over USB");
    Ok(token)
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
    batteries: &'a str,
}

fn head_json(l: &Load) -> String {
    let gpu = match l.gpu {
        Some((pct, mem)) => format!("{{\"pct\":{pct:.0},\"mem_used\":{mem}}}"),
        None => String::from("null"),
    };
    let ram_pct = if l.ram_total > 0 { l.ram_used as f64 * 100.0 / l.ram_total as f64 } else { 0.0 };
    format!(
        "{{\"ts\":{},\"interval_ms\":{},\"mac\":\"{}\",\"name\":\"{}\",\"cpu\":{{\"name\":\"{}\",\"pct\":{:.1}}},\"gpu\":{gpu},\"ram\":{{\"used\":{},\"total\":{},\"pct\":{ram_pct:.1}}},\"net\":{{\"down_bps\":{:.0},\"up_bps\":{:.0}}},\"batteries\":{},",
        l.ts, l.interval_ms, l.mac, l.name, l.cpu_name, l.cpu_pct, l.ram_used, l.ram_total, l.down_bps, l.up_bps, l.batteries,
    )
}

fn alerts_json(list: Option<&[alerts::Item]>, count: usize) -> String {
    match list {
        Some(items) => format!("\"alerts\":[{}],", items.iter().take(count).map(alerts::Item::json).collect::<Vec<_>>().join(",")),
        None => String::from("\"alerts\":null,"),
    }
}

fn main() {
    if std::env::args().nth(1).as_deref() == Some("--ax-check") {
        println!("{}", if alerts::accessibility_granted() { "yes" } else { "no" });
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
    let computer = sampler::computer_name();
    let about = lan::about_json(&crypto::id_of(&token), &computer, &sampler::model_name(), PORT);
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
            let shared = shared.clone();
            spawn("adb", move || adb::run(path, data_dir(), shared));
        }
        None => log("adb: not found, the phone has to be connected by hand"),
    }

    let machine = sampler::Host::new();
    let cpu_name = json_escape(&sampler::sysctl_string("machdep.cpu.brand_string"));
    let name = json_escape(&computer);
    let ram_total = sampler::sysctl_u64("hw.memsize");
    let page_size = sampler::sysctl_u64("hw.pagesize").max(4096);
    let gpu = sampler::Gpu::new();
    let mut batteries = sampler::Batteries::new();
    let mut database = alerts::Database::new();
    let mut screen = alerts::Screen::new();

    let mut prev: Option<Counters> = None;
    loop {
        if power::stopping() {
            log("stopping: telling the phone the Mac goes away");
            power::wait_for_poll(now_ms());
            std::process::exit(0);
        }
        shared.procs.forget_when_idle();
        let interval = settings.interval();
        let last = shared.last_request_ms.load(Ordering::Relaxed);
        let active = last != 0 && now_ms().saturating_sub(last) <= settings.idle_after().as_millis() as u64;
        if !active {
            // The sample is dropped so that the next poll gets 503, not numbers from before the pause.
            if prev.take().is_some() {
                *shared.sample.lock().unwrap_or_else(|e| e.into_inner()) = None;
            }
            // Asleep: nothing is sampled until a phone asks again. The second-long timeout only checks for a stop.
            shared.wait_for_request(Duration::from_secs(1));
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
            interval_ms: interval.as_millis(),
            mac: power::state(),
            name: &name,
            cpu_name: &cpu_name,
            cpu_pct: sampler::cpu_percent(&prev_cpu, &cpu),
            gpu: gpu.sample(),
            ram_used: machine.ram_used(page_size),
            ram_total,
            down_bps: rx as f64 / seconds,
            up_bps: tx as f64 / seconds,
            batteries: batteries.refresh(),
        });
        let (short, full) = if settings.alerts.load(Ordering::Relaxed) {
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
            batteries: "[]",
        };
        let item = alerts::Item { id: 7, ts: 5, app: "Mail".into(), title: "a \"b\"".into(), subtitle: String::new(), body: "c\nd".into() };
        let sample = http::Sample { head: head_json(&load), alerts: alerts_json(Some(std::slice::from_ref(&item)), 6), alerts_full: alerts_json(None, 30) };
        assert_eq!(
            sample.json(false, "null"),
            "{\"ts\":1700000000000,\"interval_ms\":1000,\"mac\":\"awake\",\"name\":\"Studio\",\"cpu\":{\"name\":\"Apple M4\",\"pct\":12.3},\"gpu\":{\"pct\":9,\"mem_used\":1048576},\
             \"ram\":{\"used\":8,\"total\":16,\"pct\":50.0},\"net\":{\"down_bps\":1500,\"up_bps\":0},\"batteries\":[],\
             \"alerts\":[{\"id\":7,\"ts\":5,\"app\":\"Mail\",\"title\":\"a \\\"b\\\"\",\"subtitle\":\"\",\"body\":\"c d\"}],\"weather\":null}"
        );
        assert!(sample.json(true, "{}").ends_with("\"batteries\":[],\"alerts\":null,\"weather\":{}}"));
        let none = Load { gpu: None, ram_total: 0, ..load };
        assert!(head_json(&none).contains("\"gpu\":null,\"ram\":{\"used\":8,\"total\":0,\"pct\":0.0}"));
        assert_eq!(alerts_json(Some(&[item.clone(), item]), 1).matches("\"id\"").count(), 1);
    }
}
