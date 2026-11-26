//! The app's look: colours for light and dark, type, spacing and fonts.
//! Everything visual that is not layout lives here, so the window code in
//! `app.rs` only says what goes where.

use std::path::PathBuf;
use std::sync::Arc;

use eframe::egui::{
    self, Color32, CornerRadius, FontFamily, FontId, Stroke, TextStyle, Theme, Vec2,
};

use crate::i18n::Lang;

/// Corner radius of cards.
pub const CARD_RADIUS: u8 = 8;
/// Corner radius of buttons and inputs.
pub const CONTROL_RADIUS: u8 = 5;

/// Font family for headings and the main button. Always defined; it falls
/// back to the regular face when Segoe UI Semibold is missing.
pub fn semibold() -> FontFamily {
    FontFamily::Name("semibold".into())
}

#[derive(Clone, Copy)]
pub struct Palette {
    /// Window background.
    pub bg: Color32,
    /// Cards and inputs.
    pub surface: Color32,
    pub border: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub accent: Color32,
    /// Text on an accent fill.
    pub on_accent: Color32,
    /// Accent at low strength, for badges.
    pub accent_soft: Color32,
    /// Resting fill of secondary buttons.
    pub control: Color32,
    pub control_hover: Color32,
    /// Unchecked boxes and the slider's track.
    pub track: Color32,
    pub success: Color32,
    pub warn: Color32,
    pub error: Color32,
}

// Windows 11's own flyout colours, with the app's blue as the accent
// (lighter on dark, as Windows does with its accent).
const LIGHT: Palette = Palette {
    bg: Color32::from_rgb(243, 243, 243),
    surface: Color32::from_rgb(251, 251, 251),
    border: Color32::from_rgb(229, 229, 229),
    text: Color32::from_rgb(27, 27, 27),
    muted: Color32::from_rgb(96, 96, 96),
    accent: Color32::from_rgb(37, 86, 217),
    on_accent: Color32::WHITE,
    accent_soft: Color32::from_rgb(230, 238, 255),
    control: Color32::from_rgb(255, 255, 255),
    control_hover: Color32::from_rgb(240, 240, 240),
    track: Color32::from_rgb(222, 222, 222),
    success: Color32::from_rgb(15, 123, 15),
    warn: Color32::from_rgb(157, 93, 0),
    error: Color32::from_rgb(196, 43, 28),
};

const DARK: Palette = Palette {
    bg: Color32::from_rgb(32, 32, 32),
    surface: Color32::from_rgb(43, 43, 43),
    border: Color32::from_rgb(58, 58, 58),
    text: Color32::from_rgb(255, 255, 255),
    muted: Color32::from_rgb(170, 170, 170),
    accent: Color32::from_rgb(122, 162, 255),
    on_accent: Color32::from_rgb(16, 24, 48),
    accent_soft: Color32::from_rgb(44, 52, 74),
    control: Color32::from_rgb(55, 55, 55),
    control_hover: Color32::from_rgb(65, 65, 65),
    track: Color32::from_rgb(72, 72, 72),
    success: Color32::from_rgb(108, 203, 95),
    warn: Color32::from_rgb(252, 200, 90),
    error: Color32::from_rgb(255, 153, 164),
};

/// A value under HKCU's Personalize key: 1 for light, 0 for dark.
fn personalize(value: windows::core::PCWSTR) -> Option<bool> {
    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
    let mut data = 0u32;
    let mut size = size_of::<u32>() as u32;
    let found = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            windows::core::w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            value,
            RRF_RT_REG_DWORD,
            None,
            Some((&raw mut data).cast()),
            Some(&mut size),
        ) == ERROR_SUCCESS
    };
    found.then_some(data != 0)
}

/// Whether Windows shows apps light (Settings > Personalisation > Colours).
pub fn apps_light() -> bool {
    personalize(windows::core::w!("AppsUseLightTheme")).unwrap_or(true)
}

/// Whether the taskbar is light, for the tray icons' colour. Windows 10's
/// taskbar is dark unless set otherwise.
pub fn taskbar_light() -> bool {
    personalize(windows::core::w!("SystemUsesLightTheme")).unwrap_or(false)
}

/// Follows Windows' light or dark setting for apps, unless `HWC_THEME`
/// (light or dark) says otherwise, for checking both looks.
pub fn follow_windows(ctx: &egui::Context) {
    let theme = match std::env::var("HWC_THEME").as_deref() {
        Ok("dark") => Theme::Dark,
        Ok("light") => Theme::Light,
        _ if apps_light() => Theme::Light,
        _ => Theme::Dark,
    };
    ctx.set_theme(theme);
}

pub fn palette(ctx: &egui::Context) -> Palette {
    match ctx.theme() {
        Theme::Dark => DARK,
        Theme::Light => LIGHT,
    }
}

fn visuals(p: Palette, mut v: egui::Visuals) -> egui::Visuals {
    let radius = CornerRadius::same(CONTROL_RADIUS);
    v.panel_fill = p.bg;
    v.window_fill = p.surface;
    v.window_stroke = Stroke::new(1.0, p.border);
    v.window_corner_radius = CornerRadius::same(CARD_RADIUS);
    v.menu_corner_radius = CornerRadius::same(CONTROL_RADIUS);
    v.extreme_bg_color = p.surface;
    v.faint_bg_color = p.bg;
    v.hyperlink_color = p.accent;
    v.warn_fg_color = p.warn;
    v.error_fg_color = p.error;
    v.selection.bg_fill = p.accent;
    v.selection.stroke = Stroke::new(1.0, p.on_accent);
    v.slider_trailing_fill = true;
    v.handle_shape = egui::style::HandleShape::Circle;

    let w = &mut v.widgets;
    w.noninteractive.bg_fill = p.surface;
    w.noninteractive.weak_bg_fill = p.surface;
    w.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
    w.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
    w.noninteractive.corner_radius = radius;

    w.inactive.bg_fill = p.track;
    w.inactive.weak_bg_fill = p.control;
    w.inactive.bg_stroke = Stroke::new(1.0, p.border);
    w.inactive.fg_stroke = Stroke::new(1.0, p.text);
    w.inactive.corner_radius = radius;

    w.hovered.bg_fill = p.control_hover;
    w.hovered.weak_bg_fill = p.control_hover;
    w.hovered.bg_stroke = Stroke::new(1.0, p.accent);
    w.hovered.fg_stroke = Stroke::new(1.5, p.text);
    w.hovered.corner_radius = radius;
    w.hovered.expansion = 0.0;

    w.active.bg_fill = p.accent_soft;
    w.active.weak_bg_fill = p.accent_soft;
    w.active.bg_stroke = Stroke::new(1.0, p.accent);
    w.active.fg_stroke = Stroke::new(1.5, p.text);
    w.active.corner_radius = radius;
    w.active.expansion = 0.0;

    w.open.bg_fill = p.control_hover;
    w.open.weak_bg_fill = p.control_hover;
    w.open.bg_stroke = Stroke::new(1.0, p.accent);
    w.open.fg_stroke = Stroke::new(1.0, p.text);
    w.open.corner_radius = radius;
    v
}

/// Colours, type sizes and spacing. Call once at start.
pub fn apply(ctx: &egui::Context) {
    ctx.set_visuals_of(Theme::Light, visuals(LIGHT, egui::Visuals::light()));
    ctx.set_visuals_of(Theme::Dark, visuals(DARK, egui::Visuals::dark()));
    ctx.all_styles_mut(|style| {
        style.spacing.item_spacing = Vec2::new(8.0, 8.0);
        style.spacing.button_padding = Vec2::new(12.0, 6.0);
        style.spacing.interact_size.y = 30.0;
        style.spacing.slider_width = 150.0;
        style.spacing.icon_width = 18.0;
        style.spacing.combo_width = 110.0;
        for (text_style, size) in [
            (TextStyle::Body, 14.5),
            (TextStyle::Button, 14.5),
            (TextStyle::Small, 12.0),
            (TextStyle::Heading, 20.0),
        ] {
            style
                .text_styles
                .insert(text_style, FontId::new(size, FontFamily::Proportional));
        }
    });
}

/// Whether some text (a device name) needs Windows' symbol font: arrows,
/// dingbats, emoji. Segoe UI has every letter the app's own texts use.
pub fn needs_symbols<'a>(texts: impl IntoIterator<Item = &'a str>) -> bool {
    texts
        .into_iter()
        .any(|text| text.chars().any(|c| matches!(c as u32, 0x2190..=0x2BFF | 0x1F000..=0x1FAFF)))
}

/// Uses Windows' Segoe UI for the interface, so the app looks at home, and
/// adds Windows' East Asian fonts for the display language and Windows'
/// language (so device names in the user's script show too), and Segoe UI
/// Symbol when `symbols` (see `needs_symbols`). No font is built into the
/// exe: Windows has them all, and only those needed are read.
pub fn install_fonts(ctx: &egui::Context, languages: &[Lang], symbols: bool) {
    let fonts_dir = PathBuf::from(std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into()))
        .join("Fonts");
    let mut fonts = egui::FontDefinitions::default();
    let add = |fonts: &mut egui::FontDefinitions, file: &str, shift: f32| -> bool {
        if fonts.font_data.contains_key(file) {
            return true;
        }
        let Ok(bytes) = std::fs::read(fonts_dir.join(file)) else {
            return false;
        };
        let data = egui::FontData::from_owned(bytes).tweak(egui::FontTweak {
            y_offset_factor: shift,
            ..Default::default()
        });
        fonts.font_data.insert(file.to_owned(), Arc::new(data));
        true
    };

    let proportional = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    let mut regular = proportional.clone();
    // Segoe UI is on every Windows 10 and 11; the others only in case.
    if let Some(file) = ["segoeui.ttf", "arial.ttf", "tahoma.ttf"].into_iter().find(|f| add(&mut fonts, f, 0.0)) {
        regular.insert(0, file.to_owned());
    }
    let mut strong = regular.clone();
    if add(&mut fonts, "seguisb.ttf", 0.0) {
        strong.insert(0, "seguisb.ttf".to_owned());
    }
    for lang in languages {
        // The shift puts mixed text ("残量 82%") on one baseline:
        // some of these fonts sit higher on the line than the Latin font.
        let found = lang
            .system_fonts()
            .iter()
            .find(|(file, shift)| add(&mut fonts, file, *shift));
        if let Some((file, _)) = found {
            for list in [&mut regular, &mut strong] {
                if !list.iter().any(|name| name == file) {
                    list.push((*file).to_owned());
                }
            }
        }
    }
    if symbols && add(&mut fonts, "seguisym.ttf", 0.0) {
        regular.push("seguisym.ttf".to_owned());
        strong.push("seguisym.ttf".to_owned());
    }
    // Nothing in the app is set in a fixed-width face, but egui wants one.
    fonts.families.insert(FontFamily::Monospace, regular.clone());
    fonts.families.insert(FontFamily::Proportional, regular);
    fonts.families.insert(semibold(), strong);
    ctx.set_fonts(fonts);
}

/// A white (or dark) rounded panel that groups related things.
pub fn card(p: Palette) -> egui::Frame {
    egui::Frame::new()
        .fill(p.surface)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CARD_RADIUS)
        .inner_margin(egui::Margin::symmetric(14, 12))
}

/// The one filled button on the screen: the thing to press next.
pub fn primary_button<'a>(p: Palette, text: impl Into<String>, enabled: bool) -> egui::Button<'a> {
    let label = egui::RichText::new(text)
        .family(semibold())
        .size(15.5)
        .color(if enabled { p.on_accent } else { p.muted });
    egui::Button::new(label)
        .fill(if enabled { p.accent } else { p.control_hover })
        .stroke(Stroke::NONE)
        .corner_radius(10)
        .min_size(Vec2::new(230.0, 40.0))
}

/// A checkbox with squarer corners than the buttons: at this size the
/// button radius would make it look like a radio button.
pub fn checkbox(ui: &mut egui::Ui, checked: &mut bool, text: impl Into<egui::WidgetText>) -> egui::Response {
    ui.scope(|ui| {
        let widgets = &mut ui.visuals_mut().widgets;
        for state in [&mut widgets.inactive, &mut widgets.hovered, &mut widgets.active] {
            state.corner_radius = CornerRadius::same(5);
        }
        ui.checkbox(checked, text)
    })
    .inner
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_names_with_symbols_or_emoji_load_the_symbol_font() {
        assert!(!needs_symbols(["WH-1000XM4", "Clavier de Zoé", "Наушники", "エアーポッズ", "Mouse – “Work”…"]));
        assert!(needs_symbols(["Fred's 🎧"]));
        assert!(needs_symbols(["Keys", "★ Pad"]));
        assert!(!needs_symbols(std::iter::empty::<&str>()));
    }
}