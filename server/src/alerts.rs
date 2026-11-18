//! Notifications shown on this Mac, from two sources:
//! - the Notification Center database, which holds the notifications of Mac apps
//!   (needs Full Disk Access; macOS has no public API for other apps' notifications);
//! - the Notification Center window itself, read through Accessibility, which also shows the
//!   notifications forwarded from an iPhone — those are never written to the database.

use crate::ffi::*;
use crate::util::{json_escape, log, normalize, now_ms, truncate};
use std::collections::HashMap;
use std::cell::Cell;
use std::ffi::{c_int, c_void, CStr, CString};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::time::{Duration, Instant, SystemTime};

/// How many notifications every answer carries, and how many the phone's detail page gets.
pub const SHORT: usize = 6;
pub const FULL: usize = 30;
const TEXT_LIMIT: usize = 600;
/// Notifications older than this are not served.
const MAX_AGE_MS: u64 = 48 * 3600 * 1000;
/// Seconds between 1970-01-01 and 2001-01-01, the epoch of the database timestamps.
const MAC_EPOCH_OFFSET: f64 = 978_307_200.0;

#[derive(Clone)]
pub struct Item {
    pub id: u64,
    pub ts: u64,
    pub app: String,
    pub title: String,
    pub subtitle: String,
    pub body: String,
}

impl Item {
    pub fn json(&self) -> String {
        format!(
            "{{\"id\":{},\"ts\":{},\"app\":\"{}\",\"title\":\"{}\",\"subtitle\":\"{}\",\"body\":\"{}\"}}",
            self.id,
            self.ts,
            json_escape(&self.app),
            json_escape(&self.title),
            json_escape(&self.subtitle),
            json_escape(&self.body)
        )
    }

    fn key(&self) -> String {
        normalize(&format!("{}|{}|{}|{}", self.app, self.title, self.subtitle, self.body))
    }
}

// ---- Notification Center database -------------------------------------------------------

type Stamp = (Option<SystemTime>, Option<SystemTime>);

pub struct Database {
    path: String,
    stamp: Option<Stamp>,
    items: Option<Vec<Item>>,
    failed_at: Option<Instant>,
    app_names: HashMap<String, String>,
    key_req: *const c_void,
    key_title: *const c_void,
    key_subtitle: *const c_void,
    key_body: *const c_void,
}

impl Database {
    pub fn new() -> Self {
        let home = std::env::var("HOME").unwrap_or_default();
        Self::at(format!("{home}/Library/Group Containers/group.com.apple.usernoted/db2/db"))
    }

    fn at(path: String) -> Self {
        Database {
            path,
            stamp: None,
            items: None,
            failed_at: None,
            app_names: HashMap::new(),
            key_req: cf_string("req"),
            key_title: cf_string("titl"),
            key_subtitle: cf_string("subt"),
            key_body: cf_string("body"),
        }
    }

    /// The newest notifications, or None while the database cannot be read. It is queried again
    /// only after one of its files changed; a failed read keeps the last good list and is retried
    /// after 10 seconds.
    pub fn refresh(&mut self) -> Option<&[Item]> {
        let modified = |path: &str| std::fs::metadata(path).and_then(|m| m.modified()).ok();
        let stamp = (modified(&self.path), modified(&format!("{}-wal", self.path)));
        let retry_due = self.failed_at.is_none_or(|t| t.elapsed() >= Duration::from_secs(10));
        if self.stamp != Some(stamp) && retry_due {
            match self.query() {
                Some(items) => {
                    if self.items.is_none() {
                        log("notifications: database readable");
                    }
                    self.stamp = Some(stamp);
                    self.items = Some(items);
                    self.failed_at = None;
                }
                None => {
                    if self.failed_at.is_none() {
                        log("notifications: cannot read the database (Full Disk Access?)");
                    }
                    self.failed_at = Some(Instant::now());
                }
            }
        }
        self.items.as_deref()
    }

    fn query(&mut self) -> Option<Vec<Item>> {
        let path = CString::new(self.path.as_str()).ok()?;
        let since = (now_ms().saturating_sub(MAX_AGE_MS)) as f64 / 1000.0 - MAC_EPOCH_OFFSET;
        let sql = CString::new(format!(
            "select r.rec_id, r.delivered_date, a.identifier, r.data from record r \
             join app a on a.app_id = r.app_id where r.delivered_date > {since} \
             order by r.delivered_date desc limit {FULL}"
        ))
        .ok()?;
        let mut items = Vec::new();
        unsafe {
            let mut db: *mut c_void = std::ptr::null_mut();
            if sqlite3_open_v2(path.as_ptr(), &mut db, SQLITE_OPEN_READONLY, std::ptr::null()) != 0 {
                sqlite3_close(db);
                return None;
            }
            sqlite3_busy_timeout(db, 200);
            let mut stmt: *mut c_void = std::ptr::null_mut();
            if sqlite3_prepare_v2(db, sql.as_ptr(), -1, &mut stmt, std::ptr::null_mut()) != 0 {
                sqlite3_close(db);
                return None;
            }
            let mut rc;
            loop {
                rc = sqlite3_step(stmt);
                if rc != SQLITE_ROW {
                    break;
                }
                let id = sqlite3_column_int64(stmt, 0).max(0) as u64;
                let ts = ((sqlite3_column_double(stmt, 1) + MAC_EPOCH_OFFSET) * 1000.0).max(0.0) as u64;
                let bundle = sqlite3_column_text(stmt, 2);
                let bundle = if bundle.is_null() { String::new() } else { CStr::from_ptr(bundle).to_string_lossy().into_owned() };
                let (title, subtitle, body) = self.texts(sqlite3_column_blob(stmt, 3), sqlite3_column_bytes(stmt, 3));
                let app = self.app_name(&bundle);
                items.push(Item { id, ts, app, title, subtitle, body });
            }
            sqlite3_finalize(stmt);
            sqlite3_close(db);
            // A busy or broken step must not be cached as a complete (shorter) list.
            if rc != SQLITE_DONE {
                return None;
            }
        }
        Some(items)
    }

    /// Title, subtitle and body from the binary property list a notification is stored as.
    unsafe fn texts(&self, blob: *const u8, len: c_int) -> (String, String, String) {
        let mut out = (String::new(), String::new(), String::new());
        if blob.is_null() || len <= 0 {
            return out;
        }
        let data = CFDataCreate(std::ptr::null(), blob, len as isize);
        if data.is_null() {
            return out;
        }
        let plist = CFPropertyListCreateWithData(std::ptr::null(), data, 0, std::ptr::null_mut(), std::ptr::null_mut());
        CFRelease(data);
        if plist.is_null() {
            return out;
        }
        if CFGetTypeID(plist) == CFDictionaryGetTypeID() {
            let req = CFDictionaryGetValue(plist, self.key_req);
            if !req.is_null() && CFGetTypeID(req) == CFDictionaryGetTypeID() {
                let text = |key| cf_to_string(CFDictionaryGetValue(req, key)).map(|s| truncate(&s, TEXT_LIMIT)).unwrap_or_default();
                out = (text(self.key_title), text(self.key_subtitle), text(self.key_body));
            }
        }
        CFRelease(plist);
        out
    }

    /// "com.tinyspeck.slackmacgap" -> "Slack": the name of the installed app, else the last id part.
    fn app_name(&mut self, bundle: &str) -> String {
        if let Some(name) = self.app_names.get(bundle) {
            return name.clone();
        }
        let mut name = None;
        unsafe {
            let id = cf_string(bundle);
            if !id.is_null() {
                let urls = LSCopyApplicationURLsForBundleIdentifier(id, std::ptr::null_mut());
                CFRelease(id);
                if !urls.is_null() {
                    if CFArrayGetCount(urls) > 0 {
                        let last = CFURLCopyLastPathComponent(CFArrayGetValueAtIndex(urls, 0));
                        name = cf_to_string(last).map(|s| s.trim_end_matches(".app").to_string());
                        if !last.is_null() {
                            CFRelease(last);
                        }
                    }
                    CFRelease(urls);
                }
            }
        }
        let name = name.filter(|n| !n.is_empty()).unwrap_or_else(|| bundle.rsplit('.').next().unwrap_or(bundle).to_string());
        self.app_names.insert(bundle.to_string(), name.clone());
        name
    }
}

// ---- Notification Center window (Accessibility) -------------------------------------------

/// Ids of notifications seen on screen start here, above any database row id.
const SCREEN_ID_BASE: u64 = 1 << 40;
const SCREEN_HISTORY: usize = 20;
const MAX_NODES: usize = 800;
const MAX_DEPTH: usize = 16;
/// One reading of the window may take this long; what it does not reach is not read at all.
const WALK_DEADLINE: Duration = Duration::from_millis(200);
/// A window with more elements than this is the open Notification Center, which is read less often.
const BIG_WINDOW: usize = 150;
const BIG_WINDOW_EVERY: Duration = Duration::from_secs(3);
/// The Accessibility error of a process that does not answer in time.
const AX_CANNOT_COMPLETE: i32 = -25204;
const O_NOFOLLOW: i32 = 0x100;
/// The element tree written by --ax-dump goes to its own file, never larger than this.
const DUMP_LIMIT: u64 = 2 * 1024 * 1024;

/// A notification seen on screen: its text key, kept so it is not built again on every
/// reading, and when it was last there.
struct Seen {
    item: Item,
    key: String,
    on_screen_ms: u64,
}

pub struct Screen {
    dump: bool,
    prompted: bool,
    logged_untrusted: bool,
    trusted: bool,
    trusted_checked: Option<Instant>,
    pid: Option<c_int>,
    pid_checked: Option<Instant>,
    next_id: u64,
    history: Vec<Seen>,
    /// The notifications of `history`, rebuilt only when it changes.
    items: Vec<Item>,
    keys: AxKeys,
    last_dump: String,
    dumped: u64,
    timeout_set: bool,
    next_read: Option<Instant>,
    /// The process stopped answering in the middle of a reading.
    stuck: Cell<bool>,
}

struct AxKeys {
    windows: *const c_void,
    children: *const c_void,
    role: *const c_void,
    subrole: *const c_void,
    value: *const c_void,
    description: *const c_void,
    identifier: *const c_void,
    title: *const c_void,
}

impl Screen {
    /// `dump` writes the element tree of the Notification Center windows when it changes, to
    /// adjust the parser to a new macOS version. The tree holds notification texts, so it goes
    /// to a file of its own that only the owner can read and that starts anew on each run.
    pub fn new(dump: bool) -> Self {
        Screen {
            dump,
            prompted: false,
            logged_untrusted: false,
            trusted: false,
            trusted_checked: None,
            pid: None,
            pid_checked: None,
            next_id: SCREEN_ID_BASE,
            history: Vec::new(),
            items: Vec::new(),
            dumped: 0,
            timeout_set: false,
            next_read: None,
            stuck: Cell::new(false),
            keys: AxKeys {
                windows: cf_string("AXWindows"),
                children: cf_string("AXChildren"),
                role: cf_string("AXRole"),
                subrole: cf_string("AXSubrole"),
                value: cf_string("AXValue"),
                description: cf_string("AXDescription"),
                identifier: cf_string("AXIdentifier"),
                title: cf_string("AXTitle"),
            },
            last_dump: String::new(),
        }
    }

    /// Notifications seen on screen during the last 48 hours, newest first.
    pub fn refresh(&mut self) -> &[Item] {
        let now = now_ms();
        let before = self.history.len();
        self.history.retain(|h| now.saturating_sub(h.item.ts) < MAX_AGE_MS);
        let mut changed = self.history.len() != before;
        let due = self.next_read.is_none_or(|t| Instant::now() >= t);
        if due && self.ensure_trusted() && self.ensure_pid() {
            if let Some(found) = self.read() {
                for item in found {
                    changed |= self.take(item, now);
                }
            }
        }
        if changed {
            self.history.sort_by_key(|h| std::cmp::Reverse(h.item.ts));
            self.history.truncate(SCREEN_HISTORY);
            self.items = self.history.iter().map(|h| h.item.clone()).collect();
        }
        &self.items
    }

    /// Adds a notification read from the screen to the history; false when it is already there.
    fn take(&mut self, item: Item, now: u64) -> bool {
        // An old entry of the open Notification Center is not news.
        let age = now.saturating_sub(item.ts);
        if age >= MAX_AGE_MS {
            return false;
        }
        let key = item.key();
        if let Some(seen) = self.history.iter_mut().filter(|h| h.key == key).max_by_key(|h| h.item.ts) {
            // The same text again is a new notification only if it had left the screen and is
            // surely newer: a label like "2h ago" may be up to its own length off.
            let gone = now.saturating_sub(seen.on_screen_ms) > 10_000;
            let newer = item.ts.saturating_sub(age) > seen.item.ts + 60_000;
            if !(gone && newer) {
                seen.on_screen_ms = now;
                return false;
            }
        }
        self.history.push(Seen { item: Item { id: self.next_id, ..item }, key, on_screen_ms: now });
        self.next_id += 1;
        true
    }

    /// Asks macOS once for the Accessibility permission (it shows its own dialog), then checks
    /// every 30 seconds whether it was granted.
    fn ensure_trusted(&mut self) -> bool {
        if self.trusted {
            return true;
        }
        if self.trusted_checked.is_some_and(|t| t.elapsed() < Duration::from_secs(30)) {
            return false;
        }
        self.trusted_checked = Some(Instant::now());
        unsafe {
            let options = if self.prompted {
                std::ptr::null()
            } else {
                self.prompted = true;
                let key = cf_string("AXTrustedCheckOptionPrompt");
                let keys = [key];
                let values = [kCFBooleanTrue];
                let dict = CFDictionaryCreate(
                    std::ptr::null(),
                    keys.as_ptr(),
                    values.as_ptr(),
                    1,
                    &kCFTypeDictionaryKeyCallBacks,
                    &kCFTypeDictionaryValueCallBacks,
                );
                CFRelease(key);
                dict
            };
            self.trusted = AXIsProcessTrustedWithOptions(options) != 0;
            if !options.is_null() {
                CFRelease(options);
            }
        }
        if self.trusted {
            log("notifications: Accessibility granted, reading the Notification Center window");
        } else if std::env::var_os(RESTARTED).is_none() && granted_to_a_fresh_process() {
            // macOS keeps telling a running process the answer it got at its start. A fresh
            // copy of this program sees the grant, so start over in place — once, in case the
            // two answers differ for another reason.
            log("notifications: Accessibility was granted, restarting to pick it up");
            use std::os::unix::process::CommandExt;
            if let Ok(program) = std::env::current_exe() {
                let failed = std::process::Command::new(program).args(std::env::args_os().skip(1)).env(RESTARTED, "1").exec();
                log(&format!("notifications: cannot restart: {failed}"));
            }
        } else if !self.logged_untrusted {
            self.logged_untrusted = true;
            log("notifications: no Accessibility permission, iPhone notifications are not read");
        }
        self.trusted
    }

    fn ensure_pid(&mut self) -> bool {
        if self.pid.is_some() {
            return true;
        }
        if self.pid_checked.is_some_and(|t| t.elapsed() < Duration::from_secs(30)) {
            return false;
        }
        self.pid_checked = Some(Instant::now());
        self.pid = find_process("NotificationCenter");
        self.pid.is_some()
    }

    fn read(&mut self) -> Option<Vec<Item>> {
        let pid = self.pid?;
        let mut found = Vec::new();
        let mut dump = String::new();
        let mut budget = MAX_NODES;
        self.stuck.set(false);
        unsafe {
            if !self.timeout_set {
                // On the system-wide element the timeout applies to every element of this
                // process; the default is several seconds per call.
                self.timeout_set = true;
                let system = AXUIElementCreateSystemWide();
                if !system.is_null() {
                    AXUIElementSetMessagingTimeout(system, 0.25);
                    CFRelease(system);
                }
            }
            let app = AXUIElementCreateApplication(pid);
            if app.is_null() {
                return None;
            }
            AXUIElementSetMessagingTimeout(app, 0.25);
            let mut windows: *const c_void = std::ptr::null();
            let rc = AXUIElementCopyAttributeValue(app, self.keys.windows, &mut windows);
            CFRelease(app);
            if rc != AX_SUCCESS {
                // The process restarted (new pid) or does not answer; look it up again later.
                if !windows.is_null() {
                    CFRelease(windows);
                }
                self.pid = None;
                return None;
            }
            if !windows.is_null() {
                if CFGetTypeID(windows) == CFArrayGetTypeID() {
                    let deadline = Instant::now() + WALK_DEADLINE;
                    for i in 0..CFArrayGetCount(windows) {
                        self.walk(CFArrayGetValueAtIndex(windows, i), 0, &mut budget, deadline, &mut found, &mut dump);
                    }
                }
                CFRelease(windows);
            }
        }
        // The open Notification Center is hundreds of elements; reading it every second would
        // keep both this server and the Notification Center busy.
        self.next_read = (MAX_NODES - budget > BIG_WINDOW).then(|| Instant::now() + BIG_WINDOW_EVERY);
        if self.stuck.get() {
            // Not answering: look the process up again later instead of asking it every second.
            self.pid = None;
        }
        if self.dump && dump != self.last_dump {
            self.write_dump(&dump);
            self.last_dump = dump;
        }
        Some(found)
    }

    fn write_dump(&mut self, dump: &str) {
        if dump.is_empty() || self.dumped >= DUMP_LIMIT {
            return;
        }
        let path = format!("{}/Library/Logs/deskstats-ax.log", std::env::var("HOME").unwrap_or_default());
        let mut options = std::fs::OpenOptions::new();
        // The first dump of a run makes a new file in place of whatever is there, a planted link
        // included; the next ones are appended, never through a link.
        if self.dumped == 0 {
            let _ = std::fs::remove_file(&path);
        }
        options.write(true).mode(0o600).custom_flags(O_NOFOLLOW).create_new(self.dumped == 0).append(self.dumped > 0);
        let Ok(mut file) = options.open(&path) else { return };
        if self.dumped == 0 {
            log(&format!("ax dump: writing the element tree to {path}"));
        }
        let text = format!("--- {}\n{dump}", now_ms());
        if file.write_all(text.as_bytes()).is_ok() {
            self.dumped += text.len() as u64;
        }
    }

    unsafe fn attr(&self, element: *const c_void, key: *const c_void) -> *const c_void {
        let mut value: *const c_void = std::ptr::null();
        let rc = AXUIElementCopyAttributeValue(element, key, &mut value);
        if rc != AX_SUCCESS {
            if rc == AX_CANNOT_COMPLETE {
                self.stuck.set(true);
            }
            if !value.is_null() {
                CFRelease(value);
            }
            return std::ptr::null();
        }
        value
    }

    unsafe fn attr_string(&self, element: *const c_void, key: *const c_void) -> String {
        let value = self.attr(element, key);
        if value.is_null() {
            return String::new();
        }
        let out = cf_to_string(value).unwrap_or_default();
        CFRelease(value);
        out
    }

    unsafe fn walk(&self, element: *const c_void, depth: usize, budget: &mut usize, deadline: Instant, found: &mut Vec<Item>, dump: &mut String) {
        if *budget == 0 || depth > MAX_DEPTH || self.stuck.get() || Instant::now() >= deadline {
            return;
        }
        *budget -= 1;
        let role = self.attr_string(element, self.keys.role);
        let subrole = self.attr_string(element, self.keys.subrole);
        if self.dump {
            let short = |s: String| truncate(&s.replace('\n', " "), 60);
            dump.push_str(&format!(
                "{}{role}/{subrole} id={} title={} desc={} value={}\n",
                "  ".repeat(depth),
                short(self.attr_string(element, self.keys.identifier)),
                short(self.attr_string(element, self.keys.title)),
                short(self.attr_string(element, self.keys.description)),
                short(self.attr_string(element, self.keys.value)),
            ));
        }
        let notification = subrole.starts_with("AXNotificationCenter") && role != "AXWindow";
        let children = self.attr(element, self.keys.children);
        if children.is_null() {
            return;
        }
        if CFGetTypeID(children) == CFArrayGetTypeID() {
            // A banner, and a collapsed stack too, holds the texts of its (top) notification as
            // direct children; the other notifications of an expanded stack are nested banners.
            let mut texts = Vec::new();
            for i in 0..CFArrayGetCount(children) {
                // A process that stopped answering costs a quarter of a second per question.
                if self.stuck.get() || Instant::now() >= deadline {
                    break;
                }
                let child = CFArrayGetValueAtIndex(children, i);
                if notification && self.attr_string(child, self.keys.role) == "AXStaticText" {
                    *budget = budget.saturating_sub(1);
                    let value = self.attr_string(child, self.keys.value);
                    if !value.trim().is_empty() {
                        texts.push((self.attr_string(child, self.keys.identifier), value.trim().to_string()));
                    }
                    continue;
                }
                self.walk(child, depth + 1, budget, deadline, found, dump);
            }
            if notification {
                let description = self.attr_string(element, self.keys.description);
                if let Some(item) = item_from_texts(&texts, &description, now_ms()) {
                    found.push(item);
                }
            }
        }
        CFRelease(children);
    }
}

/// Set in the environment of a server that restarted itself for the Accessibility permission.
const RESTARTED: &str = "DESKSTATS_AX_RESTARTED";

/// Whether this process may use Accessibility, without asking the user (`--ax-check`).
pub fn accessibility_granted() -> bool {
    unsafe { AXIsProcessTrustedWithOptions(std::ptr::null()) != 0 }
}

/// The same, asked by a new process of this program.
fn granted_to_a_fresh_process() -> bool {
    let Ok(program) = std::env::current_exe() else { return false };
    crate::util::run(&program.to_string_lossy(), &["--ax-check"], Duration::from_secs(5)).is_some_and(|(_, out)| out.trim() == "yes")
}

/// How long ago a relative time label means, in milliseconds: "now", "8m ago", "2 ч назад"…
/// None when the text is not such a label. Only whole known units count, so a line of the
/// notification itself ("2 messages", "15 декабря") is not taken for a time.
pub fn parse_age(text: &str) -> Option<u64> {
    let t = text.trim().to_lowercase();
    if matches!(t.as_str(), "now" | "сейчас" | "только что" | "just now") {
        return Some(0);
    }
    if t == "yesterday" || t == "вчера" {
        return Some(24 * 3600 * 1000);
    }
    let digits: String = t.chars().take_while(|c| c.is_ascii_digit()).collect();
    let n: u64 = digits.parse().ok()?;
    let rest = t[digits.len()..].trim();
    let rest = rest.strip_suffix("ago").or_else(|| rest.strip_suffix("назад")).unwrap_or(rest).trim();
    let unit = match rest.trim_end_matches('.') {
        "s" | "sec" | "secs" | "с" | "сек" => 1000,
        "m" | "min" | "mins" | "м" | "мин" => 60 * 1000,
        "h" | "hr" | "hrs" | "ч" | "час" => 3600 * 1000,
        "d" | "д" | "дн" => 24 * 3600 * 1000,
        "w" | "wk" | "нед" => 7 * 24 * 3600 * 1000,
        _ => return None,
    };
    n.checked_mul(unit)
}

/// A notification from the static texts of its on-screen element. macOS marks them with the
/// identifiers title, subtitle and body; the app name is only in the element's description
/// ("App, title, subtitle, body"). An unmarked relative time label ("29m ago") dates it; without
/// one it counts as just arrived. Unmarked texts fall back to their order: heading, title, body.
pub fn item_from_texts(texts: &[(String, String)], description: &str, now: u64) -> Option<Item> {
    let mut age = None;
    let (mut title, mut subtitle, mut body) = (String::new(), String::new(), Vec::new());
    let mut loose = Vec::new();
    for (id, text) in texts {
        match id.as_str() {
            "title" => title = truncate(text, TEXT_LIMIT),
            "subtitle" => subtitle = truncate(text, TEXT_LIMIT),
            "body" => body.push(text.clone()),
            _ => match parse_age(text) {
                Some(a) if age.is_none() => age = Some(a),
                _ => loose.push(truncate(text, TEXT_LIMIT)),
            },
        }
    }
    let ts = now.saturating_sub(age.unwrap_or(0));
    if title.is_empty() && subtitle.is_empty() && body.is_empty() {
        // No identifiers (another macOS version): heading, title, then the rest as the body.
        let mut lines = loose.into_iter();
        let app = lines.next()?;
        let title = lines.next().unwrap_or_default();
        let body = truncate(&lines.collect::<Vec<_>>().join(" "), TEXT_LIMIT);
        return Some(Item { id: 0, ts, app, title, subtitle: String::new(), body });
    }
    body.extend(loose);
    let body = truncate(&body.join(" "), TEXT_LIMIT);
    // The description starts with the app name, unless the title comes first (as for a
    // notification forwarded from an iPhone, whose title is the app's name).
    let first = description.split(", ").next().unwrap_or("").trim().to_string();
    let app = if first.is_empty() || first == title || first == subtitle || body.starts_with(&first) {
        std::mem::take(&mut title)
    } else {
        first
    };
    Some(Item { id: 0, ts, app, title, subtitle, body })
}

/// Database notifications plus those seen only on screen (the iPhone ones), newest first;
/// none older than 48 hours, whatever the sources still hold.
pub fn merge(database: &[Item], screen: &[Item], now: u64) -> Vec<Item> {
    let recent = |i: &&Item| now.saturating_sub(i.ts) < MAX_AGE_MS;
    let mut out: Vec<Item> = database.iter().filter(recent).cloned().collect();
    for s in screen.iter().filter(recent) {
        let text = normalize(&format!("{} {} {} {}", s.app, s.title, s.subtitle, s.body));
        let duplicate = database.iter().any(|d| {
            // The screen copy holds every text of the database row, though not always in the
            // same fields. A few matching characters ("OK") prove nothing, unless the two
            // also arrived together.
            let mut matched = 0;
            let mut part = |x: &str| {
                let head: String = normalize(x).chars().take(40).collect();
                matched += head.chars().count();
                head.is_empty() || text.contains(&head)
            };
            let all = part(&d.title) && part(&d.subtitle) && part(&d.body);
            all && (matched >= 8 || (matched > 0 && d.ts.abs_diff(s.ts) <= 120_000))
        });
        if !duplicate {
            out.push(s.clone());
        }
    }
    out.sort_by_key(|i| std::cmp::Reverse(i.ts));
    out.truncate(FULL);
    out
}

fn find_process(name: &str) -> Option<c_int> {
    unsafe {
        let count = proc_listallpids(std::ptr::null_mut(), 0);
        if count <= 0 {
            return None;
        }
        let mut pids = vec![0 as c_int; count as usize + 32];
        let bytes = (pids.len() * std::mem::size_of::<c_int>()) as c_int;
        let got = proc_listallpids(pids.as_mut_ptr() as *mut c_void, bytes);
        if got <= 0 {
            return None;
        }
        for &pid in pids.iter().take(got as usize) {
            let mut buf = [0u8; 64];
            if proc_name(pid, buf.as_mut_ptr() as *mut c_void, buf.len() as u32) > 0
                && CStr::from_bytes_until_nul(&buf).ok().map(|c| c.to_bytes()) == Some(name.as_bytes())
            {
                return Some(pid);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ages() {
        assert_eq!(parse_age("now"), Some(0));
        assert_eq!(parse_age("8m ago"), Some(8 * 60_000));
        assert_eq!(parse_age("2 ч назад"), Some(2 * 3_600_000));
        assert_eq!(parse_age("19 мин."), Some(19 * 60_000));
        assert_eq!(parse_age("Зачислено: 8.75 KGS"), None);
        assert_eq!(parse_age("3689"), None);
        assert_eq!(parse_age("1 hr ago"), Some(3_600_000));
        assert_eq!(parse_age("3 нед. назад"), Some(3 * 7 * 24 * 3_600_000));
        // lines of a notification, not time labels
        assert_eq!(parse_age("2 messages"), None);
        assert_eq!(parse_age("1 mo"), None);
        assert_eq!(parse_age("15 декабря"), None);
        assert_eq!(parse_age("99999999999999999 w"), None);
    }

    fn t(v: &[(&str, &str)]) -> Vec<(String, String)> {
        v.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect()
    }

    #[test]
    fn iphone_notification_from_texts() {
        let texts = t(&[("title", "MBANK"), ("body", "Зачислено: 8.75 KGS Карта: *3689"), ("", "29m ago")]);
        let item = item_from_texts(&texts, "MBANK, Зачислено: 8.75 KGS Карта: *3689", 1_000_000_000).unwrap();
        assert_eq!(item.app, "MBANK");
        assert_eq!(item.title, "");
        assert_eq!(item.body, "Зачислено: 8.75 KGS Карта: *3689");
        assert_eq!(item.ts, 1_000_000_000 - 29 * 60_000);
    }

    #[test]
    fn mac_notification_takes_the_app_from_the_description() {
        let texts = t(&[("title", "deskstats"), ("subtitle", "тест 2"), ("body", "Проверка")]);
        let item = item_from_texts(&texts, "Script Editor, deskstats, тест 2, Проверка", 5).unwrap();
        assert_eq!((item.app.as_str(), item.title.as_str(), item.subtitle.as_str(), item.body.as_str()), ("Script Editor", "deskstats", "тест 2", "Проверка"));
    }

    #[test]
    fn unmarked_texts_fall_back_to_order() {
        let item = item_from_texts(&t(&[("", "MBANK"), ("", "8m ago"), ("", "Зачислено"), ("", "Карта")]), "", 1_000_000).unwrap();
        assert_eq!((item.app.as_str(), item.title.as_str(), item.body.as_str()), ("MBANK", "Зачислено", "Карта"));
        assert_eq!(item.ts, 1_000_000 - 8 * 60_000);
        assert!(item_from_texts(&t(&[("", "now")]), "", 5).is_none());
    }

    fn item(id: u64, ts: u64, app: &str, title: &str, subtitle: &str, body: &str) -> Item {
        Item { id, ts, app: app.into(), title: title.into(), subtitle: subtitle.into(), body: body.into() }
    }

    #[test]
    fn merge_drops_screen_copies_of_database_rows() {
        let db = item(7, 100, "Script Editor", "deskstats", "тест", "Проверка связи с панелью");
        let copy = item(SCREEN_ID_BASE, 105, "deskstats", "тест", "", "Проверка связи с панелью");
        let bank = item(SCREEN_ID_BASE + 1, 200, "MBANK", "Зачислено", "", "Карта");
        let merged = merge(&[db], &[copy, bank], 1000);
        assert_eq!(merged.iter().map(|i| i.id).collect::<Vec<_>>(), vec![SCREEN_ID_BASE + 1, 7]);
    }

    #[test]
    fn merge_drops_a_screen_copy_that_has_a_subtitle() {
        let db = item(7, 100, "Mail", "Айгуль", "Счёт за ноябрь", "Добрый день");
        let copy = Item { id: SCREEN_ID_BASE, ..db.clone() };
        assert_eq!(merge(&[db], &[copy], 1000).len(), 1);
    }

    #[test]
    fn a_short_database_title_does_not_hide_other_notifications() {
        let hour = 3_600_000;
        let db = item(7, hour, "Terminal", "OK", "", "");
        let bank = item(SCREEN_ID_BASE, 2 * hour, "MBANK", "Зачислено", "", "Покупка OK, баланс 100");
        assert_eq!(merge(std::slice::from_ref(&db), &[bank], 3 * hour).len(), 2);
        // its own copy on screen, seen at the same time, is still one notification
        let copy = item(SCREEN_ID_BASE + 1, hour + 2000, "Terminal", "OK", "", "");
        assert_eq!(merge(&[db], &[copy], 3 * hour).len(), 1);
    }

    #[test]
    fn merge_leaves_out_what_is_older_than_two_days() {
        let now = 10 * MAX_AGE_MS;
        let old = item(1, now - MAX_AGE_MS - 1, "Mail", "Старое письмо", "", "");
        let fresh = item(2, now - 1000, "Mail", "Новое письмо", "", "");
        let on_screen = item(SCREEN_ID_BASE, now - MAX_AGE_MS - 1, "MBANK", "Зачислено давно", "", "");
        assert_eq!(merge(&[old, fresh], &[on_screen], now).iter().map(|i| i.id).collect::<Vec<_>>(), vec![2]);
    }

    #[test]
    fn database_rows_are_read_newest_first() {
        let dir = std::env::temp_dir().join(format!("deskstats-db-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (db, plist) = (dir.join("db"), dir.join("n.plist"));
        let run = |program: &str, args: &[&str]| assert!(std::process::Command::new(program).args(args).status().unwrap().success());
        // A notification is stored as a binary property list with its texts under "req".
        let blob = |title: &str, body: &str| {
            let xml = format!("<plist version=\"1.0\"><dict><key>req</key><dict><key>titl</key><string>{title}</string><key>subt</key><string>тема</string><key>body</key><string>{body}</string></dict></dict></plist>");
            std::fs::write(&plist, xml).unwrap();
            run("/usr/bin/plutil", &["-convert", "binary1", plist.to_str().unwrap()]);
            std::fs::read(&plist).unwrap().iter().map(|b| format!("{b:02x}")).collect::<String>()
        };
        let mac_time = |ms_ago: u64| (now_ms() - ms_ago) as f64 / 1000.0 - MAC_EPOCH_OFFSET;
        let sql = format!(
            "create table app (app_id integer primary key, identifier text); \
             create table record (rec_id integer primary key, app_id integer, delivered_date real, data blob); \
             insert into app values (1, 'com.example.nosuchapp'); \
             insert into record values (10, 1, {}, x'{}'), (11, 1, {}, x'{}'), (12, 1, {}, x'{}'), (13, 1, {}, x'00');",
            mac_time(60_000), blob("Первое", "раньше"),
            mac_time(1000), blob("Второе", "позже"),
            mac_time(MAX_AGE_MS + 60_000), blob("Старое", "давно"),
            mac_time(30_000),
        );
        run("/usr/bin/sqlite3", &[db.to_str().unwrap(), &sql]);
        let mut database = Database::at(db.to_string_lossy().into_owned());
        let rows = database.refresh().unwrap();
        // newest first, the one older than two days left out, a broken blob read as empty texts
        assert_eq!(rows.iter().map(|i| i.id).collect::<Vec<_>>(), vec![11, 13, 10]);
        assert_eq!((rows[0].app.as_str(), rows[0].title.as_str(), rows[0].subtitle.as_str(), rows[0].body.as_str()), ("nosuchapp", "Второе", "тема", "позже"));
        assert_eq!(rows[1].title, "");
        assert!(now_ms().abs_diff(rows[2].ts) < 120_000);
        std::fs::remove_dir_all(&dir).unwrap();
        // a database that is not there: nothing, and no panic
        assert!(Database::at(dir.join("missing").to_string_lossy().into_owned()).refresh().is_none());
    }

    #[test]
    fn the_same_text_again_is_a_new_notification_only_when_surely_newer() {
        let mut screen = Screen::new(false);
        let hour = 3_600_000u64;
        let now = 100 * hour;
        let call = |ts| item(0, ts, "Телефон", "Пропущенный вызов", "", "Айгуль");
        assert!(screen.take(call(now), now));
        // still on screen a few seconds later
        assert!(!screen.take(call(now + 3000), now + 3000));
        // the open Notification Center shows it as "1h ago" when it is almost two hours old
        let later = now + 2 * hour - 60_000;
        assert!(!screen.take(call(later - hour), later));
        // a banner with the same text much later is another call
        assert!(screen.take(call(later + hour), later + hour));
        // an entry older than two days is not taken at all
        assert!(!screen.take(item(0, now, "MBANK", "Старое", "", ""), now + MAX_AGE_MS));
        assert_eq!(screen.history.len(), 2);
    }
}
