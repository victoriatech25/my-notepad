//! 하단 상태 표시줄 UI (`Ln/Col`, 선택 글자 수, 줄 수, 확대율, 줄바꿈, 인코딩).
//!
//! - P1-02: 시스템 폰트 로드 실패 시 경고를 표시
//! - P1-03: 실행된 커맨드·연속 입력 대기 상태 및 현재 확대율·줄바꿈·인코딩 표시
//! - P1-12: 팝업 및 상세 정보를 구현

use eframe::egui::{self, RichText};

use crate::commands::{EncodingChoice, LineEndingChoice};
use crate::ui::theme::Palette;

/// 상태 표시줄에 표시할 값 묶음.
#[derive(Debug, Clone, Copy)]
pub struct StatusBarInfo<'a> {
    pub line_count: usize,
    pub zoom: f32,
    pub encoding: EncodingChoice,
    pub line_ending: LineEndingChoice,
    pub font_warning: Option<&'a str>,
    pub chord_hint: Option<&'a str>,
    pub last_command: Option<&'a str>,
}

/// 하단 상태 표시줄을 렌더링한다.
pub fn show(ui: &mut egui::Ui, palette: &Palette, info: &StatusBarInfo<'_>) {
    ui.horizontal(|ui| {
        ui.label("Ln 1, Col 1");
        ui.separator();
        ui.label(format!("{}줄", info.line_count));

        if let Some(warning) = info.font_warning {
            ui.separator();
            ui.label(RichText::new(warning).color(palette.status_warning));
        }

        if let Some(chord) = info.chord_hint {
            ui.separator();
            ui.label(RichText::new(chord).color(palette.modified_marker).strong());
        } else if let Some(cmd) = info.last_command {
            ui.separator();
            ui.label(RichText::new(format!("실행됨: {cmd}")).weak());
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(info.encoding.label());
            ui.separator();
            ui.label(info.line_ending.short_label());
            ui.separator();
            ui.label(format!("{}%", (info.zoom * 100.0).round() as i32));
        });
    });
}
