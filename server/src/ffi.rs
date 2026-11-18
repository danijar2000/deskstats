//! Bindings to the macOS system libraries the server reads from, and CoreFoundation helpers.

use std::ffi::{c_char, c_int, c_void, CStr, CString};

#[repr(C)]
pub struct IfAddrs {
    pub ifa_next: *mut IfAddrs,
    pub ifa_name: *const c_char,
    pub ifa_flags: u32,
    pub ifa_addr: *const u8,
    pub ifa_netmask: *const u8,
    pub ifa_dstaddr: *const u8,
    pub ifa_data: *const u8,
}

#[repr(C)]
#[derive(Default)]
pub struct VmStatistics64 {
    pub free_count: u32,
    pub active_count: u32,
    pub inactive_count: u32,
    pub wire_count: u32,
    pub zero_fill_count: u64,
    pub reactivations: u64,
    pub pageins: u64,
    pub pageouts: u64,
    pub faults: u64,
    pub cow_faults: u64,
    pub lookups: u64,
    pub hits: u64,
    pub purges: u64,
    pub purgeable_count: u32,
    pub speculative_count: u32,
    pub decompressions: u64,
    pub compressions: u64,
    pub swapins: u64,
    pub swapouts: u64,
    pub compressor_page_count: u32,
    pub throttled_count: u32,
    pub external_page_count: u32,
    pub internal_page_count: u32,
    pub total_uncompressed_pages_in_compressor: u64,
}

#[repr(C)]
pub struct Tm {
    pub tm_sec: c_int,
    pub tm_min: c_int,
    pub tm_hour: c_int,
    pub tm_mday: c_int,
    pub tm_mon: c_int,
    pub tm_year: c_int,
    pub tm_wday: c_int,
    pub tm_yday: c_int,
    pub tm_isdst: c_int,
    pub tm_gmtoff: i64,
    pub tm_zone: *const c_char,
}

pub const HOST_VM_INFO64: c_int = 4;
pub const HOST_VM_INFO64_COUNT: u32 = (std::mem::size_of::<VmStatistics64>() / 4) as u32;
pub const PROCESSOR_CPU_LOAD_INFO: c_int = 2;
pub const CF_UTF8: u32 = 0x0800_0100;
pub const CF_NUMBER_SINT64: c_int = 4;
/// kIORegistryIterateRecursively | kIORegistryIterateParents: look the property up on the ancestors too.
pub const REGISTRY_SEARCH_PARENTS: u32 = 3;
pub const SQLITE_OPEN_READONLY: c_int = 1;
pub const SQLITE_ROW: c_int = 100;
pub const SQLITE_DONE: c_int = 101;
pub const AX_SUCCESS: i32 = 0;
/// rusage_info_v2 is 20 64-bit words; ri_phys_footprint is the tenth.
pub const RUSAGE_INFO_V2: c_int = 2;
pub const RUSAGE_FOOTPRINT: usize = 9;

extern "C" {
    pub static mach_task_self_: u32;
    pub fn mach_host_self() -> u32;
    pub fn host_statistics64(host: u32, flavor: c_int, info: *mut i32, count: *mut u32) -> c_int;
    pub fn host_processor_info(
        host: u32,
        flavor: c_int,
        cpu_count: *mut u32,
        info: *mut *mut i32,
        info_count: *mut u32,
    ) -> c_int;
    pub fn vm_deallocate(task: u32, address: usize, size: usize) -> c_int;
    pub fn sysctlbyname(
        name: *const c_char,
        oldp: *mut c_void,
        oldlenp: *mut usize,
        newp: *const c_void,
        newlen: usize,
    ) -> c_int;
    pub fn sysctl(name: *mut c_int, name_len: u32, old: *mut c_void, old_len: *mut usize, new: *const c_void, new_len: usize) -> c_int;
    pub fn if_indextoname(index: u32, name: *mut c_char) -> *mut c_char;
    pub fn getifaddrs(out: *mut *mut IfAddrs) -> c_int;
    pub fn freeifaddrs(list: *mut IfAddrs);
    pub fn proc_listallpids(buffer: *mut c_void, size: c_int) -> c_int;
    pub fn proc_name(pid: c_int, buffer: *mut c_void, size: u32) -> c_int;
    pub fn proc_pidpath(pid: c_int, buffer: *mut c_void, size: u32) -> c_int;
    pub fn proc_pid_rusage(pid: c_int, flavor: c_int, buffer: *mut c_void) -> c_int;
    pub fn localtime_r(time: *const i64, out: *mut Tm) -> *mut Tm;
}

#[link(name = "IOKit", kind = "framework")]
extern "C" {
    pub fn IOServiceMatching(name: *const c_char) -> *mut c_void;
    pub fn IOServiceGetMatchingServices(port: u32, matching: *mut c_void, iter: *mut u32) -> c_int;
    pub fn IOIteratorNext(iter: u32) -> u32;
    pub fn IOObjectRelease(object: u32) -> c_int;
    pub fn IORegistryEntryGetChildIterator(entry: u32, plane: *const c_char, iter: *mut u32) -> c_int;
    pub fn IORegistryEntryCreateCFProperty(
        entry: u32,
        key: *const c_void,
        allocator: *const c_void,
        options: u32,
    ) -> *const c_void;
    pub fn IORegistryEntrySearchCFProperty(
        entry: u32,
        plane: *const c_char,
        key: *const c_void,
        allocator: *const c_void,
        options: u32,
    ) -> *const c_void;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    pub static kCFBooleanTrue: *const c_void;
    pub static kCFTypeDictionaryKeyCallBacks: u8;
    pub static kCFTypeDictionaryValueCallBacks: u8;
    pub fn CFStringCreateWithCString(alloc: *const c_void, s: *const c_char, enc: u32) -> *const c_void;
    pub fn CFStringGetLength(s: *const c_void) -> isize;
    pub fn CFStringGetMaximumSizeForEncoding(length: isize, enc: u32) -> isize;
    pub fn CFStringGetCString(s: *const c_void, buf: *mut c_char, size: isize, enc: u32) -> u8;
    pub fn CFStringGetTypeID() -> usize;
    pub fn CFDictionaryCreate(
        alloc: *const c_void,
        keys: *const *const c_void,
        values: *const *const c_void,
        count: isize,
        key_callbacks: *const u8,
        value_callbacks: *const u8,
    ) -> *const c_void;
    pub fn CFDictionaryGetValue(dict: *const c_void, key: *const c_void) -> *const c_void;
    pub fn CFDictionaryGetTypeID() -> usize;
    pub fn CFNumberGetValue(number: *const c_void, kind: c_int, out: *mut c_void) -> u8;
    pub fn CFNumberGetTypeID() -> usize;
    pub fn CFArrayGetCount(array: *const c_void) -> isize;
    pub fn CFArrayGetValueAtIndex(array: *const c_void, index: isize) -> *const c_void;
    pub fn CFArrayGetTypeID() -> usize;
    pub fn CFDataCreate(alloc: *const c_void, bytes: *const u8, length: isize) -> *const c_void;
    pub fn CFPropertyListCreateWithData(
        alloc: *const c_void,
        data: *const c_void,
        options: usize,
        format: *mut isize,
        error: *mut *const c_void,
    ) -> *const c_void;
    pub fn CFURLCopyLastPathComponent(url: *const c_void) -> *const c_void;
    pub fn CFGetTypeID(object: *const c_void) -> usize;
    pub fn CFRelease(object: *const c_void);
}

#[link(name = "CoreServices", kind = "framework")]
extern "C" {
    pub fn LSCopyApplicationURLsForBundleIdentifier(id: *const c_void, error: *mut *const c_void) -> *const c_void;
}

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    pub fn AXIsProcessTrustedWithOptions(options: *const c_void) -> u8;
    pub fn AXUIElementCreateApplication(pid: c_int) -> *const c_void;
    pub fn AXUIElementCreateSystemWide() -> *const c_void;
    pub fn AXUIElementCopyAttributeValue(element: *const c_void, attribute: *const c_void, value: *mut *const c_void) -> i32;
    pub fn AXUIElementSetMessagingTimeout(element: *const c_void, seconds: f32) -> i32;
}

#[link(name = "sqlite3")]
extern "C" {
    pub fn sqlite3_open_v2(path: *const c_char, db: *mut *mut c_void, flags: c_int, vfs: *const c_char) -> c_int;
    pub fn sqlite3_close(db: *mut c_void) -> c_int;
    pub fn sqlite3_busy_timeout(db: *mut c_void, ms: c_int) -> c_int;
    pub fn sqlite3_prepare_v2(
        db: *mut c_void,
        sql: *const c_char,
        len: c_int,
        stmt: *mut *mut c_void,
        tail: *mut *const c_char,
    ) -> c_int;
    pub fn sqlite3_step(stmt: *mut c_void) -> c_int;
    pub fn sqlite3_finalize(stmt: *mut c_void) -> c_int;
    pub fn sqlite3_column_blob(stmt: *mut c_void, col: c_int) -> *const u8;
    pub fn sqlite3_column_bytes(stmt: *mut c_void, col: c_int) -> c_int;
    pub fn sqlite3_column_double(stmt: *mut c_void, col: c_int) -> f64;
    pub fn sqlite3_column_int64(stmt: *mut c_void, col: c_int) -> i64;
    pub fn sqlite3_column_text(stmt: *mut c_void, col: c_int) -> *const c_char;
}

/// A CFString for a constant key; null if the text holds a NUL byte. The caller owns it.
pub fn cf_string(s: &str) -> *const c_void {
    match CString::new(s) {
        Ok(c) => unsafe { CFStringCreateWithCString(std::ptr::null(), c.as_ptr(), CF_UTF8) },
        Err(_) => std::ptr::null(),
    }
}

/// Copies a CFString into a Rust string; None for null or any other type.
///
/// # Safety
/// `s` must be null or a valid CoreFoundation object.
pub unsafe fn cf_to_string(s: *const c_void) -> Option<String> {
    if s.is_null() || CFGetTypeID(s) != CFStringGetTypeID() {
        return None;
    }
    let size = CFStringGetMaximumSizeForEncoding(CFStringGetLength(s), CF_UTF8) + 1;
    let mut buf = vec![0u8; size as usize];
    if CFStringGetCString(s, buf.as_mut_ptr() as *mut c_char, size, CF_UTF8) == 0 {
        return None;
    }
    CStr::from_bytes_until_nul(&buf).ok().map(|c| c.to_string_lossy().into_owned())
}

/// Reads a CFNumber as i64; None for null or any other type.
///
/// # Safety
/// `value` must be null or a valid CoreFoundation object.
pub unsafe fn cf_to_i64(value: *const c_void) -> Option<i64> {
    if value.is_null() || CFGetTypeID(value) != CFNumberGetTypeID() {
        return None;
    }
    let mut out: i64 = 0;
    CFNumberGetValue(value, CF_NUMBER_SINT64, &mut out as *mut i64 as *mut c_void);
    Some(out)
}
