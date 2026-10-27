//! 하단 상태 표시줄 UI (`Ln/Col`, 선택 글자 수, 줄 수, 확대율, 줄바꿈, 인코딩).
//!
//! - P1-02: 시스템 폰트 로드 실패 시 경고를 표시
//! - P1-03: 실행된 커맨드·연속 입력 대기 상태 및 현재 확대율·줄바꿈·인코딩 표시
//! - P1-09: 실제 커서 위치(`Ln/Col`, 문자 단위)와 선택 글자 수 표시
//! - P1-10: 열기·저장 결과 등 잠깐 보여 주는 안내 문구
//! - P1-12: 인코딩·줄바꿈을 눌러 여는 팝업(다시 읽기·변환·줄바꿈 변경), `추정됨`·`손상`·`Mixed` 표시
//!   (ui-spec: 좌 = 커서·선택·줄 수 / 우 = 확대율·줄바꿈·인코딩, "바꿀 수 있는 값" 묶음)
//! - P6-A: 문법 강조 언어 이름 (눌러서 자동·끄기·언어 선택)

use eframe::egui::{self, RichText};

use crate::commands::{Command, EncodingChoice, LineEndingChoice, SyntaxChoice};
use crate::ui::menu_bar;
use crate::ui::theme::Palette;

/// 상태 표시줄에 표시할 값 묶음.
#[derive(Debug, Clone, Copy)]
pub struct StatusBarInfo<'a> {
    /// 커서 줄 번호 (1-기반)
    pub line: usize,
    /// 커서 열 번호 (1-기반, 문자 단위 — 한글 1글자 = 1칸)
    pub column: usize,
    /// 선택된 글자 수 (0이면 표시하지 않음)
    pub selected_chars: usize,
    pub line_count: usize,
    pub zoom: f32,
    pub encoding: EncodingChoice,
    /// 자동 감지로 추정한 인코딩인지
    pub encoding_estimated: bool,
    /// 지원 목록 밖 추정 인코딩에서 대체했다면 원래 추정 이름
    pub encoding_fallback_from: Option<&'a str>,
    /// 디코딩 중 해석하지 못한 바이트가 있었는지 (`U+FFFD`)
    pub damaged: bool,
    pub line_ending: LineEndingChoice,
    pub mixed_line_endings: bool,
    /// 문법 강조 선택·지금 칠하는 이름·자동 판정 이름 (P6-A)
    pub syntax: SyntaxChoice,
    pub syntax_name: &'a str,
    pub syntax_auto_name: &'a str,
    pub font_warning: Option<&'a str>,
    pub chord_hint: Option<&'a str>,
    /// 잠깐 보여 주는 안내 (마지막 실행 커맨드보다 우선)
    pub notice: Option<&'a str>,
    pub last_command: Option<&'a str>,
}

/// 하단 상태 표시줄을 렌더링하고, 인코딩·줄바꿈 팝업에서 고른 [`Command`]를 돌려준다.
pub fn show(ui: &mut egui::Ui, palette: &Palette, info: &StatusBarInfo<'_>) -> Option<Command> {
    let mut triggered = None;
    ui.horizontal(|ui| {
        ui.label(format!("Ln {}, Col {}", info.line, info.column));
        if info.selected_chars > 0 {
            ui.separator();
            ui.label(format!("선택 {}자", info.selected_chars));
        }
        ui.separator();
        ui.label(format!("{}줄", info.line_count));

        if let Some(warning) = info.font_warning {
            ui.separator();
            ui.label(RichText::new(warning).color(palette.status_warning));
        }

        if let Some(chord) = info.chord_hint {
            ui.separator();
            ui.label(RichText::new(chord).color(palette.modified_marker).strong());
        } else if let Some(notice) = info.notice {
            ui.separator();
            ui.label(RichText::new(notice).color(palette.modified_marker));
        } else if let Some(cmd) = info.last_command {
            ui.separator();
            ui.label(RichText::new(format!("실행됨: {cmd}")).weak());
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let (enc_text, enc_tip) = encoding_label(info, palette);
            ui.menu_button(enc_text, |ui| {
                menu_bar::encoding_items(ui, info.encoding, &mut triggered);
            })
            .response
            .on_hover_text(enc_tip);
            ui.separator();

            let le_text = if info.mixed_line_endings {
                RichText::new(info.line_ending.status_label(true)).color(palette.status_warning)
            } else {
                RichText::new(info.line_ending.short_label())
            };
            let le_tip = if info.mixed_line_endings {
                "줄바꿈이 섞여 있습니다. 저장하면 괄호 안 방식으로 통일됩니다 — 클릭해서 변경"
            } else {
                "줄바꿈 방식 — 클릭해서 변경"
            };
            ui.menu_button(le_text, |ui| {
                menu_bar::line_ending_items(
                    ui,
                    info.line_ending,
                    info.mixed_line_endings,
                    &mut triggered,
                );
            })
            .response
            .on_hover_text(le_tip);
            ui.separator();

            // 문법 강조 언어 — 눌러서 바꾼다 (P6-A)
            ui.menu_button(info.syntax_name, |ui| {
                menu_bar::syntax_items(ui, info.syntax, info.syntax_auto_name, &mut triggered);
            })
            .response
            .on_hover_text("문법 강조 — 클릭해서 변경");
            ui.separator();

            ui.label(format!("{}%", (info.zoom * 100.0).round() as i32));
        });
    });
    triggered
}

/// 인코딩 표시 문구와 툴팁. 손상이 추정보다 우선한다.
fn encoding_label(info: &StatusBarInfo<'_>, palette: &Palette) -> (RichText, String) {
    let name = info.encoding.label();
    if info.damaged {
        return (
            RichText::new(format!("{name} · 손상")).color(palette.status_error),
            "일부 바이트를 해석하지 못했습니다(�). 저장하면 원본 바이트가 사라집니다. \
             클릭해서 다른 인코딩으로 다시 읽어 보세요."
                .to_owned(),
        );
    }
    if info.encoding_estimated {
        let mut tip =
            "자동 감지로 추정한 인코딩입니다. 글자가 깨져 보이면 '다시 읽기'로 바꿔 보세요."
                .to_owned();
        if let Some(from) = info.encoding_fallback_from {
            tip.push_str(&format!(
                "\n({from}(으)로 추정되었지만 지원하지 않는 인코딩이라 {name}(으)로 열었습니다.)"
            ));
        }
        return (
            RichText::new(format!("{name} · 추정됨")).color(palette.status_warning),
            tip,
        );
    }
    (
        RichText::new(name),
        "인코딩 — 클릭해서 다시 읽기 / 변환 저장".to_owned(),
    )
}
