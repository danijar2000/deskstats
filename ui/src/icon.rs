//! The icon of deskstats, drawn here from the same shapes as server/icon/icon.svg, at any size.

const LINE: [(f32, f32); 6] = [(212.0, 660.0), (346.0, 488.0), (461.0, 584.0), (614.0, 335.0), (710.0, 488.0), (812.0, 420.0)];
pub const TEAL: [u8; 3] = [0x2d, 0xd4, 0xbf];

fn distance_to_segment(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let along = (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
    ((p.0 - a.0 - along * dx).powi(2) + (p.1 - a.1 - along * dy).powi(2)).sqrt()
}

fn distance_to_line(p: (f32, f32)) -> f32 {
    LINE.windows(2).map(|pair| distance_to_segment(p, pair[0], pair[1])).fold(f32::MAX, f32::min)
}

/// How far [p] is outside the rounded square of the icon; negative inside it.
fn distance_to_square(p: (f32, f32)) -> f32 {
    let (half, radius) = (412.0, 185.0);
    let (x, y) = ((p.0 - 512.0).abs() - (half - radius), (p.1 - 512.0).abs() - (half - radius));
    (x.max(0.0).powi(2) + y.max(0.0).powi(2)).sqrt() + x.max(y).min(0.0) - radius
}

/// The whole icon, the dark square with the line on it, as RGBA.
pub fn app(size: u32) -> Vec<u8> {
    let scale = 1024.0 / size as f32;
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let p = ((x as f32 + 0.5) * scale, (y as f32 + 0.5) * scale);
            let inside = (0.5 - distance_to_square(p) / scale).clamp(0.0, 1.0);
            let line = (0.5 + (28.0 - distance_to_line(p)) / scale).clamp(0.0, 1.0);
            let shade = 0x1a as f32 - (0x1a - 0x0b) as f32 * (p.1 - 100.0).clamp(0.0, 824.0) / 824.0;
            let mix = |back: f32, front: u8| (back + (front as f32 - back) * line) as u8;
            out.extend_from_slice(&[mix(shade, TEAL[0]), mix(shade + 3.0, TEAL[1]), mix(shade + 10.0, TEAL[2]), (inside * 255.0) as u8]);
        }
    }
    out
}

/// Only the line, large and thick enough for the few pixels by the clock, in [colour] on nothing, as RGBA.
pub fn line(size: u32, colour: [u8; 3]) -> Vec<u8> {
    // The line takes x 212…812 and y 335…660 of the drawing; this window around it is square.
    let (left, top, side) = (152.0, 137.5, 720.0);
    let scale = side / size as f32;
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let p = (left + (x as f32 + 0.5) * scale, top + (y as f32 + 0.5) * scale);
            let alpha = (0.5 + (48.0 - distance_to_line(p)) / scale).clamp(0.0, 1.0);
            out.extend_from_slice(&[colour[0], colour[1], colour[2], (alpha * 255.0) as u8]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_icons_have_the_line_and_clear_corners() {
        let icon = app(64);
        assert_eq!(icon.len(), 64 * 64 * 4);
        assert_eq!(icon[3], 0, "the corner is outside the rounded square");
        let middle = (32 * 64 + 32) * 4;
        assert_eq!(icon[middle + 3], 255);
        let tray = line(32, [0, 0, 0]);
        assert_eq!(tray[3], 0);
        assert!(tray.chunks(4).any(|pixel| pixel[3] == 255));
    }
}
