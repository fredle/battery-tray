//! The window: a card for each device, the low-battery banner, and a
//! settings page. It opens as a flyout from the tray or as a pinned window
//! (`flyout.rs`), and only shows what the tray process's last poll found
//! (`link.rs`), so it never waits for a device. It runs in a process of its
//! own, started by the tray process (`tray.rs`) when it is wanted; closing
//! it hides it, and the process ends a little later. The footer with the
//! auto-update toggle and version is required on every factory app.
//! Colours and type come from `theme.rs`, the little pictures from
//! `glyphs.rs`.

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex};

use eframe::egui::{self, Align, Color32, FontId, Layout, Rect, RichText, Sense, Stroke, vec2};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use crate::autostart;
use crate::devices::{self, Ago, Device, Kind, Part, PartKind, Precision, Rules, State, Transport};
use crate::flyout::{self, Mode};
use crate::glyphs::{self, Glyph};
use crate::history::{self, Sample};
use crate::i18n::{Lang, Strings, fill};
use crate::ipc::Start;
use crate::license;
use crate::link::{self, Link};
use crate::monitor::{self, Monitor, Shared, SharedState, lock};
use crate::shots::Shots;
use crate::theme::{self, Palette};
use crate::tray;

/// The made-up scenes of screenshot mode.
mod scenes;
/// The taskbar corner some of them are drawn over.
mod taskbar;

const APP_NAME: &str = "Battery Tray";
/// Space between the window edge and its contents.
const GUTTER: i8 = 12;
/// Where a flyout is drawn the first time, to learn its height.
const OFF_SCREEN: [f32; 2] = [-20000.0, -20000.0];
/// The lowest and highest warning level the user can set, in percent.
const THRESHOLDS: std::ops::RangeInclusive<u8> = 5..=50;
/// How far back the history graph goes.
const GRAPH_SPAN: u64 = 7 * 86_400;
/// Where "Open taskbar settings" goes.
const TASKBAR_SETTINGS: &str = "ms-settings:taskbar";

/// The app as it starts: the resident tray process (`tray.rs`), or with
/// `--ui` the window process, or with `--shots` the screenshot mode.
pub fn run(args: Vec<String>) -> eframe::Result<()> {
    if args.iter().any(|a| a == "--shots") {
        return run_window(args, Start::Pinned, None);
    }
    if let Some(start) = Start::from_args(&args) {
        // Started by the tray process, which runs in efficiency mode: the
        // window must not inherit it.
        crate::power::efficiency_mode(false);
        let shared: SharedState = Arc::new(Mutex::new(Shared::new(Vec::new(), history::History::default(), Rules::default(), Lang::En)));
        // Without a tray process it is an ordinary window with its own polling.
        let link = link::find_tray().and_then(|tray| Link::connect(shared.clone(), tray));
        // A flyout needs a tray to come back from.
        let start = if link.is_some() { start } else { Start::Pinned };
        return run_window(args, start, link.map(|l| (l, shared)));
    }
    // Started by Windows at sign-in: stay in the tray.
    let in_tray = args.iter().any(|arg| arg == autostart::TRAY_ARG);
    // One copy is enough. A second start brings the first one's window up,
    // unless it is only Windows starting the app at sign-in.
    if tray::running_instance(!in_tray) {
        return Ok(());
    }
    // On by default: a battery warning is no use from an app that is not
    // running. Done once; the tick box in settings decides from then on.
    let installed = !crate::update::current_version().ends_with("(dev)");
    let mut settings = crate::settings::load();
    if autostart::default_on_now(settings.autostart_offered, installed) {
        autostart::set(true);
        settings.autostart_offered = true;
        crate::settings::save(&settings);
    } else if autostart::enabled() {
        // Keep the entry pointing at this copy if the app has moved.
        autostart::set(true);
    }
    if tray::run(!in_tray) {
        return Ok(());
    }
    // No tray: the window, in this process, and closing it quits.
    run_window(args, Start::Pinned, None)
}

/// The window. `link`: the line to the tray process and the state it fills.
fn run_window(args: Vec<String>, start: Start, link: Option<(Link, SharedState)>) -> eframe::Result<()> {
    let viewport = egui::ViewportBuilder::default()
        .with_title(APP_NAME)
        .with_inner_size([flyout::WIDTH, flyout::PINNED_HEIGHT])
        .with_decorations(false)
        .with_resizable(false)
        .with_maximize_button(false);
    // A flyout (or a window made ready while the pointer is on an icon) is
    // drawn once off-screen to learn its height; the link thread then puts
    // it in the corner (`link.rs`). No taskbar button.
    let viewport = match start {
        Start::Flyout | Start::Hidden => viewport.with_position(OFF_SCREEN).with_taskbar(false),
        Start::Pinned => viewport,
    };
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    eframe::run_native(APP_NAME, options, Box::new(move |cc| Ok(Box::new(App::new(cc, &args, start, link)))))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Page {
    Devices,
    Settings,
}

pub struct App {
    settings: crate::settings::Settings,
    version: String,
    /// "What would make this better?" (src/feedback.rs).
    feedback: crate::feedback::Feedback,
    shared: SharedState,
    /// Polling in this process: only for a window without a tray process.
    monitor: Option<Monitor>,
    /// The line to the tray process. `None` without one (and in screenshot
    /// mode); closing then quits.
    link: Option<Link>,
    /// The main window's handle.
    window: Option<isize>,
    /// How the window is to be shown once eframe has drawn it once.
    start: Option<Start>,
    /// Frames drawn.
    frames: u32,
    autostart: bool,
    licensed_to: Option<String>,
    unlock_open: bool,
    key_entry: String,
    key_message: Option<String>,
    lang: Lang,
    t: &'static Strings,
    page: Page,
    /// Cards opened to show their history.
    expanded: BTreeSet<String>,
    /// Where each card was last drawn, for its hover look.
    card_rects: HashMap<String, Rect>,
    /// The last `WindowState::shown` seen, to notice a fresh show.
    seen_shown: u32,
    /// The flyout has had the focus since it was shown, so losing it now
    /// means the user clicked elsewhere.
    had_focus: bool,
    /// Windows 10: no rounded corners or border from Windows, so the window
    /// draws its own border.
    own_border: bool,
    /// The theme the window border was last coloured for.
    styled_dark: Option<bool>,
    shots: Option<Shots>,
    /// Screenshot mode: the scene shows the flyout, sized to its content.
    shot_flyout: bool,
    /// Screenshot mode: the flyout is drawn over a taskbar and desktop.
    stage: Option<taskbar::Stage>,
    /// The flyout's height in that scene, as last measured.
    stage_height: f32,
    /// Windows' symbol font is loaded, for a device name that needs it.
    symbols: bool,
    /// The flyout height last asked of Windows, and for which show.
    asked_height: f32,
    asked_for_show: u32,
}

fn main_window(cc: &eframe::CreationContext<'_>) -> Option<isize> {
    match cc.window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(handle) => Some(handle.hwnd.get()),
        _ => None,
    }
}

/// The value after `flag` on the command line.
fn arg_value<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    let at = args.iter().position(|a| a == flag)?;
    args.get(at + 1).map(String::as_str)
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>, args: &[String], start: Start, link: Option<(Link, SharedState)>) -> Self {
        let ctx = &cc.egui_ctx;
        let shots = Shots::from_args(args);
        // Screenshots show the app as a new user first sees it.
        let settings = if shots.is_some() { crate::settings::Settings::default() } else { crate::settings::load() };
        let licensed_to = license::verify(&settings.license_key);
        let lang = match (shots.is_some(), arg_value(args, "--lang")) {
            (true, Some(locale)) => Lang::from_locale(locale),
            _ => settings.lang(),
        };
        theme::apply(ctx);
        match crate::shots::theme_from_args(args) {
            Some(theme) if shots.is_some() => ctx.set_theme(theme),
            _ => theme::follow_windows(ctx),
        }

        let window = main_window(cc);
        let rules = settings.rules(licensed_to.is_some());
        let (link, shared, monitor) = match link {
            Some((link, shared)) => {
                // The devices come from the tray process: usually here by now.
                link.wait_for_state(std::time::Duration::from_millis(500));
                {
                    let mut shared = lock(&shared);
                    shared.rules = rules;
                    shared.lang = lang;
                }
                (Some(link), shared, None)
            }
            None if shots.is_some() => {
                let shared = Arc::new(Mutex::new(Shared::new(Vec::new(), history::History::default(), rules, lang)));
                (None, shared, None)
            }
            None => {
                // No tray process: poll here.
                let shared: SharedState = Arc::new(Mutex::new(Shared::new(devices::load(), history::load(), rules, lang)));
                let repaint = ctx.clone();
                let monitor = Monitor::start(crate::source::pick(), shared.clone(), move || repaint.request_repaint());
                (None, shared, Some(monitor))
            }
        };
        // A flyout is drawn once while hidden, to learn its height, and then
        // shown; a pinned window is shown as it is.
        lock(&shared).window.mode = match start {
            Start::Flyout | Start::Hidden => Mode::Hidden,
            Start::Pinned => Mode::Pinned,
        };
        if let Some(w) = window
            && shots.is_none()
            && start == Start::Pinned
        {
            // In the middle of the taskbar's screen.
            flyout::centre(w);
        }
        let symbols = theme::needs_symbols(lock(&shared).devices.iter().map(|d| d.name.as_str()));
        theme::install_fonts(ctx, &[lang, Lang::system()], symbols);

        let autostart = shots.is_none() && autostart::enabled();
        let mut feedback = crate::feedback::Feedback::default();
        feedback.set_texts(lang.feedback_texts());
        let mut app = Self {
            lang,
            t: lang.strings(),
            settings,
            version: crate::update::current_version(),
            feedback,
            shared,
            monitor,
            link,
            window,
            start: shots.is_none().then_some(start),
            frames: 0,
            autostart,
            licensed_to,
            unlock_open: false,
            key_entry: String::new(),
            key_message: None,
            page: Page::Devices,
            expanded: BTreeSet::new(),
            card_rects: HashMap::new(),
            seen_shown: 0,
            had_focus: false,
            own_border: false,
            styled_dark: None,
            shots,
            shot_flyout: false,
            stage: None,
            stage_height: 0.0,
            symbols,
            asked_height: 0.0,
            asked_for_show: 0,
        };
        if app.shots.is_some() {
            app.version = env!("CARGO_PKG_VERSION").to_owned();
            let zoom: f32 = arg_value(args, "--scale").and_then(|s| s.parse().ok()).unwrap_or(1.0);
            ctx.set_pixels_per_point(zoom);
        }
        app
    }

    /// Saves settings, except in screenshot mode.
    fn save(&self) {
        if self.shots.is_none() {
            crate::settings::save(&self.settings);
        }
    }

    fn licensed(&self) -> bool {
        self.licensed_to.is_some()
    }

    fn rules(&self) -> Rules {
        self.settings.rules(self.licensed())
    }

    /// Hands the settings the monitor and tray depend on to them.
    fn share_settings(&self) {
        {
            let mut shared = lock(&self.shared);
            shared.rules = self.rules();
            shared.lang = self.lang;
        }
        if let Some(link) = self.link {
            link.settings_changed(false);
        }
    }

    /// Looks again now: a device may have become low under new rules.
    fn refresh(&self) {
        if let Some(monitor) = &self.monitor {
            monitor.refresh();
        }
        if let Some(link) = self.link {
            link.refresh();
        }
    }

    fn set_language(&mut self, lang: Lang, ctx: &egui::Context) {
        self.feedback.set_texts(lang.feedback_texts());
        self.lang = lang;
        self.t = lang.strings();
        self.key_message = None;
        theme::install_fonts(ctx, &[lang, Lang::system()], self.symbols);
        self.settings.language = Some(lang);
        self.save();
        self.share_settings();
    }

    /// Whether the free tier's limits apply.
    fn limited(&self) -> bool {
        !self.licensed() && license::paid_tier_enabled()
    }

    /// Hides the window to the tray. `blurred`: because it lost focus.
    fn hide(&mut self, blurred: bool) {
        {
            let mut shared = lock(&self.shared);
            shared.window.mode = Mode::Hidden;
            if blurred {
                shared.window.blurred_at = Some(std::time::Instant::now());
            }
        }
        if let Some(window) = self.window
            && self.shots.is_none()
        {
            flyout::hide(window);
        }
        if let Some(link) = self.link {
            link.hidden();
        }
    }

    fn pin(&mut self) {
        {
            let mut shared = lock(&self.shared);
            shared.window.mode = Mode::Pinned;
        }
        if let Some(window) = self.window
            && self.shots.is_none()
        {
            flyout::show_pinned(window, false);
        }
    }

    fn header(&mut self, ui: &mut egui::Ui, p: Palette, mode: Mode) {
        let t = self.t;
        // Pinned, the header is the title bar: drag it to move the window.
        if mode == Mode::Pinned {
            let drag = ui.interact(ui.max_rect(), ui.id().with("drag"), Sense::click_and_drag());
            if drag.drag_started() {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
            }
        }
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            if self.page == Page::Settings
                && glyphs::button(ui, Glyph::Back, p.text, p.control_hover, t.back).clicked()
            {
                self.page = Page::Devices;
            }
            ui.add_space(4.0);
            let title = match self.page {
                Page::Devices => t.devices_title,
                Page::Settings => t.settings_title,
            };
            ui.label(RichText::new(title).font(FontId::new(18.0, theme::semibold())).color(p.text));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if mode == Mode::Pinned && (self.link.is_some() || self.shots.is_some()) {
                    if glyphs::button(ui, Glyph::Close, p.text, p.control_hover, t.close_window).clicked() {
                        self.hide(false);
                    }
                } else if mode != Mode::Pinned
                    && glyphs::button(ui, Glyph::Pin, p.text, p.control_hover, t.pin_window).clicked()
                {
                    self.pin();
                }
                if self.page == Page::Devices {
                    if glyphs::button(ui, Glyph::Gear, p.text, p.control_hover, t.settings_title).clicked() {
                        self.page = Page::Settings;
                    }
                    if glyphs::button(ui, Glyph::Refresh, p.text, p.control_hover, t.refresh).clicked() {
                        self.refresh();
                    }
                }
            });
        });
    }

    /// Windows 11 tucks new tray icons away; say so once.
    fn pin_hint(&mut self, ui: &mut egui::Ui, p: Palette) {
        let t = self.t;
        egui::Frame::new()
            .fill(p.accent_soft)
            .corner_radius(theme::CARD_RADIUS)
            .inner_margin(egui::Margin::symmetric(14, 12))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 4.0;
                ui.label(RichText::new(t.pin_hint_title).family(theme::semibold()).color(p.text));
                ui.label(RichText::new(t.pin_hint_body).small().color(p.text));
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    let open_settings = egui::Button::new(RichText::new(t.pin_hint_open).small().color(p.on_accent))
                        .fill(p.accent)
                        .stroke(Stroke::NONE);
                    if ui.add(open_settings).clicked() {
                        open(TASKBAR_SETTINGS);
                    }
                    if ui.add(egui::Button::new(RichText::new(t.got_it).small().color(p.text))).clicked() {
                        self.settings.pin_hint_done = true;
                        self.save();
                    }
                });
            });
    }

    /// One line per device that is low right now and gets warnings.
    fn low_banner(&self, ui: &mut egui::Ui, p: Palette, devices: &[Device], rules: &Rules) {
        let t = self.t;
        let low = devices::low_now(devices, rules);
        if low.is_empty() {
            return;
        }
        egui::Frame::new()
            .fill(p.surface)
            .stroke(Stroke::new(1.0, p.error))
            .corner_radius(theme::CARD_RADIUS)
            .inner_margin(egui::Margin::symmetric(14, 10))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.label(RichText::new(t.low_title).family(theme::semibold()).color(p.error));
                for (device, level) in low {
                    let line = devices::fill_level(t.low_body, level, device.precision, t, &[("name", &device.name)]);
                    ui.label(RichText::new(line).color(p.text));
                }
            });
    }

    fn device_card(&mut self, ui: &mut egui::Ui, p: Palette, device: &Device, view: &View) {
        let t = self.t;
        let state = device.state();
        let charging = device.is_charging();
        let low = |level: u8| view.rules.is_low(device, level) && !charging;
        let level_colour = match state {
            State::Live(level) if low(level) => p.error,
            State::Live(_) => p.text,
            _ => p.muted,
        };
        let has_history = device.live_level().is_some() || device.last_level.is_some();
        let expanded = has_history && self.expanded.contains(&device.id);
        let hovered = has_history && self.card_rects.get(&device.id).is_some_and(|r| ui.rect_contains_pointer(*r));
        let frame = egui::Frame::new()
            .fill(if hovered { p.control_hover } else { p.surface })
            .stroke(Stroke::new(1.0, p.border))
            .corner_radius(theme::CARD_RADIUS)
            .inner_margin(egui::Margin::symmetric(12, 10));
        let builder = egui::UiBuilder::new().sense(if has_history { Sense::click() } else { Sense::hover() });
        let response = ui
            .scope_builder(builder, |ui| {
                frame.show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = 6.0;
                    self.card_top(ui, p, device, view, level_colour);
                    match state {
                        State::Live(level) if device.parts.is_empty() => {
                            level_bar(ui, p, level, if low(level) { p.error } else if charging { p.success } else { p.accent }, 6.0)
                        }
                        State::Off(Some((level, _))) if device.parts.is_empty() => level_bar(ui, p, level, p.muted, 6.0),
                        _ => {}
                    }
                    if !device.parts.is_empty() && matches!(state, State::Live(_) | State::Off(_)) {
                        parts_bars(ui, p, &device.parts, device.precision, device.connected, view.rules.threshold_for(&device.id), t);
                    }
                    if state == State::NotReported {
                        self.unsupported(ui, p, device);
                    }
                    if expanded {
                        self.expansion(ui, p, device, view);
                    }
                })
            })
            .response;
        self.card_rects.insert(device.id.clone(), response.rect);
        if has_history {
            let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
            if response.clicked() {
                if expanded {
                    self.expanded.remove(&device.id);
                } else {
                    self.expanded.insert(device.id.clone());
                }
            }
        }
    }

    /// Glyph, name and status, and the level on the right.
    fn card_top(&self, ui: &mut egui::Ui, p: Palette, device: &Device, view: &View, level_colour: Color32) {
        let t = self.t;
        ui.horizontal(|ui| {
            let (tile, kind) = ui.allocate_exact_size(vec2(36.0, 36.0), Sense::hover());
            kind.on_hover_text(kind_label(device.kind, t));
            ui.painter().rect_filled(tile, 8, p.accent_soft);
            glyphs::kind(ui.painter(), tile.shrink(8.0), device.kind, p.accent);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let shown = match device.state() {
                    State::Live(level) => Some(level),
                    State::Off(Some((level, _))) => Some(level),
                    _ => None,
                };
                if let Some(level) = shown {
                    ui.label(
                        RichText::new(devices::level_text(level, device.precision, t))
                            .font(FontId::new(20.0, theme::semibold()))
                            .color(level_colour),
                    );
                }
                if device.is_charging() {
                    let (rect, _) = ui.allocate_exact_size(vec2(14.0, 18.0), Sense::hover());
                    glyphs::bolt(ui.painter(), rect, p.success);
                }
                ui.with_layout(Layout::top_down(Align::Min), |ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    ui.add(
                        egui::Label::new(RichText::new(&device.name).font(FontId::new(14.5, theme::semibold())).color(p.text))
                            .truncate(),
                    );
                    let estimate = view.estimates.get(&device.id).copied();
                    let status = status_text(device, view.now, estimate, t);
                    let colour = if device.state() == State::Unreadable { p.warn } else { p.muted };
                    ui.add(egui::Label::new(RichText::new(&status).small().color(colour)).truncate())
                        .on_hover_text(status);
                });
            });
        });
    }

    /// A device Windows has no level for: say why, and offer to tell us.
    fn unsupported(&mut self, ui: &mut egui::Ui, p: Palette, device: &Device) {
        let t = self.t;
        ui.label(RichText::new(t.unsupported_body).small().color(p.muted));
        if crate::feedback::enabled() {
            let link = egui::Button::new(RichText::new(t.tell_model).small().color(p.accent))
                .fill(p.accent_soft)
                .stroke(Stroke::NONE);
            if ui.add(link).clicked() {
                self.feedback.open_with(&model_report(device, t));
            }
        }
    }

    /// The history graph and time left, or what the unlock adds.
    fn expansion(&mut self, ui: &mut egui::Ui, p: Palette, device: &Device, view: &View) {
        let t = self.t;
        ui.add_space(2.0);
        if !view.insights {
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
                glyphs::glyph(ui.painter(), rect, Glyph::Lock, p.muted);
                ui.label(RichText::new(t.insights_locked).small().color(p.muted));
            });
            if license::paid_tier_enabled() {
                let unlock = egui::Button::new(RichText::new(t.unlock).small().color(p.accent))
                    .fill(p.accent_soft)
                    .stroke(Stroke::NONE);
                if ui.add(unlock).clicked() {
                    self.unlock_open = true;
                }
            }
            return;
        }
        let samples = view.graphs.get(&device.id).map_or(&[][..], Vec::as_slice);
        if samples.is_empty() {
            ui.label(RichText::new(t.history_empty).small().color(p.muted));
            return;
        }
        // The line ends where the device went off, or now.
        let until = if device.connected { view.now } else { samples.last().map_or(view.now, |s| s.time()) };
        let threshold = (device.precision != Precision::Coarse).then(|| view.rules.threshold_for(&device.id));
        history_graph(ui, p, samples, view.now, until, threshold);
        ui.label(RichText::new(t.history_caption).small().color(p.muted));
    }

    fn devices_page(&mut self, ui: &mut egui::Ui, p: Palette, devices: &[Device], polled: bool, view: &View) {
        let t = self.t;
        ui.spacing_mut().item_spacing.y = 8.0;
        if !self.settings.pin_hint_done && (self.link.is_some() || self.shots.is_some()) {
            self.pin_hint(ui, p);
        }
        self.low_banner(ui, p, devices, &view.rules);
        if devices.is_empty() {
            egui::Frame::new()
                .fill(p.surface)
                .stroke(Stroke::new(1.0, p.border))
                .corner_radius(theme::CARD_RADIUS)
                .inner_margin(egui::Margin::same(20))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    let text = if polled { t.no_devices } else { t.searching };
                    ui.label(RichText::new(text).color(p.muted));
                });
            return;
        }
        for device in devices {
            self.device_card(ui, p, device, view);
        }
    }

    fn settings_page(&mut self, ui: &mut egui::Ui, p: Palette, devices: &[Device]) {
        let t = self.t;
        ui.spacing_mut().item_spacing.y = 6.0;

        section(ui, p, t.section_warnings, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(t.warn_at).color(p.text));
                let threshold = ui.add(egui::DragValue::new(&mut self.settings.threshold).range(THRESHOLDS).speed(0.2).suffix("%"));
                if threshold.changed() {
                    self.save();
                    self.share_settings();
                }
                if threshold.drag_stopped() || (threshold.changed() && !threshold.dragged()) {
                    self.refresh();
                }
            });
        });

        // Each device: its own tray icon, its own warning level.
        let listed: Vec<&Device> = devices.iter().filter(|d| d.live_level().is_some() || d.last_level.is_some()).collect();
        if !listed.is_empty() {
            let rules = self.rules();
            let in_tray: Vec<String> = rules.tray_devices(devices).iter().map(|d| d.id.clone()).collect();
            section(ui, p, t.each_device, |ui| {
                ui.horizontal(|ui| {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        cell(ui, WARN_COLUMN, |ui| {
                            ui.add(egui::Label::new(RichText::new(t.warn_column).small().color(p.muted)).truncate());
                        });
                        cell(ui, TRAY_COLUMN, |ui| {
                            ui.add(egui::Label::new(RichText::new(t.tray_column).small().color(p.muted)).truncate());
                        });
                    });
                });
                for device in &listed {
                    ui.horizontal(|ui| {
                        let (glyph, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
                        glyphs::kind(ui.painter(), glyph, device.kind, p.muted);
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            cell(ui, WARN_COLUMN, |ui| self.warn_choice(ui, p, device));
                            let mut on = in_tray.contains(&device.id);
                            // The last icon cannot go: the tray always has one.
                            let only = on && in_tray.len() == 1;
                            let mut changed = false;
                            cell(ui, TRAY_COLUMN, |ui| {
                                changed = ui.add_enabled_ui(!only, |ui| theme::checkbox(ui, &mut on, "")).inner.changed();
                            });
                            if changed {
                                let mut list = in_tray.clone();
                                if on {
                                    if self.limited() {
                                        list.clear();
                                    }
                                    list.push(device.id.clone());
                                } else {
                                    list.retain(|id| id != &device.id);
                                }
                                self.settings.set_tray_list(list);
                                self.save();
                                self.share_settings();
                                // A device that is already low warns straight away.
                                self.refresh();
                            }
                            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                                ui.add(egui::Label::new(RichText::new(&device.name).color(p.text)).truncate());
                            });
                        });
                    });
                }
                if self.limited() {
                    ui.label(RichText::new(t.tray_free_note).small().color(p.muted));
                }
            });
        }

        section(ui, p, t.section_notifications, |ui| {
            let mut connect = self.settings.notify_connect;
            if theme::checkbox(ui, &mut connect, t.notify_connect).changed() {
                self.settings.notify_connect = connect;
                self.save();
                self.share_settings();
            }
            let mut full = self.settings.notify_full;
            if theme::checkbox(ui, &mut full, t.notify_full).changed() {
                self.settings.notify_full = full;
                self.save();
                self.share_settings();
            }
        });

        section(ui, p, t.section_general, |ui| {
            if theme::checkbox(ui, &mut self.autostart, t.autostart).changed() && self.shots.is_none() {
                autostart::set(self.autostart);
                self.autostart = autostart::enabled();
            }
            ui.horizontal(|ui| {
                ui.label(RichText::new(t.language).color(p.text));
                let mut chosen = self.lang;
                egui::ComboBox::from_id_salt("language")
                    .selected_text(self.lang.name())
                    .width(170.0)
                    .show_ui(ui, |ui| {
                        for lang in Lang::ALL {
                            ui.selectable_value(&mut chosen, lang, lang.name());
                        }
                    });
                if chosen != self.lang {
                    self.set_language(chosen, ui.ctx());
                }
            });
        });

        if license::paid_tier_enabled() {
            section(ui, p, t.unlock_title, |ui| {
                if let Some(email) = &self.licensed_to {
                    ui.label(RichText::new(fill(t.unlocked_for, &[("email", email)])).color(p.success));
                } else {
                    ui.label(RichText::new(t.plan).small().color(p.text));
                    let unlock = egui::Button::new(RichText::new(t.unlock).color(p.on_accent)).fill(p.accent).stroke(Stroke::NONE);
                    if ui.add(unlock).clicked() {
                        self.unlock_open = true;
                    }
                }
            });
        }
    }

    /// The warning level for one device: the default, or its own.
    fn warn_choice(&mut self, ui: &mut egui::Ui, p: Palette, device: &Device) {
        let t = self.t;
        if device.precision == Precision::Coarse {
            ui.add(egui::Label::new(RichText::new(t.warn_coarse).small().color(p.muted)).truncate());
            return;
        }
        let own = self.settings.thresholds.get(&device.id).copied();
        let label = |level: Option<u8>| match level {
            Some(level) => format!("{level}%"),
            None => fill(t.level_default, &[("level", &self.settings.threshold.to_string())]),
        };
        let mut chosen = own;
        egui::ComboBox::from_id_salt(("warn", &device.id))
            .selected_text(RichText::new(label(own)).small())
            .width(WARN_COLUMN - 26.0)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut chosen, None, label(None));
                for level in THRESHOLDS.step_by(5) {
                    ui.selectable_value(&mut chosen, Some(level), label(Some(level)));
                }
            });
        if chosen != own {
            match chosen {
                Some(level) => self.settings.thresholds.insert(device.id.clone(), level),
                None => self.settings.thresholds.remove(&device.id),
            };
            self.save();
            self.share_settings();
            self.refresh();
        }
    }

    fn unlock_window(&mut self, ctx: &egui::Context, p: Palette) {
        if !self.unlock_open {
            return;
        }
        let t = self.t;
        let modal = egui::Modal::new(egui::Id::new("unlock"))
            .frame(theme::card(p).inner_margin(egui::Margin::same(20)))
            .show(ctx, |ui| {
                ui.set_width((ui.ctx().content_rect().width() - 64.0).clamp(240.0, 360.0));
                ui.label(RichText::new(t.unlock_title).font(FontId::new(18.0, theme::semibold())).color(p.text));
                ui.label(RichText::new(t.plan).color(p.muted));
                ui.add_space(4.0);
                let buy = fill(t.buy, &[("price", license::PRICE)]);
                if ui.add(theme::primary_button(p, buy, true)).clicked() {
                    open(license::BUY_URL);
                }
                ui.add_space(8.0);
                ui.label(RichText::new(t.paste_key).color(p.muted));
                ui.add(egui::TextEdit::singleline(&mut self.key_entry).hint_text("BTT1.…").desired_width(f32::INFINITY));
                ui.horizontal(|ui| {
                    if ui.button(t.activate).clicked() {
                        match license::verify(&self.key_entry) {
                            Some(email) => {
                                self.settings.license_key = self.key_entry.trim().to_owned();
                                self.save();
                                self.key_message = Some(fill(t.unlocked_thanks, &[("email", &email)]));
                                self.licensed_to = Some(email);
                                // Every device gets warnings from now on.
                                self.share_settings();
                                self.refresh();
                            }
                            None => self.key_message = Some(t.bad_key.to_owned()),
                        }
                    }
                    if ui.button(t.close_window).clicked() {
                        ui.close();
                    }
                });
                if let Some(message) = &self.key_message {
                    let colour = if self.licensed() { p.success } else { p.error };
                    ui.label(RichText::new(message).color(colour));
                }
            });
        if modal.should_close() {
            self.unlock_open = false;
            self.key_message = None;
        }
    }

    fn footer(&mut self, ui: &mut egui::Ui, p: Palette) {
        ui.spacing_mut().item_spacing.y = 2.0;
        ui.horizontal(|ui| {
            let mut auto_update = self.settings.auto_update;
            let label = RichText::new(self.t.auto_update).small().color(p.muted);
            if theme::checkbox(ui, &mut auto_update, label).on_hover_text(self.t.auto_update_hint).changed() {
                self.settings.auto_update = auto_update;
                self.save();
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(RichText::new(format!("v{}", self.version)).small().color(p.muted));
            });
        });
        self.feedback.link(ui);
    }
}

/// What a frame needs to know beyond the devices, read once per frame.
struct View {
    now: u64,
    rules: Rules,
    insights: bool,
    /// Seconds left, for devices with an estimate.
    estimates: HashMap<String, u64>,
    /// History of the expanded cards.
    graphs: HashMap<String, Vec<Sample>>,
}

/// Widths of the per-device table's columns.
const WARN_COLUMN: f32 = 140.0;
const TRAY_COLUMN: f32 = 64.0;

/// A fixed-width cell of a table row, its content centred.
fn cell(ui: &mut egui::Ui, width: f32, add: impl FnOnce(&mut egui::Ui)) {
    ui.allocate_ui_with_layout(vec2(width, 26.0), Layout::top_down(Align::Center), |ui| {
        ui.set_min_width(width);
        ui.set_max_width(width);
        add(ui);
    });
}

/// A titled group of settings.
fn section(ui: &mut egui::Ui, p: Palette, title: &str, add: impl FnOnce(&mut egui::Ui)) {
    ui.add_space(4.0);
    ui.label(RichText::new(title).small().family(theme::semibold()).color(p.muted));
    egui::Frame::new()
        .fill(p.surface)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(theme::CARD_RADIUS)
        .inner_margin(egui::Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 6.0;
            add(ui);
        });
}

/// What "Tell us this model" puts in the feedback box: shown to the user,
/// who can edit or cancel it before anything is sent.
pub fn model_report(device: &Device, t: &Strings) -> String {
    let kind = match device.kind {
        Kind::Headphones => "headphones",
        Kind::Mouse => "mouse",
        Kind::Keyboard => "keyboard",
        Kind::Gamepad => "gamepad",
        Kind::Laptop => "laptop",
        Kind::Other => "other",
    };
    let mut lines = vec![
        t.tell_model_note.to_owned(),
        String::new(),
        format!("Name: {}", device.name),
        format!("Kind: {kind}"),
        format!("Connection: {}", devices::transport_text(device.transport, Lang::En.strings())),
    ];
    if let Some((vendor, product)) = device.hardware {
        lines.push(format!("Vendor/product: {vendor:04X}/{product:04X}"));
    }
    lines.join("\n")
}

fn kind_label(kind: Kind, t: &Strings) -> &'static str {
    match kind {
        Kind::Headphones => t.kind_headphones,
        Kind::Mouse => t.kind_mouse,
        Kind::Keyboard => t.kind_keyboard,
        Kind::Gamepad => t.kind_gamepad,
        Kind::Laptop => t.kind_laptop,
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

/// "About 5 days left": hours up to two days, then days.
pub fn time_left_text(secs: u64, t: &Strings) -> String {
    const HOUR: u64 = 3600;
    if secs < HOUR {
        t.left_soon.to_owned()
    } else if secs < 48 * HOUR {
        fill(t.left_hours, &[("n", &((secs + HOUR / 2) / HOUR).to_string())])
    } else {
        fill(t.left_days, &[("n", &((secs + 12 * HOUR) / (24 * HOUR)).to_string())])
    }
}

/// The small line under a device's name: what is known, and nothing that
/// is not.
fn status_text(device: &Device, now: u64, estimate: Option<u64>, t: &Strings) -> String {
    let transport = devices::transport_text(device.transport, t);
    match device.state() {
        State::Live(_) => {
            let charging = device.is_charging();
            let detail = if charging { Some(capitalise(t.charging)) } else { estimate.map(|secs| time_left_text(secs, t)) };
            match detail {
                // The laptop's "This PC" says nothing its name does not.
                Some(detail) if device.transport == Transport::Internal => detail,
                Some(detail) if charging => format!("{detail} · {transport}"),
                Some(detail) => format!("{transport} · {detail}"),
                None => transport.to_owned(),
            }
        }
        State::NotReported => format!("{transport} · {}", t.no_report),
        State::Unreadable => t.unreadable.to_owned(),
        State::Off(Some((level, seen))) => {
            let when = ago_text(devices::ago(now, seen), t);
            // `off_last` says "{level}%": a coarse level is a word instead.
            let last = devices::fill_level(t.off_last, level, device.precision, t, &[("when", &when)]);
            format!("{} · {last}", t.off)
        }
        State::Off(None) => format!("{} · {}", t.off, t.never_seen),
    }
}

/// "charging" at the start of a line.
fn capitalise(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// A rounded bar filled to `level` percent.
fn level_bar(ui: &mut egui::Ui, p: Palette, level: u8, colour: Color32, height: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    paint_bar(ui.painter(), rect, p.track, level, colour);
}

fn paint_bar(painter: &egui::Painter, rect: Rect, track: Color32, level: u8, colour: Color32) {
    let radius = (rect.height() / 2.0) as u8;
    painter.rect_filled(rect, radius, track);
    let mut filled = rect;
    // A sliver stays visible at 0-2% so the bar never looks absent.
    filled.set_width((rect.width() * f32::from(level.min(100)) / 100.0).max(rect.height()));
    painter.rect_filled(filled, radius, colour);
}

/// Earbuds and case as three labelled bars.
fn parts_bars(ui: &mut egui::Ui, p: Palette, parts: &[Part], precision: Precision, live: bool, threshold: u8, t: &Strings) {
    let gap = 10.0;
    let width = (ui.available_width() - gap * (parts.len() as f32 - 1.0)) / parts.len().max(1) as f32;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        for part in parts {
            let (rect, _) = ui.allocate_exact_size(vec2(width, 30.0), Sense::hover());
            let label = match part.kind {
                PartKind::Left => t.part_left,
                PartKind::Right => t.part_right,
                PartKind::Case => t.part_case,
            };
            let painter = ui.painter();
            let small = FontId::new(12.0, egui::FontFamily::Proportional);
            painter.text(rect.left_top(), egui::Align2::LEFT_TOP, label, small.clone(), p.muted);
            let value = part.level.map_or("–".to_owned(), |level| devices::level_text(level, precision, t));
            let low = part.kind != PartKind::Case && part.level.is_some_and(|l| devices::warn_level(l, precision) <= threshold);
            let colour = if !live { p.muted } else if low && !part.charging { p.error } else { p.text };
            let right = rect.right_top();
            let drawn = painter.text(right, egui::Align2::RIGHT_TOP, &value, FontId::new(12.0, theme::semibold()), colour);
            if part.charging && part.level.is_some() {
                let bolt = Rect::from_min_size(drawn.left_top() - vec2(12.0, -1.0), vec2(10.0, 13.0));
                glyphs::bolt(painter, bolt, p.success);
            }
            let bar = Rect::from_min_max(rect.left_bottom() - vec2(0.0, 5.0), rect.right_bottom());
            let fill = if !live { p.muted } else if part.charging { p.success } else if low { p.error } else { p.accent };
            match part.level {
                Some(level) => paint_bar(painter, bar, p.track, level, fill),
                None => {
                    painter.rect_filled(bar, 2, p.track);
                }
            }
        }
    });
}

/// The level over the last week as steps: charging in green, the warning
/// level dashed.
fn history_graph(ui: &mut egui::Ui, p: Palette, samples: &[Sample], now: u64, until: u64, threshold: Option<u8>) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 64.0), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 6, p.bg);
    let plot = rect.shrink2(vec2(6.0, 6.0));
    let x = |fraction: f32| plot.left() + plot.width() * fraction;
    let y = |level: u8| plot.bottom() - plot.height() * f32::from(level.min(100)) / 100.0;
    for level in [50u8] {
        painter.line_segment([egui::pos2(plot.left(), y(level)), egui::pos2(plot.right(), y(level))], Stroke::new(1.0, p.border));
    }
    if let Some(threshold) = threshold {
        let a = egui::pos2(plot.left(), y(threshold));
        let b = egui::pos2(plot.right(), y(threshold));
        painter.extend(egui::Shape::dashed_line(&[a, b], Stroke::new(1.0, p.error.gamma_multiply(0.6)), 4.0, 3.0));
    }
    let points = history::graph(samples, now, GRAPH_SPAN);
    let end = (until.saturating_sub(now.saturating_sub(GRAPH_SPAN))) as f32 / GRAPH_SPAN as f32;
    for (i, point) in points.iter().enumerate() {
        let next_x = points.get(i + 1).map_or(end.clamp(0.0, 1.0), |n| n.x);
        let colour = if point.charging { p.success } else { p.accent };
        let (x0, x1, top) = (x(point.x), x(next_x.max(point.x)), y(point.level));
        if x1 > x0 {
            painter.rect_filled(Rect::from_min_max(egui::pos2(x0, top), egui::pos2(x1, plot.bottom())), 0, colour.gamma_multiply(0.14));
            painter.line_segment([egui::pos2(x0, top), egui::pos2(x1, top)], Stroke::new(2.0, colour));
        }
        if let Some(next) = points.get(i + 1) {
            painter.line_segment([egui::pos2(x1, top), egui::pos2(x1, y(next.level))], Stroke::new(2.0, colour));
        }
    }
}

/// Opens a web page or a Settings page with whatever Windows uses for it.
fn open(target: &str) {
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    use windows::core::{HSTRING, w};
    unsafe {
        ShellExecuteW(None, w!("open"), &HSTRING::from(target), None, None, SW_SHOWNORMAL);
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui_screen: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui_screen.ctx().clone();
        if let Some(scene) = self.shots.as_mut().and_then(|shots| shots.drive(&ctx, scenes::COUNT)) {
            scenes::set_up(self, scene, &ctx);
        }
        // Closing the window hides it; Quit is in the tray menu. Only a
        // process on its way out (`link::quitting`) really closes.
        if self.link.is_some() && !link::quitting() && ctx.input(|input| input.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.hide(false);
        }
        // A pinned start: eframe shows the window once it has drawn it, then
        // it is brought to the front. (A flyout is shown by the link thread
        // as soon as the first frame is drawn: see the end of this function.)
        self.frames = self.frames.saturating_add(1);
        if self.frames >= 2
            && self.start == Some(Start::Pinned)
            && let Some(w) = self.window
        {
            self.start = None;
            flyout::raise(w);
        }
        if let Some(link) = self.link {
            // A half-written note keeps the process from closing while hidden.
            link.set_busy(self.unlock_open || self.feedback.is_open());
        }

        let now = if self.shots.is_some() { scenes::NOW } else { monitor::now() };
        let (devices, polled, window, view) = {
            let shared = lock(&self.shared);
            let insights = license::insights(shared.rules.licensed);
            let estimates = if insights {
                shared
                    .devices
                    .iter()
                    .filter_map(|d| {
                        let level = d.live_level()?;
                        let secs = history::time_left(shared.history.samples(&d.id), now, level, d.precision, d.is_charging())?;
                        Some((d.id.clone(), secs))
                    })
                    .collect()
            } else {
                HashMap::new()
            };
            let graphs = self.expanded.iter().map(|id| (id.clone(), shared.history.samples(id).to_vec())).collect();
            let view = View { now, rules: shared.rules.clone(), insights, estimates, graphs };
            (shared.devices.clone(), shared.polled, shared.window, view)
        };
        // A device whose name needs the symbol font paired while open.
        if !self.symbols && theme::needs_symbols(devices.iter().map(|d| d.name.as_str())) {
            self.symbols = true;
            theme::install_fonts(&ctx, &[self.lang, Lang::system()], true);
        }
        // Keeps "5 min ago" moving while the window is open; a hidden one
        // is not woken for it.
        if window.mode != Mode::Hidden {
            ctx.request_repaint_after(std::time::Duration::from_secs(30));
        }

        // A fresh show: the flyout has not had the focus yet.
        if window.shown != self.seen_shown {
            self.seen_shown = window.shown;
            self.had_focus = false;
        }
        if window.mode == Mode::Flyout && self.shots.is_none() {
            match ctx.input(|i| i.viewport().focused) {
                Some(true) => self.had_focus = true,
                Some(false) if self.had_focus => self.hide(true),
                _ => {}
            }
        }

        let p = theme::palette(&ctx);
        let dark = ctx.theme() == egui::Theme::Dark;
        if self.styled_dark != Some(dark)
            && let Some(w) = self.window
        {
            self.styled_dark = Some(dark);
            self.own_border = !flyout::style(w, [p.border.r(), p.border.g(), p.border.b()]);
        }
        // A staged scene draws the window's own contents in the flyout's
        // place, over a desktop and taskbar.
        let screen = ui_screen.max_rect();
        let screen_painter = ui_screen.painter().clone();
        let staged_rect = taskbar::flyout_rect(screen, self.stage_height);
        let mut staged;
        let ui_root: &mut egui::Ui = if self.stage.is_some() {
            taskbar::backdrop(&screen_painter, screen, staged_rect, dark);
            staged = ui_screen.new_child(egui::UiBuilder::new().max_rect(staged_rect));
            &mut staged
        } else {
            ui_screen
        };
        let bare = |top: i8, bottom: i8| {
            egui::Frame::new().fill(p.bg).inner_margin(egui::Margin { left: GUTTER, right: GUTTER, top, bottom })
        };
        let header = egui::Panel::top("header")
            .frame(bare(10, 6))
            .show_separator_line(false)
            .show(ui_root, |ui| self.header(ui, p, window.mode))
            .response
            .rect
            .height();
        let footer = egui::Panel::bottom("footer")
            .frame(bare(6, 8))
            .show_separator_line(false)
            .show(ui_root, |ui| self.footer(ui, p))
            .response
            .rect
            .height();
        let content = egui::CentralPanel::default()
            .frame(bare(2, 6))
            .show(ui_root, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, true])
                    .show(ui, |ui| match self.page {
                        Page::Devices => self.devices_page(ui, p, &devices, polled, &view),
                        Page::Settings => self.settings_page(ui, p, &devices),
                    })
                    .content_size
                    .y
            })
            .inner;
        self.unlock_window(&ctx, p);
        self.feedback.show(&ctx, &self.version, self.shots.is_some());

        // Esc closes it, unless it is closing a box or a list first.
        let busy = ctx.memory(|m| m.top_modal_layer().is_some()) || egui::Popup::is_any_open(&ctx);
        if window.mode != Mode::Hidden && self.link.is_some() && !busy && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.hide(false);
        }

        // The flyout is as tall as what it shows.
        let wanted = (header + footer + content + 8.0 + 2.0).clamp(flyout::MIN_HEIGHT, flyout::MAX_HEIGHT);
        lock(&self.shared).window.height = wanted;
        let current = ctx.content_rect().height();
        if let Some(stage) = self.stage {
            self.stage_height = wanted;
            let canvas = taskbar::canvas_height(wanted);
            if (canvas - current).abs() > 1.5 {
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(vec2(taskbar::WIDTH, canvas)));
            }
            taskbar::flyout_corners(&screen_painter, screen, staged_rect, dark, p.border);
            taskbar::taskbar(&ctx, &screen_painter, screen, dark, self.lang, &stage);
        } else if (wanted - current).abs() > 1.5 {
            if self.shots.is_some() {
                if self.shot_flyout {
                    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(vec2(flyout::WIDTH, wanted)));
                }
            } else if window.mode == Mode::Flyout
                && let Some(w) = self.window
                && (self.asked_height != wanted || window.shown != self.asked_for_show)
            {
                // Asked once per height: if the screen is too short for it,
                // asking again every frame would change nothing.
                self.asked_height = wanted;
                self.asked_for_show = window.shown;
                flyout::resize_flyout(w, wanted);
            }
        }

        if self.own_border || (self.shots.is_some() && self.stage.is_none()) {
            let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("border")));
            painter.rect_stroke(ctx.content_rect(), 0, Stroke::new(1.0, p.border), egui::StrokeKind::Inside);
        }

        // The first frame is drawn (it is painted the moment this returns):
        // the link thread may now show the window, at the height just
        // measured, and act on clicks.
        if self.frames == 1
            && let (Some(link), Some(w), Some(start)) = (self.link, self.window, self.start)
        {
            link.attach(w, ctx.clone(), start);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::devices::{Battery, Reading};

    /// A device in a given state, through the same path a poll takes.
    fn device(reading: Reading, before: Option<(u8, u64)>) -> Device {
        let mut devices = Vec::new();
        let rules = Rules { threshold: 0, ..Rules::default() };
        if let Some((level, seen)) = before {
            let mut live = reading.clone();
            live.connected = true;
            live.battery = Battery::Level(level);
            devices::apply(&mut devices, vec![live], seen, &rules, false);
        }
        devices::apply(&mut devices, vec![reading], 1, &rules, false);
        devices.remove(0)
    }

    fn reading(transport: Transport, connected: bool, battery: Battery) -> Reading {
        Reading { connected, battery, ..Reading::new("a", "A", Kind::Other, transport) }
    }

    #[test]
    fn status_says_what_is_known_and_never_guesses() {
        let t = Lang::En.strings();
        let status = |d: Device, now: u64| status_text(&d, now, None, t);
        assert_eq!(status(device(reading(Transport::Le, true, Battery::Level(80)), None), 0), "Bluetooth LE");
        assert_eq!(status(device(reading(Transport::Receiver, true, Battery::Level(80)), None), 0), "USB receiver");
        let charging = Reading { charging: Some(true), ..reading(Transport::Internal, true, Battery::Level(40)) };
        assert_eq!(status(device(charging, None), 0), "Charging");
        let usb = Reading { charging: Some(true), ..reading(Transport::Usb, true, Battery::Level(40)) };
        assert_eq!(status(device(usb, None), 0), "Charging · USB");
        assert_eq!(status(device(reading(Transport::Internal, true, Battery::Level(40)), None), 0), "This PC");
        assert_eq!(
            status(device(reading(Transport::Bluetooth, true, Battery::NotReported), None), 0),
            "Bluetooth · Doesn't report its battery level"
        );
        assert_eq!(status(device(reading(Transport::Le, true, Battery::Unreadable), None), 0), "Couldn't read the battery level");
        assert_eq!(
            status(device(reading(Transport::Bluetooth, false, Battery::Unreadable), Some((64, 1000))), 1000 + 2 * 3600 + 5),
            "Off or out of range · last seen at 64%, 2 h ago"
        );
        let coarse = Reading { precision: Precision::Coarse, ..reading(Transport::Receiver, false, Battery::Unreadable) };
        assert_eq!(status(device(coarse, Some((25, 1000))), 1000), "Off or out of range · last seen at Low, just now");
        assert_eq!(
            status(device(reading(Transport::Bluetooth, false, Battery::Unreadable), None), 0),
            "Off or out of range · no battery level seen yet"
        );
        // With an estimate.
        let live = device(reading(Transport::Le, true, Battery::Level(80)), None);
        assert_eq!(status_text(&live, 0, Some(5 * 86_400), t), "Bluetooth LE · About 5 days left");
    }

    #[test]
    fn time_left_is_worded_in_hours_then_days() {
        let t = Lang::En.strings();
        assert_eq!(time_left_text(20 * 60, t), "Less than an hour left");
        assert_eq!(time_left_text(3600, t), "About 1 h left");
        assert_eq!(time_left_text(47 * 3600, t), "About 47 h left");
        assert_eq!(time_left_text(48 * 3600, t), "About 2 days left");
        assert_eq!(time_left_text(10 * 86_400 + 13 * 3600, t), "About 11 days left");
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
    fn the_model_report_says_what_the_developer_needs_and_nothing_else() {
        let t = Lang::En.strings();
        let with_ids = Reading {
            hardware: Some((0x054c, 0x0cd3)),
            ..Reading::new("aabbccddeeff", "WH-CH510", Kind::Headphones, Transport::Bluetooth)
        };
        let report = model_report(&device(with_ids, None), t);
        assert_eq!(
            report,
            format!("{}\n\nName: WH-CH510\nKind: headphones\nConnection: Bluetooth\nVendor/product: 054C/0CD3", t.tell_model_note)
        );
        // No address, no ids it does not have; device details in English
        // for the developer, whatever the app's language.
        assert!(!report.contains("aabbccddeeff"));
        let de = Lang::De.strings();
        let plain = model_report(&device(Reading::new("x", "Keys", Kind::Keyboard, Transport::Receiver), None), de);
        assert!(plain.ends_with("Connection: USB receiver") && plain.starts_with(de.tell_model_note));
    }

    #[test]
    fn every_kind_has_a_label_in_every_language() {
        for lang in Lang::ALL {
            let t = lang.strings();
            let labels: Vec<&str> = [Kind::Headphones, Kind::Mouse, Kind::Keyboard, Kind::Gamepad, Kind::Laptop, Kind::Other]
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
