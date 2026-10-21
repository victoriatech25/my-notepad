//! 하단 상태 표시줄 UI (`Ln/Col`, 선택 글자 수, 줄 수, 확대율, 줄바꿈, 인코딩).
//!
//! P1-01에서는 기본 상태 표시줄 자리를 표시하고, P1-12에서 팝업 및 상세 정보를 구현한다.

use eframe::egui;

/// 하단 상태 표시줄 자리 표시를 렌더링한다.
pub fn show_placeholder(ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        ui.label("Ln 1, Col 1");
        ui.separator();
        ui.label("1줄");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label("UTF-8");
            ui.separator();
            ui.label(if cfg!(windows) { "CRLF" } else { "LF" });
            ui.separator();
            ui.label("100%");
        });
    });
}
