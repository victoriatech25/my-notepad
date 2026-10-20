//! P0-09 분할 화면: 왼쪽 소스(`TextEdit`) + 오른쪽 미리보기(B 렌더러), 양방향 스크롤 동기화.
//!
//! 동기화 규칙
//! - 사용자가 움직인 쪽(이번 프레임에 기대값과 달라진 쪽, 둘 다면 마우스가 올라간 쪽)을 기준으로
//!   반대쪽의 목표 위치를 계산해 다음 프레임에 적용한다.
//! - 적용한 값을 "기대값"으로 기억해, 그 변화가 다시 반대 방향 동기화를 일으키지 않게 한다 (되먹임 방지).
//! - 기준점은 각 화면의 맨 위 (보이는 첫 줄 ↔ 그 줄에 해당하는 미리보기 위치).

use std::collections::VecDeque;
use std::time::Instant;

use eframe::egui::{self, RichText};

use crate::model::{self, Document};
use crate::render::{self, BlockPos};
use crate::sync::{self, SourceLines};

pub struct Split {
    text: String,
    doc: Document,
    pub options: render::Options,
    pub sync_on: bool,
    pub virtual_on: bool,
    // 미리보기 위치표 캐시 (폭이 바뀌거나 문서가 바뀌면 전체를 다시 그려 갱신)
    positions: Vec<BlockPos>,
    positions_width: f32,
    positions_valid: bool,
    // 소스 줄 위치표 (매 프레임 갱신)
    src_lines: SourceLines,
    // 되먹임 방지용 기대값과 다음 프레임에 적용할 목표
    expected_src: f32,
    expected_prev: f32,
    pending_src: Option<f32>,
    pending_prev: Option<f32>,
    last_driver: &'static str,
    frame_ms: VecDeque<f32>,
}

impl Split {
    pub fn new(text: String) -> Self {
        let doc = model::parse(&text);
        Self {
            text,
            doc,
            options: render::Options::default(),
            sync_on: true,
            virtual_on: true,
            positions: Vec::new(),
            positions_width: 0.0,
            positions_valid: false,
            src_lines: SourceLines {
                tops: Vec::new(),
                total: 0.0,
            },
            expected_src: 0.0,
            expected_prev: 0.0,
            pending_src: None,
            pending_prev: None,
            last_driver: "-",
            frame_ms: VecDeque::new(),
        }
    }

    pub fn controls(&mut self, ui: &mut egui::Ui) {
        ui.checkbox(&mut self.sync_on, "스크롤 동기화");
        ui.checkbox(&mut self.virtual_on, "미리보기 가상 스크롤");
        let avg = if self.frame_ms.is_empty() {
            0.0
        } else {
            self.frame_ms.iter().sum::<f32>() / self.frame_ms.len() as f32
        };
        let max = self.frame_ms.iter().copied().fold(0.0, f32::max);
        ui.separator();
        ui.label(format!(
            "프레임 평균 {avg:.1} / 최대 {max:.1} ms · 기준: {}",
            self.last_driver
        ));
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        let started = Instant::now();

        // ── 왼쪽: 소스
        let src = egui::Panel::left("split-source")
            .resizable(true)
            .default_size(ui.available_width() / 2.0)
            .show(ui, |ui| {
                ui.label(RichText::new("소스").weak());
                let mut area = egui::ScrollArea::both()
                    .id_salt("split-src")
                    .auto_shrink([false, false]);
                if let Some(offset) = self.pending_src.take() {
                    area = area.vertical_scroll_offset(offset);
                }
                area.show(ui, |ui| {
                    egui::TextEdit::multiline(&mut self.text)
                        .font(egui::TextStyle::Monospace)
                        .desired_width(f32::INFINITY)
                        .lock_focus(true)
                        .show(ui)
                })
            })
            .inner;

        if src.inner.response.changed() {
            self.doc = model::parse(&self.text);
            self.positions_valid = false;
        }
        // 소스 줄 위치표: 긴 줄은 화면에서 여러 행으로 줄바꿈되므로, 새 소스 줄이 시작되는 행만 모은다.
        let content_top = src.inner_rect.top() - src.state.clamped_offset().y;
        let galley_top = src.inner.galley_pos.y - content_top;
        let mut starts_line = true;
        self.src_lines = SourceLines {
            tops: src
                .inner
                .galley
                .rows
                .iter()
                .filter_map(|r| {
                    let top = starts_line.then(|| galley_top + r.rect().top());
                    starts_line = r.ends_with_newline;
                    top
                })
                .collect(),
            total: galley_top + src.inner.galley.rect.height(),
        };

        // ── 오른쪽: 미리보기
        let prev = egui::CentralPanel::default()
            .show(ui, |ui| {
                ui.label(RichText::new("미리보기").weak());
                let mut area = egui::ScrollArea::vertical()
                    .id_salt("split-prev")
                    .auto_shrink([false, false]);
                if let Some(offset) = self.pending_prev.take() {
                    area = area.vertical_scroll_offset(offset);
                }
                let width = ui.available_width();
                let use_virtual = self.virtual_on
                    && self.positions_valid
                    && (self.positions_width - width).abs() < 0.5;
                if use_virtual {
                    area.show_viewport(ui, |ui, viewport| {
                        render::show_viewport(
                            ui,
                            &self.doc,
                            self.options,
                            &self.positions,
                            viewport,
                        );
                    })
                } else {
                    let out = area.show(ui, |ui| render::show(ui, &self.doc, self.options));
                    self.positions.clone_from(&out.inner);
                    self.positions_width = width;
                    self.positions_valid = true;
                    egui::scroll_area::ScrollAreaOutput {
                        inner: (),
                        id: out.id,
                        state: out.state,
                        content_size: out.content_size,
                        inner_rect: out.inner_rect,
                    }
                }
            })
            .inner;

        // ── 동기화
        let src_off = src.state.clamped_offset().y;
        let prev_off = prev.state.clamped_offset().y;
        let src_max = (src.content_size.y - src.inner_rect.height()).max(0.0);
        let prev_max = (prev.content_size.y - prev.inner_rect.height()).max(0.0);
        let src_moved = (src_off - self.expected_src).abs() > 0.5;
        let prev_moved = (prev_off - self.expected_prev).abs() > 0.5;
        if std::env::var_os("MD_SPIKE_DEBUG").is_some() {
            eprintln!(
                "src {src_off:.1}/{src_max:.1} exp {:.1} | prev {prev_off:.1}/{prev_max:.1} exp {:.1} | valid {} lines {}",
                self.expected_src,
                self.expected_prev,
                self.positions_valid,
                self.src_lines.tops.len()
            );
        }
        let src_hovered = ui.rect_contains_pointer(src.inner_rect);
        let line_count = self.src_lines.tops.len();
        let total = render::total_height(&self.positions);

        self.expected_src = src_off;
        self.expected_prev = prev_off;
        if self.sync_on && self.positions_valid {
            if src_moved && (!prev_moved || src_hovered) {
                let line = self.src_lines.line_at(src_off);
                let target = sync::preview_y_of_line(&self.positions, line_count, total, line)
                    .clamp(0.0, prev_max);
                self.pending_prev = Some(target);
                self.expected_prev = target;
                self.last_driver = "소스";
            } else if prev_moved {
                let line = sync::line_of_preview_y(&self.positions, line_count, total, prev_off);
                let target = self.src_lines.y_of_line(line).clamp(0.0, src_max);
                self.pending_src = Some(target);
                self.expected_src = target;
                self.last_driver = "미리보기";
            }
            if self.pending_src.is_some() || self.pending_prev.is_some() {
                ui.ctx().request_repaint();
            }
        }

        self.frame_ms
            .push_back(started.elapsed().as_secs_f32() * 1000.0);
        while self.frame_ms.len() > 60 {
            self.frame_ms.pop_front();
        }
    }
}

/// `--scroll-bench <파일>`: 창 없이 미리보기를 위에서 아래로 스크롤하며 프레임 시간을 잰다 (전체 vs 가상).
pub fn scroll_bench(text: &str, fonts: egui::FontDefinitions) -> String {
    let doc = model::parse(text);
    let mut out = String::new();
    for virtual_on in [false, true] {
        let ctx = egui::Context::default();
        ctx.set_fonts(fonts.clone());
        let mut positions: Vec<BlockPos> = Vec::new();
        let mut times = Vec::new();
        let height = 900.0;
        let mut offset = 0.0_f32;
        // 첫 두 프레임은 위치표·폰트 준비, 이후 반 화면씩 스크롤
        for frame in 0..62 {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(900.0, height),
                )),
                ..Default::default()
            };
            let t = Instant::now();
            let output = ctx.run_ui(raw, |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    let area = egui::ScrollArea::vertical()
                        .id_salt("bench")
                        .auto_shrink([false, false])
                        .vertical_scroll_offset(offset);
                    if virtual_on && !positions.is_empty() {
                        area.show_viewport(ui, |ui, viewport| {
                            render::show_viewport(
                                ui,
                                &doc,
                                render::Options::default(),
                                &positions,
                                viewport,
                            );
                        });
                    } else {
                        let o =
                            area.show(ui, |ui| render::show(ui, &doc, render::Options::default()));
                        positions = o.inner;
                    }
                });
            });
            let _ = ctx.tessellate(output.shapes, output.pixels_per_point);
            if frame >= 2 {
                times.push(t.elapsed().as_secs_f64() * 1000.0);
                offset += height / 2.0;
                if offset > render::total_height(&positions) {
                    offset = 0.0;
                }
            }
        }
        let avg = times.iter().sum::<f64>() / times.len() as f64;
        let max = times.iter().copied().fold(0.0, f64::max);
        out.push_str(&format!(
            "{:<10} 블록 {:>5}개 · 스크롤 프레임 평균 {avg:>7.2} ms / 최대 {max:>7.2} ms (60프레임)\n",
            if virtual_on { "가상" } else { "전체" },
            doc.blocks.len()
        ));
    }
    out
}
