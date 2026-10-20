//! P0-08 Markdown 렌더러 비교 스파이크.
//!
//! - A: `egui_commonmark` (기존 크레이트)
//! - B: `pulldown-cmark` 파싱 + 자체 렌더러 (블록별 소스 줄 범위 → 화면 y좌표 대응)
//!
//! 실행: `cargo xtask try md` (기본: docs/PLAN.md를 A/B 나란히)
//! `--block-map [파일]`: 창 없이 B의 블록 위치표를 출력하고 단조성을 점검한다 (자동 검증용).

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[allow(dead_code)] // 스파이크 공용 모듈: 보고서 기능은 이 앱에서 쓰지 않음
#[path = "../../editor-spike/src/fonts.rs"]
mod fonts;
mod model;
mod render;
mod split;
mod sync;

use std::path::{Path, PathBuf};

use eframe::egui::{self, RichText};
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};

fn default_file() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/PLAN.md")
}

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().collect();
    let file = args
        .iter()
        .skip(1)
        .find(|a| !a.starts_with("--"))
        .map(PathBuf::from)
        .unwrap_or_else(default_file);

    if args.iter().any(|a| a == "--scroll-bench") {
        let text = std::fs::read_to_string(&file).unwrap_or_default();
        let (defs, _) = fonts::explicit_chain();
        println!("{}", file.display());
        print!("{}", split::scroll_bench(&text, defs));
        std::process::exit(0);
    }

    if args.iter().any(|a| a == "--block-map") {
        std::process::exit(block_map(&file));
    }

    let text = std::fs::read_to_string(&file).unwrap_or_else(|e| format!("# 읽기 실패\n\n{e}"));
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("My-Notepad Markdown 렌더러 비교 (P0-08)")
            .with_inner_size([1500.0, 900.0]),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    eframe::run_native(
        "my-notepad-md-spike",
        options,
        Box::new(move |cc| {
            let (defs, _) = fonts::explicit_chain();
            cc.egui_ctx.set_fonts(defs);
            Ok(Box::new(App::new(file, text)))
        }),
    )
}

/// 창 없이 B 렌더러를 한 번 그려 블록 위치표를 출력한다.
fn block_map(file: &Path) -> i32 {
    let Ok(text) = std::fs::read_to_string(file) else {
        eprintln!("읽기 실패: {}", file.display());
        return 1;
    };
    let doc = model::parse(&text);
    let ctx = egui::Context::default();
    let (defs, _) = fonts::explicit_chain();
    ctx.set_fonts(defs);
    let mut positions = Vec::new();
    // 폰트 아틀라스가 준비되도록 두 번 그린다.
    for _ in 0..2 {
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(900.0, 100_000.0),
            )),
            ..Default::default()
        };
        let _ = ctx.run_ui(raw, |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                positions = render::show(ui, &doc, render::Options::default());
            });
        });
    }
    println!(
        "{} ({}줄, 최상위 블록 {}개, 폭 900px)\n",
        file.display(),
        doc.line_count,
        doc.blocks.len()
    );
    print!("{}", render::format_positions(&positions));
    match render::check_positions(&positions, doc.blocks.len()) {
        Ok(()) => {
            println!("\n✔ 모든 최상위 블록에 소스 줄 범위 ↔ y좌표 대응이 있고, 둘 다 단조 증가");
            0
        }
        Err(e) => {
            println!("\n✘ {e}");
            1
        }
    }
}

#[derive(PartialEq, Eq, Clone, Copy)]
enum View {
    /// P0-09: 소스 + 미리보기 분할, 스크롤 동기화
    Split,
    Both,
    A,
    B,
}

struct App {
    file: PathBuf,
    text: String,
    doc: model::Document,
    cache: CommonMarkCache,
    view: View,
    show_map: bool,
    render_options: render::Options,
    positions: Vec<render::BlockPos>,
    split: split::Split,
}

impl App {
    fn new(file: PathBuf, text: String) -> Self {
        let doc = model::parse(&text);
        let split = split::Split::new(text.clone());
        Self {
            file,
            text,
            doc,
            cache: CommonMarkCache::default(),
            view: View::Split,
            show_map: false,
            render_options: render::Options::default(),
            positions: Vec::new(),
            split,
        }
    }

    fn pane_a(&mut self, ui: &mut egui::Ui) {
        ui.heading("A. egui_commonmark 0.26");
        ui.label(
            RichText::new("블록 위치 API 없음 — 스크롤 동기화에 쓸 대응표를 얻을 수 없다")
                .small()
                .weak(),
        );
        ui.separator();
        egui::ScrollArea::vertical()
            .id_salt("pane-a")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                CommonMarkViewer::new().show(ui, &mut self.cache, &self.text);
            });
    }

    fn pane_b(&mut self, ui: &mut egui::Ui) {
        ui.heading("B. pulldown-cmark + 자체 렌더러");
        ui.label(
            RichText::new(format!(
                "최상위 블록 {}개, 각 블록의 소스 줄 범위 ↔ y좌표 기록",
                self.doc.blocks.len()
            ))
            .small()
            .weak(),
        );
        ui.separator();
        egui::ScrollArea::vertical()
            .id_salt("pane-b")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                egui::Frame::new()
                    .inner_margin(egui::Margin::symmetric(8, 4))
                    .show(ui, |ui| {
                        self.positions = render::show(ui, &self.doc, self.render_options);
                    });
            });
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::Panel::top("controls").show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.strong("P0-08 Markdown 렌더러 비교");
                ui.separator();
                ui.label(RichText::new(self.file.display().to_string()).small());
                ui.separator();
                ui.selectable_value(&mut self.view, View::Split, "분할 동기화 (P0-09)");
                ui.selectable_value(&mut self.view, View::Both, "A | B 나란히");
                ui.selectable_value(&mut self.view, View::A, "A만");
                ui.selectable_value(&mut self.view, View::B, "B만");
                ui.separator();
                ui.toggle_value(&mut self.show_map, "B 블록 위치표");
                ui.separator();
                ui.checkbox(
                    &mut self.render_options.bold_font,
                    "B: 굵게·제목을 굵은 글꼴로 (끄면 A처럼 진한 색)",
                );
                if self.view == View::Split {
                    ui.separator();
                    self.split.controls(ui);
                }
            });
        });

        if self.show_map {
            egui::Panel::right("map")
                .resizable(true)
                .default_size(380.0)
                .show(ui, |ui| {
                    ui.heading("B 블록 위치표");
                    ui.label(
                        RichText::new("소스 줄 ↔ 렌더러 내용 기준 y(px)")
                            .small()
                            .weak(),
                    );
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        ui.label(
                            RichText::new(render::format_positions(&self.positions))
                                .monospace()
                                .small(),
                        );
                    });
                });
        }

        egui::CentralPanel::default().show(ui, |ui| match self.view {
            View::A => self.pane_a(ui),
            View::B => self.pane_b(ui),
            View::Split => {
                self.split.options = self.render_options;
                self.split.ui(ui);
            }
            View::Both => {
                let half = ui.available_width() / 2.0;
                egui::Panel::left("left-a")
                    .resizable(true)
                    .default_size(half)
                    .show(ui, |ui| self.pane_a(ui));
                self.pane_b(ui);
            }
        });
    }
}
