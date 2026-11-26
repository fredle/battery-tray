//! Screenshot mode only: the right-hand corner of a Windows 11 taskbar and
//! a desktop behind it, drawn with egui, so a scene can show the flyout
//! where it really opens. The tray icons are the app's own, from
//! `icon::render`; everything else is plain neutral shapes (an overflow
//! chevron, network and volume glyphs, a clock), not Windows' artwork.

use super::*;

use crate::icon::{self, Spec};
use egui::{Align2, Color32, Mesh, Pos2, Shape, Stroke, pos2};

/// Height of the taskbar, in points.
pub const BAR: f32 = 48.0;
/// Space between the flyout and the desktop's edge and the taskbar.
pub const GAP: f32 = 12.0;
/// The scene's window width: the flyout, and a gap each side.
pub const WIDTH: f32 = flyout::WIDTH + 2.0 * GAP;
const RADIUS: f32 = 8.0;

/// What the scene shows besides the taskbar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stage {
    /// The tray icons, first to last; the first is the one the flyout
    /// opened from.
    pub icons: [Spec; 3],
}

/// The window height for a flyout of `flyout` points.
pub fn canvas_height(flyout: f32) -> f32 {
    GAP + flyout + GAP + BAR
}

/// Where the flyout sits, in the screen's bottom right corner.
pub fn flyout_rect(screen: Rect, height: f32) -> Rect {
    let bottom = screen.bottom() - BAR - GAP;
    Rect::from_min_max(pos2(screen.right() - GAP - flyout::WIDTH, bottom - height), pos2(screen.right() - GAP, bottom))
}

/// The desktop's colours at its four corners: top left, top right, bottom
/// left, bottom right.
fn wallpaper(dark: bool) -> [Color32; 4] {
    if dark {
        [rgb(18, 30, 66), rgb(36, 28, 78), rgb(10, 18, 42), rgb(24, 38, 86)]
    } else {
        [rgb(205, 222, 252), rgb(232, 218, 246), rgb(188, 210, 242), rgb(218, 230, 248)]
    }
}

const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let m = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
    Color32::from_rgb(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()))
}

/// The wallpaper's colour at a point of the screen.
fn wallpaper_at(corners: [Color32; 4], screen: Rect, at: Pos2) -> Color32 {
    let u = ((at.x - screen.left()) / screen.width()).clamp(0.0, 1.0);
    let v = ((at.y - screen.top()) / screen.height()).clamp(0.0, 1.0);
    mix(mix(corners[0], corners[1], u), mix(corners[2], corners[3], u), v)
}

/// The desktop, and the flyout's shadow. Drawn before the flyout.
pub fn backdrop(painter: &egui::Painter, screen: Rect, flyout: Rect, dark: bool) {
    let corners = wallpaper(dark);
    let mut mesh = Mesh::default();
    for (at, colour) in [(screen.left_top(), corners[0]), (screen.right_top(), corners[1]), (screen.left_bottom(), corners[2]), (screen.right_bottom(), corners[3])] {
        mesh.colored_vertex(at, colour);
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(1, 3, 2);
    painter.add(Shape::mesh(mesh));
    let shadow = egui::Shadow { offset: [0, 4], blur: 20, spread: 0, color: Color32::from_black_alpha(if dark { 110 } else { 60 }) };
    painter.add(shadow.as_shape(flyout, RADIUS));
}

/// The flyout's rounded corners and border, over what the flyout drew
/// (egui fills it as a plain rectangle).
pub fn flyout_corners(painter: &egui::Painter, screen: Rect, flyout: Rect, dark: bool, border: Color32) {
    let corners = wallpaper(dark);
    for (corner, sx, sy) in [
        (flyout.left_top(), 1.0, 1.0),
        (flyout.right_top(), -1.0, 1.0),
        (flyout.left_bottom(), 1.0, -1.0),
        (flyout.right_bottom(), -1.0, -1.0),
    ] {
        let mut mesh = Mesh::default();
        let colour = wallpaper_at(corners, screen, corner);
        mesh.colored_vertex(corner, colour);
        let steps = 12;
        for i in 0..=steps {
            let angle = std::f32::consts::FRAC_PI_2 * i as f32 / steps as f32;
            // From the point on the vertical edge to the one on the horizontal.
            let at = pos2(corner.x + sx * RADIUS * (1.0 - angle.sin()), corner.y + sy * RADIUS * (1.0 - angle.cos()));
            mesh.colored_vertex(at, colour);
        }
        for i in 1..=steps {
            mesh.add_triangle(0, i, i + 1);
        }
        painter.add(Shape::mesh(mesh));
    }
    painter.rect_stroke(flyout, RADIUS, Stroke::new(1.0, border), egui::StrokeKind::Inside);
}

/// The taskbar's right-hand end, over the desktop.
pub fn taskbar(ctx: &egui::Context, painter: &egui::Painter, screen: Rect, dark: bool, lang: Lang, stage: &Stage) {
    let (fill, line, fg) = if dark {
        (rgb(26, 26, 28), Color32::from_white_alpha(18), Color32::WHITE)
    } else {
        (rgb(241, 244, 250), Color32::from_black_alpha(22), rgb(28, 28, 28))
    };
    let bar = Rect::from_min_max(pos2(screen.left(), screen.bottom() - BAR), screen.right_bottom());
    painter.rect_filled(bar, 0, fill);
    painter.hline(bar.x_range(), bar.top(), Stroke::new(1.0, line));
    let cy = bar.center().y;
    let right = screen.right();
    let hover = if dark { Color32::from_white_alpha(22) } else { Color32::from_black_alpha(16) };
    let font = FontId::proportional(12.0);

    // The clock.
    let (time, date) = clock(lang);
    let clock_x = right - 40.0;
    painter.text(pos2(clock_x, cy - 8.0), Align2::CENTER_CENTER, time, font.clone(), fg);
    painter.text(pos2(clock_x, cy + 8.0), Align2::CENTER_CENTER, date, font, fg);

    // Network and volume.
    let wifi = pos2(right - 120.0, cy);
    network(painter, wifi, fg);
    speaker(painter, pos2(right - 90.0, cy), fg);

    // The app's icons, and the chevron that holds the hidden ones.
    let size = (16.0 * ctx.pixels_per_point()).round() as usize;
    for (n, spec) in stage.icons.iter().enumerate() {
        let cx = right - 212.0 + 32.0 * n as f32;
        if n == 0 {
            // The one the flyout opened from.
            painter.rect_filled(Rect::from_center_size(pos2(cx, cy), vec2(32.0, 36.0)), 4, hover);
        }
        let rgba = icon::render(spec, size);
        let image = egui::ColorImage::from_rgba_unmultiplied([size, size], &rgba);
        let texture = ctx.load_texture(format!("tray-icon-{n}"), image, egui::TextureOptions::NEAREST);
        let rect = Rect::from_center_size(pos2(cx, cy), vec2(16.0, 16.0));
        painter.image(texture.id(), rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
    }
    let chevron = pos2(right - 260.0, cy);
    painter.add(Shape::line(
        vec![chevron + vec2(-4.5, 2.0), chevron + vec2(0.0, -2.5), chevron + vec2(4.5, 2.0)],
        Stroke::new(1.4, fg),
    ));
}

/// Three arcs and a dot.
fn network(painter: &egui::Painter, centre: Pos2, colour: Color32) {
    let origin = centre + vec2(0.0, 5.0);
    for radius in [4.5, 8.0, 11.5] {
        arc(painter, origin, radius, 45.0, colour);
    }
    painter.circle_filled(origin, 1.5, colour);
}

/// A speaker with two waves.
fn speaker(painter: &egui::Painter, centre: Pos2, colour: Color32) {
    let c = centre;
    painter.add(Shape::convex_polygon(
        vec![c + vec2(-7.0, -2.5), c + vec2(-4.0, -2.5), c + vec2(0.5, -6.0), c + vec2(0.5, 6.0), c + vec2(-4.0, 2.5), c + vec2(-7.0, 2.5)],
        colour,
        Stroke::NONE,
    ));
    for radius in [3.5, 6.5] {
        let mut points = Vec::new();
        for i in 0..=8 {
            let angle = (-50.0 + 100.0 * i as f32 / 8.0_f32).to_radians();
            points.push(c + vec2(1.0 + radius * angle.cos(), radius * angle.sin()));
        }
        painter.add(Shape::line(points, Stroke::new(1.3, colour)));
    }
}

/// An arc opening upwards, `half` degrees to each side of straight up.
fn arc(painter: &egui::Painter, centre: Pos2, radius: f32, half: f32, colour: Color32) {
    let points = (0..=10)
        .map(|i| {
            let angle = (-90.0 - half + 2.0 * half * i as f32 / 10.0).to_radians();
            centre + vec2(radius * angle.cos(), radius * angle.sin())
        })
        .collect();
    painter.add(Shape::line(points, Stroke::new(1.4, colour)));
}

/// The clock's two lines, in the language's habit.
fn clock(lang: Lang) -> (&'static str, &'static str) {
    match lang {
        Lang::En => ("4:00 PM", "11/24/2025"),
        Lang::De | Lang::Ru | Lang::Pl | Lang::Tr => ("16:00", "24.11.2025"),
        Lang::Nl => ("16:00", "24-11-2025"),
        Lang::Ja | Lang::ZhHans | Lang::ZhHant => ("16:00", "2025/11/24"),
        Lang::Ko => ("오후 4:00", "2025-11-24"),
        Lang::Fr | Lang::Es | Lang::It | Lang::Pt => ("16:00", "24/11/2025"),
    }
}
