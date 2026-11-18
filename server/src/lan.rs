//! The Wi-Fi side: this Mac's addresses and a beacon that lets the phone find them.
//!
//! Every few seconds the server broadcasts on each network a line
//! `DESKSTATS2 <ip> <port> <time> <hmac>` to UDP port 8788. The HMAC (key from the token) covers
//! the address, the port and the time (ms since the epoch), so another device can neither point
//! the phone at itself by faking a beacon nor replay an old one: the phone takes only a beacon
//! newer than the last.

use crate::crypto::Keys;
use crate::ffi::{freeifaddrs, getifaddrs, IfAddrs};
use crate::util::{log, now_ms};
use std::ffi::CStr;
use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};
use std::time::Duration;

pub const BEACON_PORT: u16 = 8788;
const AF_INET: u8 = 2;
const IFF_UP: u32 = 0x1;
const IFF_BROADCAST: u32 = 0x2;
const IFF_LOOPBACK: u32 = 0x8;
const IFF_RUNNING: u32 = 0x40;

/// IPv4 address and broadcast address of every active physical interface (en*).
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
            // sockaddr_in: len, family, port (2 bytes), then the address
            let addr = |p: *const u8| Ipv4Addr::new(*p.add(4), *p.add(5), *p.add(6), *p.add(7));
            out.push((addr(ifa.ifa_addr), addr(ifa.ifa_dstaddr)));
        }
        freeifaddrs(list);
    }
    out
}

pub fn beacon_text(keys: &Keys, ip: Ipv4Addr, port: u16, time_ms: u64) -> String {
    let body = format!("DESKSTATS2 {ip} {port} {time_ms}");
    format!("{body} {}", keys.sign(&body))
}

/// Broadcasts the beacon every 3 seconds on every network this Mac is on.
pub fn run_beacon(keys: Keys, port: u16) {
    let mut failing = false;
    loop {
        let mut sent = false;
        for (ip, broadcast) in interfaces() {
            let Ok(socket) = UdpSocket::bind(SocketAddrV4::new(ip, 0)) else { continue };
            if socket.set_broadcast(true).is_err() {
                continue;
            }
            let text = beacon_text(&keys, ip, port, now_ms());
            if socket.send_to(text.as_bytes(), SocketAddrV4::new(broadcast, BEACON_PORT)).is_ok() {
                sent = true;
            }
        }
        if !sent && !failing {
            log("wifi: cannot send the beacon (no network?)");
        } else if sent && failing {
            log("wifi: beacon is being sent again");
        }
        failing = !sent;
        std::thread::sleep(Duration::from_secs(3));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn beacon_is_signed_over_address_port_and_time() {
        let keys = Keys::from_token("0123456789abcdef0123456789abcdef");
        let a = beacon_text(&keys, Ipv4Addr::new(192, 168, 31, 135), 8787, 1000);
        let b = beacon_text(&keys, Ipv4Addr::new(192, 168, 31, 136), 8787, 1000);
        let later = beacon_text(&keys, Ipv4Addr::new(192, 168, 31, 135), 8787, 1001);
        assert!(a.starts_with("DESKSTATS2 192.168.31.135 8787 1000 "));
        assert_ne!(a.rsplit(' ').next(), b.rsplit(' ').next());
        assert_ne!(a.rsplit(' ').next(), later.rsplit(' ').next());
        assert_eq!(a.rsplit(' ').next().unwrap().len(), 64);
    }
}
