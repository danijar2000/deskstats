//! Windows: everything the server asks of the system it runs on, through the Win32 API.

#[path = "../portable.rs"]
mod portable;
pub mod extra;

pub use portable::{crypto, ec};
use std::collections::HashMap;
use std::ffi::c_void;
use std::net::Ipv4Addr;
use std::sync::Mutex;
use std::time::Duration;
use windows_sys::Win32::Foundation::{CloseHandle, FILETIME, HANDLE, SYSTEMTIME};
use windows_sys::Win32::NetworkManagement::IpHelper::{FreeMibTable, GetAdaptersAddresses, GetIfTable2, IP_ADAPTER_ADDRESSES_LH, MIB_IF_TABLE2};
use windows_sys::Win32::Networking::WinSock::{AF_INET, SOCKADDR_IN};
use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ};
use windows_sys::Win32::System::SystemInformation::{GetLocalTime, GlobalMemoryStatusEx, MEMORYSTATUSEX};
use windows_sys::Win32::System::Threading::GetSystemTimes;

pub const OS: &str = "windows";
/// Windows 10 and later bring curl.exe in System32.
pub const CURL: &str = "curl.exe";

const IF_TYPE_ETHERNET: u32 = 6;
const IF_TYPE_WIFI: u32 = 71;
const OPER_STATUS_UP: i32 = 1;

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn from_wide(buf: &[u16]) -> String {
    let end = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

/// A handle that is closed when it goes out of scope, so that no path forgets to.
struct Handle(HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

fn ticks(time: &FILETIME) -> u64 {
    (time.dwHighDateTime as u64) << 32 | time.dwLowDateTime as u64
}

pub fn data_dir() -> String {
    let base = std::env::var("APPDATA").or_else(|_| std::env::var("USERPROFILE")).unwrap_or_else(|_| String::from("."));
    format!("{base}\\deskstats")
}

/// The folder is in the user's profile, which other users cannot read.
pub fn private_dir(_dir: &str) -> std::io::Result<()> {
    Ok(())
}

pub fn private_file(path: &str) -> std::io::Result<std::fs::File> {
    std::fs::OpenOptions::new().write(true).create(true).truncate(true).open(path)
}

/// Year, month, day, hour, minute and second on the wall clock, now.
pub fn local_time(_secs: i64) -> Option<[i32; 6]> {
    let mut now: SYSTEMTIME = unsafe { std::mem::zeroed() };
    unsafe { GetLocalTime(&mut now) };
    Some([now.wYear as i32, now.wMonth as i32, now.wDay as i32, now.wHour as i32, now.wMinute as i32, now.wSecond as i32])
}

/// The server has no console on Windows, so the log goes to log.txt next to the token. It starts over when it grows large.
pub fn log_line(line: &str) {
    use std::io::Write;
    static LOG: Mutex<()> = Mutex::new(());
    if cfg!(test) {
        return;
    }
    let _guard = LOG.lock().unwrap_or_else(|e| e.into_inner());
    let dir = data_dir();
    let path = format!("{dir}\\log.txt");
    let _ = std::fs::create_dir_all(&dir);
    let large = std::fs::metadata(&path).is_ok_and(|meta| meta.len() > 256 * 1024);
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(!large).write(true).truncate(large).open(&path) {
        let _ = writeln!(file, "{line}");
    }
}

/// The full path of a program found through PATH; a folder may stand in quotes there.
pub fn find_in_path(program: &str) -> Option<String> {
    let path = std::env::var("PATH").unwrap_or_default();
    path.split(';').map(|dir| dir.trim().trim_matches('"')).filter(|dir| !dir.is_empty()).map(|dir| format!("{}\\{program}", dir.trim_end_matches('\\'))).find(|file| std::path::Path::new(file).is_file())
}

#[link(name = "user32")]
extern "system" {
    // Not in the headers, but exported by every Windows since XP: a message box that closes by itself.
    fn MessageBoxTimeoutW(window: *mut c_void, text: *const u16, caption: *const u16, kind: u32, language: u16, milliseconds: u32) -> i32;
}

/// Shows a Yes/No box and waits for the answer; true only for a click on Yes.
pub fn ask(text: &str, _deny: &str, allow: &str, wait: Duration) -> bool {
    const MB_YESNO: u32 = 0x4;
    const MB_ICONWARNING: u32 = 0x30;
    const MB_DEFBUTTON2: u32 = 0x100;
    const MB_SYSTEMMODAL: u32 = 0x1000;
    const MB_SETFOREGROUND: u32 = 0x1_0000;
    const ID_YES: i32 = 6;
    // The buttons say Yes and No in the language of Windows, so the question itself ends the text.
    let (text, caption) = (wide(&format!("{text}\n\n{allow}?")), wide("Deskstats"));
    let kind = MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2 | MB_SYSTEMMODAL | MB_SETFOREGROUND;
    let milliseconds = wait.as_millis().min(u32::MAX as u128) as u32;
    unsafe { MessageBoxTimeoutW(std::ptr::null_mut(), text.as_ptr(), caption.as_ptr(), kind, 0, milliseconds) == ID_YES }
}

/// Notifications of other programs are not read on Windows: the answer simply has no "alerts".
pub struct Notifications;

impl Notifications {
    pub fn new() -> Self {
        Notifications
    }

    pub fn json(&mut self) -> (String, String) {
        (String::new(), String::new())
    }
}

/// A text value from the registry of the machine.
fn registry_text(key: &str, value: &str) -> Option<String> {
    let (key, value) = (wide(key), wide(value));
    let mut buf = [0u16; 256];
    let mut bytes = std::mem::size_of_val(&buf) as u32;
    let rc = unsafe { RegGetValueW(HKEY_LOCAL_MACHINE, key.as_ptr(), value.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), buf.as_mut_ptr() as *mut c_void, &mut bytes) };
    if rc != 0 {
        return None;
    }
    let text = from_wide(&buf).split_whitespace().collect::<Vec<_>>().join(" ");
    (!text.is_empty()).then_some(text)
}

pub struct Host;

impl Host {
    pub fn new() -> Self {
        Host
    }

    pub fn cpu_name(&self) -> String {
        registry_text("HARDWARE\\DESCRIPTION\\System\\CentralProcessor\\0", "ProcessorNameString").unwrap_or_else(|| std::env::consts::ARCH.to_string())
    }

    fn memory(&self) -> Option<MEMORYSTATUSEX> {
        let mut status: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
        status.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
        (unsafe { GlobalMemoryStatusEx(&mut status) } != 0).then_some(status)
    }

    pub fn ram_total(&self) -> u64 {
        self.memory().map_or(0, |status| status.ullTotalPhys)
    }

    pub fn ram_used(&self) -> u64 {
        self.memory().map_or(0, |status| status.ullTotalPhys.saturating_sub(status.ullAvailPhys))
    }

    /// Busy and total time of all cores together, in milliseconds.
    pub fn cpu_ticks(&self) -> Vec<(u64, u64)> {
        let zero = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
        let (mut idle, mut kernel, mut user) = (zero, zero, zero);
        if unsafe { GetSystemTimes(&mut idle, &mut kernel, &mut user) } == 0 {
            return Vec::new();
        }
        // Kernel time includes idle time. Milliseconds keep the differences far below the 32 bits they are compared in.
        let (idle, total) = (ticks(&idle) / 10_000, (ticks(&kernel) + ticks(&user)) / 10_000);
        vec![(total.saturating_sub(idle), total)]
    }
}

/// Bytes received and sent by every network card that is up, by the number Windows gives the card.
pub fn net_counters() -> HashMap<String, (u64, u64)> {
    let mut out = HashMap::new();
    unsafe {
        let mut table: *mut MIB_IF_TABLE2 = std::ptr::null_mut();
        if GetIfTable2(&mut table) != 0 || table.is_null() {
            return out;
        }
        let rows = std::slice::from_raw_parts((*table).Table.as_ptr(), (*table).NumEntries as usize);
        for row in rows {
            // Bit 0 marks real hardware: without it every filter driver on top of a card would count its traffic again.
            let hardware = row.InterfaceAndOperStatusFlags._bitfield & 1 != 0;
            if hardware && row.OperStatus == OPER_STATUS_UP && (row.Type == IF_TYPE_ETHERNET || row.Type == IF_TYPE_WIFI) {
                out.insert(row.InterfaceLuid.Value.to_string(), (row.InOctets, row.OutOctets));
            }
        }
        FreeMibTable(table as *const c_void);
    }
    out
}

/// The address and the broadcast address of every Ethernet and Wi‑Fi adapter that is up.
pub fn interfaces() -> Vec<(Ipv4Addr, Ipv4Addr)> {
    const SKIP_ANYCAST_MULTICAST_DNS: u32 = 0x2 | 0x4 | 0x8;
    const ERROR_BUFFER_OVERFLOW: u32 = 111;
    let cards = net_counters();
    let mut out = Vec::new();
    let mut size: u32 = 16 * 1024;
    for _ in 0..4 {
        // u64 elements give the buffer the alignment the structures need.
        let mut buf = vec![0u64; size as usize / 8 + 1];
        let first = buf.as_mut_ptr() as *mut IP_ADAPTER_ADDRESSES_LH;
        let rc = unsafe { GetAdaptersAddresses(AF_INET as u32, SKIP_ANYCAST_MULTICAST_DNS, std::ptr::null(), first, &mut size) };
        if rc == ERROR_BUFFER_OVERFLOW {
            continue;
        }
        if rc != 0 {
            break;
        }
        let mut adapter = first as *const IP_ADAPTER_ADDRESSES_LH;
        while !adapter.is_null() {
            let a = unsafe { &*adapter };
            adapter = a.Next;
            if a.OperStatus != OPER_STATUS_UP || (a.IfType != IF_TYPE_ETHERNET && a.IfType != IF_TYPE_WIFI) {
                continue;
            }
            let card = cards.contains_key(&unsafe { a.Luid.Value }.to_string());
            let mut unicast = a.FirstUnicastAddress;
            while !unicast.is_null() {
                let u = unsafe { &*unicast };
                unicast = u.Next;
                let socket = u.Address.lpSockaddr;
                if socket.is_null() || unsafe { (*socket).sa_family } != AF_INET || !(1..=30).contains(&u.OnLinkPrefixLength) {
                    continue;
                }
                // The address is kept in network order, so its bytes in memory are the four numbers as written.
                let ip = Ipv4Addr::from(unsafe { (*(socket as *const SOCKADDR_IN)).sin_addr.S_un.S_addr }.to_ne_bytes());
                let mask = u32::MAX << (32 - u.OnLinkPrefixLength as u32);
                out.push((card, (ip, Ipv4Addr::from(u32::from(ip) | !mask))));
            }
        }
        break;
    }
    // Hyper-V, WSL and VirtualBox make adapters that look like Ethernet. Only real network cards count, unless
    // Windows marks none as real (some virtual machines), in which case all of them do.
    let real = out.iter().any(|(card, _)| *card);
    out.into_iter().filter(|(card, _)| *card || !real).map(|(_, addresses)| addresses).collect()
}

pub fn computer_name() -> String {
    std::env::var("COMPUTERNAME").ok().filter(|name| !name.trim().is_empty()).unwrap_or_else(|| String::from("Windows PC"))
}

/// What the maker calls this computer, such as "ThinkPad X1 Carbon Gen 9".
pub fn model_name() -> String {
    let placeholder = |name: &str| {
        let lower = name.to_lowercase();
        ["system product name", "to be filled", "default string", "not specified", "o.e.m", "unknown"].iter().any(|junk| lower.contains(junk))
    };
    ["SystemProductName", "BaseBoardProduct"]
        .iter()
        .filter_map(|value| registry_text("HARDWARE\\DESCRIPTION\\System\\BIOS", value))
        .find(|name| !placeholder(name))
        .unwrap_or_else(|| String::from("Windows PC"))
}

/// The load of the graphics card is not read on Windows yet.
pub struct Gpu;

impl Gpu {
    pub fn new() -> Self {
        Gpu
    }

    pub fn sample(&self) -> Option<(f64, u64)> {
        None
    }
}

/// The batteries of wireless mice and keyboards are not read on Windows yet.
pub struct Batteries;

impl Batteries {
    pub fn new() -> Self {
        Batteries
    }

    pub fn refresh(&mut self) -> &str {
        "[]"
    }
}

pub mod power {
    /// Windows ends the server without a warning, and its sleep is not followed yet: the phone just loses the connection.
    pub fn start() {}

    pub fn state() -> &'static str {
        "awake"
    }

    pub fn stopping() -> bool {
        false
    }
}

pub mod procs {
    use super::{from_wide, ticks, Handle};
    use crate::procs::Counters;
    use std::collections::HashMap;
    use std::sync::Mutex;
    use windows_sys::Win32::Foundation::{FILETIME, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS};
    use windows_sys::Win32::System::ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
    use windows_sys::Win32::System::Threading::{GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};

    /// The names from the latest listing; replaced each time, so it never grows.
    static NAMES: Mutex<Option<HashMap<i32, String>>> = Mutex::new(None);

    /// Fills in every process this user may look at; None when the list cannot be read. Per-process network use is not known, hence false.
    pub fn read(pids: &mut HashMap<i32, Counters>) -> Option<bool> {
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
        if snapshot == INVALID_HANDLE_VALUE {
            return None;
        }
        let snapshot = Handle(snapshot);
        let mut names = HashMap::new();
        let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut more = unsafe { Process32FirstW(snapshot.0, &mut entry) } != 0;
        while more {
            let pid = entry.th32ProcessID;
            // Process 0 is the idle time of the processor, not a program.
            if pid != 0 {
                let name = from_wide(&entry.szExeFile);
                let name = name.strip_suffix(".exe").or_else(|| name.strip_suffix(".EXE")).unwrap_or(&name).to_string();
                if let Some(counters) = counters(pid) {
                    pids.insert(pid as i32, counters);
                    names.insert(pid as i32, name);
                }
            }
            more = unsafe { Process32NextW(snapshot.0, &mut entry) } != 0;
        }
        *NAMES.lock().unwrap_or_else(|e| e.into_inner()) = Some(names);
        (!pids.is_empty()).then_some(false)
    }

    /// Processor time and memory of one process; None for those of the system that may not be opened.
    fn counters(pid: u32) -> Option<Counters> {
        let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if process.is_null() {
            return None;
        }
        let process = Handle(process);
        let zero = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
        let (mut created, mut exited, mut kernel, mut user) = (zero, zero, zero, zero);
        if unsafe { GetProcessTimes(process.0, &mut created, &mut exited, &mut kernel, &mut user) } == 0 {
            return None;
        }
        let mut memory: PROCESS_MEMORY_COUNTERS = unsafe { std::mem::zeroed() };
        let size = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
        let ram = if unsafe { K32GetProcessMemoryInfo(process.0, &mut memory, size) } != 0 { memory.WorkingSetSize as u64 } else { 0 };
        // Times are in units of 100 nanoseconds; the tracker counts hundredths of a second.
        Some(Counters { cpu_cs: (ticks(&kernel) + ticks(&user)) / 100_000, ram, ..Counters::default() })
    }

    pub fn app_name(pid: i32) -> String {
        NAMES.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|names| names.get(&pid).cloned()).unwrap_or_default()
    }
}

pub mod adb {
    use std::ffi::c_void;
    use std::sync::OnceLock;
    use std::time::Duration;
    use windows_sys::core::GUID;
    use windows_sys::Win32::Devices::DeviceAndDriverInstallation::{
        CM_Register_Notification, CM_NOTIFY_ACTION, CM_NOTIFY_ACTION_DEVICEINTERFACEARRIVAL, CM_NOTIFY_ACTION_DEVICEINTERFACEREMOVAL, CM_NOTIFY_EVENT_DATA, CM_NOTIFY_FILTER,
        CM_NOTIFY_FILTER_TYPE_DEVICEINTERFACE, HCMNOTIFICATION,
    };

    /// Every USB device shows this interface (GUID_DEVINTERFACE_USB_DEVICE).
    const USB_DEVICE: GUID = GUID { data1: 0xA5DC_BF10, data2: 0x6530, data3: 0x11D2, data4: [0x90, 0x1F, 0x00, 0xC0, 0x4F, 0xB9, 0x51, 0xED] };

    pub fn candidates() -> Vec<String> {
        let folder = |name: &str| std::env::var(name).unwrap_or_default();
        let mut list: Vec<String> = super::find_in_path("adb.exe").into_iter().collect();
        list.extend([
            format!("{}\\Android\\Sdk\\platform-tools\\adb.exe", folder("LOCALAPPDATA")),
            format!("{}\\Microsoft\\WinGet\\Links\\adb.exe", folder("LOCALAPPDATA")),
            format!("{}\\scoop\\shims\\adb.exe", folder("USERPROFILE")),
            String::from("C:\\platform-tools\\adb.exe"),
        ]);
        list
    }

    /// Runs adb with [input] on its standard input; its output when it ended well in time.
    pub fn run(path: &str, _dir: &str, args: &[&str], input: Option<&str>, timeout: Duration) -> Option<String> {
        // The first adb command starts adb's background server, which on Windows inherits the handles of the
        // command that started it. If that were the pipe this server reads the output from, the pipe would never
        // close. So the background server is started first, by a command whose output goes nowhere.
        if args.first() == Some(&"devices") {
            crate::util::run_silent(path, &["start-server"], timeout + Duration::from_secs(10));
        }
        let (ok, out) = crate::util::run_with_input(path, args, input, timeout)?;
        ok.then_some(out)
    }

    static CHANGED: OnceLock<fn()> = OnceLock::new();

    unsafe extern "system" fn on_device(_: HCMNOTIFICATION, _: *const c_void, action: CM_NOTIFY_ACTION, _: *const CM_NOTIFY_EVENT_DATA, _: u32) -> u32 {
        if action == CM_NOTIFY_ACTION_DEVICEINTERFACEARRIVAL || action == CM_NOTIFY_ACTION_DEVICEINTERFACEREMOVAL {
            if let Some(changed) = CHANGED.get() {
                changed();
            }
        }
        0
    }

    /// Asks Windows to say when a USB device comes or goes. The registration lasts as long as the server runs.
    pub fn watch_usb(changed: fn()) -> bool {
        let _ = CHANGED.set(changed);
        let mut filter: CM_NOTIFY_FILTER = unsafe { std::mem::zeroed() };
        filter.cbSize = std::mem::size_of::<CM_NOTIFY_FILTER>() as u32;
        filter.FilterType = CM_NOTIFY_FILTER_TYPE_DEVICEINTERFACE;
        filter.u.DeviceInterface.ClassGuid = USB_DEVICE;
        let mut registration: HCMNOTIFICATION = unsafe { std::mem::zeroed() };
        unsafe { CM_Register_Notification(&filter, std::ptr::null(), Some(on_device), &mut registration) == 0 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_system_answers() {
        let host = Host::new();
        assert!(!host.cpu_name().is_empty());
        assert!(host.ram_total() > 0 && host.ram_used() > 0 && host.ram_used() <= host.ram_total());
        let first = host.cpu_ticks();
        assert_eq!(first.len(), 1);
        assert!(first[0].1 >= first[0].0 && first[0].1 > 0);
        assert!(!computer_name().is_empty());
        assert!(!model_name().is_empty());
        assert!(data_dir().ends_with("\\deskstats"));
        let [year, month, ..] = local_time(0).unwrap();
        assert!(year >= 2024 && (1..=12).contains(&month));
        assert_eq!(Gpu::new().sample(), None);
        assert_eq!(Batteries::new().refresh(), "[]");
        assert_eq!(Notifications::new().json(), (String::new(), String::new()));
        assert_eq!((power::state(), power::stopping()), ("awake", false));
    }

    #[test]
    fn the_network_is_read() {
        for (ip, broadcast) in interfaces() {
            assert!(!ip.is_loopback() && !ip.is_unspecified());
            assert!(u32::from(broadcast) >= u32::from(ip));
        }
        let first = net_counters();
        let second = net_counters();
        for (name, (rx, _)) in &first {
            assert!(second.get(name).is_none_or(|(later, _)| later >= rx));
        }
    }

    #[test]
    fn processes_are_read() {
        let mut pids = HashMap::new();
        assert_eq!(procs::read(&mut pids), Some(false));
        let me = std::process::id() as i32;
        assert!(pids[&me].ram > 0);
        assert!(procs::app_name(me).starts_with("deskstats"));
        assert_eq!(procs::app_name(-5), "");
    }

    #[test]
    fn text_and_programs() {
        assert_eq!(wide("ab"), vec![97, 98, 0]);
        assert_eq!(from_wide(&[97, 98, 0, 99]), "ab");
        assert!(find_in_path("cmd.exe").is_some());
        assert_eq!(find_in_path("no-such-program-here.exe"), None);
        assert_eq!(registry_text("HARDWARE\\DESCRIPTION\\System\\Nothing", "Nothing"), None);
        assert!(!adb::candidates().is_empty());
    }
}
