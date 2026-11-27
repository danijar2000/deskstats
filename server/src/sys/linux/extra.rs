//! Linux: the disk, the time since start and the CPU temperature.

use std::path::Path;

/// The size of the disk the system is on and what is free on it for an ordinary user, in bytes.
pub fn disk() -> Option<(u64, u64)> {
    let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c"/".as_ptr(), &mut s) } != 0 {
        return None;
    }
    let unit = if s.f_frsize > 0 { s.f_frsize } else { s.f_bsize } as u64;
    Some((s.f_blocks as u64 * unit, s.f_bavail as u64 * unit))
}

/// Bytes read from and written to the whole disks since start; partitions, loops and RAM disks are left out, so that
/// nothing counts twice.
pub fn disk_counters() -> Option<(u64, u64)> {
    let text = std::fs::read_to_string("/proc/diskstats").ok()?;
    Some(parse_diskstats(&text, |name| whole_disk(name) && Path::new(&format!("/sys/block/{name}/device")).exists()))
}

/// The boot areas and the replay block of an eMMC look like disks of their own, but are part of the one disk.
pub fn whole_disk(name: &str) -> bool {
    !(name.starts_with("mmcblk") && (name.contains("boot") || name.contains("rpmb")))
}

pub fn parse_diskstats(text: &str, is_disk: impl Fn(&str) -> bool) -> (u64, u64) {
    let (mut read, mut written) = (0u64, 0u64);
    for line in text.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() < 10 || !is_disk(f[2]) {
            continue;
        }
        // Sectors are 512 bytes here whatever the disk's own sector size.
        read = read.saturating_add(f[5].parse::<u64>().unwrap_or(0).saturating_mul(512));
        written = written.saturating_add(f[9].parse::<u64>().unwrap_or(0).saturating_mul(512));
    }
    (read, written)
}

pub fn uptime() -> Option<u64> {
    let text = std::fs::read_to_string("/proc/uptime").ok()?;
    text.split_whitespace().next()?.parse::<f64>().ok().map(|s| s as u64)
}

/// The CPU temperature from the hardware monitors, found once at start.
pub struct Thermal {
    file: Option<String>,
}

impl Thermal {
    pub fn new() -> Self {
        Thermal { file: find_sensor() }
    }

    pub fn celsius(&mut self) -> Option<f64> {
        let milli: f64 = std::fs::read_to_string(self.file.as_ref()?).ok()?.trim().parse().ok()?;
        let c = milli / 1000.0;
        (1.0..=150.0).contains(&c).then_some(c)
    }
}

/// Monitors of CPUs by their driver's name, best first.
const CPU_MONITORS: [&str; 6] = ["coretemp", "k10temp", "zenpower", "cpu_thermal", "soc_thermal", "cpu-thermal"];

fn find_sensor() -> Option<String> {
    let mut found: Vec<(usize, String)> = Vec::new();
    for entry in std::fs::read_dir("/sys/class/hwmon").ok()?.flatten() {
        let dir = entry.path();
        let name = std::fs::read_to_string(dir.join("name")).unwrap_or_default();
        if let Some(rank) = CPU_MONITORS.iter().position(|m| *m == name.trim()) {
            let file = dir.join("temp1_input");
            if file.exists() {
                found.push((rank, file.to_string_lossy().into_owned()));
            }
        }
    }
    found.sort();
    if let Some((_, file)) = found.into_iter().next() {
        return Some(file);
    }
    // Small boards often have only a thermal zone.
    for entry in std::fs::read_dir("/sys/class/thermal").ok()?.flatten() {
        let kind = std::fs::read_to_string(entry.path().join("type")).unwrap_or_default().to_lowercase();
        if kind.contains("x86_pkg_temp") || kind.contains("cpu") || kind.contains("soc") {
            return Some(entry.path().join("temp").to_string_lossy().into_owned());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whole_disks_are_counted_once() {
        let text = "   8       0 sda 100 0 2000 0 50 0 4000 0 0 0 0\n   8       1 sda1 90 0 1800 0 40 0 3600 0 0 0 0\n   7       0 loop0 5 0 10 0 0 0 0 0 0 0 0\n";
        assert_eq!(parse_diskstats(text, |n| n == "sda"), (2000 * 512, 4000 * 512));
        assert_eq!(parse_diskstats("", |_| true), (0, 0));
        assert!(whole_disk("mmcblk0") && whole_disk("nvme0n1") && !whole_disk("mmcblk0boot0") && !whole_disk("mmcblk0rpmb"));
    }
}
