//! 색상 토큰 (docs/ux/ui-spec.md §2 초안).

use eframe::egui::{Color32, Visuals};

pub struct Palette {
    pub smart_highlight: Color32,
    pub search_match: Color32,
    pub search_current: Color32,
    pub gutter_text: Color32,
    pub gutter_current: Color32,
    pub modified_marker: Color32,
    pub status_warning: Color32,
    pub status_error: Color32,
    pub preview_code_bg: Color32,
    pub cursor_block: Color32,
}

pub fn palette(visuals: &Visuals) -> Palette {
    if visuals.dark_mode {
        Palette {
            smart_highlight: Color32::from_rgb(0x2E, 0x5A, 0x2E),
            search_match: Color32::from_rgb(0x6A, 0x5A, 0x1E),
            search_current: Color32::from_rgb(0xB8, 0x69, 0x1A),
            gutter_text: Color32::from_gray(110),
            gutter_current: Color32::from_gray(220),
            modified_marker: Color32::from_rgb(0x6C, 0xB4, 0xFF),
            status_warning: Color32::from_rgb(0xFF, 0xB0, 0x40),
            status_error: Color32::from_rgb(0xFF, 0x6B, 0x6B),
            preview_code_bg: Color32::from_gray(40),
            cursor_block: Color32::from_rgba_unmultiplied(0x6C, 0xB4, 0xFF, 24),
        }
    } else {
        Palette {
            smart_highlight: Color32::from_rgb(0xC8, 0xF0, 0xC8),
            search_match: Color32::from_rgb(0xFF, 0xF0, 0xA0),
            search_current: Color32::from_rgb(0xFF, 0xB3, 0x47),
            gutter_text: Color32::from_gray(150),
            gutter_current: Color32::from_gray(30),
            modified_marker: Color32::from_rgb(0x1E, 0x6F, 0xD9),
            status_warning: Color32::from_rgb(0xC0, 0x6A, 0x00),
            status_error: Color32::from_rgb(0xC6, 0x28, 0x28),
            preview_code_bg: Color32::from_gray(238),
            cursor_block: Color32::from_rgba_unmultiplied(0x1E, 0x6F, 0xD9, 20),
        }
    }
}
