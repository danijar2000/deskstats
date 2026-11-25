use crate::crypto::Keys;
use crate::procs::Tracker;
use crate::util::{log, now_ms};
use crate::settings::Settings;
use crate::weather::Weather;
use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

const MAX_CONNECTIONS: usize = 16;
const REQUEST_LIMIT: usize = 4096;
const FIRST_REQUEST: Duration = Duration::from_secs(3);
const REQUEST_DEADLINE: Duration = Duration::from_secs(10);
const IDLE_TIMEOUT: Duration = Duration::from_secs(30);
const PAD: usize = 1024;

pub struct Sample {
    pub head: String,
    pub alerts: String,
    pub alerts_full: String,
}

impl Sample {
    pub fn json(&self, full: bool, weather: &str) -> String {
        format!("{}{}\"weather\":{weather}}}", self.head, if full { &self.alerts_full } else { &self.alerts })
    }
}

struct Connection {
    id: u64,
    stream: TcpStream,
    trusted: Arc<AtomicBool>,
    since: Instant,
}

pub struct Shared {
    pub sample: Mutex<Option<Sample>>,
    pub weather: Arc<Weather>,
    pub settings: Arc<Settings>,
    pub procs: Tracker,
    pub last_request_ms: AtomicU64,
    pub last_local_ms: AtomicU64,
    pub token: String,
    pub keys: Keys,
    pub port: u16,
    /// What /info answers: {"app":"deskstats","id":…,"name":…,"model":…,"port":…}.
    pub about: String,
    pub pairing: Arc<crate::pair::Pairing>,
    /// Shows the Allow/Deny dialog; tests put a stand-in here.
    pub ask_user: fn(&str, &str, &str) -> bool,
    /// Rung by every request for data, so that the sampler sleeps until there is someone to sample for.
    asked: (Mutex<()>, Condvar),
    connections: Mutex<Vec<Connection>>,
    next_id: AtomicU64,
    addresses: Mutex<(Option<Instant>, Vec<Ipv4Addr>)>,
}

impl Shared {
    pub fn new(token: String, port: u16, weather: Arc<Weather>, settings: Arc<Settings>) -> Self {
        let keys = Keys::from_token(&token);
        Shared {
            sample: Mutex::new(None),
            weather,
            settings,
            procs: Tracker::default(),
            last_request_ms: AtomicU64::new(0),
            last_local_ms: AtomicU64::new(0),
            token,
            keys,
            port,
            about: String::from("{}"),
            pairing: Arc::default(),
            ask_user: crate::pair::ask_user,
            asked: (Mutex::new(()), Condvar::new()),
            connections: Mutex::new(Vec::new()),
            next_id: AtomicU64::new(0),
            addresses: Mutex::new((None, Vec::new())),
        }
    }

    fn ring(&self) {
        let _guard = self.asked.0.lock().unwrap_or_else(|e| e.into_inner());
        self.asked.1.notify_all();
    }

    /// Sleeps until a phone asks for data or [most] passes.
    pub fn wait_for_request(&self, most: Duration) {
        let guard = self.asked.0.lock().unwrap_or_else(|e| e.into_inner());
        let _ = self.asked.1.wait_timeout(guard, most);
    }

    fn arrived_on_own_network(&self, stream: &TcpStream) -> bool {
        let Ok(IpAddr::V4(to)) = stream.local_addr().map(|a| a.ip()) else { return false };
        if to.is_loopback() {
            return true;
        }
        let mut cache = self.addresses.lock().unwrap_or_else(|e| e.into_inner());
        let stale = cache.0.is_none_or(|t| t.elapsed() >= Duration::from_secs(1));
        if !cache.1.contains(&to) && stale {
            *cache = (Some(Instant::now()), crate::lan::interfaces().into_iter().map(|(ip, _)| ip).collect());
        }
        cache.1.contains(&to)
    }

    fn admit(&self, stream: &TcpStream) -> Option<(u64, Arc<AtomicBool>)> {
        let mut list = self.connections.lock().unwrap_or_else(|e| e.into_inner());
        if list.len() >= MAX_CONNECTIONS {
            let oldest = (0..list.len()).filter(|i| !list[*i].trusted.load(Ordering::Relaxed)).min_by_key(|i| list[*i].since)?;
            let _ = list.swap_remove(oldest).stream.shutdown(Shutdown::Both);
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let trusted = Arc::new(AtomicBool::new(false));
        list.push(Connection { id, stream: stream.try_clone().ok()?, trusted: trusted.clone(), since: Instant::now() });
        Some((id, trusted))
    }

    fn release(&self, id: u64) {
        self.connections.lock().unwrap_or_else(|e| e.into_inner()).retain(|c| c.id != id);
    }
}

pub struct Reply {
    pub status: &'static str,
    pub body: Vec<u8>,
    pub sealed: bool,
    pub trusted: bool,
}

pub fn run(listener: TcpListener, shared: Arc<Shared>) {
    let mut refused_logged = false;
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        if !shared.arrived_on_own_network(&stream) {
            continue;
        }
        let Some((id, trusted)) = shared.admit(&stream) else {
            if !refused_logged {
                log("http: too many connections, refusing new ones");
                refused_logged = true;
            }
            continue;
        };
        refused_logged = false;
        let worker = shared.clone();
        let spawned = std::thread::Builder::new().name("http".into()).stack_size(128 * 1024).spawn(move || {
            serve(stream, &worker, &trusted);
            worker.release(id);
        });
        if spawned.is_err() {
            shared.release(id);
            log("http: cannot start a thread for a connection");
        }
    }
}

pub struct Request<'a> {
    pub method: &'a str,
    pub path: &'a str,
    pub query: &'a str,
    pub host: Option<&'a str>,
    pub origin: bool,
    pub authorization: Option<&'a str>,
    pub body: bool,
    /// The X-Deskstats header: a browser cannot send it to another site without asking first.
    pub app: bool,
}

pub fn parse(head: &[u8]) -> Option<Request<'_>> {
    let text = std::str::from_utf8(head).ok()?;
    let mut lines = text.split("\r\n");
    let mut first = lines.next()?.split(' ');
    let method = first.next()?;
    let target = first.next()?;
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let mut request = Request { method, path, query, host: None, origin: false, authorization: None, body: false, app: false };
    for line in lines {
        let Some((name, value)) = line.split_once(':') else { continue };
        let value = value.trim();
        if name.eq_ignore_ascii_case("host") {
            request.host = Some(value);
        } else if name.eq_ignore_ascii_case("origin") {
            request.origin = true;
        } else if name.eq_ignore_ascii_case("x-deskstats") {
            request.app = true;
        } else if name.eq_ignore_ascii_case("authorization") {
            request.authorization = Some(value);
        } else if name.eq_ignore_ascii_case("transfer-encoding") || (name.eq_ignore_ascii_case("content-length") && value != "0") {
            request.body = true;
        }
    }
    Some(request)
}

fn token_of<'a>(request: &Request<'a>) -> Option<&'a str> {
    let value = request.authorization?;
    let (scheme, token) = value.split_once(' ')?;
    scheme.eq_ignore_ascii_case("bearer").then(|| token.trim())
}

pub fn param<'a>(query: &'a str, name: &str) -> Option<&'a str> {
    query.split('&').filter_map(|pair| pair.split_once('=')).find(|(key, _)| *key == name).map(|(_, value)| value)
}

fn error_json(text: &str) -> String {
    format!("{{\"error\":\"{text}\"}}")
}

fn answer(target: &str, shared: &Shared, local: bool) -> (u16, String) {
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    if path == "/config" {
        shared.settings.apply(query);
        shared.settings.save();
        shared.weather.set_every(Duration::from_secs(shared.settings.weather_min.load(Ordering::Relaxed) * 60));
        return (200, shared.settings.json());
    }
    if path != "/stats" && path != "/procs" {
        return (404, error_json("not found"));
    }
    let woken = now_ms().saturating_sub(shared.last_request_ms.swap(now_ms(), Ordering::Relaxed)) > 1000;
    if woken {
        shared.ring();
    }
    if local {
        shared.last_local_ms.store(now_ms(), Ordering::Relaxed);
    }
    let json = if crate::power::state() == "sleep" {
        Some(String::from("{\"mac\":\"sleep\"}"))
    } else if path == "/procs" {
        Some(shared.procs.json(param(query, "sort").unwrap_or("cpu")))
    } else {
        let sample = shared.sample.lock().unwrap_or_else(|e| e.into_inner());
        sample.as_ref().map(|sample| {
            let weather = param(query, "place")
                .and_then(crate::weather::parse_place)
                .map_or_else(|| String::from("null"), |(lat, lon)| shared.weather.place(lat, lon));
            sample.json(param(query, "alerts") == Some("full"), &weather)
        })
    };
    let Some(json) = json else {
        return (503, error_json("starting"));
    };
    crate::power::LAST_POLL_MS.store(now_ms(), Ordering::Relaxed);
    (200, json)
}

fn unseal<'a>(query: &str, keys: &Keys, plain: &'a mut String) -> Option<(&'a str, &'a str)> {
    *plain = String::from_utf8(keys.open(&crate::crypto::unhex(query)?)?).ok()?;
    let (nonce, target) = plain.strip_prefix("Q\n")?.split_once('\n')?;
    let valid = (16..=64).contains(&nonce.len()) && nonce.bytes().all(|b| b.is_ascii_alphanumeric());
    valid.then_some((nonce, target))
}

pub fn respond(request: &Request, shared: &Shared, local: bool) -> Reply {
    let refuse = |status, text: &str| Reply { status, body: error_json(text).into_bytes(), sealed: false, trusted: false };
    if request.origin {
        return refuse("403 Forbidden", "browser requests are not allowed");
    }
    if request.method != "GET" && request.method != "HEAD" {
        return refuse("405 Method Not Allowed", "method not allowed");
    }
    if request.body {
        return refuse("400 Bad Request", "a request body is not accepted");
    }
    if matches!(request.path, "/info" | "/pair" | "/pair/wait") {
        return open_door(request, shared);
    }
    if request.path == "/q" {
        if !local && !shared.settings.lan.load(Ordering::Relaxed) {
            return refuse("403 Forbidden", "wifi is off on this Mac");
        }
        let mut plain = String::new();
        let Some((nonce, target)) = unseal(request.query, &shared.keys, &mut plain) else {
            return refuse("401 Unauthorized", "not sealed with the token");
        };
        let (code, json) = answer(target, shared, local);
        let mut text = format!("A\n{nonce}\n{code}\n{json}");
        text.extend(std::iter::repeat_n(' ', (PAD - text.len() % PAD) % PAD));
        return match shared.keys.seal(text.as_bytes()) {
            Some(body) => Reply { status: "200 OK", body, sealed: true, trusted: true },
            None => Reply { status: "500 Internal Server Error", body: error_json("cannot encrypt").into_bytes(), sealed: false, trusted: true },
        };
    }
    if !local {
        return refuse("401 Unauthorized", "sealed requests only");
    }
    let allowed = [format!("127.0.0.1:{}", shared.port), format!("localhost:{}", shared.port)];
    if !request.host.is_some_and(|h| allowed.iter().any(|a| a.eq_ignore_ascii_case(h))) {
        return refuse("421 Misdirected Request", "unexpected host");
    }
    if !token_of(request).is_some_and(|t| crate::crypto::same(t.as_bytes(), shared.token.as_bytes())) {
        return refuse("401 Unauthorized", "token required");
    }
    let target = if request.query.is_empty() { request.path.to_string() } else { format!("{}?{}", request.path, request.query) };
    let (code, json) = answer(&target, shared, true);
    let status = match code {
        200 => "200 OK",
        404 => "404 Not Found",
        _ => "503 Service Unavailable",
    };
    Reply { status, body: json.into_bytes(), sealed: false, trusted: true }
}

/// What any phone on the network may ask without the token: who this Mac is, and to be let in.
fn open_door(request: &Request, shared: &Shared) -> Reply {
    use crate::pair::{Outcome, Start};
    let reply = |status, body: String| Reply { status, body: body.into_bytes(), sealed: false, trusted: false };
    // An address, not a name: a web page that points a name of its own at this Mac does not get in (DNS rebinding).
    let by_address = request.host.is_some_and(|h| h.rsplit_once(':').map_or(h, |(ip, _)| ip).parse::<Ipv4Addr>().is_ok());
    if !request.app || !by_address {
        return reply("403 Forbidden", error_json("not the deskstats app"));
    }
    match request.path {
        "/info" => reply("200 OK", shared.about.clone()),
        "/pair" => {
            let Some(key) = param(request.query, "k").and_then(crate::crypto::unhex) else {
                return reply("400 Bad Request", error_json("no key"));
            };
            let phone = crate::pair::decode(param(request.query, "n").unwrap_or(""), 40);
            let phone = if phone.is_empty() { String::from("Android") } else { phone };
            let language = param(request.query, "l").unwrap_or("en").chars().take(2).collect::<String>();
            match shared.pairing.start(&key) {
                Start::Busy => reply("429 Too Many Requests", error_json("busy")),
                Start::BadKey => reply("400 Bad Request", error_json("bad key")),
                Start::Started { mac_key, session, code } => {
                    let (pairing, settings, ask, id) = (shared.pairing.clone(), shared.settings.clone(), shared.ask_user, session.clone());
                    let asked = std::thread::Builder::new().name("pair".into()).spawn(move || {
                        let allowed = ask(&phone, &code, &language);
                        if allowed {
                            // The phone was let in over Wi‑Fi, so it is going to come back over Wi‑Fi.
                            settings.lan.store(true, Ordering::Relaxed);
                            settings.save();
                        }
                        pairing.decide(&id, allowed);
                    });
                    if asked.is_err() {
                        shared.pairing.decide(&session, false);
                        return reply("503 Service Unavailable", error_json("busy"));
                    }
                    reply("200 OK", format!("{{\"k\":\"{}\",\"s\":\"{session}\"}}", crate::crypto::hex(&mac_key)))
                }
            }
        }
        _ => match shared.pairing.wait(param(request.query, "s").unwrap_or(""), &shared.token, crate::pair::WAIT) {
            Outcome::Token(sealed) => reply("200 OK", format!("{{\"t\":\"{sealed}\"}}")),
            Outcome::Denied => reply("403 Forbidden", error_json("denied")),
            Outcome::Waiting => reply("202 Accepted", String::from("{\"wait\":true}")),
            Outcome::Unknown => reply("404 Not Found", error_json("no such pairing")),
        },
    }
}

fn serve(mut stream: TcpStream, shared: &Shared, trusted: &AtomicBool) {
    let local = stream.peer_addr().map(|a| match a.ip() {
        IpAddr::V4(ip) => ip.is_loopback(),
        IpAddr::V6(ip) => ip.is_loopback() || ip.to_ipv4_mapped().is_some_and(|v4| v4.is_loopback()),
    });
    let Ok(local) = local else { return };
    let _ = stream.set_nodelay(true);
    let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));
    let mut buf = vec![0u8; REQUEST_LIMIT];
    let mut len = 0;
    let accepted = Instant::now();
    let mut idle_since = accepted;
    let mut request_started: Option<Instant> = None;
    loop {
        let deadline = match request_started {
            _ if !trusted.load(Ordering::Relaxed) => accepted + FIRST_REQUEST,
            Some(t) => t + REQUEST_DEADLINE,
            None => idle_since + IDLE_TIMEOUT,
        };
        let Some(left) = deadline.checked_duration_since(Instant::now()).filter(|d| !d.is_zero()) else { return };
        let _ = stream.set_read_timeout(Some(left));
        match stream.read(&mut buf[len..]) {
            Ok(0) | Err(_) => return,
            Ok(n) => {
                len += n;
                request_started.get_or_insert_with(Instant::now);
            }
        }
        while let Some(end) = buf[..len].windows(4).position(|w| w == b"\r\n\r\n") {
            let (reply, head_only) = match parse(&buf[..end]) {
                Some(request) => (respond(&request, shared, local), request.method == "HEAD"),
                None => (Reply { status: "400 Bad Request", body: error_json("bad request").into_bytes(), sealed: false, trusted: false }, false),
            };
            let head = format!(
                "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: no-store\r\n{}Connection: {}\r\n\r\n",
                reply.status,
                if reply.sealed { "application/octet-stream" } else { "application/json; charset=utf-8" },
                reply.body.len(),
                if reply.status.starts_with("401") { "WWW-Authenticate: Bearer\r\n" } else { "" },
                if reply.trusted { "keep-alive" } else { "close" },
            );
            if stream.write_all(head.as_bytes()).is_err() || (!head_only && stream.write_all(&reply.body).is_err()) || !reply.trusted {
                return;
            }
            trusted.store(true, Ordering::Relaxed);
            buf.copy_within(end + 4..len, 0);
            len -= end + 4;
            request_started = if len > 0 { Some(Instant::now()) } else { None };
            idle_since = Instant::now();
        }
        if len == buf.len() {
            let body = "{\"error\":\"request too large\"}";
            let _ = stream.write_all(
                format!(
                    "HTTP/1.1 431 Request Header Fields Too Large\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .as_bytes(),
            );
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: &str = "0123456789abcdef0123456789abcdef";
    const AUTH: &str = "Authorization: Bearer 0123456789abcdef0123456789abcdef";
    const NONCE: &str = "00112233445566778899aabbccddeeff";

    fn test_settings() -> Arc<Settings> {
        let path = std::env::temp_dir().join(format!("deskstats-http-{}-{:?}", std::process::id(), std::thread::current().id()));
        let _ = std::fs::remove_file(&path);
        Arc::new(Settings::load(path.to_string_lossy().into_owned()))
    }

    #[test]
    fn config_changes_and_returns_the_settings() {
        let s = shared();
        let reply = respond(&parse(sealed(&s, "/config?interval_ms=2000&lan=1").as_bytes()).unwrap(), &s, true);
        assert_eq!(opened(&s, &reply.body), ("200".into(), r#"{"interval_ms":2000,"lan":true,"alerts":true,"weather_min":60}"#.into()));
        let _ = std::fs::remove_file(s.settings.path());
    }

    fn starting() -> Shared {
        Shared::new(TOKEN.into(), 8787, Arc::new(Weather::new(Duration::from_secs(3600))), test_settings())
    }

    fn shared() -> Shared {
        let s = starting();
        s.settings.lan.store(true, Ordering::Relaxed);
        *s.sample.lock().unwrap() =
            Some(Sample { head: "{\"ts\":1,".into(), alerts: "\"alerts\":[1],".into(), alerts_full: "\"alerts\":[1,2],".into() });
        s
    }

    fn ask(head: &str, s: &Shared, local: bool) -> &'static str {
        respond(&parse(head.as_bytes()).unwrap(), s, local).status
    }

    fn sealed(s: &Shared, target: &str) -> String {
        format!("GET /q?{} HTTP/1.1\r\nHost: x", crate::crypto::hex(&s.keys.seal(format!("Q\n{NONCE}\n{target}").as_bytes()).unwrap()))
    }

    fn opened(s: &Shared, body: &[u8]) -> (String, String) {
        assert_eq!(body.len() % 16, 0);
        let text = String::from_utf8(s.keys.open(body).unwrap()).unwrap();
        assert_eq!(text.len() % PAD, 0);
        let mut parts = text.splitn(4, '\n');
        assert_eq!((parts.next(), parts.next()), (Some("A"), Some(NONCE)));
        (parts.next().unwrap().to_string(), parts.next().unwrap().trim_end().to_string())
    }

    #[test]
    fn plain_json_only_on_this_mac_with_the_token() {
        let s = shared();
        assert_eq!(ask(&format!("GET /stats HTTP/1.1\r\nHost: 127.0.0.1:8787\r\n{AUTH}"), &s, true), "200 OK");
        assert_eq!(ask(&format!("GET /stats HTTP/1.1\r\nHost: localhost:8787\r\n{}", AUTH.replace("Bearer", "bearer")), &s, true), "200 OK");
        assert_eq!(ask("GET /stats HTTP/1.1\r\nHost: 127.0.0.1:8787", &s, true), "401 Unauthorized");
        assert_eq!(ask("GET /stats?token=0123456789abcdef0123456789abcdef HTTP/1.1\r\nHost: 127.0.0.1:8787", &s, true), "401 Unauthorized");
        assert_eq!(ask("GET /stats HTTP/1.1\r\nHost: 127.0.0.1:8787\r\nAuthorization: Bearer wrong!", &s, true), "401 Unauthorized");
        assert_eq!(ask(&format!("GET /stats HTTP/1.1\r\nHost: evil.example:8787\r\n{AUTH}"), &s, true), "421 Misdirected Request");
        assert_eq!(ask(&format!("GET /stats HTTP/1.1\r\n{AUTH}"), &s, true), "421 Misdirected Request");
        assert_eq!(ask(&format!("GET /stats HTTP/1.1\r\nHost: 127.0.0.1:8787\r\nOrigin: http://evil.example\r\n{AUTH}"), &s, true), "403 Forbidden");
        assert_eq!(ask(&format!("POST /stats HTTP/1.1\r\nHost: 127.0.0.1:8787\r\n{AUTH}"), &s, true), "405 Method Not Allowed");
        assert_eq!(ask(&format!("GET /stats HTTP/1.1\r\nHost: 127.0.0.1:8787\r\nContent-Length: 5\r\n{AUTH}"), &s, true), "400 Bad Request");
        assert_eq!(ask(&format!("GET /x HTTP/1.1\r\nHost: 127.0.0.1:8787\r\n{AUTH}"), &s, true), "404 Not Found");
        assert_eq!(ask(&format!("GET /stats HTTP/1.1\r\nHost: 192.168.1.5:8787\r\n{AUTH}"), &s, false), "401 Unauthorized");
        assert_eq!(ask(&format!("GET /procs HTTP/1.1\r\nHost: 192.168.1.5:8787\r\n{AUTH}"), &s, false), "401 Unauthorized");
    }

    #[test]
    fn a_sealed_request_gets_a_sealed_answer() {
        let s = shared();
        let reply = respond(&parse(sealed(&s, "/stats").as_bytes()).unwrap(), &s, false);
        assert_eq!((reply.status, reply.sealed, reply.trusted), ("200 OK", true, true));
        assert_eq!(opened(&s, &reply.body), ("200".into(), "{\"ts\":1,\"alerts\":[1],\"weather\":null}".into()));
        let reply = respond(&parse(sealed(&s, "/stats?alerts=full").as_bytes()).unwrap(), &s, false);
        assert_eq!(opened(&s, &reply.body).1, "{\"ts\":1,\"alerts\":[1,2],\"weather\":null}");
        let reply = respond(&parse(sealed(&s, "/nothing").as_bytes()).unwrap(), &s, false);
        assert_eq!(opened(&s, &reply.body).0, "404");
        let reply = respond(&parse(sealed(&starting(), "/stats").as_bytes()).unwrap(), &starting(), true);
        assert_eq!(opened(&s, &reply.body).0, "503");
    }

    #[test]
    fn what_is_not_sealed_with_the_token_does_nothing() {
        let s = shared();
        let other = Shared::new("ffffffffffffffffffffffffffffffff".into(), 8787, Arc::new(Weather::new(Duration::from_secs(3600))), test_settings());
        for head in [sealed(&other, "/stats?place=1.00,2.00&alerts=full"), "GET /q?00 HTTP/1.1".into(), "GET /q HTTP/1.1".into(), "GET /stats?alerts=full HTTP/1.1".into()] {
            let reply = respond(&parse(head.as_bytes()).unwrap(), &s, false);
            assert_eq!((reply.status, reply.trusted), ("401 Unauthorized", false));
        }
        assert_eq!(s.last_request_ms.load(Ordering::Relaxed), 0);
        assert_eq!(ask(&format!("{}\r\nOrigin: http://x", sealed(&s, "/stats")), &s, false), "403 Forbidden");
    }

    #[test]
    fn places_and_parameters() {
        let s = shared();
        assert_eq!((param("place=42.87,74.59&alerts=full", "place"), param("place=1,2&alerts=full", "alerts"), param("a=1", "x")), (Some("42.87,74.59"), Some("full"), None));
        let weather = |target: &str| opened(&s, &respond(&parse(sealed(&s, target).as_bytes()).unwrap(), &s, false).body).1;
        assert!(weather("/stats").ends_with("\"weather\":null}"));
        assert!(weather("/stats?place=91,0").ends_with("\"weather\":null}"));
        assert!(weather("/stats?place=abc").ends_with("\"weather\":null}"));
        assert!(weather("/stats?place=42.87,74.59").ends_with("\"weather\":null}"));
    }

    fn allow(_: &str, _: &str, _: &str) -> bool {
        true
    }

    fn deny(_: &str, _: &str, _: &str) -> bool {
        false
    }

    fn reply(s: &Shared, head: &str) -> (&'static str, String) {
        let r = respond(&parse(head.as_bytes()).unwrap(), s, false);
        (r.status, String::from_utf8(r.body).unwrap())
    }

    #[test]
    fn info_only_for_the_app_and_by_address() {
        let mut s = shared();
        s.about = "{\"app\":\"deskstats\"}".into();
        assert_eq!(reply(&s, "GET /info HTTP/1.1\r\nHost: 192.168.1.5:8787\r\nX-Deskstats: 1"), ("200 OK", s.about.clone()));
        assert_eq!(reply(&s, "GET /info HTTP/1.1\r\nHost: 192.168.1.5:8787").0, "403 Forbidden");
        assert_eq!(reply(&s, "GET /info HTTP/1.1\r\nHost: evil.example:8787\r\nX-Deskstats: 1").0, "403 Forbidden");
        assert_eq!(reply(&s, "GET /info HTTP/1.1\r\nX-Deskstats: 1").0, "403 Forbidden");
        assert_eq!(reply(&s, "GET /info HTTP/1.1\r\nHost: 192.168.1.5\r\nX-Deskstats: 1\r\nOrigin: http://x").0, "403 Forbidden");
    }

    #[test]
    fn wifi_off_refuses_sealed_requests_from_the_network() {
        let s = shared();
        assert_eq!(ask(&sealed(&s, "/stats"), &s, false), "200 OK");
        s.settings.lan.store(false, Ordering::Relaxed);
        assert_eq!(ask(&sealed(&s, "/stats"), &s, false), "403 Forbidden");
        assert_eq!(ask(&sealed(&s, "/stats"), &s, true), "200 OK");
    }

    fn pair_once(s: &Shared) -> (crate::pair::EcKey, &'static str, String) {
        let phone = crate::pair::EcKey::new().unwrap();
        let head = format!("GET /pair?k={}&n=Galaxy+J7&l=ru HTTP/1.1\r\nHost: 192.168.1.5:8787\r\nX-Deskstats: 1", crate::crypto::hex(&phone.public));
        let (status, body) = reply(s, &head);
        assert_eq!(status, "200 OK", "{body}");
        let started = |key: &str| body.split(&format!("\"{key}\":\"")).nth(1).unwrap().split('"').next().unwrap().to_string();
        let mac_key = crate::crypto::unhex(&started("k")).unwrap();
        let session = started("s");
        let secret = phone.shared(&mac_key).unwrap();
        let mut answer = reply(s, &format!("GET /pair/wait?s={session} HTTP/1.1\r\nHost: 192.168.1.5:8787\r\nX-Deskstats: 1"));
        while answer.0 == "202 Accepted" {
            answer = reply(s, &format!("GET /pair/wait?s={session} HTTP/1.1\r\nHost: 192.168.1.5:8787\r\nX-Deskstats: 1"));
        }
        let token = answer.1.strip_prefix("{\"t\":\"").and_then(|t| t.strip_suffix("\"}")).map(|t| {
            String::from_utf8(Keys::for_pairing(&secret).open(&crate::crypto::unhex(t).unwrap()).unwrap()).unwrap()
        });
        (phone, answer.0, token.unwrap_or(answer.1))
    }

    #[test]
    fn pairing_over_wifi_hands_over_the_token_and_turns_wifi_on() {
        let mut s = shared();
        s.settings.lan.store(false, Ordering::Relaxed);
        s.ask_user = allow;
        let (_, status, token) = pair_once(&s);
        assert_eq!((status, token.as_str()), ("200 OK", TOKEN));
        assert!(s.settings.lan.load(Ordering::Relaxed));
        let mut s = shared();
        s.settings.lan.store(false, Ordering::Relaxed);
        s.ask_user = deny;
        let (_, status, _) = pair_once(&s);
        assert_eq!(status, "403 Forbidden");
        assert!(!s.settings.lan.load(Ordering::Relaxed));
        assert_eq!(reply(&s, &format!("GET /pair?k={} HTTP/1.1\r\nHost: 192.168.1.5:8787\r\nX-Deskstats: 1", "04".repeat(65))).0, "429 Too Many Requests");
        assert_eq!(reply(&shared(), "GET /pair?k=00 HTTP/1.1\r\nHost: 192.168.1.5:8787\r\nX-Deskstats: 1").0, "400 Bad Request");
        assert_eq!(reply(&s, "GET /pair/wait?s=x HTTP/1.1\r\nHost: 192.168.1.5:8787\r\nX-Deskstats: 1").0, "404 Not Found");
    }

    fn server() -> (Arc<Shared>, std::net::SocketAddr) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let s = Arc::new(shared());
        let worker = s.clone();
        std::thread::spawn(move || run(listener, worker));
        (s, address)
    }

    fn read_answer(stream: &mut TcpStream) -> (String, Vec<u8>) {
        let mut data = Vec::new();
        let mut byte = [0u8; 1];
        while !data.ends_with(b"\r\n\r\n") {
            assert_eq!(stream.read(&mut byte).unwrap(), 1, "connection closed inside the head");
            data.push(byte[0]);
        }
        let head = String::from_utf8(data).unwrap();
        let length: usize = head.lines().find_map(|l| l.strip_prefix("Content-Length: ")).unwrap().parse().unwrap();
        let mut body = vec![0u8; length];
        stream.read_exact(&mut body).unwrap();
        (head.lines().next().unwrap().to_string(), body)
    }

    #[test]
    fn serves_over_a_socket_and_drops_strangers() {
        let (s, address) = server();
        let mut phone = TcpStream::connect(address).unwrap();
        phone.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        for _ in 0..2 {
            let request = format!("{}\r\n\r\n", sealed(&s, "/stats"));
            let (first, second) = request.split_at(20);
            phone.write_all(first.as_bytes()).unwrap();
            phone.write_all(second.as_bytes()).unwrap();
            let (status, body) = read_answer(&mut phone);
            assert_eq!(status, "HTTP/1.1 200 OK");
            assert_eq!(opened(&s, &body).0, "200");
        }
        let mut stranger = TcpStream::connect(address).unwrap();
        stranger.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        stranger.write_all(b"GET /q?00 HTTP/1.1\r\n\r\nGET /q?00 HTTP/1.1\r\n\r\n").unwrap();
        assert_eq!(read_answer(&mut stranger).0, "HTTP/1.1 401 Unauthorized");
        assert_eq!(stranger.read(&mut [0u8; 1]).unwrap_or(0), 0);
        let mut flood = TcpStream::connect(address).unwrap();
        flood.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        flood.write_all(&vec![b'a'; REQUEST_LIMIT]).unwrap();
        assert_eq!(read_answer(&mut flood).0, "HTTP/1.1 431 Request Header Fields Too Large");
    }

    #[test]
    fn silent_connections_do_not_keep_the_phone_out() {
        let (s, address) = server();
        let started = Instant::now();
        let mut idle: Vec<TcpStream> = (0..MAX_CONNECTIONS + 4).map(|_| TcpStream::connect(address).unwrap()).collect();
        let mut phone = TcpStream::connect(address).unwrap();
        phone.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        phone.write_all(format!("{}\r\n\r\n", sealed(&s, "/stats")).as_bytes()).unwrap();
        assert_eq!(read_answer(&mut phone).0, "HTTP/1.1 200 OK");
        let last = idle.last_mut().unwrap();
        last.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        assert_eq!(last.read(&mut [0u8; 1]).unwrap_or(0), 0);
        assert!(started.elapsed() < FIRST_REQUEST + Duration::from_secs(3));
        phone.write_all(format!("{}\r\n\r\n", sealed(&s, "/stats")).as_bytes()).unwrap();
        assert_eq!(read_answer(&mut phone).0, "HTTP/1.1 200 OK");
    }
}
