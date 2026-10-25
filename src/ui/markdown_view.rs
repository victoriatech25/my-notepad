//! Markdown 미리보기 렌더러 (자체 블록 렌더링 — ADR 0002, P3-02).
//!
//! - [`core::markdown`](crate::core::markdown) 블록 모델을 egui 위젯으로 직접 그린다. 파싱은 탭별
//!   [`MarkdownCache`]가 문서 버전이 바뀔 때만 한다.
//! - 화면 규칙은 UX-004(P0-08)에서 정한 A(egui_commonmark) 모양을 따른다: 제목은 본문 크기의 단계별 배율
//!   (H1 = 2배) + 진한 색, 굵게는 진한 색(굵은 글꼴은 쓰지 않음 — ADR 0002), 블록 사이 한 줄 여백,
//!   인용은 왼쪽 3px 선 + 흐린 글자, 목록 글머리는 공백 4칸 폭 안에 그린다.
//! - 최상위 블록마다 위치([`BlockPos`])를 기록한다 — 스크롤 동기화(P3-08)의 기반.
//! - 가상 스크롤(P3-05): 블록 높이를 (블록 원문, 본문 폭·확대·테마)로 캐시하고, **화면에 보이는 블록만** 그린다.
//!   처음 보는 블록은 보이지 않는 측정용 UI에 한 번 그려 높이를 잰다 — 편집하면 바뀐 블록만 다시 잰다.
//!   보이는 블록을 실제로 그린 높이가 캐시와 다르면(이미지 크기·접기 상태 등) 캐시를 고치고 다시 그린다.
//!   1MB를 넘는 문서는 작업 스레드에서 파싱한다 ([`MarkdownCache::get_or_spawn`]).
//! - 코드 블록은 줄을 접지 않고 블록 안에서만 가로 스크롤, 표는 내용에 맞춘 열 폭·넘치면 열을 내용 폭에 비례해
//!   줄여 줄바꿈·그래도 넘치면 표만 가로 스크롤 (P3-03).
//! - 이미지(P3-04): 문단 안 이미지는 자기 줄에 그린다. 로컬 파일은 원본 크기(보이는 폭에 맞춰 축소)로,
//!   디코딩·텍스처는 [`ImageCache`]가 맡는다. 원격 이미지는 차단, 없는·읽지 못한 이미지는 파일 이름 + 깨진 이미지 아이콘.
//! - Front Matter는 접힌 메타데이터 블록, 각주 참조 번호를 누르면 각주로·각주의 `↑ 본문으로`를 누르면 참조로 이동,
//!   원시 HTML은 회색 원문(`<br>`만 줄바꿈), Mermaid·수식 코드 블록에는 "미리보기 미지원" 라벨.

use eframe::egui::{
    self, Align, Color32, FontFamily, FontId, RichText, Sense, Stroke, TextFormat, TextStyle,
    text::{CCursor, LayoutJob},
};

use std::cell::RefCell;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::path::Path;
use std::sync::Arc;

use crate::core::document::Document;
use crate::core::image_load::{self, ImageSource};
use crate::core::markdown::{
    self, Block, BlockKind, Inlines, ListItem, MarkdownCache, Span, SpanKind,
};
use crate::fonts::ICON_FAMILY;
use crate::ui::preview_images::{ImageCache, TextureState};
use crate::ui::theme::Palette;

/// Phosphor 아이콘: 깨진 이미지, 차단된 원격(구름에 빗금) (egui-phosphor 0.14 codepoints)
const ICON_IMAGE_BROKEN: &str = "\u{E7A8}";
const ICON_CLOUD_SLASH: &str = "\u{E1B6}";

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
    /// 이미지 텍스처 캐시 (P3-04)
    images: RefCell<ImageCache>,
    /// 블록 높이 캐시 (P3-05)
    heights: HeightCache,
}

/// 블록 높이 캐시: 블록 원문 해시 → 높이. 본문 폭·확대·테마가 바뀌면 비운다.
#[derive(Debug, Default)]
struct HeightCache {
    /// (본문 폭 px, 확대 배율 ×100, 다크 모드)
    layout: Option<(i32, i32, bool)>,
    heights: HashMap<u64, f32>,
    /// `keys`를 계산한 파싱 결과
    keys_for: Option<Arc<markdown::Document>>,
    /// 최상위 블록마다의 원문 해시
    keys: Vec<u64>,
    /// 마지막 프레임에 측정한 블록 수와 실제로 그린 블록 수
    measured: usize,
    rendered: usize,
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
            images: RefCell::new(ImageCache::default()),
            heights: HeightCache::default(),
        }
    }

    /// 마지막 프레임에 높이를 새로 잰 블록 수와 실제로 그린 블록 수 (가상 스크롤 확인용).
    pub fn last_frame_counts(&self) -> (usize, usize) {
        (self.heights.measured, self.heights.rendered)
    }

    /// 큰 문서를 작업 스레드에서 파싱 중인지.
    pub fn is_parsing(&self) -> bool {
        self.cache.is_parsing()
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

    /// 지금 올라가 있는 이미지 텍스처 수.
    pub fn image_texture_count(&self) -> usize {
        self.images.borrow().texture_count()
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
        let ctx = ui.ctx().clone();
        let parsed = self
            .cache
            .get_or_spawn(doc.revision(), doc.text(), move || ctx.request_repaint());
        self.parsed = Some(std::sync::Arc::clone(&parsed));
        self.images.borrow_mut().begin_frame(ui.ctx());
        let doc_dir = doc.path().and_then(Path::parent);
        let mut style = Style::new(ui, zoom, palette, doc_dir, &self.images, &parsed.footnotes);
        style.cursor = cursor_line.map(|line| (line, palette.cursor_block));
        if self.pending_top_line.is_some() {
            ui.set_invisible();
        }
        let mut scroll = egui::ScrollArea::vertical()
            .id_salt(("md-preview", self.id))
            .auto_shrink(false);
        // 스크롤 범위를 넘는 위치(문서 끝 각주로 이동 등)는 지난 프레임의 최대값으로 자른다 —
        // egui는 넘는 값을 그대로 써서 내용을 그 자리에 그린다
        if let Some(y) = self.pending_offset.take() {
            scroll = scroll.vertical_scroll_offset(y.min(self.max_offset));
        }
        let parsing = self.cache.is_parsing() && parsed.blocks.is_empty();
        let heights = &mut self.heights;
        let positions = &mut self.positions;
        let output = scroll.show_viewport(ui, |ui, viewport| {
            if parsing {
                ui.add_space(MARGIN as f32);
                ui.label(RichText::new("  큰 문서를 분석하는 중…").weak());
                positions.clear();
                return;
            }
            let width = (ui.available_width() - 2.0 * MARGIN as f32).max(1.0);
            heights.prepare(&parsed, doc, width, zoom, ui.visuals().dark_mode);
            heights.measure_missing(ui, &parsed.blocks, &style, width);
            *positions = heights.positions(&parsed.blocks, &style);
            let total = positions
                .last()
                .map_or(0.0, |p| p.top + p.height + MARGIN as f32);
            let content = ui.min_rect().min;
            ui.allocate_space(egui::vec2(ui.available_width(), total));
            let changed = heights.render_visible(
                ui,
                &parsed.blocks,
                positions,
                &style,
                content,
                width,
                viewport,
            );
            if changed {
                ui.ctx().request_repaint();
            }
        });
        self.offset_y = output.state.clamped_offset().y;
        self.view_height = output.inner_rect.height();
        self.max_offset = (output.content_size.y - output.inner_rect.height()).max(0.0);
        if let Some(line) = self.pending_top_line.take() {
            self.pending_offset = Some(line_to_y(&self.positions, line));
            ui.ctx().request_repaint();
        }

        self.images.borrow_mut().end_frame();

        // 블록 더블클릭 → 그 블록의 첫 소스 줄 (P3-08)
        let view = output.inner_rect;
        let content_top = view.top() - self.offset_y;

        // 각주 이동 (P3-04): 참조 → 각주 블록 위, 각주 → 처음 참조가 화면 위쪽 1/3에 오게
        if let Some(jump) = style.jump.take() {
            let target = match jump {
                Jump::ToFootnote(label) => parsed
                    .blocks
                    .iter()
                    .position(
                        |b| matches!(&b.kind, BlockKind::Footnote { label: l, .. } if *l == label),
                    )
                    .and_then(|i| self.positions.get(i))
                    .map(|p| p.top - MARGIN as f32),
                Jump::BackToRef(label) => parsed
                    .footnote_ref_line(&label)
                    .map(|line| line_to_y(&self.positions, line as f32) - view.height() / 3.0),
            };
            if let Some(y) = target {
                self.pending_offset = Some(y.max(0.0));
                ui.ctx().request_repaint();
            }
        }
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

/// 각주 이동 요청.
enum Jump {
    ToFootnote(String),
    BackToRef(String),
}

/// 크기·색 규칙과 한 프레임 동안 모으는 상태.
struct Style<'a> {
    body: f32,
    line: f32,
    /// 위젯 사이 세로 간격 (블록 사이에도 더한다)
    item_gap: f32,
    code_bg: Color32,
    warning: Color32,
    /// (커서 소스 줄, 강조 색) — 이 줄을 포함한 최상위 블록 뒤에 배경을 깐다
    cursor: Option<(usize, Color32)>,
    /// 문서가 있는 폴더 (상대 경로 이미지 기준, 저장하지 않은 문서면 `None`)
    doc_dir: Option<&'a Path>,
    images: &'a RefCell<ImageCache>,
    /// 각주 이름 (처음 참조한 순서 — 화면 번호 = 순서 + 1)
    footnotes: &'a [String],
    /// 이번 프레임에 누른 각주 이동
    jump: RefCell<Option<Jump>>,
}

impl<'a> Style<'a> {
    fn new(
        ui: &egui::Ui,
        zoom: f32,
        palette: &Palette,
        doc_dir: Option<&'a Path>,
        images: &'a RefCell<ImageCache>,
        footnotes: &'a [String],
    ) -> Self {
        let size =
            |s: TextStyle, default: f32| ui.style().text_styles.get(&s).map_or(default, |f| f.size);
        let body = size(TextStyle::Body, 14.0) * zoom;
        Self {
            body,
            line: ui.text_style_height(&TextStyle::Body) * zoom,
            item_gap: ui.spacing().item_spacing.y,
            code_bg: palette.preview_code_bg,
            warning: palette.status_warning,
            cursor: None,
            doc_dir,
            images,
            footnotes,
            jump: RefCell::new(None),
        }
    }

    /// 각주 화면 번호 문자열 (참조가 없으면 이름 그대로).
    fn footnote_mark(&self, label: &str) -> String {
        self.footnotes
            .iter()
            .position(|l| l == label)
            .map_or_else(|| label.to_owned(), |i| (i + 1).to_string())
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

/// 최상위 블록 `index` 앞 여백: 위젯 간격 + 한 줄 여백, 제목 앞은 조금 더 (가상 스크롤 전과 같은 배치).
fn gap_before(blocks: &[Block], index: usize, style: &Style) -> f32 {
    if index == 0 {
        return 0.0;
    }
    let extra = if matches!(blocks[index].kind, BlockKind::Heading { .. }) {
        0.6
    } else {
        0.0
    };
    style.item_gap + style.line * (0.8 + extra)
}

/// 블록 원문(소스 줄 범위의 글자)과 종류로 만든 높이 캐시 키.
fn block_key(doc: &Document, block: &Block) -> u64 {
    let index = doc.line_index();
    let text = doc.text();
    let start = index
        .line_start_byte(block.first_line)
        .unwrap_or(text.len());
    let end = index
        .line_start_byte(block.last_line + 1)
        .unwrap_or(text.len())
        .max(start);
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    block.kind.name().hash(&mut hasher);
    text.get(start..end).hash(&mut hasher);
    hasher.finish()
}

impl HeightCache {
    /// 본문 폭·확대·테마가 바뀌었으면 높이를 비우고, 파싱 결과가 바뀌었으면 블록 키를 다시 만든다.
    fn prepare(
        &mut self,
        parsed: &Arc<markdown::Document>,
        doc: &Document,
        width: f32,
        zoom: f32,
        dark: bool,
    ) {
        let layout = (width.round() as i32, (zoom * 100.0).round() as i32, dark);
        if self.layout != Some(layout) {
            self.layout = Some(layout);
            self.heights.clear();
        }
        if !self
            .keys_for
            .as_ref()
            .is_some_and(|d| Arc::ptr_eq(d, parsed))
        {
            self.keys = parsed.blocks.iter().map(|b| block_key(doc, b)).collect();
            self.keys_for = Some(Arc::clone(parsed));
        }
    }

    /// 높이를 모르는 블록을 보이지 않는 측정용 UI에 그려 높이를 잰다.
    fn measure_missing(&mut self, ui: &mut egui::Ui, blocks: &[Block], style: &Style, width: f32) {
        self.measured = 0;
        let missing: Vec<usize> = (0..blocks.len())
            .filter(|&i| !self.heights.contains_key(&self.keys[i]))
            .collect();
        if missing.is_empty() {
            return;
        }
        let area = egui::Rect::from_min_size(ui.min_rect().min, egui::vec2(width, f32::INFINITY));
        let mut measure = ui.new_child(
            egui::UiBuilder::new()
                .id_salt("md-measure")
                .max_rect(area)
                .invisible(),
        );
        for i in missing {
            let height = measure
                .push_id(("md-block", blocks[i].first_line), |ui| {
                    ui.set_width(width);
                    render_block(ui, &blocks[i], style, 0);
                })
                .response
                .rect
                .height();
            self.heights.insert(self.keys[i], height);
            self.measured += 1;
        }
    }

    /// 캐시된 높이로 전체 위치표를 만든다 (스크롤 내용 좌표, 바깥 여백 포함).
    fn positions(&self, blocks: &[Block], style: &Style) -> Vec<BlockPos> {
        let mut y = MARGIN as f32;
        blocks
            .iter()
            .enumerate()
            .map(|(i, block)| {
                y += gap_before(blocks, i, style);
                let height = self
                    .heights
                    .get(&self.keys[i])
                    .copied()
                    .unwrap_or(style.line);
                let pos = BlockPos {
                    first_line: block.first_line,
                    last_line: block.last_line,
                    top: y,
                    height,
                };
                y += height;
                pos
            })
            .collect()
    }

    /// `viewport`(스크롤 내용 좌표)와 겹치는 블록만 그린다. 실제 높이가 캐시와 다르면 고치고 `true`.
    #[allow(clippy::too_many_arguments)]
    fn render_visible(
        &mut self,
        ui: &mut egui::Ui,
        blocks: &[Block],
        positions: &[BlockPos],
        style: &Style,
        content: egui::Pos2,
        width: f32,
        viewport: egui::Rect,
    ) -> bool {
        let first = positions.partition_point(|p| p.top + p.height < viewport.min.y);
        let last = positions.partition_point(|p| p.top <= viewport.max.y);
        let mut changed = false;
        self.rendered = 0;
        for i in first..last.max(first) {
            let (block, pos) = (&blocks[i], positions[i]);
            let min = content + egui::vec2(MARGIN as f32, pos.top);
            let area = egui::Rect::from_min_size(min, egui::vec2(width, f32::INFINITY));
            // 커서 블록 배경은 글자 뒤에 깔리도록 자리를 먼저 잡아 둔다
            let background = ui.painter().add(egui::Shape::Noop);
            let mut child = ui.new_child(
                egui::UiBuilder::new()
                    .id_salt(("md-block", block.first_line))
                    .max_rect(area),
            );
            child.set_width(width);
            render_block(&mut child, block, style, 0);
            let rect = child.min_rect();
            self.rendered += 1;
            if let Some((line, color)) = style.cursor
                && (block.first_line..=block.last_line).contains(&line)
            {
                let pad = MARGIN as f32 / 2.0;
                ui.painter().set(
                    background,
                    egui::Shape::rect_filled(rect.expand2(egui::vec2(pad, pad / 2.0)), 4.0, color),
                );
            }
            if (rect.height() - pos.height).abs() > 0.5 {
                self.heights.insert(self.keys[i], rect.height());
                changed = true;
            }
        }
        changed
    }
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
            inline_text(ui, text, style.heading_size(*level), strong, style);
        }
        BlockKind::Paragraph(text) => inline_text(ui, text, style.body, false, style),
        BlockKind::List { start, items } => list(ui, *start, items, style, depth),
        BlockKind::Quote(children) => quote(ui, children, style, depth),
        BlockKind::Code { lang, text } => code_block(ui, lang, text, style, block.first_line),
        BlockKind::Table { align, head, rows } => {
            table(ui, align, head, rows, style, block.first_line);
        }
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
            // 본문 대신 접힌 메타데이터 블록 (§3.8-4)
            egui::Frame::new()
                .fill(style.code_bg)
                .corner_radius(4)
                .inner_margin(egui::Margin::symmetric(8, 4))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    let title = format!("Front Matter · {}줄", yaml.lines().count());
                    egui::CollapsingHeader::new(RichText::new(title).size(style.body * 0.9).weak())
                        .id_salt(("md-front-matter", block.first_line))
                        .default_open(false)
                        .show(ui, |ui| {
                            ui.label(
                                RichText::new(yaml)
                                    .font(FontId::monospace(style.body))
                                    .weak(),
                            );
                        });
                });
        }
        BlockKind::Footnote { label, blocks } => footnote(ui, label, blocks, style, depth),
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

/// 코드 블록: 고정폭 + 배경, 줄은 접지 않고 긴 줄은 블록 안에서만 가로 스크롤 (P3-03).
fn code_block(ui: &mut egui::Ui, lang: &str, text: &str, style: &Style, salt: usize) {
    egui::Frame::new()
        .fill(style.code_bg)
        .corner_radius(4)
        .inner_margin(8)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            if !lang.is_empty() {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(lang).size(style.body * 0.85).weak());
                    // Mermaid·수식은 그리지 않고 코드로 보여 준다 (§3.8-4, 2차 목표 검토)
                    if is_unsupported_diagram(lang) {
                        ui.label(
                            RichText::new("미리보기 미지원")
                                .size(style.body * 0.85)
                                .color(style.warning),
                        );
                    }
                });
            }
            // 가로 스크롤바가 마지막 줄을 가리지 않도록 아래 여백을 둔다.
            ui.spacing_mut().scroll.content_margin.bottom = ui.spacing().scroll.bar_width as i8;
            egui::ScrollArea::horizontal()
                .id_salt(("md-code", salt))
                .show(ui, |ui| {
                    let color = ui
                        .visuals()
                        .override_text_color
                        .unwrap_or(ui.visuals().text_color());
                    let job = LayoutJob::simple(
                        text.to_owned(),
                        FontId::monospace(style.body),
                        color,
                        f32::INFINITY,
                    );
                    ui.add(egui::Label::new(job).extend());
                });
        });
}

/// 표 셀 안쪽 여백 (가로, 세로).
const CELL_PAD: egui::Vec2 = egui::vec2(10.0, 3.0);
/// 표가 보이는 폭을 넘을 때 열을 좁히는 하한 (이보다 좁히지 않고 가로 스크롤).
const MIN_COLUMN_WIDTH: f32 = 60.0;

/// 표: 열 폭은 내용에 맞추고, 표가 보이는 폭을 넘으면 열을 내용 폭에 비례해 줄여 줄바꿈한다.
/// 그래도 넘치면(열이 많을 때) 표 영역만 가로 스크롤한다. 머리글은 배경 + 진한 글자, 본문은 줄무늬, 칸마다 테두리.
fn table(
    ui: &mut egui::Ui,
    align: &[markdown::Align],
    head: &[Inlines],
    rows: &[Vec<Inlines>],
    style: &Style,
    salt: usize,
) {
    let columns = rows.iter().map(Vec::len).fold(head.len(), usize::max);
    if columns == 0 {
        return;
    }
    let empty = Inlines::new();
    // 0번 행 = 머리글
    let cell = |row: usize, col: usize| -> &Inlines {
        let cells = if row == 0 { head } else { &rows[row - 1][..] };
        cells.get(col).unwrap_or(&empty)
    };
    let row_count = rows.len() + 1;

    // 열의 내용 폭 = 줄바꿈 없이 가장 넓은 칸
    let natural: Vec<f32> = (0..columns)
        .map(|c| {
            (0..row_count)
                .map(|r| {
                    cell_galley(ui, cell(r, c), r == 0, style, f32::INFINITY)
                        .size()
                        .x
                })
                .fold(0.0, f32::max)
        })
        .collect();
    let caps = column_widths(&natural, ui.available_width(), CELL_PAD.x * 2.0 + 1.0);

    let galleys: Vec<Vec<Arc<egui::Galley>>> = (0..row_count)
        .map(|r| {
            (0..columns)
                .map(|c| cell_galley(ui, cell(r, c), r == 0, style, caps[c]))
                .collect()
        })
        .collect();
    let widths: Vec<f32> = (0..columns)
        .map(|c| {
            let text = galleys
                .iter()
                .map(|row| row[c].size().x)
                .fold(0.0, f32::max);
            text.min(caps[c]).ceil() + CELL_PAD.x * 2.0
        })
        .collect();
    let heights: Vec<f32> = galleys
        .iter()
        .map(|row| {
            let text = row.iter().map(|g| g.size().y).fold(style.line, f32::max);
            text.ceil() + CELL_PAD.y * 2.0
        })
        .collect();
    let size = egui::vec2(widths.iter().sum(), heights.iter().sum());

    // 가로 스크롤바가 마지막 행을 가리지 않도록 아래 여백을 둔다.
    ui.spacing_mut().scroll.content_margin.bottom = ui.spacing().scroll.bar_width as i8;
    egui::ScrollArea::horizontal()
        .id_salt(("md-table", salt))
        .show(ui, |ui| {
            let (rect, _) = ui.allocate_exact_size(size + egui::vec2(1.0, 1.0), Sense::hover());
            let outer = egui::Rect::from_min_size(rect.min, size);
            let mut y = rect.top();
            for (r, &h) in heights.iter().enumerate() {
                let row_rect =
                    egui::Rect::from_min_size(egui::pos2(rect.left(), y), egui::vec2(size.x, h));
                if r == 0 {
                    ui.painter().rect_filled(row_rect, 0.0, style.code_bg);
                } else if r % 2 == 0 {
                    let stripe = style.code_bg.gamma_multiply(0.45);
                    ui.painter().rect_filled(row_rect, 0.0, stripe);
                }
                let mut x = rect.left();
                for (c, &w) in widths.iter().enumerate() {
                    let galley = &galleys[r][c];
                    let inner = w - CELL_PAD.x * 2.0;
                    let dx = align_offset(align.get(c).copied(), inner, galley.size().x);
                    let pos = egui::pos2(x + CELL_PAD.x + dx, y + CELL_PAD.y);
                    // 글자는 선택·복사되도록 라벨로 놓는다
                    ui.put(
                        egui::Rect::from_min_size(pos, galley.size()),
                        egui::Label::new(Arc::clone(galley)),
                    );
                    x += w;
                }
                y += h;
            }
            // 테두리: 바깥 틀 + 행·열 구분선
            let painter = ui.painter();
            let border = ui.visuals().widgets.noninteractive.bg_stroke;
            painter.rect_stroke(outer, 0.0, border, egui::StrokeKind::Inside);
            let mut y = rect.top();
            for &h in &heights[..heights.len() - 1] {
                y += h;
                painter.hline(outer.x_range(), y, border);
            }
            let mut x = rect.left();
            for &w in &widths[..widths.len() - 1] {
                x += w;
                painter.vline(x, outer.y_range(), border);
            }
        });
}

/// 셀 내용 레이아웃. `wrap`은 줄바꿈 폭(무한대면 줄바꿈 없음).
fn cell_galley(
    ui: &egui::Ui,
    cell: &Inlines,
    head: bool,
    style: &Style,
    wrap: f32,
) -> Arc<egui::Galley> {
    let mut job = inline_job(ui, cell, style.body, head, style);
    job.wrap.max_width = wrap;
    ui.fonts_mut(|f| f.layout_job(job))
}

/// 열마다 글자 영역의 최대 폭. 다 들어가면 내용 폭 그대로, 넘치면 **내용 폭에 비례해** 줄인다
/// (칸마다 접히는 줄 수가 비슷해져 행 높이가 낮다 — 브라우저 자동 표 레이아웃과 같은 방향).
/// 열 하나는 [`MIN_COLUMN_WIDTH`]와 내용 폭 중 작은 값보다 좁히지 않으며, 그래도 넘치면 가로 스크롤한다.
/// `extra`는 열마다 더해지는 여백·테두리.
fn column_widths(natural: &[f32], available: f32, extra: f32) -> Vec<f32> {
    let n = natural.len();
    let budget = available - extra * n as f32;
    if natural.iter().sum::<f32>() <= budget {
        return natural.to_vec();
    }
    let floor = |i: usize| natural[i].min(MIN_COLUMN_WIDTH);
    // 비례로 줄이면 하한보다 좁아지는 열은 하한에 고정하고, 남은 폭을 나머지 열에 다시 비례 배분한다.
    let mut fixed = vec![false; n];
    loop {
        let fixed_sum: f32 = (0..n).filter(|&i| fixed[i]).map(floor).sum();
        let free_sum: f32 = (0..n).filter(|&i| !fixed[i]).map(|i| natural[i]).sum();
        let scale = if free_sum > 0.0 {
            ((budget - fixed_sum) / free_sum).max(0.0)
        } else {
            0.0
        };
        let mut changed = false;
        for i in 0..n {
            if !fixed[i] && natural[i] * scale < floor(i) {
                fixed[i] = true;
                changed = true;
            }
        }
        if !changed {
            return (0..n)
                .map(|i| {
                    if fixed[i] {
                        floor(i)
                    } else {
                        natural[i] * scale
                    }
                })
                .collect();
        }
    }
}

/// 셀 안 글자의 왼쪽 여백: 왼쪽·기본 0, 가운데는 남는 폭의 절반, 오른쪽은 남는 폭 전부.
fn align_offset(align: Option<markdown::Align>, inner: f32, text: f32) -> f32 {
    let spare = (inner - text).max(0.0);
    match align {
        Some(markdown::Align::Center) => (spare / 2.0).round(),
        Some(markdown::Align::Right) => spare,
        _ => 0.0,
    }
}

/// 인라인 스팬 → `LayoutJob`. `all_strong`이면 전체를 진한 색으로(제목·표 머리글).
fn inline_job(
    ui: &egui::Ui,
    spans: &[Span],
    size: f32,
    all_strong: bool,
    style: &Style,
) -> LayoutJob {
    inline_job_marked(ui, spans, size, all_strong, style).0
}

/// [`inline_job`]과 같고, 각주 참조의 (글자 범위, 각주 이름)을 함께 돌려준다.
fn inline_job_marked(
    ui: &egui::Ui,
    spans: &[Span],
    size: f32,
    all_strong: bool,
    style: &Style,
) -> (LayoutJob, Vec<(Range<usize>, String)>) {
    let visuals = ui.visuals();
    let text_color = visuals.override_text_color.unwrap_or(visuals.text_color());
    let weak = visuals.weak_text_color();
    let mut job = LayoutJob::default();
    let mut refs = Vec::new();
    let mut chars = 0;
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
                // 문단 밖(제목·표 칸)의 이미지는 대체 텍스트로 (문단 안 이미지는 [`inline_text`]가 그린다)
                format.color = weak;
                format!("[이미지: {}]", span.text).into()
            }
            SpanKind::FootnoteRef => {
                format.font_id = FontId::proportional(size * 0.75);
                format.color = visuals.hyperlink_color;
                format.valign = Align::TOP;
                format!("[{}]", style.footnote_mark(&span.text)).into()
            }
        };
        let len = text.chars().count();
        if span.kind == SpanKind::FootnoteRef {
            refs.push((chars..chars + len, span.text.clone()));
        }
        chars += len;
        job.append(&text, 0.0, format);
    }
    job.wrap.max_width = ui.available_width();
    (job, refs)
}

/// 제목·문단 글자. 이미지가 있으면 글자 덩어리와 이미지를 차례로 그린다 (이미지는 자기 줄).
fn inline_text(ui: &mut egui::Ui, spans: &[Span], size: f32, all_strong: bool, style: &Style) {
    let mut start = 0;
    for (i, span) in spans.iter().enumerate() {
        if let SpanKind::Image { url, title } = &span.kind {
            if start < i {
                rich_label(ui, &spans[start..i], size, all_strong, style);
            }
            image(ui, &span.text, url, title, style);
            start = i + 1;
        }
    }
    if start < spans.len() || spans.is_empty() {
        rich_label(ui, &spans[start..], size, all_strong, style);
    }
}

/// 글자 라벨. 각주 참조 번호는 눌러서 각주로 이동한다.
fn rich_label(ui: &mut egui::Ui, spans: &[Span], size: f32, all_strong: bool, style: &Style) {
    let (job, refs) = inline_job_marked(ui, spans, size, all_strong, style);
    if refs.is_empty() {
        ui.label(job);
        return;
    }
    let galley = ui.fonts_mut(|f| f.layout_job(job));
    let response = ui.add(egui::Label::new(Arc::clone(&galley)));
    let origin = response.rect.min.to_vec2();
    for (range, label) in refs {
        let start = galley.pos_from_cursor(CCursor::new(range.start));
        let end = galley.pos_from_cursor(CCursor::new(range.end));
        let right = if end.min.y == start.min.y {
            end.min.x
        } else {
            start.max.x + size
        };
        let rect =
            egui::Rect::from_min_max(start.min, egui::pos2(right, start.max.y)).translate(origin);
        let hit = ui
            .interact(
                rect,
                ui.scope_id().with(("md-fnref", &label, range.start)),
                Sense::click(),
            )
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text("각주로 이동");
        if hit.clicked() {
            *style.jump.borrow_mut() = Some(Jump::ToFootnote(label));
        }
    }
}

/// 각주 정의: 번호 + 내용 + `↑ 본문으로`(누르면 처음 참조로).
fn footnote(ui: &mut egui::Ui, label: &str, blocks: &[Block], style: &Style, depth: usize) {
    ui.horizontal_top(|ui| {
        ui.label(
            RichText::new(format!("{}.", style.footnote_mark(label)))
                .size(style.body * 0.85)
                .weak(),
        );
        ui.vertical(|ui| {
            render_children(ui, blocks, style, depth, 0.4);
            if style.footnotes.iter().any(|l| l == label) {
                let back = ui
                    .add(
                        egui::Label::new(
                            RichText::new("↑ 본문으로")
                                .size(style.body * 0.85)
                                .color(ui.visuals().hyperlink_color),
                        )
                        .sense(Sense::click()),
                    )
                    .on_hover_cursor(egui::CursorIcon::PointingHand);
                if back.clicked() {
                    *style.jump.borrow_mut() = Some(Jump::BackToRef(label.to_owned()));
                }
            }
        });
    });
}

/// 문단 안 이미지 (자기 줄). 로컬 파일은 원본 크기(보이는 폭보다 크면 줄임), 그 밖은 자리 표시.
fn image(ui: &mut egui::Ui, alt: &str, url: &str, title: &str, style: &Style) {
    let path = match image_load::resolve(url, style.doc_dir) {
        ImageSource::Local(path) => path,
        ImageSource::Remote(url) => {
            image_placeholder(ui, ICON_CLOUD_SLASH, "원격 이미지 차단됨", &url, style);
            return;
        }
        ImageSource::Unsupported(reason) => {
            image_placeholder(ui, ICON_IMAGE_BROKEN, file_label(url, alt), &reason, style);
            return;
        }
    };
    let mut images = style.images.borrow_mut();
    let size = images.size_of(&path);
    if let Some(error) = images.error_of(&path).map(str::to_owned) {
        drop(images);
        let name = path
            .file_name()
            .map_or_else(|| url.to_owned(), |n| n.to_string_lossy().into_owned());
        let detail = format!(
            "{}
{error}",
            path.display()
        );
        image_placeholder(ui, ICON_IMAGE_BROKEN, &name, &detail, style);
        return;
    }
    // 크기를 아직 모르면(SVG 디코딩 중) 임시 크기
    let natural = size.map_or(egui::vec2(240.0, 135.0), |(w, h)| {
        egui::vec2(w as f32, h as f32)
    });
    let scale = (ui.available_width() / natural.x.max(1.0)).min(1.0);
    let (rect, response) = ui.allocate_exact_size(natural * scale, Sense::hover());
    if ui.is_rect_visible(rect) {
        match images.texture(ui.ctx(), &path) {
            TextureState::Ready(texture) => {
                let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                ui.painter().image(texture, rect, uv, Color32::WHITE);
            }
            // 읽는 중이거나 방금 실패(다음 프레임에 자리 표시로 바뀜)
            TextureState::Loading | TextureState::Failed(_) => {
                ui.painter().rect_filled(rect, 4.0, style.code_bg);
            }
        }
    }
    let tip = if title.is_empty() { alt } else { title };
    if !tip.is_empty() {
        response.on_hover_text(tip);
    }
}

/// 이미지를 보여 줄 수 없을 때: 아이콘 + 이름, 마우스를 올리면 자세한 사유.
fn image_placeholder(ui: &mut egui::Ui, icon: &str, name: &str, detail: &str, style: &Style) {
    let stroke = ui.visuals().widgets.noninteractive.bg_stroke;
    egui::Frame::new()
        .stroke(stroke)
        .corner_radius(4)
        .inner_margin(egui::Margin::symmetric(8, 4))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(icon)
                        .family(FontFamily::Name(ICON_FAMILY.into()))
                        .size(style.body * 1.3)
                        .weak(),
                );
                ui.label(RichText::new(name).size(style.body).weak());
            });
        })
        .response
        .on_hover_text(detail);
}

/// 자리 표시에 쓸 이름: 주소의 마지막 부분, 없으면 대체 텍스트.
fn file_label<'s>(url: &'s str, alt: &'s str) -> &'s str {
    let name = url.rsplit(['/', '\\']).next().unwrap_or(url);
    if name.is_empty() { alt } else { name }
}

/// 미리보기로 그리지 않는 코드 블록 언어 (Mermaid·수식).
fn is_unsupported_diagram(lang: &str) -> bool {
    let lang = lang.trim().to_ascii_lowercase();
    matches!(
        lang.as_str(),
        "mermaid" | "math" | "latex" | "katex" | "tex"
    )
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

    /// 헤드리스 프레임: 600×800 창에 미리보기를 그리고 그려진 글자를 (글자, 위치, 크기, 줄 수)로 돌려준다.
    fn text_shapes(ctx: &egui::Context, doc: &Document) -> Vec<(String, Pos2, egui::Vec2, usize)> {
        let mut state = PreviewState::new(7);
        let mut shapes = Vec::new();
        // 첫 프레임은 스크롤 영역 크기를 재느라 위치가 다를 수 있으므로 두 번째 프레임을 본다
        for _ in 0..2 {
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
            shapes = full
                .shapes
                .iter()
                .filter_map(|c| match &c.shape {
                    egui::Shape::Text(t) => Some((
                        t.galley.text().to_owned(),
                        t.pos,
                        t.galley.size(),
                        t.galley.rows.len(),
                    )),
                    _ => None,
                })
                .collect();
        }
        shapes
    }

    fn find<'a>(
        shapes: &'a [(String, Pos2, egui::Vec2, usize)],
        text: &str,
    ) -> &'a (String, Pos2, egui::Vec2, usize) {
        shapes
            .iter()
            .find(|s| s.0 == text)
            .unwrap_or_else(|| panic!("{text:?} 글자가 없음"))
    }

    #[test]
    fn column_widths_fit_or_shrink_in_proportion() {
        // 다 들어가면 그대로
        assert_eq!(column_widths(&[50.0, 80.0], 400.0, 21.0), vec![50.0, 80.0]);
        // 넘치면 내용 폭에 비례: 남은 폭 500 - 63 = 437, 첫 열은 비례로 줄이면 하한(60)보다 좁아져 60에 고정,
        // 나머지 377을 400 : 900으로 나눈다
        let w = column_widths(&[60.0, 400.0, 900.0], 500.0, 21.0);
        assert_eq!(w[0], 60.0);
        assert!((w[1] - 377.0 * 4.0 / 13.0).abs() < 0.01, "{w:?}");
        assert!((w[2] - 377.0 * 9.0 / 13.0).abs() < 0.01, "{w:?}");
        // 하한보다 좁은 내용은 내용 폭 그대로
        let w = column_widths(&[20.0, 2000.0], 300.0, 21.0);
        assert_eq!(w[0], 20.0);
        assert!((w[1] - (300.0 - 42.0 - 20.0)).abs() < 0.01, "{w:?}");
        // 아무리 좁아도 하한 이하로는 줄이지 않는다 (가로 스크롤)
        let w = column_widths(&[300.0; 8], 400.0, 21.0);
        assert!(w.iter().all(|&x| x == MIN_COLUMN_WIDTH), "{w:?}");
    }

    #[test]
    fn align_offset_follows_column_alignment() {
        use markdown::Align as A;
        assert_eq!(align_offset(Some(A::Left), 100.0, 40.0), 0.0);
        assert_eq!(align_offset(None, 100.0, 40.0), 0.0);
        assert_eq!(align_offset(Some(A::Center), 100.0, 40.0), 30.0);
        assert_eq!(align_offset(Some(A::Right), 100.0, 40.0), 60.0);
        // 글자가 더 넓으면 여백 없음
        assert_eq!(align_offset(Some(A::Right), 100.0, 140.0), 0.0);
    }

    #[test]
    fn long_code_line_stays_on_one_row() {
        let ctx = egui::Context::default();
        let long = "x".repeat(400);
        let shapes = text_shapes(&ctx, &doc(&format!("```\n{long}\nshort\n```\n")));
        let code = shapes
            .iter()
            .find(|s| s.0.starts_with("xxxx"))
            .expect("코드 글자가 없음");
        assert_eq!(code.3, 2, "코드 두 줄이 접히지 않고 두 행이어야 함");
        assert!(
            code.2.x > 600.0,
            "긴 줄이 창 폭보다 넓게 그려져야 함 (가로 스크롤)"
        );
    }

    #[test]
    fn table_cells_follow_column_alignment() {
        let ctx = egui::Context::default();
        let shapes = text_shapes(
            &ctx,
            &doc(
                "| 왼쪽 정렬 열 | 가운데 정렬 열 | 오른쪽 정렬 열 |\n| :--- | :---: | ---: |\n| a | b | c |\n",
            ),
        );
        let (h0, h1, h2) = (
            find(&shapes, "왼쪽 정렬 열"),
            find(&shapes, "가운데 정렬 열"),
            find(&shapes, "오른쪽 정렬 열"),
        );
        let (a, b, c) = (find(&shapes, "a"), find(&shapes, "b"), find(&shapes, "c"));
        // 머리글이 열에서 가장 넓으므로 머리글 글자 범위 = 열의 글자 영역
        assert!((a.1.x - h0.1.x).abs() < 1.0, "왼쪽 정렬");
        let center = |s: &(String, Pos2, egui::Vec2, usize)| s.1.x + s.2.x / 2.0;
        assert!((center(b) - center(h1)).abs() < 1.0, "가운데 정렬");
        let right = |s: &(String, Pos2, egui::Vec2, usize)| s.1.x + s.2.x;
        assert!((right(c) - right(h2)).abs() < 1.0, "오른쪽 정렬");
        // 머리글 행이 본문 행보다 위
        assert!(h0.1.y < a.1.y);
    }

    #[test]
    fn wide_table_wraps_long_cells_within_width() {
        let ctx = egui::Context::default();
        let long = "아주 긴 설명 문장이 들어가는 열입니다 ".repeat(8);
        let shapes = text_shapes(
            &ctx,
            &doc(&format!(
                "| 항목 | 설명 |\n| --- | --- |\n| 짧음 | {} |\n",
                long.trim()
            )),
        );
        let cell = shapes
            .iter()
            .find(|s| s.0.starts_with("아주 긴"))
            .expect("긴 셀이 없음");
        assert!(cell.3 > 1, "긴 셀은 여러 줄로 접혀야 함");
        assert!(cell.1.x + cell.2.x <= 600.0, "표가 창 폭 안에 들어와야 함");
        assert_eq!(find(&shapes, "짧음").3, 1);
    }

    /// 헤드리스 프레임 (입력 포함): 그려진 글자를 (글자, 위치, galley)로 돌려준다.
    fn galleys_frame(
        ctx: &egui::Context,
        state: &mut PreviewState,
        doc: &Document,
        events: Vec<egui::Event>,
    ) -> Vec<(String, Pos2, Arc<egui::Galley>)> {
        let raw = RawInput {
            events,
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
                egui::Shape::Text(t) => {
                    Some((t.galley.text().to_owned(), t.pos, Arc::clone(&t.galley)))
                }
                _ => None,
            })
            .collect()
    }

    fn click_events(pos: Pos2) -> Vec<egui::Event> {
        let button = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        vec![egui::Event::PointerMoved(pos), button(true), button(false)]
    }

    /// 임시 폴더에 문서를 저장하고 연다 (상대 경로 이미지용).
    fn saved_doc(dir: &Path, text: &str) -> Document {
        let path = dir.join("doc.md");
        std::fs::write(&path, text).unwrap();
        Document::from_bytes(Some(path), text.as_bytes(), false, None).unwrap()
    }

    #[test]
    fn local_missing_and_remote_images() {
        let dir = tempfile::tempdir().unwrap();
        image::RgbaImage::from_pixel(64, 32, image::Rgba([200, 50, 50, 255]))
            .save(dir.path().join("a.png"))
            .unwrap();
        let d = saved_doc(
            dir.path(),
            "![그림](a.png)\n\n![없는 그림](none.png)\n\n![원격](https://example.com/x.png)\n",
        );
        let ctx = egui::Context::default();
        let mut state = PreviewState::new(1);
        // 없는 파일 자리 표시는 첫 프레임부터 보인다
        let mut texts = galleys_frame(&ctx, &mut state, &d, Vec::new());
        assert!(
            texts.iter().any(|t| t.0 == "none.png"),
            "없는 이미지 자리 표시 (첫 프레임)"
        );
        for _ in 0..400 {
            texts = galleys_frame(&ctx, &mut state, &d, Vec::new());
            if state.image_texture_count() == 1 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(
            state.image_texture_count(),
            1,
            "로컬 이미지가 올라가지 않음"
        );
        // 원본 크기(64×32)로 자리를 잡는다
        assert!((state.positions()[0].height - 32.0).abs() < 1.0);
        assert!(
            texts.iter().any(|t| t.0 == "none.png"),
            "없는 이미지 자리 표시"
        );
        assert!(texts.iter().any(|t| t.0 == "원격 이미지 차단됨"));
    }

    #[test]
    fn front_matter_starts_folded() {
        let ctx = egui::Context::default();
        let mut state = PreviewState::new(1);
        let d = doc("---\ntitle: 제목\ntags: [a]\n---\n\n본문\n");
        let texts = galleys_frame(&ctx, &mut state, &d, Vec::new());
        assert!(texts.iter().any(|t| t.0 == "Front Matter · 2줄"));
        assert!(
            !texts.iter().any(|t| t.0.contains("title: 제목")),
            "접혀 있어야 함"
        );
        assert!(texts.iter().any(|t| t.0 == "본문"));
    }

    #[test]
    fn footnote_reference_jumps_to_footnote_and_back() {
        let ctx = egui::Context::default();
        let mut state = PreviewState::new(1);
        let filler: String = (0..80).map(|i| format!("문단 {i}\n\n")).collect();
        let d = doc(&format!("본문[^note] 끝\n\n{filler}[^note]: 각주 내용\n"));
        let texts = galleys_frame(&ctx, &mut state, &d, Vec::new());
        // 참조는 화면 번호 1로 보인다
        let (_, pos, galley) = texts
            .iter()
            .find(|t| t.0 == "본문[1] 끝")
            .expect("참조 문단");
        let start = galley.pos_from_cursor(CCursor::new(2));
        let end = galley.pos_from_cursor(CCursor::new(5));
        let target = *pos + egui::vec2((start.min.x + end.min.x) / 2.0, start.center().y);
        galleys_frame(&ctx, &mut state, &d, click_events(target));
        galleys_frame(&ctx, &mut state, &d, Vec::new());
        let texts = galleys_frame(&ctx, &mut state, &d, Vec::new());
        let footnote = *state.positions().last().unwrap();
        assert!(
            state.offset_y() > 500.0,
            "각주로 이동하지 않음: {}",
            state.offset_y()
        );
        let on_screen = footnote.top - state.offset_y();
        assert!(
            (0.0..800.0).contains(&on_screen),
            "각주가 화면 밖: {on_screen}"
        );

        // ↑ 본문으로 → 참조가 다시 화면 안으로
        let (_, back, galley) = texts
            .iter()
            .find(|t| t.0 == "↑ 본문으로")
            .expect("되돌아가기");
        let back = *back + galley.rect.center().to_vec2();
        galleys_frame(&ctx, &mut state, &d, click_events(back));
        galleys_frame(&ctx, &mut state, &d, Vec::new());
        galleys_frame(&ctx, &mut state, &d, Vec::new());
        assert!(
            state.offset_y() < 50.0,
            "참조로 돌아오지 않음: {}",
            state.offset_y()
        );
    }

    #[test]
    fn mermaid_and_math_blocks_get_unsupported_label() {
        let ctx = egui::Context::default();
        let mut state = PreviewState::new(1);
        let d = doc("```mermaid\nA --> B\n```\n\n```rust\nfn a() {}\n```\n");
        let texts = galleys_frame(&ctx, &mut state, &d, Vec::new());
        assert_eq!(texts.iter().filter(|t| t.0 == "미리보기 미지원").count(), 1);
        assert!(
            texts.iter().any(|t| t.0 == "A --> B"),
            "코드는 그대로 보인다"
        );
        assert!(is_unsupported_diagram(" Math "));
        assert!(!is_unsupported_diagram("rust"));
    }

    /// `gfm_all.md`·`korean.md`를 `bytes` 이상이 되도록 반복한 문서.
    fn long_markdown(bytes: usize) -> String {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/markdown");
        let unit = format!(
            "{}\n{}\n",
            std::fs::read_to_string(dir.join("gfm_all.md")).unwrap(),
            std::fs::read_to_string(dir.join("korean.md")).unwrap()
        );
        let mut text = String::with_capacity(bytes + unit.len());
        while text.len() < bytes {
            text.push_str(&unit);
        }
        text
    }

    #[test]
    fn only_visible_blocks_are_rendered() {
        let ctx = egui::Context::default();
        let mut state = PreviewState::new(1);
        let d = doc(&long_markdown(200 * 1024));
        for _ in 0..3 {
            galleys_frame(&ctx, &mut state, &d, Vec::new());
        }
        let blocks = state.positions().len();
        let (measured, rendered) = state.last_frame_counts();
        assert!(blocks > 1000, "블록 수 {blocks}");
        assert_eq!(measured, 0, "높이가 이미 캐시됨");
        assert!(
            rendered < 40,
            "보이는 블록만 그려야 함: {rendered}/{blocks}"
        );
        assert!(
            state.positions().windows(2).all(|w| w[1].top > w[0].top),
            "위치표가 단조 증가해야 함"
        );

        // 문서 가운데로 이동하면 그 블록이 그려진다
        let mid = state.positions()[blocks / 2];
        state.set_scroll_offset(mid.top - 20.0);
        galleys_frame(&ctx, &mut state, &d, Vec::new());
        let texts = galleys_frame(&ctx, &mut state, &d, Vec::new());
        let (_, rendered) = state.last_frame_counts();
        assert!(rendered < 40);
        let top = state.top_line().unwrap();
        assert!(
            (top - mid.first_line as f32).abs() < 3.0,
            "가운데 블록이 화면 맨 위에 있어야 함: {top} / {}",
            mid.first_line
        );
        assert!(!texts.is_empty());
    }

    #[test]
    fn editing_remeasures_only_changed_blocks() {
        let ctx = egui::Context::default();
        let mut state = PreviewState::new(1);
        let text = long_markdown(100 * 1024);
        let mut d = doc(&text);
        galleys_frame(&ctx, &mut state, &d, Vec::new());
        let (first_measured, _) = state.last_frame_counts();
        assert!(first_measured > 500, "처음에는 모두 잰다: {first_measured}");
        galleys_frame(&ctx, &mut state, &d, Vec::new());
        assert_eq!(state.last_frame_counts().0, 0);

        // 첫 문단 한 곳만 고친다 → 그 블록만 다시 잰다
        d.set_text(text.replacen("문단에 **굵게**", "문단을 고쳐 **굵게**", 1));
        galleys_frame(&ctx, &mut state, &d, Vec::new());
        let (measured, _) = state.last_frame_counts();
        assert!(measured <= 2, "바뀐 블록만 다시 재야 함: {measured}");
    }

    /// 1MB 미리보기 스크롤 프레임 시간 — `cargo test --release --lib -- --ignored --nocapture preview_scroll_frame_time`
    #[test]
    #[ignore = "측정용: 릴리스 빌드로 실행"]
    fn preview_scroll_frame_time_1mb() {
        let ctx = egui::Context::default();
        let mut state = PreviewState::new(1);
        let d = doc(&long_markdown(1 << 20));
        // 처음 파싱(백그라운드)과 전체 측정이 끝날 때까지
        let start = std::time::Instant::now();
        while state.positions().is_empty() || state.is_parsing() {
            galleys_frame(&ctx, &mut state, &d, Vec::new());
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let first = std::time::Instant::now();
        galleys_frame(&ctx, &mut state, &d, Vec::new());
        let first_layout = first.elapsed().as_secs_f64() * 1000.0;
        let ready = start.elapsed().as_secs_f64() * 1000.0;
        let total = state.positions().last().map_or(0.0, |p| p.top + p.height);
        let frames = 300;
        let mut times = Vec::with_capacity(frames);
        for i in 0..frames {
            state.set_scroll_offset(total * i as f32 / frames as f32);
            let t = std::time::Instant::now();
            galleys_frame(&ctx, &mut state, &d, Vec::new());
            times.push(t.elapsed().as_secs_f64() * 1000.0);
        }
        let avg = times.iter().sum::<f64>() / frames as f64;
        let max = times.iter().copied().fold(0.0, f64::max);
        println!(
            "1MB 미리보기 스크롤: 블록 {}개, 첫 표시까지 {ready:.0}ms(첫 레이아웃 프레임 {first_layout:.1}ms), 스크롤 {frames}프레임 평균 {avg:.2}ms·최대 {max:.2}ms",
            state.positions().len()
        );
    }

    #[test]
    fn br_tag_variants() {
        assert!(is_br("<br>"));
        assert!(is_br("<BR />"));
        assert!(is_br("<br/>"));
        assert!(!is_br("<b>"));
    }
}
