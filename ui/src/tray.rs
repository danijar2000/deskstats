//! The icon by the clock. It draws no window itself: a click starts the panel as a program of its own, which hangs
//! from the icon. There is no menu: the panel has the Quit button.

use std::io::Read;
use std::net::{Ipv4Addr, TcpListener};

/// Where the icon listens: for the panel's Quit, and so that a second icon does not start.
pub const TRAY_PORT: u16 = 8790;

/// Takes the port of the icon; None when an icon runs already.
pub fn claim() -> Option<TcpListener> {
    TcpListener::bind((Ipv4Addr::LOCALHOST, TRAY_PORT)).ok()
}

/// Waits for the panel's "quit" and ends the icon. A plain exit: the system does not start it again until the next
/// sign-in.
fn listen_for_quit(listener: TcpListener) {
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let mut buf = [0u8; 4];
            let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(1)));
            if stream.read_exact(&mut buf).is_ok() && &buf == b"quit" {
                std::process::exit(0);
            }
        }
    });
}

/// Opens the panel hanging from the icon: [x] is the middle of the icon and [y] the edge of the bar it opens from, in
/// points; [side] says where the bar is ("up" for a bar at the bottom, "down" at the top, "auto" when unknown).
/// A click on the icon while the panel is open closes it.
fn open_panel(x: f64, y: f64, side: &str) {
    // The click gave this program the right to bring a window to the front; Windows lets it pass that on.
    #[cfg(windows)]
    crate::window::allow_any_to_come_forward();
    if let Ok(me) = std::env::current_exe() {
        let _ = std::process::Command::new(me).args(["--at", &format!("{x:.0}"), &format!("{y:.0}"), side]).spawn();
    }
}

#[cfg(any(target_os = "macos", windows))]
pub fn run(listener: TcpListener) {
    use tray_icon::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
    use winit::application::ApplicationHandler;
    use winit::event::{StartCause, WindowEvent};
    use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
    use winit::window::WindowId;

    struct Tray {
        icon: Option<tray_icon::TrayIcon>,
    }

    impl ApplicationHandler for Tray {
        fn new_events(&mut self, event_loop: &ActiveEventLoop, cause: StartCause) {
            if cause == StartCause::Init && self.icon.is_none() {
                // On macOS the icon is a template: the system paints it in the colour of the menu bar.
                let size = 44;
                let colour = if cfg!(target_os = "macos") { [0, 0, 0] } else { crate::icon::TEAL };
                let image = tray_icon::Icon::from_rgba(crate::icon::line(size, colour), size, size).ok();
                let mut builder = TrayIconBuilder::new().with_tooltip("Deskstats");
                if let Some(image) = image {
                    #[cfg(target_os = "macos")]
                    {
                        builder = builder.with_icon_templated(image);
                    }
                    #[cfg(windows)]
                    {
                        builder = builder.with_icon(image);
                    }
                }
                match builder.build() {
                    Ok(icon) => {
                        self.icon = Some(icon);
                        #[cfg(windows)]
                        std::thread::spawn(promote);
                    }
                    Err(_) => event_loop.exit(),
                }
            }
        }

        fn resumed(&mut self, _event_loop: &ActiveEventLoop) {}

        fn window_event(&mut self, _event_loop: &ActiveEventLoop, _id: WindowId, _event: WindowEvent) {}

        fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
            while let Ok(event) = TrayIconEvent::receiver().try_recv() {
                if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, rect, .. } = event {
                    // The icon's place comes in pixels of its screen; the panel wants points.
                    let monitor = event_loop.primary_monitor();
                    let scale = monitor.as_ref().map_or(1.0, |m| m.scale_factor());
                    let screen_high = monitor.as_ref().map_or(f64::MAX, |m| m.size().height as f64);
                    let (top, bottom) = (rect.position.y, rect.position.y + rect.size.height as f64);
                    let x = (rect.position.x + rect.size.width as f64 / 2.0) / scale;
                    if bottom > screen_high / 2.0 {
                        open_panel(x, top / scale, "up");
                    } else {
                        open_panel(x, bottom / scale, "down");
                    }
                }
            }
            event_loop.set_control_flow(ControlFlow::WaitUntil(std::time::Instant::now() + std::time::Duration::from_millis(250)));
        }
    }

    listen_for_quit(listener);
    let mut builder = EventLoop::builder();
    #[cfg(target_os = "macos")]
    {
        use winit::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS};
        // An icon in the menu bar only: no Dock icon and no menu of its own.
        builder.with_activation_policy(ActivationPolicy::Accessory);
    }
    let Ok(event_loop) = builder.build() else { return };
    let mut tray = Tray { icon: None };
    let _ = event_loop.run_app(&mut tray);
}

/// Windows 11 puts a new icon under the ^ arrow by the clock, and a click there never reaches the program (the
/// library asks where the icon is first, and Windows does not say for a hidden one). So the icon is put on the
/// taskbar itself, as the user could do in Settings: once Windows has noted it, a few seconds after it appears.
/// This is done once: a mark in the data folder remembers it, so an icon the user hides again stays hidden.
#[cfg(windows)]
fn promote() {
    use std::os::windows::process::CommandExt;
    let dir = std::path::PathBuf::from(crate::server::data_dir());
    let mark = dir.join("icon-on-taskbar");
    if mark.exists() {
        return;
    }
    let Ok(me) = std::env::current_exe() else { return };
    let script = format!(
        "$me = '{}'; foreach ($k in Get-ChildItem 'HKCU:\\Control Panel\\NotifyIconSettings' -EA 0) {{ \
         if ((Get-ItemProperty $k.PSPath).ExecutablePath -eq $me) {{ \
         Set-ItemProperty $k.PSPath IsPromoted 1 -Type DWord; 'done' }} }}",
        me.display().to_string().replace('\'', "''")
    );
    for _ in 0..6 {
        std::thread::sleep(std::time::Duration::from_secs(5));
        const NO_WINDOW: u32 = 0x0800_0000;
        let out = std::process::Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .creation_flags(NO_WINDOW)
            .output();
        if out.is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains("done")) {
            let _ = std::fs::create_dir_all(&dir);
            let _ = std::fs::write(&mark, b"");
            return;
        }
    }
}

/// Linux: the StatusNotifierItem of KDE and of GNOME with the AppIndicator extension (on by default in Ubuntu).
#[cfg(target_os = "linux")]
pub fn run(listener: TcpListener) {
    use ksni::blocking::TrayMethods;

    struct Tray;

    impl ksni::Tray for Tray {
        fn id(&self) -> String {
            "deskstats".into()
        }
        fn title(&self) -> String {
            "Deskstats".into()
        }
        fn icon_pixmap(&self) -> Vec<ksni::Icon> {
            // ARGB in network order, as the specification wants.
            [22, 44].iter().map(|&size| {
                let rgba = crate::icon::line(size, crate::icon::TEAL);
                let data = rgba.chunks(4).flat_map(|p| [p[3], p[0], p[1], p[2]]).collect();
                ksni::Icon { width: size as i32, height: size as i32, data }
            }).collect()
        }
        fn activate(&mut self, x: i32, y: i32) {
            // Some desktops say where the click was, some say 0, 0: then the panel finds its own place.
            let (x, y) = if x == 0 && y == 0 { (f64::NAN, f64::NAN) } else { (x as f64, y as f64) };
            open_panel(x, y, "auto");
        }
    }

    listen_for_quit(listener);
    // No tray on this desktop (GNOME without the extension): the menu of applications opens the panel instead.
    let Ok(_handle) = Tray.spawn() else { return };
    loop {
        std::thread::park();
    }
}
