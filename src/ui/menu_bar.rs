//! 상단 메뉴바 UI (`파일 · 편집 · 검색 · 보기 · 인코딩 · 창 · 도움말`).
//!
//! P1-01에서는 빈 앱 창의 메뉴바 자리를 표시하고, P1-03에서 `Command` 발행과 연결한다.

use eframe::egui;

/// 상단 메뉴바 자리 표시를 렌더링한다.
pub fn show_placeholder(ui: &mut egui::Ui) {
    egui::MenuBar::new().ui(ui, |ui| {
        for title in ["파일", "편집", "검색", "보기", "인코딩", "창", "도움말"] {
            ui.menu_button(title, |ui| {
                ui.weak("P1-03에서 메뉴 항목을 연결합니다.");
            });
        }
    });
}
