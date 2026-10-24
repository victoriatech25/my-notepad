//! 대용량 읽기 전용 뷰어 (ADR 0001 — 5MB 초과 파일, P1-14).
//!
//! 편집 위젯(방식 A `TextEdit`)은 문서 전체를 한 번에 레이아웃하므로 10MB 문서에 수 초·약 2GB가 든다 (P0-06/07).
//! 뷰어는 [`LineIndex`](crate::core::buffer::LineIndex)로 **보이는 줄만** 그린다.
//!
//! - 세로 스크롤은 `줄 수 × 줄 높이` 크기의 가상 영역 (스크롤바 드래그·휠 그대로 동작). 수백만 줄은
//!   [`MAX_VIRTUAL_HEIGHT`] 안에서 비율로 줄에 대응시켜 `f32` 정밀도 한계를 피한다
//! - 줄 번호 Gutter는 가로 스크롤과 상관없이 왼쪽에 고정
//! - 캐럿·선택은 문서(`LF` 정규화) 바이트 오프셋으로 기억. 마우스 클릭·드래그·`Shift`+클릭, 방향키·`Home`/`End`·
//!   `PageUp`/`PageDown`·`Ctrl+Home`/`End`(`Shift`로 선택 확장), `Ctrl/Cmd+A`, `Ctrl/Cmd+C`
//! - 아주 긴 줄(압축된 로그 등)은 앞 [`MAX_RENDER_CHARS`]자까지만 그린다 (한 줄 레이아웃이 프레임을 막지 않게)

use std::ops::Range;
use std::sync::Arc;

use eframe::egui::{
    self, Event, EventFilter, FontId, Id, Key, Pos2, Rect, RichText, Sense, Vec2,
    text::{CCursor, Galley},
};

use crate::core::buffer::{LineIndex, byte_to_char_offset, char_to_byte_offset};
use crate::core::document::Document;
use crate::ui::theme::Palette;

/// 한 줄에서 그리는 최대 글자 수.
pub const MAX_RENDER_CHARS: usize = 4000;

/// 캐럿 이동 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Motion {
    Left,
    Right,
    Up,
    Down,
    LineStart,
    LineEnd,
    PageUp,
    PageDown,
    DocStart,
    DocEnd,
}

/// 스크롤 영역의 최대 높이(px). 이보다 긴 문서는 스크롤 위치를 비율로 줄에 대응시킨다.
///
/// 줄 수 × 줄 높이를 그대로 쓰면 수백만 줄에서 `f32` 정밀도(약 1,670만 px 이후 1px 단위 표현 불가)를 넘어
/// 줄이 겹쳐 그려지고 클릭 위치가 다른 줄로 잡힌다. 이 높이까지는 `f32`가 0.25px 단위로 정확하다.
const MAX_VIRTUAL_HEIGHT: f32 = 2_000_000.0;

/// 탭별 뷰어 상태.
#[derive(Debug)]
pub struct ViewerState {
    id: Id,
    /// 캐럿 (바이트 오프셋, 문자 경계)
    caret: usize,
    /// 선택 시작점 (캐럿과 같으면 선택 없음)
    anchor: usize,
    /// 위/아래 이동 시 유지할 열 (문자 수)
    preferred_col: Option<usize>,
    /// 선택 글자 수 캐시 — 캐럿이 움직인 만큼만 더하고 빼서 갱신한다 (50MB 드래그 선택도 프레임마다 세지 않게)
    selected_chars: usize,
    /// 다음 프레임에 캐럿이 보이도록 스크롤
    scroll_to_caret: bool,
    /// 지난 프레임의 맨 위 줄 (소수 = 줄 안에서 스크롤된 비율)
    top_line: f64,
    /// 지난 프레임의 화면 높이·글 영역 폭·가로 스크롤 위치 (px)
    view_height: f32,
    row_height: f32,
    text_width: f32,
    scroll_x: f32,
    /// 다음 프레임에 맨 위로 보여 줄 줄 (소수 허용)
    pending_top_line: Option<f64>,
    /// 다음 프레임의 가로 스크롤 위치
    pending_scroll_x: Option<f32>,
    /// 가로 폭 캐시 `(문서 revision, 가장 긴 줄 바이트 수, 그려 본 가장 넓은 줄 폭 px)`
    width_cache: Option<(u64, usize, f32)>,
    focus_requested: bool,
}

impl ViewerState {
    pub fn new(id_salt: impl std::hash::Hash + std::fmt::Debug) -> Self {
        Self {
            id: Id::unique(("viewer", id_salt)),
            caret: 0,
            anchor: 0,
            preferred_col: None,
            selected_chars: 0,
            scroll_to_caret: false,
            top_line: 0.0,
            view_height: 400.0,
            row_height: 18.0,
            text_width: 600.0,
            scroll_x: 0.0,
            pending_top_line: None,
            pending_scroll_x: None,
            width_cache: None,
            focus_requested: true,
        }
    }

    pub fn request_focus(&mut self) {
        self.focus_requested = true;
    }

    /// 캐럿 바이트 오프셋.
    pub fn caret(&self) -> usize {
        self.caret
    }

    /// 선택 범위 (바이트, 오름차순).
    pub fn selection(&self) -> Range<usize> {
        self.caret.min(self.anchor)..self.caret.max(self.anchor)
    }

    /// 상태 표시줄용 `(Ln, Col, 선택 글자 수)` — `Ln`/`Col`은 1-기반, 문자 단위.
    pub fn cursor_status(&self, doc: &Document) -> (usize, usize, usize) {
        let (line, col) = doc.line_index().line_col_of_byte(doc.text(), self.caret);
        (line, col, self.selected_chars)
    }

    /// 다음 프레임에 `line`(0-기반)이 맨 위에 오도록 스크롤한다.
    pub fn scroll_to_line(&mut self, line: usize) {
        self.pending_top_line = Some(line as f64);
    }

    /// 지난 프레임의 맨 위 줄 (0-기반, 소수 포함).
    pub fn top_line(&self) -> f64 {
        self.top_line
    }

    /// 모두 선택.
    pub fn select_all(&mut self, doc: &Document) {
        self.set_selection(doc, 0, doc.text().len());
    }

    /// 선택한 텍스트 (없으면 `None`).
    pub fn selected_text<'a>(&self, doc: &'a Document) -> Option<&'a str> {
        let range = self.selection();
        (!range.is_empty()).then(|| &doc.text()[range])
    }

    /// 커서 이동 (`extend`면 선택 확장).
    pub fn move_caret(&mut self, doc: &Document, motion: Motion, extend: bool) {
        if !extend && !self.selection().is_empty() && matches!(motion, Motion::Left | Motion::Right)
        {
            // 선택이 있을 때 ←/→는 선택 끝으로 접는다 (일반 편집기와 같게)
            let range = self.selection();
            let to = if motion == Motion::Left {
                range.start
            } else {
                range.end
            };
            self.set_selection(doc, to, to);
            self.scroll_to_caret = true;
            return;
        }
        let page_lines = self.page_lines();
        let caret = move_caret(
            doc.text(),
            doc.line_index(),
            self.caret,
            motion,
            &mut self.preferred_col,
            page_lines,
        );
        let anchor = if extend { self.anchor } else { caret };
        self.set_selection(doc, anchor, caret);
        self.scroll_to_caret = true;
    }

    /// 선택을 바꾸고 선택 글자 수를 갱신한다. 시작점이 같으면 캐럿이 움직인 구간만 센다.
    fn set_selection(&mut self, doc: &Document, anchor: usize, caret: usize) {
        let text = doc.text();
        let anchor = anchor.min(text.len());
        let caret = caret.min(text.len());
        let count = |a: usize, b: usize| text[a.min(b)..a.max(b)].chars().count();
        self.selected_chars = if anchor != self.anchor {
            count(anchor, caret)
        } else if (self.caret >= anchor) == (caret >= anchor) {
            // 시작점 기준 같은 쪽에서 움직였다: 움직인 만큼만 늘리거나 줄인다
            let moved = count(self.caret, caret);
            if caret.abs_diff(anchor) >= self.caret.abs_diff(anchor) {
                self.selected_chars + moved
            } else {
                self.selected_chars - moved
            }
        } else {
            // 시작점을 넘어갔다: 새 구간(움직인 거리보다 짧다)만 센다
            count(anchor, caret)
        };
        self.anchor = anchor;
        self.caret = caret;
    }

    /// 편집 모드로 바꿀 때 캐럿 위치를 문자 인덱스로 넘긴다.
    pub fn caret_char_range(&self, doc: &Document) -> Range<usize> {
        let text = doc.text();
        let range = self.selection();
        let start = text[..range.start].chars().count();
        let end = start + self.selected_chars;
        if self.caret < self.anchor {
            end..start
        } else {
            start..end
        }
    }

    /// 지난 프레임 기준 한 화면 줄 수 (`PageUp`/`PageDown`).
    fn page_lines(&self) -> usize {
        ((self.view_height / self.row_height).floor() as usize).max(1)
    }

    /// 뷰어를 그리고 입력을 처리한다. 남은 영역을 모두 차지한다.
    pub fn show(&mut self, ui: &mut egui::Ui, doc: &Document, font: &FontId, palette: &Palette) {
        let text = doc.text();
        let index = doc.line_index();
        let line_count = index.line_count();
        let (row_height, char_width) =
            ui.fonts_mut(|f| (f.row_height(font), f.glyph_width(font, '0')));
        self.row_height = row_height;
        let gutter_width = crate::ui::editor::gutter_width(line_count, char_width);
        // 본문은 에디터와 같은 간격(Gutter 간격 + 에디터 안쪽 여백 6px) 뒤에서 시작한다.
        let number_right = gutter_width - crate::ui::editor::gutter_gap(char_width);
        let gutter_width = gutter_width + 6.0;
        let text_color = ui.visuals().text_color();
        let layout = |ui: &egui::Ui, line: usize| -> Option<(Range<usize>, Arc<Galley>)> {
            let range = index.line_byte_range(line, text)?;
            let shown = truncate_chars(&text[range.clone()], MAX_RENDER_CHARS);
            let galley =
                ui.fonts_mut(|f| f.layout_no_wrap(shown.to_owned(), font.clone(), text_color));
            Some((range, galley))
        };

        // ── 가로 폭: 바이트 수 기반 상한 추정 + 실제로 그려 본 가장 넓은 줄 ──
        let (max_line_bytes, drawn_width) = match self.width_cache {
            Some((rev, bytes, width)) if rev == doc.revision() => (bytes, width),
            _ => (longest_line_bytes(index, text.len()), 0.0),
        };
        // 모든 문자는 화면 칸 수 ≤ 바이트 수(ASCII 1/1, 한글 2/3, 이모지 2/4)이고,
        // 그리는 글자는 최대 MAX_RENDER_CHARS자 × 2칸이다.
        let estimated = max_line_bytes.min(MAX_RENDER_CHARS * 2) as f32 * char_width;
        let content_width = gutter_width + estimated.max(drawn_width) + 40.0;

        // ── 세로: 실제 높이가 크면 MAX_VIRTUAL_HEIGHT 안에서 비율로 대응 ──
        let real_height = (line_count + 1) as f64 * row_height as f64;
        let virtual_height = (real_height as f32).min(MAX_VIRTUAL_HEIGHT);
        let view_height = self.view_height;
        let max_top_line = |view_h: f32| (real_height - view_h as f64).max(0.0) / row_height as f64;
        let to_virtual = |top_line: f64, view_h: f32| -> f32 {
            let max_top = max_top_line(view_h);
            let max_off = (virtual_height - view_h).max(0.0);
            if max_top <= 0.0 {
                0.0
            } else {
                ((top_line.clamp(0.0, max_top) / max_top) * max_off as f64) as f32
            }
        };

        // 캐럿이 보이도록 (세로·가로)
        if std::mem::take(&mut self.scroll_to_caret) && self.pending_top_line.is_none() {
            let caret_line = index.line_of_byte(self.caret) as f64;
            let rows = (view_height / row_height) as f64;
            if caret_line < self.top_line {
                self.pending_top_line = Some(caret_line);
            } else if caret_line + 1.0 > self.top_line + rows {
                self.pending_top_line = Some(caret_line + 1.0 - rows);
            }
            if let Some((range, galley)) = layout(ui, caret_line as usize) {
                let col = byte_to_char_offset(&text[range.clone()], self.caret - range.start);
                let x = galley.pos_from_cursor(CCursor::new(col)).min.x;
                let margin = char_width * 4.0;
                if x < self.scroll_x {
                    self.pending_scroll_x = Some((x - margin).max(0.0));
                } else if x > self.scroll_x + self.text_width - char_width * 2.0 {
                    self.pending_scroll_x = Some(x - self.text_width + margin);
                }
            }
        }

        let mut scroll = egui::ScrollArea::both()
            .id_salt(self.id.with("scroll"))
            .auto_shrink([false, false]);
        if let Some(top) = self.pending_top_line.take() {
            scroll = scroll.vertical_scroll_offset(to_virtual(top, view_height));
        }
        if let Some(x) = self.pending_scroll_x.take() {
            scroll = scroll.horizontal_scroll_offset(x);
        }

        let id = self.id;
        let mut widest = drawn_width;
        let output = scroll.show_viewport(ui, |ui, viewport| {
            // 가상 영역 전체를 차지해 스크롤 범위를 만들고, 입력은 뷰어 ID로 받는다 (포커스 유지).
            let (content_rect, _) = ui.allocate_exact_size(
                Vec2::new(content_width.max(ui.available_width()), virtual_height),
                Sense::hover(),
            );
            let response = ui.interact(content_rect, id, Sense::click_and_drag());
            let view_h = viewport.height();
            self.view_height = view_h;
            self.text_width = (viewport.width() - gutter_width).max(char_width);
            self.scroll_x = viewport.min.x;

            // 가상 스크롤 위치 → 맨 위 줄 (소수). 그리기는 화면 위쪽 기준의 작은 좌표만 쓴다.
            let max_off = (virtual_height - view_h).max(0.0);
            let top_line = if max_off > 0.0 {
                (viewport.min.y / max_off).clamp(0.0, 1.0) as f64 * max_top_line(view_h)
            } else {
                0.0
            };
            self.top_line = top_line;
            let first = (top_line.floor() as usize).min(line_count - 1);
            let frac = (top_line - first as f64) as f32;
            let rows = (view_h / row_height).ceil() as usize + 2;
            let last = (first + rows).min(line_count);

            let origin = content_rect.min;
            let text_left = origin.x + gutter_width;
            let visible = Rect::from_min_max(
                origin + viewport.min.to_vec2(),
                origin + viewport.max.to_vec2(),
            );
            let row_y = |line: usize| visible.min.y + ((line - first) as f32 - frac) * row_height;
            let gutter_x = visible.min.x;

            let selection_color = ui.visuals().selection.bg_fill;
            let caret_line = index.line_of_byte(self.caret);
            let sel = self.selection();

            // 보이는 줄만 레이아웃한다.
            let lines: Vec<(usize, Range<usize>, Arc<Galley>)> = (first..last)
                .filter_map(|line| layout(ui, line).map(|(range, galley)| (line, range, galley)))
                .collect();
            for (_, _, galley) in &lines {
                widest = widest.max(galley.size().x);
            }

            // 포인터 → 바이트 오프셋 (클릭·드래그)
            let pointer_byte = |ui: &egui::Ui, pos: Pos2| -> usize {
                let offset = ((pos.y - visible.min.y) / row_height + frac).floor();
                let line = (first as f32 + offset).max(0.0) as usize;
                let Some((range, galley)) = layout(ui, line.min(line_count - 1)) else {
                    return text.len();
                };
                let cursor = galley.cursor_from_pos(Vec2::new(pos.x - text_left, row_height * 0.5));
                let shown = truncate_chars(&text[range.clone()], MAX_RENDER_CHARS);
                range.start + char_to_byte_offset(shown, cursor.index.into())
            };

            if response.drag_started() || response.clicked() {
                response.request_focus();
                if let Some(pos) = response.interact_pointer_pos() {
                    let byte = pointer_byte(ui, pos);
                    let extend = ui.input(|i| i.modifiers.shift);
                    let anchor = if extend { self.anchor } else { byte };
                    self.set_selection(doc, anchor, byte);
                    self.preferred_col = None;
                }
            } else if response.dragged()
                && let Some(pos) = response.interact_pointer_pos()
            {
                let byte = pointer_byte(ui, pos);
                self.set_selection(doc, self.anchor, byte);
                self.preferred_col = None;
                // 위·아래 밖으로 끌면 그 방향으로 한 줄씩 스크롤 (포인터가 멈춰 있어도 계속)
                if pos.y < visible.min.y {
                    self.pending_top_line = Some((top_line - 1.0).max(0.0));
                    ui.ctx().request_repaint();
                } else if pos.y > visible.max.y {
                    self.pending_top_line = Some(top_line + 1.0);
                    ui.ctx().request_repaint();
                }
            }

            if std::mem::take(&mut self.focus_requested) {
                response.request_focus();
            }
            if response.has_focus() {
                ui.memory_mut(|m| {
                    m.set_focus_lock_filter(
                        id,
                        EventFilter {
                            tab: false,
                            horizontal_arrows: true,
                            vertical_arrows: true,
                            escape: false,
                        },
                    );
                });
                self.handle_keys(ui, doc);
            }

            let painter = ui.painter().with_clip_rect(visible);
            let text_clip = Rect::from_min_max(
                Pos2::new(visible.min.x + gutter_width, visible.min.y),
                visible.max,
            );
            let text_painter = painter.with_clip_rect(text_clip);
            let gutter_font = font.clone();

            for (line, range, galley) in &lines {
                let y = row_y(*line);

                // 선택 배경
                if sel.start < range.end + 1 && sel.end > range.start && !sel.is_empty() {
                    let line_text = &text[range.clone()];
                    let a = byte_to_char_offset(line_text, sel.start.saturating_sub(range.start));
                    let b = byte_to_char_offset(
                        line_text,
                        sel.end.min(range.end).saturating_sub(range.start),
                    );
                    let x0 = galley.pos_from_cursor(CCursor::new(a)).min.x;
                    let mut x1 = galley.pos_from_cursor(CCursor::new(b)).min.x;
                    // 줄바꿈까지 선택했으면 줄 끝에 작은 표시
                    if sel.end > range.end {
                        x1 += char_width * 0.6;
                    }
                    if x1 > x0 {
                        text_painter.rect_filled(
                            Rect::from_min_max(
                                Pos2::new(text_left + x0, y),
                                Pos2::new(text_left + x1, y + row_height),
                            ),
                            0.0,
                            selection_color,
                        );
                    }
                }

                text_painter.galley(Pos2::new(text_left, y), galley.clone(), text_color);

                // 줄 번호 (가로 스크롤과 상관없이 고정)
                painter.text(
                    Pos2::new(gutter_x + number_right, y + row_height * 0.5),
                    egui::Align2::RIGHT_CENTER,
                    (line + 1).to_string(),
                    gutter_font.clone(),
                    if *line == caret_line {
                        palette.gutter_current
                    } else {
                        palette.gutter_text
                    },
                );

                // 캐럿
                if *line == caret_line && response.has_focus() {
                    let col = byte_to_char_offset(&text[range.clone()], self.caret - range.start);
                    let x = galley.pos_from_cursor(CCursor::new(col)).min.x;
                    text_painter.vline(
                        text_left + x,
                        y..=y + row_height,
                        egui::Stroke::new(1.5, ui.visuals().text_cursor.stroke.color),
                    );
                }
            }
        });
        let _ = output;
        self.width_cache = Some((doc.revision(), max_line_bytes, widest));
    }

    fn handle_keys(&mut self, ui: &egui::Ui, doc: &Document) {
        let events = ui.input(|i| i.events.clone());
        for event in events {
            match event {
                Event::Copy | Event::Cut => {
                    if let Some(text) = self.selected_text(doc) {
                        ui.ctx().copy_text(text.to_owned());
                    }
                }
                Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } => {
                    let cmd = modifiers.command || modifiers.ctrl || modifiers.mac_cmd;
                    let motion = match key {
                        Key::ArrowLeft => Some(Motion::Left),
                        Key::ArrowRight => Some(Motion::Right),
                        Key::ArrowUp => Some(Motion::Up),
                        Key::ArrowDown => Some(Motion::Down),
                        Key::Home if cmd => Some(Motion::DocStart),
                        Key::End if cmd => Some(Motion::DocEnd),
                        Key::Home => Some(Motion::LineStart),
                        Key::End => Some(Motion::LineEnd),
                        Key::PageUp => Some(Motion::PageUp),
                        Key::PageDown => Some(Motion::PageDown),
                        Key::A if cmd => {
                            self.select_all(doc);
                            None
                        }
                        _ => None,
                    };
                    if let Some(motion) = motion {
                        self.move_caret(doc, motion, modifiers.shift);
                    }
                }
                _ => {}
            }
        }
    }
}

/// 뷰어 위 안내 막대: "읽기 전용으로 보는 중" + `편집 모드로 열기`. 눌렀으면 `true`.
pub fn readonly_bar(ui: &mut egui::Ui, palette: &Palette, size_bytes: u64) -> bool {
    let mut open_edit = false;
    egui::Frame::new()
        .fill(palette.dialog_title_bg)
        .inner_margin(egui::Margin::symmetric(10, 4))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!(
                    "큰 파일({})이라 읽기 전용으로 보는 중입니다. 선택·복사는 할 수 있습니다.",
                    format_size(size_bytes)
                )));
                if ui.button("편집 모드로 열기").clicked() {
                    open_edit = true;
                }
            });
        });
    open_edit
}

/// 1~5MB 편집 위젯 위 안내 막대의 선택.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BannerAction {
    ViewReadOnly,
    Close,
}

/// 편집 위젯 위 안내 막대 (ui-spec §3 대용량 1~5MB): "큰 파일이라 입력이 느릴 수 있습니다 · **읽기 전용으로 보기** · ×".
pub fn size_banner(ui: &mut egui::Ui, palette: &Palette) -> Option<BannerAction> {
    let mut action = None;
    egui::Frame::new()
        .fill(palette.dialog_title_bg)
        .inner_margin(egui::Margin::symmetric(10, 4))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("큰 파일이라 입력이 느릴 수 있습니다")
                        .color(palette.status_warning),
                );
                if ui
                    .button(RichText::new("읽기 전용으로 보기").strong())
                    .clicked()
                {
                    action = Some(BannerAction::ViewReadOnly);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add(egui::Button::new("✕").frame(false))
                        .on_hover_text("안내 닫기")
                        .clicked()
                    {
                        action = Some(BannerAction::Close);
                    }
                });
            });
        });
    action
}

/// `12.3MB`·`850KB` 형식.
pub fn format_size(bytes: u64) -> String {
    let mb = bytes as f64 / 1_048_576.0;
    if mb >= 1.0 {
        format!("{mb:.1}MB")
    } else {
        format!("{:.0}KB", bytes as f64 / 1024.0)
    }
}

/// 가장 긴 줄의 바이트 수 (줄바꿈 포함 근사).
fn longest_line_bytes(index: &LineIndex, text_len: usize) -> usize {
    let starts = index.line_starts();
    starts
        .windows(2)
        .map(|w| w[1] - w[0])
        .chain(std::iter::once(text_len - starts[starts.len() - 1]))
        .max()
        .unwrap_or(0)
}

/// 앞 `max`자까지의 슬라이스.
fn truncate_chars(s: &str, max: usize) -> &str {
    match s.char_indices().nth(max) {
        Some((i, _)) => &s[..i],
        None => s,
    }
}

/// 캐럿을 옮긴 새 바이트 위치. 위/아래 이동은 `preferred_col`(문자 열)을 유지한다.
pub fn move_caret(
    text: &str,
    index: &LineIndex,
    caret: usize,
    motion: Motion,
    preferred_col: &mut Option<usize>,
    page_lines: usize,
) -> usize {
    let line = index.line_of_byte(caret);
    let range = index.line_byte_range(line, text).unwrap_or(0..0);
    let col = byte_to_char_offset(&text[range.clone()], caret - range.start);
    let to_line = |target: usize, preferred: &mut Option<usize>| -> usize {
        let want = *preferred.get_or_insert(col);
        let target = target.min(index.line_count() - 1);
        let r = index.line_byte_range(target, text).unwrap_or(0..0);
        r.start + char_to_byte_offset(&text[r], want)
    };
    let vertical = matches!(
        motion,
        Motion::Up | Motion::Down | Motion::PageUp | Motion::PageDown
    );
    if !vertical {
        *preferred_col = None;
    }
    match motion {
        Motion::Left => {
            if caret == 0 {
                0
            } else {
                text[..caret]
                    .char_indices()
                    .next_back()
                    .map_or(0, |(i, _)| i)
            }
        }
        Motion::Right => text[caret..]
            .chars()
            .next()
            .map_or(caret, |c| caret + c.len_utf8()),
        Motion::Up => {
            if line == 0 {
                0
            } else {
                to_line(line - 1, preferred_col)
            }
        }
        Motion::Down => {
            if line + 1 >= index.line_count() {
                text.len()
            } else {
                to_line(line + 1, preferred_col)
            }
        }
        Motion::PageUp => to_line(line.saturating_sub(page_lines), preferred_col),
        Motion::PageDown => to_line(line + page_lines, preferred_col),
        Motion::LineStart => range.start,
        Motion::LineEnd => range.end,
        Motion::DocStart => 0,
        Motion::DocEnd => text.len(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(text: &str) -> Document {
        Document::from_bytes(None, text.as_bytes(), true, None).unwrap()
    }

    fn mv(d: &Document, caret: usize, motion: Motion, pref: &mut Option<usize>) -> usize {
        move_caret(d.text(), d.line_index(), caret, motion, pref, 2)
    }

    #[test]
    fn caret_moves_by_characters_and_keeps_column() {
        let d = doc("가나다라\nab\n한글 줄입니다\n");
        let mut pref = None;
        // → 두 번: 한글 2글자(6바이트)
        let c = mv(&d, 0, Motion::Right, &mut pref);
        let c = mv(&d, c, Motion::Right, &mut pref);
        assert_eq!(c, 6);
        // ↓: "ab"는 2글자뿐이라 줄 끝, 다시 ↓하면 원래 열(2) 유지
        let c = mv(&d, c, Motion::Down, &mut pref);
        assert_eq!(&d.text()[c..c + 1], "\n");
        let c = mv(&d, c, Motion::Down, &mut pref);
        assert_eq!(d.line_index().line_col_of_byte(d.text(), c), (3, 3));
        // ← 는 열 기억을 지운다
        let c = mv(&d, c, Motion::Left, &mut pref);
        assert_eq!(pref, None);
        assert_eq!(d.line_index().line_col_of_byte(d.text(), c), (3, 2));
        // Home/End, 문서 끝/처음
        let line3 = d.line_index().line_byte_range(2, d.text()).unwrap();
        assert_eq!(mv(&d, c, Motion::LineStart, &mut pref), line3.start);
        assert_eq!(mv(&d, c, Motion::LineEnd, &mut pref), line3.end);
        assert_eq!(mv(&d, c, Motion::DocEnd, &mut pref), d.text().len());
        assert_eq!(mv(&d, c, Motion::DocStart, &mut pref), 0);
        assert_eq!(mv(&d, 0, Motion::Left, &mut pref), 0);
        assert_eq!(mv(&d, 0, Motion::Up, &mut pref), 0);
        // PageDown은 2줄씩, 끝을 넘지 않는다
        let mut pref = None;
        let c = mv(&d, 0, Motion::PageDown, &mut pref);
        assert_eq!(d.line_index().line_of_byte(c), 2);
        let c = mv(&d, c, Motion::PageDown, &mut pref);
        assert_eq!(d.line_index().line_of_byte(c), 3);
    }

    #[test]
    fn selection_counts_characters_and_copies_lf_text() {
        let d = doc("첫 줄\r\n둘째 줄\r\n");
        let mut v = ViewerState::new(1);
        v.select_all(&d);
        assert_eq!(v.selected_text(&d), Some("첫 줄\n둘째 줄\n"));
        assert_eq!(v.cursor_status(&d), (3, 1, 9));

        v.move_caret(&d, Motion::Left, false);
        assert_eq!(v.selection(), 0..0);
        v.move_caret(&d, Motion::LineEnd, true);
        assert_eq!(v.selected_text(&d), Some("첫 줄"));
        assert_eq!(v.cursor_status(&d), (1, 4, 3));
        assert_eq!(v.caret_char_range(&d), 0..3);
    }

    /// 줄 색인: CRLF / LF / Mixed, 마지막 줄 개행 유무, 한글 멀티바이트 경계 (WORKPLAN P1-14 🤖)
    #[test]
    fn line_index_over_line_endings_and_multibyte() {
        for (text, lines, last) in [
            ("가\r\n나\r\n다", 3, "다"),   // CRLF, 끝 개행 없음
            ("가\n나\n다\n", 4, ""),       // LF, 끝 개행 있음 → 빈 마지막 줄
            ("가\r\n나\n다\r라", 4, "라"), // Mixed (CRLF·LF·단독 CR)
            ("한 줄", 1, "한 줄"),
            ("", 1, ""),
        ] {
            let d = doc(text);
            let index = d.line_index();
            assert_eq!(index.line_count(), lines, "{text:?}");
            assert_eq!(index.line_str(lines - 1, d.text()), Some(last), "{text:?}");
            // 모든 줄 범위가 문자 경계이고, `\r`이 남지 않는다
            for line in 0..lines {
                let r = index.line_byte_range(line, d.text()).unwrap();
                assert!(d.text().is_char_boundary(r.start) && d.text().is_char_boundary(r.end));
                assert!(!d.text()[r].contains('\r'));
            }
        }
        // 멀티바이트 중간 바이트도 같은 줄·앞 글자 열로 보정
        let d = doc("ab\n가나다");
        let second = d.line_index().line_start_byte(1).unwrap();
        assert_eq!(
            d.line_index().line_col_of_byte(d.text(), second + 4),
            (2, 2)
        );
    }

    /// 큰 문서도 한 프레임에 그려지고, 모두 선택 → 복사가 LF 텍스트를 클립보드로 보낸다.
    #[test]
    fn renders_large_document_and_copies_selection() {
        use eframe::egui::{Modifiers, RawInput};

        let text: String = (0..200_000).map(|i| format!("{i}번째 줄 내용\n")).collect();
        let d = doc(&text);
        let mut v = ViewerState::new(9);
        let ctx = egui::Context::default();
        let palette = Palette::dark();
        let mut copied = String::new();
        let run = |events: Vec<Event>, v: &mut ViewerState, copied: &mut String| {
            let raw = RawInput {
                events,
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 600.0))),
                ..Default::default()
            };
            let mut full = ctx.run_ui(raw, |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    v.show(ui, &d, &FontId::monospace(14.0), &palette);
                });
            });
            full.textures_delta.clear();
            for cmd in full.platform_output.commands {
                if let egui::OutputCommand::CopyText(t) = cmd {
                    *copied = t;
                }
            }
        };
        let started = std::time::Instant::now();
        run(vec![], &mut v, &mut copied);
        assert!(started.elapsed().as_secs() < 5, "보이는 줄만 그려야 한다");
        run(
            vec![Event::Key {
                key: Key::A,
                physical_key: Some(Key::A),
                pressed: true,
                repeat: false,
                modifiers: Modifiers::COMMAND,
            }],
            &mut v,
            &mut copied,
        );
        assert_eq!(v.selection(), 0..d.text().len());
        run(vec![Event::Copy], &mut v, &mut copied);
        assert_eq!(copied.len(), d.text().len());
        assert!(copied.starts_with("0번째 줄 내용\n1번째"));

        // ↓ 로 캐럿 이동 후 상태 표시줄 값
        run(
            vec![Event::Key {
                key: Key::ArrowDown,
                physical_key: Some(Key::ArrowDown),
                pressed: true,
                repeat: false,
                modifiers: Modifiers::NONE,
            }],
            &mut v,
            &mut copied,
        );
        assert_eq!(v.cursor_status(&d).2, 0);
    }

    #[test]
    fn helpers_handle_long_lines_and_boundaries() {
        let long = "가".repeat(MAX_RENDER_CHARS + 10);
        assert_eq!(
            truncate_chars(&long, MAX_RENDER_CHARS).chars().count(),
            MAX_RENDER_CHARS
        );
        let d = doc("a\nbbbb\ncc");
        assert_eq!(longest_line_bytes(d.line_index(), d.text().len()), 5);
        assert_eq!(format_size(12 * 1_048_576 + 400_000), "12.4MB");
        assert_eq!(format_size(2048), "2KB");
    }

    /// 헤드리스 프레임 하나 (`events`를 넣고 뷰어를 그린다).
    fn view_frame(
        ctx: &egui::Context,
        v: &mut ViewerState,
        d: &Document,
        events: Vec<Event>,
    ) -> Vec<f32> {
        let raw = egui::RawInput {
            events,
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 600.0))),
            ..Default::default()
        };
        let mut full = ctx.run_ui(raw, |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                v.show(ui, d, &FontId::monospace(14.0), &Palette::dark());
            });
        });
        full.textures_delta.clear();
        // 본문 글자(줄 번호가 아닌 'x' 줄) 텍스트 도형의 y 좌표
        let mut ys: Vec<f32> = full
            .shapes
            .iter()
            .filter_map(|c| match &c.shape {
                egui::Shape::Text(t) if t.galley.text() == "x" => Some(t.pos.y),
                _ => None,
            })
            .collect();
        ys.sort_by(f32::total_cmp);
        ys
    }

    fn key(key: Key, modifiers: egui::Modifiers) -> Event {
        Event::Key {
            key,
            physical_key: Some(key),
            pressed: true,
            repeat: false,
            modifiers,
        }
    }

    /// 리뷰 7: 선택 글자 수를 움직인 만큼만 갱신해도 매번 세는 값과 같아야 한다 (시작점 넘나들기 포함).
    #[test]
    fn incremental_selection_count_matches_full_count() {
        let d = doc("가나다 abc\n한글 둘째 줄\n셋째\n");
        let len = d.text().len();
        let mut v = ViewerState::new(5);
        let boundaries: Vec<usize> = (0..=len)
            .filter(|&i| d.text().is_char_boundary(i))
            .collect();
        let anchor = boundaries[boundaries.len() / 2];
        v.set_selection(&d, anchor, anchor);
        // 시작점 기준 앞뒤로 오가며 확장·축소
        for &caret in boundaries.iter().rev().chain(boundaries.iter()).step_by(3) {
            v.set_selection(&d, anchor, caret);
            let range = v.selection();
            assert_eq!(
                v.selected_chars,
                d.text()[range].chars().count(),
                "caret {caret}"
            );
        }
    }

    /// 리뷰 3: 수백만 줄 문서에서도 끝부분 줄이 일정한 간격으로 그려지고(겹침·틈 없음) 클릭이 그 줄을 고른다.
    #[test]
    fn multi_million_lines_map_click_to_right_line() {
        let lines = 3_000_000;
        let text = "x\n".repeat(lines);
        let d = doc(&text);
        let ctx = egui::Context::default();
        let mut v = ViewerState::new(6);
        view_frame(&ctx, &mut v, &d, vec![]);
        let target = lines - 1000;
        v.scroll_to_line(target);
        view_frame(&ctx, &mut v, &d, vec![]);
        let ys = view_frame(&ctx, &mut v, &d, vec![]);
        assert!(
            (v.top_line() - target as f64).abs() < 1.0,
            "top {}",
            v.top_line()
        );
        let row = v.row_height;
        assert!(ys.len() > 10);
        for pair in ys.windows(2) {
            let gap = pair[1] - pair[0];
            assert!((gap - row).abs() < 0.5, "줄 간격 {gap} (줄 높이 {row})");
        }

        // 화면 맨 위 줄 가운데를 클릭 → 그 줄 (CentralPanel 안쪽 여백만큼 아래)
        let row = 18.0_f32.max(1.0);
        let click = Pos2::new(200.0, 8.0 + row * 2.5);
        view_frame(
            &ctx,
            &mut v,
            &d,
            vec![
                Event::PointerMoved(click),
                Event::PointerButton {
                    pos: click,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
                Event::PointerButton {
                    pos: click,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        let line = d.line_index().line_of_byte(v.caret());
        assert!(
            line >= target && line <= target + 4,
            "클릭한 줄 {line}, 맨 위 줄 {target}"
        );
    }

    /// 리뷰 2·6: 한글 긴 줄에서 `End`를 누르면 가로로 스크롤되어 캐럿이 화면 안에 보인다.
    #[test]
    fn end_on_long_korean_line_scrolls_caret_into_view() {
        let text = format!("{}\n짧은 줄\n", "가".repeat(3000));
        let d = doc(&text);
        let ctx = egui::Context::default();
        let mut v = ViewerState::new(7);
        view_frame(&ctx, &mut v, &d, vec![]);
        view_frame(&ctx, &mut v, &d, vec![key(Key::End, egui::Modifiers::NONE)]);
        for _ in 0..3 {
            view_frame(&ctx, &mut v, &d, vec![]);
        }
        let line = d.line_index().line_byte_range(0, d.text()).unwrap();
        assert_eq!(v.caret(), line.end);
        // 캐럿 x(글자 3000개 폭)가 지금 보이는 가로 범위 안
        let caret_x = ctx.fonts_mut(|f| {
            f.layout_no_wrap(
                "가".repeat(3000),
                FontId::monospace(14.0),
                egui::Color32::WHITE,
            )
            .size()
            .x
        });
        assert!(
            caret_x >= v.scroll_x && caret_x <= v.scroll_x + v.text_width,
            "캐럿 {caret_x}, 보이는 범위 {}..{}",
            v.scroll_x,
            v.scroll_x + v.text_width
        );
    }
}
