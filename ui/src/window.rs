//! The window: how the server is, the code for the phone, and the few things one can do to the server.

use crate::server::{self, Server, Status};
use crate::text::{self, Language, Words};
use eframe::egui::{self, Color32, RichText, Vec2};
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

pub fn run() {
    open(None);
}

/// Opens the window, waits for the server's answer, saves a picture of it to [path] and quits.
pub fn run_snapshot(path: String, theme: Option<String>) {
    let theme = match theme.as_deref() {
        Some("dark") => Some(egui::ThemePreference::Dark),
        Some("light") => Some(egui::ThemePreference::Light),
        _ => None,
    };
    open(Some(Snapshot { path, theme, asked: false, at: Instant::now() }));
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
}

fn open(snapshot: Option<Snapshot>) {
    // A picture is taken by a window of its own, even while the real one is open.
    let port = if snapshot.is_some() { 0 } else { WINDOW_PORT };
    let Ok(listener) = TcpListener::bind((Ipv4Addr::LOCALHOST, port)) else {
        // Already open: it is asked to come to the front. On Windows only a program the user just used may do
        // that, which this one is and the open window is not, so it is let.
        #[cfg(windows)]
        allow_any_to_come_forward();
        if let Ok(mut stream) = TcpStream::connect((Ipv4Addr::LOCALHOST, WINDOW_PORT)) {
            let _ = stream.write_all(b"show");
        }
        return;
    };
    let size = 64;
    let icon = egui::IconData { rgba: crate::icon::app(size), width: size, height: size };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Deskstats")
            .with_app_id("deskstats")
            .with_inner_size([380.0, 720.0])
            .with_min_inner_size([340.0, 480.0])
            .with_icon(icon),
        centered: true,
        ..Default::default()
    };
    let _ = eframe::run_native("Deskstats", options, Box::new(move |cc| Ok(Box::new(App::new(cc, listener, snapshot)))));
}

struct App {
    words: &'static Words,
    state: Arc<Mutex<Server>>,
    asking: Arc<AtomicBool>,
    shown: Arc<AtomicBool>,
    qr: Option<egui::TextureHandle>,
    copied: Option<Instant>,
    failed: Arc<Mutex<Option<Instant>>>,
    asking_to_forget: bool,
    checked: Instant,
    dark_checked: Option<Instant>,
    snapshot: Option<Snapshot>,
    fitted: bool,
}

impl App {
    fn new(cc: &eframe::CreationContext, listener: TcpListener, snapshot: Option<Snapshot>) -> Self {
        let language = text::system_language();
        fonts(&cc.egui_ctx, language);
        style(&cc.egui_ctx);
        // Started from the icon, the window comes to the front at once, as when it is asked to later.
        let shown = Arc::new(AtomicBool::new(snapshot.is_none()));
        {
            let (shown, ctx) = (shown.clone(), cc.egui_ctx.clone());
            std::thread::spawn(move || {
                for mut stream in listener.incoming().flatten() {
                    let mut buf = [0u8; 4];
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(1)));
                    if stream.read_exact(&mut buf).is_ok() && &buf == b"show" {
                        shown.store(true, Ordering::Relaxed);
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
            qr: None,
            copied: None,
            failed: Arc::default(),
            asking_to_forget: false,
            checked: Instant::now(),
            dark_checked: None,
            snapshot,
            fitted: false,
        };
        app.refresh(&cc.egui_ctx);
        app
    }

    /// Asks the server again, off the thread that draws.
    fn refresh(&mut self, ctx: &egui::Context) {
        self.checked = Instant::now();
        if self.asking.swap(true, Ordering::Relaxed) {
            return;
        }
        let (state, asking, ctx) = (self.state.clone(), self.asking.clone(), ctx.clone());
        std::thread::spawn(move || {
            let now = server::ask();
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
            for _ in 0..20 {
                std::thread::sleep(Duration::from_millis(250));
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

    fn qr(&mut self, ctx: &egui::Context) -> Option<egui::TextureHandle> {
        if self.qr.is_none() {
            let code = crate::qr::Qr::encode(server::APP_LINK)?;
            let (side, quiet) = (code.size, 3);
            let full = side + quiet * 2;
            let mut pixels = Vec::with_capacity(full * full);
            for y in 0..full {
                for x in 0..full {
                    let dark = x >= quiet && y >= quiet && x < side + quiet && y < side + quiet && code.dark(x - quiet, y - quiet);
                    pixels.push(if dark { Color32::BLACK } else { Color32::WHITE });
                }
            }
            let image = egui::ColorImage { size: [full, full], pixels, source_size: Vec2::new(full as f32, full as f32) };
            self.qr = Some(ctx.load_texture("qr", image, egui::TextureOptions::NEAREST));
        }
        self.qr.clone()
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
        if !self.fitted && self.snapshot.is_none() {
            // On a low screen the window would reach under the taskbar: it is made to fit, once it knows the screen.
            if let Some(screen) = ctx.input(|i| i.viewport().monitor_size) {
                self.fitted = true;
                let high = (screen.y - 120.0).clamp(480.0, 720.0);
                if high < 720.0 {
                    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(Vec2::new(380.0, high)));
                    ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2((screen.x - 380.0) / 2.0, 30.0)));
                }
            }
        }
        if self.shown.swap(false, Ordering::Relaxed) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }
        if let Some(snapshot) = &mut self.snapshot {
            // A second and a half: the server has answered and the fonts are in place.
            if !snapshot.asked && snapshot.at.elapsed() >= Duration::from_millis(1500) {
                snapshot.asked = true;
                let path = snapshot.path.clone();
                let ctx2 = ctx.clone();
                ctx.request_screenshot(move |image| {
                    let rgba: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
                    let _ = std::fs::write(&path, crate::png::encode(image.size[0] as u32, image.size[1] as u32, &rgba));
                    ctx2.send_viewport_cmd(egui::ViewportCommand::Close);
                });
            }
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        if self.checked.elapsed() >= Duration::from_secs(2) {
            self.refresh(ctx);
        }
        // Nothing moves on its own; two seconds are enough to see a phone come and go.
        ctx.request_repaint_after(Duration::from_secs(2));
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let w = self.words;
        egui::CentralPanel::default().show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.add_space(4.0);
                header(ui, w, &state);
                ui.add_space(12.0);
                if let Server::Running(status) = &state {
                    details(ui, w, status);
                }
                if state == Server::Stopped && ui.add(egui::Button::new(RichText::new(w.start).strong()).min_size(Vec2::new(120.0, 30.0))).clicked() {
                    self.act(ctx, server::restart);
                }
                ui.add_space(16.0);
                card(ui, |ui| {
                    ui.label(RichText::new(w.connect).strong().size(16.0));
                    ui.add_space(8.0);
                    if let Some(qr) = self.qr(ctx) {
                        ui.vertical_centered(|ui| {
                            ui.add(egui::Image::new(&qr).fit_to_exact_size(Vec2::splat(200.0)).corner_radius(6.0));
                        });
                    }
                    ui.add_space(8.0);
                    for (number, step) in w.steps.iter().enumerate() {
                        ui.horizontal_top(|ui| {
                            ui.label(RichText::new(format!("{}", number + 1)).color(TEAL).strong());
                            ui.add(egui::Label::new(*step).wrap());
                        });
                    }
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        let copied = self.copied.is_some_and(|t| t.elapsed() < Duration::from_secs(2));
                        if ui.button(if copied { w.copied } else { w.copy }).clicked() {
                            ctx.copy_text(server::APP_LINK.to_string());
                            self.copied = Some(Instant::now());
                        }
                    });
                });
                ui.add_space(16.0);
                ui.horizontal_wrapped(|ui| {
                    if matches!(state, Server::Running(_) | Server::Old) && ui.button(w.restart).clicked() {
                        self.act(ctx, server::restart);
                    }
                    if ui.button(w.forget).clicked() {
                        self.asking_to_forget = true;
                    }
                });
                if self.asking_to_forget {
                    ui.add_space(8.0);
                    card(ui, |ui| {
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
                    });
                }
                if self.failed.lock().unwrap_or_else(|e| e.into_inner()).is_some_and(|t| t.elapsed() < Duration::from_secs(4)) {
                    ui.colored_label(RED, w.failed);
                }
                ui.add_space(12.0);
                ui.label(RichText::new(format!("Deskstats {}", crate::VERSION)).small().weak());
            });
        });
    }
}

fn header(ui: &mut egui::Ui, w: &Words, state: &Server) {
    let (colour, text) = match state {
        Server::Unknown => (Color32::GRAY, w.checking),
        Server::Stopped => (RED, w.stopped),
        Server::Old => (AMBER, w.old),
        Server::Running(_) => (TEAL, w.running),
    };
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(14.0), egui::Sense::hover());
        ui.painter().circle_filled(rect.center(), 6.0, colour);
        ui.add(egui::Label::new(RichText::new(text).strong().size(18.0)).wrap());
    });
}

fn details(ui: &mut egui::Ui, w: &Words, status: &Status) {
    card(ui, |ui| {
        ui.label(RichText::new(w.computer).strong().size(16.0));
        ui.add_space(6.0);
        egui::Grid::new("details").num_columns(2).spacing([14.0, 6.0]).show(ui, |ui| {
            ui.label(RichText::new(w.name).weak());
            ui.add(egui::Label::new(&status.name).wrap());
            ui.end_row();
            ui.label(RichText::new(w.address).weak());
            if status.addresses.is_empty() {
                ui.colored_label(AMBER, w.no_network);
            } else {
                ui.label(status.addresses.join("\n"));
            }
            ui.end_row();
            ui.label(RichText::new(w.wifi).weak());
            ui.label(if status.wifi { w.wifi_on } else { w.wifi_off });
            ui.end_row();
            ui.label(RichText::new(w.usb).weak());
            ui.label(if status.adb { w.usb_on } else { w.usb_off });
            ui.end_row();
            ui.label(RichText::new(w.phone).weak());
            match status.phone_ago {
                Some(ago) if ago < Duration::from_secs(10) => ui.colored_label(TEAL, w.phone_now),
                Some(ago) => ui.label(w.phone_ago.replace("{}", &how_long(w, ago))),
                None => ui.label(w.phone_never),
            };
            ui.end_row();
        });
    });
}

pub fn how_long(w: &Words, time: Duration) -> String {
    let seconds = time.as_secs();
    match seconds {
        0..60 => format!("{seconds} {}", w.seconds),
        60..3600 => format!("{} {}", seconds / 60, w.minutes),
        _ => format!("{} {}", seconds / 3600, w.hours),
    }
}

fn card(ui: &mut egui::Ui, content: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::group(ui.style()).corner_radius(10.0).inner_margin(14.0).show(ui, |ui| {
        ui.set_width(ui.available_width());
        content(ui);
    });
}

/// The same look everywhere: the fonts that come with the window, a little more room and the teal of the icon.
fn style(ctx: &egui::Context) {
    for theme in [egui::Theme::Dark, egui::Theme::Light] {
        ctx.style_mut_of(theme, |style| {
            style.spacing.item_spacing = Vec2::new(8.0, 6.0);
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
