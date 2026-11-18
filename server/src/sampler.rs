//! Load of this Mac: CPU, memory, network, GPU and the battery of wireless peripherals.

use crate::ffi::*;
use crate::util::json_escape;
use std::collections::HashMap;
use std::ffi::{c_void, CStr, CString};
use std::time::{Duration, Instant};

pub fn sysctl_u64(name: &str) -> u64 {
    let Ok(name) = CString::new(name) else { return 0 };
    let mut value: u64 = 0;
    let mut len = std::mem::size_of::<u64>();
    unsafe { sysctlbyname(name.as_ptr(), &mut value as *mut u64 as *mut c_void, &mut len, std::ptr::null(), 0) };
    value
}

pub fn sysctl_string(name: &str) -> String {
    let Ok(name) = CString::new(name) else { return String::new() };
    let mut buf = [0u8; 128];
    let mut len = buf.len() - 1;
    let rc = unsafe { sysctlbyname(name.as_ptr(), buf.as_mut_ptr() as *mut c_void, &mut len, std::ptr::null(), 0) };
    if rc != 0 {
        return String::new();
    }
    CStr::from_bytes_until_nul(&buf).map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

/// The host port, taken once: every mach_host_self() call adds a reference to it.
pub struct Host(u32);

impl Host {
    pub fn new() -> Self {
        Host(unsafe { mach_host_self() })
    }

    /// Per-core tick counters: (busy, total).
    pub fn cpu_ticks(&self) -> Vec<(u64, u64)> {
        let mut cores = Vec::new();
        unsafe {
            let mut count: u32 = 0;
            let mut info: *mut i32 = std::ptr::null_mut();
            let mut info_count: u32 = 0;
            if host_processor_info(self.0, PROCESSOR_CPU_LOAD_INFO, &mut count, &mut info, &mut info_count) != 0 || info.is_null() {
                return cores;
            }
            // user, system, idle, nice per core
            let usable = (count as usize).min(info_count as usize / 4);
            for core in 0..usable {
                let t = std::slice::from_raw_parts(info.add(core * 4), 4);
                let (user, system, idle, nice) = (t[0] as u32 as u64, t[1] as u32 as u64, t[2] as u32 as u64, t[3] as u32 as u64);
                cores.push((user + system + nice, user + system + nice + idle));
            }
            vm_deallocate(mach_task_self_, info as usize, info_count as usize * 4);
        }
        cores
    }

    /// Memory in use the way Activity Monitor counts it: app memory + wired + compressed.
    pub fn ram_used(&self, page_size: u64) -> u64 {
        let mut vm = VmStatistics64::default();
        let mut count = HOST_VM_INFO64_COUNT;
        let rc = unsafe { host_statistics64(self.0, HOST_VM_INFO64, &mut vm as *mut VmStatistics64 as *mut i32, &mut count) };
        if rc != 0 {
            return 0;
        }
        let app = (vm.internal_page_count as u64).saturating_sub(vm.purgeable_count as u64);
        (app + vm.wire_count as u64 + vm.compressor_page_count as u64) * page_size
    }
}

/// CPU load in percent between two tick snapshots.
pub fn cpu_percent(prev: &[(u64, u64)], cur: &[(u64, u64)]) -> f64 {
    let (mut busy, mut total) = (0u64, 0u64);
    for (i, (b, t)) in cur.iter().enumerate() {
        let Some((pb, pt)) = prev.get(i) else { continue };
        // The kernel counters are 32-bit and wrap.
        busy += b.wrapping_sub(*pb) & 0xffff_ffff;
        total += t.wrapping_sub(*pt) & 0xffff_ffff;
    }
    if total > 0 {
        busy as f64 * 100.0 / total as f64
    } else {
        0.0
    }
}

const CTL_NET: i32 = 4;
const PF_ROUTE: i32 = 17;
const NET_RT_IFLIST2: i32 = 6;
const RTM_IFINFO2: u8 = 0x12;
/// Offsets inside `struct if_msghdr2`: the interface index, and the 64-bit byte counters of
/// its `if_data64`.
const IFM_INDEX: usize = 12;
const IFM_IBYTES: usize = 96;
const IFM_OBYTES: usize = 104;

/// Received and sent byte counters of each physical interface (en*), by name. They come from
/// the routing sysctl: the counters getifaddrs gives are 32-bit and wrap every 4 GB.
pub fn net_counters() -> HashMap<String, (u64, u64)> {
    let mut out = HashMap::new();
    let mut mib = [CTL_NET, PF_ROUTE, 0, 0, NET_RT_IFLIST2, 0];
    let mut len = 0usize;
    unsafe {
        if sysctl(mib.as_mut_ptr(), 6, std::ptr::null_mut(), &mut len, std::ptr::null(), 0) != 0 {
            return out;
        }
        // Interfaces may appear between the two calls; leave some room.
        let mut buf = vec![0u8; len + 4096];
        len = buf.len();
        if sysctl(mib.as_mut_ptr(), 6, buf.as_mut_ptr() as *mut c_void, &mut len, std::ptr::null(), 0) != 0 {
            return out;
        }
        let buf = &buf[..len.min(buf.len())];
        let mut at = 0;
        // Each message starts with its length (u16), a version and a type.
        while at + 4 <= buf.len() {
            let size = u16::from_ne_bytes([buf[at], buf[at + 1]]) as usize;
            if size == 0 || at + size > buf.len() {
                break;
            }
            let message = &buf[at..at + size];
            at += size;
            if message[3] != RTM_IFINFO2 || message.len() < IFM_OBYTES + 8 {
                continue;
            }
            let index = u16::from_ne_bytes([message[IFM_INDEX], message[IFM_INDEX + 1]]) as u32;
            let mut name = [0 as std::ffi::c_char; 32];
            if if_indextoname(index, name.as_mut_ptr()).is_null() {
                continue;
            }
            let name = CStr::from_ptr(name.as_ptr()).to_bytes();
            if !name.starts_with(b"en") {
                continue;
            }
            let counter = |offset: usize| u64::from_ne_bytes(message[offset..offset + 8].try_into().unwrap_or([0; 8]));
            out.insert(String::from_utf8_lossy(name).into_owned(), (counter(IFM_IBYTES), counter(IFM_OBYTES)));
        }
    }
    out
}

/// Bytes received and sent between two snapshots. Only interfaces present in both count, so one
/// appearing or vanishing does not show up as traffic; a counter that went back (a reset) adds nothing.
pub fn net_delta(prev: &HashMap<String, (u64, u64)>, cur: &HashMap<String, (u64, u64)>) -> (u64, u64) {
    let (mut rx, mut tx) = (0u64, 0u64);
    for (name, (r, t)) in cur {
        if let Some((pr, pt)) = prev.get(name) {
            rx += r.saturating_sub(*pr);
            tx += t.saturating_sub(*pt);
        }
    }
    (rx, tx)
}

pub struct Gpu {
    stats: *const c_void,
    utilization: *const c_void,
    memory: *const c_void,
}

impl Gpu {
    pub fn new() -> Self {
        Gpu {
            stats: cf_string("PerformanceStatistics"),
            utilization: cf_string("Device Utilization %"),
            memory: cf_string("In use system memory"),
        }
    }

    /// GPU load in percent and GPU memory in use, read from the accelerator's registry entry.
    /// Apple does not document these keys; None when they are missing.
    pub fn sample(&self) -> Option<(f64, u64)> {
        let mut best: Option<(f64, u64)> = None;
        unsafe {
            let class = CString::new("IOAccelerator").ok()?;
            let mut iter: u32 = 0;
            if IOServiceGetMatchingServices(0, IOServiceMatching(class.as_ptr()), &mut iter) != 0 {
                return None;
            }
            loop {
                let entry = IOIteratorNext(iter);
                if entry == 0 {
                    break;
                }
                let dict = IORegistryEntryCreateCFProperty(entry, self.stats, std::ptr::null(), 0);
                IOObjectRelease(entry);
                if dict.is_null() {
                    continue;
                }
                if CFGetTypeID(dict) == CFDictionaryGetTypeID() {
                    if let Some(pct) = cf_to_i64(CFDictionaryGetValue(dict, self.utilization)) {
                        let mem = cf_to_i64(CFDictionaryGetValue(dict, self.memory)).unwrap_or(0).max(0) as u64;
                        let sample = (pct.clamp(0, 100) as f64, mem);
                        if best.is_none_or(|b| sample.0 > b.0) {
                            best = Some(sample);
                        }
                    }
                }
                CFRelease(dict);
            }
            IOObjectRelease(iter);
        }
        best
    }
}

pub struct Batteries {
    key_percent: *const c_void,
    key_product: *const c_void,
    key_category: *const c_void,
    json: String,
    read_at: Option<Instant>,
}

impl Batteries {
    pub fn new() -> Self {
        Batteries {
            key_percent: cf_string("BatteryPercent"),
            key_product: cf_string("Product"),
            key_category: cf_string("Accessory Category"),
            json: String::from("[]"),
            read_at: None,
        }
    }

    /// Battery levels of the wireless mouse, keyboard and trackpad as a JSON array, read at most
    /// once a minute. Only devices that report a level to macOS show up (Apple's own do).
    pub fn refresh(&mut self) -> &str {
        if self.read_at.is_none_or(|t| t.elapsed() >= Duration::from_secs(60)) {
            self.read_at = Some(Instant::now());
            self.json = format!("[{}]", self.read().join(","));
        }
        &self.json
    }

    fn read(&self) -> Vec<String> {
        let mut items = Vec::new();
        let (Ok(class), Ok(plane)) = (CString::new("AppleDeviceManagementHIDEventService"), CString::new("IOService")) else {
            return items;
        };
        unsafe {
            let mut iter: u32 = 0;
            if IOServiceGetMatchingServices(0, IOServiceMatching(class.as_ptr()), &mut iter) != 0 {
                return items;
            }
            loop {
                let entry = IOIteratorNext(iter);
                if entry == 0 {
                    break;
                }
                let percent = IORegistryEntryCreateCFProperty(entry, self.key_percent, std::ptr::null(), 0);
                if !percent.is_null() {
                    let pct = cf_to_i64(percent).unwrap_or(-1);
                    CFRelease(percent);
                    let text = |key: *const c_void, search: bool| -> String {
                        let value = if search {
                            IORegistryEntrySearchCFProperty(entry, plane.as_ptr(), key, std::ptr::null(), REGISTRY_SEARCH_PARENTS)
                        } else {
                            IORegistryEntryCreateCFProperty(entry, key, std::ptr::null(), 0)
                        };
                        if value.is_null() {
                            return String::new();
                        }
                        let out = cf_to_string(value).unwrap_or_default();
                        CFRelease(value);
                        out
                    };
                    if (0..=100).contains(&pct) {
                        let kind = text(self.key_category, true).to_lowercase();
                        let name = text(self.key_product, false);
                        items.push(format!(
                            "{{\"kind\":\"{}\",\"name\":\"{}\",\"pct\":{pct}}}",
                            json_escape(&kind),
                            json_escape(name.trim())
                        ));
                    }
                }
                IOObjectRelease(entry);
            }
            IOObjectRelease(iter);
        }
        items
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_percent_handles_wrap_and_missing_cores() {
        let prev = vec![(u32::MAX as u64 - 10, u32::MAX as u64 - 20)];
        let cur = vec![(30, 80), (5, 10)];
        // busy +41, total +101 on core 0; core 1 has no previous sample and is skipped
        let pct = cpu_percent(&prev, &cur);
        assert!((pct - 41.0 * 100.0 / 101.0).abs() < 1e-9);
        assert_eq!(cpu_percent(&[], &cur), 0.0);
    }

    #[test]
    fn net_delta_ignores_interfaces_that_come_and_go() {
        let prev = HashMap::from([("en0".to_string(), (1000u64, 500u64)), ("en5".to_string(), (9_000_000, 9_000_000))]);
        let cur = HashMap::from([("en0".to_string(), (1500u64, 700u64)), ("en7".to_string(), (4_000_000, 4_000_000))]);
        assert_eq!(net_delta(&prev, &cur), (500, 200));
    }

    #[test]
    fn net_delta_counts_past_four_gigabytes_and_ignores_a_reset() {
        let prev = HashMap::from([("en0".to_string(), (u32::MAX as u64 - 99, 5000u64))]);
        let cur = HashMap::from([("en0".to_string(), (u32::MAX as u64 + 2_500_000_000, 10u64))]);
        // rx grew by 2.5 GB across the 32-bit boundary; tx went back (interface reset) and counts as nothing
        assert_eq!(net_delta(&prev, &cur), (2_500_000_099, 0));
    }

    #[test]
    fn net_counters_read_the_interfaces() {
        // Every Mac has en0; its counters only grow.
        let first = net_counters();
        assert!(first.contains_key("en0"), "{first:?}");
        let second = net_counters();
        assert!(second["en0"].0 >= first["en0"].0);
    }
}
