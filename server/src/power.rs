use crate::util::log;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};

const MSG_CAN_SYSTEM_SLEEP: u32 = 0xe000_0270;
const MSG_SYSTEM_WILL_SLEEP: u32 = 0xe000_0280;
const MSG_SYSTEM_WILL_POWER_ON: u32 = 0xe000_0320;
const MSG_SYSTEM_HAS_POWERED_ON: u32 = 0xe000_0300;
const SIGTERM: i32 = 15;
const SIGINT: i32 = 2;
const GRACE: Duration = Duration::from_secs(3);

static GOING_TO_SLEEP: AtomicBool = AtomicBool::new(false);
static STOPPING: AtomicBool = AtomicBool::new(false);
static ROOT_PORT: AtomicU32 = AtomicU32::new(0);
pub static LAST_POLL_MS: AtomicU64 = AtomicU64::new(0);

#[link(name = "IOKit", kind = "framework")]
extern "C" {
    fn IORegisterForSystemPower(
        refcon: *mut c_void,
        port: *mut *mut c_void,
        callback: extern "C" fn(*mut c_void, u32, u32, *mut c_void),
        notifier: *mut u32,
    ) -> u32;
    fn IONotificationPortGetRunLoopSource(port: *mut c_void) -> *mut c_void;
    fn IOAllowPowerChange(kernel_port: u32, notification_id: isize) -> i32;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    static kCFRunLoopCommonModes: *const c_void;
    fn CFRunLoopGetCurrent() -> *mut c_void;
    fn CFRunLoopAddSource(rl: *mut c_void, source: *mut c_void, mode: *const c_void);
    fn CFRunLoopRun();
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGMainDisplayID() -> u32;
    fn CGDisplayIsAsleep(display: u32) -> i32;
}

extern "C" {
    fn signal(sig: i32, handler: extern "C" fn(i32)) -> usize;
}

extern "C" fn on_signal(_: i32) {
    STOPPING.store(true, Ordering::SeqCst);
}

pub fn state() -> &'static str {
    if GOING_TO_SLEEP.load(Ordering::SeqCst) || STOPPING.load(Ordering::SeqCst) {
        "sleep"
    } else if unsafe { CGDisplayIsAsleep(CGMainDisplayID()) } != 0 {
        "display_off"
    } else {
        "awake"
    }
}

pub fn stopping() -> bool {
    STOPPING.load(Ordering::SeqCst)
}

pub fn wait_for_poll(since: u64) {
    if since.saturating_sub(LAST_POLL_MS.load(Ordering::Relaxed)) > 5000 {
        return;
    }
    let start = Instant::now();
    while start.elapsed() < GRACE && LAST_POLL_MS.load(Ordering::Relaxed) <= since {
        std::thread::sleep(Duration::from_millis(100));
    }
}

extern "C" fn on_power(_: *mut c_void, _: u32, message: u32, argument: *mut c_void) {
    let port = ROOT_PORT.load(Ordering::SeqCst);
    match message {
        MSG_CAN_SYSTEM_SLEEP => unsafe {
            IOAllowPowerChange(port, argument as isize);
        },
        MSG_SYSTEM_WILL_SLEEP => {
            GOING_TO_SLEEP.store(true, Ordering::SeqCst);
            log("power: the Mac is going to sleep");
            let since = crate::util::now_ms();
            wait_for_poll(since);
            unsafe { IOAllowPowerChange(port, argument as isize) };
        }
        MSG_SYSTEM_WILL_POWER_ON | MSG_SYSTEM_HAS_POWERED_ON if GOING_TO_SLEEP.swap(false, Ordering::SeqCst) => {
            log("power: the Mac woke up");
        }
        _ => {}
    }
}

pub fn start() {
    unsafe {
        signal(SIGTERM, on_signal);
        signal(SIGINT, on_signal);
    }
    let spawned = std::thread::Builder::new().name("power".into()).spawn(|| unsafe {
        let mut port: *mut c_void = std::ptr::null_mut();
        let mut notifier: u32 = 0;
        let root = IORegisterForSystemPower(std::ptr::null_mut(), &mut port, on_power, &mut notifier);
        if root == 0 || port.is_null() {
            log("power: cannot register for sleep notifications");
            return;
        }
        ROOT_PORT.store(root, Ordering::SeqCst);
        CFRunLoopAddSource(CFRunLoopGetCurrent(), IONotificationPortGetRunLoopSource(port), kCFRunLoopCommonModes);
        CFRunLoopRun();
    });
    if spawned.is_err() {
        log("power: cannot start the power thread");
    }
}
