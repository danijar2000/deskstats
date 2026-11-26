//! The icon by the clock. It draws no window itself: a click starts the window as a program of its own, which
//! comes to the front if it is open already.

/// Opens the window, or brings the open one to the front.
fn open_window() {
    if let Ok(me) = std::env::current_exe() {
        let _ = std::process::Command::new(me).spawn();
    }
}

#[cfg(any(target_os = "macos", windows))]
pub fn run() {
    use tray_icon::menu::{Menu, MenuEvent, MenuItem};
    use tray_icon::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
    use winit::application::ApplicationHandler;
    use winit::event::{StartCause, WindowEvent};
    use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
    use winit::window::WindowId;

    struct Tray {
        icon: Option<tray_icon::TrayIcon>,
        open: MenuItem,
        quit: MenuItem,
    }

    impl ApplicationHandler for Tray {
        fn new_events(&mut self, event_loop: &ActiveEventLoop, cause: StartCause) {
            if cause == StartCause::Init && self.icon.is_none() {
                // On macOS the icon is a template: the system paints it in the colour of the menu bar.
                let size = 44;
                let colour = if cfg!(target_os = "macos") { [0, 0, 0] } else { crate::icon::TEAL };
                let image = tray_icon::Icon::from_rgba(crate::icon::line(size, colour), size, size).ok();
                let menu = Menu::new();
                let _ = menu.append(&self.open);
                let _ = menu.append(&self.quit);
                let mut builder = TrayIconBuilder::new().with_tooltip("Deskstats").with_menu(Box::new(menu));
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
                if !cfg!(target_os = "macos") {
                    // On Windows a left click opens the window and the right one shows the menu.
                    builder = builder.with_menu_on_left_click(false);
                }
                match builder.build() {
                    Ok(icon) => self.icon = Some(icon),
                    Err(_) => event_loop.exit(),
                }
            }
        }

        fn resumed(&mut self, _event_loop: &ActiveEventLoop) {}

        fn window_event(&mut self, _event_loop: &ActiveEventLoop, _id: WindowId, _event: WindowEvent) {}

        fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
            while let Ok(event) = MenuEvent::receiver().try_recv() {
                if event.id == self.open.id() {
                    open_window();
                } else if event.id == self.quit.id() {
                    self.icon = None;
                    event_loop.exit();
                    return;
                }
            }
            while let Ok(event) = TrayIconEvent::receiver().try_recv() {
                if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                    open_window();
                }
            }
            event_loop.set_control_flow(ControlFlow::WaitUntil(std::time::Instant::now() + std::time::Duration::from_millis(250)));
        }
    }

    let mut builder = EventLoop::builder();
    #[cfg(target_os = "macos")]
    {
        use winit::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS};
        // An icon in the menu bar only: no Dock icon and no menu of its own.
        builder.with_activation_policy(ActivationPolicy::Accessory);
    }
    let Ok(event_loop) = builder.build() else { return };
    let w = crate::text::words(crate::text::system_language());
    let mut tray = Tray { icon: None, open: MenuItem::new(w.open, true, None), quit: MenuItem::new(w.quit, true, None) };
    let _ = event_loop.run_app(&mut tray);
}

/// Linux: the StatusNotifierItem of KDE and of GNOME with the AppIndicator extension (on by default in Ubuntu).
#[cfg(target_os = "linux")]
pub fn run() {
    use ksni::blocking::TrayMethods;

    struct Tray {
        open: &'static str,
        quit: &'static str,
    }

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
        fn activate(&mut self, _x: i32, _y: i32) {
            open_window();
        }
        fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
            use ksni::menu::StandardItem;
            vec![
                StandardItem { label: self.open.into(), activate: Box::new(|_: &mut Self| open_window()), ..Default::default() }.into(),
                StandardItem { label: self.quit.into(), activate: Box::new(|_: &mut Self| std::process::exit(0)), ..Default::default() }.into(),
            ]
        }
    }

    let w = crate::text::words(crate::text::system_language());
    // No tray on this desktop (GNOME without the extension): the window is still in the menu of applications.
    let Ok(_handle) = (Tray { open: w.open, quit: w.quit }).spawn() else { return };
    loop {
        std::thread::park();
    }
}
