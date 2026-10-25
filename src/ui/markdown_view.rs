//! Markdown 미리보기 렌더러 (자체 블록 렌더링 — ADR 0002, P3-02).
//!
//! - [`core::markdown`](crate::core::markdown) 블록 모델을 egui 위젯으로 직접 그린다. 파싱은 탭별
//!   [`MarkdownCache`]가 문서 버전이 바뀔 때만 한다.
//! - 화면 규칙은 UX-004(P0-08)에서 정한 A(egui_commonmark) 모양을 따른다: 제목은 본문 크기의 단계별 배율
//!   (H1 = 2배) + 진한 색, 굵게는 진한 색(굵은 글꼴은 쓰지 않음 — ADR 0002), 블록 사이 한 줄 여백,
//!   인용은 왼쪽 3px 선 + 흐린 글자, 목록 글머리는 공백 4칸 폭 안에 그린다.
//! - 최상위 블록마다 그려진 위치([`BlockPos`])를 기록한다 — 스크롤 동기화(P3-08)·가상 스크롤(P3-05)의 기반.
//! - 코드 블록·표(P3-03), 이미지·Front Matter·각주·HTML(P3-04)은 지금은 단순 표시만 한다.

use eframe::egui::{
    self, Align, Color32, FontFamily, FontId, Layout, RichText, Sense, Stroke, TextFormat,
    TextStyle, text::LayoutJob,
};

use crate::core::document::Document;
use crate::core::markdown::{self, Block, BlockKind, Inlines, ListItem, MarkdownCache, SpanKind};
use crate::ui::theme::Palette;

/// 미리보기 본문 바깥 여백(px).
const MARGIN: i8 = 16;

/// 최상위 블록 하나의 소스 줄 범위와 화면 위치.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BlockPos {
    pub first_line: usize,
    pub last_line: usize,
    /// 스크롤 내용 기준 y (바깥 여백 포함, 스크롤 위치와 무관)
    pub top: f32,
    pub height: f32,
}

/// 탭 하나의 미리보기 상태.
#[derive(Debug)]
pub struct PreviewState {
    id: u64,
    cache: MarkdownCache,
    /// 마지막으로 그린 파싱 결과 (제목 줄 조회용)
    parsed: Option<std::sync::Arc<markdown::Document>>,
    positions: Vec<BlockPos>,
    /// 마지막 프레임의 세로 스크롤 위치
    offset_y: f32,
    /// 마지막 프레임의 보이는 높이와 세로 스크롤 최대값
    view_height: f32,
    max_offset: f32,
    /// 다음 프레임에 적용할 세로 스크롤 위치
    pending_offset: Option<f32>,
    /// 다음에 그릴 때 이 소스 줄이 화면 맨 위에 오게 한다 (보기 모드 전환, P3-06)
    pending_top_line: Option<f32>,
}

impl PreviewState {
    pub fn new(id: u64) -> Self {
        Self {
            id,
            cache: MarkdownCache::default(),
            parsed: None,
            positions: Vec::new(),
            offset_y: 0.0,
            view_height: 0.0,
            max_offset: 0.0,
            pending_offset: None,
            pending_top_line: None,
        }
    }

    /// 마지막으로 그린 화면 맨 위의 소스 줄 (0부터, 소수 = 줄 안 위치). 아직 그린 적이 없으면 `None`.
    pub fn top_line(&self) -> Option<f32> {
        self.line_at(0.0)
    }

    /// 마지막으로 그린 화면에서 위로부터 `dy`만큼 내려간 곳의 소스 줄.
    pub fn line_at(&self, dy: f32) -> Option<f32> {
        (!self.positions.is_empty()).then(|| y_to_line(&self.positions, self.offset_y + dy))
    }

    /// `line`이 위로부터 `dy` 위치에 오는 세로 스크롤 위치 (0 미만이면 0).
    pub fn offset_for_line(&self, line: f32, dy: f32) -> f32 {
        (line_to_y(&self.positions, line) - dy).max(0.0)
    }

    /// 마지막 프레임의 (스크롤 비율 0~1, 보이는 높이). 스크롤할 내용이 없으면 비율은 0.
    pub fn scroll_fraction(&self) -> (f32, f32) {
        let fraction = if self.max_offset > 0.0 {
            (self.offset_y / self.max_offset).clamp(0.0, 1.0)
        } else {
            0.0
        };
        (fraction, self.view_height)
    }

    /// 다음에 그릴 때 `line`이 화면 맨 위에 오도록 스크롤한다.
    ///
    /// 블록 위치는 그 폭으로 그려 봐야 알 수 있으므로, 그 프레임은 보이지 않게 그리고 다음 프레임에 맞춘다.
    pub fn scroll_to_line(&mut self, line: f32) {
        self.pending_top_line = Some(line.max(0.0));
    }

    /// 마지막 프레임의 세로 스크롤 위치.
    pub fn offset_y(&self) -> f32 {
        self.offset_y
    }

    /// 다음 프레임에 세로 스크롤 위치를 `y`로 맞춘다 (스크롤 동기화, P3-08).
    pub fn set_scroll_offset(&mut self, y: f32) {
        self.pending_offset = Some(y.max(0.0));
    }

    /// 다음 프레임에 적용할 스크롤 요청이 있는지 (그 프레임의 스크롤 변화는 사용자 조작이 아니다).
    pub fn has_pending_scroll(&self) -> bool {
        self.pending_offset.is_some() || self.pending_top_line.is_some()
    }

    /// 보기 모드 전환으로 위치를 다시 맞추는 중인지 (숨겨 그리는 프레임).
    pub fn is_repositioning(&self) -> bool {
        self.pending_top_line.is_some()
    }

    /// 마지막으로 그린 문서의 제목 소스 줄 (문서 순서, 중첩 제목 포함).
    pub fn heading_lines(&self) -> impl Iterator<Item = usize> + '_ {
        self.parsed
            .iter()
            .flat_map(|d| d.anchors.iter().map(|a| a.line))
    }

    /// 마지막으로 그린 최상위 블록 위치표.
    pub fn positions(&self) -> &[BlockPos] {
        &self.positions
    }

    /// 문서가 통째로 바뀌었을 때 다음 그리기에서 다시 파싱하게 한다.
    pub fn invalidate(&mut self) {
        self.cache.invalidate();
    }

    /// 실제로 파싱한 횟수 (캐시 확인용).
    pub fn parse_count(&self) -> u64 {
        self.cache.parse_count()
    }

    /// 문서를 미리보기로 그린다. `zoom`은 에디터 확대/축소 배율,
    /// `cursor_line`(0부터)이 있으면 그 줄을 포함한 최상위 블록에 옅은 배경을 깐다 (분할 모드 커서 위치, P3-08).
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        doc: &Document,
        zoom: f32,
        palette: &Palette,
        cursor_line: Option<usize>,
    ) -> PreviewOutput {
        let parsed = self.cache.get(doc.revision(), doc.text());
        self.parsed = Some(std::sync::Arc::clone(&parsed));
        let mut style = Style::new(ui, zoom, palette);
        style.cursor = cursor_line.map(|line| (line, palette.cursor_block));
        if self.pending_top_line.is_some() {
            ui.set_invisible();
        }
        let mut scroll = egui::ScrollArea::vertical()
            .id_salt(("md-preview", self.id))
            .auto_shrink(false);
        if let Some(y) = self.pending_offset.take() {
            scroll = scroll.vertical_scroll_offset(y);
        }
        let positions = &mut self.positions;
        let output = scroll.show(ui, |ui| {
            let origin = ui.cursor().top();
            egui::Frame::new().inner_margin(MARGIN).show(ui, |ui| {
                ui.set_width(ui.available_width());
                let top = ui.cursor().top() - origin;
                *positions = show_blocks(ui, &parsed.blocks, &style);
                for p in positions.iter_mut() {
                    p.top += top;
                }
            });
        });
        self.offset_y = output.state.clamped_offset().y;
        self.view_height = output.inner_rect.height();
        self.max_offset = (output.content_size.y - output.inner_rect.height()).max(0.0);
        if let Some(line) = self.pending_top_line.take() {
            self.pending_offset = Some(line_to_y(&self.positions, line));
            ui.ctx().request_repaint();
        }

        // 블록 더블클릭 → 그 블록의 첫 소스 줄 (P3-08)
        let view = output.inner_rect;
        let content_top = view.top() - self.offset_y;
        let double_clicked_line = ui
            .input(|i| {
                i.pointer
                    .button_double_clicked(egui::PointerButton::Primary)
                    .then(|| i.pointer.interact_pos())
                    .flatten()
            })
            .filter(|pos| view.contains(*pos))
            .and_then(|pos| {
                let y = pos.y - content_top;
                self.positions
                    .iter()
                    .find(|p| p.top <= y && y < p.top + p.height)
                    .map(|p| p.first_line)
            });
        PreviewOutput {
            double_clicked_line,
        }
    }
}

/// [`PreviewState::show`] 결과.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PreviewOutput {
    /// 이번 프레임에 더블클릭한 블록의 첫 소스 줄 (0부터)
    pub double_clicked_line: Option<usize>,
}

/// 소스 줄(0부터, 소수 = 줄 안 위치) → 미리보기 스크롤 내용 y.
///
/// 블록 안은 줄 비율대로, 블록 사이(빈 줄)는 앞 블록 끝과 다음 블록 위 사이를 선형 보간한다.
pub fn line_to_y(positions: &[BlockPos], line: f32) -> f32 {
    let Some(last) = positions.last() else {
        return 0.0;
    };
    let i = positions.partition_point(|p| ((p.last_line + 1) as f32) <= line);
    let Some(p) = positions.get(i) else {
        return last.top + last.height;
    };
    let first = p.first_line as f32;
    if line >= first {
        let lines = (p.last_line + 1 - p.first_line) as f32;
        return p.top + p.height * (line - first) / lines;
    }
    let (gap_line, gap_y) = match i.checked_sub(1).map(|j| positions[j]) {
        Some(prev) => ((prev.last_line + 1) as f32, prev.top + prev.height),
        None => (0.0, 0.0),
    };
    lerp_gap(line, gap_line, first, gap_y, p.top)
}

/// 미리보기 스크롤 내용 y → 소스 줄 ([`line_to_y`]의 역).
pub fn y_to_line(positions: &[BlockPos], y: f32) -> f32 {
    let Some(last) = positions.last() else {
        return 0.0;
    };
    let i = positions.partition_point(|p| p.top + p.height <= y);
    let Some(p) = positions.get(i) else {
        return (last.last_line + 1) as f32;
    };
    if y >= p.top {
        let lines = (p.last_line + 1 - p.first_line) as f32;
        let ratio = if p.height > 0.0 {
            (y - p.top) / p.height
        } else {
            0.0
        };
        return p.first_line as f32 + lines * ratio;
    }
    let (gap_line, gap_y) = match i.checked_sub(1).map(|j| positions[j]) {
        Some(prev) => ((prev.last_line + 1) as f32, prev.top + prev.height),
        None => (0.0, 0.0),
    };
    lerp_gap(y, gap_y, p.top, gap_line, p.first_line as f32)
}

/// `x`를 `[x0, x1]` → `[y0, y1]`로 선형 변환 (구간 길이가 0이면 `y1`).
fn lerp_gap(x: f32, x0: f32, x1: f32, y0: f32, y1: f32) -> f32 {
    if x1 > x0 {
        y0 + (y1 - y0) * ((x - x0) / (x1 - x0)).clamp(0.0, 1.0)
    } else {
        y1
    }
}

/// 크기·색 규칙.
struct Style {
    body: f32,
    line: f32,
    code_bg: Color32,
    /// (커서 소스 줄, 강조 색) — 이 줄을 포함한 최상위 블록 뒤에 배경을 깐다
    cursor: Option<(usize, Color32)>,
}

impl Style {
    fn new(ui: &egui::Ui, zoom: f32, palette: &Palette) -> Self {
        let size =
            |s: TextStyle, default: f32| ui.style().text_styles.get(&s).map_or(default, |f| f.size);
        let body = size(TextStyle::Body, 14.0) * zoom;
        Self {
            body,
            line: ui.text_style_height(&TextStyle::Body) * zoom,
            code_bg: palette.preview_code_bg,
            cursor: None,
        }
    }

    /// 제목 크기: 본문 크기의 배율 (GitHub 미리보기에 가깝게 H1 = 2배, P3-02 사용자 요청).
    fn heading_size(&self, level: u8) -> f32 {
        let factor = match level {
            1 => 2.0,
            2 => 1.6,
            3 => 1.35,
            4 => 1.2,
            5 => 1.1,
            _ => 1.05,
        };
        self.body * factor
    }
}

/// 최상위 블록을 차례로 그리고 위치표를 돌려준다.
fn show_blocks(ui: &mut egui::Ui, blocks: &[Block], style: &Style) -> Vec<BlockPos> {
    let origin = ui.cursor().top();
    let mut positions = Vec::with_capacity(blocks.len());
    for (index, block) in blocks.iter().enumerate() {
        if index > 0 {
            // 블록 사이 한 줄 여백, 제목 앞은 조금 더
            let extra = if matches!(block.kind, BlockKind::Heading { .. }) {
                0.6
            } else {
                0.0
            };
            ui.add_space(style.line * (0.8 + extra));
        }
        // 커서 블록 배경은 글자 뒤에 깔리도록 자리를 먼저 잡아 둔다
        let background = ui.painter().add(egui::Shape::Noop);
        let rect = ui
            .scope(|ui| {
                ui.set_width(ui.available_width());
                render_block(ui, block, style, 0);
            })
            .response
            .rect;
        if let Some((line, color)) = style.cursor
            && (block.first_line..=block.last_line).contains(&line)
        {
            let pad = MARGIN as f32 / 2.0;
            ui.painter().set(
                background,
                egui::Shape::rect_filled(rect.expand2(egui::vec2(pad, pad / 2.0)), 4.0, color),
            );
        }
        positions.push(BlockPos {
            first_line: block.first_line,
            last_line: block.last_line,
            top: rect.top() - origin,
            height: rect.height(),
        });
    }
    positions
}

/// 하위 블록 목록 (인용·목록 항목·각주 안). `gap`은 블록 사이 여백(줄 높이 배수).
fn render_children(ui: &mut egui::Ui, blocks: &[Block], style: &Style, depth: usize, gap: f32) {
    for (i, child) in blocks.iter().enumerate() {
        if i > 0 {
            ui.add_space(style.line * gap);
        }
        render_block(ui, child, style, depth);
    }
}

/// 블록 하나. `depth`는 목록 중첩 깊이(글머리 모양).
fn render_block(ui: &mut egui::Ui, block: &Block, style: &Style, depth: usize) {
    match &block.kind {
        BlockKind::Heading { level, text, .. } => {
            // H1~H4는 진한 색, H5·H6은 본문 색 (A와 같음)
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
        BlockKind::List { start, items } => list(ui, *start, items, style, depth),
        BlockKind::Quote(children) => quote(ui, children, style, depth),
        BlockKind::Code { lang, text } => {
            egui::Frame::new()
                .fill(style.code_bg)
                .corner_radius(4)
                .inner_margin(8)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    if !lang.is_empty() {
                        ui.label(RichText::new(lang).size(style.body * 0.85).weak());
                    }
                    egui::ScrollArea::horizontal()
                        .id_salt(ui.next_auto_id())
                        .show(ui, |ui| {
                            ui.label(RichText::new(text).font(FontId::monospace(style.body)));
                        });
                });
        }
        BlockKind::Table { align, head, rows } => table(ui, align, head, rows, style),
        BlockKind::Rule => {
            ui.add(egui::Separator::default().horizontal());
        }
        BlockKind::Html(html) => {
            ui.label(
                RichText::new(html)
                    .font(FontId::monospace(style.body))
                    .weak(),
            );
        }
        BlockKind::FrontMatter(yaml) => {
            egui::Frame::new()
                .fill(style.code_bg)
                .corner_radius(4)
                .inner_margin(8)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new("Front Matter").size(style.body * 0.85).weak());
                    ui.label(
                        RichText::new(yaml)
                            .font(FontId::monospace(style.body))
                            .weak(),
                    );
                });
        }
        BlockKind::Footnote { label, blocks } => {
            ui.horizontal_top(|ui| {
                ui.label(
                    RichText::new(format!("{label}."))
                        .size(style.body * 0.85)
                        .weak(),
                );
                ui.vertical(|ui| render_children(ui, blocks, style, depth, 0.4));
            });
        }
    }
}

/// 인용: 왼쪽 3px 세로선 + 들여쓰기 + 흐린 글자. 중첩 인용은 선이 하나씩 늘어난다.
fn quote(ui: &mut egui::Ui, children: &[Block], style: &Style, depth: usize) {
    let line_shape = ui.painter().add(egui::Shape::Noop);
    let rect = egui::Frame::new()
        .outer_margin(egui::Margin {
            left: 4,
            ..Default::default()
        })
        .inner_margin(egui::Margin {
            left: 14,
            ..Default::default()
        })
        .show(ui, |ui| {
            ui.visuals_mut().override_text_color = Some(ui.visuals().weak_text_color());
            render_children(ui, children, style, depth, 0.5);
        })
        .response
        .rect;
    let x = rect.left() + 4.0 + 1.5;
    ui.painter().set(
        line_shape,
        egui::Shape::line_segment(
            [
                egui::pos2(x, rect.top() + 2.0),
                egui::pos2(x, rect.bottom() - 2.0),
            ],
            Stroke::new(3.0, ui.visuals().weak_text_color().gamma_multiply(0.6)),
        ),
    );
}

/// 목록: 글머리(공백 4칸 폭) + 항목 내용. 비순서 글머리는 깊이에 따라 ● ○ ■ 순서.
fn list(ui: &mut egui::Ui, start: Option<u64>, items: &[ListItem], style: &Style, depth: usize) {
    let space = ui.fonts_mut(|f| f.glyph_width(&FontId::proportional(style.body), ' '));
    let marker_width = space * 4.0;
    // 번호가 길어지면(예: 100.) 글머리 칸을 넓힌다
    let number_width = start.map_or(0.0, |s| {
        let last = s + items.len().saturating_sub(1) as u64;
        ui.fonts_mut(|f| {
            f.layout_no_wrap(
                format!("{last}."),
                FontId::proportional(style.body),
                Color32::WHITE,
            )
            .size()
            .x
        }) + space
    });
    let width = marker_width.max(number_width);
    for (n, item) in items.iter().enumerate() {
        if n > 0 {
            ui.add_space(style.line * 0.2);
        }
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            let (rect, _) = ui.allocate_exact_size(egui::vec2(width, style.line), Sense::hover());
            let color = ui
                .visuals()
                .override_text_color
                .unwrap_or_else(|| ui.visuals().strong_text_color());
            let painter = ui.painter();
            match (item.task, start) {
                (Some(checked), _) => checkbox(painter, rect, style, checked, color),
                (None, Some(s)) => {
                    painter.text(
                        rect.right_center() - egui::vec2(space, 0.0),
                        egui::Align2::RIGHT_CENTER,
                        format!("{}.", s + n as u64),
                        FontId::proportional(style.body),
                        color,
                    );
                }
                (None, None) => {
                    let r = style.body / 6.0;
                    let c = rect.center();
                    match depth {
                        0 => {
                            painter.circle_filled(c, r, color);
                        }
                        1 => {
                            painter.circle_stroke(c, r, Stroke::new(1.0, color));
                        }
                        _ => {
                            let square = egui::vec2(r * 1.8, r * 1.8);
                            painter.rect_filled(
                                egui::Rect::from_center_size(c, square),
                                0.0,
                                color,
                            );
                        }
                    }
                }
            }
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                render_children(ui, &item.blocks, style, depth + 1, 0.4);
            });
        });
    }
}

/// 작업 목록 체크박스 (표시 전용, §3.8-4).
fn checkbox(
    painter: &egui::Painter,
    rect: egui::Rect,
    style: &Style,
    checked: bool,
    color: Color32,
) {
    let size = style.body * 0.85;
    let box_rect = egui::Rect::from_center_size(rect.center(), egui::vec2(size, size));
    painter.rect_stroke(
        box_rect,
        2.0,
        Stroke::new(1.0, color),
        egui::StrokeKind::Inside,
    );
    if checked {
        let r = box_rect.shrink(size * 0.2);
        painter.line(
            vec![
                egui::pos2(r.left(), r.center().y),
                egui::pos2(r.left() + r.width() * 0.4, r.bottom()),
                egui::pos2(r.right(), r.top()),
            ],
            Stroke::new(1.6, color),
        );
    }
}

/// 표: 테두리 틀 + 줄무늬 격자, 넓으면 표 영역만 가로 스크롤 (P3-03에서 다듬는다).
fn table(
    ui: &mut egui::Ui,
    align: &[markdown::Align],
    head: &[Inlines],
    rows: &[Vec<Inlines>],
    style: &Style,
) {
    egui::Frame::group(ui.style()).show(ui, |ui| {
        // 가로 스크롤바가 마지막 행을 가리지 않도록 아래 여백을 둔다.
        ui.spacing_mut().scroll.content_margin.bottom = ui.spacing().scroll.bar_width as i8;
        egui::ScrollArea::horizontal()
            .id_salt(ui.next_auto_id())
            .show(ui, |ui| {
                egui::Grid::new(ui.next_auto_id())
                    .striped(true)
                    .show(ui, |ui| {
                        for (c, cell) in head.iter().enumerate() {
                            table_cell(ui, cell, align.get(c), true, style);
                        }
                        ui.end_row();
                        for row in rows {
                            for (c, cell) in row.iter().enumerate() {
                                table_cell(ui, cell, align.get(c), false, style);
                            }
                            ui.end_row();
                        }
                    });
            });
    });
}

fn table_cell(
    ui: &mut egui::Ui,
    cell: &Inlines,
    align: Option<&markdown::Align>,
    head: bool,
    style: &Style,
) {
    let layout = match align {
        Some(markdown::Align::Center) => Layout::top_down(Align::Center),
        Some(markdown::Align::Right) => Layout::top_down(Align::Max),
        _ => Layout::top_down(Align::Min),
    };
    ui.with_layout(layout, |ui| {
        let mut job = inline_job(ui, cell, style.body, head, style);
        // 첫 프레임의 좁은 열 폭에 갇히지 않게 셀 안에서는 줄바꿈하지 않는다.
        job.wrap.max_width = f32::INFINITY;
        ui.add(egui::Label::new(job).extend());
    });
}

/// 인라인 스팬 → `LayoutJob`. `all_strong`이면 전체를 진한 색으로(제목·표 머리글).
fn inline_job(
    ui: &egui::Ui,
    spans: &Inlines,
    size: f32,
    all_strong: bool,
    style: &Style,
) -> LayoutJob {
    let visuals = ui.visuals();
    let text_color = visuals.override_text_color.unwrap_or(visuals.text_color());
    let weak = visuals.weak_text_color();
    let mut job = LayoutJob::default();
    for span in spans {
        let s = span.style;
        let strong = s.strong || all_strong;
        let linked = span.link.is_some();
        let mut format = TextFormat {
            font_id: FontId::proportional(size),
            color: if linked {
                visuals.hyperlink_color
            } else if strong && visuals.override_text_color.is_none() {
                visuals.strong_text_color()
            } else {
                text_color
            },
            italics: s.emph,
            ..Default::default()
        };
        if s.strike {
            format.strikethrough = Stroke::new(1.0, format.color);
        }
        if linked {
            format.underline = Stroke::new(1.0, visuals.hyperlink_color);
        }
        let text: std::borrow::Cow<'_, str> = match &span.kind {
            SpanKind::Text => span.text.as_str().into(),
            SpanKind::Code => {
                // 인라인 코드는 본문 크기의 고정폭 + 배경
                format.font_id = FontId::new(style.body.min(size), FontFamily::Monospace);
                format.background = style.code_bg;
                span.text.as_str().into()
            }
            SpanKind::Html => {
                // 원시 HTML은 실행하지 않고 회색 원문으로, `<br>`만 줄바꿈 (§3.8-4)
                if is_br(&span.text) {
                    "\n".into()
                } else {
                    format.font_id = FontId::new(style.body.min(size), FontFamily::Monospace);
                    format.color = weak;
                    span.text.as_str().into()
                }
            }
            SpanKind::Image { .. } => {
                // 이미지 표시는 P3-04 — 지금은 대체 텍스트 자리 표시
                format.color = weak;
                format!("[이미지: {}]", span.text).into()
            }
            SpanKind::FootnoteRef => {
                format.font_id = FontId::proportional(size * 0.75);
                format.color = visuals.hyperlink_color;
                format.valign = Align::TOP;
                format!("[{}]", span.text).into()
            }
        };
        job.append(&text, 0.0, format);
    }
    job.wrap.max_width = ui.available_width();
    job
}

/// `<br>`, `<br/>`, `<br />` (대소문자 무시).
fn is_br(html: &str) -> bool {
    let tag: String = html
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_ascii_lowercase();
    tag == "<br>" || tag == "<br/>"
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{Pos2, RawInput, Rect};

    /// 헤드리스 프레임: 600×800 창에 미리보기를 그리고 그려진 글자들을 돌려준다.
    fn frame(ctx: &egui::Context, state: &mut PreviewState, doc: &Document) -> Vec<String> {
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(600.0, 800.0))),
            ..Default::default()
        };
        let mut full = ctx.run_ui(raw, |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                state.show(ui, doc, 1.0, &Palette::light(), None);
            });
        });
        full.textures_delta.clear();
        full.shapes
            .iter()
            .filter_map(|c| match &c.shape {
                egui::Shape::Text(t) => Some(t.galley.text().to_owned()),
                _ => None,
            })
            .collect()
    }

    fn doc(text: &str) -> Document {
        let mut d = Document::new_untitled(1);
        d.set_text(text);
        d
    }

    #[test]
    fn renders_blocks_and_records_monotonic_positions() {
        let ctx = egui::Context::default();
        let mut state = PreviewState::new(1);
        let d = doc("# 제목\n\n문단 **굵게**\n\n- 하나\n  - 둘\n\n3. 셋\n\n> 인용\n\n---\n");
        let texts = frame(&ctx, &mut state, &d);
        assert!(texts.iter().any(|t| t == "제목"));
        assert!(texts.iter().any(|t| t == "문단 굵게"));
        assert!(texts.iter().any(|t| t == "3."));
        assert!(texts.iter().any(|t| t == "인용"));
        let pos = state.positions();
        assert_eq!(pos.len(), 6);
        assert!(pos.windows(2).all(|w| w[1].top > w[0].top));
        assert_eq!((pos[2].first_line, pos[2].last_line), (4, 5));
    }

    #[test]
    fn reparses_only_on_revision_change() {
        let ctx = egui::Context::default();
        let mut state = PreviewState::new(1);
        let mut d = doc("# 하나\n");
        frame(&ctx, &mut state, &d);
        frame(&ctx, &mut state, &d);
        assert_eq!(state.parse_count(), 1);
        d.set_text("# 둘\n");
        let texts = frame(&ctx, &mut state, &d);
        assert_eq!(state.parse_count(), 2);
        assert!(texts.iter().any(|t| t == "둘"));
    }

    #[test]
    fn ordered_list_numbers_continue_from_start() {
        let ctx = egui::Context::default();
        let mut state = PreviewState::new(1);
        let texts = frame(&ctx, &mut state, &doc("9. 아홉\n10. 열\n"));
        assert!(texts.iter().any(|t| t == "9."));
        assert!(texts.iter().any(|t| t == "10."));
    }

    fn pos(first_line: usize, last_line: usize, top: f32, height: f32) -> BlockPos {
        BlockPos {
            first_line,
            last_line,
            top,
            height,
        }
    }

    #[test]
    fn line_and_y_mapping_round_trips() {
        // 줄 0~1 블록(16~56), 빈 줄 2, 줄 3 블록(70~90), 빈 줄 4, 줄 5~9 블록(110~210)
        let p = [
            pos(0, 1, 16.0, 40.0),
            pos(3, 3, 70.0, 20.0),
            pos(5, 9, 110.0, 100.0),
        ];
        assert_eq!(line_to_y(&p, 0.0), 16.0);
        assert_eq!(line_to_y(&p, 1.0), 36.0);
        assert_eq!(line_to_y(&p, 2.5), 63.0);
        assert_eq!(line_to_y(&p, 7.0), 150.0);
        assert_eq!(line_to_y(&p, 12.0), 210.0);
        for line in [0.0, 0.5, 1.5, 2.25, 3.0, 4.5, 6.0, 9.9] {
            let back = y_to_line(&p, line_to_y(&p, line));
            assert!((back - line).abs() < 1e-3, "{line} → {back}");
        }
        // 맨 위 여백(0~16)은 0번 줄
        assert_eq!(y_to_line(&p, 5.0), 0.0);
        assert_eq!(y_to_line(&p, 500.0), 10.0);
        assert_eq!(line_to_y(&[], 3.0), 0.0);
    }

    #[test]
    fn scroll_to_line_takes_effect_next_frame() {
        let ctx = egui::Context::default();
        let mut state = PreviewState::new(1);
        let text: String = (0..200)
            .map(|i| {
                format!(
                    "문단 {i}

"
                )
            })
            .collect();
        let d = doc(&text);
        frame(&ctx, &mut state, &d);
        assert_eq!(state.top_line(), Some(0.0));
        state.scroll_to_line(100.0);
        frame(&ctx, &mut state, &d);
        frame(&ctx, &mut state, &d);
        let top = state.top_line().unwrap();
        assert!((top - 100.0).abs() < 1.0, "맨 위 줄 {top}");
    }

    #[test]
    fn cursor_line_block_gets_background() {
        let ctx = egui::Context::default();
        let mut state = PreviewState::new(1);
        let d = doc("# 제목

첫 문단
둘째 줄

끝
");
        let color = Palette::light().cursor_block;
        let backgrounds = |state: &mut PreviewState, line| {
            let raw = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(600.0, 800.0))),
                ..Default::default()
            };
            let mut full = ctx.run_ui(raw, |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    state.show(ui, &d, 1.0, &Palette::light(), line);
                });
            });
            full.textures_delta.clear();
            full.shapes
                .iter()
                .filter(|c| matches!(&c.shape, egui::Shape::Rect(r) if r.fill == color))
                .count()
        };
        // 줄 3(0부터)은 둘째 블록(줄 2~3) 안
        assert_eq!(backgrounds(&mut state, Some(3)), 1);
        // 블록 사이 빈 줄은 강조 없음, 커서 정보가 없어도 없음
        assert_eq!(backgrounds(&mut state, Some(4)), 0);
        assert_eq!(backgrounds(&mut state, None), 0);
    }

    #[test]
    fn br_tag_variants() {
        assert!(is_br("<br>"));
        assert!(is_br("<BR />"));
        assert!(is_br("<br/>"));
        assert!(!is_br("<b>"));
    }
}
