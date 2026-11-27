use super::ffi::*;
use crate::util::{json_escape, log, normalize, now_ms, truncate};
use std::collections::HashMap;
use std::cell::Cell;
use std::ffi::{c_int, c_void, CStr, CString};
use std::time::{Duration, Instant, SystemTime};

pub const SHORT: usize = 6;
pub const FULL: usize = 30;
const TEXT_LIMIT: usize = 600;
const MAX_AGE_MS: u64 = 48 * 3600 * 1000;
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
            if rc != SQLITE_DONE {
                return None;
            }
        }
        Some(items)
    }

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

const SCREEN_ID_BASE: u64 = 1 << 40;
const SCREEN_HISTORY: usize = 20;
const MAX_NODES: usize = 800;
const MAX_DEPTH: usize = 16;
const WALK_DEADLINE: Duration = Duration::from_millis(200);
const BIG_WINDOW: usize = 150;
const BIG_WINDOW_EVERY: Duration = Duration::from_secs(3);
const AX_CANNOT_COMPLETE: i32 = -25204;

struct Seen {
    item: Item,
    key: String,
    on_screen_ms: u64,
}

pub struct Screen {
    prompted: bool,
    logged_untrusted: bool,
    trusted: bool,
    trusted_checked: Option<Instant>,
    pid: Option<c_int>,
    pid_checked: Option<Instant>,
    next_id: u64,
    history: Vec<Seen>,
    items: Vec<Item>,
    keys: AxKeys,
    timeout_set: bool,
    next_read: Option<Instant>,
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
}

impl Screen {
    pub fn new() -> Self {
        Screen {
            prompted: false,
            logged_untrusted: false,
            trusted: false,
            trusted_checked: None,
            pid: None,
            pid_checked: None,
            next_id: SCREEN_ID_BASE,
            history: Vec::new(),
            items: Vec::new(),
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
            },
        }
    }

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

    fn take(&mut self, item: Item, now: u64) -> bool {
        let age = now.saturating_sub(item.ts);
        if age >= MAX_AGE_MS {
            return false;
        }
        let key = item.key();
        if let Some(seen) = self.history.iter_mut().filter(|h| h.key == key).max_by_key(|h| h.item.ts) {
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

    fn ensure_trusted(&mut self) -> bool {
        if self.trusted {
            return true;
        }
        if self.trusted_checked.is_some_and(|t| t.elapsed() < Duration::from_secs(30)) {
            return false;
        }
        self.trusted_checked = Some(Instant::now());
        unsafe {
            // macOS shows its question once per installed program: a restart of the same one asks nothing.
            let options = if self.prompted || already_asked() {
                std::ptr::null()
            } else {
                self.prompted = true;
                remember_asked();
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
        let mut budget = MAX_NODES;
        self.stuck.set(false);
        unsafe {
            if !self.timeout_set {
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
                        self.walk(CFArrayGetValueAtIndex(windows, i), 0, &mut budget, deadline, &mut found);
                    }
                }
                CFRelease(windows);
            }
        }
        self.next_read = (MAX_NODES - budget > BIG_WINDOW).then(|| Instant::now() + BIG_WINDOW_EVERY);
        if self.stuck.get() {
            self.pid = None;
        }
        Some(found)
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

    unsafe fn walk(&self, element: *const c_void, depth: usize, budget: &mut usize, deadline: Instant, found: &mut Vec<Item>) {
        if *budget == 0 || depth > MAX_DEPTH || self.stuck.get() || Instant::now() >= deadline {
            return;
        }
        *budget -= 1;
        let role = self.attr_string(element, self.keys.role);
        let subrole = self.attr_string(element, self.keys.subrole);
        let notification = subrole.starts_with("AXNotificationCenter") && role != "AXWindow";
        let children = self.attr(element, self.keys.children);
        if children.is_null() {
            return;
        }
        if CFGetTypeID(children) == CFArrayGetTypeID() {
            let mut texts = Vec::new();
            for i in 0..CFArrayGetCount(children) {
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
                self.walk(child, depth + 1, budget, deadline, found);
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

const RESTARTED: &str = "DESKSTATS_AX_RESTARTED";

pub fn list_json(list: Option<&[Item]>, count: usize) -> String {
    match list {
        Some(items) => format!("\"alerts\":[{}],", items.iter().take(count).map(Item::json).collect::<Vec<_>>().join(",")),
        None => String::from("\"alerts\":null,"),
    }
}

pub fn accessibility_granted() -> bool {
    unsafe { AXIsProcessTrustedWithOptions(std::ptr::null()) != 0 }
}

fn granted_to_a_fresh_process() -> bool {
    let Ok(program) = std::env::current_exe() else { return false };
    crate::util::run(&program.to_string_lossy(), &["--ax-check"], Duration::from_secs(5)).is_some_and(|(_, out)| out.trim() == "yes")
}

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
        let mut lines = loose.into_iter();
        let app = lines.next()?;
        let title = lines.next().unwrap_or_default();
        let body = truncate(&lines.collect::<Vec<_>>().join(" "), TEXT_LIMIT);
        return Some(Item { id: 0, ts, app, title, subtitle: String::new(), body });
    }
    body.extend(loose);
    let body = truncate(&body.join(" "), TEXT_LIMIT);
    let first = description.split(", ").next().unwrap_or("").trim().to_string();
    let app = if first.is_empty() || first == title || first == subtitle || body.starts_with(&first) {
        std::mem::take(&mut title)
    } else {
        first
    };
    Some(Item { id: 0, ts, app, title, subtitle, body })
}

pub fn merge(database: &[Item], screen: &[Item], now: u64) -> Vec<Item> {
    let recent = |i: &&Item| now.saturating_sub(i.ts) < MAX_AGE_MS;
    let mut out: Vec<Item> = database.iter().filter(recent).cloned().collect();
    for s in screen.iter().filter(recent) {
        let text = normalize(&format!("{} {} {} {}", s.app, s.title, s.subtitle, s.body));
        let duplicate = database.iter().any(|d| {
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

/// What tells this program apart from the one installed before it: the time its file was written.
fn this_build() -> String {
    std::env::current_exe()
        .and_then(std::fs::metadata)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or_else(String::new, |d| d.as_secs().to_string())
}

fn asked_mark() -> String {
    format!("{}/accessibility-asked", super::data_dir())
}

/// Whether macOS was already asked to show its Accessibility question for this program.
fn already_asked() -> bool {
    let build = this_build();
    !build.is_empty() && std::fs::read_to_string(asked_mark()).is_ok_and(|b| b.trim() == build)
}

fn remember_asked() {
    let _ = std::fs::write(asked_mark(), this_build());
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_json_escapes_and_caps() {
        let item = Item { id: 7, ts: 5, app: "Mail".into(), title: "a \"b\"".into(), subtitle: String::new(), body: "c\nd".into() };
        assert_eq!(
            list_json(Some(std::slice::from_ref(&item)), 6),
            "\"alerts\":[{\"id\":7,\"ts\":5,\"app\":\"Mail\",\"title\":\"a \\\"b\\\"\",\"subtitle\":\"\",\"body\":\"c d\"}],"
        );
        assert_eq!(list_json(None, 30), "\"alerts\":null,");
        assert_eq!(list_json(Some(&[item.clone(), item]), 1).matches("\"id\"").count(), 1);
    }

    #[test]
    fn ages() {
        assert_eq!(parse_age("now"), Some(0));
        assert_eq!(parse_age("8m ago"), Some(8 * 60_000));
        assert_eq!(parse_age("2 ч назад"), Some(2 * 3_600_000));
        assert_eq!(parse_age("19 мин."), Some(19 * 60_000));
        assert_eq!(parse_age("Credited: 8.75 KGS"), None);
        assert_eq!(parse_age("3689"), None);
        assert_eq!(parse_age("1 hr ago"), Some(3_600_000));
        assert_eq!(parse_age("3 нед. назад"), Some(3 * 7 * 24 * 3_600_000));
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
        let texts = t(&[("title", "MBANK"), ("body", "Credited: 8.75 KGS Card: *3689"), ("", "29m ago")]);
        let item = item_from_texts(&texts, "MBANK, Credited: 8.75 KGS Card: *3689", 1_000_000_000).unwrap();
        assert_eq!(item.app, "MBANK");
        assert_eq!(item.title, "");
        assert_eq!(item.body, "Credited: 8.75 KGS Card: *3689");
        assert_eq!(item.ts, 1_000_000_000 - 29 * 60_000);
    }

    #[test]
    fn mac_notification_takes_the_app_from_the_description() {
        let texts = t(&[("title", "deskstats"), ("subtitle", "test 2"), ("body", "Check")]);
        let item = item_from_texts(&texts, "Script Editor, deskstats, test 2, Check", 5).unwrap();
        assert_eq!((item.app.as_str(), item.title.as_str(), item.subtitle.as_str(), item.body.as_str()), ("Script Editor", "deskstats", "test 2", "Check"));
    }

    #[test]
    fn unmarked_texts_fall_back_to_order() {
        let item = item_from_texts(&t(&[("", "MBANK"), ("", "8m ago"), ("", "Credited"), ("", "Card")]), "", 1_000_000).unwrap();
        assert_eq!((item.app.as_str(), item.title.as_str(), item.body.as_str()), ("MBANK", "Credited", "Card"));
        assert_eq!(item.ts, 1_000_000 - 8 * 60_000);
        assert!(item_from_texts(&t(&[("", "now")]), "", 5).is_none());
    }

    fn item(id: u64, ts: u64, app: &str, title: &str, subtitle: &str, body: &str) -> Item {
        Item { id, ts, app: app.into(), title: title.into(), subtitle: subtitle.into(), body: body.into() }
    }

    #[test]
    fn merge_drops_screen_copies_of_database_rows() {
        let db = item(7, 100, "Script Editor", "deskstats", "test", "Checking the link to the panel");
        let copy = item(SCREEN_ID_BASE, 105, "deskstats", "test", "", "Checking the link to the panel");
        let bank = item(SCREEN_ID_BASE + 1, 200, "MBANK", "Credited", "", "Card");
        let merged = merge(&[db], &[copy, bank], 1000);
        assert_eq!(merged.iter().map(|i| i.id).collect::<Vec<_>>(), vec![SCREEN_ID_BASE + 1, 7]);
    }

    #[test]
    fn merge_drops_a_screen_copy_that_has_a_subtitle() {
        let db = item(7, 100, "Mail", "Alice", "November invoice", "Good afternoon");
        let copy = Item { id: SCREEN_ID_BASE, ..db.clone() };
        assert_eq!(merge(&[db], &[copy], 1000).len(), 1);
    }

    #[test]
    fn a_short_database_title_does_not_hide_other_notifications() {
        let hour = 3_600_000;
        let db = item(7, hour, "Terminal", "OK", "", "");
        let bank = item(SCREEN_ID_BASE, 2 * hour, "MBANK", "Credited", "", "Purchase OK, balance 100");
        assert_eq!(merge(std::slice::from_ref(&db), &[bank], 3 * hour).len(), 2);
        let copy = item(SCREEN_ID_BASE + 1, hour + 2000, "Terminal", "OK", "", "");
        assert_eq!(merge(&[db], &[copy], 3 * hour).len(), 1);
    }

    #[test]
    fn merge_leaves_out_what_is_older_than_two_days() {
        let now = 10 * MAX_AGE_MS;
        let old = item(1, now - MAX_AGE_MS - 1, "Mail", "Old letter", "", "");
        let fresh = item(2, now - 1000, "Mail", "New letter", "", "");
        let on_screen = item(SCREEN_ID_BASE, now - MAX_AGE_MS - 1, "MBANK", "Credited long ago", "", "");
        assert_eq!(merge(&[old, fresh], &[on_screen], now).iter().map(|i| i.id).collect::<Vec<_>>(), vec![2]);
    }

    #[test]
    fn database_rows_are_read_newest_first() {
        let dir = std::env::temp_dir().join(format!("deskstats-db-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (db, plist) = (dir.join("db"), dir.join("n.plist"));
        let run = |program: &str, args: &[&str]| assert!(std::process::Command::new(program).args(args).status().unwrap().success());
        let blob = |title: &str, body: &str| {
            let xml = format!("<plist version=\"1.0\"><dict><key>req</key><dict><key>titl</key><string>{title}</string><key>subt</key><string>topic</string><key>body</key><string>{body}</string></dict></dict></plist>");
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
            mac_time(60_000), blob("First", "earlier"),
            mac_time(1000), blob("Second", "later"),
            mac_time(MAX_AGE_MS + 60_000), blob("Old", "long ago"),
            mac_time(30_000),
        );
        run("/usr/bin/sqlite3", &[db.to_str().unwrap(), &sql]);
        let mut database = Database::at(db.to_string_lossy().into_owned());
        let rows = database.refresh().unwrap();
        assert_eq!(rows.iter().map(|i| i.id).collect::<Vec<_>>(), vec![11, 13, 10]);
        assert_eq!((rows[0].app.as_str(), rows[0].title.as_str(), rows[0].subtitle.as_str(), rows[0].body.as_str()), ("nosuchapp", "Second", "topic", "later"));
        assert_eq!(rows[1].title, "");
        assert!(now_ms().abs_diff(rows[2].ts) < 120_000);
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(Database::at(dir.join("missing").to_string_lossy().into_owned()).refresh().is_none());
    }

    #[test]
    fn the_same_text_again_is_a_new_notification_only_when_surely_newer() {
        let mut screen = Screen::new();
        let hour = 3_600_000u64;
        let now = 100 * hour;
        let call = |ts| item(0, ts, "Phone", "Missed call", "", "Alice");
        assert!(screen.take(call(now), now));
        assert!(!screen.take(call(now + 3000), now + 3000));
        let later = now + 2 * hour - 60_000;
        assert!(!screen.take(call(later - hour), later));
        assert!(screen.take(call(later + hour), later + hour));
        assert!(!screen.take(item(0, now, "MBANK", "Old", "", ""), now + MAX_AGE_MS));
        assert_eq!(screen.history.len(), 2);
    }
}
