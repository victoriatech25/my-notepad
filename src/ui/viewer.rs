//! 대용량 읽기 전용 뷰어 (ADR 0001 — 5MB 초과 파일, P1-14).
//!
//! 편집 위젯(방식 A `TextEdit`)은 문서 전체를 한 번에 레이아웃하므로 10MB 문서에 수 초·약 2GB가 든다 (P0-06/07).
//! 뷰어는 [`LineIndex`](crate::core::buffer::LineIndex)로 **보이는 줄만** 그린다.
//!
//! - 세로 스크롤은 `줄 수 × 줄 높이` 크기의 가상 영역 (스크롤바 드래그·휠 그대로 동작)
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

use crate::core::buffer::LineIndex;
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
    /// 선택 글자 수 캐시 (50MB 전체 선택을 매 프레임 세지 않게)
    selected_chars: usize,
    /// 다음 프레임에 캐럿이 보이도록 스크롤
    scroll_to_caret: bool,
    /// 지난 프레임의 한 화면 줄 수 (`PageUp`/`PageDown`)
    page_lines: usize,
    /// 지난 프레임의 세로 스크롤 위치
    scroll_y: f32,
    /// 다음 프레임에 맨 위로 보여 줄 줄 (측정·줄 이동용)
    pending_scroll_line: Option<usize>,
    /// 가장 긴 줄 바이트 수 캐시 (`(문서 revision, 바이트 수)`)
    max_line_bytes: Option<(u64, usize)>,
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
            page_lines: 20,
            scroll_y: 0.0,
            pending_scroll_line: None,
            max_line_bytes: None,
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
        self.pending_scroll_line = Some(line);
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
        let caret = move_caret(
            doc.text(),
            doc.line_index(),
            self.caret,
            motion,
            &mut self.preferred_col,
            self.page_lines,
        );
        let anchor = if extend { self.anchor } else { caret };
        self.set_selection(doc, anchor, caret);
        self.scroll_to_caret = true;
    }

    fn set_selection(&mut self, doc: &Document, anchor: usize, caret: usize) {
        let text = doc.text();
        self.anchor = anchor.min(text.len());
        self.caret = caret.min(text.len());
        let range = self.selection();
        self.selected_chars = text[range].chars().count();
    }

    /// 편집 모드로 바꿀 때 캐럿 위치를 문자 인덱스로 넘긴다.
    pub fn caret_char_range(&self, doc: &Document) -> Range<usize> {
        let text = doc.text();
        let range = self.selection();
        let start = text[..range.start].chars().count();
        let end = start + text[range].chars().count();
        if self.caret < self.anchor {
            end..start
        } else {
            start..end
        }
    }

    /// 뷰어를 그리고 입력을 처리한다. 남은 영역을 모두 차지한다.
    pub fn show(&mut self, ui: &mut egui::Ui, doc: &Document, font: &FontId, palette: &Palette) {
        let text = doc.text();
        let index = doc.line_index();
        let line_count = index.line_count();
        let (row_height, char_width) =
            ui.fonts_mut(|f| (f.row_height(font), f.glyph_width(font, '0')));
        let digits = line_count.to_string().len().max(3);
        let gutter_width = digits as f32 * char_width + 20.0;

        let max_line_bytes = match self.max_line_bytes {
            Some((rev, bytes)) if rev == doc.revision() => bytes,
            _ => {
                let bytes = longest_line_bytes(index, text.len());
                self.max_line_bytes = Some((doc.revision(), bytes));
                bytes
            }
        };
        // 바이트 수 ≥ 글자 수이고 한글은 2칸 폭이라 넉넉한 추정치가 된다.
        let content_width =
            gutter_width + max_line_bytes.min(MAX_RENDER_CHARS) as f32 * char_width + 40.0;
        let content_height = line_count as f32 * row_height + row_height;

        let mut scroll = egui::ScrollArea::both()
            .id_salt(self.id.with("scroll"))
            .auto_shrink([false, false]);
        if let Some(line) = self.pending_scroll_line.take() {
            scroll = scroll.vertical_scroll_offset(line.min(line_count - 1) as f32 * row_height);
        } else if std::mem::take(&mut self.scroll_to_caret) {
            let caret_line = index.line_of_byte(self.caret);
            let y = caret_line as f32 * row_height;
            let view_h = (self.page_lines as f32) * row_height;
            if y < self.scroll_y {
                scroll = scroll.vertical_scroll_offset(y);
            } else if y + row_height > self.scroll_y + view_h {
                scroll = scroll.vertical_scroll_offset(y + row_height - view_h);
            }
        }

        let id = self.id;
        let output = scroll.show_viewport(ui, |ui, viewport| {
            // 가상 영역 전체를 차지해 스크롤 범위를 만들고, 입력은 뷰어 ID로 받는다 (포커스 유지).
            let (content_rect, _) = ui.allocate_exact_size(
                Vec2::new(content_width.max(ui.available_width()), content_height),
                Sense::hover(),
            );
            let response = ui.interact(content_rect, id, Sense::click_and_drag());
            self.page_lines = ((viewport.height() / row_height).floor() as usize).max(1);

            let first =
                ((viewport.min.y / row_height).floor().max(0.0) as usize).min(line_count - 1);
            let last = (((viewport.max.y / row_height).ceil() as usize) + 1).min(line_count);
            let origin = content_rect.min;
            let text_left = origin.x + gutter_width;
            let visible = Rect::from_min_max(
                origin + viewport.min.to_vec2(),
                origin + viewport.max.to_vec2(),
            );
            let gutter_x = visible.min.x;

            let text_color = ui.visuals().text_color();
            let selection_color = ui.visuals().selection.bg_fill;
            let caret_line = index.line_of_byte(self.caret);
            let sel = self.selection();

            // 보이는 줄만 레이아웃한다.
            let lines: Vec<(usize, Range<usize>, Arc<Galley>)> = (first..last)
                .filter_map(|line| {
                    let range = index.line_byte_range(line, text)?;
                    let shown = truncate_chars(&text[range.clone()], MAX_RENDER_CHARS);
                    let galley = ui.fonts_mut(|f| {
                        f.layout_no_wrap(shown.to_owned(), font.clone(), text_color)
                    });
                    Some((line, range, galley))
                })
                .collect();

            // 포인터 → 바이트 오프셋 (클릭·드래그)
            let pointer_byte = |pos: Pos2| -> usize {
                let line = (((pos.y - origin.y) / row_height).floor().max(0.0) as usize)
                    .min(line_count - 1);
                let Some(range) = index.line_byte_range(line, text) else {
                    return text.len();
                };
                let line_text = &text[range.clone()];
                let shown = truncate_chars(line_text, MAX_RENDER_CHARS);
                let galley =
                    ui.fonts_mut(|f| f.layout_no_wrap(shown.to_owned(), font.clone(), text_color));
                let cursor = galley.cursor_from_pos(Vec2::new(pos.x - text_left, row_height * 0.5));
                range.start + char_to_byte(shown, cursor.index.into())
            };

            if response.drag_started() || response.clicked() {
                response.request_focus();
                if let Some(pos) = response.interact_pointer_pos() {
                    let byte = pointer_byte(pos);
                    let extend = ui.input(|i| i.modifiers.shift);
                    let anchor = if extend { self.anchor } else { byte };
                    self.set_selection(doc, anchor, byte);
                    self.preferred_col = None;
                }
            } else if response.dragged()
                && let Some(pos) = response.interact_pointer_pos()
            {
                let byte = pointer_byte(pos);
                self.set_selection(doc, self.anchor, byte);
                self.preferred_col = None;
                // 화면 밖으로 끌면 그 방향으로 스크롤
                if !visible.contains(pos) {
                    ui.scroll_to_rect(Rect::from_center_size(pos, Vec2::splat(row_height)), None);
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
                let y = origin.y + *line as f32 * row_height;

                // 선택 배경
                if sel.start < range.end + 1 && sel.end > range.start && !sel.is_empty() {
                    let line_text = &text[range.clone()];
                    let a = char_count_to(line_text, sel.start.saturating_sub(range.start));
                    let b = char_count_to(
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
                    Pos2::new(gutter_x + gutter_width - 10.0, y + row_height * 0.5),
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
                    let col = char_count_to(&text[range.clone()], self.caret - range.start);
                    let x = galley.pos_from_cursor(CCursor::new(col)).min.x;
                    text_painter.vline(
                        text_left + x,
                        y..=y + row_height,
                        egui::Stroke::new(1.5, ui.visuals().text_cursor.stroke.color),
                    );
                }
            }
            // Gutter 배경 경계선
            painter.vline(
                gutter_x + gutter_width - 4.0,
                visible.y_range(),
                egui::Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color),
            );
        });
        self.scroll_y = output.state.clamped_offset().y;
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

/// `line` 안 `chars`번째 문자의 바이트 위치 (넘치면 줄 끝).
fn char_to_byte(line: &str, chars: usize) -> usize {
    line.char_indices()
        .nth(chars)
        .map_or(line.len(), |(i, _)| i)
}

/// `line`의 앞 `bytes` 바이트에 들어 있는 문자 수 (문자 경계가 아니면 그 앞 경계까지).
fn char_count_to(line: &str, bytes: usize) -> usize {
    let mut end = bytes.min(line.len());
    while !line.is_char_boundary(end) {
        end -= 1;
    }
    line[..end].chars().count()
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
    let col = char_count_to(&text[range.clone()], caret - range.start);
    let to_line = |target: usize, preferred: &mut Option<usize>| -> usize {
        let want = *preferred.get_or_insert(col);
        let target = target.min(index.line_count() - 1);
        let r = index.line_byte_range(target, text).unwrap_or(0..0);
        r.start + char_to_byte(&text[r], want)
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
        assert_eq!(char_to_byte("가나", 1), 3);
        assert_eq!(char_to_byte("가나", 9), 6);
        assert_eq!(char_count_to("가나", 4), 1);
        let d = doc("a\nbbbb\ncc");
        assert_eq!(longest_line_bytes(d.line_index(), d.text().len()), 5);
        assert_eq!(format_size(12 * 1_048_576 + 400_000), "12.4MB");
        assert_eq!(format_size(2048), "2KB");
    }
}
