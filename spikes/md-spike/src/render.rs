//! B안 렌더러: 블록 모델을 egui로 그리고, 최상위 블록마다 화면 위치를 기록한다.

use eframe::egui::{
    self, Align, Color32, FontFamily, FontId, Layout, RichText, Stroke, TextFormat, text::LayoutJob,
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

pub struct Style {
    pub base: f32,
    pub code_bg: Color32,
    pub rule: Color32,
}

impl Style {
    pub fn new(ui: &egui::Ui) -> Self {
        let dark = ui.visuals().dark_mode;
        Self {
            base: 15.0,
            code_bg: if dark {
                Color32::from_gray(40)
            } else {
                Color32::from_gray(238)
            },
            rule: ui.visuals().widgets.noninteractive.bg_stroke.color,
        }
    }
}

/// 문서를 그리고 블록 위치표를 돌려준다.
pub fn show(ui: &mut egui::Ui, doc: &Document) -> Vec<BlockPos> {
    let style = Style::new(ui);
    let origin = ui.cursor().top();
    let mut positions = Vec::with_capacity(doc.blocks.len());
    for (index, block) in doc.blocks.iter().enumerate() {
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
        ui.add_space(style.base * 0.5);
    }
    positions
}

fn render_block(ui: &mut egui::Ui, block: &Block, style: &Style) {
    match &block.kind {
        BlockKind::Heading { level, text } => {
            let scale = match level {
                1 => 1.9,
                2 => 1.5,
                3 => 1.25,
                4 => 1.1,
                _ => 1.0,
            };
            ui.add_space(style.base * 0.3);
            ui.label(inline_job(ui, text, style.base * scale, true, style));
            if *level <= 2 {
                let rect = ui.available_rect_before_wrap();
                ui.painter().hline(
                    rect.x_range(),
                    rect.top() + 2.0,
                    Stroke::new(1.0, style.rule),
                );
                ui.add_space(6.0);
            }
        }
        BlockKind::Paragraph(text) => {
            ui.label(inline_job(ui, text, style.base, false, style));
        }
        BlockKind::List { start, items } => list(ui, *start, items, style),
        BlockKind::Quote(children) => {
            let response = egui::Frame::new()
                .inner_margin(egui::Margin {
                    left: 14,
                    right: 4,
                    top: 2,
                    bottom: 2,
                })
                .show(ui, |ui| {
                    ui.visuals_mut().override_text_color = Some(ui.visuals().weak_text_color());
                    for child in children {
                        render_block(ui, child, style);
                    }
                })
                .response;
            ui.painter().vline(
                response.rect.left() + 4.0,
                response.rect.y_range(),
                Stroke::new(3.0, style.rule),
            );
        }
        BlockKind::Code { lang, text } => {
            egui::Frame::new()
                .fill(style.code_bg)
                .corner_radius(4.0)
                .inner_margin(egui::Margin::same(10))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    if !lang.is_empty() {
                        ui.label(RichText::new(lang).small().weak());
                    }
                    egui::ScrollArea::horizontal()
                        .id_salt(ui.next_auto_id())
                        .show(ui, |ui| {
                            ui.label(RichText::new(text).monospace().size(style.base * 0.9));
                        });
                });
        }
        BlockKind::Table { align, head, rows } => {
            egui::ScrollArea::horizontal()
                .id_salt(ui.next_auto_id())
                .show(ui, |ui| {
                    egui::Grid::new(ui.next_auto_id())
                        .striped(true)
                        .spacing([18.0, 6.0])
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
        }
        BlockKind::Rule => {
            ui.add_space(4.0);
            ui.separator();
            ui.add_space(4.0);
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
        let mut job = inline_job(ui, cell, style.base, head, style);
        job.wrap.max_width = f32::INFINITY;
        // 라벨 기본값은 가용 폭에 맞춰 줄바꿈하므로, 첫 프레임의 좁은 열 폭에 갇히지 않게 줄바꿈을 끈다.
        ui.add(egui::Label::new(job).extend());
    });
}

fn list(ui: &mut egui::Ui, start: Option<u64>, items: &[ListItem], style: &Style) {
    for (n, item) in items.iter().enumerate() {
        ui.horizontal_top(|ui| {
            ui.add_space(style.base * 0.4);
            let marker = match (item.task, start) {
                (Some(true), _) => "☑".to_owned(),
                (Some(false), _) => "☐".to_owned(),
                (None, Some(s)) => format!("{}.", s + n as u64),
                (None, None) => "•".to_owned(),
            };
            ui.label(RichText::new(marker).size(style.base));
            ui.vertical(|ui| {
                for child in &item.blocks {
                    render_block(ui, child, style);
                }
            });
        });
    }
}

/// 인라인 스팬 → LayoutJob. 굵게는 bold 패밀리(실제 굵은 폰트), 기울임은 egui의 italics.
fn inline_job(
    ui: &egui::Ui,
    spans: &Inlines,
    size: f32,
    all_bold: bool,
    style: &Style,
) -> LayoutJob {
    let visuals = ui.visuals();
    let text_color = visuals.override_text_color.unwrap_or(visuals.text_color());
    let mut job = LayoutJob::default();
    for span in spans {
        let s = span.style;
        let family = if s.code {
            FontFamily::Monospace
        } else if s.strong || all_bold {
            FontFamily::Name(BOLD_FAMILY.into())
        } else {
            FontFamily::Proportional
        };
        let size = if s.code { size * 0.9 } else { size };
        let mut format = TextFormat {
            font_id: FontId::new(size, family),
            color: if s.link {
                visuals.hyperlink_color
            } else if s.strong || all_bold {
                visuals.strong_text_color()
            } else {
                text_color
            },
            italics: s.emph,
            ..Default::default()
        };
        if s.code {
            format.background = style.code_bg;
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
