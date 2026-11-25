use crate::sys::procs::{app_name, read};
use crate::util::json_escape;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const ROWS: usize = 16;
const MIN_GAP: Duration = Duration::from_millis(300);
const IDLE: Duration = Duration::from_secs(10);
const NAME_AGE: Duration = Duration::from_secs(5);

/// What a process has used so far: CPU time in hundredths of a second, memory in bytes, network bytes, GPU time.
#[derive(Default, Clone, Copy)]
pub struct Counters {
    pub cpu_cs: u64,
    pub ram: u64,
    pub rx: u64,
    pub tx: u64,
    pub gpu_ns: u64,
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
    let network = read(&mut pids)?;
    Some(Snapshot { at: Instant::now(), pids, network })
}

#[cfg(test)]
mod tests {
    use super::*;

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
