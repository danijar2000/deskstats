//! Windows: the disk and the time since start. The CPU temperature is not told to ordinary programs, so there is none.

use super::{wide, Handle};
use std::ffi::c_void;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
use windows_sys::Win32::Storage::FileSystem::{CreateFileW, GetDiskFreeSpaceExW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING};
use windows_sys::Win32::System::Ioctl::{DISK_PERFORMANCE, IOCTL_DISK_PERFORMANCE};
use windows_sys::Win32::System::SystemInformation::GetTickCount64;
use windows_sys::Win32::System::IO::DeviceIoControl;

/// The size of the system drive and what is free on it for this user, in bytes.
pub fn disk() -> Option<(u64, u64)> {
    let drive = std::env::var("SystemDrive").ok().filter(|d| !d.is_empty()).unwrap_or_else(|| String::from("C:"));
    let root = wide(&format!("{drive}\\"));
    let (mut free, mut total, mut all_free) = (0u64, 0u64, 0u64);
    let ok = unsafe { GetDiskFreeSpaceExW(root.as_ptr(), &mut free, &mut total, &mut all_free) };
    (ok != 0 && total > 0).then_some((total, free))
}

/// Which physical disks answered, looked for again now and then: a disk may be plugged in.
static DRIVES: Mutex<Option<(Instant, Vec<u32>)>> = Mutex::new(None);
const LOOK_AGAIN: Duration = Duration::from_secs(300);

/// Bytes read from and written to the physical disks since start.
pub fn disk_counters() -> Option<(u64, u64)> {
    let mut drives = DRIVES.lock().unwrap_or_else(|e| e.into_inner());
    let fresh = drives.as_ref().is_some_and(|(at, _)| at.elapsed() < LOOK_AGAIN);
    let candidates: Vec<u32> = if fresh { drives.as_ref().map(|(_, list)| list.clone()).unwrap_or_default() } else { (0..8).collect() };
    let (mut read, mut written, mut any) = (0u64, 0u64, false);
    let mut answered = Vec::new();
    for n in candidates {
        let path = wide(&format!("\\\\.\\PhysicalDrive{n}"));
        // No access rights are needed for the counters, so an ordinary user may ask.
        let handle = unsafe { CreateFileW(path.as_ptr(), 0, FILE_SHARE_READ | FILE_SHARE_WRITE, std::ptr::null(), OPEN_EXISTING, 0, std::ptr::null_mut()) };
        if handle == INVALID_HANDLE_VALUE || handle.is_null() {
            continue;
        }
        let handle = Handle(handle);
        let mut perf: DISK_PERFORMANCE = unsafe { std::mem::zeroed() };
        let mut got = 0u32;
        let size = std::mem::size_of::<DISK_PERFORMANCE>() as u32;
        let ok = unsafe { DeviceIoControl(handle.0, IOCTL_DISK_PERFORMANCE, std::ptr::null(), 0, &mut perf as *mut _ as *mut c_void, size, &mut got, std::ptr::null_mut()) };
        if ok != 0 {
            any = true;
            answered.push(n);
            read = read.saturating_add(perf.BytesRead.max(0) as u64);
            written = written.saturating_add(perf.BytesWritten.max(0) as u64);
        }
    }
    if !fresh {
        *drives = Some((Instant::now(), answered));
    }
    any.then_some((read, written))
}

pub fn uptime() -> Option<u64> {
    Some(unsafe { GetTickCount64() } / 1000)
}

pub struct Thermal;

impl Thermal {
    pub fn new() -> Self {
        Thermal
    }

    pub fn celsius(&mut self) -> Option<f64> {
        None
    }
}
