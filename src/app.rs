//! 앱 루트 상태 및 화면 조합 (`NotepadApp` — PLAN.md §4.2).
//!
//! - P1-01: 메뉴바·상태 표시줄·본문 영역 골격
//! - P1-02: OS별 시스템 폰트 Fallback 체인(`fonts.rs`) 적용, 로드 실패 시 상태 표시줄/로그 경고,
//!   P0-02 체크리스트 4개 항목 확인용 화면 구성

use eframe::egui::{self, FontFamily, RichText};

use crate::fonts::{self, BOLD_FAMILY, FontReport, ICON_FAMILY};
use crate::ui::{menu_bar, status_bar, theme::Palette};

const SAMPLE_TEXT: &str = "\
English: The quick brown fox jumps over the lazy dog. 0123456789
한글: 다람쥐 헌 쳇바퀴에 타고파. 키스의 고유조건은 입술끼리 만나야 하고 특별한 기술은 필요치 않다.
기호: ()[]{}<>!@#$%^&*-_=+|\\/:;\"'`~ ← → ★ ※ ①②③

[정렬 확인: 영문·숫자 줄 (고정폭 세로 정렬)]
ID    NAME        QTY    PRICE
0001  apple         12    1,200
0002  banana       345   34,500
0003  cherry      6789  678,900

[정렬 확인: 한글 포함 줄]
ID    이름        수량   가격
0004  사과          12    1,200
0005  바나나       345   34,500

Tab 키 입력 확인 →\t← 이 사이에 탭 문자가 있습니다.
";

const PROPORTIONAL_SAMPLE: &str = "메뉴·대화상자·Markdown 미리보기 본문에 쓰이는 비례폭(Proportional) 글꼴입니다. \
한글과 English가 섞인 문장도 자연스럽게 이어져야 하며, 숫자 1234567890과 기호 (괄호), \"따옴표\"도 깨짐 없이 표시됩니다.";

/// My-Notepad 메인 애플리케이션 상태.
pub struct NotepadApp {
    frames: u64,
    font_report: FontReport,
    font_warning: Option<String>,
    sample_text: String,
}

impl NotepadApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let (defs, report) = fonts::explicit_chain();
        cc.egui_ctx.set_fonts(defs);

        log::info!(
            "폰트 체인 로드 완료 ({:.2} ms, 성공 {}개, 실패 {}개)",
            report.total.as_secs_f64() * 1000.0,
            report.loaded.len(),
            report.failures.len()
        );
        for failure in &report.failures {
            log::warn!("시스템 폰트 로드 실패 (기본 폰트로 대체): {failure}");
        }

        let font_warning = report.status_warning();
        Self {
            frames: 0,
            font_report: report,
            font_warning,
            sample_text: SAMPLE_TEXT.to_owned(),
        }
    }
}

impl eframe::App for NotepadApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.frames += 1;
        let palette = Palette::from_visuals(ui.visuals());

        egui::Panel::top("menu_bar").show(ui, |ui| {
            menu_bar::show_placeholder(ui);
        });

        egui::Panel::bottom("status_bar").show(ui, |ui| {
            status_bar::show_placeholder(ui, &palette, self.font_warning.as_deref());
        });

        egui::CentralPanel::default().show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.heading("1. 에디터 고정폭 (Monospace: 영문 고정폭 + 한글 Fallback)");
                ui.add(
                    egui::TextEdit::multiline(&mut self.sample_text)
                        .font(egui::TextStyle::Monospace)
                        .desired_width(f32::INFINITY)
                        .desired_rows(16)
                        .lock_focus(true),
                );

                ui.add_space(10.0);
                ui.heading("2. 비례폭 (Proportional: 메뉴·대화상자·미리보기 본문)");
                ui.label(PROPORTIONAL_SAMPLE);

                ui.add_space(10.0);
                ui.heading("3. 굵게 (bold 패밀리: 보통 한글 vs 굵은 한글)");
                ui.horizontal(|ui| {
                    ui.label(RichText::new("보통 한글 Regular 123").size(18.0));
                    ui.separator();
                    ui.label(
                        RichText::new("굵은 한글 Bold 123")
                            .size(18.0)
                            .family(FontFamily::Name(BOLD_FAMILY.into())),
                    );
                    ui.separator();
                    ui.label(
                        RichText::new("\u{EB28} \u{ED50} \u{E24A} \u{E256}")
                            .size(18.0)
                            .family(FontFamily::Name(ICON_FAMILY.into()))
                            .color(palette.modified_marker),
                    );
                    ui.weak("(Phosphor 아이콘 폰트)");
                });

                ui.add_space(10.0);
                ui.collapsing("시스템 폰트 로드 보고서 (P1-02)", |ui| {
                    ui.label(
                        RichText::new(self.font_report.to_text())
                            .monospace()
                            .small(),
                    );
                });
            });
        });
    }
}
