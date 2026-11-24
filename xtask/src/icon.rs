//! `cargo xtask icon`: RecallCraft's icon in every size and format the packages use, written into
//! `assets/app-icon/`. The drawing is `recallcraft_desktop::icon` (original, drawn in code: a round
//! arrow going back round three bars of sound, white on an indigo tile); nothing here draws.
//!
//! - `recallcraft.ico`: Windows (the program's own resources, the installer, its shortcut), as
//!   `icon::ico()` makes it.
//! - `recallcraft.icns`: macOS (the app bundle), drawn on Apple's grid (an 824 px body on a
//!   1024 px canvas, so a margin around the tile). The drawing goes up to 512 px, so that's the
//!   biggest entry.
//! - `hicolor/<n>x<n>/apps/io.github.billsecond.recallcraft.png`, 16 to 512 px: Linux and FreeBSD
//!   (the desktop entries), the Flatpak and the AppImage.
//!
//! Every file is listed in ATTRIBUTION.md; `packaging/check-icons.sh` checks them.

use std::path::Path;

use recallcraft_desktop::icon::{self, Look};

/// The name the Linux icon files carry (the desktop entries' `Icon=`).
pub const APP_ID: &str = "io.github.billsecond.recallcraft";
/// The hicolor theme's sizes.
pub const HICOLOR: [u32; 8] = [16, 24, 32, 48, 64, 128, 256, 512];
/// macOS icons sit on Apple's grid: an 824 px body on a 1024 px canvas.
const MAC_BODY: f32 = 824.0 / 1024.0;

/// The icon as `size`×`size` RGBA8 pixels filling the canvas.
fn plain(size: u32) -> Result<Vec<u8>, String> {
    let px = icon::pixels(size, Look::Normal);
    if px.len() != (size * size * 4) as usize {
        return Err(format!("the icon can't be drawn at {size} px"));
    }
    Ok(px)
}

/// The icon as `size`×`size` RGBA8 pixels, on Apple's grid (see-through around it).
fn mac(size: u32) -> Result<Vec<u8>, String> {
    let body = ((size as f32 * MAC_BODY).round() as u32).clamp(1, size);
    let inner = plain(body)?;
    let off = (size - body) / 2;
    let mut out = vec![0u8; (size * size * 4) as usize];
    for y in 0..body {
        let src = (y * body * 4) as usize;
        let dst = (((y + off) * size + off) * 4) as usize;
        let row = (body * 4) as usize;
        match (inner.get(src..src + row), out.get_mut(dst..dst + row)) {
            (Some(s), Some(d)) => d.copy_from_slice(s),
            _ => return Err(format!("the {size} px macOS icon doesn't fit")),
        }
    }
    Ok(out)
}

/// `size`×`size` RGBA8 pixels as a PNG file.
fn png(size: u32, rgba: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, size, size);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.set_compression(png::Compression::High);
    let mut w = enc.write_header().map_err(|e| format!("encoding the {size} px icon: {e}"))?;
    w.write_image_data(rgba).map_err(|e| format!("encoding the {size} px icon: {e}"))?;
    w.finish().map_err(|e| format!("encoding the {size} px icon: {e}"))?;
    Ok(out)
}

/// An Apple `.icns` holding PNG entries.
fn icns(entries: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
    let total: usize = 8 + entries.iter().map(|(_, d)| 8 + d.len()).sum::<usize>();
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"icns");
    out.extend_from_slice(&(total as u32).to_be_bytes());
    for (kind, data) in entries {
        out.extend_from_slice(*kind);
        out.extend_from_slice(&((8 + data.len()) as u32).to_be_bytes());
        out.extend_from_slice(data);
    }
    out
}

/// The macOS icon: the sizes Finder and the Dock ask for, up to 512 px (and 256 px at 2x).
fn mac_icns() -> Result<Vec<u8>, String> {
    let at = |size: u32| -> Result<Vec<u8>, String> { png(size, &mac(size)?) };
    let entries: Vec<(&[u8; 4], Vec<u8>)> = vec![
        (b"icp4", at(16)?),
        (b"icp5", at(32)?),
        (b"icp6", at(64)?),
        (b"ic07", at(128)?),
        (b"ic08", at(256)?),
        (b"ic09", at(512)?),
        (b"ic11", at(32)?),
        (b"ic12", at(64)?),
        (b"ic13", at(256)?),
        (b"ic14", at(512)?),
    ];
    Ok(icns(&entries))
}

/// Every file, by its name under `assets/app-icon`.
pub fn files() -> Result<Vec<(String, Vec<u8>)>, String> {
    let mut out = vec![("recallcraft.ico".to_string(), icon::ico()), ("recallcraft.icns".to_string(), mac_icns()?)];
    for size in HICOLOR {
        out.push((format!("hicolor/{size}x{size}/apps/{APP_ID}.png"), png(size, &plain(size)?)?));
    }
    Ok(out)
}

pub fn run(root: &Path) -> Result<(), String> {
    let dir = root.join("assets").join("app-icon");
    for (rel, bytes) in files()? {
        let path = dir.join(&rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        std::fs::write(&path, &bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        println!("wrote {} ({} bytes)", path.display(), bytes.len());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Width and height from a PNG's IHDR chunk.
    fn png_size(png: &[u8]) -> Option<(u32, u32)> {
        if png.get(..8)? != b"\x89PNG\r\n\x1a\n" || png.get(12..16)? != b"IHDR" {
            return None;
        }
        let w = u32::from_be_bytes(png.get(16..20)?.try_into().ok()?);
        let h = u32::from_be_bytes(png.get(20..24)?.try_into().ok()?);
        Some((w, h))
    }

    fn alpha(px: &[u8], size: u32, x: u32, y: u32) -> u8 {
        px[((y * size + x) * 4 + 3) as usize]
    }

    #[test]
    fn every_file_is_made_at_its_size() {
        let files = files().unwrap();
        assert_eq!(files.len(), 2 + HICOLOR.len());
        for size in HICOLOR {
            let name = format!("hicolor/{size}x{size}/apps/{APP_ID}.png");
            let (_, data) = files.iter().find(|(n, _)| *n == name).unwrap();
            assert_eq!(png_size(data), Some((size, size)), "{name}");
        }
        let (_, ico) = files.iter().find(|(n, _)| n == "recallcraft.ico").unwrap();
        assert_eq!(&ico[..4], &[0, 0, 1, 0]);
        let (_, icns) = files.iter().find(|(n, _)| n == "recallcraft.icns").unwrap();
        assert_eq!(&icns[..4], b"icns");
        assert_eq!(u32::from_be_bytes([icns[4], icns[5], icns[6], icns[7]]) as usize, icns.len());
    }

    #[test]
    fn the_macos_icon_keeps_its_margin() {
        let px = mac(64).unwrap();
        assert_eq!(px.len(), 64 * 64 * 4);
        assert_eq!(alpha(&px, 64, 2, 32), 0, "the margin is see-through");
        assert_eq!(alpha(&px, 64, 32, 32), 255, "the tile is there");
        let full = plain(64).unwrap();
        assert_eq!(alpha(&full, 64, 2, 32), 255, "without the grid the tile reaches the edge");
    }

    #[test]
    fn not_a_png_has_no_size() {
        assert_eq!(png_size(b"GIF89a"), None);
        assert_eq!(png_size(b""), None);
        assert_eq!(png_size(&png(3, &[255; 36]).unwrap()), Some((3, 3)));
    }
}
