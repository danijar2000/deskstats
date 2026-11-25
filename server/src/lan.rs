use crate::crypto::Keys;
use crate::sys::interfaces;
use crate::util::{json_escape, log, now_ms};
use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};
use std::time::Duration;

pub const BEACON_PORT: u16 = 8788;
pub fn beacon_text(keys: &Keys, ip: Ipv4Addr, port: u16, time_ms: u64) -> String {
    let body = format!("DESKSTATS2 {ip} {port} {time_ms}");
    format!("{body} {}", keys.sign(&body))
}

/// Who this Mac is, for any phone on the network: what /info answers and what the open announcement carries.
pub fn about_json(id: &str, name: &str, model: &str, port: u16) -> String {
    format!("{{\"app\":\"deskstats\",\"id\":\"{id}\",\"name\":\"{}\",\"model\":\"{}\",\"os\":\"{}\",\"port\":{port}}}", json_escape(name), json_escape(model), crate::sys::OS)
}

pub const QUERY: &str = "DESKSTATS?";

/// What this Mac says about itself: the open announcement, and the beacon signed with the token when Wi‑Fi is on.
fn messages(keys: &Keys, about: &str, ip: Ipv4Addr, port: u16, signed: bool) -> Vec<String> {
    let mut out = vec![format!("DESKSTATS3 {about}")];
    if signed {
        out.push(beacon_text(keys, ip, port, now_ms()));
    }
    out
}

/// The address of this Mac that [to] reaches it at.
fn own_address_towards(to: Ipv4Addr) -> Option<Ipv4Addr> {
    let socket = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    socket.connect(SocketAddrV4::new(to, BEACON_PORT)).ok()?;
    match socket.local_addr().ok()?.ip() {
        std::net::IpAddr::V4(ip) if !ip.is_unspecified() => Some(ip),
        _ => None,
    }
}

fn broadcast(keys: &Keys, about: &str, port: u16, signed: bool) -> bool {
    let mut sent = false;
    for (ip, broadcast) in interfaces() {
        let Ok(socket) = UdpSocket::bind(SocketAddrV4::new(ip, 0)) else { continue };
        if socket.set_broadcast(true).is_err() {
            continue;
        }
        for text in messages(keys, about, ip, port, signed) {
            sent |= socket.send_to(text.as_bytes(), SocketAddrV4::new(broadcast, BEACON_PORT)).is_ok();
        }
    }
    sent
}

/// Answers on demand, like LocalSend: a phone that looks for Macs broadcasts a question and every Mac answers it.
/// A Mac nobody asks sends nothing and sleeps in recv. It announces itself a few times when it starts, so that
/// lists already open learn about it, and says nothing more until asked.
pub fn run_beacon(keys: Keys, port: u16, about: String, settings: std::sync::Arc<crate::settings::Settings>) {
    let signed = || settings.lan.load(std::sync::atomic::Ordering::Relaxed);
    let socket = loop {
        match UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, BEACON_PORT)) {
            Ok(s) => break s,
            Err(e) => {
                log(&format!("wifi: cannot listen for phones on port {BEACON_PORT}: {e}"));
                std::thread::sleep(Duration::from_secs(60));
            }
        }
    };
    for wait in [0, 2, 5] {
        std::thread::sleep(Duration::from_secs(wait));
        if !broadcast(&keys, &about, port, signed()) {
            log("wifi: cannot announce this Mac (no network?)");
        }
    }
    let mut limit = Limit::default();
    let mut buf = [0u8; 64];
    loop {
        let Ok((n, from)) = socket.recv_from(&mut buf) else { continue };
        let std::net::SocketAddr::V4(from) = from else { continue };
        let asked = std::str::from_utf8(&buf[..n]).is_ok_and(|t| t.trim() == QUERY);
        if !asked || !(from.ip().is_private() || from.ip().is_link_local()) || !limit.allow(*from.ip()) {
            continue;
        }
        let Some(ip) = own_address_towards(*from.ip()) else { continue };
        for text in messages(&keys, &about, ip, port, signed()) {
            let _ = socket.send_to(text.as_bytes(), from);
        }
    }
}

/// Answers each phone at most twice a second and everybody at most 20 times a second, so a question cannot be
/// used to flood the network.
#[derive(Default)]
struct Limit {
    last: std::collections::HashMap<Ipv4Addr, std::time::Instant>,
    second: Option<std::time::Instant>,
    count: u32,
}

impl Limit {
    fn allow(&mut self, from: Ipv4Addr) -> bool {
        let now = std::time::Instant::now();
        if self.second.is_none_or(|t| now.duration_since(t) >= Duration::from_secs(1)) {
            self.second = Some(now);
            self.count = 0;
            self.last.retain(|_, t| now.duration_since(*t) < Duration::from_secs(1));
        }
        if self.count >= 20 || self.last.get(&from).is_some_and(|t| now.duration_since(*t) < Duration::from_millis(500)) {
            return false;
        }
        self.count += 1;
        self.last.insert(from, now);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_are_limited() {
        let mut limit = Limit::default();
        let phone = Ipv4Addr::new(192, 168, 1, 5);
        assert!(limit.allow(phone));
        assert!(!limit.allow(phone));
        let allowed = (0..40).filter(|i| limit.allow(Ipv4Addr::new(10, 0, 0, *i as u8))).count();
        assert_eq!(allowed, 19);
    }

    #[test]
    fn answers_carry_the_signed_beacon_only_with_wifi_on() {
        let keys = Keys::from_token("0123456789abcdef0123456789abcdef");
        let ip = Ipv4Addr::new(192, 168, 1, 2);
        assert_eq!(messages(&keys, "{}", ip, 8787, false), vec!["DESKSTATS3 {}".to_string()]);
        let both = messages(&keys, "{}", ip, 8787, true);
        assert!(both[1].starts_with("DESKSTATS2 192.168.1.2 8787 "));
    }

    #[test]
    fn about_is_json() {
        assert_eq!(
            about_json("0011223344556677", "Dan's \"Mac\"", "Mac mini (2024)", 8787),
            format!(r#"{{"app":"deskstats","id":"0011223344556677","name":"Dan's \"Mac\"","model":"Mac mini (2024)","os":"{}","port":8787}}"#, crate::sys::OS)
        );
    }

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
