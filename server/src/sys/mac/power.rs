use crate::util::log;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering};

const MSG_CAN_SYSTEM_SLEEP: u32 = 0xe000_0270;
const MSG_SYSTEM_WILL_SLEEP: u32 = 0xe000_0280;
const MSG_SYSTEM_WILL_POWER_ON: u32 = 0xe000_0320;
const MSG_SYSTEM_HAS_POWERED_ON: u32 = 0xe000_0300;
const SIGTERM: i32 = 15;
const SIGINT: i32 = 2;

static GOING_TO_SLEEP: AtomicBool = AtomicBool::new(false);
static STOPPING: AtomicBool = AtomicBool::new(false);
static ROOT_PORT: AtomicU32 = AtomicU32::new(0);
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

/// struct sigaction of macOS: the handler, the mask (32 bits) and the flags.
#[repr(C)]
struct SigAction {
    handler: usize,
    mask: u32,
    flags: i32,
}

const SA_SIGINFO: i32 = 0x40;

extern "C" {
    fn sigaction(sig: i32, action: *const SigAction, old: *mut SigAction) -> i32;
}

/// Who sent the signal that stops the server (si_pid), so that the log can say.
static SENDER: AtomicI32 = AtomicI32::new(0);

extern "C" fn on_signal(_: i32, info: *const i32, _: *mut c_void) {
    // siginfo_t: si_signo, si_errno, si_code, si_pid, … (32-bit each at the start).
    if !info.is_null() {
        SENDER.store(unsafe { *info.add(3) }, Ordering::SeqCst);
    }
    STOPPING.store(true, Ordering::SeqCst);
}

/// The process that asked the server to stop; 0 when unknown.
pub fn stop_sender() -> i32 {
    SENDER.load(Ordering::SeqCst)
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
            crate::power::wait_for_poll(since);
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
        let action = SigAction { handler: on_signal as *const () as usize, mask: 0, flags: SA_SIGINFO };
        sigaction(SIGTERM, &action, std::ptr::null_mut());
        sigaction(SIGINT, &action, std::ptr::null_mut());
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
