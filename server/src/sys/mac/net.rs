//! The network interfaces of the Mac: the built-in and plugged-in ones are all called en….

use super::ffi::{freeifaddrs, getifaddrs, IfAddrs};
use std::ffi::CStr;
use std::net::Ipv4Addr;

const AF_INET: u8 = 2;
const IFF_UP: u32 = 0x1;
const IFF_BROADCAST: u32 = 0x2;
const IFF_LOOPBACK: u32 = 0x8;
const IFF_RUNNING: u32 = 0x40;

/// The address and the broadcast address of every interface that is up.
pub fn interfaces() -> Vec<(Ipv4Addr, Ipv4Addr)> {
    let mut out = Vec::new();
    unsafe {
        let mut list: *mut IfAddrs = std::ptr::null_mut();
        if getifaddrs(&mut list) != 0 {
            return out;
        }
        let mut cur = list;
        while !cur.is_null() {
            let ifa = &*cur;
            cur = ifa.ifa_next;
            let flags = ifa.ifa_flags;
            if ifa.ifa_addr.is_null() || ifa.ifa_dstaddr.is_null() || *ifa.ifa_addr.add(1) != AF_INET {
                continue;
            }
            if flags & (IFF_UP | IFF_RUNNING | IFF_BROADCAST) != (IFF_UP | IFF_RUNNING | IFF_BROADCAST) || flags & IFF_LOOPBACK != 0 {
                continue;
            }
            if !CStr::from_ptr(ifa.ifa_name).to_bytes().starts_with(b"en") {
                continue;
            }
            let addr = |p: *const u8| Ipv4Addr::new(*p.add(4), *p.add(5), *p.add(6), *p.add(7));
            out.push((addr(ifa.ifa_addr), addr(ifa.ifa_dstaddr)));
        }
        freeifaddrs(list);
    }
    out
}
