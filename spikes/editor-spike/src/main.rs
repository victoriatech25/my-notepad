//! P0-01 스파이크 골격: 버전을 고정한 eframe/egui로 창을 띄우고 텍스트 입력란 하나를 표시한다.
//!
//! 한글 폰트는 P0-02에서 다루므로 이 단계의 화면 문구는 영문으로 둔다.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("My-Notepad Spike (P0-01)")
            .with_inner_size([960.0, 640.0])
            .with_min_inner_size([480.0, 320.0]),
        ..Default::default()
    };
    eframe::run_native(
        "my-notepad-editor-spike",
        options,
        Box::new(|_cc| Ok(Box::<SpikeApp>::default())),
    )
}

struct SpikeApp {
    text: String,
}

impl Default for SpikeApp {
    fn default() -> Self {
        Self {
            text: "Type here to check English input.\n".to_owned(),
        }
    }
}

impl eframe::App for SpikeApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let pixels_per_point = ui.ctx().pixels_per_point();

        egui::Panel::top("info").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.strong("P0-01 spike skeleton");
                ui.separator();
                ui.label(format!(
                    "egui {} · {} ({})",
                    env!("EGUI_VERSION"),
                    std::env::consts::OS,
                    std::env::consts::ARCH
                ));
            });
        });

        egui::Panel::bottom("status").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(format!("{} chars", self.text.chars().count()));
                ui.separator();
                ui.label(format!("{} lines", self.text.lines().count().max(1)));
                ui.separator();
                ui.label(format!("scale {pixels_per_point:.2}"));
            });
        });

        egui::CentralPanel::default().show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.add_sized(
                    ui.available_size(),
                    egui::TextEdit::multiline(&mut self.text)
                        .font(egui::TextStyle::Monospace)
                        .desired_width(f32::INFINITY),
                );
            });
        });
    }
}
