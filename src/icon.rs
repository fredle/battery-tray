//! The tray icon: a rounded tile with the battery percentage drawn into it.
//! Rendered here as plain RGBA so it can be tested; `tray.rs` hands it to
//! Windows.

/// Width and height in pixels. Windows scales it to the tray's size.
pub const SIZE: usize = 32;

const NORMAL: [u8; 3] = [47, 107, 255];
const LOW: [u8; 3] = [207, 58, 58];
const UNKNOWN: [u8; 3] = [110, 116, 128];
const CORNER: i32 = 7;

/// 5x7 digits, one row per byte, most significant of the five bits leftmost.
const DIGITS: [[u8; 7]; 10] = [
    [0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110],
    [0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110],
    [0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111],
    [0b11110, 0b00001, 0b00001, 0b01110, 0b00001, 0b00001, 0b11110],
    [0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010],
    [0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110],
    [0b00110, 0b01000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110],
    [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000],
    [0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110],
    [0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00010, 0b01100],
];
/// A dash, for "no level".
const DASH: [u8; 7] = [0, 0, 0, 0b11111, 0, 0, 0];

/// Each glyph cell is drawn this many pixels wide and tall.
const SCALE_X: usize = 2;
const SCALE_Y: usize = 3;
const GAP: usize = 2;

/// The columns of a glyph that have any ink, so "1" takes less room.
fn ink_columns(glyph: &[u8; 7]) -> (usize, usize) {
    let all = glyph.iter().fold(0u8, |acc, row| acc | row);
    let first = (0..5).find(|c| all & (0b10000 >> c) != 0).unwrap_or(0);
    let last = (0..5).rev().find(|c| all & (0b10000 >> c) != 0).unwrap_or(4);
    (first, last)
}

fn inside_tile(x: i32, y: i32) -> bool {
    let max = SIZE as i32 - 1;
    // Distance into the nearest corner's square, if in one.
    let dx = (CORNER - x).max(x - (max - CORNER)).max(0);
    let dy = (CORNER - y).max(y - (max - CORNER)).max(0);
    dx * dx + dy * dy <= CORNER * CORNER
}

/// RGBA pixels, `SIZE` x `SIZE`, top row first. `level` is the percentage to
/// draw, or `None` for a dash; `low` turns the tile red.
pub fn render(level: Option<u8>, low: bool) -> Vec<u8> {
    let colour = match (level, low) {
        (None, _) => UNKNOWN,
        (Some(_), true) => LOW,
        (Some(_), false) => NORMAL,
    };
    let mut pixels = vec![0u8; SIZE * SIZE * 4];
    for y in 0..SIZE {
        for x in 0..SIZE {
            if inside_tile(x as i32, y as i32) {
                let at = (y * SIZE + x) * 4;
                pixels[at..at + 3].copy_from_slice(&colour);
                pixels[at + 3] = 255;
            }
        }
    }

    let glyphs: Vec<&[u8; 7]> = match level {
        Some(level) => level
            .min(100)
            .to_string()
            .bytes()
            .map(|digit| &DIGITS[(digit - b'0') as usize])
            .collect(),
        None => vec![&DASH],
    };
    let width: usize = glyphs
        .iter()
        .map(|g| {
            let (first, last) = ink_columns(g);
            (last - first + 1) * SCALE_X
        })
        .sum::<usize>()
        + GAP * (glyphs.len() - 1);
    let mut left = (SIZE - width) / 2;
    let top = (SIZE - 7 * SCALE_Y) / 2;
    for glyph in glyphs {
        let (first, last) = ink_columns(glyph);
        for (row, bits) in glyph.iter().enumerate() {
            for column in first..=last {
                if bits & (0b10000 >> column) == 0 {
                    continue;
                }
                for y in 0..SCALE_Y {
                    for x in 0..SCALE_X {
                        let px = left + (column - first) * SCALE_X + x;
                        let py = top + row * SCALE_Y + y;
                        let at = (py * SIZE + px) * 4;
                        pixels[at..at + 4].copy_from_slice(&[255, 255, 255, 255]);
                    }
                }
            }
        }
        left += (last - first + 1) * SCALE_X + GAP;
    }
    pixels
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel(pixels: &[u8], x: usize, y: usize) -> [u8; 4] {
        let at = (y * SIZE + x) * 4;
        [pixels[at], pixels[at + 1], pixels[at + 2], pixels[at + 3]]
    }

    fn white(pixels: &[u8]) -> usize {
        pixels.chunks(4).filter(|p| *p == [255, 255, 255, 255]).count()
    }

    #[test]
    fn has_rounded_transparent_corners_and_a_filled_tile() {
        let pixels = render(Some(50), false);
        assert_eq!(pixels.len(), SIZE * SIZE * 4);
        for (x, y) in [(0, 0), (SIZE - 1, 0), (0, SIZE - 1), (SIZE - 1, SIZE - 1)] {
            assert_eq!(pixel(&pixels, x, y)[3], 0, "{x},{y}");
        }
        assert_eq!(pixel(&pixels, SIZE / 2, 0), [47, 107, 255, 255]);
        assert_eq!(pixel(&pixels, 0, SIZE / 2), [47, 107, 255, 255]);
    }

    #[test]
    fn colour_says_low_or_unknown() {
        assert_eq!(pixel(&render(Some(10), true), 1, SIZE / 2), [207, 58, 58, 255]);
        assert_eq!(pixel(&render(None, false), 1, SIZE / 2), [110, 116, 128, 255]);
        assert_eq!(pixel(&render(None, true), 1, SIZE / 2), [110, 116, 128, 255]);
    }

    #[test]
    fn every_level_draws_distinct_digits_that_fit() {
        // Would panic on an out-of-range pixel if a number did not fit.
        let all: Vec<Vec<u8>> = (0..=100).map(|level| render(Some(level), false)).collect();
        for (level, pixels) in all.iter().enumerate() {
            assert!(white(pixels) > 20, "{level}");
            // One blank column at each edge, even for "100".
            for y in 0..SIZE {
                assert_ne!(pixel(pixels, 0, y), [255, 255, 255, 255], "{level}");
                assert_ne!(pixel(pixels, SIZE - 1, y), [255, 255, 255, 255], "{level}");
            }
        }
        for a in 0..all.len() {
            for b in a + 1..all.len() {
                assert_ne!(all[a], all[b], "{a} and {b} look the same");
            }
        }
        assert_eq!(render(Some(200), false), all[100]);
        // The dash is a single bar.
        assert_eq!(white(&render(None, false)), 5 * SCALE_X * SCALE_Y);
    }
}
