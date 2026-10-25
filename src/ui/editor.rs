//! 기본 에디터 뷰 (방식 A `TextEdit` + [`Document`] 연동 — ADR 0001, PLAN.md §3.2, P1-09).
//!
//! 설계 원칙:
//! - 텍스트의 원본은 [`Document`]이다. `TextEdit`은 탭별 [`EditorState`]가 가진 사본(`scratch`)을 편집하고,
//!   바뀌면 [`Document::apply_edit_with_cursor`]로 넘겨 바뀐 구간만 Undo 이력에 기록한다.
//!   문서가 다른 경로(Undo/Redo, 잘라내기 메뉴 등)로 바뀌면 `revision`이 달라지므로 다음 프레임에 사본을 다시 맞춘다.
//! - 실행 취소·다시 실행은 `TextEdit` 내장 Undo가 아니라 문서의 탭별 이력을 쓴다. 전역 단축키(`KeyDispatcher`)가
//!   `Ctrl/Cmd+Z`·`Ctrl+Y`·`Cmd+Shift+Z`를 먼저 소비하고, 남은 Undo 계열 키 조합은 [`filter_input_events`]가 지운다.
//! - 붙여넣기 텍스트의 `CRLF`/`CR`은 위젯에 들어가기 전에 `LF`로 바꾼다 (문서 내부는 `LF` 정규화, 커서 위치 어긋남 방지).
//! - `Tab` 키는 탭 문자를 넣고 포커스를 옮기지 않는다 (`lock_focus`). `Ctrl+Tab`은 전역 단축키(다음 탭)로 먼저 소비된다.
//! - 커서만 움직이면(입력 없이) 연속 입력 묶음을 끊어, 떨어진 위치의 입력이 한 Undo 단계로 합쳐지지 않게 한다.
//! - 줄 번호 Gutter는 스크롤 영역 밖 왼쪽에 고정하고 본문만 스크롤한다 (가로 스크롤해도 번호는 제자리, P2-01).
//!   번호는 `TextEdit`이 그린 레이아웃(galley)의 행에서 **논리 줄의 첫 행**에만 붙인다 (자동 줄바꿈으로 접힌 행은 번호 없음).
//! - 선택 단어 일괄 강조(P2-04): 선택·문서 `revision`·옵션이 바뀔 때만 [`find_matches`]로 다시 계산해 캐시하고,
//!   **화면에 보이는 매치만** 글자 뒤에 배경 사각형(`smart_highlight`)으로 그린다. 선택 자체는 강조하지 않는다.
//!   글자 레이아웃(`LayoutJob`)에 배경색 구간을 넣으면 강조가 켜지고 꺼질 때마다 매치가 있는 줄을 모두 다시
//!   레이아웃해 1MB·2천여 매치에서 한 프레임 50~60ms가 걸렸다. 사각형 방식은 레이아웃 캐시를 건드리지 않는다.

use std::ops::Range;
use std::sync::Arc;

use eframe::egui::{
    self, Event, FontId, Galley, Id, Key, Pos2, Rect, UiBuilder, ViewportCommand,
    text::{CCursor, CCursorRange, LayoutJob},
    text_edit::TextEditState,
};

use crate::commands::Command;
use crate::core::buffer::{byte_to_char_offset, char_to_byte_offset};
use crate::core::document::Document;
use crate::core::highlight::{HighlightOptions, find_matches, target_word};
use crate::core::line_ending::normalize_to_lf;
use crate::core::markdown::{EditMap, SyntaxKind, SyntaxSpans, syntax_spans};
use crate::ui::theme::Palette;

/// 에디터 표시 옵션.
#[derive(Debug, Clone)]
pub struct EditorOptions {
    /// 자동 줄바꿈 여부 (`false`면 가로 스크롤).
    pub word_wrap: bool,
    /// 본문 글꼴 (확대/축소 반영).
    pub font: FontId,
    /// 선택 단어 일괄 강조 매칭 옵션.
    pub highlight: HighlightOptions,
    /// 탭 너비 (공백 칸 수) — `Tab` 키로 공백을 넣을 때 다음 탭 위치 계산에 쓴다.
    pub tab_width: usize,
    /// `Tab` 키로 탭 문자 대신 공백을 넣는다.
    pub tab_spaces: bool,
}

/// 한 프레임의 에디터 결과.
#[derive(Debug, Clone, Copy, Default)]
pub struct EditorOutput {
    /// 이번 프레임에 문서 내용이 바뀌었는지 여부.
    pub changed: bool,
    /// 에디터가 키보드 포커스를 가졌는지 여부.
    pub has_focus: bool,
    /// 이번 프레임에 선택 단어 강조를 다시 계산했는데 매치가 상한을 넘어 강조하지 않았는지 여부.
    pub highlight_exceeded: bool,
}

/// 선택 단어 강조 캐시 (선택·문서·옵션이 그대로면 다시 계산하지 않는다).
#[derive(Debug)]
struct SmartHighlight {
    /// 계산 기준: (문서 `revision`, 선택 문자 범위, 옵션)
    key: (u64, Range<usize>, HighlightOptions),
    /// 계산 당시 문서 텍스트 길이 — 그릴 때 사본 길이가 다르면(입력 중) 그리지 않는다.
    text_len: usize,
    /// 강조할 문자 범위 (오름차순, 선택 자체 제외). 조건 미충족·상한 초과면 비어 있다.
    ranges: Arc<Vec<Range<usize>>>,
}

/// 탭별 에디터 위젯 상태 (`TextEdit` 사본, 대기 중인 커서·포커스·스크롤 요청).
#[derive(Debug)]
pub struct EditorState {
    id: Id,
    /// `TextEdit`이 편집하는 문서 텍스트 사본.
    scratch: String,
    /// `scratch`가 맞춰진 문서 `revision` (`None`이면 아직 맞추지 않음).
    synced_revision: Option<u64>,
    /// 다음 프레임에 `TextEdit`에 적용할 커서/선택 (문자 인덱스).
    pending_cursor: Option<Range<usize>>,
    /// 다음 프레임에 복원할 세로 스크롤 위치.
    pending_scroll_y: Option<f32>,
    /// 다음 프레임에 커서 줄이 화면 가운데에 오도록 스크롤한다 (줄 이동).
    center_cursor: bool,
    focus_requested: bool,
    highlight: Option<SmartHighlight>,
    /// 다음 프레임에 이 소스 줄(0부터, 소수 = 줄 안 위치)이 화면 맨 위에 오게 한다 (보기 모드 전환, P3-06).
    pending_top_line: Option<f32>,
    /// 마지막으로 그린 레이아웃과 화면 맨 위의 galley y — 화면 맨 위 줄 계산용.
    last_view: Option<LastView>,
    /// Markdown 구문 색상을 칠할지 (`.md` 문서, P3-10)
    markdown_syntax: bool,
    /// 마지막으로 계산한 구문 색상과 아직 반영하지 않은 편집이 처음 보인 시각
    syntax: Option<Arc<SyntaxState>>,
    syntax_dirty_since: Option<f64>,
}

/// 구문 색상 구간과 그 기준 텍스트 (편집 중에는 기준 텍스트와의 차이만큼 구간을 옮겨 쓴다).
#[derive(Debug)]
struct SyntaxState {
    revision: u64,
    text: String,
    spans: SyntaxSpans,
}

/// 구문 색상을 다시 계산하기까지 기다리는 시간 (초) — 미리보기 갱신과 같다.
const SYNTAX_DEBOUNCE: f64 = 0.15;
/// 이보다 큰 문서는 구문 색상을 칠하지 않는다 (ADR 0001: 1MB 초과 편집은 느림 안내 구간).
pub const SYNTAX_MAX_BYTES: usize = 1 << 20;
/// 구문 색상은 지난 프레임에 보인 부분 앞뒤로 이만큼(바이트)만 칠한다 — 구간(섹션)이 많으면 egui가
/// 레이아웃 캐시를 찾을 때마다 섹션을 모두 훑어 1MB 문서 입력이 느려진다 (P3-10 측정).
const SYNTAX_WINDOW_MARGIN: usize = 16 * 1024;

/// 구문 색상 종류별 색 (팔레트에서).
#[derive(Debug, Clone, Copy)]
struct SyntaxColors {
    heading: egui::Color32,
    emphasis: egui::Color32,
    strong: egui::Color32,
    code: egui::Color32,
    link: egui::Color32,
    list: egui::Color32,
    quote: egui::Color32,
    meta: egui::Color32,
}

#[derive(Debug)]
struct LastView {
    galley: Arc<Galley>,
    top_in_galley: f32,
    /// 보이는 영역 높이
    view_height: f32,
    /// 세로 스크롤 최대값 (내용 높이 − 보이는 높이)
    max_offset: f32,
}

impl EditorState {
    /// `id_salt`는 탭마다 달라야 한다 (위젯 상태·스크롤 위치가 탭별로 저장됨).
    pub fn new(id_salt: impl std::hash::Hash + std::fmt::Debug) -> Self {
        Self {
            id: Id::unique(("editor", id_salt)),
            scratch: String::new(),
            synced_revision: None,
            pending_cursor: None,
            pending_scroll_y: None,
            center_cursor: false,
            focus_requested: false,
            highlight: None,
            pending_top_line: None,
            last_view: None,
            markdown_syntax: false,
            syntax: None,
            syntax_dirty_since: None,
        }
    }

    /// Markdown 구문 색상을 켜고 끈다 (`.md` 문서의 소스 보기에서 켠다).
    pub fn set_markdown_syntax(&mut self, on: bool) {
        self.markdown_syntax = on;
        if !on {
            self.syntax = None;
            self.syntax_dirty_since = None;
        }
    }

    /// 구문 색상을 칠할 바이트 범위: 지난 프레임에 보인 부분 앞뒤로 [`SYNTAX_WINDOW_MARGIN`]씩.
    fn syntax_window(&self, doc: &Document) -> Range<usize> {
        let text = doc.text();
        let Some(view) = &self.last_view else {
            return 0..text.len().min(2 * SYNTAX_WINDOW_MARGIN);
        };
        let top = view.top_in_galley.max(0.0);
        let first = view.galley.cursor_from_pos(egui::vec2(0.0, top)).index.0;
        let last = view
            .galley
            .cursor_from_pos(egui::vec2(f32::INFINITY, top + view.view_height))
            .index
            .0;
        let (start, end) = char_range_to_bytes(text, first, last);
        start.saturating_sub(SYNTAX_WINDOW_MARGIN)..(end + SYNTAX_WINDOW_MARGIN).min(text.len())
    }

    /// 구문 색상을 문서에 맞춘다. 편집이 멈추고 [`SYNTAX_DEBOUNCE`]가 지나면 다시 계산한다.
    fn update_syntax(&mut self, doc: &Document, ctx: &egui::Context) -> Option<Arc<SyntaxState>> {
        if !self.markdown_syntax || doc.text().len() > SYNTAX_MAX_BYTES {
            self.syntax = None;
            return None;
        }
        let now = ctx.input(|i| i.time);
        let stale = self
            .syntax
            .as_ref()
            .is_none_or(|s| s.revision != doc.revision());
        if stale {
            let since = *self.syntax_dirty_since.get_or_insert(now);
            let waited = now - since;
            if self.syntax.is_none() || waited >= SYNTAX_DEBOUNCE {
                self.syntax = Some(Arc::new(SyntaxState {
                    revision: doc.revision(),
                    text: doc.text().to_owned(),
                    spans: syntax_spans(doc.text()),
                }));
                self.syntax_dirty_since = None;
            } else {
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(
                    SYNTAX_DEBOUNCE - waited,
                ));
            }
        }
        self.syntax.clone()
    }

    /// `TextEdit` 위젯 ID.
    pub fn id(&self) -> Id {
        self.id
    }

    /// 문서 자체가 바뀌었으므로(같은 `revision`이어도) 다음 프레임에 사본을 다시 맞춘다.
    pub fn invalidate(&mut self) {
        self.synced_revision = None;
    }

    /// 다음 프레임에 에디터로 키보드 포커스를 옮긴다.
    pub fn request_focus(&mut self) {
        self.focus_requested = true;
    }

    /// 문서에 저장된 커서·스크롤 위치를 다음 프레임에 위젯으로 되돌린다 (탭 전환 시).
    pub fn restore_view(&mut self, doc: &Document) {
        self.pending_cursor = Some(doc.cursor_char_range());
        self.pending_scroll_y = Some(doc.scroll_offset_y());
    }

    /// 마지막으로 그린 화면 맨 위의 소스 줄 (0부터, 소수 = 줄 안 위치). 아직 그린 적이 없으면 `None`.
    pub fn top_line(&self, doc: &Document) -> Option<f32> {
        self.line_at(doc, 0.0)
    }

    /// 마지막으로 그린 화면에서 위로부터 `dy`만큼 내려간 곳의 소스 줄 (0부터, 소수 = 줄 안 위치).
    pub fn line_at(&self, doc: &Document, dy: f32) -> Option<f32> {
        let view = self.last_view.as_ref()?;
        let y = view.top_in_galley + dy;
        let cursor = view.galley.cursor_from_pos(egui::vec2(0.0, y.max(0.0)));
        let byte = char_to_byte_offset(doc.text(), cursor.index.0);
        let line = doc.line_index().line_of_byte(byte);
        let (top, bottom) = line_span(doc, &view.galley, line);
        let fraction = if bottom > top {
            ((y - top) / (bottom - top)).clamp(0.0, 1.0)
        } else {
            0.0
        };
        Some(line as f32 + fraction)
    }

    /// 마지막 프레임의 (스크롤 비율 0~1, 보이는 높이). 스크롤할 내용이 없으면 비율은 0.
    pub fn scroll_fraction(&self, doc: &Document) -> Option<(f32, f32)> {
        let view = self.last_view.as_ref()?;
        let fraction = if view.max_offset > 0.0 {
            (doc.scroll_offset_y() / view.max_offset).clamp(0.0, 1.0)
        } else {
            0.0
        };
        Some((fraction, view.view_height))
    }

    /// 다음에 그릴 때 `line`(0부터, 소수 = 줄 안 위치)이 화면 맨 위에 오도록 스크롤한다.
    ///
    /// 새 레이아웃(폭이 바뀌었을 수 있다)으로 위치를 계산해야 하므로, 그 프레임은 보이지 않게 그리고 다음 프레임에 맞춘다.
    pub fn scroll_to_line(&mut self, line: f32) {
        self.pending_top_line = Some(line.max(0.0));
    }

    /// 보이는 화면 그대로(폭이 같을 때) `line`이 위로부터 `dy` 위치에 오는 세로 스크롤 위치
    /// (0 미만이면 0). 아직 그린 적이 없으면 `None`.
    pub fn offset_for_line(&self, doc: &Document, line: f32, dy: f32) -> Option<f32> {
        let view = self.last_view.as_ref()?;
        let line = line.max(0.0);
        let (top, bottom) = line_span(doc, &view.galley, line.floor() as usize);
        let y = top + (bottom - top) * line.fract();
        Some((doc.scroll_offset_y() + y - view.top_in_galley - dy).max(0.0))
    }

    /// 다음 프레임에 세로 스크롤 위치를 `y`로 맞춘다 (스크롤 동기화, P3-08).
    pub fn set_scroll_offset(&mut self, y: f32) {
        self.pending_scroll_y = Some(y.max(0.0));
    }

    /// 다음 프레임에 적용할 스크롤 요청이 있는지 (그 프레임의 스크롤 변화는 사용자 조작이 아니다).
    pub fn has_pending_scroll(&self) -> bool {
        self.pending_scroll_y.is_some() || self.pending_top_line.is_some() || self.center_cursor
    }

    /// 보기 모드 전환으로 위치를 다시 맞추는 중인지 (숨겨 그리는 프레임).
    pub fn is_repositioning(&self) -> bool {
        self.pending_top_line.is_some()
    }

    /// `line`(1-기반) 줄 처음으로 커서를 옮기고, 그 줄이 화면 가운데에 오도록 스크롤한다 (줄 이동, P2-02).
    pub fn go_to_line(&mut self, doc: &mut Document, line: usize) {
        let index = doc.line_index();
        let line = line.clamp(1, index.line_count().max(1));
        let start = index.char_of_line_col(doc.text(), line, 1);
        doc.set_cursor_char_range(start..start);
        doc.break_undo_group();
        self.pending_cursor = Some(start..start);
        self.center_cursor = true;
        self.focus_requested = true;
    }

    /// 에디터가 처리하는 편집 커맨드를 실행한다. 처리했으면 `true`.
    ///
    /// 메뉴에서 실행되면 에디터가 포커스를 잃으므로, 문서에 저장된 선택 범위를 기준으로 동작하고 포커스를 되돌린다.
    pub fn handle_command(
        &mut self,
        ctx: &egui::Context,
        doc: &mut Document,
        command: Command,
    ) -> bool {
        match command {
            Command::Undo => {
                if doc.undo() {
                    self.pending_cursor = Some(doc.cursor_char_range());
                }
            }
            Command::Redo => {
                if doc.redo() {
                    self.pending_cursor = Some(doc.cursor_char_range());
                }
            }
            Command::Copy => {
                if let Some(text) = selected_text(doc) {
                    ctx.copy_text(text.to_owned());
                }
            }
            Command::Cut => {
                if let Some(text) = selected_text(doc) {
                    ctx.copy_text(text.to_owned());
                    if !doc.is_readonly() {
                        let start = doc.cursor_char_range().start;
                        doc.replace_range(selected_byte_range(doc), "");
                        doc.set_cursor_char_range(start..start);
                        self.pending_cursor = Some(start..start);
                    }
                }
            }
            Command::Paste => {
                // 클립보드 읽기는 통합 계층이 맡는다: 다음 프레임에 `Event::Paste`가 포커스된 `TextEdit`으로 들어온다.
                self.pending_cursor = Some(doc.cursor_char_range());
                ctx.send_viewport_cmd(ViewportCommand::RequestPaste);
            }
            Command::SelectAll => {
                let len = doc.text().chars().count();
                doc.set_cursor_char_range(0..len);
                self.pending_cursor = Some(0..len);
            }
            _ => return false,
        }
        self.focus_requested = true;
        true
    }

    /// 에디터를 그리고 입력을 문서에 반영한다. 남은 영역을 모두 차지한다.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        doc: &mut Document,
        options: &EditorOptions,
        palette: &Palette,
    ) -> EditorOutput {
        let ctx = ui.ctx().clone();
        if self.pending_top_line.is_some() {
            ui.set_invisible();
        }

        if self.synced_revision != Some(doc.revision()) {
            self.sync_from(doc);
        }

        let focused_before = ctx.memory(|m| m.has_focus(self.id));
        // 이번 프레임에 포커스를 받는 경우(메뉴 `붙여넣기` 직후 등)에도 위젯보다 먼저 입력을 고친다.
        if focused_before || self.focus_requested {
            ctx.input_mut(|i| filter_input_events(&mut i.events));
            if options.tab_spaces
                && !doc.is_readonly()
                && ctx.input(|i| i.events.iter().any(is_plain_tab))
            {
                let (_, column, _) = cursor_status(doc);
                ctx.input_mut(|i| expand_tab_key(&mut i.events, options.tab_width, column - 1));
            }
        }

        if let Some(range) = self.pending_cursor.take() {
            let mut state = TextEditState::load(&ctx, self.id).unwrap_or_default();
            state.cursor.set_char_range(Some(CCursorRange::two(
                CCursor::new(range.start),
                CCursor::new(range.end),
            )));
            state.store(&ctx, self.id);
        }
        if std::mem::take(&mut self.focus_requested) {
            ctx.memory_mut(|m| m.request_focus(self.id));
        }

        let mut scroll = if options.word_wrap {
            egui::ScrollArea::vertical()
        } else {
            egui::ScrollArea::both()
        }
        .id_salt(self.id.with("scroll"))
        .auto_shrink([false, false]);
        if let Some(y) = self.pending_scroll_y.take() {
            scroll = scroll.vertical_scroll_offset(y);
        }

        // 왼쪽 Gutter 칸과 오른쪽 본문(스크롤 영역)으로 나눈다.
        let char_width = ui.fonts_mut(|f| f.glyph_width(&options.font, '0'));
        let full = ui.available_rect_before_wrap();
        let gutter_rect = Rect::from_min_max(
            full.min,
            Pos2::new(
                full.min.x + gutter_width(doc.line_count(), char_width),
                full.max.y,
            ),
        );
        let text_rect = Rect::from_min_max(Pos2::new(gutter_rect.max.x, full.min.y), full.max);

        let readonly = doc.is_readonly();
        let id = self.id;
        let marks = self
            .highlight
            .as_ref()
            .filter(|h| !h.ranges.is_empty() && h.text_len == self.scratch.len())
            .map(|h| Arc::clone(&h.ranges));
        let mark_color = palette.smart_highlight;
        let syntax = self.update_syntax(doc, &ctx);
        let window = self.syntax_window(doc);
        let scratch = &mut self.scratch;
        // 기본 레이아웃과 같되, 자동 줄바꿈을 끄면 줄바꿈 폭을 무한대로 한다
        // (`desired_width(INFINITY)`만으로는 보이는 폭에서 접혀 가로 스크롤이 생기지 않는다).
        let wrap = options.word_wrap;
        let font = options.font.clone();
        let colors = SyntaxColors {
            heading: palette.md_heading,
            emphasis: palette.md_emphasis,
            strong: palette.md_strong,
            code: palette.md_code,
            link: palette.md_link,
            list: palette.md_list,
            quote: palette.md_quote,
            meta: palette.md_meta,
        };
        let mut layouter = move |ui: &egui::Ui, buf: &dyn egui::TextBuffer, wrap_width: f32| {
            let text = buf.as_str();
            let color = ui.visuals().text_color();
            let mut job = match &syntax {
                Some(syntax) => syntax_job(text, syntax, &window, &font, color, &colors),
                None => LayoutJob::simple(text.to_owned(), font.clone(), color, f32::INFINITY),
            };
            job.wrap.max_width = if wrap { wrap_width } else { f32::INFINITY };
            job.keep_trailing_whitespace = true;
            ui.fonts_mut(|f| f.layout_job(job))
        };
        let scroll_output = ui
            .scope_builder(UiBuilder::new().max_rect(text_rect), |ui| {
                scroll.show(ui, |ui| {
                    // 강조 사각형은 글자보다 먼저(뒤에) 그려지도록 자리를 잡아 두고, 레이아웃이 나온 뒤 채운다.
                    let marks_slot = ui.painter().add(egui::Shape::Noop);
                    let min_size = ui.available_size();
                    let output = if readonly {
                        // 읽기 전용 문서도 선택·복사는 되도록 `&str`로 그린다.
                        let mut text = scratch.as_str();
                        text_edit(&mut text, id, options, min_size, &mut layouter).show(ui)
                    } else {
                        text_edit(scratch, id, options, min_size, &mut layouter).show(ui)
                    };
                    if let Some(ranges) = &marks {
                        let origin = output.galley_pos - egui::vec2(output.galley.rect.left(), 0.0);
                        let shapes =
                            highlight_rects(&output.galley, origin, ui.clip_rect(), ranges)
                                .into_iter()
                                .map(|r| egui::Shape::rect_filled(r, 0.0, mark_color))
                                .collect();
                        ui.painter().set(marks_slot, egui::Shape::Vec(shapes));
                    }
                    output
                })
            })
            .inner;
        doc.set_scroll_offset_y(scroll_output.state.clamped_offset().y);
        let view = scroll_output.inner_rect;
        let galley_pos = scroll_output.inner.galley_pos;
        self.last_view = Some(LastView {
            galley: Arc::clone(&scroll_output.inner.galley),
            top_in_galley: view.min.y - galley_pos.y,
            view_height: view.height(),
            max_offset: (scroll_output.content_size.y - view.height()).max(0.0),
        });
        if let Some(line) = self.pending_top_line.take() {
            let (top, bottom) = line_span(doc, &scroll_output.inner.galley, line.floor() as usize);
            let y = top + (bottom - top) * line.fract();
            let content_y = galley_pos.y - view.min.y + scroll_output.state.clamped_offset().y + y;
            self.pending_scroll_y = Some(content_y.max(0.0));
            ctx.request_repaint();
        }

        // 줄 이동: 커서 줄이 화면 가운데에 오도록 다음 프레임 스크롤 위치를 정한다.
        if std::mem::take(&mut self.center_cursor) {
            let galley = &scroll_output.inner.galley;
            let cursor = galley.pos_from_cursor(CCursor::new(doc.cursor_char_range().start));
            let view = scroll_output.inner_rect;
            let content_y = scroll_output.inner.galley_pos.y + cursor.center().y - view.min.y
                + scroll_output.state.clamped_offset().y;
            self.pending_scroll_y = Some((content_y - view.height() / 2.0).max(0.0));
            ctx.request_repaint();
        }

        let output = scroll_output.inner;
        let cursor = output
            .cursor_range
            .map(|r| sorted_char_range(&r))
            .unwrap_or_else(|| doc.cursor_char_range());

        let mut changed = false;
        if output.response.changed() && !readonly {
            // 그래도 `\r`이 들어왔다면 문서는 `LF`로 정규화하므로, 커서도 지워지는 `\r` 수만큼 당겨서 넘긴다.
            let had_cr = self.scratch.as_bytes().contains(&b'\r');
            let cursor = if had_cr {
                lf_char_index(&self.scratch, cursor.start)..lf_char_index(&self.scratch, cursor.end)
            } else {
                cursor
            };
            changed = doc.apply_edit_with_cursor(&self.scratch, cursor);
            if had_cr {
                self.sync_from(doc);
                self.pending_cursor = Some(doc.cursor_char_range());
            }
            self.synced_revision = Some(doc.revision());
        } else if cursor != doc.cursor_char_range() {
            doc.break_undo_group();
            doc.set_cursor_char_range(cursor);
        }

        let has_focus = output.response.has_focus();
        if focused_before && !has_focus {
            doc.break_undo_group();
        }

        let highlight_exceeded = self.update_highlight(doc, &options.highlight, &ctx);

        let (current_line, _, _) = cursor_status(doc);
        paint_gutter(
            ui,
            gutter_rect,
            &output.galley,
            output.galley_pos,
            current_line,
            &options.font,
            palette,
        );
        EditorOutput {
            changed,
            has_focus,
            highlight_exceeded,
        }
    }

    /// 선택 단어 강조를 필요할 때만 다시 계산한다. 상한을 넘어 강조하지 않았으면 `true`.
    ///
    /// 결과는 다음 프레임에 그려지므로, 강조가 바뀌면 다시 그리기를 요청한다.
    fn update_highlight(
        &mut self,
        doc: &Document,
        options: &HighlightOptions,
        ctx: &egui::Context,
    ) -> bool {
        let selection = doc.cursor_char_range();
        let key = (doc.revision(), selection.clone(), *options);
        if self.highlight.as_ref().is_some_and(|h| h.key == key) {
            return false;
        }
        let had_marks = self
            .highlight
            .as_ref()
            .is_some_and(|h| !h.ranges.is_empty());
        let mut exceeded = false;
        let mut ranges = Vec::new();
        // 2글자 미만 선택은 텍스트를 훑지 않고 바로 건너뛴다.
        if selection.len() >= 2 {
            let selected = selected_byte_range(doc);
            if let Some(word) = target_word(&doc.text()[selected.clone()]) {
                let found = find_matches(doc.text(), word, options);
                exceeded = found.exceeded;
                ranges = found.ranges;
                // 선택 자체(앞뒤 공백을 뺀 단어 위치)는 강조하지 않는다.
                ranges.retain(|r| !(selected.start <= r.start && r.end <= selected.end));
                ranges = byte_ranges_to_chars(doc.text(), &ranges);
            }
        }
        if had_marks || !ranges.is_empty() {
            ctx.request_repaint();
        }
        self.highlight = Some(SmartHighlight {
            key,
            text_len: doc.text().len(),
            ranges: Arc::new(ranges),
        });
        exceeded
    }

    /// 지금 강조 중인 매치 수 (선택 자체 제외).
    pub fn highlight_count(&self) -> usize {
        self.highlight.as_ref().map_or(0, |h| h.ranges.len())
    }

    fn sync_from(&mut self, doc: &Document) {
        self.scratch.clear();
        self.scratch.push_str(doc.text());
        self.synced_revision = Some(doc.revision());
    }
}

/// 구문 색상을 칠한 레이아웃 (`window` 바이트 범위 안의 구간만). 텍스트가 계산 기준과 다르면
/// (아직 다시 계산하기 전의 편집) 구간 위치를 편집에 맞게 옮겨 쓴다.
fn syntax_job(
    text: &str,
    syntax: &SyntaxState,
    window: &Range<usize>,
    font: &FontId,
    color: egui::Color32,
    colors: &SyntaxColors,
) -> LayoutJob {
    let edit = (text != syntax.text).then(|| EditMap::new(&syntax.text, text));
    let map = |pos: usize| edit.as_ref().map_or(pos, |m| m.map(pos));
    let base = egui::TextFormat::simple(font.clone(), color);
    let mut job = LayoutJob::default();
    let mut pos = 0;
    let first = syntax
        .spans
        .partition_point(|(range, _)| map(range.end) <= window.start);
    for (range, kind) in &syntax.spans[first..] {
        let (start, end) = (map(range.start), map(range.end));
        if start >= window.end {
            break;
        }
        if end > text.len() || start < pos || start >= end {
            continue;
        }
        if start > pos {
            job.append(&text[pos..start], 0.0, base.clone());
        }
        let mut format = base.clone();
        match kind {
            SyntaxKind::Heading => format.color = colors.heading,
            SyntaxKind::Emphasis => {
                format.color = colors.emphasis;
                format.italics = true;
            }
            SyntaxKind::Strong => format.color = colors.strong,
            SyntaxKind::Strike => {
                format.color = colors.meta;
                format.strikethrough = egui::Stroke::new(1.0, colors.meta);
            }
            SyntaxKind::Code => format.color = colors.code,
            SyntaxKind::Link => format.color = colors.link,
            SyntaxKind::ListMarker => format.color = colors.list,
            SyntaxKind::QuoteMarker => format.color = colors.quote,
            SyntaxKind::Meta => format.color = colors.meta,
        }
        job.append(&text[start..end], 0.0, format);
        pos = end;
    }
    if pos < text.len() || job.sections.is_empty() {
        job.append(&text[pos..], 0.0, base);
    }
    job
}

/// 문자 인덱스 두 개를 한 번 훑어 바이트 위치로 바꾼다 (`first <= last`, 넘으면 텍스트 끝).
fn char_range_to_bytes(text: &str, first: usize, last: usize) -> (usize, usize) {
    let mut start = text.len();
    for (n, (byte, _)) in text.char_indices().enumerate() {
        if n == first {
            start = byte;
        }
        if n == last {
            return (start, byte);
        }
    }
    (start.min(text.len()), text.len())
}

/// 소스 줄(0부터)의 galley 안 위·아래 y. 줄이 없으면 galley 끝.
fn line_span(doc: &Document, galley: &Galley, line: usize) -> (f32, f32) {
    let index = doc.line_index();
    let text = doc.text();
    let y_of = |line: usize| {
        index
            .line_start_byte(line)
            .map_or(galley.rect.bottom(), |b| {
                galley
                    .pos_from_cursor(CCursor::new(byte_to_char_offset(text, b)))
                    .min
                    .y
            })
    };
    let top = y_of(line);
    (top, y_of(line + 1).max(top))
}

fn text_edit<'t>(
    text: &'t mut dyn egui::TextBuffer,
    id: Id,
    options: &EditorOptions,
    min_size: egui::Vec2,
    layouter: &'t mut dyn FnMut(&egui::Ui, &dyn egui::TextBuffer, f32) -> Arc<Galley>,
) -> egui::TextEdit<'t> {
    egui::TextEdit::multiline(text)
        .id(id)
        .font(options.font.clone())
        .layouter(layouter)
        .frame(egui::Frame::NONE)
        .margin(egui::Margin::symmetric(6, 4))
        .lock_focus(true)
        .desired_width(if options.word_wrap {
            min_size.x
        } else {
            f32::INFINITY
        })
        .min_size(min_size)
}

/// 오름차순 바이트 범위들을 문자 범위로 바꾼다 (텍스트를 한 번만 훑는다).
fn byte_ranges_to_chars(text: &str, ranges: &[Range<usize>]) -> Vec<Range<usize>> {
    let (mut byte, mut chars) = (0, 0);
    let mut to_char = |to: usize| {
        chars += text[byte..to].chars().count();
        byte = to;
        chars
    };
    ranges
        .iter()
        .map(|r| {
            let start = to_char(r.start);
            start..to_char(r.end)
        })
        .collect()
}

/// 레이아웃 `galley`(화면 위치 `origin`)에서 `clip` 안에 보이는 매치(문자 범위, 오름차순)의 배경 사각형.
///
/// 보이는 첫·끝 글자 위치를 구해 그 사이 매치만 계산하므로 문서 전체 매치 수와 상관없이 가볍다.
/// 자동 줄바꿈으로 단어가 두 행에 걸치면 두 조각으로 나눠 그린다.
fn highlight_rects(
    galley: &Galley,
    origin: Pos2,
    clip: Rect,
    ranges: &[Range<usize>],
) -> Vec<Rect> {
    if ranges.is_empty() {
        return Vec::new();
    }
    let index = |pos: Pos2| -> usize { galley.cursor_from_pos(pos - origin).index.into() };
    let (first, last) = (index(clip.min), index(clip.max));
    let at = |i: usize| {
        galley
            .pos_from_cursor(CCursor::new(i))
            .translate(origin.to_vec2())
    };
    let from = ranges.partition_point(|r| r.end <= first);
    let mut rects = Vec::new();
    for range in ranges[from..].iter().take_while(|r| r.start <= last) {
        let (a, b) = (at(range.start), at(range.end));
        if (a.min.y - b.min.y).abs() < 0.5 {
            rects.push(Rect::from_min_max(a.min, Pos2::new(b.min.x, a.max.y)));
        } else {
            rects.push(Rect::from_min_max(
                a.min,
                Pos2::new(origin.x + galley.rect.right(), a.max.y),
            ));
            rects.push(Rect::from_min_max(
                Pos2::new(origin.x + galley.rect.left(), b.min.y),
                b.max,
            ));
        }
    }
    rects.retain(|r| r.width() > 0.0);
    rects
}

/// 줄 번호 Gutter 폭. 최소 3자리이고 줄 수 자릿수(1,000·10,000줄 …)에 맞춰 늘어난다 (뷰어와 같은 폭).
///
/// 구성: 왼쪽 여백 1칸 + 번호 + 본문과의 빈 간격 [`gutter_gap`] (구분선 없이 여백으로 구분 — UX-007).
pub fn gutter_width(line_count: usize, char_width: f32) -> f32 {
    let digits = line_count.max(1).to_string().len().max(3);
    (digits + 1) as f32 * char_width + gutter_gap(char_width)
}

/// 줄 번호 오른쪽 끝과 Gutter 오른쪽 경계 사이의 빈 간격 (본문 글자는 여기에 에디터 안쪽 여백이 더해진 뒤 시작).
pub fn gutter_gap(char_width: f32) -> f32 {
    char_width * 2.5
}

/// 레이아웃 행 중 논리 줄의 첫 행만 `(1-기반 줄 번호, galley 기준 행 영역)`으로 돌려준다
/// (자동 줄바꿈으로 접힌 행은 건너뜀).
fn logical_line_rows(galley: &Galley) -> impl Iterator<Item = (usize, Rect)> + '_ {
    let mut line = 0;
    let mut starts_line = true;
    galley.rows.iter().filter_map(move |row| {
        let numbered = starts_line.then(|| {
            line += 1;
            (line, row.rect())
        });
        starts_line = row.ends_with_newline;
        numbered
    })
}

/// 본문 레이아웃의 행 위치에 맞춰 Gutter에 줄 번호를 그린다 (보이는 행만).
fn paint_gutter(
    ui: &egui::Ui,
    rect: Rect,
    galley: &Galley,
    galley_pos: Pos2,
    current_line: usize,
    font: &FontId,
    palette: &Palette,
) {
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, palette.gutter_bg);
    let number_right = rect.max.x - gutter_gap(painter.fonts_mut(|f| f.glyph_width(font, '0')));
    for (line, row_rect) in logical_line_rows(galley) {
        let row_rect = row_rect.translate(galley_pos.to_vec2());
        if row_rect.max.y < rect.min.y {
            continue;
        }
        if row_rect.min.y > rect.max.y {
            break;
        }
        painter.text(
            Pos2::new(number_right, row_rect.center().y),
            egui::Align2::RIGHT_CENTER,
            line.to_string(),
            font.clone(),
            if line == current_line {
                palette.gutter_current
            } else {
                palette.gutter_text
            },
        );
    }
}

/// `TextEdit`에 들어가기 전에 입력 이벤트를 고친다.
///
/// - `TextEdit` 내장 Undo/Redo를 일으키는 `Ctrl/Cmd+Z`·`Ctrl/Cmd+Y` 조합을 지운다 (문서 이력만 사용).
///   앱 단축키로 쓰는 조합은 이미 `KeyDispatcher`가 소비했으므로, 여기 남은 것은 바인딩되지 않은 변형이다.
/// - 붙여넣기 텍스트의 `CRLF`/`CR`을 `LF`로 바꾼다.
pub fn filter_input_events(events: &mut Vec<Event>) {
    events.retain(|e| {
        !matches!(e, Event::Key { key: Key::Z | Key::Y, modifiers, .. }
            if modifiers.command || modifiers.ctrl || modifiers.mac_cmd)
    });
    for event in events.iter_mut() {
        if let Event::Paste(text) = event
            && text.contains('\r')
        {
            *text = normalize_to_lf(text);
        }
    }
}

/// 수식키 없는 `Tab` 키 누름 (들여쓰기 입력).
fn is_plain_tab(event: &Event) -> bool {
    matches!(event, Event::Key { key: Key::Tab, pressed: true, modifiers, .. } if modifiers.is_none())
}

/// `Tab` 키 누름을 다음 탭 위치까지의 공백 입력으로 바꾼다 (`column`은 커서의 0-기반 문자 열).
///
/// 한 프레임에 여러 번 눌렸으면 이어서 계산한다.
pub fn expand_tab_key(events: &mut [Event], tab_width: usize, mut column: usize) {
    let width = tab_width.max(1);
    for event in events.iter_mut() {
        if is_plain_tab(event) {
            let spaces = width - column % width;
            column += spaces;
            *event = Event::Text(" ".repeat(spaces));
        } else if let Event::Text(text) = event {
            column += text.chars().count();
        }
    }
}

/// 상태 표시줄용 `(Ln, Col, 선택 글자 수)` — 모두 문자 단위, `Ln`/`Col`은 1-기반.
pub fn cursor_status(doc: &Document) -> (usize, usize, usize) {
    let range = doc.cursor_char_range();
    let (line, col) = doc.line_index().line_col_of_char(doc.text(), range.end);
    (line, col, range.end.saturating_sub(range.start))
}

/// `\r`이 섞인 `text`의 문자 인덱스를 `LF` 정규화(`\r\n` → `\n`, 단독 `\r` → `\n`) 뒤의 문자 인덱스로 바꾼다.
fn lf_char_index(text: &str, char_index: usize) -> usize {
    let mut removed = 0;
    let mut chars = text.chars().peekable();
    for _ in 0..char_index {
        let Some(c) = chars.next() else {
            break;
        };
        // `\r\n`의 `\r`만 사라진다. 단독 `\r`은 `\n`으로 바뀌어 글자 수가 같다.
        if c == '\r' && chars.peek() == Some(&'\n') {
            removed += 1;
        }
    }
    char_index - removed
}

fn sorted_char_range(range: &CCursorRange) -> Range<usize> {
    let [a, b] = range.sorted_cursors();
    a.index.into()..b.index.into()
}

fn selected_byte_range(doc: &Document) -> Range<usize> {
    let range = doc.cursor_char_range();
    let text = doc.text();
    let start = char_to_byte_offset(text, range.start);
    let end = start + char_to_byte_offset(&text[start..], range.end.saturating_sub(range.start));
    start..end
}

fn selected_text(doc: &Document) -> Option<&str> {
    let range = selected_byte_range(doc);
    (!range.is_empty()).then(|| &doc.text()[range])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::encoding::TextEncoding;
    use crate::core::line_ending::LineEnding;
    use eframe::egui::{Modifiers, RawInput};

    fn new_doc() -> Document {
        Document::new_untitled_with_defaults(1, TextEncoding::Utf8, LineEnding::Lf)
    }

    fn options() -> EditorOptions {
        EditorOptions {
            word_wrap: true,
            font: FontId::monospace(14.0),
            highlight: HighlightOptions::default(),
            tab_width: 4,
            tab_spaces: false,
        }
    }

    /// 헤드리스 `egui` 프레임 하나를 돌린다.
    fn frame(
        ctx: &egui::Context,
        editor: &mut EditorState,
        doc: &mut Document,
        events: Vec<Event>,
    ) -> EditorOutput {
        frame_with(ctx, editor, doc, events, &options())
    }

    fn frame_with(
        ctx: &egui::Context,
        editor: &mut EditorState,
        doc: &mut Document,
        events: Vec<Event>,
        options: &EditorOptions,
    ) -> EditorOutput {
        let raw = RawInput {
            events,
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800.0, 600.0),
            )),
            ..Default::default()
        };
        let mut out = EditorOutput::default();
        let mut full = ctx.run_ui(raw, |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                out = editor.show(ui, doc, options, &Palette::light());
            });
        });
        // 렌더러가 없으므로 폰트 텍스처 갱신분을 직접 버린다 (버리지 않으면 디버그 빌드에서 panic).
        full.textures_delta.clear();
        out
    }

    fn key(key: Key, modifiers: Modifiers) -> Event {
        Event::Key {
            key,
            physical_key: Some(key),
            pressed: true,
            repeat: false,
            modifiers,
        }
    }

    /// 헤드리스 프레임 (시각 지정): 에디터 본문 galley의 `needle` 첫 글자 색을 돌려준다.
    fn color_at(
        ctx: &egui::Context,
        editor: &mut EditorState,
        doc: &mut Document,
        events: Vec<Event>,
        time: f64,
        needle: &str,
    ) -> Option<egui::Color32> {
        let raw = RawInput {
            events,
            time: Some(time),
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800.0, 600.0),
            )),
            ..Default::default()
        };
        let mut full = ctx.run_ui(raw, |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                editor.show(ui, doc, &options(), &Palette::light());
            });
        });
        full.textures_delta.clear();
        full.shapes.iter().find_map(|c| match &c.shape {
            egui::Shape::Text(t) => {
                let text = t.galley.text();
                let at = text.find(needle)?;
                t.galley
                    .job
                    .sections
                    .iter()
                    .find(|s| s.byte_range.start.0 <= at && at < s.byte_range.end.0)
                    .map(|s| s.format.color)
            }
            _ => None,
        })
    }

    fn markdown_doc(text: &str) -> Document {
        let mut doc = new_doc();
        doc.set_text(text);
        doc
    }

    #[test]
    fn markdown_source_gets_syntax_colors() {
        let ctx = egui::Context::default();
        let mut editor = EditorState::new(1);
        editor.set_markdown_syntax(true);
        let mut doc =
            markdown_doc("# 제목\n\n본문 **굵게** `코드` [링크](a.md)\n\n- 항목\n> 인용\n");
        let p = Palette::light();
        let mut at = |needle| color_at(&ctx, &mut editor, &mut doc, vec![], 1.0, needle);
        assert_eq!(at("# 제목"), Some(p.md_heading));
        assert_eq!(at("**굵게**"), Some(p.md_strong));
        assert_eq!(at("`코드`"), Some(p.md_code));
        assert_eq!(at("[링크]"), Some(p.md_link));
        assert_eq!(at("- 항목"), Some(p.md_list));
        assert_eq!(at("> 인용"), Some(p.md_quote));
        assert_ne!(at("본문"), Some(p.md_heading));

        // 끄면 모두 본문 색
        editor.set_markdown_syntax(false);
        let plain = color_at(&ctx, &mut editor, &mut doc, vec![], 1.1, "# 제목");
        let body = color_at(&ctx, &mut editor, &mut doc, vec![], 1.2, "본문");
        assert_eq!(plain, body);
    }

    #[test]
    fn syntax_colors_follow_edits_until_recomputed() {
        let ctx = egui::Context::default();
        let mut editor = EditorState::new(1);
        editor.set_markdown_syntax(true);
        let mut doc = markdown_doc("앞 문단\n\n**굵게** 뒤\n");
        let p = Palette::light();
        assert_eq!(
            color_at(&ctx, &mut editor, &mut doc, vec![], 1.0, "**굵게**"),
            Some(p.md_strong)
        );
        // 앞쪽에 글자를 넣은 직후(다시 계산하기 전)에도 굵게 색은 같은 글자에 붙어 있다
        doc.set_text("앞쪽에 넣은 글자 문단\n\n**굵게** 뒤\n");
        assert_eq!(
            color_at(&ctx, &mut editor, &mut doc, vec![], 1.05, "**굵게**"),
            Some(p.md_strong)
        );
        assert_ne!(
            color_at(&ctx, &mut editor, &mut doc, vec![], 1.06, "앞쪽"),
            Some(p.md_strong)
        );
        // 디바운스가 지나면 새로 쓴 제목도 칠해진다
        doc.set_text("# 새 제목\n\n**굵게** 뒤\n");
        color_at(&ctx, &mut editor, &mut doc, vec![], 1.1, "새 제목");
        assert_eq!(
            color_at(
                &ctx,
                &mut editor,
                &mut doc,
                vec![],
                1.1 + SYNTAX_DEBOUNCE + 0.01,
                "# 새"
            ),
            Some(p.md_heading)
        );
    }

    /// 1MB `.md` 구문 색상 켠 상태의 입력 프레임 시간 —
    /// `cargo test --release --lib -- --ignored --nocapture markdown_syntax_frame_time`
    #[test]
    #[ignore = "성능 측정 (릴리스 빌드로 수동 실행)"]
    fn markdown_syntax_frame_time_1mb() {
        use std::time::Instant;

        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/markdown");
        let unit = format!(
            "{}\n{}\n",
            std::fs::read_to_string(format!("{dir}/gfm_all.md")).unwrap(),
            std::fs::read_to_string(format!("{dir}/korean.md")).unwrap()
        );
        let mut text = String::new();
        while text.len() + unit.len() <= SYNTAX_MAX_BYTES {
            text.push_str(&unit);
        }
        let ctx = egui::Context::default();
        let measure = |syntax: bool| {
            let mut doc = markdown_doc(&text);
            let mut editor = EditorState::new(31);
            editor.set_markdown_syntax(syntax);
            editor.request_focus();
            let options = options();
            let clock = Instant::now();
            let timed = |editor: &mut EditorState, doc: &mut Document, events: Vec<Event>| {
                let started = Instant::now();
                let raw = RawInput {
                    events,
                    time: Some(clock.elapsed().as_secs_f64()),
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1280.0, 900.0),
                    )),
                    ..Default::default()
                };
                let mut full = ctx.run_ui(raw, |ui| {
                    egui::CentralPanel::default().show(ui, |ui| {
                        editor.show(ui, doc, &options, &Palette::light());
                    });
                });
                full.textures_delta.clear();
                let _ = ctx.tessellate(std::mem::take(&mut full.shapes), full.pixels_per_point);
                started.elapsed().as_secs_f64() * 1000.0
            };
            for _ in 0..3 {
                timed(&mut editor, &mut doc, vec![]);
            }
            // 연속 입력 (디바운스 안) 30프레임 + 멈춘 뒤 다시 계산하는 프레임
            let typing: Vec<f64> = (0..30)
                .map(|i| {
                    let t = if i % 2 == 0 { "가" } else { "a" };
                    timed(&mut editor, &mut doc, vec![Event::Text(t.into())])
                })
                .collect();
            std::thread::sleep(std::time::Duration::from_secs_f64(SYNTAX_DEBOUNCE + 0.02));
            let recompute = timed(&mut editor, &mut doc, vec![]);
            let avg = typing.iter().sum::<f64>() / typing.len() as f64;
            let max = typing.iter().copied().fold(0.0, f64::max);
            (avg, max, recompute)
        };
        let (plain_avg, plain_max, _) = measure(false);
        let (avg, max, recompute) = measure(true);
        println!(
            "1MB .md({} B) 입력 프레임: 색상 끔 평균 {plain_avg:.1}ms·최대 {plain_max:.1}ms / 색상 켬 평균 {avg:.1}ms·최대 {max:.1}ms, 멈춘 뒤 다시 계산 프레임 {recompute:.1}ms",
            text.len()
        );
    }

    /// 포커스를 준 에디터와 문서를 만든다.
    fn focused_editor() -> (egui::Context, EditorState, Document) {
        let ctx = egui::Context::default();
        let mut editor = EditorState::new(1);
        let mut doc = new_doc();
        editor.request_focus();
        frame(&ctx, &mut editor, &mut doc, vec![]);
        frame(&ctx, &mut editor, &mut doc, vec![]);
        (ctx, editor, doc)
    }

    #[test]
    fn typing_updates_document_and_cursor() {
        let (ctx, mut editor, mut doc) = focused_editor();
        let out = frame(
            &ctx,
            &mut editor,
            &mut doc,
            vec![Event::Text("한글".into()), Event::Text(" 입력".into())],
        );
        assert!(out.changed);
        assert!(out.has_focus);
        assert_eq!(doc.text(), "한글 입력");
        assert!(doc.is_modified());
        assert_eq!(doc.cursor_char_range(), 5..5);
        assert_eq!(cursor_status(&doc), (1, 6, 0));
    }

    #[test]
    fn tab_key_inserts_tab_and_keeps_focus() {
        let (ctx, mut editor, mut doc) = focused_editor();
        let out = frame(
            &ctx,
            &mut editor,
            &mut doc,
            vec![
                Event::Text("a".into()),
                key(Key::Tab, Modifiers::NONE),
                Event::Text("b".into()),
            ],
        );
        assert_eq!(doc.text(), "a\tb");
        assert!(out.has_focus);
        let out = frame(&ctx, &mut editor, &mut doc, vec![]);
        assert!(out.has_focus);
    }

    #[test]
    fn pasted_crlf_is_normalized_and_cursor_stays_consistent() {
        let (ctx, mut editor, mut doc) = focused_editor();
        frame(
            &ctx,
            &mut editor,
            &mut doc,
            vec![Event::Paste("첫 줄\r\n둘째 줄\r셋째".into())],
        );
        assert_eq!(doc.text(), "첫 줄\n둘째 줄\n셋째");
        assert_eq!(doc.cursor_char_range(), 11..11);
        frame(&ctx, &mut editor, &mut doc, vec![Event::Text("!".into())]);
        assert_eq!(doc.text(), "첫 줄\n둘째 줄\n셋째!");
    }

    #[test]
    fn undo_redo_use_document_history_and_restore_cursor() {
        let (ctx, mut editor, mut doc) = focused_editor();
        frame(
            &ctx,
            &mut editor,
            &mut doc,
            vec![Event::Text("안녕".into())],
        );
        frame(
            &ctx,
            &mut editor,
            &mut doc,
            vec![Event::Text(" 하세요".into())],
        );
        assert_eq!(doc.text(), "안녕 하세요");

        assert!(editor.handle_command(&ctx, &mut doc, Command::Undo));
        frame(&ctx, &mut editor, &mut doc, vec![]);
        assert_eq!(doc.text(), "안녕");
        assert_eq!(doc.cursor_char_range(), 2..2);

        // 위젯 사본도 되돌아갔으므로 이어서 입력하면 Undo 시점 위치에 들어간다.
        frame(&ctx, &mut editor, &mut doc, vec![Event::Text("!".into())]);
        assert_eq!(doc.text(), "안녕!");

        assert!(editor.handle_command(&ctx, &mut doc, Command::Undo));
        assert!(editor.handle_command(&ctx, &mut doc, Command::Undo));
        frame(&ctx, &mut editor, &mut doc, vec![]);
        assert_eq!(doc.text(), "");
        assert!(!doc.is_modified());

        assert!(editor.handle_command(&ctx, &mut doc, Command::Redo));
        frame(&ctx, &mut editor, &mut doc, vec![]);
        assert_eq!(doc.text(), "안녕");
    }

    #[test]
    fn unbound_undo_keys_do_not_reach_text_edit() {
        let (ctx, mut editor, mut doc) = focused_editor();
        frame(&ctx, &mut editor, &mut doc, vec![Event::Text("abc".into())]);
        // Windows의 Ctrl+Shift+Z는 앱 단축키가 아니지만 TextEdit 내장 Redo/Undo를 일으키면 안 된다.
        frame(
            &ctx,
            &mut editor,
            &mut doc,
            vec![
                key(Key::Z, Modifiers::COMMAND | Modifiers::SHIFT),
                key(Key::Z, Modifiers::COMMAND),
            ],
        );
        assert_eq!(doc.text(), "abc");
        assert_eq!(doc.undo_len(), 1);
    }

    #[test]
    fn select_all_cut_and_copy_commands() {
        let (ctx, mut editor, mut doc) = focused_editor();
        frame(
            &ctx,
            &mut editor,
            &mut doc,
            vec![Event::Text("가나다".into())],
        );

        assert!(editor.handle_command(&ctx, &mut doc, Command::SelectAll));
        frame(&ctx, &mut editor, &mut doc, vec![]);
        assert_eq!(doc.cursor_char_range(), 0..3);
        assert_eq!(cursor_status(&doc).2, 3);

        assert!(editor.handle_command(&ctx, &mut doc, Command::Cut));
        frame(&ctx, &mut editor, &mut doc, vec![]);
        assert_eq!(doc.text(), "");
        assert!(editor.handle_command(&ctx, &mut doc, Command::Undo));
        frame(&ctx, &mut editor, &mut doc, vec![]);
        assert_eq!(doc.text(), "가나다");

        // 모두 선택 후 삭제 (키보드)
        assert!(editor.handle_command(&ctx, &mut doc, Command::SelectAll));
        frame(&ctx, &mut editor, &mut doc, vec![]);
        frame(
            &ctx,
            &mut editor,
            &mut doc,
            vec![key(Key::Delete, Modifiers::NONE)],
        );
        assert_eq!(doc.text(), "");

        assert!(!editor.handle_command(&ctx, &mut doc, Command::Save));
    }

    #[test]
    fn moving_cursor_breaks_typing_group() {
        let (ctx, mut editor, mut doc) = focused_editor();
        frame(&ctx, &mut editor, &mut doc, vec![Event::Text("abc".into())]);
        frame(
            &ctx,
            &mut editor,
            &mut doc,
            vec![key(Key::ArrowLeft, Modifiers::NONE)],
        );
        frame(&ctx, &mut editor, &mut doc, vec![Event::Text("X".into())]);
        assert_eq!(doc.text(), "abXc");
        assert_eq!(doc.undo_len(), 2);
    }

    #[test]
    fn ime_composition_becomes_one_undo_step() {
        use eframe::egui::ImeEvent;

        let (ctx, mut editor, mut doc) = focused_editor();
        for preedit in ["ㅎ", "하", "한", "한ㄱ", "한그", "한글"] {
            frame(
                &ctx,
                &mut editor,
                &mut doc,
                vec![Event::Ime(ImeEvent::Preedit {
                    text: preedit.into(),
                    active_range_chars: None,
                })],
            );
        }
        frame(
            &ctx,
            &mut editor,
            &mut doc,
            vec![Event::Ime(ImeEvent::Commit("한글".into()))],
        );
        assert_eq!(doc.text(), "한글");
        assert_eq!(doc.undo_len(), 1);
        assert!(editor.handle_command(&ctx, &mut doc, Command::Undo));
        frame(&ctx, &mut editor, &mut doc, vec![]);
        assert_eq!(doc.text(), "");
    }

    #[test]
    fn readonly_document_is_not_edited() {
        let ctx = egui::Context::default();
        let mut editor = EditorState::new(2);
        let mut doc = Document::from_bytes(None, "읽기 전용".as_bytes(), true, None).unwrap();
        editor.request_focus();
        frame(&ctx, &mut editor, &mut doc, vec![]);
        let out = frame(&ctx, &mut editor, &mut doc, vec![Event::Text("x".into())]);
        assert!(!out.changed);
        assert_eq!(doc.text(), "읽기 전용");
    }

    /// 리뷰 1: 메뉴 `붙여넣기`처럼 포커스를 되찾는 프레임에 들어온 CRLF도 정규화되고 커서가 맞아야 한다.
    #[test]
    fn menu_paste_while_unfocused_normalizes_crlf_and_cursor() {
        let ctx = egui::Context::default();
        let mut editor = EditorState::new(3);
        let mut doc = new_doc();
        // 포커스 없이 한 프레임 (메뉴를 누른 상태)
        frame(&ctx, &mut editor, &mut doc, vec![]);
        assert!(editor.handle_command(&ctx, &mut doc, Command::Paste));
        // 다음 프레임: 포커스 요청 + 통합 계층이 넣은 Paste 이벤트
        frame(
            &ctx,
            &mut editor,
            &mut doc,
            vec![Event::Paste("a\r\nb\r\nc".into())],
        );
        assert_eq!(doc.text(), "a\nb\nc");
        assert_eq!(doc.cursor_char_range(), 5..5);
        frame(&ctx, &mut editor, &mut doc, vec![Event::Text("!".into())]);
        assert_eq!(doc.text(), "a\nb\nc!");
    }

    /// 헤드리스 프레임 하나를 그리고 Gutter 줄 번호 `(번호, 오른쪽 끝 x, y)`와 본문 글자 도형의 화면 영역을 돌려준다.
    fn gutter_frame(
        ctx: &egui::Context,
        editor: &mut EditorState,
        doc: &mut Document,
        word_wrap: bool,
    ) -> (Vec<(usize, f32, f32)>, egui::Rect) {
        let raw = RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800.0, 600.0),
            )),
            ..Default::default()
        };
        let options = EditorOptions {
            word_wrap,
            font: FontId::monospace(14.0),
            highlight: HighlightOptions::default(),
            tab_width: 4,
            tab_spaces: false,
        };
        let mut full = ctx.run_ui(raw, |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                editor.show(ui, doc, &options, &Palette::light());
            });
        });
        full.textures_delta.clear();
        let mut numbers = Vec::new();
        let mut body = egui::Rect::NOTHING;
        for clipped in &full.shapes {
            if let egui::Shape::Text(t) = &clipped.shape {
                match t.galley.text().parse::<usize>() {
                    Ok(n) => numbers.push((n, t.pos.x + t.galley.rect.max.x, t.pos.y)),
                    Err(_) => body = t.galley.rect.translate(t.pos.to_vec2()),
                }
            }
        }
        numbers.sort_by_key(|(n, _, _)| *n);
        (numbers, body)
    }

    #[test]
    fn gutter_width_grows_with_line_count_digits() {
        let w = |lines| gutter_width(lines, 8.0);
        assert_eq!(w(0), w(999));
        assert!(w(999) < w(1_000));
        assert!(w(9_999) < w(10_000));
        assert!(w(99_999) < w(100_000));
    }

    #[test]
    fn wrapped_rows_get_no_line_number() {
        let ctx = egui::Context::default();
        let mut editor = EditorState::new(10);
        let long = "가나다라 ".repeat(80);
        let mut doc =
            Document::from_bytes(None, format!("짧은 줄\n{long}\n끝").as_bytes(), false, None)
                .unwrap();
        gutter_frame(&ctx, &mut editor, &mut doc, true);
        let (numbers, body) = gutter_frame(&ctx, &mut editor, &mut doc, true);
        assert!(body.width() < 800.0, "창 폭 안에서 접힘");
        let lines: Vec<usize> = numbers.iter().map(|(n, _, _)| *n).collect();
        assert_eq!(lines, [1, 2, 3], "논리 줄마다 번호 하나");
        // 2번 줄이 여러 행으로 접혔으므로 3번 번호는 2번보다 여러 행 아래에 있다.
        let row_gap = numbers[1].2 - numbers[0].2;
        assert!(numbers[2].2 - numbers[1].2 > row_gap * 2.0);
    }

    /// 자동 줄바꿈을 끄면 긴 줄이 창보다 넓게 한 행으로 놓여(가로 스크롤 대상) 접히지 않고,
    /// 줄 번호는 스크롤 영역 밖의 Gutter(본문 왼쪽)에 그려진다.
    #[test]
    fn no_wrap_keeps_long_line_on_one_row_and_numbers_in_gutter() {
        let ctx = egui::Context::default();
        let mut editor = EditorState::new(11);
        let long = "abcdefghij".repeat(40);
        let mut doc = Document::from_bytes(
            None,
            format!(
                "첫 줄
{long}
끝"
            )
            .as_bytes(),
            false,
            None,
        )
        .unwrap();
        gutter_frame(&ctx, &mut editor, &mut doc, false);
        let (numbers, body) = gutter_frame(&ctx, &mut editor, &mut doc, false);
        assert!(
            body.width() > 800.0 * 2.0,
            "긴 줄이 접히지 않음 (폭 {})",
            body.width()
        );
        let lines: Vec<usize> = numbers.iter().map(|(n, _, _)| *n).collect();
        assert_eq!(lines, [1, 2, 3]);
        // 접히지 않았으므로 줄 간격이 일정하다.
        assert_eq!(numbers[1].2 - numbers[0].2, numbers[2].2 - numbers[1].2);
        // 번호와 본문 사이에 2칸 이상 빈 간격 (UX-007)
        let char_width = ctx.fonts_mut(|f| f.glyph_width(&FontId::monospace(14.0), '0'));
        assert!(
            numbers
                .iter()
                .all(|(_, right, _)| body.min.x - right >= char_width * 2.0)
        );
    }

    /// 문서와 선택(문자 범위)을 정해 에디터를 두 프레임 그린다 (선택 반영 → 강조 계산).
    fn select(
        ctx: &egui::Context,
        editor: &mut EditorState,
        doc: &mut Document,
        range: Range<usize>,
        options: &EditorOptions,
    ) -> EditorOutput {
        doc.set_cursor_char_range(range);
        editor.restore_view(doc);
        frame_with(ctx, editor, doc, vec![], options);
        frame_with(ctx, editor, doc, vec![], options)
    }

    fn highlight_options(whole_word: bool, max_matches: usize) -> EditorOptions {
        EditorOptions {
            highlight: HighlightOptions {
                whole_word,
                max_matches,
                ..Default::default()
            },
            ..options()
        }
    }

    #[test]
    fn selecting_word_highlights_other_matches_until_deselected() {
        let ctx = egui::Context::default();
        let mut editor = EditorState::new(20);
        let mut doc = Document::from_bytes(
            None,
            "검색 검색어 검색 결과
검색"
                .as_bytes(),
            false,
            None,
        )
        .unwrap();
        let whole = highlight_options(true, 10_000);
        // 첫 `검색` 선택 → 단어 단위: 나머지 `검색` 2개 강조 (선택 자체·`검색어` 제외)
        select(&ctx, &mut editor, &mut doc, 0..2, &whole);
        assert_eq!(editor.highlight_count(), 2);
        // 부분 일치 옵션 → `검색어`도 포함
        select(
            &ctx,
            &mut editor,
            &mut doc,
            0..2,
            &highlight_options(false, 10_000),
        );
        assert_eq!(editor.highlight_count(), 3);
        // 선택 해제 → 강조 없음
        select(&ctx, &mut editor, &mut doc, 1..1, &whole);
        assert_eq!(editor.highlight_count(), 0);
        // 1글자·공백 포함 선택 → 강조 없음
        select(&ctx, &mut editor, &mut doc, 0..1, &whole);
        assert_eq!(editor.highlight_count(), 0);
        select(&ctx, &mut editor, &mut doc, 0..6, &whole);
        assert_eq!(editor.highlight_count(), 0);
    }

    #[test]
    fn highlight_is_cached_until_selection_document_or_options_change() {
        let ctx = egui::Context::default();
        let mut editor = EditorState::new(21);
        let mut doc = Document::from_bytes(None, b"abc abc abc", false, None).unwrap();
        let whole = highlight_options(true, 10_000);
        select(&ctx, &mut editor, &mut doc, 0..3, &whole);
        let first = Arc::clone(&editor.highlight.as_ref().unwrap().ranges);
        frame_with(&ctx, &mut editor, &mut doc, vec![], &whole);
        frame_with(&ctx, &mut editor, &mut doc, vec![], &whole);
        assert!(
            Arc::ptr_eq(&first, &editor.highlight.as_ref().unwrap().ranges),
            "그대로면 다시 계산하지 않음"
        );
        select(&ctx, &mut editor, &mut doc, 4..7, &whole);
        assert!(!Arc::ptr_eq(
            &first,
            &editor.highlight.as_ref().unwrap().ranges
        ));
        assert_eq!(editor.highlight_count(), 2);
    }

    #[test]
    fn too_many_matches_reports_exceeded_once() {
        let ctx = egui::Context::default();
        let mut editor = EditorState::new(22);
        let mut doc = Document::from_bytes(None, "ab ".repeat(20).as_bytes(), false, None).unwrap();
        let capped = highlight_options(true, 10);
        doc.set_cursor_char_range(0..2);
        editor.restore_view(&doc);
        let out = frame_with(&ctx, &mut editor, &mut doc, vec![], &capped);
        let again = frame_with(&ctx, &mut editor, &mut doc, vec![], &capped);
        assert!(out.highlight_exceeded, "상한 초과 → 안내");
        assert!(!again.highlight_exceeded, "같은 선택이면 다시 알리지 않음");
        assert_eq!(editor.highlight_count(), 0);
    }

    /// P2-04 🤖: 1MB 문서에서 선택 단어 강조를 켠 상태의 프레임 시간 (레이아웃 + 테셀레이션 CPU, P0-06과 같은 기준).
    ///
    /// 실행: `cargo test --release --lib -- --ignored --nocapture highlight_frame_time_1mb`
    /// (샘플 `spikes/samples/text-1mb.txt`는 `cargo xtask gen-samples`)
    #[test]
    #[ignore = "성능 측정 (릴리스 빌드로 수동 실행)"]
    fn highlight_frame_time_1mb() {
        use std::time::Instant;

        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/spikes/samples/text-1mb.txt");
        let bytes = std::fs::read(path).expect("샘플 없음: cargo xtask gen-samples");
        let mut doc = Document::from_bytes(None, &bytes, false, None).unwrap();
        let ctx = egui::Context::default();
        let mut editor = EditorState::new(30);
        let options = options();

        // 프레임 하나: 레이아웃(run_ui) + 테셀레이션
        let timed = |editor: &mut EditorState, doc: &mut Document, events: Vec<Event>| {
            let started = Instant::now();
            let raw = RawInput {
                events,
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1280.0, 900.0),
                )),
                ..Default::default()
            };
            let mut full = ctx.run_ui(raw, |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    editor.show(ui, doc, &options, &Palette::light());
                });
            });
            full.textures_delta.clear();
            let _ = ctx.tessellate(std::mem::take(&mut full.shapes), full.pixels_per_point);
            started.elapsed().as_secs_f64() * 1000.0
        };
        let stats = |v: &[f64]| {
            let avg = v.iter().sum::<f64>() / v.len() as f64;
            let max = v.iter().copied().fold(0.0, f64::max);
            (avg, max)
        };

        editor.request_focus();
        for _ in 0..3 {
            timed(&mut editor, &mut doc, vec![]);
        }

        // 자주 나오는 단어를 선택 → 강조가 나타나는 프레임
        let word = "인코딩";
        let start = doc.text()[..doc.text().find(word).unwrap()].chars().count();
        doc.set_cursor_char_range(start..start + word.chars().count());
        editor.restore_view(&doc);
        let select_ms = timed(&mut editor, &mut doc, vec![]);
        let appear_ms = timed(&mut editor, &mut doc, vec![]);
        let count = editor.highlight_count();
        let idle: Vec<f64> = (0..10)
            .map(|_| timed(&mut editor, &mut doc, vec![]))
            .collect();

        // 강조가 켜진 문서에 입력 (선택이 입력으로 바뀐 뒤 계속 타이핑)
        let typing: Vec<f64> = (0..30)
            .map(|i| {
                let text = if i % 2 == 0 { "가" } else { "a" };
                timed(&mut editor, &mut doc, vec![Event::Text(text.into())])
            })
            .collect();

        let (idle_avg, idle_max) = stats(&idle);
        let (type_avg, type_max) = stats(&typing);
        println!(
            "1MB ({} 줄): 강조 {count}개 · 선택 프레임 {select_ms:.1}ms · 강조 표시 프레임 {appear_ms:.1}ms · \
             강조 유지 프레임 평균 {idle_avg:.2}ms(최대 {idle_max:.2}) · 입력 프레임 평균 {type_avg:.1}ms(최대 {type_max:.1})",
            doc.line_count()
        );
        assert!(count > 100, "강조 대상이 충분해야 의미 있는 측정");
    }

    #[test]
    fn byte_ranges_convert_to_char_ranges() {
        let text = "한글 abc 한글";
        assert_eq!(
            byte_ranges_to_chars(text, &[0..6, 7..10, 11..17]),
            [0..2, 3..6, 7..9]
        );
    }

    #[test]
    fn highlight_rects_cover_only_visible_matches() {
        let ctx = egui::Context::default();
        let lines: Vec<String> = (0..200).map(|i| format!("줄{i} 강조 끝")).collect();
        let text = lines.join("\n");
        let mut galley = None;
        let mut full = ctx.run_ui(RawInput::default(), |ui| {
            galley = Some(ui.fonts_mut(|f| {
                f.layout_no_wrap(text.clone(), FontId::monospace(14.0), egui::Color32::WHITE)
            }));
        });
        full.textures_delta.clear();
        let galley = galley.unwrap();
        // 줄마다 `강조` 위치 (문자 범위)
        let mut ranges = Vec::new();
        let mut line_start = 0;
        for (i, line) in lines.iter().enumerate() {
            let col = format!("줄{i} ").chars().count();
            ranges.push(line_start + col..line_start + col + 2);
            line_start += line.chars().count() + 1;
        }
        let row = galley.rows[0].rect().height();
        // 10줄 높이만 보이는 영역, 문서는 50줄 위로 스크롤된 상태
        let origin = Pos2::new(0.0, -50.0 * row);
        let clip = Rect::from_min_size(Pos2::ZERO, egui::vec2(400.0, 10.0 * row));
        let rects = highlight_rects(&galley, origin, clip, &ranges);
        assert!(
            (10..=12).contains(&rects.len()),
            "보이는 줄의 매치만 ({})",
            rects.len()
        );
        assert!(
            rects
                .iter()
                .all(|r| r.max.y >= clip.min.y && r.min.y <= clip.max.y)
        );
        // 사각형 폭은 `강조` 두 글자 폭
        let two = galley.pos_from_cursor(CCursor::new(ranges[0].end)).min.x
            - galley.pos_from_cursor(CCursor::new(ranges[0].start)).min.x;
        assert!(rects.iter().all(|r| (r.width() - two).abs() < 0.5));
    }

    #[test]
    fn go_to_line_moves_cursor_and_centers_line() {
        let ctx = egui::Context::default();
        let mut editor = EditorState::new(40);
        let text: String = (1..=1000).map(|i| format!("{i}번째 줄\n")).collect();
        let mut doc = Document::from_bytes(None, text.as_bytes(), false, None).unwrap();
        frame(&ctx, &mut editor, &mut doc, vec![]);

        editor.go_to_line(&mut doc, 500);
        for _ in 0..3 {
            frame(&ctx, &mut editor, &mut doc, vec![]);
        }
        assert_eq!(cursor_status(&doc), (500, 1, 0));
        // 화면(높이 600px)에 그려진 줄 번호 `500`이 세로 가운데쯤에 있다
        let (numbers, _) = gutter_frame(&ctx, &mut editor, &mut doc, true);
        let row = ctx.fonts_mut(|f| f.row_height(&FontId::monospace(14.0)));
        let y = numbers
            .iter()
            .find(|n| n.0 == 500)
            .expect("500번 줄이 보임")
            .2
            + row / 2.0;
        assert!((y - 300.0).abs() < row * 2.0, "500번 줄 높이 {y}");

        // 범위를 넘으면 마지막 줄로
        editor.go_to_line(&mut doc, 99_999);
        frame(&ctx, &mut editor, &mut doc, vec![]);
        assert_eq!(cursor_status(&doc).0, doc.line_count());
    }

    #[test]
    fn tab_key_expands_to_next_tab_stop() {
        let tab = || key(Key::Tab, Modifiers::NONE);
        let texts = |events: &[Event]| -> Vec<String> {
            events
                .iter()
                .map(|e| match e {
                    Event::Text(t) => t.clone(),
                    other => format!("{other:?}"),
                })
                .collect()
        };
        let mut events = vec![tab()];
        expand_tab_key(&mut events, 4, 0);
        assert_eq!(texts(&events), ["    "]);
        let mut events = vec![tab(), Event::Text("ab".into()), tab()];
        expand_tab_key(&mut events, 4, 1);
        assert_eq!(texts(&events), ["   ", "ab", "  "]);
        let mut events = vec![tab()];
        expand_tab_key(&mut events, 8, 8);
        assert_eq!(texts(&events), ["        "]);
        // Shift+Tab·Ctrl+Tab은 그대로
        let mut events = vec![key(Key::Tab, Modifiers::SHIFT)];
        expand_tab_key(&mut events, 4, 0);
        assert!(matches!(events[0], Event::Key { .. }));
    }

    #[test]
    fn tab_spaces_option_inserts_spaces_in_editor() {
        let (ctx, mut editor, mut doc) = focused_editor();
        let spaces = EditorOptions {
            tab_spaces: true,
            tab_width: 4,
            ..options()
        };
        frame_with(
            &ctx,
            &mut editor,
            &mut doc,
            vec![Event::Text("ab".into())],
            &spaces,
        );
        frame_with(
            &ctx,
            &mut editor,
            &mut doc,
            vec![key(Key::Tab, Modifiers::NONE), Event::Text("c".into())],
            &spaces,
        );
        assert_eq!(doc.text(), "ab  c");
        // 끄면 탭 문자
        frame(
            &ctx,
            &mut editor,
            &mut doc,
            vec![key(Key::Tab, Modifiers::NONE)],
        );
        assert_eq!(doc.text(), "ab  c\t");
    }

    #[test]
    fn zoomed_font_enlarges_line_numbers() {
        let ctx = egui::Context::default();
        let mut editor = EditorState::new(41);
        let mut doc = Document::from_bytes(None, b"a\nb\nc", false, None).unwrap();
        let number_height = |ctx: &egui::Context,
                             editor: &mut EditorState,
                             doc: &mut Document,
                             size: f32| {
            let options = EditorOptions {
                font: FontId::monospace(size),
                ..options()
            };
            frame_with(ctx, editor, doc, vec![], &options);
            let raw = RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 600.0),
                )),
                ..Default::default()
            };
            let mut full = ctx.run_ui(raw, |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    editor.show(ui, doc, &options, &Palette::light());
                });
            });
            full.textures_delta.clear();
            full.shapes
                .iter()
                .find_map(|c| match &c.shape {
                    egui::Shape::Text(t) if t.galley.text() == "2" => Some(t.galley.rect.height()),
                    _ => None,
                })
                .unwrap()
        };
        let normal = number_height(&ctx, &mut editor, &mut doc, 14.0);
        let zoomed = number_height(&ctx, &mut editor, &mut doc, 28.0);
        assert!(
            zoomed > normal * 1.8,
            "줄 번호도 함께 커짐 ({normal} → {zoomed})"
        );
    }

    #[test]
    fn lf_char_index_drops_only_crlf_carriage_returns() {
        // "a\r\nb\rc": \r\n의 \r만 사라지고 단독 \r은 \n으로 바뀌어 글자 수 유지
        let text = "a\r\nb\rc";
        assert_eq!(lf_char_index(text, 0), 0);
        assert_eq!(lf_char_index(text, 1), 1);
        assert_eq!(lf_char_index(text, 3), 2);
        assert_eq!(lf_char_index(text, 6), 5);
        assert_eq!(lf_char_index("한\r\n글", 4), 3);
    }
}
