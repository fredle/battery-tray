//! Tray icons, rendered here as RGBA pixels at the size Windows asks for
//! (16 px at 100%, 20 at 125%, 24 at 150%, 32 at 200%), so they are sharp
//! rather than scaled. `tray.rs` hands them to Windows.
//!
//! A device's icon is its level as a number in the taskbar's own text
//! colour (white on a dark taskbar, near-black on a light one), over a
//! thin bar that fills with the level: red when low, green while charging.
//! A device that reports only a band (an Xbox controller) gets a battery
//! with that many segments instead of a made-up number, and a full battery
//! stands for 100, which does not fit. With nothing to show, the icon is
//! the app's own: a battery sending a signal on a blue tile.
//!
//! The digits are Bahnschrift, drawn by GDI into a coverage mask at the exact
//! size; everything else is plain geometry with 4x4 supersampling.

use crate::devices::{Coarse, Precision};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Look {
    /// A level, in the device's precision.
    Level(u8, Precision),
    /// A device that has no level right now.
    NoLevel,
    /// The app's own icon.
    App,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Normal,
    /// At or under the device's warning level.
    Low,
    Charging,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Spec {
    pub look: Look,
    pub tone: Tone,
    /// The taskbar is light (Windows' "SystemUsesLightTheme").
    pub light_taskbar: bool,
}

type Rgb = [u8; 3];

/// Text and glyph colours for each taskbar, tuned for contrast on it.
struct Colours {
    fg: Rgb,
    low: Rgb,
    charging: Rgb,
}

const ON_DARK: Colours = Colours { fg: [255, 255, 255], low: [255, 99, 99], charging: [108, 214, 96] };
const ON_LIGHT: Colours = Colours { fg: [28, 28, 28], low: [196, 43, 28], charging: [16, 124, 16] };

/// The app tile's gradient, from the app icon.
const TILE_TOP: Rgb = [60, 120, 255];
const TILE_BOTTOM: Rgb = [37, 86, 217];
const ARC_OUTER: Rgb = [196, 216, 255];

/// A canvas of coverage-blended colour.
struct Canvas {
    size: usize,
    rgba: Vec<u8>,
}

impl Canvas {
    fn new(size: usize) -> Canvas {
        Canvas { size, rgba: vec![0; size * size * 4] }
    }

    /// Paints `colour` at `alpha` (0-1) over pixel (x, y), "over" blending.
    fn blend(&mut self, x: usize, y: usize, colour: Rgb, alpha: f32) {
        if alpha <= 0.0 || x >= self.size || y >= self.size {
            return;
        }
        let at = (y * self.size + x) * 4;
        let dst_a = f32::from(self.rgba[at + 3]) / 255.0;
        let out_a = alpha + dst_a * (1.0 - alpha);
        for c in 0..3 {
            let dst = f32::from(self.rgba[at + c]);
            let value = (f32::from(colour[c]) * alpha + dst * dst_a * (1.0 - alpha)) / out_a.max(1e-6);
            self.rgba[at + c] = value.round().clamp(0.0, 255.0) as u8;
        }
        self.rgba[at + 3] = (out_a * 255.0).round().clamp(0.0, 255.0) as u8;
    }

    /// Fills whatever `inside` covers, sampled 4 x 4 per pixel. `colour`
    /// may vary down the canvas (0 at the top, 1 at the bottom).
    fn fill(&mut self, inside: impl Fn(f32, f32) -> bool, colour: impl Fn(f32) -> Rgb, alpha: f32) {
        for y in 0..self.size {
            for x in 0..self.size {
                let mut hits = 0;
                for sy in 0..4 {
                    for sx in 0..4 {
                        let px = x as f32 + (sx as f32 + 0.5) / 4.0;
                        let py = y as f32 + (sy as f32 + 0.5) / 4.0;
                        if inside(px, py) {
                            hits += 1;
                        }
                    }
                }
                if hits > 0 {
                    self.blend(x, y, colour(y as f32 / self.size as f32), alpha * hits as f32 / 16.0);
                }
            }
        }
    }
}

/// Inside a rectangle with rounded corners of `radius`.
fn rounded(x0: f32, y0: f32, x1: f32, y1: f32, radius: f32) -> impl Fn(f32, f32) -> bool {
    move |x, y| {
        if x < x0 || x > x1 || y < y0 || y > y1 {
            return false;
        }
        let r = radius.min((x1 - x0) / 2.0).min((y1 - y0) / 2.0).max(0.0);
        let dx = (x0 + r - x).max(x - (x1 - r)).max(0.0);
        let dy = (y0 + r - y).max(y - (y1 - r)).max(0.0);
        dx * dx + dy * dy <= r * r + 1e-6
    }
}

/// On a ring of `radius` and `width` around (cx, cy), to the right of the
/// centre between -`spread` and `spread` radians, with round ends.
fn arc(cx: f32, cy: f32, radius: f32, width: f32, spread: f32) -> impl Fn(f32, f32) -> bool {
    move |x, y| {
        let (dx, dy) = (x - cx, y - cy);
        let angle = dy.atan2(dx);
        if angle.abs() <= spread {
            return ((dx * dx + dy * dy).sqrt() - radius).abs() <= width / 2.0;
        }
        let end = spread * angle.signum();
        let (ex, ey) = (cx + radius * end.cos(), cy + radius * end.sin());
        (x - ex).powi(2) + (y - ey).powi(2) <= (width / 2.0).powi(2)
    }
}

/// RGBA pixels, `size` x `size`, top row first.
pub fn render(spec: &Spec, size: usize) -> Vec<u8> {
    let size = size.clamp(12, 256);
    let colours = if spec.light_taskbar { &ON_LIGHT } else { &ON_DARK };
    let mut canvas = Canvas::new(size);
    match spec.look {
        Look::App => app(&mut canvas),
        Look::Level(level, Precision::Coarse) => {
            let segments = match Coarse::from_level(level) {
                Coarse::VeryLow => 0,
                Coarse::Low => 1,
                Coarse::Medium => 2,
                Coarse::Full => 3,
            };
            battery(&mut canvas, colours, spec.tone, segments);
        }
        Look::Level(level, _) if level >= 100 => battery(&mut canvas, colours, spec.tone, 3),
        Look::Level(level, _) => number(&mut canvas, colours, spec.tone, &level.to_string(), Some(level)),
        Look::NoLevel => number(&mut canvas, colours, Tone::Normal, "–", None),
    }
    canvas.rgba
}

/// The bar's thickness and the height left above it for the digits.
fn bar_layout(size: usize) -> (f32, f32) {
    let unit = size as f32 / 16.0;
    let bar = (2.0 * unit).round().max(2.0);
    let gap = unit.round().max(1.0);
    (bar, size as f32 - bar - gap)
}

/// Digits over a level bar.
fn number(canvas: &mut Canvas, colours: &Colours, tone: Tone, text: &str, level: Option<u8>) {
    let size = canvas.size;
    let (bar, text_height) = bar_layout(size);
    let text_colour = match tone {
        Tone::Low => colours.low,
        _ => colours.fg,
    };
    if let Some(mask) = text_mask(text, size as i32, text_height as i32) {
        let left = (size as i32 - mask.width) / 2;
        let top = ((text_height as i32 - mask.height) / 2).max(0);
        for y in 0..mask.height {
            for x in 0..mask.width {
                let coverage = f32::from(mask.coverage[(y * mask.width + x) as usize]) / 255.0;
                let alpha = if level.is_none() { coverage * 0.6 } else { coverage };
                let (px, py) = (left + x, top + y);
                if px >= 0 && py >= 0 {
                    canvas.blend(px as usize, py as usize, text_colour, alpha);
                }
            }
        }
    }
    // The bar: a faint track, filled to the level.
    let s = size as f32;
    let inset = (s / 16.0).round().max(1.0);
    let (y0, y1) = (s - bar, s);
    let (x0, x1) = (inset, s - inset);
    canvas.fill(rounded(x0, y0, x1, y1, bar / 2.0), |_| colours.fg, 0.3);
    if let Some(level) = level {
        let fill = match tone {
            Tone::Normal => colours.fg,
            Tone::Low => colours.low,
            Tone::Charging => colours.charging,
        };
        let end = (x0 + (x1 - x0) * f32::from(level.min(100)) / 100.0).max(x0 + bar);
        canvas.fill(rounded(x0, y0, end, y1, bar / 2.0), |_| fill, 1.0);
    }
}

/// A battery with 0-3 segments: a band, or full.
fn battery(canvas: &mut Canvas, colours: &Colours, tone: Tone, segments: u8) {
    let s = canvas.size as f32;
    let unit = s / 16.0;
    let line = unit.round().max(1.0);
    let (x0, x1) = ((0.5 * unit).round(), (s - 2.5 * unit).round());
    let (y0, y1) = ((3.5 * unit).round(), (s - 3.5 * unit).round());
    let outline = match tone {
        Tone::Low => colours.low,
        _ => colours.fg,
    };
    let outer = rounded(x0, y0, x1, y1, 2.0 * unit);
    let inner = rounded(x0 + line, y0 + line, x1 - line, y1 - line, (2.0 * unit - line).max(0.5));
    canvas.fill(|x, y| outer(x, y) && !inner(x, y), |_| outline, 1.0);
    // The terminal.
    let (t0, t1) = ((s / 2.0 - 2.0 * unit).round(), (s / 2.0 + 2.0 * unit).round());
    canvas.fill(rounded(x1 + (0.5 * unit).round(), t0, x1 + (2.0 * unit).round(), t1, 0.8 * unit), |_| outline, 1.0);
    let fill = match tone {
        Tone::Normal => colours.fg,
        Tone::Low => colours.low,
        Tone::Charging => colours.charging,
    };
    // Segments inside, with a pixel of air around them.
    let gap = unit.round().max(1.0);
    let (ix0, ix1) = (x0 + line + gap, x1 - line - gap);
    let (iy0, iy1) = (y0 + line + gap, y1 - line - gap);
    let width = (ix1 - ix0 - 2.0 * gap) / 3.0;
    if segments == 0 {
        // Nearly empty: a sliver, so it reads as "almost nothing", not "no data".
        canvas.fill(rounded(ix0, iy0, ix0 + (width / 3.0).max(1.0), iy1, 0.0), |_| fill, 1.0);
    }
    for i in 0..segments.min(3) {
        let left = ix0 + f32::from(i) * (width + gap);
        canvas.fill(rounded(left, iy0, left + width, iy1, 0.4 * unit), |_| fill, 1.0);
    }
}

/// The app icon: a blue tile, a half-full battery and two signal arcs
/// (assets/icon.svg, on its 256 grid).
fn app(canvas: &mut Canvas) {
    let s = canvas.size as f32;
    let k = s / 256.0;
    canvas.fill(rounded(0.0, 0.0, s, s, 56.0 * k), |t| mix(TILE_TOP, TILE_BOTTOM, t), 1.0);
    // The SVG scales its glyph by 0.84 around the centre and shifts it 3 right.
    let g = |v: f32| (128.0 + (v - 128.0) * 0.84) * k;
    let gx = |v: f32| g(v + 3.0);
    let line = 16.0 * 0.84 * k;
    let white = [255, 255, 255];
    let outer = rounded(gx(24.0) - line / 2.0, g(80.0) - line / 2.0, gx(152.0) + line / 2.0, g(176.0) + line / 2.0, 26.0 * 0.84 * k);
    let inner = rounded(gx(24.0) + line / 2.0, g(80.0) + line / 2.0, gx(152.0) - line / 2.0, g(176.0) - line / 2.0, 10.0 * 0.84 * k);
    canvas.fill(|x, y| outer(x, y) && !inner(x, y), |_| white, 1.0);
    canvas.fill(rounded(gx(40.0), g(96.0), gx(92.0), g(160.0), 8.0 * 0.84 * k), |_| white, 1.0);
    canvas.fill(rounded(gx(152.0), g(104.0), gx(176.0), g(152.0), 8.0 * 0.84 * k), |_| white, 1.0);
    let width = 14.0 * 0.84 * k;
    // Both arcs centre on (160, 128) of the SVG; about 50 degrees either side.
    canvas.fill(arc(gx(160.0), g(128.0), 28.0 * 0.84 * k, width, 0.87), |_| white, 1.0);
    canvas.fill(arc(gx(160.0), g(128.0), 56.0 * 0.84 * k, width, 0.87), |_| ARC_OUTER, 1.0);
}

fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let t = t.clamp(0.0, 1.0);
    [0, 1, 2].map(|i| (f32::from(a[i]) + (f32::from(b[i]) - f32::from(a[i])) * t).round() as u8)
}

/// Coverage of text, cropped to its ink.
struct Mask {
    width: i32,
    height: i32,
    coverage: Vec<u8>,
}

/// `text` in Bahnschrift SemiCondensed (narrow, so two digits stand tall in
/// 16 pixels; Windows 10 1709 and later have it), as an anti-aliased coverage mask drawn by
/// GDI (white on black, so a colour channel is the coverage). The size is
/// the largest at which "88" fits `max_width` x `max_height` pixels of ink,
/// so every number is drawn at the same size and "7" is not a giant.
fn text_mask(text: &str, max_width: i32, max_height: i32) -> Option<Mask> {
    let mut height = max_height * 2;
    while height >= 4 {
        let fits = draw_text("88", height, max_width * 2 + max_height * 3)
            .is_some_and(|m| m.width <= max_width && m.height <= max_height);
        if fits {
            return draw_text(text, height, max_width * 2 + max_height * 3);
        }
        height -= 1;
    }
    None
}

/// `text` at a font height of `height` pixels, cropped to its ink.
fn draw_text(text: &str, height: i32, canvas: i32) -> Option<Mask> {
    use windows::Win32::Foundation::COLORREF;
    use windows::Win32::Graphics::Gdi::{
        ANTIALIASED_QUALITY, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CLIP_DEFAULT_PRECIS, CreateCompatibleDC,
        CreateDIBSection, CreateFontW, DEFAULT_CHARSET, DIB_RGB_COLORS, DeleteDC, DeleteObject, FF_DONTCARE,
        GdiFlush, OUT_TT_PRECIS, SelectObject, SetBkMode, SetTextColor, TRANSPARENT, TextOutW,
    };
    use windows::core::w;

    let wide: Vec<u16> = text.encode_utf16().collect();
    unsafe {
        let dc = CreateCompatibleDC(None);
        if dc.is_invalid() {
            return None;
        }
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: canvas,
                // Negative: top row first.
                biHeight: -canvas,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let Ok(bitmap) = CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0) else {
            let _ = DeleteDC(dc);
            return None;
        };
        let old_bitmap = SelectObject(dc, bitmap.into());
        SetBkMode(dc, TRANSPARENT);
        SetTextColor(dc, COLORREF(0x00FF_FFFF));
        let pixels = std::slice::from_raw_parts_mut(bits.cast::<u8>(), (canvas * canvas * 4) as usize);

        let mut found = None;
        {
            let font = CreateFontW(
                -height,
                0,
                0,
                0,
                600,
                0,
                0,
                0,
                DEFAULT_CHARSET,
                OUT_TT_PRECIS,
                CLIP_DEFAULT_PRECIS,
                ANTIALIASED_QUALITY,
                FF_DONTCARE.0 as u32,
                w!("Bahnschrift SemiCondensed"),
            );
            let old_font = SelectObject(dc, font.into());
            pixels.fill(0);
            let _ = TextOutW(dc, 4, 4, &wide);
            let _ = GdiFlush();
            SelectObject(dc, old_font);
            let _ = DeleteObject(font.into());
            let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, -1, -1);
            for y in 0..canvas {
                for x in 0..canvas {
                    if pixels[((y * canvas + x) * 4 + 1) as usize] > 24 {
                        x0 = x0.min(x);
                        x1 = x1.max(x);
                        y0 = y0.min(y);
                        y1 = y1.max(y);
                    }
                }
            }
            if x1 >= 0 {
                let (width, height) = (x1 - x0 + 1, y1 - y0 + 1);
                let mut coverage = Vec::with_capacity((width * height) as usize);
                for y in y0..=y1 {
                    for x in x0..=x1 {
                        coverage.push(pixels[((y * canvas + x) * 4 + 1) as usize]);
                    }
                }
                found = Some(Mask { width, height, coverage });
            }
        }
        SelectObject(dc, old_bitmap);
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(dc);
        found
    }
}

/// The icon size Windows uses in the tray: the small-icon size at the
/// system DPI.
pub fn tray_size() -> usize {
    use windows::Win32::UI::HiDpi::{GetDpiForSystem, GetSystemMetricsForDpi};
    use windows::Win32::UI::WindowsAndMessaging::SM_CXSMICON;
    let size = unsafe { GetSystemMetricsForDpi(SM_CXSMICON, GetDpiForSystem()) };
    if size > 0 { size as usize } else { 16 }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(look: Look, tone: Tone, light: bool) -> Spec {
        Spec { look, tone, light_taskbar: light }
    }

    fn pixel(pixels: &[u8], size: usize, x: usize, y: usize) -> [u8; 4] {
        let at = (y * size + x) * 4;
        [pixels[at], pixels[at + 1], pixels[at + 2], pixels[at + 3]]
    }

    /// Pixels of `colour` with at least half coverage.
    fn count(pixels: &[u8], colour: Rgb) -> usize {
        pixels.chunks(4).filter(|p| p[..3] == colour && p[3] >= 128).count()
    }

    #[test]
    fn renders_at_every_tray_size() {
        for size in [16, 20, 24, 32] {
            let pixels = render(&spec(Look::Level(82, Precision::Percent), Tone::Normal, false), size);
            assert_eq!(pixels.len(), size * size * 4);
            assert!(count(&pixels, ON_DARK.fg) > size, "{size}");
            assert_eq!(pixel(&pixels, size, 0, 0)[3], 0);
        }
    }

    #[test]
    fn every_level_draws_differently() {
        for size in [16, 24] {
            let all: Vec<Vec<u8>> = (0..100)
                .map(|level| render(&spec(Look::Level(level, Precision::Percent), Tone::Normal, true), size))
                .collect();
            for a in 0..all.len() {
                for b in a + 1..all.len() {
                    assert_ne!(all[a], all[b], "{size}: {a} and {b} look the same");
                }
            }
        }
    }

    #[test]
    fn one_digit_is_drawn_as_tall_as_two() {
        let ink_rows = |pixels: &[u8], size: usize| {
            (0..size).filter(|y| (0..size).any(|x| pixel(pixels, size, x, *y)[3] > 128)).count()
        };
        for size in [16, 24] {
            let seven = render(&spec(Look::Level(7, Precision::Percent), Tone::Normal, false), size);
            let forty_two = render(&spec(Look::Level(42, Precision::Percent), Tone::Normal, false), size);
            assert!(ink_rows(&seven, size).abs_diff(ink_rows(&forty_two, size)) <= 1, "{size}");
        }
    }

    #[test]
    fn colour_follows_the_taskbar_and_the_state() {
        let size = 24;
        let level = Look::Level(15, Precision::Percent);
        let dark = render(&spec(level, Tone::Normal, false), size);
        let light = render(&spec(level, Tone::Normal, true), size);
        assert!(count(&dark, ON_DARK.fg) > 0 && count(&dark, ON_LIGHT.fg) == 0);
        assert!(count(&light, ON_LIGHT.fg) > 0 && count(&light, ON_DARK.fg) == 0);
        let low = render(&spec(level, Tone::Low, true), size);
        assert!(count(&low, ON_LIGHT.low) > 20);
        let charging = render(&spec(Look::Level(60, Precision::Percent), Tone::Charging, false), size);
        assert!(count(&charging, ON_DARK.charging) > 10);
        assert_eq!(count(&charging, ON_DARK.low), 0);
    }

    #[test]
    fn coarse_levels_and_full_are_batteries_not_numbers() {
        let size = 20;
        let band = |level| render(&spec(Look::Level(level, Precision::Coarse), Tone::Normal, false), size);
        let bands: Vec<Vec<u8>> = [5, 25, 60, 100].into_iter().map(band).collect();
        let ink: Vec<usize> = bands.iter().map(|p| count(p, ON_DARK.fg)).collect();
        assert!(ink.windows(2).all(|w| w[0] < w[1]), "{ink:?}");
        assert_eq!(band(70), band(100));
        assert_eq!(render(&spec(Look::Level(100, Precision::Percent), Tone::Normal, false), size), bands[3]);
    }

    #[test]
    fn the_app_icon_is_a_blue_tile_with_a_white_battery() {
        for size in [16, 32] {
            let pixels = render(&spec(Look::App, Tone::Normal, true), size);
            assert_eq!(pixel(&pixels, size, 0, 0)[3], 0, "rounded corner");
            let middle_left = pixel(&pixels, size, 0, size / 2);
            assert!(middle_left[2] > 200 && middle_left[0] < 80, "{middle_left:?}");
            let white = pixels.chunks(4).filter(|p| p[0] > 220 && p[1] > 220 && p[2] > 220).count();
            assert!(white > size, "{size}: {white}");
            assert_eq!(pixels, render(&spec(Look::App, Tone::Normal, false), size));
        }
    }

    #[test]
    fn no_level_is_a_faint_dash_and_an_empty_bar() {
        let pixels = render(&spec(Look::NoLevel, Tone::Low, false), 16);
        assert_eq!(count(&pixels, ON_DARK.low), 0);
        assert!(pixels.chunks(4).any(|p| p[3] > 0));
        assert!(pixels.chunks(4).all(|p| p[3] < 250));
    }

    /// Writes a sheet of icons on both taskbars to `BT_ICON_DIR`, to look at:
    /// `BT_ICON_DIR=... cargo test icon_sheet -- --ignored`
    #[test]
    #[ignore]
    fn icon_sheet() {
        let dir = std::path::PathBuf::from(std::env::var("BT_ICON_DIR").expect("set BT_ICON_DIR"));
        let looks = [
            (Look::Level(7, Precision::Percent), Tone::Low),
            (Look::Level(42, Precision::Percent), Tone::Normal),
            (Look::Level(85, Precision::Percent), Tone::Normal),
            (Look::Level(63, Precision::Percent), Tone::Charging),
            (Look::Level(100, Precision::Percent), Tone::Normal),
            (Look::Level(70, Precision::Tens), Tone::Normal),
            (Look::Level(5, Precision::Coarse), Tone::Low),
            (Look::Level(25, Precision::Coarse), Tone::Low),
            (Look::Level(60, Precision::Coarse), Tone::Normal),
            (Look::Level(100, Precision::Coarse), Tone::Charging),
            (Look::NoLevel, Tone::Normal),
            (Look::App, Tone::Normal),
        ];
        for (light, background) in [(false, [32u8, 32, 32]), (true, [238, 238, 238])] {
            let zoom = 4;
            let cell = 32 * zoom + 8;
            let (width, height) = (looks.len() * cell + 8, 4 * cell + 8);
            let mut sheet = vec![0u8; width * height * 4];
            for p in sheet.chunks_mut(4) {
                p.copy_from_slice(&[background[0], background[1], background[2], 255]);
            }
            for (row, size) in [16usize, 20, 24, 32].into_iter().enumerate() {
                for (column, (look, tone)) in looks.iter().enumerate() {
                    let icon = render(&spec(*look, *tone, light), size);
                    for y in 0..size * zoom {
                        for x in 0..size * zoom {
                            let src = ((y / zoom) * size + x / zoom) * 4;
                            let a = f32::from(icon[src + 3]) / 255.0;
                            let (dx, dy) = (8 + column * cell + x, 8 + row * cell + y);
                            let dst = (dy * width + dx) * 4;
                            for c in 0..3 {
                                sheet[dst + c] = (f32::from(icon[src + c]) * a + f32::from(background[c]) * (1.0 - a)) as u8;
                            }
                        }
                    }
                }
            }
            let name = if light { "icons-light.png" } else { "icons-dark.png" };
            crate::shots::write_png(&dir.join(name), width, height, &sheet).unwrap();
        }
    }
}
