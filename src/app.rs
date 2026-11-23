//! The window: a card for each paired Bluetooth device, the low-battery
//! banner and the few settings. It only shows what the monitor thread last
//! found, so it never waits for Bluetooth. Closing it hides it; the app
//! lives on in the tray (`tray.rs`). The footer with the auto-update toggle
//! and version is required on every factory app. Colours and type come from
//! `theme.rs`.

use std::sync::{Arc, Mutex};

use eframe::egui::{self, Align, Align2, FontId, Layout, RichText, Stroke};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use crate::autostart;
use crate::devices::{self, Ago, Device, Kind, State};
use crate::i18n::{Lang, Strings, fill};
use crate::license;
use crate::monitor::{self, Monitor, Shared, SharedState, lock};
use crate::theme::{self, Palette};
use crate::tray::{self, Tray};

const WIDTH: f32 = 560.0;
const HEIGHT: f32 = 720.0;
const APP_NAME: &str = "Bluetooth Battery Tray";
/// Space between the window edge and its contents.
const GUTTER: i8 = 16;
/// Where the window is created when the app starts in the tray.
const OFF_SCREEN: [f32; 2] = [-20000.0, -20000.0];
/// The lowest and highest warning threshold the user can set, in percent.
const THRESHOLDS: std::ops::RangeInclusive<u8> = 5..=50;

pub fn run(args: Vec<String>) -> eframe::Result<()> {
    // Started by Windows at sign-in: stay in the tray.
    let in_tray = args.iter().any(|arg| arg == autostart::TRAY_ARG);
    // One copy is enough. A second start brings the first one's window up,
    // unless it is only Windows starting the app at sign-in.
    if tray::running_instance(!in_tray) {
        return Ok(());
    }
    let viewport = egui::ViewportBuilder::default()
        .with_title(APP_NAME)
        .with_inner_size([WIDTH, HEIGHT])
        .with_min_inner_size([460.0, 420.0]);
    // eframe always shows its window after the first frame, so a tray start
    // opens it off-screen and hides it on the second (see `App::ui`).
    let viewport = if in_tray {
        viewport.with_position(OFF_SCREEN)
    } else {
        viewport
    };
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        APP_NAME,
        options,
        Box::new(move |cc| Ok(Box::new(App::new(cc, in_tray)))),
    )
}

pub struct App {
    settings: crate::settings::Settings,
    version: String,
    shared: SharedState,
    monitor: Monitor,
    /// `None` if the tray icon could not be created; closing then quits.
    tray: Option<Tray>,
    /// Frames left until a tray start hides the window; `None` otherwise.
    hide_in: Option<u8>,
    autostart: bool,
    licensed_to: Option<String>,
    unlock_open: bool,
    key_entry: String,
    key_message: Option<String>,
    lang: Lang,
    t: &'static Strings,
}

fn main_window(cc: &eframe::CreationContext<'_>) -> Option<isize> {
    match cc.window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(handle) => Some(handle.hwnd.get()),
        _ => None,
    }
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>, in_tray: bool) -> Self {
        let ctx = &cc.egui_ctx;
        let settings = crate::settings::load();
        let licensed_to = license::verify(&settings.license_key);
        let lang = settings.language.unwrap_or_else(Lang::system);
        theme::apply(ctx);
        theme::install_fonts(ctx, &[lang, Lang::system()]);
        // For checking both looks without changing Windows' setting.
        match std::env::var("HWC_THEME").as_deref() {
            Ok("dark") => ctx.set_theme(egui::Theme::Dark),
            Ok("light") => ctx.set_theme(egui::Theme::Light),
            _ => {}
        }

        let shared: SharedState = Arc::new(Mutex::new(Shared {
            devices: devices::load(),
            polled: false,
            tray_device: settings.tray_device.clone(),
            threshold: settings.threshold,
            licensed: licensed_to.is_some(),
            lang,
            pending: Vec::new(),
        }));
        let tray = main_window(cc).and_then(|window| Tray::start(shared.clone(), window, ctx.clone()));
        if tray.is_none() && in_tray {
            // No tray to open it from: put the window where it can be seen.
            ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(80.0, 80.0)));
        }
        let repaint = ctx.clone();
        let monitor = Monitor::start(crate::source::pick(), shared.clone(), move || {
            if let Some(tray) = tray {
                tray.refresh();
            }
            repaint.request_repaint();
        });

        let autostart = autostart::enabled();
        if autostart {
            // Keep the entry pointing at this copy if the app has moved.
            autostart::set(true);
        }
        Self {
            lang,
            t: lang.strings(),
            settings,
            version: crate::update::current_version(),
            shared,
            monitor,
            tray,
            hide_in: (in_tray && tray.is_some()).then_some(1),
            autostart,
            licensed_to,
            unlock_open: false,
            key_entry: String::new(),
            key_message: None,
        }
    }

    fn save(&self) {
        crate::settings::save(&self.settings);
    }

    /// Hands the settings the monitor and tray depend on to them.
    fn share_settings(&self) {
        {
            let mut shared = lock(&self.shared);
            shared.tray_device = self.settings.tray_device.clone();
            shared.threshold = self.settings.threshold;
            shared.licensed = self.licensed_to.is_some();
            shared.lang = self.lang;
        }
        if let Some(tray) = self.tray {
            tray.refresh();
        }
    }

    fn set_language(&mut self, lang: Lang, ctx: &egui::Context) {
        self.lang = lang;
        self.t = lang.strings();
        self.key_message = None;
        theme::install_fonts(ctx, &[lang, Lang::system()]);
        self.settings.language = Some(lang);
        self.save();
        self.share_settings();
    }

    /// Whether the free tier's one-device limit applies.
    fn limited(&self) -> bool {
        self.licensed_to.is_none() && license::paid_tier_enabled()
    }

    fn toolbar(&mut self, ui: &mut egui::Ui, p: Palette) {
        let t = self.t;
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(t.devices_title)
                    .font(FontId::new(20.0, theme::semibold()))
                    .color(p.text),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if let Some(email) = &self.licensed_to {
                    ui.label(
                        RichText::new(fill(t.unlocked_for, &[("email", email)]))
                            .small()
                            .color(p.muted),
                    );
                } else if license::paid_tier_enabled() {
                    let unlock = egui::Button::new(RichText::new(t.unlock).color(p.accent))
                        .stroke(Stroke::new(1.0, p.accent));
                    if ui.add(unlock).clicked() {
                        self.unlock_open = true;
                    }
                }
                if ui.button(t.refresh).clicked() {
                    self.monitor.refresh();
                }
            });
        });
    }

    /// One card per device that is low right now and gets warnings.
    fn warnings(&self, ui: &mut egui::Ui, p: Palette, devices: &[Device]) {
        let t = self.t;
        let low = devices::low_now(
            devices,
            self.settings.threshold,
            self.settings.tray_device.as_deref(),
            self.licensed_to.is_some(),
        );
        for (device, level) in low {
            theme::card(p)
                .stroke(Stroke::new(1.5, p.error))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal_wrapped(|ui| {
                        ui.label(
                            RichText::new(t.low_title)
                                .family(theme::semibold())
                                .color(p.error),
                        );
                        ui.label(fill(
                            t.low_body,
                            &[("name", &device.name), ("level", &level.to_string())],
                        ));
                    });
                });
        }
    }

    fn device_card(&mut self, ui: &mut egui::Ui, p: Palette, device: &Device, in_tray: bool, now: u64) {
        let t = self.t;
        let state = device.state();
        let threshold = self.settings.threshold;
        let low = |level: u8| level <= threshold;
        theme::card(p).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                theme::badge(ui, kind_label(device.kind, t), p.accent, p.accent_soft);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    // The number: live in full colour, remembered in grey.
                    let number = match state {
                        State::Live(level) => Some((level, if low(level) { p.error } else { p.text })),
                        State::Off(Some((level, _))) => Some((level, p.muted)),
                        _ => None,
                    };
                    if let Some((level, colour)) = number {
                        ui.label(
                            RichText::new(format!("{level}%"))
                                .font(FontId::new(22.0, theme::semibold()))
                                .color(colour),
                        );
                    }
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        ui.add(
                            egui::Label::new(
                                RichText::new(&device.name)
                                    .font(FontId::new(16.0, theme::semibold()))
                                    .color(p.text),
                            )
                            .truncate(),
                        );
                    });
                });
            });

            match state {
                State::Live(level) => {
                    level_bar(ui, p, level, if low(level) { p.error } else { p.success });
                }
                State::Off(Some((level, _))) => level_bar(ui, p, level, p.muted),
                _ => {}
            }

            ui.horizontal(|ui| {
                // A small line: not the full height of a button row.
                ui.spacing_mut().interact_size.y = 20.0;
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let label = if self.limited() {
                        t.tray_choice_free
                    } else {
                        t.tray_choice
                    };
                    let label = RichText::new(label).small().color(p.muted);
                    if ui.radio(in_tray, label).clicked() && !in_tray {
                        self.settings.tray_device = Some(device.id.clone());
                        self.save();
                        self.share_settings();
                        // A device that is already low warns straight away.
                        self.monitor.refresh();
                    }
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        let colour = match state {
                            State::Unreadable => p.warn,
                            _ => p.muted,
                        };
                        let text = status_text(state, device.le, now, t);
                        ui.add(egui::Label::new(RichText::new(&text).small().color(colour)).truncate())
                            .on_hover_text(text);
                    });
                });
            });
        });
    }

    fn list(&mut self, ui: &mut egui::Ui, p: Palette, devices: &[Device], polled: bool) {
        let t = self.t;
        if devices.is_empty() {
            let rect = ui.available_rect_before_wrap();
            ui.allocate_rect(rect, egui::Sense::hover());
            ui.painter().rect_filled(rect, theme::CARD_RADIUS, p.surface);
            ui.painter().rect_stroke(
                rect,
                theme::CARD_RADIUS,
                Stroke::new(1.0, p.border),
                egui::StrokeKind::Inside,
            );
            let text = if polled { t.no_devices } else { t.searching };
            let galley = ui.painter().layout(
                text.to_owned(),
                egui::TextStyle::Body.resolve(ui.style()),
                p.muted,
                (rect.width() - 48.0).max(100.0),
            );
            let corner = rect.center() - galley.size() / 2.0;
            ui.painter().galley(corner, galley, p.muted);
            return;
        }
        let now = monitor::now();
        let chosen = self.settings.tray_device.as_deref();
        let in_tray = devices::tray_device(devices, chosen).map(|d| d.id.clone());
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                self.warnings(ui, p, devices);
                for device in devices {
                    let chosen = in_tray.as_deref() == Some(device.id.as_str());
                    self.device_card(ui, p, device, chosen, now);
                }
            });
    }

    fn settings_card(&mut self, ui: &mut egui::Ui, p: Palette) {
        let t = self.t;
        theme::card(p).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(t.warn_at).color(p.muted));
                let threshold = ui.add(
                    egui::DragValue::new(&mut self.settings.threshold)
                        .range(THRESHOLDS)
                        .speed(0.2)
                        .suffix("%"),
                );
                if threshold.changed() {
                    self.save();
                    self.share_settings();
                }
                // Once the value has settled, look again: a device may now
                // be at or under the threshold.
                if threshold.drag_stopped() || (threshold.changed() && !threshold.dragged()) {
                    self.monitor.refresh();
                }
            });
            if theme::checkbox(ui, &mut self.autostart, t.autostart).changed() {
                autostart::set(self.autostart);
                self.autostart = autostart::enabled();
            }
            if self.limited() {
                ui.label(RichText::new(t.plan).small().color(p.muted));
            }
        });
    }

    fn unlock_window(&mut self, ctx: &egui::Context, p: Palette) {
        if !self.unlock_open {
            return;
        }
        let mut open_flag = true;
        let t = self.t;
        egui::Window::new(RichText::new(t.unlock_title).family(theme::semibold()))
            .id(egui::Id::new("unlock"))
            .open(&mut open_flag)
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .frame(
                egui::Frame::window(&ctx.global_style()).inner_margin(egui::Margin::same(18)),
            )
            .show(ctx, |ui| {
                ui.set_width(400.0);
                ui.label(t.plan);
                ui.add_space(4.0);
                let buy = fill(t.buy, &[("price", license::PRICE)]);
                if ui.add(theme::primary_button(p, buy, true)).clicked() {
                    open(license::BUY_URL);
                }
                ui.add_space(10.0);
                ui.label(RichText::new(t.paste_key).color(p.muted));
                ui.add(
                    egui::TextEdit::singleline(&mut self.key_entry)
                        .hint_text("BTT1.…")
                        .desired_width(f32::INFINITY),
                );
                if ui.button(t.activate).clicked() {
                    match license::verify(&self.key_entry) {
                        Some(email) => {
                            self.settings.license_key = self.key_entry.trim().to_owned();
                            self.save();
                            self.key_message = Some(fill(t.unlocked_thanks, &[("email", &email)]));
                            self.licensed_to = Some(email);
                            // Every device gets warnings from now on.
                            self.share_settings();
                            self.monitor.refresh();
                        }
                        None => self.key_message = Some(t.bad_key.to_owned()),
                    }
                }
                if let Some(message) = &self.key_message {
                    let colour = if self.licensed_to.is_some() {
                        p.success
                    } else {
                        p.error
                    };
                    ui.label(RichText::new(message).color(colour));
                }
            });
        if !open_flag {
            self.unlock_open = false;
            self.key_message = None;
        }
    }

    fn footer(&mut self, ui: &mut egui::Ui, p: Palette) {
        ui.horizontal(|ui| {
            let mut auto_update = self.settings.auto_update;
            let label = RichText::new(self.t.auto_update).small().color(p.muted);
            if theme::checkbox(ui, &mut auto_update, label)
                .on_hover_text(self.t.auto_update_hint)
                .changed()
            {
                self.settings.auto_update = auto_update;
                crate::settings::save(&self.settings);
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(
                    RichText::new(format!("v{}", self.version))
                        .small()
                        .color(p.muted),
                );
                let mut chosen = self.lang;
                egui::ComboBox::from_id_salt("language")
                    .selected_text(RichText::new(self.lang.name()).small())
                    .show_ui(ui, |ui| {
                        for lang in Lang::ALL {
                            ui.selectable_value(&mut chosen, lang, lang.name());
                        }
                    })
                    .response
                    .on_hover_text(self.t.language);
                if chosen != self.lang {
                    self.set_language(chosen, ui.ctx());
                }
            });
        });
    }
}

fn kind_label(kind: Kind, t: &Strings) -> &'static str {
    match kind {
        Kind::Headphones => t.kind_headphones,
        Kind::Mouse => t.kind_mouse,
        Kind::Keyboard => t.kind_keyboard,
        Kind::Gamepad => t.kind_gamepad,
        Kind::Other => t.kind_other,
    }
}

fn ago_text(ago: Ago, t: &Strings) -> String {
    match ago {
        Ago::Now => t.ago_now.to_owned(),
        Ago::Minutes(n) => fill(t.ago_minutes, &[("n", &n.to_string())]),
        Ago::Hours(n) => fill(t.ago_hours, &[("n", &n.to_string())]),
        Ago::Days(n) => fill(t.ago_days, &[("n", &n.to_string())]),
    }
}

/// The small line under a device: what is known, and nothing that is not.
fn status_text(state: State, le: bool, now: u64, t: &Strings) -> String {
    let transport = if le { "Bluetooth LE" } else { "Bluetooth" };
    match state {
        State::Live(_) => format!("{} · {transport}", t.connected),
        State::NotReported => t.no_report.to_owned(),
        State::Unreadable => t.unreadable.to_owned(),
        State::Off(Some((level, seen))) => {
            let when = ago_text(devices::ago(now, seen), t);
            let last = fill(t.off_last, &[("level", &level.to_string()), ("when", &when)]);
            format!("{} · {last}", t.off)
        }
        State::Off(None) => format!("{} · {}", t.off, t.never_seen),
    }
}

/// A rounded bar filled to `level` percent.
fn level_bar(ui: &mut egui::Ui, p: Palette, level: u8, colour: egui::Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 8.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, 4, p.track);
    let mut filled = rect;
    // A sliver stays visible at 0-2% so the bar never looks absent.
    filled.set_width((rect.width() * f32::from(level.min(100)) / 100.0).max(8.0));
    ui.painter().rect_filled(filled, 4, colour);
}

/// Opens a web page with whatever Windows uses for it.
fn open(target: &str) {
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    use windows::core::{HSTRING, w};
    unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            &HSTRING::from(target),
            None,
            None,
            SW_SHOWNORMAL,
        );
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui_root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui_root.ctx().clone();
        // Closing the window hides it; Quit is in the tray menu.
        if let Some(tray) = self.tray
            && ctx.input(|input| input.viewport().close_requested())
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            tray.hide_main();
        }
        // A tray start: the window was created off-screen. Once eframe has
        // shown it, hide it and move it to where Open should bring it up.
        if let (Some(tray), Some(frames)) = (self.tray, self.hide_in) {
            if frames == 0 {
                self.hide_in = None;
                tray.hide_main();
                let (monitor, size) = ctx.input(|input| {
                    let viewport = input.viewport();
                    (viewport.monitor_size, viewport.outer_rect.map(|r| r.size()))
                });
                let position = match (monitor, size) {
                    (Some(monitor), Some(size)) => ((monitor - size) / 2.0).max(egui::Vec2::ZERO).to_pos2(),
                    _ => egui::pos2(80.0, 80.0),
                };
                ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(position));
            } else {
                self.hide_in = Some(frames - 1);
                ctx.request_repaint();
            }
        }
        // A copy, so the lock is not held while drawing.
        let (devices, polled) = {
            let shared = lock(&self.shared);
            (shared.devices.clone(), shared.polled)
        };
        // Keeps "5 min ago" moving while the window is open.
        ctx.request_repaint_after(std::time::Duration::from_secs(30));

        let p = theme::palette(&ctx);
        let bare = |top: i8, bottom: i8| {
            egui::Frame::new().fill(p.bg).inner_margin(egui::Margin {
                left: GUTTER,
                right: GUTTER,
                top,
                bottom,
            })
        };
        egui::Panel::bottom("footer")
            .frame(bare(6, 8))
            .show_separator_line(false)
            .show(ui_root, |ui| self.footer(ui, p));
        egui::Panel::bottom("controls")
            .frame(bare(10, 6))
            .show_separator_line(false)
            .show(ui_root, |ui| self.settings_card(ui, p));
        egui::Panel::top("toolbar")
            .frame(bare(14, 10))
            .show_separator_line(false)
            .show(ui_root, |ui| self.toolbar(ui, p));
        egui::CentralPanel::default()
            .frame(bare(0, 0))
            .show(ui_root, |ui| self.list(ui, p, &devices, polled));
        self.unlock_window(&ctx, p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_says_what_is_known_and_never_guesses() {
        let t = Lang::En.strings();
        assert_eq!(status_text(State::Live(80), true, 0, t), "Connected · Bluetooth LE");
        assert_eq!(status_text(State::Live(80), false, 0, t), "Connected · Bluetooth");
        assert_eq!(
            status_text(State::NotReported, false, 0, t),
            "This device does not report its battery level"
        );
        assert_eq!(status_text(State::Unreadable, true, 0, t), "Couldn't read the battery level");
        assert_eq!(
            status_text(State::Off(Some((64, 1000))), false, 1000 + 2 * 3600 + 5, t),
            "Off or out of range · last seen at 64%, 2 h ago"
        );
        assert_eq!(
            status_text(State::Off(None), false, 0, t),
            "Off or out of range · no battery level seen yet"
        );
    }

    #[test]
    fn times_are_worded_in_every_unit() {
        let t = Lang::En.strings();
        assert_eq!(ago_text(Ago::Now, t), "just now");
        assert_eq!(ago_text(Ago::Minutes(5), t), "5 min ago");
        assert_eq!(ago_text(Ago::Hours(3), t), "3 h ago");
        assert_eq!(ago_text(Ago::Days(2), t), "2 d ago");
    }

    #[test]
    fn every_kind_has_a_label_in_every_language() {
        for lang in Lang::ALL {
            let t = lang.strings();
            let labels: Vec<&str> = [Kind::Headphones, Kind::Mouse, Kind::Keyboard, Kind::Gamepad, Kind::Other]
                .into_iter()
                .map(|kind| kind_label(kind, t))
                .collect();
            let mut unique = labels.clone();
            unique.sort();
            unique.dedup();
            assert_eq!(unique.len(), labels.len(), "{lang:?}: {labels:?}");
        }
    }
}
