//! The processes of macOS: ps for CPU time, the kernel for memory, nettop for network, the GPU drivers for GPU time.

use super::ffi::*;
use crate::procs::Counters;
use std::collections::HashMap;
use std::ffi::{c_void, CStr, CString};
use std::time::Duration;

const TOOL_TIMEOUT: Duration = Duration::from_secs(2);

/// Fills in every process; None when the list cannot be read, otherwise whether the network counters are there.
pub fn read(pids: &mut HashMap<i32, Counters>) -> Option<bool> {
    if !read_ps(pids) {
        return None;
    }
    read_footprints(pids);
    let network = read_network(pids);
    read_gpu(pids);
    Some(network)
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

pub fn app_name(pid: i32) -> String {
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
}
