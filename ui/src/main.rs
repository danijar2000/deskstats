// The icon of deskstats by the clock and its panel.
//
//   deskstats-ui                    shows the icon by the clock; a click on it opens the panel. If the icon runs
//                                   already (started again from the menu of applications), opens the panel instead
//   deskstats-ui --at X Y SIDE      opens the panel hanging from the icon at X, Y (points); SIDE is down, up or auto
//   deskstats-ui --icon FILE.png    saves the icon (for the menu of applications on Linux)
//   deskstats-ui --snapshot FILE.png [dark|light]   saves a picture of the panel and quits (for checks)
//
// The icon is a process of its own that draws nothing, so it costs next to nothing while the panel is shut.
#![cfg_attr(windows, windows_subsystem = "windows")]

mod icon;
mod png;
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
        // --tray: what the installers of earlier releases start at sign-in.
        None | Some("--tray") => match tray::claim() {
            Some(listener) => tray::run(listener),
            None => window::run_panel_alone(),
        },
        Some("--version") => println!("{VERSION}"),
        // The installer on Linux puts this picture beside the entry in the menu of applications.
        Some("--icon") => {
            let path = std::env::args().nth(2).unwrap_or_else(|| "deskstats.png".into());
            if std::fs::write(&path, png::encode(256, 256, &icon::app(256))).is_err() {
                std::process::exit(1);
            }
        }
        Some("--at") => {
            let arg = |n: usize| std::env::args().nth(n).and_then(|v| v.parse::<f32>().ok()).unwrap_or(f32::NAN);
            let side = match std::env::args().nth(4).as_deref() {
                Some("up") => window::Side::Up,
                Some("auto") => window::Side::Auto,
                _ => window::Side::Down,
            };
            window::run_panel(arg(2), arg(3), side);
        }
        Some("--snapshot") => window::run_snapshot(std::env::args().nth(2).unwrap_or_else(|| "deskstats.png".into()), std::env::args().nth(3)),
        _ => window::run_panel_alone(),
    }
}
