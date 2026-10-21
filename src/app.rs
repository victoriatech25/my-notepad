//! 앱 루트 상태 및 화면 조합 (`NotepadApp` — PLAN.md §4.2).
//!
//! P1-01에서는 메뉴바·상태 표시줄·본문 영역 자리를 갖춘 빈 창 골격을 구성한다.

use eframe::egui;

use crate::ui::{menu_bar, status_bar, theme::Palette};

/// My-Notepad 메인 애플리케이션 상태.
#[derive(Default)]
pub struct NotepadApp {
    frames: u64,
}

impl NotepadApp {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        Self::default()
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
            status_bar::show_placeholder(ui);
        });

        egui::CentralPanel::default().show(ui, |ui| {
            ui.centered_and_justified(|ui| {
                ui.colored_label(
                    palette.gutter_text,
                    "My-Notepad — 빈 문서 (P1-09에서 에디터 위젯을 연결합니다)",
                );
            });
        });
    }
}
