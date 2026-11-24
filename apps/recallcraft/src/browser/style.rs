//! The window's look: the system's light or dark theme, a little more room, and a colour for each
//! person.

use egui::Color32;
use recallcraft_model::Speaker;

pub const INDIGO: Color32 = Color32::from_rgb(0x3B, 0x4C, 0xCA);
pub const RED: Color32 = Color32::from_rgb(0xE5, 0x39, 0x35);
pub const AMBER: Color32 = Color32::from_rgb(0xE0, 0x9A, 0x1B);
pub const TEAL: Color32 = Color32::from_rgb(0x0E, 0x9C, 0x8B);

const VOICES: [Color32; 8] = [
    Color32::from_rgb(0x0E, 0x9C, 0x8B),
    Color32::from_rgb(0xD9, 0x6C, 0x1E),
    Color32::from_rgb(0x9C, 0x3B, 0xC9),
    Color32::from_rgb(0x2E, 0x8B, 0x3E),
    Color32::from_rgb(0xC2, 0x3B, 0x6E),
    Color32::from_rgb(0x1F, 0x7A, 0xC4),
    Color32::from_rgb(0x8A, 0x6D, 0x1B),
    Color32::from_rgb(0x5D, 0x6B, 0x7A),
];

/// A person's colour.
pub fn speaker(s: Speaker, dark: bool) -> Color32 {
    let c = match s {
        Speaker::Me => {
            if dark {
                Color32::from_rgb(0x8E, 0x9B, 0xF5)
            } else {
                INDIGO
            }
        }
        Speaker::Others => TEAL,
        Speaker::Voice(n) => VOICES.get(usize::from(n.saturating_sub(1)) % VOICES.len()).copied().unwrap_or(TEAL),
    };
    if dark && !matches!(s, Speaker::Me) { c.gamma_multiply(1.25) } else { c }
}

pub fn setup(ctx: &egui::Context) {
    ctx.all_styles_mut(|s| {
        s.spacing.item_spacing = egui::vec2(8.0, 6.0);
        s.spacing.button_padding = egui::vec2(8.0, 4.0);
    });
}
