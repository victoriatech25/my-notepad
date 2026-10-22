//! 색상 토큰 팔레트 (`docs/ux/ui-spec.md` §2 확정 v1).
//!
//! 라이트/다크 테마에 맞춰 에디터 하이라이트, 검색 매치, 줄 번호 Gutter,
//! 탭 수정 표시, 상태 표시줄 경고/오류, 미리보기 코드 배경 등의 색상을 제공한다.

use eframe::egui::{Color32, Visuals};

/// `docs/ux/ui-spec.md` §2에 정의된 UI 색상 토큰 묶음.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Palette {
    /// 선택 단어 일괄 강조 (`smart_highlight`)
    pub smart_highlight: Color32,
    /// 찾기 매치 전체 (`search_match`)
    pub search_match: Color32,
    /// 현재 매치 (`search_current`)
    pub search_current: Color32,
    /// 줄 번호 기본 색 (`gutter_text`)
    pub gutter_text: Color32,
    /// 현재 줄 번호 강조 색 (`gutter_current`)
    pub gutter_current: Color32,
    /// 탭 수정 표시 `●` (`modified_marker`)
    pub modified_marker: Color32,
    /// 추정됨·Mixed 등 상태 표시줄 경고 (`status_warning`)
    pub status_warning: Color32,
    /// 디코딩 손상 등 상태 표시줄 오류 (`status_error`)
    pub status_error: Color32,
    /// Markdown 미리보기 코드 블록 배경
    pub preview_code_bg: Color32,
    /// 분할 모드에서 현재 커서 위치 블록 강조 배경
    pub cursor_block: Color32,
    /// 대화상자 제목 띠 배경 (`dialog_title_bg`)
    pub dialog_title_bg: Color32,
    /// 대화상자 버튼 띠 배경 (`dialog_footer_bg`)
    pub dialog_footer_bg: Color32,
}

impl Palette {
    /// 현재 `egui::Visuals`의 다크/라이트 모드에 맞는 팔레트를 반환한다.
    pub fn from_visuals(visuals: &Visuals) -> Self {
        if visuals.dark_mode {
            Self::dark()
        } else {
            Self::light()
        }
    }

    /// 다크 테마 색상 토큰 (`docs/ux/ui-spec.md` §2).
    pub fn dark() -> Self {
        Self {
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
            dialog_title_bg: Color32::from_rgb(0x1B, 0x22, 0x33),
            dialog_footer_bg: Color32::from_gray(20),
        }
    }

    /// 라이트 테마 색상 토큰 (`docs/ux/ui-spec.md` §2).
    pub fn light() -> Self {
        Self {
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
            dialog_title_bg: Color32::from_rgb(0xE3, 0xEA, 0xF5),
            dialog_footer_bg: Color32::from_gray(236),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_matches_visuals_mode() {
        let dark = Palette::from_visuals(&Visuals::dark());
        let light = Palette::from_visuals(&Visuals::light());
        assert_eq!(dark, Palette::dark());
        assert_eq!(light, Palette::light());
        assert_ne!(dark.smart_highlight, light.smart_highlight);
        assert_ne!(dark.search_match, light.search_match);
    }
}
