//! Small pictures drawn with egui's painter: what kind of device a card is
//! for, the charging bolt, and the header buttons. Drawn rather than taken
//! from an icon font, so they follow the palette and stay sharp at any
//! scale.
//!
//! Every glyph is laid out on a 20 x 20 grid and scaled to the rect it is
//! given.

use std::f32::consts::PI;

use eframe::egui::{self, Color32, Painter, Pos2, Rect, Shape, Stroke, vec2};
#[cfg(test)]
use eframe::egui::pos2;

use crate::devices::Kind;

/// Maps the 20 x 20 grid onto `rect` (square, centred).
struct Grid {
    origin: Pos2,
    unit: f32,
}

impl Grid {
    fn new(rect: Rect) -> Grid {
        let side = rect.width().min(rect.height());
        Grid {
            origin: rect.center() - vec2(side, side) / 2.0,
            unit: side / 20.0,
        }
    }

    fn p(&self, x: f32, y: f32) -> Pos2 {
        self.origin + vec2(x, y) * self.unit
    }

    fn r(&self, x0: f32, y0: f32, x1: f32, y1: f32) -> Rect {
        Rect::from_min_max(self.p(x0, y0), self.p(x1, y1))
    }

    fn stroke(&self, width: f32, colour: Color32) -> Stroke {
        Stroke::new((width * self.unit).max(1.0), colour)
    }

    /// An arc around (cx, cy) from `from` to `to` radians (0 = right,
    /// clockwise on screen).
    fn arc(&self, cx: f32, cy: f32, radius: f32, from: f32, to: f32) -> Vec<Pos2> {
        let steps = 24;
        (0..=steps)
            .map(|i| {
                let a = from + (to - from) * i as f32 / steps as f32;
                self.p(cx + radius * a.cos(), cy + radius * a.sin())
            })
            .collect()
    }
}

/// The picture for a kind of device.
pub fn kind(painter: &Painter, rect: Rect, kind: Kind, colour: Color32) {
    let g = Grid::new(rect);
    let line = g.stroke(1.6, colour);
    match kind {
        Kind::Headphones => {
            painter.add(Shape::line(g.arc(10.0, 11.0, 7.0, PI, 2.0 * PI), line));
            painter.rect_filled(g.r(2.2, 10.0, 6.2, 17.0), 1.6 * g.unit, colour);
            painter.rect_filled(g.r(13.8, 10.0, 17.8, 17.0), 1.6 * g.unit, colour);
        }
        Kind::Mouse => {
            painter.rect_stroke(g.r(5.5, 2.0, 14.5, 18.0), 4.5 * g.unit, line, egui::StrokeKind::Middle);
            painter.line_segment([g.p(10.0, 2.0), g.p(10.0, 8.0)], line);
            painter.line_segment([g.p(5.5, 8.0), g.p(14.5, 8.0)], line);
        }
        Kind::Keyboard => {
            painter.rect_stroke(g.r(1.5, 5.0, 18.5, 15.5), 2.0 * g.unit, line, egui::StrokeKind::Middle);
            for y in [8.2, 10.7] {
                for x in [4.6, 7.8, 11.0, 14.2] {
                    painter.rect_filled(g.r(x - 0.8, y - 0.8, x + 1.4, y + 0.8), 0.4 * g.unit, colour);
                }
            }
            painter.line_segment([g.p(6.5, 13.2), g.p(13.5, 13.2)], line);
        }
        Kind::Gamepad => {
            painter.rect_stroke(g.r(1.8, 5.5, 18.2, 15.5), 5.0 * g.unit, line, egui::StrokeKind::Middle);
            painter.line_segment([g.p(4.4, 10.5), g.p(8.0, 10.5)], line);
            painter.line_segment([g.p(6.2, 8.7), g.p(6.2, 12.3)], line);
            painter.circle_filled(g.p(13.2, 9.4), 1.1 * g.unit, colour);
            painter.circle_filled(g.p(15.4, 11.6), 1.1 * g.unit, colour);
        }
        Kind::Laptop => {
            painter.rect_stroke(g.r(3.5, 4.0, 16.5, 13.0), 1.2 * g.unit, line, egui::StrokeKind::Middle);
            painter.line_segment([g.p(1.2, 15.8), g.p(18.8, 15.8)], g.stroke(2.0, colour));
        }
        Kind::Other => {
            // A battery: the app's own subject.
            painter.rect_stroke(g.r(2.0, 6.0, 16.0, 14.0), 2.0 * g.unit, line, egui::StrokeKind::Middle);
            painter.rect_filled(g.r(16.8, 8.3, 18.4, 11.7), 0.6 * g.unit, colour);
            painter.rect_filled(g.r(4.4, 8.4, 9.6, 11.6), 0.8 * g.unit, colour);
        }
    }
}

/// The charging bolt.
pub fn bolt(painter: &Painter, rect: Rect, colour: Color32) {
    let g = Grid::new(rect);
    // Two overlapping triangles make the zig-zag.
    painter.add(Shape::convex_polygon(vec![g.p(12.5, 1.0), g.p(4.0, 11.5), g.p(11.0, 11.5)], colour, Stroke::NONE));
    painter.add(Shape::convex_polygon(vec![g.p(9.0, 8.5), g.p(16.0, 8.5), g.p(7.5, 19.0)], colour, Stroke::NONE));
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Glyph {
    Gear,
    Pin,
    Close,
    Refresh,
    Back,
    Lock,
}

pub fn glyph(painter: &Painter, rect: Rect, glyph: Glyph, colour: Color32) {
    let g = Grid::new(rect);
    let line = g.stroke(1.5, colour);
    match glyph {
        Glyph::Gear => {
            // A toothed outline: eight teeth, each a flat top between two
            // flanks, around a hole.
            let mut outline = Vec::new();
            for i in 0..8 {
                let a = i as f32 * PI / 4.0;
                for (radius, offset) in [(6.0, -0.30), (8.2, -0.17), (8.2, 0.17), (6.0, 0.30)] {
                    let (s, c) = (a + offset).sin_cos();
                    outline.push(g.p(10.0 + radius * c, 10.0 + radius * s));
                }
            }
            painter.add(Shape::closed_line(outline, line));
            painter.circle_stroke(g.p(10.0, 10.0), 2.7 * g.unit, line);
        }
        Glyph::Pin => {
            // A push pin, leaning like Windows' own.
            let at = |x: f32, y: f32| {
                // Rotate 45 degrees around the centre.
                let (dx, dy) = (x - 10.0, y - 10.0);
                let (s, c) = (PI / 4.0).sin_cos();
                g.p(10.0 + dx * c - dy * s, 10.0 + dx * s + dy * c)
            };
            let head = [at(7.0, 2.5), at(13.0, 2.5), at(13.0, 4.5), at(12.0, 5.0), at(12.0, 9.5), at(14.5, 12.0), at(5.5, 12.0), at(8.0, 9.5), at(8.0, 5.0), at(7.0, 4.5)];
            painter.add(Shape::closed_line(head.to_vec(), line));
            painter.line_segment([at(10.0, 12.0), at(10.0, 18.0)], line);
        }
        Glyph::Close => {
            painter.line_segment([g.p(5.0, 5.0), g.p(15.0, 15.0)], line);
            painter.line_segment([g.p(15.0, 5.0), g.p(5.0, 15.0)], line);
        }
        Glyph::Refresh => {
            painter.add(Shape::line(g.arc(10.0, 10.0, 6.0, -PI * 0.35, PI * 1.45), line));
            let tip = g.p(10.0 + 6.0 * (-PI * 0.35).cos(), 10.0 + 6.0 * (-PI * 0.35).sin());
            painter.add(Shape::convex_polygon(
                vec![tip + vec2(2.6, -2.6) * g.unit, tip + vec2(0.9, 3.0) * g.unit, tip + vec2(-2.8, -0.6) * g.unit],
                colour,
                Stroke::NONE,
            ));
        }
        Glyph::Back => {
            painter.add(Shape::line(vec![g.p(12.5, 4.0), g.p(6.5, 10.0), g.p(12.5, 16.0)], line));
        }
        Glyph::Lock => {
            painter.add(Shape::line(g.arc(10.0, 8.5, 3.5, PI, 2.0 * PI), line));
            painter.line_segment([g.p(6.5, 8.5), g.p(6.5, 10.0)], line);
            painter.line_segment([g.p(13.5, 8.5), g.p(13.5, 10.0)], line);
            painter.rect_filled(g.r(4.5, 9.5, 15.5, 17.5), 1.8 * g.unit, colour);
        }
    }
}

/// A flat, round-cornered button showing a glyph, with a hover fill.
pub fn button(ui: &mut egui::Ui, which: Glyph, colour: Color32, hover: Color32, tip: &str) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(30.0, 30.0), egui::Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(rect, 6, hover);
    }
    glyph(ui.painter(), rect.shrink(7.0), which, colour);
    response.on_hover_text(tip)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_grid_fits_the_rect_and_stays_square() {
        let g = Grid::new(Rect::from_min_size(pos2(10.0, 0.0), vec2(40.0, 20.0)));
        assert_eq!(g.p(0.0, 0.0), pos2(20.0, 0.0));
        assert_eq!(g.p(20.0, 20.0), pos2(40.0, 20.0));
        let arc = g.arc(10.0, 10.0, 10.0, 0.0, PI);
        assert!((arc[0].x - 40.0).abs() < 1e-3 && (arc.last().unwrap().x - 20.0).abs() < 1e-3);
    }
}
