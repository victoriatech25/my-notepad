//! 제품명·앱 아이콘 (PLAN.md §5 Phase 5, P5-01).
//!
//! 사용자에게 보이는 이름은 "Vic Notepad"이고, 크레이트·실행 파일·설정 폴더 이름은 `my-notepad`/`My-Notepad`를 유지한다.
//! 아이콘 원본은 `assets/icons/vic-notepad.svg`, PNG·`.ico`·`.icns`는 `cargo xtask icons`로 만든다.

use eframe::egui;

/// 창 제목·메뉴 등 사용자에게 보이는 제품명
pub const APP_NAME: &str = "Vic Notepad";

/// 실행 중 창 아이콘 (Windows 작업 표시줄·Alt+Tab, macOS Dock)
const WINDOW_ICON_PNG: &[u8] = include_bytes!("../assets/icons/vic-notepad-512.png");

/// 창 아이콘을 디코딩한다. 실패하면 로그를 남기고 기본 아이콘으로 둔다
pub fn window_icon() -> Option<egui::IconData> {
    match image::load_from_memory_with_format(WINDOW_ICON_PNG, image::ImageFormat::Png) {
        Ok(img) => {
            let rgba = img.into_rgba8();
            let (width, height) = rgba.dimensions();
            Some(egui::IconData {
                rgba: rgba.into_raw(),
                width,
                height,
            })
        }
        Err(e) => {
            log::warn!("창 아이콘 디코딩 실패: {e}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_icon_decodes_to_512_square() {
        let icon = window_icon().expect("내장 아이콘 PNG");
        assert_eq!((icon.width, icon.height), (512, 512));
        assert_eq!(icon.rgba.len(), 512 * 512 * 4);
        // 모서리는 투명, 가운데(메모지)는 불투명
        assert_eq!(icon.rgba[3], 0);
        let center = (256 * 512 + 200) * 4;
        assert_eq!(icon.rgba[center + 3], 255);
    }
}
