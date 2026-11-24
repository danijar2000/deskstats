use crate::ffi::*;
use crate::util::json_escape;
use std::collections::HashMap;
use std::ffi::{c_void, CStr, CString};
use std::sync::Mutex;
use std::time::{Duration, Instant};

const ROWS: usize = 16;
const MIN_GAP: Duration = Duration::from_millis(300);
const IDLE: Duration = Duration::from_secs(10);
const TOOL_TIMEOUT: Duration = Duration::from_secs(2);
const NAME_AGE: Duration = Duration::from_secs(5);

#[derive(Default, Clone, Copy)]
struct Counters {
    cpu_cs: u64,
    ram: u64,
    rx: u64,
    tx: u64,
    gpu_ns: u64,
}

struct Snapshot {
    at: Instant,
    pids: HashMap<i32, Counters>,
    network: bool,
}

#[derive(Default, Clone)]
struct Row {
    name: String,
    cpu: f64,
    ram: u64,
    down: f64,
    up: f64,
    gpu: f64,
    count: u32,
}

struct Procs {
    prev: Snapshot,
    names: HashMap<i32, (String, Instant)>,
    rows: Vec<Row>,
}

#[derive(Default)]
pub struct Tracker {
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    procs: Option<Procs>,
    used: Option<Instant>,
    busy: bool,
}

impl Tracker {
    pub fn json(&self, sort: &str) -> String {
        let first = {
            let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            inner.used = Some(Instant::now());
            let due = match &inner.procs {
                _ if inner.busy => None,
                None => Some(true),
                Some(p) => (p.prev.at.elapsed() >= MIN_GAP).then_some(false),
            };
            inner.busy |= due.is_some();
            due
        };
        if let Some(first) = first {
            let before = if first { snapshot() } else { None };
            if first {
                std::thread::sleep(Duration::from_millis(400));
            }
            let now = snapshot();
            let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            inner.busy = false;
            if let Some(now) = now {
                match (&mut inner.procs, before) {
                    (Some(p), _) => p.advance(now),
                    (procs @ None, Some(prev)) => procs.insert(Procs { prev, names: HashMap::new(), rows: Vec::new() }).advance(now),
                    (procs @ None, None) => *procs = Some(Procs { prev: now, names: HashMap::new(), rows: Vec::new() }),
                }
            }
        }

        let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let key = |r: &Row| match sort {
            "ram" => r.ram as f64,
            "net" => r.down + r.up,
            "gpu" => r.gpu,
            _ => r.cpu,
        };
        let mut sorted: Vec<&Row> = inner.procs.iter().flat_map(|p| p.rows.iter()).collect();
        sorted.sort_by(|a, b| key(b).total_cmp(&key(a)).then_with(|| a.name.cmp(&b.name)));
        let list: Vec<String> = sorted
            .iter()
            .take(ROWS)
            .map(|r| {
                format!(
                    "{{\"name\":\"{}\",\"cpu\":{:.1},\"ram\":{},\"down\":{:.0},\"up\":{:.0},\"gpu\":{:.1},\"count\":{}}}",
                    json_escape(&r.name),
                    r.cpu,
                    r.ram,
                    r.down,
                    r.up,
                    r.gpu,
                    r.count
                )
            })
            .collect();
        let sort = if matches!(sort, "ram" | "net" | "gpu") { sort } else { "cpu" };
        format!("{{\"sort\":\"{sort}\",\"total\":{},\"procs\":[{}]}}", sorted.len(), list.join(","))
    }

    pub fn forget_when_idle(&self) {
        let Ok(mut inner) = self.inner.try_lock() else { return };
        if !inner.busy && inner.used.is_some_and(|t| t.elapsed() >= IDLE) {
            *inner = Inner::default();
        }
    }
}

impl Procs {
    fn advance(&mut self, now: Snapshot) {
        self.names.retain(|pid, _| now.pids.contains_key(pid));
        for pid in now.pids.keys() {
            if self.names.get(pid).is_none_or(|(name, at)| name.is_empty() || at.elapsed() >= NAME_AGE) {
                self.names.insert(*pid, (app_name(*pid), Instant::now()));
            }
        }
        self.rows = rows(&self.prev, &now, &self.names);
        self.prev = now;
    }
}

fn rows(prev: &Snapshot, now: &Snapshot, names: &HashMap<i32, (String, Instant)>) -> Vec<Row> {
    let seconds = now.at.duration_since(prev.at).as_secs_f64().max(0.001);
    let mut apps: HashMap<&str, Row> = HashMap::new();
    for (pid, c) in &now.pids {
        let Some(name) = names.get(pid).map(|(n, _)| n).filter(|n| !n.is_empty()) else { continue };
        let row = apps.entry(name.as_str()).or_insert_with(|| Row { name: name.clone(), ..Row::default() });
        row.count += 1;
        row.ram += c.ram;
        if let Some(p) = prev.pids.get(pid) {
            row.cpu += c.cpu_cs.saturating_sub(p.cpu_cs) as f64 / seconds;
            if prev.network && now.network {
                row.down += c.rx.saturating_sub(p.rx) as f64 / seconds;
                row.up += c.tx.saturating_sub(p.tx) as f64 / seconds;
            }
            row.gpu += c.gpu_ns.saturating_sub(p.gpu_ns) as f64 / 1e7 / seconds;
        }
    }
    apps.into_values().collect()
}

fn snapshot() -> Option<Snapshot> {
    let mut pids = HashMap::new();
    if !read_ps(&mut pids) {
        return None;
    }
    read_footprints(&mut pids);
    let network = read_network(&mut pids);
    read_gpu(&mut pids);
    Some(Snapshot { at: Instant::now(), pids, network })
}

fn output(program: &str, args: &[&str]) -> Option<String> {
    crate::util::run(program, args, TOOL_TIMEOUT).map(|(_, text)| text)
}

fn read_ps(pids: &mut HashMap<i32, Counters>) -> bool {
    let Some(text) = output("/bin/ps", &["-axo", "pid=,time=,rss="]) else { return false };
    for line in text.lines() {
        let mut parts = line.split_whitespace();
        let (Some(pid), Some(time), Some(rss)) = (parts.next(), parts.next(), parts.next()) else { continue };
        let (Ok(pid), Some(cpu_cs), Ok(rss)) = (pid.parse::<i32>(), parse_cpu_time(time), rss.parse::<u64>()) else { continue };
        pids.insert(pid, Counters { cpu_cs, ram: rss * 1024, ..Counters::default() });
    }
    !pids.is_empty()
}

pub fn parse_cpu_time(text: &str) -> Option<u64> {
    let (days, rest) = match text.split_once('-') {
        Some((d, r)) => (d.parse::<u64>().ok()?, r),
        None => (0, text),
    };
    let (clock, hundredths) = match rest.split_once('.') {
        Some((c, h)) => (c, h.get(..2).unwrap_or(h).parse::<u64>().ok()?),
        None => (rest, 0),
    };
    let fields: Vec<u64> = clock.split(':').map(|f| f.parse().ok()).collect::<Option<_>>()?;
    let seconds = match fields[..] {
        [m, s] => m * 60 + s,
        [h, m, s] => h * 3600 + m * 60 + s,
        _ => return None,
    };
    Some((days * 86400 + seconds) * 100 + hundredths)
}

fn read_footprints(pids: &mut HashMap<i32, Counters>) {
    for (pid, c) in pids.iter_mut() {
        let mut info = [0u64; 20];
        if unsafe { proc_pid_rusage(*pid, RUSAGE_INFO_V2, info.as_mut_ptr() as *mut c_void) } == 0 && info[RUSAGE_FOOTPRINT] > 0 {
            c.ram = info[RUSAGE_FOOTPRINT];
        }
    }
}

fn read_network(pids: &mut HashMap<i32, Counters>) -> bool {
    let Some(text) = output("/usr/bin/nettop", &["-P", "-L", "1", "-x", "-n", "-J", "bytes_in,bytes_out"]) else { return false };
    for line in text.lines().skip(1) {
        let Some((pid, rx, tx)) = parse_nettop(line) else { continue };
        if let Some(c) = pids.get_mut(&pid) {
            c.rx = rx;
            c.tx = tx;
        }
    }
    true
}

pub fn parse_nettop(line: &str) -> Option<(i32, u64, u64)> {
    let mut fields = line.split(',');
    let pid = fields.next()?.rsplit_once('.')?.1.parse().ok()?;
    Some((pid, fields.next()?.parse().ok()?, fields.next()?.parse().ok()?))
}

struct GpuKeys {
    creator: *const c_void,
    usage: *const c_void,
    time: *const c_void,
}

impl Drop for GpuKeys {
    fn drop(&mut self) {
        for key in [self.creator, self.usage, self.time] {
            if !key.is_null() {
                unsafe { CFRelease(key) };
            }
        }
    }
}

fn read_gpu(pids: &mut HashMap<i32, Counters>) {
    let (Ok(class), Ok(plane)) = (CString::new("IOAccelerator"), CString::new("IOService")) else { return };
    let keys = GpuKeys { creator: cf_string("IOUserClientCreator"), usage: cf_string("AppUsage"), time: cf_string("accumulatedGPUTime") };
    let keys = &keys;
    unsafe {
        let mut accelerators: u32 = 0;
        if IOServiceGetMatchingServices(0, IOServiceMatching(class.as_ptr()), &mut accelerators) != 0 {
            return;
        }
        loop {
            let accelerator = IOIteratorNext(accelerators);
            if accelerator == 0 {
                break;
            }
            let mut clients: u32 = 0;
            if IORegistryEntryGetChildIterator(accelerator, plane.as_ptr(), &mut clients) == 0 {
                loop {
                    let client = IOIteratorNext(clients);
                    if client == 0 {
                        break;
                    }
                    if let Some((pid, ns)) = gpu_client(keys, client) {
                        if let Some(c) = pids.get_mut(&pid) {
                            c.gpu_ns += ns;
                        }
                    }
                    IOObjectRelease(client);
                }
                IOObjectRelease(clients);
            }
            IOObjectRelease(accelerator);
        }
        IOObjectRelease(accelerators);
    }
}

unsafe fn gpu_client(keys: &GpuKeys, client: u32) -> Option<(i32, u64)> {
    let usage = IORegistryEntryCreateCFProperty(client, keys.usage, std::ptr::null(), 0);
    if usage.is_null() {
        return None;
    }
    let mut ns = 0u64;
    if CFGetTypeID(usage) == CFArrayGetTypeID() {
        for i in 0..CFArrayGetCount(usage) {
            let entry = CFArrayGetValueAtIndex(usage, i);
            if !entry.is_null() && CFGetTypeID(entry) == CFDictionaryGetTypeID() {
                ns += cf_to_i64(CFDictionaryGetValue(entry, keys.time)).unwrap_or(0).max(0) as u64;
            }
        }
    }
    CFRelease(usage);
    if ns == 0 {
        return None;
    }
    let creator = IORegistryEntryCreateCFProperty(client, keys.creator, std::ptr::null(), 0);
    if creator.is_null() {
        return None;
    }
    let text = cf_to_string(creator);
    CFRelease(creator);
    Some((parse_creator(&text?)?, ns))
}

pub fn parse_creator(text: &str) -> Option<i32> {
    text.strip_prefix("pid ")?.split(',').next()?.trim().parse().ok()
}

fn app_name(pid: i32) -> String {
    let mut buf = [0u8; 4096];
    let len = unsafe { proc_pidpath(pid, buf.as_mut_ptr() as *mut c_void, buf.len() as u32) };
    if len > 0 {
        return name_from_path(&String::from_utf8_lossy(&buf[..len as usize]));
    }
    let len = unsafe { proc_name(pid, buf.as_mut_ptr() as *mut c_void, 256) };
    if len > 0 {
        return CStr::from_bytes_until_nul(&buf).map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    }
    if pid == 0 {
        String::from("kernel_task")
    } else {
        String::new()
    }
}

pub fn name_from_path(path: &str) -> String {
    if let Some(end) = path.find(".app/") {
        let bundle = &path[..end];
        return bundle.rsplit('/').next().unwrap_or(bundle).to_string();
    }
    path.rsplit('/').next().unwrap_or(path).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_time_formats() {
        assert_eq!(parse_cpu_time("0:00.03"), Some(3));
        assert_eq!(parse_cpu_time("535:45.35"), Some((535 * 60 + 45) * 100 + 35));
        assert_eq!(parse_cpu_time("1:02:03.04"), Some(372_304));
        assert_eq!(parse_cpu_time("2-01:00:00"), Some((2 * 86400 + 3600) * 100));
        assert_eq!(parse_cpu_time("abc"), None);
    }

    #[test]
    fn nettop_lines() {
        assert_eq!(parse_nettop("apsd.386,39483,59679,"), Some((386, 39483, 59679)));
        assert_eq!(parse_nettop("com.apple.WebKit.12,1,2,"), Some((12, 1, 2)));
        assert_eq!(parse_nettop(",bytes_in,bytes_out,"), None);
    }

    #[test]
    fn names_and_creators() {
        assert_eq!(
            name_from_path("/Applications/Google Chrome.app/Contents/Frameworks/X.framework/Helpers/Google Chrome Helper.app/Contents/MacOS/Google Chrome Helper"),
            "Google Chrome"
        );
        assert_eq!(name_from_path("/usr/libexec/trustd"), "trustd");
        assert_eq!(parse_creator("pid 415, WindowServer"), Some(415));
        assert_eq!(parse_creator("WindowServer"), None);
    }

    #[test]
    fn rows_group_by_application_and_skip_new_processes() {
        let at = Instant::now();
        let c = |cpu_cs, ram, rx| Counters { cpu_cs, ram, rx, tx: 0, gpu_ns: 0 };
        let prev = Snapshot { at, network: true, pids: HashMap::from([(1, c(100, 10, 1000)), (2, c(50, 10, 500))]) };
        let mut now = Snapshot { at: at + Duration::from_secs(2), network: true, pids: HashMap::from([(1, c(140, 20, 3000)), (2, c(60, 30, 100)), (3, c(900, 5, 0))]) };
        let names = HashMap::from([(1, ("A".to_string(), at)), (2, ("A".to_string(), at)), (3, ("B".to_string(), at)), (4, (String::new(), at))]);
        let mut out = rows(&prev, &now, &names);
        out.sort_by(|a, b| a.name.cmp(&b.name));
        assert_eq!((out[0].count, out[0].ram), (2, 50));
        assert!((out[0].cpu - 25.0).abs() < 1e-9);
        assert!((out[0].down - 1000.0).abs() < 1e-9);
        assert_eq!(out[1].cpu, 0.0);
        now.network = false;
        assert!(rows(&prev, &now, &names).iter().all(|r| r.down == 0.0));
    }

    #[test]
    fn tracker_answers_and_forgets() {
        let tracker = Tracker::default();
        let json = tracker.json("ram");
        assert!(json.starts_with("{\"sort\":\"ram\",\"total\":"), "{json}");
        assert!(json.contains("\"name\":"));
        assert!(tracker.json("nonsense").starts_with("{\"sort\":\"cpu\""));
        tracker.inner.lock().unwrap().used = Some(Instant::now() - IDLE);
        tracker.forget_when_idle();
        assert!(tracker.inner.lock().unwrap().procs.is_none());
    }
}
