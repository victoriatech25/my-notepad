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
    /// 텍스트 선택 및 선택된 메뉴/버튼 강조 배경 (`selection_bg`)
    pub selection_bg: Color32,
    /// 텍스트 선택 및 선택된 메뉴/버튼 글자 스트로크 (`selection_stroke`)
    pub selection_stroke: Color32,
    /// 메뉴에서 선택된 항목 왼쪽의 점 `•` (`menu_selected_marker`)
    pub menu_selected_marker: Color32,
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
            // 기본 egui의 쨍한 청록(#005C80) 대신 톤을 낮춘 차분한 슬레이트 네이비
            selection_bg: Color32::from_rgb(0x26, 0x46, 0x69),
            selection_stroke: Color32::from_rgb(0xE0, 0xEC, 0xF8),
            menu_selected_marker: Color32::from_rgb(0x4C, 0xC3, 0x8A),
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
            // 기본 egui의 쨍한 하늘색(#90D1FF) 대신 톤을 낮춘 부드러운 소프트 블루
            selection_bg: Color32::from_rgb(0xCE, 0xDF, 0xF2),
            selection_stroke: Color32::from_rgb(0x1A, 0x36, 0x5D),
            menu_selected_marker: Color32::from_rgb(0x1E, 0x8E, 0x3E),
        }
    }
}

/// 다크 모드용 커스텀 `egui::Visuals`를 반환한다 (강조색 톤 다운).
pub fn dark_visuals() -> Visuals {
    let mut visuals = Visuals::dark();
    let palette = Palette::dark();
    visuals.selection = eframe::egui::style::Selection {
        bg_fill: palette.selection_bg,
        stroke: eframe::egui::Stroke::new(1.0, palette.selection_stroke),
    };
    visuals
}

/// 라이트 모드용 커스텀 `egui::Visuals`를 반환한다 (강조색 톤 다운).
pub fn light_visuals() -> Visuals {
    let mut visuals = Visuals::light();
    let palette = Palette::light();
    visuals.selection = eframe::egui::style::Selection {
        bg_fill: palette.selection_bg,
        stroke: eframe::egui::Stroke::new(1.0, palette.selection_stroke),
    };
    visuals
}

/// egui Context에 My-Notepad 커스텀 테마 Visuals를 등록한다.
pub fn configure_visuals(ctx: &eframe::egui::Context) {
    ctx.set_visuals_of(eframe::egui::Theme::Dark, dark_visuals());
    ctx.set_visuals_of(eframe::egui::Theme::Light, light_visuals());
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
        assert_ne!(dark.selection_bg, light.selection_bg);
    }

    #[test]
    fn custom_visuals_apply_toned_down_selection() {
        let default_dark = Visuals::dark();
        let custom_dark = dark_visuals();
        // 기본 egui 쨍한 청록색(0, 92, 128)과 다른 차분한 색상이 적용되었는지 확인
        assert_ne!(
            custom_dark.selection.bg_fill,
            default_dark.selection.bg_fill
        );
        assert_eq!(custom_dark.selection.bg_fill, Palette::dark().selection_bg);

        let default_light = Visuals::light();
        let custom_light = light_visuals();
        assert_ne!(
            custom_light.selection.bg_fill,
            default_light.selection.bg_fill
        );
        assert_eq!(
            custom_light.selection.bg_fill,
            Palette::light().selection_bg
        );
    }
}
