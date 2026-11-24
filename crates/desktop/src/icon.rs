//! RecallCraft's icon, drawn in code: a round arrow going back (replay) round three bars of
//! sound, white on an indigo tile. With a red dot at its corner while a call is recorded, and
//! greyed while recording is paused.

/// How the icon looks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Look {
    Normal,
    /// A call is being recorded.
    Call,
    /// Recording is paused.
    Paused,
}

const INDIGO: [f32; 3] = [0x3B as f32 / 255.0, 0x4C as f32 / 255.0, 0xCA as f32 / 255.0];
const GREY: [f32; 3] = [0x80 as f32 / 255.0, 0x86 as f32 / 255.0, 0x90 as f32 / 255.0];
const RED: [f32; 3] = [0xE5 as f32 / 255.0, 0x39 as f32 / 255.0, 0x35 as f32 / 255.0];

fn round_rect(x: f32, y: f32, cx: f32, cy: f32, hw: f32, hh: f32, r: f32) -> f32 {
    let (dx, dy) = ((x - cx).abs() - (hw - r), (y - cy).abs() - (hh - r));
    dx.max(dy).min(0.0) + (dx.max(0.0).powi(2) + dy.max(0.0).powi(2)).sqrt() - r
}

/// Whether a point is on the round arrow: a ring open at the top left, with its head there.
fn arrow(x: f32, y: f32) -> bool {
    let (dx, dy) = (x - 0.5, y - 0.5);
    let r = (dx * dx + dy * dy).sqrt();
    // The angle, clockwise from straight up, 0 to 360.
    let mut a = dx.atan2(-dy).to_degrees();
    if a < 0.0 {
        a += 360.0;
    }
    let ring = (0.255..=0.335).contains(&r) && !(285.0..=345.0).contains(&a);
    // The head: a triangle at the ring's end (285°), pointing round, anticlockwise.
    let end = 285f32.to_radians();
    let (ex, ey) = (0.5 + 0.295 * end.sin(), 0.5 - 0.295 * end.cos());
    // Along the ring (tangent, going anticlockwise) and across it.
    let (tx, ty) = (-end.cos(), -end.sin());
    let (nx, ny) = (end.sin(), -end.cos());
    let (px, py) = (x - ex, y - ey);
    let along = px * tx + py * ty;
    let across = px * nx + py * ny;
    let head = (0.0..=0.13).contains(&along) && across.abs() <= 0.11 * (1.0 - along / 0.13);
    ring || head
}

/// Whether a point is on one of the three bars of sound in the middle.
fn bars(x: f32, y: f32) -> bool {
    [(0.40, 0.085), (0.50, 0.15), (0.60, 0.10)].iter().any(|&(cx, hh)| round_rect(x, y, cx, 0.5, 0.032, hh, 0.032) <= 0.0)
}

fn sample(x: f32, y: f32, look: Look) -> [f32; 4] {
    if round_rect(x, y, 0.5, 0.5, 0.5, 0.5, 0.22) > 0.0 {
        return [0.0; 4];
    }
    if look == Look::Call && ((x - 0.80).powi(2) + (y - 0.80).powi(2)).sqrt() <= 0.17 {
        return [RED[0], RED[1], RED[2], 1.0];
    }
    if arrow(x, y) || bars(x, y) {
        return [1.0, 1.0, 1.0, 1.0];
    }
    let c = if look == Look::Paused { GREY } else { INDIGO };
    [c[0], c[1], c[2], 1.0]
}

/// The icon as `size`×`size` RGBA pixels (4×4 samples a pixel).
pub fn pixels(size: u32, look: Look) -> Vec<u8> {
    const SS: u32 = 4;
    let size = size.clamp(1, 512);
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    for py in 0..size {
        for px in 0..size {
            let mut acc = [0.0f32; 4];
            for sy in 0..SS {
                for sx in 0..SS {
                    let x = (px as f32 + (sx as f32 + 0.5) / SS as f32) / size as f32;
                    let y = (py as f32 + (sy as f32 + 0.5) / SS as f32) / size as f32;
                    let s = sample(x, y, look);
                    for c in 0..3 {
                        acc[c] += s[c] * s[3];
                    }
                    acc[3] += s[3];
                }
            }
            let n = (SS * SS) as f32;
            let a = acc[3] / n;
            for c in acc.iter().take(3) {
                out.push(if a > 0.0 { ((c / n / a) * 255.0).round().clamp(0.0, 255.0) as u8 } else { 0 });
            }
            out.push((a * 255.0).round().clamp(0.0, 255.0) as u8);
        }
    }
    out
}

/// An `.ico` file of the icon at the usual sizes (32-bit pictures, no compression).
pub fn ico() -> Vec<u8> {
    let sizes = [16u32, 20, 24, 32, 40, 48, 64, 128, 256];
    let mut images: Vec<Vec<u8>> = Vec::new();
    for &s in &sizes {
        let px = pixels(s, Look::Normal);
        let mut img = Vec::new();
        // BITMAPINFOHEADER: twice the height (the picture and its mask).
        img.extend_from_slice(&40u32.to_le_bytes());
        img.extend_from_slice(&(s as i32).to_le_bytes());
        img.extend_from_slice(&((s * 2) as i32).to_le_bytes());
        img.extend_from_slice(&1u16.to_le_bytes());
        img.extend_from_slice(&32u16.to_le_bytes());
        img.extend_from_slice(&[0u8; 24]);
        // Bottom row first, blue green red alpha.
        for row in (0..s).rev() {
            for col in 0..s {
                let i = ((row * s + col) * 4) as usize;
                let p = px.get(i..i + 4).unwrap_or(&[0, 0, 0, 0]);
                img.extend_from_slice(&[p[2], p[1], p[0], p[3]]);
            }
        }
        // The mask: all shown (the alpha says what's clear), rows padded to 4 bytes.
        let mask_row = (s.div_ceil(32) * 4) as usize;
        img.extend(std::iter::repeat_n(0u8, mask_row * s as usize));
        images.push(img);
    }
    let mut out = Vec::new();
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&(sizes.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * sizes.len() as u32;
    for (s, img) in sizes.iter().zip(&images) {
        let b = if *s >= 256 { 0u8 } else { *s as u8 };
        out.extend_from_slice(&[b, b, 0, 0]);
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&32u16.to_le_bytes());
        out.extend_from_slice(&(img.len() as u32).to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        offset += img.len() as u32;
    }
    for img in images {
        out.extend(img);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_icon_has_its_parts() {
        let n = pixels(64, Look::Normal);
        assert_eq!(n.len(), 64 * 64 * 4);
        assert_eq!(n[3], 0, "the corner outside the tile is clear");
        let at = |p: &[u8], x: usize, y: usize| p[(y * 64 + x) * 4..(y * 64 + x) * 4 + 4].to_vec();
        assert_eq!(at(&n, 32, 32), vec![255, 255, 255, 255], "the middle bar is white");
        assert_eq!(at(&n, 32, 12)[..3], [255, 255, 255], "the ring at the top");
        let bg = at(&n, 10, 32);
        assert!(bg[2] > bg[0] && bg[3] == 255, "indigo between: {bg:?}");
        let c = pixels(64, Look::Call);
        let dot = at(&c, 52, 52);
        assert!(dot[0] > 200 && dot[1] < 100, "a red dot: {dot:?}");
        let p = pixels(64, Look::Paused);
        let g = at(&p, 10, 32);
        assert!((g[0] as i32 - g[2] as i32).abs() < 20, "grey: {g:?}");
        let file = ico();
        assert_eq!(&file[..4], &[0, 0, 1, 0]);
        assert_eq!(u16::from_le_bytes([file[4], file[5]]), 9);
        assert!(file.len() > 256 * 256 * 4);
    }
}
