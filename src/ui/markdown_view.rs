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
//! - 실시간 갱신(P3-07): 편집이 [`REFRESH_DEBOUNCE`] 동안 멈추면 다시 파싱한다 (그동안은 이전 결과를 그대로 그린다).
//!   위치표는 스크롤 영역보다 먼저 만들고, 화면 맨 위 블록 위쪽의 높이가 바뀌면 같은 프레임에 그만큼 스크롤을 옮겨
//!   보던 내용이 튀지 않게 한다.
//! - 코드 블록은 줄을 접지 않고 블록 안에서만 가로 스크롤, 표는 내용에 맞춘 열 폭·넘치면 열을 내용 폭에 비례해
//!   줄여 줄바꿈·그래도 넘치면 표만 가로 스크롤 (P3-03).
//! - 이미지(P3-04): 문단 안 이미지는 자기 줄에 그린다. 로컬 파일은 원본 크기(보이는 폭에 맞춰 축소)로,
//!   디코딩·텍스처는 [`ImageCache`]가 맡는다. 원격 이미지는 차단, 없는·읽지 못한 이미지는 파일 이름 + 깨진 이미지 아이콘.
//! - Front Matter는 접힌 메타데이터 블록, 각주 참조 번호를 누르면 각주로·각주의 `↑ 본문으로`를 누르면 참조로 이동,
//!   원시 HTML은 회색 원문(`<br>`만 줄바꿈), Mermaid·수식 코드 블록에는 "미리보기 미지원" 라벨.
//! - 링크(P3-09): 마우스를 올리면 주소 툴팁, 누르면 [`LinkAction`]으로 앱에 알린다 (외부 주소·파일 열기·안내).
//!   문서 안 `#앵커`는 미리보기가 직접 그 제목으로 스크롤한다. 대상 파일이 없는 링크는 오류 색으로 그린다.
//! - 찾기(P4-06, 미리보기 단독 모드): 미리보기에 **보이는 글자**에서 찾는다. 블록마다 그리는 글자 조각(문단·제목·
//!   코드·표 칸 …)을 그리는 순서대로 모아 매치 수를 세고([`count_matches`]), 그릴 때 같은 조각의 매치에 배경을
//!   칠한다([`mark_find`]). 두 쪽 모두 스팬 글자를 [`span_text`]로 만들어 어긋나지 않게 한다. 현재 매치는 전체 번호로
//!   기억하고, 이동하면 그 블록으로 스크롤한 뒤 그려진 매치 위치가 화면 밖이면 한 번 더 맞춘다.
//!   원시 HTML 블록·Front Matter·이미지 자리 표시는 찾지 않는다.

use eframe::egui::{
    self, Align, Color32, FontFamily, FontId, RichText, Sense, Stroke, TextFormat, TextStyle,
    text::{CCursor, LayoutJob},
};

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::path::Path;
use std::sync::Arc;

use crate::core::document::Document;
use crate::core::image_load::{self, ImageSource};
use crate::core::link::{self, LinkTarget};
use crate::core::markdown::{
    self, Block, BlockKind, Inlines, ListItem, MarkdownCache, Span, SpanKind,
};
use crate::core::search::{SearchOptions, SearchQuery, byte_ranges_to_char_ranges};
use crate::fonts::ICON_FAMILY;
use crate::ui::preview_images::{ImageCache, TextureState};
use crate::ui::theme::Palette;

/// Phosphor 아이콘: 깨진 이미지, 차단된 원격(구름에 빗금) (egui-phosphor 0.14 codepoints)
const ICON_IMAGE_BROKEN: &str = "\u{E7A8}";
const ICON_CLOUD_SLASH: &str = "\u{E1B6}";

/// 미리보기 본문 바깥 여백(px).
const MARGIN: i8 = 16;
/// 미리보기 본문 최대 폭(px) — 넓은 창에서는 이 폭으로 가운데 둔다 (PLAN.md §3.8-7 가독성, P3-11).
pub const MAX_BODY_WIDTH: f32 = 860.0;
/// 편집 후 미리보기를 다시 파싱하기까지 기다리는 시간 (초, PLAN.md §3.8-2 "약 150~200ms").
pub const REFRESH_DEBOUNCE: f64 = 0.15;

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
    /// 최상위 블록마다의 위치 (블록 순서와 같다 — 그리기·찾기·더블클릭용)
    positions: Vec<BlockPos>,
    /// 소스 줄 ↔ y 변환용 위치표 (표는 행마다 — [`HeightCache::line_map`])
    line_map: Vec<BlockPos>,
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
    /// 마지막으로 본 문서 버전과 그 버전이 처음 보인 시각 (디바운스, P3-07)
    seen_revision: Option<u64>,
    changed_at: f64,
    /// 지난 프레임 화면 맨 위 블록 (위쪽 높이가 바뀌어도 보던 내용을 고정)
    anchor: Option<ViewAnchor>,
    /// 파싱이 끝나면 이 제목 앵커로 스크롤한다 (다른 문서의 `파일.md#앵커` 링크로 열었을 때)
    pending_anchor: Option<String>,
    /// 미리보기 찾기 (P4-06)
    find: Option<PreviewFind>,
}

/// 미리보기 찾기 이동 결과 (P4-06).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewStep {
    Moved,
    /// 문서 끝(이전 찾기는 처음)을 지나 반대쪽 끝에서 이어 찾았다
    Wrapped,
    /// Wrap Around를 꺼 둔 상태에서 더 찾을 매치가 없다
    End,
    NoMatch,
}

/// 미리보기 찾기 상태.
#[derive(Debug)]
struct PreviewFind {
    query: SearchQuery,
    pattern: String,
    options: SearchOptions,
    /// 매치 수를 센 파싱 결과
    counted_for: Option<Arc<markdown::Document>>,
    /// `prefix[i]` = 최상위 블록 i 앞까지의 매치 수 (길이 = 블록 수 + 1)
    prefix: Vec<usize>,
    /// 현재 매치 (전체 번호)
    current: Option<usize>,
    /// 검색 조건이 바뀌어, 다 세면 화면 맨 위 이후 첫 매치를 고른다
    pick_first: bool,
    /// 현재 매치가 보이도록 스크롤할 남은 프레임 수 (블록으로 옮긴 뒤 그려진 위치로 한 번 더 맞춘다)
    reveal: u8,
}

impl PreviewFind {
    fn total(&self) -> usize {
        self.prefix.last().copied().unwrap_or(0)
    }

    /// 전체 번호 → (최상위 블록, 블록 안 순번)
    fn locate(&self, index: usize) -> Option<(usize, usize)> {
        if index >= self.total() {
            return None;
        }
        let block = self.prefix.partition_point(|&p| p <= index) - 1;
        Some((block, index - self.prefix[block]))
    }
}

/// 화면 맨 위(스크롤 위치 `offset_y`) 이후 처음 보이는 매치의 전체 번호. 없으면 첫 매치.
fn first_match_from(prefix: &[usize], positions: &[BlockPos], offset_y: f32) -> Option<usize> {
    let total = prefix.last().copied().unwrap_or(0);
    if total == 0 {
        return None;
    }
    let from = positions.partition_point(|p| p.top + p.height <= offset_y);
    let index = prefix.get(from).copied().unwrap_or(total);
    Some(if index < total { index } else { 0 })
}

/// 화면 맨 위 블록: (원문 해시, 순번, 그때의 위치)
#[derive(Debug, Clone, Copy)]
struct ViewAnchor {
    key: u64,
    index: usize,
    top: f32,
}

/// 블록 높이 캐시: 블록 원문 해시 → 높이. 본문 폭·확대·테마가 바뀌면 비운다.
#[derive(Debug, Default)]
struct HeightCache {
    /// (본문 폭 px, 확대 배율 ×100, 다크 모드)
    layout: Option<(i32, i32, bool)>,
    heights: HashMap<u64, f32>,
    /// 최상위 표 블록의 행 높이 (원문 해시 → [머리글, 1행, 2행, …]) — 줄 ↔ y 위치표용
    table_rows: HashMap<u64, Vec<f32>>,
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
            line_map: Vec::new(),
            offset_y: 0.0,
            view_height: 0.0,
            max_offset: 0.0,
            pending_offset: None,
            pending_top_line: None,
            images: RefCell::new(ImageCache::default()),
            heights: HeightCache::default(),
            seen_revision: None,
            changed_at: 0.0,
            anchor: None,
            pending_anchor: None,
            find: None,
        }
    }

    /// 미리보기에서 찾을 조건을 정한다 (미리보기 단독 모드에서 찾기 바가 열려 있는 동안 매 프레임).
    /// `None`이면 찾지 않고 강조도 지운다. 검색어·옵션이 바뀌면 다시 세고 화면 맨 위 이후 첫 매치로 간다.
    pub fn set_find(&mut self, find: Option<(SearchQuery, &str, SearchOptions)>) {
        let Some((query, pattern, options)) = find else {
            self.find = None;
            return;
        };
        if self
            .find
            .as_ref()
            .is_some_and(|f| f.pattern == pattern && f.options == options)
        {
            return;
        }
        self.find = Some(PreviewFind {
            query,
            pattern: pattern.to_owned(),
            options,
            counted_for: None,
            prefix: Vec::new(),
            current: None,
            pick_first: true,
            reveal: 0,
        });
    }

    /// 찾기 상태 `(현재 매치 번호, 매치 수)` — 아직 세지 않았으면 `None`.
    pub fn find_status(&self) -> Option<(Option<usize>, usize)> {
        let find = self.find.as_ref()?;
        find.counted_for.as_ref()?;
        Some((find.current, find.total()))
    }

    /// 다음(`forward`)/이전 매치로 옮긴다. 현재 매치가 없으면 화면 맨 위 이후 첫 매치로.
    pub fn find_step(&mut self, forward: bool, wrap_around: bool) -> PreviewStep {
        let Some(find) = self.find.as_mut().filter(|f| f.total() > 0) else {
            return PreviewStep::NoMatch;
        };
        let total = find.total();
        let (next, step) = match find.current {
            None => (
                first_match_from(&find.prefix, &self.positions, self.offset_y),
                PreviewStep::Moved,
            ),
            Some(c) if forward && c + 1 < total => (Some(c + 1), PreviewStep::Moved),
            Some(c) if !forward && c > 0 => (Some(c - 1), PreviewStep::Moved),
            Some(_) if !wrap_around => (None, PreviewStep::End),
            Some(_) if forward => (Some(0), PreviewStep::Wrapped),
            Some(_) => (Some(total - 1), PreviewStep::Wrapped),
        };
        if next.is_some() {
            find.current = next;
            find.reveal = 3;
        }
        step
    }

    /// 다음에 그릴 때(큰 문서는 파싱이 끝난 뒤) `anchor` 제목으로 스크롤한다.
    pub fn jump_to_anchor(&mut self, anchor: String) {
        self.pending_anchor = Some(anchor);
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
        (!self.line_map.is_empty()).then(|| y_to_line(&self.line_map, self.offset_y + dy))
    }

    /// `line`이 위로부터 `dy` 위치에 오는 세로 스크롤 위치 (0 미만이면 0).
    pub fn offset_for_line(&self, line: f32, dy: f32) -> f32 {
        (line_to_y(&self.line_map, line) - dy).max(0.0)
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

    /// 소스 줄 ↔ y 변환용 위치표 ([`line_to_y`]·[`y_to_line`]에 넘긴다 — 표는 행마다 나뉘어 있다).
    pub fn line_map(&self) -> &[BlockPos] {
        &self.line_map
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
        // 편집이 멈출 때까지(디바운스) 이전 파싱 결과를 그대로 그린다
        let now = ui.input(|i| i.time);
        let revision = doc.revision();
        if self.seen_revision != Some(revision) {
            self.seen_revision = Some(revision);
            self.changed_at = now;
        }
        let waited = now - self.changed_at;
        let parsed = match &self.parsed {
            Some(previous)
                if !self.cache.is_current(revision, doc.text()) && waited < REFRESH_DEBOUNCE =>
            {
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_secs_f64(
                        REFRESH_DEBOUNCE - waited,
                    ));
                Arc::clone(previous)
            }
            _ => {
                let ctx = ui.ctx().clone();
                self.cache
                    .get_or_spawn(revision, doc.text(), move || ctx.request_repaint())
            }
        };
        self.parsed = Some(std::sync::Arc::clone(&parsed));
        // 미리보기 찾기: 파싱 결과나 조건이 바뀌었으면 블록마다 매치를 다시 센다
        if let Some(find) = &mut self.find
            && !(self.cache.is_parsing() && parsed.blocks.is_empty())
            && !find
                .counted_for
                .as_ref()
                .is_some_and(|d| Arc::ptr_eq(d, &parsed))
        {
            find.prefix = count_matches(&parsed.blocks, &find.query, &parsed.footnotes);
            find.counted_for = Some(Arc::clone(&parsed));
            let total = find.total();
            find.current = find.current.filter(|&c| c < total);
            ui.ctx().request_repaint();
        }
        self.images.borrow_mut().begin_frame(ui.ctx());
        let doc_dir = doc.path().and_then(Path::parent);
        let mut style = Style::new(ui, zoom, palette, doc_dir, &self.images, &parsed.footnotes);
        style.cursor = cursor_line.map(|line| (line, palette.cursor_block));
        if self.pending_top_line.is_some() {
            ui.set_invisible();
        }
        let parsing = self.cache.is_parsing() && parsed.blocks.is_empty();
        // 위치표는 스크롤 영역보다 먼저 만든다 (스크롤바는 내용 위에 떠 있어 폭이 같다)
        // 넓은 창에서는 본문 폭을 제한하고 가운데 둔다 (P3-11)
        let width = (ui.available_width() - 2.0 * MARGIN as f32).clamp(1.0, MAX_BODY_WIDTH);
        let left = ((ui.available_width() - width) / 2.0)
            .max(MARGIN as f32)
            .round();
        let mut anchored = false;
        let mut link_action = None;
        let mut jumped_line = None;
        if parsing {
            self.positions.clear();
            self.line_map.clear();
        } else {
            let heights = &mut self.heights;
            heights.prepare(&parsed, doc, width, zoom, ui.visuals().dark_mode);
            heights.measure_missing(ui, &parsed.blocks, &style, width);
            self.positions = heights.positions(&parsed.blocks, &style);
            self.line_map = heights.line_map(&parsed.blocks, &self.positions);
            if let Some(anchor) = self.pending_anchor.take() {
                match link::anchor_line(&parsed, &anchor) {
                    Some(line) => {
                        self.pending_offset = Some(anchor_offset(&self.line_map, line));
                        jumped_line = Some(line);
                    }
                    None => link_action = Some(LinkAction::Notice(missing_anchor(&anchor))),
                }
            }
            // 찾기: 조건이 바뀐 뒤 처음 센 결과면 화면 맨 위 이후 첫 매치로, 현재 매치 블록이 화면 밖이면 그 블록으로
            if let Some(find) = &mut self.find
                && find.counted_for.is_some()
            {
                if std::mem::take(&mut find.pick_first) {
                    find.current = first_match_from(&find.prefix, &self.positions, self.offset_y);
                    find.reveal = if find.current.is_some() { 3 } else { 0 };
                }
                if find.reveal > 0
                    && self.pending_offset.is_none()
                    && let Some((block, _)) = find.current.and_then(|c| find.locate(c))
                    && let Some(pos) = self.positions.get(block)
                {
                    let visible = self.offset_y..self.offset_y + self.view_height;
                    if !(visible.contains(&pos.top) && visible.contains(&(pos.top + pos.height))) {
                        self.pending_offset = Some((pos.top - self.view_height / 3.0).max(0.0));
                    }
                }
                style.find = Some(FindPaint {
                    query: find.query.clone(),
                    match_color: palette.search_match,
                    current_color: palette.search_current,
                    current: find.current.and_then(|c| find.locate(c)),
                    block: Cell::new(None),
                    ordinal: Cell::new(0),
                    current_rect: Cell::new(None),
                });
            }
            // 지난 프레임 맨 위 블록이 움직였으면(위쪽 블록의 높이가 바뀜) 같은 만큼 스크롤해 보던 내용을 고정
            if self.pending_offset.is_none()
                && self.pending_top_line.is_none()
                && let Some(anchor) = self.anchor
                && let Some(j) = heights.find(anchor.key, anchor.index)
            {
                let shift = self.positions[j].top - anchor.top;
                if shift.abs() > 0.5 {
                    self.pending_offset = Some((self.offset_y + shift).max(0.0));
                    anchored = true;
                }
            }
        }
        let total = self
            .positions
            .last()
            .map_or(0.0, |p| p.top + p.height + MARGIN as f32);

        let mut scroll = egui::ScrollArea::vertical()
            .id_salt(("md-preview", self.id))
            .auto_shrink(false);
        // 스크롤 범위를 넘는 위치(문서 끝 각주로 이동 등)는 내용 높이로 자른다 —
        // egui는 넘는 값을 그대로 써서 내용을 그 자리에 그린다
        if let Some(y) = self.pending_offset.take() {
            let max = (total - ui.available_height()).max(0.0);
            scroll = scroll.vertical_scroll_offset(y.min(max));
        }
        let heights = &mut self.heights;
        let positions = &self.positions;
        let output = scroll.show_viewport(ui, |ui, viewport| {
            if parsing {
                ui.add_space(MARGIN as f32);
                ui.label(RichText::new("  큰 문서를 분석하는 중…").weak());
                return;
            }
            let content = ui.min_rect().min + egui::vec2(left - MARGIN as f32, 0.0);
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
        // 다음 프레임의 기준 블록 = 지금 화면 맨 위 블록
        let top = self
            .positions
            .partition_point(|p| p.top + p.height <= self.offset_y);
        self.anchor = self.positions.get(top).map(|p| ViewAnchor {
            key: self.heights.keys[top],
            index: top,
            top: p.top,
        });
        if let Some(line) = self.pending_top_line.take() {
            self.pending_offset = Some(line_to_y(&self.line_map, line));
            ui.ctx().request_repaint();
        }
        // 찾기: 그려진 현재 매치가 화면 위·아래 밖이면 가운데로 (긴 블록 안의 매치).
        // 블록으로 옮긴 프레임에는 아직 그려지지 않았을 수 있어 몇 프레임 더 확인한다.
        let drawn = style.find.as_ref().and_then(|f| f.current_rect.get());
        if let Some(find) = &mut self.find
            && find.reveal > 0
        {
            find.reveal -= 1;
            let view = output.inner_rect;
            match drawn {
                Some(rect) if rect.top() >= view.top() && rect.bottom() <= view.bottom() => {
                    find.reveal = 0;
                }
                Some(rect) if self.pending_offset.is_none() => {
                    let content_y = rect.center().y - view.top() + self.offset_y;
                    self.pending_offset = Some((content_y - view.height() / 2.0).max(0.0));
                }
                _ => {}
            }
            if find.reveal > 0 {
                ui.ctx().request_repaint();
            }
        }

        self.images.borrow_mut().end_frame();

        // 블록 더블클릭 → 그 블록의 첫 소스 줄 (P3-08)
        let view = output.inner_rect;
        let content_top = view.top() - self.offset_y;

        // 각주 이동 (P3-04): 참조 → 각주 블록 위, 각주 → 처음 참조가 화면 위쪽 1/3에 오게
        if let Some(jump) = style.jump.take() {
            let target = match jump {
                Jump::Link(url) => match link::resolve(&url, doc_dir) {
                    LinkTarget::Anchor(anchor) => {
                        self.anchor_jump(&parsed, &anchor, &mut jumped_line, &mut link_action)
                    }
                    LinkTarget::File {
                        path,
                        anchor,
                        exists: true,
                    } if doc.path().is_some_and(|p| same_file(p, &path)) => {
                        // 이 문서 자신(`이문서.md#제목`) — 앵커만 따른다
                        anchor.and_then(|a| {
                            self.anchor_jump(&parsed, &a, &mut jumped_line, &mut link_action)
                        })
                    }
                    LinkTarget::File {
                        path,
                        anchor,
                        exists: true,
                    } => {
                        link_action = Some(LinkAction::OpenFile { path, anchor });
                        None
                    }
                    LinkTarget::File { path, .. } => {
                        link_action = Some(LinkAction::Notice(format!(
                            "링크 대상 파일이 없습니다: {}",
                            path.display()
                        )));
                        None
                    }
                    LinkTarget::External(url) => {
                        link_action = Some(LinkAction::OpenUrl(url));
                        None
                    }
                    LinkTarget::Unresolved(reason) => {
                        link_action = Some(LinkAction::Notice(format!(
                            "링크를 열 수 없습니다: {reason}"
                        )));
                        None
                    }
                },
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
                    .map(|line| line_to_y(&self.line_map, line as f32) - view.height() / 3.0),
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
            anchored,
            link: link_action,
            jumped_line,
        }
    }

    /// 이 문서의 `#앵커`로 스크롤할 위치. 없으면 안내를 남기고 `None`.
    fn anchor_jump(
        &self,
        parsed: &markdown::Document,
        anchor: &str,
        jumped_line: &mut Option<usize>,
        link_action: &mut Option<LinkAction>,
    ) -> Option<f32> {
        match link::anchor_line(parsed, anchor) {
            Some(line) => {
                *jumped_line = Some(line);
                Some(anchor_offset(&self.line_map, line))
            }
            None => {
                *link_action = Some(LinkAction::Notice(missing_anchor(anchor)));
                None
            }
        }
    }
}

/// 제목 줄이 화면 맨 위 조금 아래에 오는 스크롤 위치.
fn anchor_offset(positions: &[BlockPos], line: usize) -> f32 {
    (line_to_y(positions, line as f32) - MARGIN as f32 / 2.0).max(0.0)
}

fn missing_anchor(anchor: &str) -> String {
    format!("문서에 '#{anchor}' 제목이 없습니다")
}

/// 두 경로가 같은 파일인지 (실제 경로로 비교).
fn same_file(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// 미리보기에서 누른 링크 중 앱이 처리할 것.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkAction {
    /// 외부 주소를 OS 기본 프로그램(브라우저·메일)으로 연다
    OpenUrl(String),
    /// 파일을 앱 탭으로 연다 (이미 열려 있으면 그 탭으로), `anchor`가 있으면 그 제목으로
    OpenFile {
        path: std::path::PathBuf,
        anchor: Option<String>,
    },
    /// 상태 표시줄 안내 (없는 파일·제목 등)
    Notice(String),
}

/// [`PreviewState::show`] 결과.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PreviewOutput {
    /// 이번 프레임에 더블클릭한 블록의 첫 소스 줄 (0부터)
    pub double_clicked_line: Option<usize>,
    /// 위쪽 블록 높이가 바뀌어 이번 프레임에 스크롤을 옮겼다 (사용자 조작이 아님 — 스크롤 동기화가 따라가지 않는다)
    pub anchored: bool,
    /// 누른 링크 중 앱이 처리할 것
    pub link: Option<LinkAction>,
    /// `#앵커` 링크로 미리보기를 이 소스 줄의 제목으로 옮겼다 (분할 모드면 소스도 따라간다)
    pub jumped_line: Option<usize>,
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
    /// 링크 (주소 원문)
    Link(String),
}

/// 크기·색 규칙과 한 프레임 동안 모으는 상태.
struct Style<'a> {
    body: f32,
    line: f32,
    /// 위젯 사이 세로 간격 (블록 사이에도 더한다)
    item_gap: f32,
    code_bg: Color32,
    warning: Color32,
    /// 대상이 없는 링크 색
    error: Color32,
    /// (커서 소스 줄, 강조 색) — 이 줄을 포함한 최상위 블록 뒤에 배경을 깐다
    cursor: Option<(usize, Color32)>,
    /// 문서가 있는 폴더 (상대 경로 이미지 기준, 저장하지 않은 문서면 `None`)
    doc_dir: Option<&'a Path>,
    images: &'a RefCell<ImageCache>,
    /// 각주 이름 (처음 참조한 순서 — 화면 번호 = 순서 + 1)
    footnotes: &'a [String],
    /// 이번 프레임에 누른 각주 이동·링크
    jump: RefCell<Option<Jump>>,
    /// 이번 프레임에 본 링크의 대상 없음 여부 (주소 → 깨짐)
    broken_links: RefCell<HashMap<String, bool>>,
    /// 미리보기 찾기 강조 (P4-06)
    find: Option<FindPaint>,
    /// 방금 그린 표의 행 높이 ([`table`]이 남기고 최상위 블록을 그린 쪽이 가져간다 — 줄 ↔ y 위치표용)
    table_rows: RefCell<Option<Vec<f32>>>,
}

/// 그리는 동안의 미리보기 찾기 강조 상태.
struct FindPaint {
    query: SearchQuery,
    match_color: Color32,
    current_color: Color32,
    /// 현재 매치 (최상위 블록, 블록 안 순번)
    current: Option<(usize, usize)>,
    /// 지금 화면에 그리는 최상위 블록 (높이 측정용으로 그릴 때는 `None` — 칠하지 않는다)
    block: Cell<Option<usize>>,
    /// 지금 블록에서 지금까지 칠한 매치 수 (블록 안 순번)
    ordinal: Cell<usize>,
    /// 이번 프레임에 그린 현재 매치의 화면 위치
    current_rect: Cell<Option<egui::Rect>>,
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
            error: palette.status_error,
            cursor: None,
            doc_dir,
            images,
            footnotes,
            jump: RefCell::new(None),
            broken_links: RefCell::new(HashMap::new()),
            find: None,
            table_rows: RefCell::new(None),
        }
    }

    /// 대상이 없는 링크인지 (없는 파일·알 수 없는 대상). 프레임 안에서는 주소마다 한 번만 확인한다.
    fn link_broken(&self, url: &str) -> bool {
        if let Some(&broken) = self.broken_links.borrow().get(url) {
            return broken;
        }
        let broken = link::resolve(url, self.doc_dir).is_broken();
        self.broken_links
            .borrow_mut()
            .insert(url.to_owned(), broken);
        broken
    }

    /// 각주 화면 번호 문자열 (참조가 없으면 이름 그대로).
    fn footnote_mark(&self, label: &str) -> String {
        footnote_mark(self.footnotes, label)
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

/// 각주 화면 번호 문자열 (참조가 없으면 이름 그대로). `footnotes`는 처음 참조한 순서의 각주 이름.
fn footnote_mark(footnotes: &[String], label: &str) -> String {
    footnotes
        .iter()
        .position(|l| l == label)
        .map_or_else(|| label.to_owned(), |i| (i + 1).to_string())
}

/// 스팬이 미리보기에 그려지는 글자 — 그리기([`inline_job_marked`])와 찾기 매치 세기([`count_matches`])가 함께 쓴다.
fn span_text<'s>(span: &'s Span, footnotes: &[String]) -> std::borrow::Cow<'s, str> {
    match &span.kind {
        SpanKind::Text | SpanKind::Code => span.text.as_str().into(),
        SpanKind::Html if is_br(&span.text) => "\n".into(),
        SpanKind::Html => span.text.as_str().into(),
        SpanKind::Image { .. } => format!("[이미지: {}]", span.text).into(),
        SpanKind::FootnoteRef => format!("[{}]", footnote_mark(footnotes, &span.text)).into(),
    }
}

/// 스팬들의 그려지는 글자 (라벨 하나).
fn spans_text(spans: &[Span], footnotes: &[String]) -> String {
    spans.iter().map(|s| span_text(s, footnotes)).collect()
}

/// 블록이 미리보기에 그리는 글자 조각들 — 그리는 순서와 같다 (찾기 대상: 문단·제목·목록·인용·코드·표 칸·각주).
/// 원시 HTML 블록·Front Matter·구분선·이미지는 찾지 않는다 (칠하는 쪽도 마찬가지).
fn block_segments(block: &Block, footnotes: &[String], out: &mut Vec<String>) {
    match &block.kind {
        BlockKind::Heading { text, .. } | BlockKind::Paragraph(text) => {
            // [`inline_text`]처럼 이미지에서 라벨이 나뉜다
            let mut start = 0;
            for (i, span) in text.iter().enumerate() {
                if matches!(span.kind, SpanKind::Image { .. }) {
                    if start < i {
                        out.push(spans_text(&text[start..i], footnotes));
                    }
                    start = i + 1;
                }
            }
            if start < text.len() || text.is_empty() {
                out.push(spans_text(&text[start..], footnotes));
            }
        }
        BlockKind::List { items, .. } => {
            for item in items {
                for child in &item.blocks {
                    block_segments(child, footnotes, out);
                }
            }
        }
        BlockKind::Quote(children)
        | BlockKind::Footnote {
            blocks: children, ..
        } => {
            for child in children {
                block_segments(child, footnotes, out);
            }
        }
        BlockKind::Code { text, .. } => out.push(text.clone()),
        BlockKind::Table { head, rows, .. } => {
            // [`table`]처럼 가장 긴 행에 맞춰 빈 칸을 채운다
            let columns = rows.iter().map(Vec::len).fold(head.len(), usize::max);
            for cells in std::iter::once(head).chain(rows) {
                for c in 0..columns {
                    out.push(
                        cells
                            .get(c)
                            .map_or_else(String::new, |cell| spans_text(cell, footnotes)),
                    );
                }
            }
        }
        BlockKind::Rule | BlockKind::Html(_) | BlockKind::FrontMatter(_) => {}
    }
}

/// 최상위 블록마다 미리보기 글자의 매치 수 누적 (`prefix[i]` = 블록 i 앞까지, 길이 = 블록 수 + 1).
fn count_matches(blocks: &[Block], query: &SearchQuery, footnotes: &[String]) -> Vec<usize> {
    let mut prefix = Vec::with_capacity(blocks.len() + 1);
    let mut total = 0;
    let mut segments = Vec::new();
    prefix.push(0);
    for block in blocks {
        segments.clear();
        block_segments(block, footnotes, &mut segments);
        total += segments.iter().map(|s| query.count(s)).sum::<usize>();
        prefix.push(total);
    }
    prefix
}

/// 미리보기 찾기: 화면에 그리는 블록의 글자 조각 `job`에서 매치에 배경을 칠한다 (높이 측정 때는 하지 않음).
/// 현재 매치가 이 조각에 있으면 그 글자 범위(문자 인덱스)를 돌려준다.
fn mark_find(job: &mut LayoutJob, style: &Style) -> Option<Range<usize>> {
    let find = style.find.as_ref()?;
    let block = find.block.get()?;
    let matches = find.query.find_all(&job.text);
    if matches.is_empty() {
        return None;
    }
    let mut current = None;
    let mut paints = Vec::with_capacity(matches.len());
    for range in matches {
        let n = find.ordinal.get();
        find.ordinal.set(n + 1);
        if find.current == Some((block, n)) {
            current = byte_ranges_to_char_ranges(&job.text, std::slice::from_ref(&range)).pop();
            paints.push((range, find.current_color));
        } else {
            paints.push((range, find.match_color));
        }
    }
    paint_backgrounds(job, &paints);
    current
}

/// `job`의 글자 범위(바이트, 오름차순·겹침 없음)에 배경색을 칠한다 — 걸친 구간(section)을 나눈다.
fn paint_backgrounds(job: &mut LayoutJob, paints: &[(Range<usize>, Color32)]) {
    let mut sections = Vec::with_capacity(job.sections.len() + paints.len() * 2);
    for section in job.sections.drain(..) {
        let (mut pos, end) = (section.byte_range.start.0, section.byte_range.end.0);
        let mut leading_space = section.leading_space;
        let mut push = |range: Range<usize>, background: Option<Color32>| {
            let mut format = section.format.clone();
            if let Some(color) = background {
                format.background = color;
            }
            sections.push(egui::text::LayoutSection {
                leading_space: std::mem::take(&mut leading_space),
                byte_range: egui::text::ByteIndex(range.start)..egui::text::ByteIndex(range.end),
                format,
            });
        };
        for (range, color) in paints {
            // 빈 매치(`^` 등)는 칠할 글자가 없다
            if range.is_empty() || range.start >= end || range.end <= pos {
                continue;
            }
            let (start, stop) = (range.start.max(pos), range.end.min(end));
            if start > pos {
                push(pos..start, None);
            }
            push(start..stop, Some(*color));
            pos = stop;
        }
        if pos < end {
            push(pos..end, None);
        }
    }
    job.sections = sections;
}

/// 현재 매치(`chars`, 글자 범위)가 그려진 화면 위치를 기록한다. `origin`은 `galley`의 화면 위치.
fn note_current(style: &Style, galley: &egui::Galley, origin: egui::Pos2, chars: Range<usize>) {
    let Some(find) = &style.find else {
        return;
    };
    let rect = if chars.is_empty() {
        galley.pos_from_cursor(CCursor::new(chars.start))
    } else {
        range_rects(galley, chars)
            .into_iter()
            .reduce(|a, b| a.union(b))
            .unwrap_or(egui::Rect::NOTHING)
    };
    find.current_rect
        .set(Some(rect.translate(origin.to_vec2())));
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
            self.table_rows.clear();
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
            self.take_table_rows(&blocks[i], i, style);
            self.measured += 1;
        }
    }

    /// 방금 그린 블록이 최상위 표면 [`table`]이 남긴 행 높이를 보관한다 (목록 안의 표 등은 버린다).
    fn take_table_rows(&mut self, block: &Block, index: usize, style: &Style) {
        if let Some(rows) = style.table_rows.take()
            && matches!(block.kind, BlockKind::Table { .. })
        {
            self.table_rows.insert(self.keys[index], rows);
        }
    }

    /// 줄 ↔ y 위치표: [`Self::positions`]에서 최상위 표를 행마다 나눈다. 표 행은 길이에 따라 높이가 크게 달라
    /// 블록 하나로 두고 줄 비율로 나누면 큰 표 근처에서 분할 스크롤이 어긋난다. GFM 표는 행마다 소스 한 줄이다
    /// (머리글 + 구분 줄 = 머리글 행, 그 뒤 한 줄 = 한 행). 줄 수가 맞지 않거나 아직 그리지 않았으면 블록 그대로.
    fn line_map(&self, blocks: &[Block], positions: &[BlockPos]) -> Vec<BlockPos> {
        let mut map = Vec::with_capacity(positions.len());
        for (i, (block, &pos)) in blocks.iter().zip(positions).enumerate() {
            let rows = self
                .table_rows
                .get(&self.keys[i])
                .filter(|rows| block.last_line + 1 - block.first_line == rows.len() + 1);
            let Some(rows) = rows else {
                map.push(pos);
                continue;
            };
            let mut top = pos.top;
            for (r, &height) in rows.iter().enumerate() {
                let (first_line, last_line) = if r == 0 {
                    (block.first_line, block.first_line + 1)
                } else {
                    (block.first_line + 1 + r, block.first_line + 1 + r)
                };
                // 마지막 행은 블록 끝(가로 스크롤 여백 등)까지
                let height = if r + 1 == rows.len() {
                    (pos.top + pos.height - top).max(0.0)
                } else {
                    height
                };
                map.push(BlockPos {
                    first_line,
                    last_line,
                    top,
                    height,
                });
                top += height;
            }
        }
        map
    }

    /// 원문 해시가 `key`인 블록 중 `index`에 가장 가까운 것.
    fn find(&self, key: u64, index: usize) -> Option<usize> {
        if self.keys.get(index) == Some(&key) {
            return Some(index);
        }
        (1..self.keys.len()).find_map(|d| {
            let before = index
                .checked_sub(d)
                .filter(|&i| self.keys.get(i) == Some(&key));
            let after = Some(index + d).filter(|&i| self.keys.get(i) == Some(&key));
            before.or(after)
        })
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
            if let Some(find) = &style.find {
                find.block.set(Some(i));
                find.ordinal.set(0);
            }
            render_block(&mut child, block, style, 0);
            if let Some(find) = &style.find {
                find.block.set(None);
            }
            self.take_table_rows(block, i, style);
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
                    let mut job = LayoutJob::simple(
                        text.to_owned(),
                        FontId::monospace(style.body),
                        color,
                        f32::INFINITY,
                    );
                    let current = mark_find(&mut job, style);
                    let galley = ui.fonts_mut(|f| f.layout_job(job));
                    let response = ui.add(egui::Label::new(Arc::clone(&galley)).extend());
                    if let Some(chars) = current {
                        note_current(style, &galley, response.rect.min, chars);
                    }
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

    // 실제로 그릴 칸 (찾기 매치는 여기서만 칠한다 — 위의 내용 폭 측정은 칠하지 않음)
    let mut current_cell = None;
    let mut galleys: Vec<Vec<Arc<egui::Galley>>> = Vec::with_capacity(row_count);
    for r in 0..row_count {
        let mut row = Vec::with_capacity(columns);
        for (c, &cap) in caps.iter().enumerate() {
            let mut job = inline_job(ui, cell(r, c), style.body, r == 0, style);
            job.wrap.max_width = cap;
            if let Some(chars) = mark_find(&mut job, style) {
                current_cell = Some((r, c, chars));
            }
            row.push(ui.fonts_mut(|f| f.layout_job(job)));
        }
        galleys.push(row);
    }
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
    style.table_rows.replace(Some(heights.clone()));

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
                    if let Some((_, _, chars)) = current_cell
                        .clone()
                        .filter(|&(cr, cc, _)| (cr, cc) == (r, c))
                    {
                        note_current(style, galley, pos, chars);
                    }
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

/// 글자 라벨 안의 누를 수 있는 구간 (글자 범위).
#[derive(Default)]
struct Marks {
    /// (범위, 각주 이름)
    footnotes: Vec<(Range<usize>, String)>,
    /// (범위, 링크 주소) — 같은 링크가 이어지는 스팬은 하나로
    links: Vec<(Range<usize>, String)>,
}

/// [`inline_job`]과 같고, 각주 참조·링크의 글자 범위를 함께 돌려준다.
fn inline_job_marked(
    ui: &egui::Ui,
    spans: &[Span],
    size: f32,
    all_strong: bool,
    style: &Style,
) -> (LayoutJob, Marks) {
    let visuals = ui.visuals();
    let text_color = visuals.override_text_color.unwrap_or(visuals.text_color());
    let weak = visuals.weak_text_color();
    let mut job = LayoutJob::default();
    let mut marks = Marks::default();
    let mut chars = 0;
    for span in spans {
        let s = span.style;
        let strong = s.strong || all_strong;
        let linked = span.link.is_some();
        let link_color = match &span.link {
            Some(url) if style.link_broken(url) => style.error,
            _ => visuals.hyperlink_color,
        };
        let mut format = TextFormat {
            font_id: FontId::proportional(size),
            color: if linked {
                link_color
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
            format.underline = Stroke::new(1.0, link_color);
        }
        // 글자는 [`span_text`] (찾기 매치 세기와 같은 글자), 여기서는 모양만 정한다
        match &span.kind {
            SpanKind::Text => {}
            SpanKind::Code => {
                // 인라인 코드는 본문 크기의 고정폭 + 배경
                format.font_id = FontId::new(style.body.min(size), FontFamily::Monospace);
                format.background = style.code_bg;
            }
            // 원시 HTML은 실행하지 않고 회색 원문으로, `<br>`만 줄바꿈 (§3.8-4)
            SpanKind::Html if is_br(&span.text) => {}
            SpanKind::Html => {
                format.font_id = FontId::new(style.body.min(size), FontFamily::Monospace);
                format.color = weak;
            }
            // 문단 밖(제목·표 칸)의 이미지는 대체 텍스트로 (문단 안 이미지는 [`inline_text`]가 그린다)
            SpanKind::Image { .. } => format.color = weak,
            SpanKind::FootnoteRef => {
                format.font_id = FontId::proportional(size * 0.75);
                format.color = visuals.hyperlink_color;
                format.valign = Align::TOP;
            }
        }
        let text = span_text(span, style.footnotes);
        let len = text.chars().count();
        if span.kind == SpanKind::FootnoteRef {
            marks
                .footnotes
                .push((chars..chars + len, span.text.clone()));
        } else if let Some(url) = &span.link {
            match marks.links.last_mut() {
                Some((range, last)) if last == url && range.end == chars => range.end += len,
                _ => marks.links.push((chars..chars + len, url.clone())),
            }
        }
        chars += len;
        job.append(&text, 0.0, format);
    }
    job.wrap.max_width = ui.available_width();
    (job, marks)
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

/// 글자 라벨. 각주 참조 번호는 눌러서 각주로, 링크는 눌러서 대상으로 이동한다 (마우스를 올리면 주소 툴팁).
fn rich_label(ui: &mut egui::Ui, spans: &[Span], size: f32, all_strong: bool, style: &Style) {
    let (mut job, marks) = inline_job_marked(ui, spans, size, all_strong, style);
    let current = mark_find(&mut job, style);
    if marks.footnotes.is_empty() && marks.links.is_empty() && current.is_none() {
        ui.label(job);
        return;
    }
    let galley = ui.fonts_mut(|f| f.layout_job(job));
    let response = ui.add(egui::Label::new(Arc::clone(&galley)));
    if let Some(chars) = current {
        note_current(style, &galley, response.rect.min, chars);
    }
    let origin = response.rect.min.to_vec2();
    for (range, url) in marks.links {
        let tip = if style.link_broken(&url) {
            format!("대상을 찾을 수 없음: {url}")
        } else {
            url.clone()
        };
        // 줄이 바뀌는 링크는 줄마다 누를 수 있는 영역을 둔다
        for (row, rect) in range_rects(&galley, range.clone()).into_iter().enumerate() {
            let hit = ui
                .interact(
                    rect.translate(origin),
                    link_id(ui, &galley, ("md-link", range.start, row)),
                    Sense::click(),
                )
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text(&tip);
            if hit.clicked() {
                *style.jump.borrow_mut() = Some(Jump::Link(url.clone()));
            }
        }
    }
    for (range, label) in marks.footnotes {
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
                link_id(ui, &galley, ("md-fnref", range.start)),
                Sense::click(),
            )
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text("각주로 이동");
        if hit.clicked() {
            *style.jump.borrow_mut() = Some(Jump::ToFootnote(label));
        }
    }
}

/// 라벨 안 누를 수 있는 구간의 위젯 ID. 프레임마다 같아야 클릭이 잡히므로(자동 ID는 첫 프레임에 바뀔 수 있음)
/// 범위(scope) ID + 라벨 글자 + 구간으로 만든다 — 같은 목록의 항목들은 범위 ID가 같아 글자로 구분한다.
fn link_id(ui: &egui::Ui, galley: &egui::Galley, part: impl Hash + std::fmt::Debug) -> egui::Id {
    ui.scope_id().with((galley.text(), part))
}

/// 글자 범위가 차지하는 줄별 사각형 (galley 좌표).
fn range_rects(galley: &egui::Galley, range: Range<usize>) -> Vec<egui::Rect> {
    let mut rects: Vec<egui::Rect> = Vec::new();
    for c in range {
        let here = galley.pos_from_cursor(CCursor::new(c));
        let next = galley.pos_from_cursor(CCursor::new(c + 1));
        // 다음 글자가 같은 줄이면 거기까지, 줄 끝 글자면 글자 높이의 절반 폭
        let right = if (next.min.y - here.min.y).abs() < 0.5 && next.min.x > here.min.x {
            next.min.x
        } else {
            here.min.x + here.height() / 2.0
        };
        let rect = egui::Rect::from_min_max(here.min, egui::pos2(right, here.max.y));
        match rects.last_mut() {
            Some(last) if (last.min.y - rect.min.y).abs() < 0.5 => *last = last.union(rect),
            _ => rects.push(rect),
        }
    }
    rects
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

    /// 찾기 프레임: (현재 매치 색으로 칠한 글자들, 나머지 매치 색으로 칠한 조각 수)
    fn find_frame(
        ctx: &egui::Context,
        state: &mut PreviewState,
        doc: &Document,
    ) -> (Vec<String>, usize) {
        let palette = Palette::light();
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(600.0, 800.0))),
            ..Default::default()
        };
        let mut full = ctx.run_ui(raw, |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                state.show(ui, doc, 1.0, &palette, None);
            });
        });
        full.textures_delta.clear();
        let (mut current, mut others) = (Vec::new(), 0);
        for clipped in &full.shapes {
            if let egui::Shape::Text(t) = &clipped.shape {
                let job = &t.galley.job;
                for section in &job.sections {
                    let range = section.byte_range.start.0..section.byte_range.end.0;
                    if section.format.background == palette.search_current {
                        current.push(job.text[range].to_owned());
                    } else if section.format.background == palette.search_match {
                        others += 1;
                    }
                }
            }
        }
        (current, others)
    }

    fn find_query(pattern: &str) -> (SearchQuery, &str, SearchOptions) {
        let options = SearchOptions::default();
        (
            SearchQuery::new(pattern, &options).unwrap(),
            pattern,
            options,
        )
    }

    /// 세는 쪽(`count_matches`)과 칠하는 쪽(`mark_find`)이 같은 매치를 본다: 모든 번호의 매치가 정확히 한 번
    /// 현재 매치로 칠해지고, 원시 HTML 블록은 찾지 않는다.
    #[test]
    fn preview_find_counts_and_paints_the_same_matches() {
        let text = "# 검색 제목\n\n\
                    문단의 검색 단어와 `검색` 코드, [검색 링크](https://example.com) 그리고 각주[^1].\n\n\
                    - 목록 검색\n  - 중첩 검색\n\n\
                    > 인용 검색\n\n\
                    ```rust\nlet 검색 = 1;\n```\n\n\
                    | 머리 | 검색 |\n| --- | --- |\n| 검색 |\n\n\
                    <div>검색 html</div>\n\n\
                    [^1]: 각주 검색\n";
        let d = doc(text);
        let ctx = egui::Context::default();
        let mut state = PreviewState::new(1);
        frame(&ctx, &mut state, &d);
        assert_eq!(state.find_status(), None, "찾기 전");

        state.set_find(Some(find_query("검색")));
        for _ in 0..3 {
            find_frame(&ctx, &mut state, &d);
        }
        // 제목 1 · 문단 3(글자·코드·링크) · 목록 2 · 인용 1 · 코드 1 · 표 2 · 각주 1 (HTML 블록 제외)
        assert_eq!(state.find_status(), Some((Some(0), 11)));

        for i in 0..11 {
            let (current, others) = find_frame(&ctx, &mut state, &d);
            assert_eq!(current, ["검색"], "{i}번째 매치가 현재 매치로 칠해진다");
            assert_eq!(others, 10, "{i}: 나머지 매치");
            assert_eq!(state.find_status(), Some((Some(i), 11)));
            let step = state.find_step(true, true);
            assert_eq!(
                step,
                if i == 10 {
                    PreviewStep::Wrapped
                } else {
                    PreviewStep::Moved
                }
            );
            find_frame(&ctx, &mut state, &d);
        }
        assert_eq!(state.find_status(), Some((Some(0), 11)));
        assert_eq!(state.find_step(false, false), PreviewStep::End);

        // 끄면 강조도 사라진다
        state.set_find(None);
        let (current, others) = find_frame(&ctx, &mut state, &d);
        assert!(current.is_empty() && others == 0);
        assert_eq!(state.find_step(true, true), PreviewStep::NoMatch);
    }

    #[test]
    fn preview_find_scrolls_to_match_far_below() {
        let mut text = String::new();
        for i in 0..300 {
            text.push_str(&format!("{i}번째 문단 내용입니다.\n\n"));
        }
        text.push_str("마지막 목표 문단\n");
        let d = doc(&text);
        let ctx = egui::Context::default();
        let mut state = PreviewState::new(1);
        frame(&ctx, &mut state, &d);
        assert_eq!(state.offset_y(), 0.0);

        state.set_find(Some(find_query("목표")));
        let mut current = Vec::new();
        for _ in 0..6 {
            current = find_frame(&ctx, &mut state, &d).0;
        }
        assert_eq!(state.find_status(), Some((Some(0), 1)));
        assert!(
            state.offset_y() > 1000.0,
            "매치 블록으로 스크롤 — {}",
            state.offset_y()
        );
        assert_eq!(current, ["목표"], "화면에 현재 매치가 칠해져 있다");

        // 검색어가 바뀌면 다시 세고, 화면 맨 위 이후 첫 매치로
        state.set_find(Some(find_query("문단")));
        for _ in 0..3 {
            find_frame(&ctx, &mut state, &d);
        }
        let (index, total) = state.find_status().unwrap();
        assert_eq!(total, 301);
        assert!(index.unwrap() > 200, "화면 맨 위 이후 첫 매치 — {index:?}");
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
        // 편집이 멈추고 디바운스(한 프레임 = 1/60초)가 지나야 다시 파싱한다
        let mut frames = 0;
        let texts = loop {
            frames += 1;
            let texts = frame(&ctx, &mut state, &d);
            if state.parse_count() == 2 || frames > 60 {
                break texts;
            }
        };
        assert_eq!(state.parse_count(), 2);
        assert!(
            frames as f64 / 60.0 >= REFRESH_DEBOUNCE,
            "디바운스 전에 파싱함: {frames}프레임"
        );
        assert!(texts.iter().any(|t| t == "둘"));
    }

    /// 지정한 시각(초)의 헤드리스 프레임. 그려진 글자를 돌려준다.
    fn frame_at(
        ctx: &egui::Context,
        state: &mut PreviewState,
        doc: &Document,
        time: f64,
    ) -> Vec<String> {
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(600.0, 800.0))),
            time: Some(time),
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

    #[test]
    fn refresh_waits_until_typing_pauses() {
        let ctx = egui::Context::default();
        let mut state = PreviewState::new(1);
        let mut d = doc("가\n");
        frame_at(&ctx, &mut state, &d, 1.0);
        // 0.1초 간격으로 계속 입력하는 동안은 이전 결과
        for (i, text) in ["가나\n", "가나다\n", "가나다라\n"].iter().enumerate() {
            d.set_text(*text);
            let t = 1.1 + 0.1 * i as f64;
            let texts = frame_at(&ctx, &mut state, &d, t);
            assert!(
                texts.iter().any(|x| x == "가"),
                "{t}초: 아직 이전 결과여야 함"
            );
        }
        // 마지막 입력(1.3초) 뒤 0.1초: 아직
        assert!(
            frame_at(&ctx, &mut state, &d, 1.4)
                .iter()
                .any(|x| x == "가")
        );
        // 디바운스가 지나면 한 번에 최신 내용
        let texts = frame_at(&ctx, &mut state, &d, 1.3 + REFRESH_DEBOUNCE + 0.01);
        assert!(texts.iter().any(|x| x == "가나다라"), "{texts:?}");
        assert_eq!(state.parse_count(), 2, "중간 입력은 파싱하지 않음");
    }

    #[test]
    fn editing_above_keeps_visible_content_in_place() {
        let ctx = egui::Context::default();
        let mut state = PreviewState::new(1);
        let paragraphs: Vec<String> = (0..200).map(|i| format!("문단 {i}")).collect();
        let mut d = doc(&(paragraphs.join("\n\n") + "\n"));
        for _ in 0..3 {
            frame(&ctx, &mut state, &d);
        }
        // 문단 100(소스 줄 200)을 화면 맨 위로
        let target = state.positions()[100];
        state.set_scroll_offset(target.top);
        for _ in 0..3 {
            frame(&ctx, &mut state, &d);
        }
        let before = state.top_line().unwrap();
        assert!((before - 200.0).abs() < 1.0, "맨 위 줄 {before}");

        // 화면 위쪽 문단 10을 20줄짜리 긴 문단으로 바꾼다 (소스 줄 +19)
        let mut edited = paragraphs.clone();
        edited[10] = (0..20)
            .map(|i| format!("긴 문단 {i}줄"))
            .collect::<Vec<_>>()
            .join("\n");
        d.set_text(edited.join("\n\n") + "\n");
        for _ in 0..20 {
            frame(&ctx, &mut state, &d);
        }
        assert_eq!(state.parse_count(), 2);
        let after = state.top_line().unwrap();
        assert!(
            (after - (before + 19.0)).abs() < 1.0,
            "같은 문단이 맨 위에 남아야 함: {before} → {after}"
        );
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

        // 첫 문단 한 곳만 고친다 → (디바운스 뒤 다시 파싱한 프레임에) 그 블록만 다시 잰다
        d.set_text(text.replacen("문단에 **굵게**", "문단을 고쳐 **굵게**", 1));
        let mut measured = None;
        for _ in 0..60 {
            galleys_frame(&ctx, &mut state, &d, Vec::new());
            if state.parse_count() == 2 {
                measured = Some(state.last_frame_counts().0);
                break;
            }
        }
        let measured = measured.expect("다시 파싱하지 않음");
        assert!(
            (1..=2).contains(&measured),
            "바뀐 블록만 다시 재야 함: {measured}"
        );
    }

    /// 100KB 편집 → 미리보기 반영 시간 (PLAN.md §1.3 "≤ 디바운스 + 50ms") —
    /// `cargo test --release --lib -- --ignored --nocapture preview_refresh_time`
    #[test]
    #[ignore = "측정용: 릴리스 빌드로 실행"]
    fn preview_refresh_time_100kb() {
        let ctx = egui::Context::default();
        let mut state = PreviewState::new(1);
        let text = long_markdown(100 * 1024);
        let mut d = doc(&text);
        // 실제 경과 시간으로 프레임을 돌린다 (디바운스가 실제 시간 기준으로 지나가게)
        let clock = std::time::Instant::now();
        let now = || 1.0 + clock.elapsed().as_secs_f64();
        for _ in 0..3 {
            frame_at(&ctx, &mut state, &d, now());
        }
        let mut report = Vec::new();
        for n in 0..5 {
            let marker = format!("반영확인{n}");
            let edited = text.replacen("같은 문단의 둘째 줄이다.", &marker, 1 + n * 3);
            d.set_text(&edited);
            let start = std::time::Instant::now();
            let mut work = 0.0;
            loop {
                let t = std::time::Instant::now();
                let texts = frame_at(&ctx, &mut state, &d, now());
                let frame_ms = t.elapsed().as_secs_f64() * 1000.0;
                if texts.iter().any(|x| x.contains(&marker))
                    || state.parse_count() as usize == n + 2
                {
                    work = frame_ms;
                    break;
                }
                // 60fps로 다시 그리는 것처럼 프레임 사이를 띄운다
                std::thread::sleep(std::time::Duration::from_millis(16));
                if start.elapsed().as_secs() > 5 {
                    break;
                }
            }
            report.push(format!(
                "반영 {:.0}ms(갱신 프레임 {work:.1}ms)",
                start.elapsed().as_secs_f64() * 1000.0
            ));
        }
        println!(
            "100KB 편집 → 미리보기 반영 (디바운스 {:.0}ms, {} B): {}",
            REFRESH_DEBOUNCE * 1000.0,
            text.len(),
            report.join(" · ")
        );
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

    /// 저장소 링크 픽스처 폴더의 문서를 연다.
    fn links_doc(name: &str) -> Document {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/markdown/links")
            .join(name);
        let bytes = std::fs::read(&path).unwrap();
        Document::from_bytes(Some(path), &bytes, false, None).unwrap()
    }

    /// 헤드리스 프레임 (입력 포함): (결과, 그려진 글자 (글자, 위치, galley)).
    fn output_frame(
        ctx: &egui::Context,
        state: &mut PreviewState,
        doc: &Document,
        events: Vec<egui::Event>,
    ) -> (PreviewOutput, Vec<(String, Pos2, Arc<egui::Galley>)>) {
        let raw = RawInput {
            events,
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(600.0, 800.0))),
            ..Default::default()
        };
        let mut output = PreviewOutput::default();
        let mut full = ctx.run_ui(raw, |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                output = state.show(ui, doc, 1.0, &Palette::light(), None);
            });
        });
        full.textures_delta.clear();
        let texts = full
            .shapes
            .iter()
            .filter_map(|c| match &c.shape {
                egui::Shape::Text(t) => {
                    Some((t.galley.text().to_owned(), t.pos, Arc::clone(&t.galley)))
                }
                _ => None,
            })
            .collect();
        (output, texts)
    }

    /// `needle` 글자의 화면 가운데 (그 글자를 포함한 라벨에서 찾는다).
    fn text_point(texts: &[(String, Pos2, Arc<egui::Galley>)], needle: &str) -> Pos2 {
        let (text, pos, galley) = texts
            .iter()
            .find(|t| t.0.contains(needle))
            .unwrap_or_else(|| panic!("{needle:?} 글자가 없음"));
        let start = text[..text.find(needle).unwrap()].chars().count();
        let end = start + needle.chars().count();
        let a = galley.pos_from_cursor(CCursor::new(start));
        let b = galley.pos_from_cursor(CCursor::new(end));
        *pos + egui::vec2((a.min.x + b.min.x) / 2.0, a.center().y)
    }

    /// `needle` 링크를 누르고 그 프레임의 결과를 돌려준다.
    fn click_link(
        ctx: &egui::Context,
        state: &mut PreviewState,
        doc: &Document,
        needle: &str,
    ) -> PreviewOutput {
        // 앞선 클릭과 이어진 두 번 클릭으로 보지 않게 잠시 그린다
        for _ in 0..30 {
            output_frame(ctx, state, doc, Vec::new());
        }
        let (_, texts) = output_frame(ctx, state, doc, Vec::new());
        let point = text_point(&texts, needle);
        output_frame(ctx, state, doc, click_events(point)).0
    }

    #[test]
    fn link_clicks_report_external_file_and_missing_targets() {
        let ctx = egui::Context::default();
        let mut state = PreviewState::new(1);
        let d = links_doc("index.md");
        let dir = d.path().unwrap().parent().unwrap().to_path_buf();
        assert_eq!(
            click_link(&ctx, &mut state, &d, "GitHub 저장소").link,
            Some(LinkAction::OpenUrl(
                "https://github.com/victoriatech25/my-notepad".into()
            ))
        );
        assert_eq!(
            click_link(&ctx, &mut state, &d, "다른 문서의 \"세 번째 절\"").link,
            Some(LinkAction::OpenFile {
                path: dir.join("other.md"),
                anchor: Some("세-번째-절".into()),
            })
        );
        assert_eq!(
            click_link(&ctx, &mut state, &d, "공백 이름").link,
            Some(LinkAction::OpenFile {
                path: dir.join("공백 있는 이름.md"),
                anchor: None,
            })
        );
        assert!(matches!(
            click_link(&ctx, &mut state, &d, "없는 파일").link,
            Some(LinkAction::Notice(text)) if text.contains("없는-파일.md")
        ));
        assert!(matches!(
            click_link(&ctx, &mut state, &d, "없는 제목").link,
            Some(LinkAction::Notice(text)) if text.contains("#없는-제목")
        ));
    }

    #[test]
    fn missing_link_target_is_drawn_in_error_color() {
        let ctx = egui::Context::default();
        let mut state = PreviewState::new(1);
        let d = links_doc("index.md");
        let (_, texts) = output_frame(&ctx, &mut state, &d, Vec::new());
        let color_of = |needle: &str| {
            let (text, _, galley) = texts.iter().find(|t| t.0.contains(needle)).unwrap();
            let at = text.find(needle).unwrap();
            galley
                .job
                .sections
                .iter()
                .find(|s| s.byte_range.start.0 <= at && at < s.byte_range.end.0)
                .unwrap()
                .format
                .color
        };
        assert_eq!(color_of("없는 파일"), Palette::light().status_error);
        assert_ne!(color_of("다른 문서"), Palette::light().status_error);
    }

    #[test]
    fn anchor_link_scrolls_to_heading() {
        let ctx = egui::Context::default();
        let mut state = PreviewState::new(1);
        let d = links_doc("index.md");
        let output = click_link(&ctx, &mut state, &d, "아래 \"긴 절\"로");
        let line = output.jumped_line.expect("앵커로 이동하지 않음");
        assert!(output.link.is_none());
        output_frame(&ctx, &mut state, &d, Vec::new());
        assert_heading_on_screen(&state, line);

        // 원문 제목을 그대로 쓴 앵커도 찾는다 (맨 위 목록의 링크이므로 새로 열어서)
        let mut state = PreviewState::new(2);
        let output = click_link(&ctx, &mut state, &d, "3.8 링크 규칙");
        assert!(output.jumped_line.is_some(), "{:?}", output.link);
    }

    /// 표의 행 높이가 크게 달라도 줄 → y가 실제로 그린 행 위치와 맞는다 (분할 스크롤 동기화 기준).
    /// 표를 블록 하나로 두고 줄 비율로 나누면 긴 행 뒤의 행이 수십 px 위로 계산된다.
    #[test]
    fn line_map_follows_table_row_heights() {
        let long = "아주 긴 칸 ".repeat(60);
        let text = format!(
            "앞 문단\n\n| 이름 | 설명 |\n| :--- | :--- |\n| 첫행 | 짧음 |\n| 긴행 | {long} |\n| 끝행 | 짧음 |\n\n뒤 문단\n"
        );
        let d = doc(&text);
        let ctx = egui::Context::default();
        let mut state = PreviewState::new(1);
        let mut texts = Vec::new();
        for _ in 0..4 {
            texts = output_frame(&ctx, &mut state, &d, Vec::new()).1;
        }
        let y_of = |needle: &str| {
            texts
                .iter()
                .find(|(t, _, _)| t == needle)
                .unwrap_or_else(|| panic!("{needle} 안 그려짐"))
                .1
                .y
        };
        // 표: 머리글(2·3줄) + 행 3개(4·5·6줄) → 위치표 4칸
        let map = state.line_map();
        assert_eq!(map.len(), state.positions().len() + 3, "{map:?}");
        let drawn = y_of("끝행") - y_of("첫행");
        let mapped = line_to_y(map, 6.0) - line_to_y(map, 4.0);
        assert!(drawn > 100.0, "긴 행이 줄바꿈되어야 함: {drawn}");
        assert!(
            (drawn - mapped).abs() <= 1.0,
            "그린 {drawn} · 위치표 {mapped}"
        );
        for line in [2.0, 3.5, 4.0, 5.25, 6.0] {
            let back = y_to_line(map, line_to_y(map, line));
            assert!((back - line).abs() < 1e-3, "{line} → {back}");
        }
    }

    /// 제목 줄이 스크롤되어 화면 안(문서 끝이라 맨 위까지 못 오면 그 아래)에 보인다.
    fn assert_heading_on_screen(state: &PreviewState, line: usize) {
        let y = line_to_y(state.line_map(), line as f32) - state.offset_y();
        assert!(state.offset_y() > 0.0, "스크롤하지 않음");
        assert!((0.0..760.0).contains(&y), "제목이 화면 밖: y {y}");
    }

    #[test]
    fn pending_anchor_scrolls_newly_opened_document() {
        let ctx = egui::Context::default();
        let mut state = PreviewState::new(1);
        let d = links_doc("other.md");
        state.jump_to_anchor("세-번째-절".into());
        let (output, _) = output_frame(&ctx, &mut state, &d, Vec::new());
        let line = output.jumped_line.expect("열자마자 앵커로");
        output_frame(&ctx, &mut state, &d, Vec::new());
        assert_heading_on_screen(&state, line);
    }

    /// 그려진 글자 (글자, 위치, galley).
    type DrawnTexts = Vec<(String, Pos2, Arc<egui::Galley>)>;

    /// 헤드리스 프레임: 복사된 글자(있으면)와 그려진 글자.
    fn copy_frame(
        ctx: &egui::Context,
        state: &mut PreviewState,
        doc: &Document,
        events: Vec<egui::Event>,
        width: f32,
    ) -> (Option<String>, DrawnTexts) {
        let raw = RawInput {
            events,
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(width, 800.0))),
            ..Default::default()
        };
        let mut full = ctx.run_ui(raw, |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                state.show(ui, doc, 1.0, &Palette::light(), None);
            });
        });
        full.textures_delta.clear();
        let copied = full.platform_output.commands.iter().find_map(|c| match c {
            egui::OutputCommand::CopyText(text) => Some(text.clone()),
            _ => None,
        });
        let texts = full
            .shapes
            .iter()
            .filter_map(|c| match &c.shape {
                egui::Shape::Text(t) => {
                    Some((t.galley.text().to_owned(), t.pos, Arc::clone(&t.galley)))
                }
                _ => None,
            })
            .collect();
        (copied, texts)
    }

    #[test]
    fn drag_selecting_preview_text_copies_it() {
        let ctx = egui::Context::default();
        let mut state = PreviewState::new(1);
        let d = doc("# 제목\n\n첫째 문단입니다.\n\n둘째 문단입니다.\n");
        let mut texts = Vec::new();
        for _ in 0..3 {
            texts = copy_frame(&ctx, &mut state, &d, Vec::new(), 600.0).1;
        }
        let first = texts.iter().find(|t| t.0 == "첫째 문단입니다.").unwrap();
        let second = texts.iter().find(|t| t.0 == "둘째 문단입니다.").unwrap();
        let from = first.1 + egui::vec2(1.0, first.2.size().y / 2.0);
        let to = second.1 + egui::vec2(second.2.size().x - 1.0, second.2.size().y / 2.0);
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        copy_frame(
            &ctx,
            &mut state,
            &d,
            vec![egui::Event::PointerMoved(from)],
            600.0,
        );
        copy_frame(&ctx, &mut state, &d, vec![button(from, true)], 600.0);
        for step in 1..=5 {
            let p = from + (to - from) * (step as f32 / 5.0);
            copy_frame(
                &ctx,
                &mut state,
                &d,
                vec![egui::Event::PointerMoved(p)],
                600.0,
            );
        }
        copy_frame(&ctx, &mut state, &d, vec![button(to, false)], 600.0);
        let (copied, _) = copy_frame(&ctx, &mut state, &d, vec![egui::Event::Copy], 600.0);
        let copied = copied.expect("복사되지 않음");
        assert!(copied.contains("첫째 문단"), "{copied:?}");
        assert!(
            copied.contains("둘째 문단"),
            "블록을 넘어 선택해야 함: {copied:?}"
        );
    }

    #[test]
    fn body_width_is_capped_and_centered_on_wide_windows() {
        let ctx = egui::Context::default();
        let mut state = PreviewState::new(1);
        let long = "아주 긴 문단 ".repeat(200);
        let d = doc(&format!("{long}\n"));
        let mut texts = Vec::new();
        for _ in 0..3 {
            texts = copy_frame(&ctx, &mut state, &d, Vec::new(), 2000.0).1;
        }
        let (_, pos, galley) = texts.iter().find(|t| t.0.starts_with("아주 긴")).unwrap();
        assert!(
            galley.size().x <= MAX_BODY_WIDTH + 1.0,
            "본문 폭 제한: {}",
            galley.size().x
        );
        // 가운데 정렬: 왼쪽 여백 ≈ 오른쪽 여백
        let left = pos.x;
        let right = 2000.0 - (pos.x + MAX_BODY_WIDTH);
        assert!(
            (left - right).abs() < 40.0,
            "가운데가 아님: 왼쪽 {left} 오른쪽 {right}"
        );

        // 좁은 창에서는 폭을 다 쓴다
        let mut state = PreviewState::new(2);
        for _ in 0..3 {
            texts = copy_frame(&ctx, &mut state, &d, Vec::new(), 600.0).1;
        }
        let (_, pos, _) = texts.iter().find(|t| t.0.starts_with("아주 긴")).unwrap();
        assert!(pos.x < 40.0);
    }

    #[test]
    fn preview_text_follows_editor_zoom() {
        let ctx = egui::Context::default();
        let d = doc("본문 글자
");
        let size_at = |zoom: f32| {
            let mut state = PreviewState::new(1);
            let mut height = 0.0;
            for _ in 0..2 {
                let raw = RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(600.0, 800.0))),
                    ..Default::default()
                };
                let mut full = ctx.run_ui(raw, |ui| {
                    egui::CentralPanel::default().show(ui, |ui| {
                        state.show(ui, &d, zoom, &Palette::light(), None);
                    });
                });
                full.textures_delta.clear();
                height = full
                    .shapes
                    .iter()
                    .find_map(|c| match &c.shape {
                        egui::Shape::Text(t) if t.galley.text() == "본문 글자" => {
                            Some(t.galley.size().y)
                        }
                        _ => None,
                    })
                    .unwrap();
            }
            height
        };
        let (normal, zoomed) = (size_at(1.0), size_at(2.0));
        assert!(
            (zoomed / normal - 2.0).abs() < 0.2,
            "확대 배율을 따라야 함: {normal} → {zoomed}"
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
