//! What the window knows about the server on this computer and what it can do to it.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::process::Command;
use std::time::Duration;

pub const PORT: u16 = 8787;
pub const APP_LINK: &str = "https://github.com/danijar2000/deskstats/releases/latest/download/deskstats.apk";

/// What the server says about itself.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Status {
    pub version: String,
    pub name: String,
    pub addresses: Vec<String>,
    pub wifi: bool,
    pub adb: bool,
    /// How long ago a phone last asked for numbers; None when none has since the server started.
    pub phone_ago: Option<Duration>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum Server {
    /// Not asked yet.
    #[default]
    Unknown,
    /// Nothing listens on the port.
    Stopped,
    /// It runs, but is a release from before the window and says nothing more.
    Old,
    Running(Status),
}

/// The folder of the token: the same one the server uses.
pub fn data_dir() -> String {
    let var = |name: &str| std::env::var(name).ok().filter(|value| !value.is_empty());
    if cfg!(target_os = "macos") {
        format!("{}/Library/Application Support/deskstats", var("HOME").unwrap_or_default())
    } else if cfg!(windows) {
        format!("{}\\deskstats", var("APPDATA").or_else(|| var("USERPROFILE")).unwrap_or_else(|| String::from(".")))
    } else {
        let base = var("XDG_CONFIG_HOME").filter(|dir| dir.starts_with('/'));
        format!("{}/deskstats", base.unwrap_or_else(|| format!("{}/.config", var("HOME").unwrap_or_default())))
    }
}

fn token_path() -> std::path::PathBuf {
    std::path::Path::new(&data_dir()).join("token")
}

fn get(path: &str, token: &str) -> Option<(u16, String)> {
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, PORT));
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_millis(700)).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(3))).ok()?;
    stream.set_write_timeout(Some(Duration::from_secs(3))).ok()?;
    write!(stream, "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{PORT}\r\nAuthorization: Bearer {token}\r\nConnection: close\r\n\r\n").ok()?;
    // Read up to the end of the body that Content-Length announces: the server may keep the connection open.
    let mut answer = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        if let Some(end) = answer.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&answer[..end]).to_ascii_lowercase();
            let length = head.lines().find_map(|line| line.strip_prefix("content-length:")).and_then(|v| v.trim().parse::<usize>().ok());
            if length.is_some_and(|length| answer.len() >= end + 4 + length) {
                break;
            }
        }
        match stream.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) if answer.len() < 64 * 1024 => answer.extend_from_slice(&buf[..n]),
            Ok(_) => break,
        }
    }
    let answer = String::from_utf8_lossy(&answer).into_owned();
    let (head, body) = answer.split_once("\r\n\r\n")?;
    let code = head.split(' ').nth(1)?.parse().ok()?;
    let length = head.to_ascii_lowercase().lines().find_map(|line| line.strip_prefix("content-length:").and_then(|v| v.trim().parse::<usize>().ok()));
    let body = match length {
        Some(length) => body.get(..length).unwrap_or(body),
        None => body,
    };
    Some((code, body.to_string()))
}

/// Asks the server how it is.
pub fn ask() -> Server {
    let token = std::fs::read_to_string(token_path()).unwrap_or_default();
    match get("/status", token.trim()) {
        None => Server::Stopped,
        Some((200, body)) => parse(&body).map_or(Server::Old, Server::Running),
        Some(_) => Server::Old,
    }
}

/// The text after `"key":` in [json]. The answer of the server is small and its keys do not repeat.
fn after<'a>(json: &'a str, key: &str) -> Option<&'a str> {
    let mark = format!("\"{key}\":");
    json.find(&mark).map(|at| &json[at + mark.len()..])
}

fn string(json: &str, key: &str) -> Option<String> {
    let rest = after(json, key)?.strip_prefix('"')?;
    let mut out = String::new();
    let mut chars = rest.chars();
    loop {
        match chars.next()? {
            '"' => return Some(out),
            '\\' => match chars.next()? {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                'u' => {
                    let code: String = chars.by_ref().take(4).collect();
                    out.push(u32::from_str_radix(&code, 16).ok().and_then(char::from_u32).unwrap_or('?'));
                }
                other => out.push(other),
            },
            other => out.push(other),
        }
    }
}

fn word<'a>(json: &'a str, key: &str) -> Option<&'a str> {
    let rest = after(json, key)?;
    Some(&rest[..rest.find([',', '}', ']']).unwrap_or(rest.len())])
}

pub fn parse(json: &str) -> Option<Status> {
    let addresses = after(json, "ips")?.strip_prefix('[')?;
    let addresses = &addresses[..addresses.find(']')?];
    Some(Status {
        version: string(json, "version")?,
        name: string(json, "name").unwrap_or_default(),
        addresses: addresses.split(',').map(|ip| ip.trim().trim_matches('"').to_string()).filter(|ip| !ip.is_empty()).collect(),
        wifi: word(json, "wifi")? == "true",
        adb: word(json, "adb")? == "true",
        phone_ago: word(json, "phone_ago_ms")?.parse().ok().map(Duration::from_millis),
    })
}

/// A command that shows no window of its own, not even for a moment.
fn quiet(program: &str) -> Command {
    #[allow(unused_mut)]
    let mut command = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

/// Starts the server, or stops it and starts it again. True when the system took the command.
pub fn restart() -> bool {
    if cfg!(target_os = "macos") {
        let user = quiet("/usr/bin/id").arg("-u").output().map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string()).unwrap_or_default();
        quiet("/bin/launchctl").args(["kickstart", "-k", &format!("gui/{user}/app.deskstats.server")]).status().is_ok_and(|s| s.success())
    } else if cfg!(windows) {
        let program = format!("{}\\deskstats\\deskstats.exe", std::env::var("LOCALAPPDATA").unwrap_or_default());
        // Only the server of this user: the one whose program is in this user's folder.
        let stop = format!("Get-Process -Name deskstats -ErrorAction SilentlyContinue | Where-Object {{ $_.Path -eq '{}' }} | Stop-Process -Force", program.replace('\'', "''"));
        let _ = quiet("powershell").args(["-NoProfile", "-Command", &stop]).status();
        quiet(&program).spawn().is_ok()
    } else {
        quiet("systemctl").args(["--user", "restart", "deskstats.service"]).status().is_ok_and(|s| s.success())
    }
}

/// Throws the key away, so that no phone that had it gets in any more, and starts the server, which makes a new one.
pub fn forget_phones() -> bool {
    let path = token_path();
    (std::fs::remove_file(&path).is_ok() || !path.exists()) && restart()
}

/// Whether the system shows dark windows, where the toolkit cannot tell by itself (Linux). None: ask the toolkit.
pub fn dark_mode() -> Option<bool> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    let out = quiet("gsettings").args(["get", "org.gnome.desktop.interface", "color-scheme"]).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).contains("dark"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_answer_of_the_server_is_read() {
        let json = r#"{"version":"0.6.0","about":{"app":"deskstats","id":"ab","name":"Doni’s \"Mac\"","model":"Mac16,10","os":"macos","port":8787},"ips":["192.168.1.5","10.0.0.2"],"wifi":true,"adb":false,"phone_ago_ms":1500,"state":"awake"}"#;
        let status = parse(json).unwrap();
        assert_eq!(status.version, "0.6.0");
        assert_eq!(status.name, "Doni’s \"Mac\"");
        assert_eq!(status.addresses, ["192.168.1.5", "10.0.0.2"]);
        assert!(status.wifi && !status.adb);
        assert_eq!(status.phone_ago, Some(Duration::from_millis(1500)));
        let never = parse(r#"{"version":"1","about":{"name":"x"},"ips":[],"wifi":false,"adb":true,"phone_ago_ms":null,"state":"awake"}"#).unwrap();
        assert_eq!((never.addresses.len(), never.phone_ago, never.adb), (0, None, true));
        assert_eq!(parse(r#"{"error":"not found"}"#), None);
    }
}
