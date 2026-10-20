//! P0-06 대용량 텍스트 성능 측정.
//!
//! 방식 A(egui `TextEdit` + `String` + 하이라이트 layouter)의 실용 한계 크기를 찾는다.
//! - 화면: 샘플(1/5/10/50MB)을 열고 문서 중간에 직접 타이핑해 체감 확인, 프레임 시간 표시
//! - `--perf-bench`: 창 없이 egui를 돌려 열기·첫 레이아웃·타이핑 프레임 시간을 표로 출력
//!
//! 샘플은 `cargo xtask gen-samples`로 `spikes/samples/`에 만든다.

use std::collections::VecDeque;
use std::fmt::Write as _;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use eframe::egui::{
    self, Color32, Event, FontId, Galley, Id, RawInput, Rect, RichText, TextFormat,
    text::{CCursor, CCursorRange, LayoutJob},
};

pub const SAMPLE_NAMES: &[&str] = &[
    "text-1mb.txt",
    "text-5mb.txt",
    "text-10mb.txt",
    "text-50mb.txt",
];
/// 가짜 하이라이트 개수 (WORKPLAN P0-06: 100개)
const HIGHLIGHT_LIMIT: usize = 100;
const HIGHLIGHT_WORD: &str = "ERROR";

pub fn samples_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../samples")
}

/// 앞에서부터 최대 `HIGHLIGHT_LIMIT`개의 매치 (바이트 범위)
fn find_highlights(text: &str) -> Vec<Range<usize>> {
    text.match_indices(HIGHLIGHT_WORD)
        .take(HIGHLIGHT_LIMIT)
        .map(|(start, m)| start..start + m.len())
        .collect()
}

/// 하이라이트 구간을 배경색으로 칠한 LayoutJob을 만들어 레이아웃한다.
/// 텍스트가 바뀐 직후라 구간이 어긋나면 그 구간은 건너뛴다.
fn layout_with_highlights(
    ui: &egui::Ui,
    text: &str,
    highlights: &[Range<usize>],
    font: &FontId,
) -> Arc<Galley> {
    let color = ui.visuals().text_color();
    let normal = TextFormat::simple(font.clone(), color);
    let marked = TextFormat {
        background: Color32::from_rgb(0xFF, 0xB3, 0x47),
        color: Color32::BLACK,
        ..TextFormat::simple(font.clone(), color)
    };
    let mut job = LayoutJob::default();
    let mut pos = 0;
    for r in highlights {
        if r.start < pos
            || r.end > text.len()
            || !text.is_char_boundary(r.start)
            || !text.is_char_boundary(r.end)
        {
            continue;
        }
        job.append(&text[pos..r.start], 0.0, normal.clone());
        job.append(&text[r.clone()], 0.0, marked.clone());
        pos = r.end;
    }
    job.append(&text[pos..], 0.0, normal);
    job.wrap.max_width = f32::INFINITY;
    ui.fonts_mut(|f| f.layout_job(job))
}

fn editor(
    ui: &mut egui::Ui,
    id: Id,
    text: &mut String,
    highlight: bool,
    highlights: &[Range<usize>],
) -> egui::text_edit::TextEditOutput {
    let font = FontId::monospace(14.0);
    let mut layouter = |ui: &egui::Ui, buf: &dyn egui::TextBuffer, _wrap: f32| {
        layout_with_highlights(ui, buf.as_str(), highlights, &font)
    };
    let edit = egui::TextEdit::multiline(text)
        .id(id)
        .font(FontId::monospace(14.0))
        .desired_width(f32::INFINITY)
        .lock_focus(true);
    if highlight {
        edit.layouter(&mut layouter).show(ui)
    } else {
        edit.show(ui)
    }
}

// ---------------------------------------------------------------- 화면

#[derive(Default)]
pub struct PerfScreen {
    path: Option<PathBuf>,
    text: String,
    load_ms: f64,
    highlight: bool,
    highlights: Vec<Range<usize>>,
    highlights_dirty: bool,
    frame_ms: VecDeque<f32>,
    edit_frame_ms: Option<f32>,
    error: Option<String>,
}

impl PerfScreen {
    pub fn load(&mut self, path: PathBuf) {
        let t = Instant::now();
        match std::fs::read(&path).map(String::from_utf8) {
            Ok(Ok(text)) => {
                self.text = text;
                self.load_ms = t.elapsed().as_secs_f64() * 1000.0;
                self.highlights_dirty = true;
                self.frame_ms.clear();
                self.edit_frame_ms = None;
                self.error = None;
                self.path = Some(path);
            }
            Ok(Err(e)) => self.error = Some(format!("UTF-8 아님: {e}")),
            Err(e) => self.error = Some(format!("{}: {e}", path.display())),
        }
    }

    pub fn screen(&mut self, ui: &mut egui::Ui) {
        let started = Instant::now();
        if self.highlights_dirty {
            self.highlights = find_highlights(&self.text);
            self.highlights_dirty = false;
        }

        egui::Panel::top("perf-controls").show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label("샘플:");
                let dir = samples_dir();
                for name in SAMPLE_NAMES {
                    let path = dir.join(name);
                    let selected = self.path.as_ref() == Some(&path);
                    if ui
                        .add_enabled(path.exists(), egui::Button::selectable(selected, *name))
                        .on_disabled_hover_text("먼저 `cargo xtask gen-samples`를 실행하세요")
                        .clicked()
                    {
                        self.load(path);
                    }
                }
                ui.separator();
                ui.checkbox(&mut self.highlight, format!("하이라이트 layouter ({HIGHLIGHT_LIMIT}개)"));
            });
            ui.horizontal_wrapped(|ui| {
                let avg = if self.frame_ms.is_empty() {
                    0.0
                } else {
                    self.frame_ms.iter().sum::<f32>() / self.frame_ms.len() as f32
                };
                let max = self.frame_ms.iter().copied().fold(0.0, f32::max);
                ui.label(format!(
                    "열기 {:.0} ms · 프레임(최근 60) 평균 {avg:.1} ms / 최대 {max:.1} ms",
                    self.load_ms
                ));
                if let Some(ms) = self.edit_frame_ms {
                    ui.separator();
                    let color = if ms <= 16.0 {
                        ui.visuals().text_color()
                    } else {
                        ui.visuals().warn_fg_color
                    };
                    ui.label(RichText::new(format!("마지막 입력 프레임 {ms:.1} ms")).color(color));
                }
                ui.separator();
                ui.label(format!(
                    "{:.1} MB · {} 줄",
                    self.text.len() as f64 / 1_048_576.0,
                    self.text.lines().count()
                ));
            });
            if let Some(error) = &self.error {
                ui.colored_label(ui.visuals().error_fg_color, error);
            }
            ui.label(
                RichText::new("문서 중간을 클릭해 타이핑해 보고, 스크롤바를 끝까지 끌어 보세요. 16ms 이하가 60fps입니다.")
                    .small()
                    .weak(),
            );
        });

        egui::CentralPanel::default().show(ui, |ui| {
            egui::ScrollArea::both()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let output = editor(
                        ui,
                        Id::unique("perf-editor"),
                        &mut self.text,
                        self.highlight,
                        &self.highlights,
                    );
                    if output.response.changed() {
                        self.highlights_dirty = true;
                        self.edit_frame_ms = Some(started.elapsed().as_secs_f32() * 1000.0);
                    }
                });
        });

        self.frame_ms
            .push_back(started.elapsed().as_secs_f32() * 1000.0);
        while self.frame_ms.len() > 60 {
            self.frame_ms.pop_front();
        }
        ui.ctx().request_repaint();
    }
}

// ---------------------------------------------------------------- 자동 벤치

struct Frames {
    ctx: egui::Context,
    text: String,
    highlight: bool,
    highlights: Vec<Range<usize>>,
}

impl Frames {
    /// 한 프레임을 돌리고 (레이아웃 + 테셀레이션) 걸린 시간을 ms로 돌려준다.
    fn run(&mut self, events: Vec<Event>) -> f64 {
        let raw = RawInput {
            events,
            focused: true,
            screen_rect: Some(Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1200.0, 800.0),
            )),
            ..Default::default()
        };
        let t = Instant::now();
        let mut changed = false;
        let output = self.ctx.run_ui(raw, |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                egui::ScrollArea::both().show(ui, |ui| {
                    let out = editor(
                        ui,
                        Id::unique("bench"),
                        &mut self.text,
                        self.highlight,
                        &self.highlights,
                    );
                    changed |= out.response.changed();
                });
            });
        });
        let _primitives = self.ctx.tessellate(output.shapes, output.pixels_per_point);
        if changed && self.highlight {
            self.highlights = find_highlights(&self.text);
        }
        t.elapsed().as_secs_f64() * 1000.0
    }
}

pub fn bench(fonts: egui::FontDefinitions) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "{:<14} {:>8} {:>9} {:>11} {:>11} {:>11} {:>10}",
        "샘플", "하이라이트", "열기 ms", "첫 프레임", "입력 평균", "입력 최대", "유휴 평균"
    );
    for name in SAMPLE_NAMES {
        let path = samples_dir().join(name);
        let t = Instant::now();
        let Ok(text) = std::fs::read(&path).map(String::from_utf8) else {
            let _ = writeln!(out, "{name:<14} (없음 — cargo xtask gen-samples)");
            continue;
        };
        let Ok(text) = text else {
            continue;
        };
        let load_ms = t.elapsed().as_secs_f64() * 1000.0;
        let typed_frames = if text.len() > (20 << 20) { 3 } else { 10 };

        for highlight in [false, true] {
            let ctx = egui::Context::default();
            ctx.set_fonts(fonts.clone());
            let highlights = if highlight {
                find_highlights(&text)
            } else {
                Vec::new()
            };
            let middle = text.chars().count() / 2;
            let mut frames = Frames {
                ctx: ctx.clone(),
                text: text.clone(),
                highlight,
                highlights,
            };

            let first = frames.run(Vec::new());
            // 입력란에 포커스를 주고 커서를 문서 중간에 둔다.
            ctx.memory_mut(|m| m.request_focus(Id::unique("bench")));
            frames.run(Vec::new());
            if let Some(mut state) = egui::TextEdit::load_state(&ctx, Id::unique("bench")) {
                state
                    .cursor
                    .set_char_range(Some(CCursorRange::one(CCursor::new(middle))));
                state.store(&ctx, Id::unique("bench"));
            }
            frames.run(Vec::new());

            let typed: Vec<f64> = (0..typed_frames)
                .map(|_| frames.run(vec![Event::Text("가".to_owned())]))
                .collect();
            let idle: Vec<f64> = (0..3).map(|_| frames.run(Vec::new())).collect();
            let inserted = frames.text.len() - text.len();

            let avg = |v: &[f64]| v.iter().sum::<f64>() / v.len() as f64;
            let max = |v: &[f64]| v.iter().copied().fold(0.0, f64::max);
            let _ = writeln!(
                out,
                "{name:<14} {:>8} {load_ms:>9.1} {first:>11.1} {:>11.1} {:>11.1} {:>10.1}{}",
                if highlight { "켬" } else { "끔" },
                avg(&typed),
                max(&typed),
                avg(&idle),
                if inserted == 0 {
                    "  ⚠ 입력이 반영되지 않음"
                } else {
                    ""
                }
            );
        }
    }
    out
}
