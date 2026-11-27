//! The window: the computer's numbers, how the server is, how to connect a phone, and the few things one can do to
//! the server. Laid out like the popups of a menu bar monitor: rings on top, a history chart, then rows of details.

use crate::server::{self, Server, Stats, Status};
use crate::text::{self, Language, Words};
use eframe::egui::{self, Color32, Pos2, RichText, Stroke, Vec2};
use std::collections::VecDeque;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Where the open window listens, so that a second start brings it to the front instead of opening another.
const WINDOW_PORT: u16 = 8789;
const TEAL: Color32 = Color32::from_rgb(crate::icon::TEAL[0], crate::icon::TEAL[1], crate::icon::TEAL[2]);
const RED: Color32 = Color32::from_rgb(0xef, 0x44, 0x44);
const AMBER: Color32 = Color32::from_rgb(0xf5, 0x9e, 0x0b);
const BLUE: Color32 = Color32::from_rgb(0x3b, 0x82, 0xf6);
const PURPLE: Color32 = Color32::from_rgb(0x8b, 0x5c, 0xf6);
const GREEN: Color32 = Color32::from_rgb(0x22, 0xc5, 0x5e);
/// How many seconds of history the chart keeps.
const HISTORY: usize = 120;
/// The width of the panel that hangs from the icon.
const PANEL_WIDTH: f32 = 300.0;

/// Where the panel hangs from: the middle of the icon and the edge of the bar, in points (NaN when unknown).
#[derive(Clone, Copy)]
struct Anchor {
    x: f32,
    y: f32,
    side: Side,
}

/// Where the bar with the icon is: at the top (the panel opens down), at the bottom (up), or unknown.
#[derive(Clone, Copy, PartialEq)]
pub enum Side {
    Down,
    Up,
    Auto,
}

impl Anchor {
    /// Whether the panel opens upwards on a screen [high] points tall.
    fn up(&self, high: f32) -> bool {
        match self.side {
            Side::Up => true,
            Side::Down => false,
            Side::Auto => self.y.is_finite() && self.y > high / 2.0,
        }
    }
}

/// One second of numbers for the charts.
#[derive(Clone, Copy, Default)]
struct Point {
    cpu: f32,
    ram: f32,
    read: f32,
    write: f32,
    down: f32,
    up: f32,
}

/// Opens the panel under (or above) the icon; it closes when something else is clicked.
pub fn run_panel(x: f32, y: f32, side: Side) {
    open(None, Anchor { x, y, side });
}

/// Opens the panel where there is no icon to hang from (from the menu of applications): at the top in the middle.
pub fn run_panel_alone() {
    open(None, Anchor { x: f32::NAN, y: f32::NAN, side: Side::Down });
}

/// Opens the window, waits for the server's answer, saves a picture of it to [path] and quits.
pub fn run_snapshot(path: String, theme: Option<String>) {
    let theme = match theme.as_deref() {
        Some("dark") => Some(egui::ThemePreference::Dark),
        Some("light") => Some(egui::ThemePreference::Light),
        _ => None,
    };
    open(Some(Snapshot { path, theme, asked: false, at: Instant::now(), taken: 0 }), Anchor { x: 400.0, y: 30.0, side: Side::Down });
}

/// Lets whichever program asks next bring its window to the front (AllowSetForegroundWindow with ASFW_ANY).
#[cfg(windows)]
pub fn allow_any_to_come_forward() {
    #[link(name = "user32")]
    unsafe extern "system" {
        fn AllowSetForegroundWindow(process: u32) -> i32;
    }
    // SAFETY: a plain call with a number; it only changes who may take the focus next.
    unsafe {
        AllowSetForegroundWindow(u32::MAX);
    }
}

struct Snapshot {
    path: String,
    theme: Option<egui::ThemePreference>,
    asked: bool,
    at: Instant,
    /// For a moving picture (the video in the docs): a path with %03d takes this many pictures, half a second apart.
    taken: usize,
}

fn open(snapshot: Option<Snapshot>, anchor: Anchor) {
    // A picture is taken by a window of its own, even while the real one is open.
    let port = if snapshot.is_some() { 0 } else { WINDOW_PORT };
    let Ok(listener) = TcpListener::bind((Ipv4Addr::LOCALHOST, port)) else {
        // Already open: it is asked to come to the front. On Windows only a program the user just used may do
        // that, which this one is and the open window is not, so it is let.
        #[cfg(windows)]
        allow_any_to_come_forward();
        // From the icon, a second click closes the open panel, as a menu would.
        if let Ok(mut stream) = TcpStream::connect((Ipv4Addr::LOCALHOST, WINDOW_PORT)) {
            let _ = stream.write_all(b"togl");
        }
        return;
    };
    let size = 64;
    let icon = egui::IconData { rgba: crate::icon::app(size), width: size, height: size };
    // A panel: no frame of its own, rounded corners drawn here, above other windows, not in the taskbar.
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Deskstats")
        .with_app_id("deskstats")
        .with_icon(icon)
        .with_decorations(false)
        .with_transparent(true)
        .with_resizable(false)
        .with_always_on_top()
        .with_taskbar(false)
        .with_has_shadow(true)
        .with_inner_size([PANEL_WIDTH, 520.0]);
    if anchor.x.is_finite() && anchor.y.is_finite() {
        let y = if anchor.side == Side::Up { anchor.y - 526.0 } else { anchor.y + 6.0 };
        viewport = viewport.with_position([anchor.x - PANEL_WIDTH / 2.0, y]);
    }
    #[allow(unused_mut)]
    let mut options = eframe::NativeOptions { viewport, centered: !anchor.x.is_finite(), ..Default::default() };
    #[cfg(target_os = "macos")]
    {
        // Like a menu: no icon in the Dock while the panel is open.
        options.event_loop_builder = Some(Box::new(|builder| {
            use winit::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS};
            builder.with_activation_policy(ActivationPolicy::Accessory);
        }));
    }
    let _ = eframe::run_native("Deskstats", options, Box::new(move |cc| Ok(Box::new(App::new(cc, listener, snapshot, anchor)))));
}

struct App {
    words: &'static Words,
    state: Arc<Mutex<Server>>,
    asking: Arc<AtomicBool>,
    shown: Arc<AtomicBool>,
    /// A newer release on GitHub, when there is one; and whether the update was started.
    release: Arc<Mutex<Option<String>>>,
    updating: bool,
    /// The log of an update that did not take, to show once.
    update_failed: Option<String>,
    /// The latest numbers, and those of the last minutes.
    stats: Arc<Mutex<Option<Stats>>>,
    history: Arc<Mutex<VecDeque<Point>>>,
    /// Where the panel hangs from; it closes when it loses the focus or the icon is clicked again.
    anchor: Anchor,
    toggled: Arc<AtomicBool>,
    focused_once: bool,
    blurred: Option<Instant>,
    height: f32,
    copied: Option<Instant>,
    failed: Arc<Mutex<Option<Instant>>>,
    asking_to_forget: bool,
    checked: Instant,
    dark_checked: Option<Instant>,
    snapshot: Option<Snapshot>,
}

impl App {
    fn new(cc: &eframe::CreationContext, listener: TcpListener, snapshot: Option<Snapshot>, anchor: Anchor) -> Self {
        let language = text::system_language();
        fonts(&cc.egui_ctx, language);
        style(&cc.egui_ctx);
        // Started from the icon, the window comes to the front at once, as when it is asked to later.
        let shown = Arc::new(AtomicBool::new(snapshot.is_none()));
        let toggled = Arc::new(AtomicBool::new(false));
        {
            let (shown, toggled, ctx) = (shown.clone(), toggled.clone(), cc.egui_ctx.clone());
            std::thread::spawn(move || {
                for mut stream in listener.incoming().flatten() {
                    let mut buf = [0u8; 4];
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(1)));
                    if stream.read_exact(&mut buf).is_ok() {
                        match &buf {
                            b"show" => shown.store(true, Ordering::Relaxed),
                            b"togl" => toggled.store(true, Ordering::Relaxed),
                            _ => continue,
                        }
                        ctx.request_repaint();
                    }
                }
            });
        }
        let mut app = App {
            words: text::words(language),
            state: Arc::default(),
            asking: Arc::default(),
            shown,
            stats: Arc::default(),
            history: Arc::default(),
            release: Arc::default(),
            updating: false,
            update_failed: server::failed_update(),
            anchor,
            toggled,
            focused_once: false,
            blurred: None,
            height: 0.0,
            copied: None,
            failed: Arc::default(),
            asking_to_forget: false,
            checked: Instant::now(),
            dark_checked: None,
            snapshot,
        };
        app.refresh(&cc.egui_ctx);
        {
            let (release, ctx) = (app.release.clone(), cc.egui_ctx.clone());
            std::thread::spawn(move || {
                *release.lock().unwrap_or_else(|e| e.into_inner()) = server::newer_release();
                ctx.request_repaint();
            });
        }
        app
    }

    /// Asks the server again, off the thread that draws.
    fn refresh(&mut self, ctx: &egui::Context) {
        self.checked = Instant::now();
        if self.asking.swap(true, Ordering::Relaxed) {
            return;
        }
        let (state, asking, ctx) = (self.state.clone(), self.asking.clone(), ctx.clone());
        let (stats, history) = (self.stats.clone(), self.history.clone());
        std::thread::spawn(move || {
            let now = server::ask();
            let numbers = if matches!(now, Server::Running(_)) { server::stats() } else { None };
            if let Some(n) = &numbers {
                let mut history = history.lock().unwrap_or_else(|e| e.into_inner());
                let (read, write) = n.disk_rates.unwrap_or((0.0, 0.0));
                history.push_back(Point { cpu: n.cpu, ram: n.ram_pct, read: read as f32, write: write as f32, down: n.down as f32, up: n.up as f32 });
                while history.len() > HISTORY {
                    history.pop_front();
                }
            }
            *stats.lock().unwrap_or_else(|e| e.into_inner()) = numbers;
            *state.lock().unwrap_or_else(|e| e.into_inner()) = now;
            asking.store(false, Ordering::Relaxed);
            ctx.request_repaint();
        });
    }

    /// Does [work] off the thread that draws, then asks the server until it answers again.
    fn act(&mut self, ctx: &egui::Context, work: fn() -> bool) {
        let (state, failed, ctx) = (self.state.clone(), self.failed.clone(), ctx.clone());
        *state.lock().unwrap_or_else(|e| e.into_inner()) = Server::Unknown;
        std::thread::spawn(move || {
            if !work() {
                *failed.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
            }
            // The server needs a moment to listen again.
            // launchd may hold a server back for up to half a minute after it last started.
            for _ in 0..80 {
                std::thread::sleep(Duration::from_millis(500));
                let now = server::ask();
                let up = matches!(now, Server::Running(_) | Server::Old);
                *state.lock().unwrap_or_else(|e| e.into_inner()) = now;
                if up {
                    break;
                }
            }
            ctx.request_repaint();
        });
    }

    fn follow_system_theme(&mut self, ctx: &egui::Context) {
        if let Some(theme) = self.snapshot.as_ref().and_then(|s| s.theme) {
            ctx.set_theme(theme);
            return;
        }
        // Linux desktops do not always tell the toolkit; GNOME is asked now and then.
        if self.dark_checked.is_some_and(|t| t.elapsed() < Duration::from_secs(3)) {
            return;
        }
        self.dark_checked = Some(Instant::now());
        match server::dark_mode() {
            Some(true) => ctx.set_theme(egui::ThemePreference::Dark),
            Some(false) => ctx.set_theme(egui::ThemePreference::Light),
            None => ctx.set_theme(egui::ThemePreference::System),
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = &ui.ctx().clone();
        self.follow_system_theme(ctx);
        if self.shown.swap(false, Ordering::Relaxed) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }
        if self.snapshot.is_none() {
            // Like a menu: it goes away when something else is clicked, or the icon again.
            let focused = ctx.input(|i| i.viewport().focused).unwrap_or(false);
            if focused {
                self.focused_once = true;
                self.blurred = None;
            } else if self.focused_once && self.blurred.is_none() {
                self.blurred = Some(Instant::now());
            }
            // A moment's grace: a click on the icon takes the focus first and then asks to close the panel.
            if self.toggled.swap(false, Ordering::Relaxed) || self.blurred.is_some_and(|t| t.elapsed() >= Duration::from_millis(300)) {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            if self.blurred.is_some() {
                ctx.request_repaint_after(Duration::from_millis(100));
            }
        }
        if let Some(snapshot) = &mut self.snapshot {
            // A second and a half: the server has answered and the fonts are in place.
            let frames = if snapshot.path.contains("%03d") { std::env::var("DESKSTATS_SNAPSHOT_FRAMES").ok().and_then(|n| n.parse().ok()).unwrap_or(1) } else { 1usize };
            let due = Duration::from_millis(1500 + 500 * snapshot.taken as u64);
            if !snapshot.asked && snapshot.at.elapsed() >= due {
                snapshot.taken += 1;
                snapshot.asked = snapshot.taken >= frames;
                let last = snapshot.asked;
                let path = snapshot.path.replace("%03d", &format!("{:03}", snapshot.taken));
                let ctx2 = ctx.clone();
                ctx.request_screenshot(move |image| {
                    let rgba: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
                    let _ = std::fs::write(&path, crate::png::encode(image.size[0] as u32, image.size[1] as u32, &rgba));
                    if last {
                        ctx2.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
            }
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        // The numbers move every second, like on the phone.
        if self.checked.elapsed() >= Duration::from_secs(1) {
            self.refresh(ctx);
        }
        ctx.request_repaint_after(Duration::from_secs(1));
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner()).clone();
        // Pictures for the docs show a made-up computer, not the one they were taken on.
        if let (Some(_), Server::Running(status)) = (&self.snapshot, &mut state) {
            if let Ok(name) = std::env::var("DESKSTATS_SNAPSHOT_NAME") {
                status.name = name;
                status.addresses = vec![String::from("192.168.1.20")];
            }
        }
        let w = self.words;
        let stats = self.stats.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let history: Vec<Point> = self.history.lock().unwrap_or_else(|e| e.into_inner()).iter().copied().collect();
        // The panel draws its own rounded body on a see-through window.
        let v = ctx.global_style().visuals.clone();
        let frame = egui::Frame::new().fill(v.panel_fill).corner_radius(12.0).stroke(v.widgets.noninteractive.bg_stroke).inner_margin(14);
        // Room below the icon on this screen, for the panel's content.
        let screen = ctx.input(|i| i.viewport().monitor_size).unwrap_or(Vec2::splat(10_000.0));
        let anchor = self.anchor;
        let up = anchor.up(screen.y);
        let top = if anchor.y.is_finite() { anchor.y } else { 30.0 };
        let room = if up { top - 40.0 } else { screen.y - top - 40.0 };
        let mut content_high = 0.0;
        egui::CentralPanel::default().frame(frame).show(ui, |ui| {
            let release = self.release.lock().unwrap_or_else(|e| e.into_inner()).clone();
            let mut content = |ui: &mut egui::Ui| {
                header(ui, w, &state);
                if let Some(log) = &self.update_failed {
                    ui.add_space(8.0);
                    inner_card(ui, |ui| {
                        ui.add(egui::Label::new(RichText::new(w.update_failed.replace("{}", log)).color(RED).size(12.0)).wrap());
                    });
                }
                if let Some(version) = &release {
                    ui.add_space(8.0);
                    inner_card(ui, |ui| {
                        ui.add(egui::Label::new(RichText::new(w.update_available.replace("{}", version.trim_start_matches('v'))).strong()).wrap());
                        if !self.updating && ui.button(RichText::new(w.update).color(BLUE)).clicked() {
                            self.updating = server::update();
                            if !self.updating {
                                *self.failed.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
                            }
                        }
                        if self.updating {
                            ui.label(RichText::new(w.updating).size(12.0).weak());
                        }
                    });
                }
                if let Some(n) = &stats {
                    ui.add_space(10.0);
                    rings(ui, w, n);
                    section(ui, w.history);
                    chart(ui, &history);
                    ui.add_space(2.0);
                    ui.horizontal(|ui| {
                        legend(ui, BLUE, "CPU");
                        legend(ui, PURPLE, "RAM");
                    });
                    if n.disk.is_some() || n.disk_rates.is_some() {
                        ui.add_space(10.0);
                        disk_card(ui, w, n, &history);
                    }
                    ui.add_space(8.0);
                    net_card(ui, w, n, &history);
                    section(ui, w.details);
                    details(ui, w, n);
                }
                if state == Server::Stopped {
                    ui.add_space(10.0);
                    ui.vertical_centered(|ui| {
                        if ui.add(egui::Button::new(RichText::new(w.start).strong()).min_size(Vec2::new(120.0, 30.0))).clicked() {
                            self.act(ctx, server::start);
                        }
                    });
                }
                if let Server::Running(status) = &state {
                    section(ui, w.computer);
                    connection(ui, w, status);
                }
                // How to connect a phone, while none has been: later it would only take room.
                let paired = matches!(&state, Server::Running(s) if s.wifi || s.phone_ago.is_some());
                if !paired {
                    section(ui, w.connect);
                    for (number, step) in w.steps.iter().enumerate() {
                        ui.horizontal_top(|ui| {
                            ui.label(RichText::new(format!("{}", number + 1)).color(BLUE).strong());
                            ui.add(egui::Label::new(*step).wrap());
                        });
                    }
                    ui.add_space(4.0);
                    let copied = self.copied.is_some_and(|t| t.elapsed() < Duration::from_secs(2));
                    if ui.button(if copied { w.copied } else { w.copy }).clicked() {
                        ctx.copy_text(server::APP_LINK.to_string());
                        self.copied = Some(Instant::now());
                    }
                }
                ui.add_space(10.0);
                ui.separator();
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().button_padding = Vec2::new(8.0, 4.0);
                    if matches!(state, Server::Running(_) | Server::Old) && ui.button(RichText::new(w.restart).size(12.0)).clicked() {
                        self.act(ctx, server::restart);
                    }
                    if ui.button(RichText::new(w.forget).size(12.0)).clicked() {
                        self.asking_to_forget = true;
                    }
                    // Quits the icon too; it comes back at the next sign-in.
                    if ui.button(RichText::new(w.quit).size(12.0)).clicked() {
                        if let Ok(mut stream) = TcpStream::connect((Ipv4Addr::LOCALHOST, crate::tray::TRAY_PORT)) {
                            let _ = stream.write_all(b"quit");
                        }
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
                if self.asking_to_forget {
                    ui.add_space(6.0);
                    ui.add(egui::Label::new(w.forget_question).wrap());
                    ui.horizontal(|ui| {
                        if ui.button(RichText::new(w.forget_yes).color(RED)).clicked() {
                            self.asking_to_forget = false;
                            self.act(ctx, server::forget_phones);
                        }
                        if ui.button(w.cancel).clicked() {
                            self.asking_to_forget = false;
                        }
                    });
                }
                if self.failed.lock().unwrap_or_else(|e| e.into_inner()).is_some_and(|t| t.elapsed() < Duration::from_secs(4)) {
                    ui.colored_label(RED, w.failed);
                }
                ui.add_space(6.0);
                ui.vertical_centered(|ui| ui.label(RichText::new(format!("Deskstats {}", crate::VERSION)).small().weak()));
            };
            {
                let out = egui::ScrollArea::vertical().max_height((room - 30.0).max(200.0)).show(ui, |ui| {
                    ui.set_max_width(ui.available_width() - 2.0);
                    content(ui);
                });
                content_high = out.content_size.y;
            }
        });
        {
            // The panel is as high as what it shows (up to the room on the screen), and hangs from the icon.
            let high = (content_high + 30.0).min(room).ceil();
            if (high - self.height).abs() > 1.0 {
                self.height = high;
                let middle = if anchor.x.is_finite() { anchor.x } else { screen.x / 2.0 };
                let x = (middle - PANEL_WIDTH / 2.0).clamp(4.0, (screen.x - PANEL_WIDTH - 4.0).max(4.0));
                let y = if up { top - 6.0 - high } else { top + 6.0 };
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(Vec2::new(PANEL_WIDTH, high)));
                ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(x, y.max(0.0))));
            }
        }
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        // The panel's corners are rounded: around them the window shows through.
        [0.0; 4]
    }
}

fn header(ui: &mut egui::Ui, w: &Words, state: &Server) {
    let (colour, text) = match state {
        Server::Unknown => (Color32::GRAY, w.checking),
        Server::Stopped => (RED, w.stopped),
        Server::Old => (AMBER, w.old),
        Server::Running(_) => (GREEN, w.running),
    };
    ui.vertical_centered(|ui| {
        ui.label(RichText::new("Deskstats").strong().size(17.0));
        let name = match state {
            Server::Running(status) if !status.name.is_empty() => format!("{} · {text}", status.name),
            _ => text.to_string(),
        };
        ui.horizontal(|ui| {
            // Centred by hand: a dot and a label of unknown width.
            let galley = ui.painter().layout_no_wrap(name.clone(), egui::FontId::proportional(12.0), Color32::GRAY);
            let width = galley.size().x + 14.0;
            ui.add_space(((ui.available_width() - width) / 2.0).max(0.0));
            let (rect, _) = ui.allocate_exact_size(Vec2::splat(10.0), egui::Sense::hover());
            ui.painter().circle_filled(rect.center(), 4.0, colour);
            ui.add(egui::Label::new(RichText::new(name).size(12.0).weak()).truncate());
        });
    });
}

/// A thin line with a small grey title in capitals in its middle.
fn section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(9.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 16.0), egui::Sense::hover());
    let painter = ui.painter();
    let colour = ui.visuals().weak_text_color();
    let line = ui.visuals().widgets.noninteractive.bg_stroke.color;
    let galley = painter.layout_no_wrap(title.to_uppercase(), egui::FontId::proportional(10.5), colour);
    let half = galley.size().x / 2.0 + 8.0;
    let (y, mid) = (rect.center().y, rect.center().x);
    painter.line_segment([Pos2::new(rect.left(), y), Pos2::new(mid - half, y)], Stroke::new(1.0, line));
    painter.line_segment([Pos2::new(mid + half, y), Pos2::new(rect.right(), y)], Stroke::new(1.0, line));
    painter.galley(Pos2::new(mid - galley.size().x / 2.0, y - galley.size().y / 2.0), galley, colour);
    ui.add_space(6.0);
}

/// Rings of the load: CPU, memory, disk and, where there is one, the graphics chip.
fn rings(ui: &mut egui::Ui, w: &Words, n: &Stats) {
    let mut list: Vec<(f32, String, &str, Color32)> = vec![(n.cpu, percent(n.cpu), "CPU", BLUE), (n.ram_pct, percent(n.ram_pct), "RAM", PURPLE)];
    if let Some((total, free)) = n.disk {
        let used = (total.saturating_sub(free)) as f32 * 100.0 / total as f32;
        list.push((used, percent(used), w.disk, TEAL));
    }
    if let Some(gpu) = n.gpu {
        list.push((gpu, percent(gpu), "GPU", GREEN));
    }
    let size = 58.0;
    let gap = ((ui.available_width() - size * list.len() as f32) / (list.len() as f32 + 1.0)).max(4.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        for (value, text, label, colour) in list {
            ui.add_space(gap);
            ring(ui, value, &text, label, colour, size);
        }
    });
}

fn ring(ui: &mut egui::Ui, value: f32, text: &str, label: &str, colour: Color32, size: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(size, size + 18.0), egui::Sense::hover());
    let painter = ui.painter();
    let centre = Pos2::new(rect.center().x, rect.top() + size / 2.0);
    let radius = size / 2.0 - 4.0;
    let track = ui.visuals().widgets.noninteractive.bg_stroke.color;
    painter.circle_stroke(centre, radius, Stroke::new(5.0, track));
    let share = (value / 100.0).clamp(0.0, 1.0);
    if share > 0.0 {
        let steps = (64.0 * share).ceil().max(2.0) as usize;
        let points: Vec<Pos2> = (0..=steps)
            .map(|i| {
                let angle = -std::f32::consts::FRAC_PI_2 + std::f32::consts::TAU * share * i as f32 / steps as f32;
                centre + Vec2::new(angle.cos(), angle.sin()) * radius
            })
            .collect();
        painter.add(egui::Shape::line(points, Stroke::new(5.0, colour)));
    }
    let strong = ui.visuals().strong_text_color();
    painter.text(centre, egui::Align2::CENTER_CENTER, text, egui::FontId::proportional(13.0), strong);
    painter.text(Pos2::new(centre.x, rect.bottom() - 6.0), egui::Align2::CENTER_CENTER, label, egui::FontId::proportional(11.0), ui.visuals().weak_text_color());
}

/// The CPU load as a filled area and the memory as a line, over the last two minutes.
fn chart(ui: &mut egui::Ui, history: &[Point]) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 60.0), egui::Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 6.0, ui.visuals().faint_bg_color);
    if history.len() < 2 {
        return;
    }
    let step = rect.width() / (HISTORY - 1) as f32;
    let start = rect.right() - step * (history.len() - 1) as f32;
    let at = |i: usize, v: f32| Pos2::new(start + step * i as f32, rect.bottom() - 2.0 - (rect.height() - 6.0) * (v / 100.0).clamp(0.0, 1.0));
    let mut mesh = egui::Mesh::default();
    let fill = BLUE.gamma_multiply(0.45);
    for (i, p) in history.iter().enumerate() {
        mesh.colored_vertex(at(i, p.cpu), fill);
        mesh.colored_vertex(Pos2::new(start + step * i as f32, rect.bottom()), BLUE.gamma_multiply(0.08));
        if i > 0 {
            let k = (i * 2) as u32;
            mesh.add_triangle(k - 2, k - 1, k);
            mesh.add_triangle(k - 1, k, k + 1);
        }
    }
    painter.add(egui::Shape::mesh(mesh));
    painter.add(egui::Shape::line(history.iter().enumerate().map(|(i, p)| at(i, p.cpu)).collect(), Stroke::new(1.5, BLUE)));
    painter.add(egui::Shape::line(history.iter().enumerate().map(|(i, p)| at(i, p.ram)).collect(), Stroke::new(1.5, PURPLE)));
}

fn legend(ui: &mut egui::Ui, colour: Color32, label: &str) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(10.0), egui::Sense::hover());
    ui.painter().circle_filled(rect.center(), 4.0, colour);
    ui.label(RichText::new(label).size(11.0).weak());
    ui.add_space(6.0);
}

/// A row: a coloured dot, the name on the left and the value on the right.
fn row(ui: &mut egui::Ui, colour: Color32, label: &str, value: &str) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(Vec2::new(12.0, 18.0), egui::Sense::hover());
        ui.painter().rect_filled(egui::Rect::from_center_size(rect.center(), Vec2::splat(9.0)), 2.5, colour);
        ui.label(label);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add(egui::Label::new(RichText::new(value).strong()).truncate());
        });
    });
}

/// A box like a volume in a disk utility: two speeds on the left, their chart on the right, and below a bar of how full
/// it is with the free space.
fn disk_card(ui: &mut egui::Ui, w: &Words, n: &Stats, history: &[Point]) {
    inner_card(ui, |ui| {
        ui.label(RichText::new(w.disk).strong());
        ui.add_space(4.0);
        let (read, write) = n.disk_rates.unwrap_or((0.0, 0.0));
        speeds(ui, (RED, w.disk_read, read), (BLUE, w.disk_write, write), history, |p| (p.read, p.write));
        if let Some((total, free)) = n.disk {
            ui.add_space(6.0);
            let used = total.saturating_sub(free) as f32 / total.max(1) as f32;
            let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 6.0), egui::Sense::hover());
            ui.painter().rect_filled(rect, 3.0, ui.visuals().widgets.noninteractive.bg_stroke.color);
            let fill = egui::Rect::from_min_size(rect.min, Vec2::new(rect.width() * used.clamp(0.0, 1.0), rect.height()));
            ui.painter().rect_filled(fill, 3.0, if used > 0.9 { RED } else { BLUE });
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("{} / {}", size(free), size(total))).size(12.0).weak());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(format!("{:.0}%", (1.0 - used) * 100.0)).size(12.0).weak());
                });
            });
        }
    });
}

/// The same box for the network: download and upload, and the ping.
fn net_card(ui: &mut egui::Ui, w: &Words, n: &Stats, history: &[Point]) {
    inner_card(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Wi-Fi / Ethernet").strong());
            if let Some(ms) = n.ping {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(format!("{} {ms} ms", w.ping)).size(12.0).weak());
                });
            }
        });
        ui.add_space(4.0);
        speeds(ui, (BLUE, w.download, n.down), (RED, w.upload, n.up), history, |p| (p.down, p.up));
    });
}

/// Two speeds as coloured squares with values, and a chart of both mirrored around a middle line.
fn speeds(ui: &mut egui::Ui, first: (Color32, &str, f64), second: (Color32, &str, f64), history: &[Point], pick: fn(&Point) -> (f32, f32)) {
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.set_width(ui.available_width() * 0.5);
            for (colour, label, value) in [first, second] {
                ui.horizontal(|ui| {
                    let (rect, _) = ui.allocate_exact_size(Vec2::new(12.0, 16.0), egui::Sense::hover());
                    ui.painter().rect_filled(egui::Rect::from_center_size(rect.center(), Vec2::splat(9.0)), 2.5, colour);
                    ui.label(RichText::new(rate(value)).strong()).on_hover_text(label);
                });
            }
        });
        let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 40.0), egui::Sense::hover());
        let values: Vec<(f32, f32)> = history.iter().map(pick).collect();
        let top = values.iter().fold(1.0f32, |m, (a, b)| m.max(*a).max(*b));
        let painter = ui.painter();
        let mid = rect.center().y;
        painter.line_segment([Pos2::new(rect.left(), mid), Pos2::new(rect.right(), mid)], Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color));
        if values.len() < 2 {
            return;
        }
        let step = rect.width() / (HISTORY - 1) as f32;
        let start = rect.right() - step * (values.len() - 1) as f32;
        for (half, colour, up) in [(0usize, first.0, true), (1, second.0, false)] {
            let mut mesh = egui::Mesh::default();
            for (i, v) in values.iter().enumerate() {
                let value = if half == 0 { v.0 } else { v.1 };
                let reach = rect.height() / 2.0 * (value / top).clamp(0.0, 1.0);
                let x = start + step * i as f32;
                mesh.colored_vertex(Pos2::new(x, if up { mid - reach } else { mid + reach }), colour.gamma_multiply(0.85));
                mesh.colored_vertex(Pos2::new(x, mid), colour.gamma_multiply(0.25));
                if i > 0 {
                    let k = (i * 2) as u32;
                    mesh.add_triangle(k - 2, k - 1, k);
                    mesh.add_triangle(k - 1, k, k + 1);
                }
            }
            painter.add(egui::Shape::mesh(mesh));
        }
    });
}

fn inner_card(ui: &mut egui::Ui, content: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new().fill(ui.visuals().faint_bg_color).corner_radius(8.0).inner_margin(10).show(ui, |ui| {
        ui.set_width(ui.available_width());
        content(ui);
    });
}

fn details(ui: &mut egui::Ui, w: &Words, n: &Stats) {
    if !n.cpu_name.is_empty() {
        row(ui, BLUE, "CPU", &n.cpu_name);
    }
    if let Some(t) = n.temp {
        row(ui, AMBER, w.temperature, &format!("{t:.0} °C"));
    }
    if let Some(s) = n.uptime {
        row(ui, GREEN, w.uptime, &uptime(w, s));
    }
    row(ui, PURPLE, "RAM", &format!("{} / {}", size(n.ram_used), size(n.ram_total)));
    if let Some(mem) = n.gpu_mem {
        row(ui, GREEN, w.gpu_memory, &size(mem));
    }
}

fn connection(ui: &mut egui::Ui, w: &Words, status: &Status) {
    row(ui, BLUE, w.name, &status.name);
    let address = if status.addresses.is_empty() { w.no_network.to_string() } else { status.addresses.join(", ") };
    row(ui, if status.addresses.is_empty() { AMBER } else { BLUE }, w.address, &address);
    row(ui, if status.wifi { GREEN } else { Color32::GRAY }, w.wifi, if status.wifi { w.wifi_on } else { w.wifi_off });
    row(ui, if status.adb { GREEN } else { Color32::GRAY }, w.usb, if status.adb { w.usb_on } else { w.usb_off });
    let phone = match status.phone_ago {
        Some(ago) if ago < Duration::from_secs(10) => w.phone_now.to_string(),
        Some(ago) => w.phone_ago.replace("{}", &how_long(w, ago)),
        None => w.phone_never.to_string(),
    };
    let near = status.phone_ago.is_some_and(|ago| ago < Duration::from_secs(10));
    row(ui, if near { GREEN } else { Color32::GRAY }, w.phone, &phone);
}

fn percent(value: f32) -> String {
    format!("{value:.0}%")
}

/// Bytes as the largest unit that keeps the number at least 1.
pub fn size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 || value >= 100.0 {
        format!("{value:.0} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

pub fn rate(bytes_per_second: f64) -> String {
    format!("{}/s", size(bytes_per_second.max(0.0) as u64))
}

pub fn uptime(w: &Words, seconds: u64) -> String {
    let (days, hours, minutes) = (seconds / 86_400, seconds % 86_400 / 3_600, seconds % 3_600 / 60);
    if days > 0 {
        format!("{days} {} {hours} {}", w.days, w.hours)
    } else {
        format!("{hours} {} {minutes} {}", w.hours, w.minutes)
    }
}

pub fn how_long(w: &Words, time: Duration) -> String {
    let seconds = time.as_secs();
    match seconds {
        0..60 => format!("{seconds} {}", w.seconds),
        60..3600 => format!("{} {}", seconds / 60, w.minutes),
        _ => format!("{} {}", seconds / 3600, w.hours),
    }
}

/// The same look everywhere: the fonts that come with the window, a little more room and the teal of the icon.
fn style(ctx: &egui::Context) {
    for theme in [egui::Theme::Dark, egui::Theme::Light] {
        ctx.style_mut_of(theme, |style| {
            style.spacing.item_spacing = Vec2::new(8.0, 6.0);
            style.spacing.scroll = egui::style::ScrollStyle::solid();
            style.spacing.button_padding = Vec2::new(12.0, 6.0);
            style.visuals.selection.bg_fill = TEAL.linear_multiply(0.6);
            style.visuals.hyperlink_color = TEAL;
            style.text_styles.insert(egui::TextStyle::Body, egui::FontId::proportional(14.0));
            style.text_styles.insert(egui::TextStyle::Button, egui::FontId::proportional(14.0));
        });
    }
}

/// The fonts that come with egui have no Chinese or Japanese; for those a font of the system is added.
fn fonts(ctx: &egui::Context, language: Language) {
    if !matches!(language, Language::Zh | Language::Ja) {
        return;
    }
    let candidates = [
        "/System/Library/Fonts/PingFang.ttc",
        "/System/Library/Fonts/Hiragino Sans GB.ttc",
        "C:\\Windows\\Fonts\\msyh.ttc",
        "C:\\Windows\\Fonts\\YuGothM.ttc",
        "C:\\Windows\\Fonts\\meiryo.ttc",
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/google-noto-cjk/NotoSansCJK-Regular.ttc",
    ];
    let Some(bytes) = candidates.iter().find_map(|path| std::fs::read(path).ok()) else { return };
    let mut definitions = egui::FontDefinitions::default();
    definitions.font_data.insert("cjk".into(), Arc::new(egui::FontData::from_owned(bytes)));
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        definitions.families.entry(family).or_default().push("cjk".into());
    }
    ctx.set_fonts(definitions);
}
