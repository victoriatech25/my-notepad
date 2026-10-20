//! B안 렌더러: 블록 모델을 egui로 그리고, 최상위 블록마다 화면 위치를 기록한다.
//!
//! UX-004: 화면 모양은 A(egui_commonmark 0.26)에 맞춘다 — 제목은 egui Heading/Body 크기 사이의
//! 단계 크기 + 진한 색(굵은 글꼴 아님), 굵게는 진한 색, 블록 사이 한 줄 여백, 표는 테두리 틀 +
//! 줄무늬 격자, 인용은 왼쪽 3px 선 + 흐린 글자. 굵게를 실제 굵은 글꼴로 그릴지는 옵션으로 비교한다.

use eframe::egui::{
    self, Align, FontFamily, FontId, Layout, RichText, Sense, Stroke, TextFormat, TextStyle,
    text::LayoutJob,
};
use pulldown_cmark::Alignment;

use crate::fonts::BOLD_FAMILY;
use crate::model::{Block, BlockKind, Document, Inlines, ListItem};

/// 최상위 블록 하나의 소스 줄 범위와 화면 위치 (스크롤 동기화의 기반)
#[derive(Clone)]
pub struct BlockPos {
    pub index: usize,
    pub kind: &'static str,
    pub first_line: usize,
    pub last_line: usize,
    /// 렌더러 내용 기준 y (스크롤과 무관하게 0부터)
    pub top: f32,
    pub height: f32,
}

#[derive(Clone, Copy, Default)]
pub struct Options {
    /// 굵게·제목을 실제 굵은 글꼴(bold 패밀리)로 그린다. 끄면 A처럼 진한 색만 쓴다.
    pub bold_font: bool,
}

struct Style {
    body: f32,
    heading_max: f32,
    line: f32,
    options: Options,
}

impl Style {
    fn new(ui: &egui::Ui, options: Options) -> Self {
        let size =
            |s: TextStyle, default: f32| ui.style().text_styles.get(&s).map_or(default, |f| f.size);
        Self {
            body: size(TextStyle::Body, 14.0),
            heading_max: size(TextStyle::Heading, 32.0),
            line: ui.text_style_height(&TextStyle::Body),
            options,
        }
    }

    /// A와 같은 제목 크기: H1은 Heading 크기, H2~H6은 Body와 Heading 사이를 단계별로
    fn heading_size(&self, level: u8) -> f32 {
        let diff = self.heading_max - self.body;
        let factor = match level {
            1 => 1.0,
            2 => 0.835,
            3 => 0.668,
            4 => 0.501,
            5 => 0.334,
            _ => 0.167,
        };
        self.body + diff * factor
    }
}

/// 문서를 그리고 블록 위치표를 돌려준다.
pub fn show(ui: &mut egui::Ui, doc: &Document, options: Options) -> Vec<BlockPos> {
    let style = Style::new(ui, options);
    let origin = ui.cursor().top();
    let mut positions = Vec::with_capacity(doc.blocks.len());
    for (index, block) in doc.blocks.iter().enumerate() {
        if index > 0 {
            // A처럼 블록 사이에 한 줄 여백, 제목 앞은 조금 더
            let extra = if matches!(block.kind, BlockKind::Heading { .. }) {
                0.6
            } else {
                0.0
            };
            ui.add_space(style.line * (0.8 + extra));
        }
        let rect = ui
            .scope(|ui| {
                ui.set_width(ui.available_width());
                render_block(ui, block, &style);
            })
            .response
            .rect;
        positions.push(BlockPos {
            index,
            kind: block.kind.name(),
            first_line: block.first_line,
            last_line: block.last_line,
            top: rect.top() - origin,
            height: rect.height(),
        });
    }
    positions
}

fn render_block(ui: &mut egui::Ui, block: &Block, style: &Style) {
    match &block.kind {
        BlockKind::Heading { level, text } => {
            // H1~H4는 진한 색(A의 strong), H5·H6은 본문 색
            let strong = *level <= 4;
            ui.label(inline_job(
                ui,
                text,
                style.heading_size(*level),
                strong,
                style,
            ));
        }
        BlockKind::Paragraph(text) => {
            ui.label(inline_job(ui, text, style.body, false, style));
        }
        BlockKind::List { start, items } => list(ui, *start, items, style),
        BlockKind::Quote(children) => {
            let start = ui.painter().add(egui::Shape::Noop);
            let response = egui::Frame::new()
                .outer_margin(egui::Margin {
                    left: 10,
                    ..Default::default()
                })
                .inner_margin(egui::Margin {
                    left: 10,
                    ..Default::default()
                })
                .show(ui, |ui| {
                    ui.visuals_mut().override_text_color = Some(ui.visuals().weak_text_color());
                    for (i, child) in children.iter().enumerate() {
                        if i > 0 {
                            ui.add_space(style.line * 0.5);
                        }
                        render_block(ui, child, style);
                    }
                })
                .response;
            let r = response.rect;
            ui.painter().set(
                start,
                egui::Shape::line_segment(
                    [
                        egui::pos2(r.left() + 10.0, r.top() + 2.0),
                        egui::pos2(r.left() + 10.0, r.bottom() - 2.0),
                    ],
                    Stroke::new(3.0, ui.visuals().weak_text_color()),
                ),
            );
        }
        BlockKind::Code { lang, text } => {
            egui::Frame::group(ui.style())
                .fill(ui.visuals().code_bg_color)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    if !lang.is_empty() {
                        ui.label(RichText::new(lang).small().weak());
                    }
                    egui::ScrollArea::horizontal()
                        .id_salt(ui.next_auto_id())
                        .show(ui, |ui| {
                            ui.label(RichText::new(text).monospace());
                        });
                });
        }
        BlockKind::Table { align, head, rows } => {
            egui::Frame::group(ui.style()).show(ui, |ui| {
                // A와 같이 가로 스크롤바가 마지막 행을 가리지 않도록 아래 여백을 둔다.
                ui.spacing_mut().scroll.content_margin.bottom = ui.spacing().scroll.bar_width as i8;
                egui::ScrollArea::horizontal()
                    .id_salt(ui.next_auto_id())
                    .show(ui, |ui| {
                        egui::Grid::new(ui.next_auto_id())
                            .striped(true)
                            .show(ui, |ui| {
                                for (c, cell) in head.iter().enumerate() {
                                    cell_ui(ui, cell, align.get(c), true, style);
                                }
                                ui.end_row();
                                for row in rows {
                                    for (c, cell) in row.iter().enumerate() {
                                        cell_ui(ui, cell, align.get(c), false, style);
                                    }
                                    ui.end_row();
                                }
                            });
                    });
            });
        }
        BlockKind::Rule => {
            ui.add(egui::Separator::default().horizontal());
        }
        BlockKind::Html(html) => {
            ui.label(RichText::new(html).monospace().weak());
        }
    }
}

fn cell_ui(
    ui: &mut egui::Ui,
    cell: &Inlines,
    align: Option<&Alignment>,
    head: bool,
    style: &Style,
) {
    let layout = match align {
        Some(Alignment::Center) => Layout::top_down(Align::Center),
        Some(Alignment::Right) => Layout::top_down(Align::Max),
        _ => Layout::top_down(Align::Min),
    };
    ui.with_layout(layout, |ui| {
        let mut job = inline_job(ui, cell, style.body, head, style);
        job.wrap.max_width = f32::INFINITY;
        // 라벨 기본값은 가용 폭에 맞춰 줄바꿈하므로, 첫 프레임의 좁은 열 폭에 갇히지 않게 줄바꿈을 끈다.
        ui.add(egui::Label::new(job).extend());
    });
}

/// A와 같은 글머리: 공백 4칸 폭 안에 작은 원 (번호 목록은 번호, 작업 목록은 체크 표시)
fn list(ui: &mut egui::Ui, start: Option<u64>, items: &[ListItem], style: &Style) {
    let space = ui.fonts_mut(|f| f.glyph_width(&FontId::proportional(style.body), ' '));
    for (n, item) in items.iter().enumerate() {
        ui.horizontal_top(|ui| {
            let (rect, _) =
                ui.allocate_exact_size(egui::vec2(space * 4.0, style.line), Sense::hover());
            let color = ui.visuals().strong_text_color();
            match (item.task, start) {
                (Some(checked), _) => {
                    ui.painter().text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        if checked { "☑" } else { "☐" },
                        FontId::proportional(style.body),
                        color,
                    );
                }
                (None, Some(s)) => {
                    ui.painter().text(
                        rect.right_center() - egui::vec2(space, 0.0),
                        egui::Align2::RIGHT_CENTER,
                        format!("{}.", s + n as u64),
                        FontId::proportional(style.body),
                        color,
                    );
                }
                (None, None) => {
                    ui.painter()
                        .circle_filled(rect.center(), rect.height() / 6.0, color);
                }
            }
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                for (i, child) in item.blocks.iter().enumerate() {
                    if i > 0 {
                        ui.add_space(style.line * 0.4);
                    }
                    render_block(ui, child, style);
                }
            });
        });
    }
}

/// 인라인 스팬 → LayoutJob.
fn inline_job(
    ui: &egui::Ui,
    spans: &Inlines,
    size: f32,
    all_strong: bool,
    style: &Style,
) -> LayoutJob {
    let visuals = ui.visuals();
    let text_color = visuals.override_text_color.unwrap_or(visuals.text_color());
    let mut job = LayoutJob::default();
    for span in spans {
        let s = span.style;
        let strong = s.strong || all_strong;
        let family = if s.code {
            FontFamily::Monospace
        } else if strong && style.options.bold_font {
            FontFamily::Name(BOLD_FAMILY.into())
        } else {
            FontFamily::Proportional
        };
        let mut format = TextFormat {
            font_id: FontId::new(if s.code { style.body } else { size }, family),
            color: if s.link {
                visuals.hyperlink_color
            } else if strong && visuals.override_text_color.is_none() {
                visuals.strong_text_color()
            } else {
                text_color
            },
            italics: s.emph,
            ..Default::default()
        };
        if s.code {
            format.background = visuals.code_bg_color;
        }
        if s.strike {
            format.strikethrough = Stroke::new(1.0, text_color);
        }
        if s.link {
            format.underline = Stroke::new(1.0, visuals.hyperlink_color);
        }
        job.append(&span.text, 0.0, format);
    }
    job.wrap.max_width = ui.available_width();
    job
}

/// 위치표를 사람이 읽을 수 있는 표로
pub fn format_positions(positions: &[BlockPos]) -> String {
    let mut out = String::from("  #  종류    소스 줄        y(px)   높이\n");
    for p in positions {
        out.push_str(&format!(
            "{:>3}  {:<6} {:>5}–{:<5} {:>8.0} {:>6.0}\n",
            p.index,
            p.kind,
            p.first_line + 1,
            p.last_line + 1,
            p.top,
            p.height
        ));
    }
    out
}

/// 위치표 점검: 모든 블록에 위치가 있고, y와 소스 줄이 모두 단조 증가하는지
pub fn check_positions(positions: &[BlockPos], block_count: usize) -> Result<(), String> {
    if positions.len() != block_count {
        return Err(format!(
            "블록 {block_count}개 중 {}개만 위치가 있음",
            positions.len()
        ));
    }
    for w in positions.windows(2) {
        if w[1].top < w[0].top {
            return Err(format!("y가 거꾸로: #{} → #{}", w[0].index, w[1].index));
        }
        if w[1].first_line < w[0].first_line {
            return Err(format!(
                "소스 줄이 거꾸로: #{} → #{}",
                w[0].index, w[1].index
            ));
        }
    }
    Ok(())
}
