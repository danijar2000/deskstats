//! A tiny HTTP/1.1 server for the phone.
//!
//! The phone asks `GET /q?<hex>`, over USB (adb reverse) and over Wi-Fi alike. The hex is a
//! sealed text (see crypto.rs) `Q\n<nonce>\n<target>`, where the target is one of
//!
//!   /stats                      the latest sample (every poll)
//!   /stats?place=<lat>,<lon>    with the weather of that place instead of the startup city
//!   /stats?alerts=full          with the long list of notifications instead of the newest few
//!   /procs?sort=cpu|ram|net|gpu the heaviest applications (only while the phone shows them)
//!
//! and the answer is the sealed text `A\n<nonce>\n<status>\n<json>`, padded with spaces to a
//! multiple of a kilobyte. So the token never travels, a request that was not sealed with it
//! does nothing at all, an old answer cannot be replayed (the nonce is the phone's), and an
//! onlooker learns neither what is asked nor, from the size, when a notification arrived.
//!
//! For a look by hand, the same targets are served as plain JSON to this Mac only, with
//! `Authorization: Bearer <token>`.

use crate::crypto::Keys;
use crate::procs::Tracker;
use crate::util::{log, now_ms};
use crate::weather::Weather;
use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const MAX_CONNECTIONS: usize = 16;
const REQUEST_LIMIT: usize = 4096;
/// A connection must prove itself with a first valid request within this time.
const FIRST_REQUEST: Duration = Duration::from_secs(3);
/// A started request must arrive whole within this time.
const REQUEST_DEADLINE: Duration = Duration::from_secs(10);
/// An idle keep-alive connection is closed after this time.
const IDLE_TIMEOUT: Duration = Duration::from_secs(30);
/// Sealed answers grow in steps of this many bytes.
const PAD: usize = 1024;

/// The latest sample as JSON in pieces: the request decides which list of notifications and
/// which weather complete it.
pub struct Sample {
    /// Everything up to the notifications, ending with a comma.
    pub head: String,
    /// `"alerts":[…],` with the newest few, `"alerts":null,`, or nothing when they are off.
    pub alerts: String,
    /// The same with the long list.
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
    /// Set once the connection sent a request with the right token or seal.
    trusted: Arc<AtomicBool>,
    since: Instant,
}

pub struct Shared {
    /// None until the first sample and while sampling is paused.
    pub sample: Mutex<Option<Sample>>,
    pub weather: Arc<Weather>,
    /// Per-application load; empty unless the phone is looking at it.
    pub procs: Tracker,
    /// When the phone last asked for anything, over USB or Wi-Fi (ms since the epoch). The
    /// sampler pauses and adb looks for a phone when this gets old.
    pub last_request_ms: AtomicU64,
    /// The same for requests from this Mac, i.e. a phone on USB through adb reverse.
    pub last_local_ms: AtomicU64,
    pub token: String,
    pub keys: Keys,
    pub port: u16,
    connections: Mutex<Vec<Connection>>,
    next_id: AtomicU64,
    /// This Mac's addresses on its own networks and when they were read.
    addresses: Mutex<(Option<Instant>, Vec<Ipv4Addr>)>,
}

impl Shared {
    pub fn new(token: String, port: u16, weather: Arc<Weather>) -> Self {
        let keys = Keys::from_token(&token);
        Shared {
            sample: Mutex::new(None),
            weather,
            procs: Tracker::default(),
            last_request_ms: AtomicU64::new(0),
            last_local_ms: AtomicU64::new(0),
            token,
            keys,
            port,
            connections: Mutex::new(Vec::new()),
            next_id: AtomicU64::new(0),
            addresses: Mutex::new((None, Vec::new())),
        }
    }

    /// With --lan the socket listens on every interface, but only connections that came in
    /// through this Mac's own networks (en*, where the beacon goes) are served — not a VPN,
    /// a phone's hotspot or the like.
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

    /// Makes room for one more connection; false when all are busy with a known phone. A
    /// connection that has not proved itself yet gives way first, so idle strangers cannot
    /// keep the panel out.
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
    /// The body is sealed rather than plain JSON.
    pub sealed: bool,
    /// The request carried the right token or seal.
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
    /// The request announces a body, which this server never reads.
    pub body: bool,
}

/// Parses the request head (everything before the blank line).
pub fn parse(head: &[u8]) -> Option<Request<'_>> {
    let text = std::str::from_utf8(head).ok()?;
    let mut lines = text.split("\r\n");
    let mut first = lines.next()?.split(' ');
    let method = first.next()?;
    let target = first.next()?;
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let mut request = Request { method, path, query, host: None, origin: false, authorization: None, body: false };
    for line in lines {
        let Some((name, value)) = line.split_once(':') else { continue };
        let value = value.trim();
        if name.eq_ignore_ascii_case("host") {
            request.host = Some(value);
        } else if name.eq_ignore_ascii_case("origin") {
            request.origin = true;
        } else if name.eq_ignore_ascii_case("authorization") {
            request.authorization = Some(value);
        } else if name.eq_ignore_ascii_case("transfer-encoding") || (name.eq_ignore_ascii_case("content-length") && value != "0") {
            request.body = true;
        }
    }
    Some(request)
}

/// The token from `Authorization: Bearer <token>` (the scheme in any letter case).
fn token_of<'a>(request: &Request<'a>) -> Option<&'a str> {
    let value = request.authorization?;
    let (scheme, token) = value.split_once(' ')?;
    scheme.eq_ignore_ascii_case("bearer").then(|| token.trim())
}

/// The value of a query parameter, as sent (the phone sends nothing that needs decoding).
pub fn param<'a>(query: &'a str, name: &str) -> Option<&'a str> {
    query.split('&').filter_map(|pair| pair.split_once('=')).find(|(key, _)| *key == name).map(|(_, value)| value)
}

fn error_json(text: &str) -> String {
    format!("{{\"error\":\"{text}\"}}")
}

/// What an authorised request for `target` gets: a status code and JSON.
fn answer(target: &str, shared: &Shared, local: bool) -> (u16, String) {
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    if path != "/stats" && path != "/procs" {
        return (404, error_json("not found"));
    }
    shared.last_request_ms.store(now_ms(), Ordering::Relaxed);
    if local {
        shared.last_local_ms.store(now_ms(), Ordering::Relaxed);
    }
    // While the Mac is about to sleep or the server stops, the sampler may not run any more:
    // answer the news right here, and tell the power side that the phone got it.
    let json = if crate::power::state() == "sleep" {
        Some(String::from("{\"mac\":\"sleep\"}"))
    } else if path == "/procs" {
        Some(shared.procs.json(param(query, "sort").unwrap_or("cpu")))
    } else {
        let sample = shared.sample.lock().unwrap_or_else(|e| e.into_inner());
        sample.as_ref().map(|sample| {
            // A place that cannot be read gets no weather, rather than another city's.
            let weather = match param(query, "place") {
                Some(place) => crate::weather::parse_place(place).map_or_else(|| String::from("null"), |(lat, lon)| shared.weather.place(lat, lon)),
                None => shared.weather.city(),
            };
            sample.json(param(query, "alerts") == Some("full"), &weather)
        })
    };
    let Some(json) = json else {
        // Before the first sample, and for a moment after sampling resumes.
        return (503, error_json("starting"));
    };
    crate::power::LAST_POLL_MS.store(now_ms(), Ordering::Relaxed);
    (200, json)
}

/// The nonce and target of a sealed request; None when it was not sealed with the token.
fn unseal<'a>(query: &str, keys: &Keys, plain: &'a mut String) -> Option<(&'a str, &'a str)> {
    *plain = String::from_utf8(keys.open(&crate::crypto::unhex(query)?)?).ok()?;
    let (nonce, target) = plain.strip_prefix("Q\n")?.split_once('\n')?;
    let valid = (16..=64).contains(&nonce.len()) && nonce.bytes().all(|b| b.is_ascii_alphanumeric());
    valid.then_some((nonce, target))
}

/// The reply to one request; `local` is true for a connection from this Mac.
pub fn respond(request: &Request, shared: &Shared, local: bool) -> Reply {
    let refuse = |status, text: &str| Reply { status, body: error_json(text).into_bytes(), sealed: false, trusted: false };
    // A page in a browser always sends Origin on a cross-site fetch; this server has no browser clients.
    if request.origin {
        return refuse("403 Forbidden", "browser requests are not allowed");
    }
    if request.method != "GET" && request.method != "HEAD" {
        return refuse("405 Method Not Allowed", "method not allowed");
    }
    if request.body {
        return refuse("400 Bad Request", "a request body is not accepted");
    }
    if request.path == "/q" {
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
    // Plain JSON never leaves this Mac.
    if !local {
        return refuse("401 Unauthorized", "sealed requests only");
    }
    // DNS rebinding: a foreign name pointed at 127.0.0.1 arrives with that name in Host.
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
            // Whoever sent something this server refuses is not kept on the line.
            let head = format!(
                "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: no-store\r\n{}Connection: {}\r\n\r\n",
                reply.status,
                if reply.sealed { "application/octet-stream" } else { "application/json; charset=utf-8" },
                reply.body.len(),
                // Old Android HTTP stacks fail on a 401 that carries no challenge.
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

    fn starting() -> Shared {
        Shared::new(TOKEN.into(), 8787, Arc::new(Weather::new(Duration::from_secs(3600))))
    }

    fn shared() -> Shared {
        let s = starting();
        *s.sample.lock().unwrap() =
            Some(Sample { head: "{\"ts\":1,".into(), alerts: "\"alerts\":[1],".into(), alerts_full: "\"alerts\":[1,2],".into() });
        s
    }

    fn ask(head: &str, s: &Shared, local: bool) -> &'static str {
        respond(&parse(head.as_bytes()).unwrap(), s, local).status
    }

    /// The request line of a sealed request for `target`.
    fn sealed(s: &Shared, target: &str) -> String {
        format!("GET /q?{} HTTP/1.1\r\nHost: x", crate::crypto::hex(&s.keys.seal(format!("Q\n{NONCE}\n{target}").as_bytes()).unwrap()))
    }

    /// The status and JSON inside a sealed answer; checks the nonce and the padding.
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
        // from the network even the right token gets nothing in the clear
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
        let other = Shared::new("ffffffffffffffffffffffffffffffff".into(), 8787, Arc::new(Weather::new(Duration::from_secs(3600))));
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
        // a place that cannot be read is not replaced by the startup city
        s.weather.set_city("{\"city\":\"Bishkek\"}");
        let weather = |target: &str| opened(&s, &respond(&parse(sealed(&s, target).as_bytes()).unwrap(), &s, false).body).1;
        assert!(weather("/stats").ends_with("\"weather\":{\"city\":\"Bishkek\"}}"));
        assert!(weather("/stats?place=91,0").ends_with("\"weather\":null}"));
        assert!(weather("/stats?place=abc").ends_with("\"weather\":null}"));
        assert!(weather("/stats?place=42.87,74.59").ends_with("\"weather\":null}"));
    }

    /// A server on a free local port.
    fn server() -> (Arc<Shared>, std::net::SocketAddr) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let s = Arc::new(shared());
        let worker = s.clone();
        std::thread::spawn(move || run(listener, worker));
        (s, address)
    }

    /// Reads one answer: its status line and body.
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
        // two requests on one connection, the second in two pieces
        for _ in 0..2 {
            let request = format!("{}\r\n\r\n", sealed(&s, "/stats"));
            let (first, second) = request.split_at(20);
            phone.write_all(first.as_bytes()).unwrap();
            phone.write_all(second.as_bytes()).unwrap();
            let (status, body) = read_answer(&mut phone);
            assert_eq!(status, "HTTP/1.1 200 OK");
            assert_eq!(opened(&s, &body).0, "200");
        }
        // a request without the seal is answered once and the connection is closed
        let mut stranger = TcpStream::connect(address).unwrap();
        stranger.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        stranger.write_all(b"GET /q?00 HTTP/1.1\r\n\r\nGET /q?00 HTTP/1.1\r\n\r\n").unwrap();
        assert_eq!(read_answer(&mut stranger).0, "HTTP/1.1 401 Unauthorized");
        assert_eq!(stranger.read(&mut [0u8; 1]).unwrap_or(0), 0);
        // a head that never ends
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
        // and a connection that says nothing is closed after a few seconds
        let last = idle.last_mut().unwrap();
        last.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        assert_eq!(last.read(&mut [0u8; 1]).unwrap_or(0), 0);
        assert!(started.elapsed() < FIRST_REQUEST + Duration::from_secs(3));
        // the phone's own connection stays
        phone.write_all(format!("{}\r\n\r\n", sealed(&s, "/stats")).as_bytes()).unwrap();
        assert_eq!(read_answer(&mut phone).0, "HTTP/1.1 200 OK");
    }
}
