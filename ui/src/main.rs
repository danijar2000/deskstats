// The window of deskstats on the computer and its icon by the clock.
//
//   deskstats-ui          opens the window (or brings the one already open to the front)
//   deskstats-ui --tray   shows the icon by the clock; a click on it opens the window
//   deskstats-ui --icon FILE.png   saves the icon (for the menu of applications on Linux)
//   deskstats-ui --snapshot FILE.png [dark|light]   saves a picture of the window and quits (for checks)
//
// The icon is a process of its own that draws nothing, so it costs next to nothing while the window is shut.
#![cfg_attr(windows, windows_subsystem = "windows")]

mod icon;
mod png;
#[allow(dead_code, clippy::manual_is_multiple_of)]
#[path = "../../server/src/qr.rs"]
mod qr;
mod server;
mod text;
mod tray;
mod window;

/// The release this was built for (the workflow passes the tag); a build by hand has the version of the crate.
pub const VERSION: &str = match option_env!("DESKSTATS_VERSION") {
    Some(version) if !version.is_empty() => version,
    _ => env!("CARGO_PKG_VERSION"),
};

fn main() {
    match std::env::args().nth(1).as_deref() {
        Some("--tray") => tray::run(),
        Some("--version") => println!("{VERSION}"),
        // The installer on Linux puts this picture beside the entry in the menu of applications.
        Some("--icon") => {
            let path = std::env::args().nth(2).unwrap_or_else(|| "deskstats.png".into());
            if std::fs::write(&path, png::encode(256, 256, &icon::app(256))).is_err() {
                std::process::exit(1);
            }
        }
        Some("--snapshot") => window::run_snapshot(std::env::args().nth(2).unwrap_or_else(|| "deskstats.png".into()), std::env::args().nth(3)),
        _ => window::run(),
    }
}
