use super::ffi::*;
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

/// The name the Mac shows in System Settings → General → Sharing; the host name if there is none.
pub fn computer_name() -> String {
    let name = unsafe {
        let s = SCDynamicStoreCopyComputerName(std::ptr::null(), std::ptr::null_mut());
        let out = cf_to_string(s);
        if !s.is_null() {
            CFRelease(s);
        }
        out
    };
    name.map(|n| n.trim().to_string()).filter(|n| !n.is_empty()).unwrap_or_else(|| {
        let host = sysctl_string("kern.hostname");
        host.strip_suffix(".local").unwrap_or(&host).to_string()
    })
}

/// What kind of Mac this is, such as "MacBook Air (M2, 2022)" or "Mac mini (2024)"; the model code if the system does not say.
pub fn model_name() -> String {
    let name = unsafe {
        let entry = IORegistryEntryFromPath(0, c"IODeviceTree:/product".as_ptr());
        if entry == 0 {
            None
        } else {
            let key = cf_string("product-name");
            let value = IORegistryEntryCreateCFProperty(entry, key, std::ptr::null(), 0);
            let out = cf_data_text(value);
            for object in [key, value] {
                if !object.is_null() {
                    CFRelease(object);
                }
            }
            IOObjectRelease(entry);
            out
        }
    };
    name.filter(|n| !n.is_empty()).unwrap_or_else(|| sysctl_string("hw.model"))
}

unsafe fn cf_data_text(value: *const c_void) -> Option<String> {
    if value.is_null() || CFGetTypeID(value) != CFDataGetTypeID() {
        return None;
    }
    let bytes = std::slice::from_raw_parts(CFDataGetBytePtr(value), CFDataGetLength(value).max(0) as usize);
    let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
    Some(String::from_utf8_lossy(&bytes[..end]).trim().to_string())
}

pub struct Host(u32);

impl Host {
    pub fn new() -> Self {
        Host(unsafe { mach_host_self() })
    }

    pub fn cpu_name(&self) -> String {
        sysctl_string("machdep.cpu.brand_string")
    }

    pub fn ram_total(&self) -> u64 {
        sysctl_u64("hw.memsize")
    }

    pub fn cpu_ticks(&self) -> Vec<(u64, u64)> {
        let mut cores = Vec::new();
        unsafe {
            let mut count: u32 = 0;
            let mut info: *mut i32 = std::ptr::null_mut();
            let mut info_count: u32 = 0;
            if host_processor_info(self.0, PROCESSOR_CPU_LOAD_INFO, &mut count, &mut info, &mut info_count) != 0 || info.is_null() {
                return cores;
            }
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

    pub fn ram_used(&self) -> u64 {
        let page_size = sysctl_u64("hw.pagesize").max(4096);
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

const CTL_NET: i32 = 4;
const PF_ROUTE: i32 = 17;
const NET_RT_IFLIST2: i32 = 6;
const RTM_IFINFO2: u8 = 0x12;
const IFM_INDEX: usize = 12;
const IFM_IBYTES: usize = 96;
const IFM_OBYTES: usize = 104;

pub fn net_counters() -> HashMap<String, (u64, u64)> {
    let mut out = HashMap::new();
    let mut mib = [CTL_NET, PF_ROUTE, 0, 0, NET_RT_IFLIST2, 0];
    let mut len = 0usize;
    unsafe {
        if sysctl(mib.as_mut_ptr(), 6, std::ptr::null_mut(), &mut len, std::ptr::null(), 0) != 0 {
            return out;
        }
        let mut buf = vec![0u8; len + 4096];
        len = buf.len();
        if sysctl(mib.as_mut_ptr(), 6, buf.as_mut_ptr() as *mut c_void, &mut len, std::ptr::null(), 0) != 0 {
            return out;
        }
        let buf = &buf[..len.min(buf.len())];
        let mut at = 0;
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
    fn net_counters_read_the_interfaces() {
        let first = net_counters();
        assert!(first.contains_key("en0"), "{first:?}");
        let second = net_counters();
        assert!(second["en0"].0 >= first["en0"].0);
    }
}
