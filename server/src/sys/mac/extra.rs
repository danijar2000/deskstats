//! macOS: the disk, the time since start and the CPU temperature (from the System Management Controller).

use super::ffi::*;
use std::ffi::{c_char, c_int, c_void};

extern "C" {
    // Intel Macs keep the old statfs under this name; the 64-bit one is what the struct below describes.
    #[cfg_attr(target_arch = "x86_64", link_name = "statfs$INODE64")]
    fn statfs(path: *const c_char, buf: *mut c_void) -> c_int;
}

#[link(name = "IOKit", kind = "framework")]
extern "C" {
    fn IOServiceOpen(service: u32, task: u32, kind: u32, connect: *mut u32) -> c_int;
    fn IOServiceClose(connect: u32) -> c_int;
    fn IOConnectCallStructMethod(connect: u32, selector: u32, input: *const c_void, input_size: usize, output: *mut c_void, output_size: *mut usize) -> c_int;
}

/// The size of the disk of the user's files and what is free on it, in bytes. The system volume is sealed and small, so
/// the data volume is the one that fills up.
pub fn disk() -> Option<(u64, u64)> {
    for path in [c"/System/Volumes/Data", c"/"] {
        // struct statfs: f_bsize (u32) at 0, f_blocks at 8, f_bfree at 16, f_bavail at 24; 2168 bytes in all.
        let mut buf = [0u64; 300];
        if unsafe { statfs(path.as_ptr(), buf.as_mut_ptr() as *mut c_void) } == 0 {
            let block = buf[0] & 0xffff_ffff;
            let (total, free) = (buf[1].saturating_mul(block), buf[3].saturating_mul(block));
            if total > 0 {
                return Some((total, free));
            }
        }
    }
    None
}

/// Bytes read from and written to the disks since start, from the statistics of every block storage driver.
pub fn disk_counters() -> Option<(u64, u64)> {
    let (mut read, mut written, mut any) = (0u64, 0u64, false);
    unsafe {
        let mut iter: u32 = 0;
        if IOServiceGetMatchingServices(0, IOServiceMatching(c"IOBlockStorageDriver".as_ptr()), &mut iter) != 0 {
            return None;
        }
        let (stats, bytes_read, bytes_written) = (cf_string("Statistics"), cf_string("Bytes (Read)"), cf_string("Bytes (Write)"));
        loop {
            let entry = IOIteratorNext(iter);
            if entry == 0 {
                break;
            }
            let dict = IORegistryEntryCreateCFProperty(entry, stats, std::ptr::null(), 0);
            IOObjectRelease(entry);
            if dict.is_null() {
                continue;
            }
            if CFGetTypeID(dict) == CFDictionaryGetTypeID() {
                if let (Some(r), Some(w)) = (cf_to_i64(CFDictionaryGetValue(dict, bytes_read)), cf_to_i64(CFDictionaryGetValue(dict, bytes_written))) {
                    any = true;
                    read = read.saturating_add(r.max(0) as u64);
                    written = written.saturating_add(w.max(0) as u64);
                }
            }
            CFRelease(dict);
        }
        IOObjectRelease(iter);
        for s in [stats, bytes_read, bytes_written] {
            if !s.is_null() {
                CFRelease(s);
            }
        }
    }
    any.then_some((read, written))
}

pub fn uptime() -> Option<u64> {
    // struct timeval: seconds, then microseconds.
    let mut boot = [0i64; 2];
    let mut len = std::mem::size_of_val(&boot);
    let rc = unsafe { sysctlbyname(c"kern.boottime".as_ptr(), boot.as_mut_ptr() as *mut c_void, &mut len, std::ptr::null(), 0) };
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok()?.as_secs() as i64;
    (rc == 0 && boot[0] > 0).then(|| now.saturating_sub(boot[0]).max(0) as u64)
}

/// The keys of CPU temperatures: Apple silicon (M1 to M4) and Intel. Which exist depends on the chip; the found ones
/// are averaged.
const KEYS: [&[u8; 4]; 28] = [
    b"Tp01", b"Tp05", b"Tp09", b"Tp0D", b"Tp0H", b"Tp0L", b"Tp0P", b"Tp0T", b"Tp0X", b"Tp0b", b"Tp0f", b"Tp0j", b"Tp0V", b"Tp0Y",
    b"Tp1h", b"Tp1t", b"Tp1p", b"Tp1l", b"Te05", b"Te09", b"Te0H", b"Te0L", b"Te0P", b"Te0S", b"Tf04", b"TC0P", b"TC0D", b"TC0E",
];

/// struct SMCKeyData_t of the AppleSMC driver: 80 bytes.
#[repr(C)]
#[derive(Clone, Copy)]
struct KeyData {
    key: u32,
    vers: [u8; 6],
    p_limit: [u8; 18],
    data_size: u32,
    data_type: u32,
    attributes: u8,
    pad: [u8; 3],
    result: u8,
    status: u8,
    command: u8,
    pad2: u8,
    data32: u32,
    bytes: [u8; 32],
}

const READ_BYTES: u8 = 5;
const READ_KEY_INFO: u8 = 9;
const KERNEL_INDEX: u32 = 2;

pub struct Thermal {
    connection: u32,
    keys: Option<Vec<(u32, u32, u32)>>,
}

impl Thermal {
    pub fn new() -> Self {
        let mut connection = 0;
        unsafe {
            let mut iter: u32 = 0;
            if IOServiceGetMatchingServices(0, IOServiceMatching(c"AppleSMC".as_ptr()), &mut iter) == 0 {
                let service = IOIteratorNext(iter);
                if service != 0 {
                    if IOServiceOpen(service, mach_task_self_, 0, &mut connection) != 0 {
                        connection = 0;
                    }
                    IOObjectRelease(service);
                }
                IOObjectRelease(iter);
            }
        }
        Thermal { connection, keys: None }
    }

    fn call(&self, input: &KeyData) -> Option<KeyData> {
        let mut output: KeyData = unsafe { std::mem::zeroed() };
        let mut size = std::mem::size_of::<KeyData>();
        let rc = unsafe { IOConnectCallStructMethod(self.connection, KERNEL_INDEX, input as *const _ as *const c_void, size, &mut output as *mut _ as *mut c_void, &mut size) };
        (rc == 0 && output.result == 0).then_some(output)
    }

    fn read(&self, key: u32, size: u32, kind: u32) -> Option<f64> {
        let mut input: KeyData = unsafe { std::mem::zeroed() };
        input.key = key;
        input.data_size = size;
        input.command = READ_BYTES;
        let out = self.call(&input)?;
        let b = out.bytes;
        let value = match &kind.to_be_bytes() {
            b"flt " if size == 4 => f32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64,
            b"sp78" if size == 2 => i16::from_be_bytes([b[0], b[1]]) as f64 / 256.0,
            _ => return None,
        };
        (10.0..=125.0).contains(&value).then_some(value)
    }

    pub fn celsius(&mut self) -> Option<f64> {
        if self.connection == 0 {
            return None;
        }
        if self.keys.is_none() {
            // Which keys this Mac has is asked once.
            let mut found = Vec::new();
            for name in KEYS {
                let key = u32::from_be_bytes(*name);
                let mut input: KeyData = unsafe { std::mem::zeroed() };
                input.key = key;
                input.command = READ_KEY_INFO;
                if let Some(info) = self.call(&input) {
                    if self.read(key, info.data_size, info.data_type).is_some() {
                        found.push((key, info.data_size, info.data_type));
                    }
                }
            }
            self.keys = Some(found);
        }
        let values: Vec<f64> = self.keys.as_ref()?.iter().filter_map(|&(k, s, t)| self.read(k, s, t)).collect();
        (!values.is_empty()).then(|| values.iter().sum::<f64>() / values.len() as f64)
    }
}

impl Drop for Thermal {
    fn drop(&mut self) {
        if self.connection != 0 {
            unsafe { IOServiceClose(self.connection) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_smc_struct_has_the_driver_layout() {
        assert_eq!(std::mem::size_of::<KeyData>(), 80);
        assert!(disk().is_some_and(|(total, free)| total > 0 && free <= total));
        assert!(uptime().is_some_and(|s| s > 0));
        assert!(disk_counters().is_some());
        // A Mac in a virtual machine has no SMC, so the temperature may be missing; it is only shown.
        eprintln!("CPU temperature: {:?}", Thermal::new().celsius());
    }
}
