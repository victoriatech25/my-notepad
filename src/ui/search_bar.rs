//! 찾기 바 UI (PLAN.md §3.2-2, ui-spec §1 — P4-04). 바꾸기 줄은 P4-05에서 붙인다.
//!
//! - 탭 바 아래·사이드바 오른쪽, 에디터 바로 위에 한 줄로 펼쳐진다:
//!   `[검색어 입력란] [.*] [Aa] [\b]  3 / 28  [↑] [↓] … [×]`
//! - 검색은 지금은 UI 스레드에서 동기로 한다 (P4-03에서 백그라운드로 바꾼다). 탭·문서 `revision`·검색어·옵션이
//!   바뀔 때만 다시 찾고, 매치는 문자 범위로 캐시해 에디터 강조([`SearchMarks`])와 이동에 같이 쓴다.
//! - 현재 매치는 문서 선택 범위와 똑같은 매치다 (번호를 따로 들고 있지 않는다). 커서를 옮기면 `? / 28`.
//! - 검색어나 옵션이 바뀌면 커서(선택 시작) 이후 첫 매치를 바로 선택해 보여 준다 (입력하는 즉시 이동).
//! - 입력란 키: `Enter` 다음 · `Shift+Enter` 이전 · `Esc` 닫기. `F3`/`Shift+F3`(macOS `Cmd+G`/`Cmd+Shift+G`)는 전역 단축키.
//! - 큰 파일 읽기 전용 뷰어 탭에서는 아직 찾지 않는다 (P4-03 백그라운드 검색과 함께 연결).

use std::ops::Range;
use std::sync::Arc;

use eframe::egui::{
    self, Align, FontFamily, Id, Key, Layout, Modifiers, RichText, Stroke, StrokeKind,
    text::{CCursor, CCursorRange},
    text_edit::TextEditState,
};

use crate::core::document::Document;
use crate::core::search::{
    SearchError, SearchOptions, SearchQuery, byte_ranges_to_char_ranges, char_range_to_byte_range,
    current_match_index, next_match_index, prev_match_index,
};
use crate::fonts::ICON_FAMILY;
use crate::ui::editor::SearchMarks;
use crate::ui::theme::Palette;

/// 검색어 입력란 폭 (논리 픽셀).
const INPUT_WIDTH: f32 = 260.0;

mod glyph {
    pub const ARROW_UP: &str = "\u{E08E}";
    pub const ARROW_DOWN: &str = "\u{E03E}";
    pub const X: &str = "\u{E4F6}";
}

/// 찾기 바에서 사용자가 고른 동작.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BarEvent {
    Next,
    Prev,
    Close,
}

/// 다음/이전 찾기 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// 이 매치(문자 범위)로 옮긴다.
    Moved(Range<usize>),
    /// 문서 끝(이전 찾기는 처음)을 지나 반대쪽 끝에서 이어 찾았다.
    Wrapped(Range<usize>),
    /// Wrap Around를 꺼 둔 상태에서 더 찾을 매치가 없다.
    End,
    /// 매치가 없다 (빈 검색어·오류 포함).
    NoMatch,
}

/// 매치 캐시: 이 조건으로 찾은 결과.
#[derive(Debug)]
struct MatchCache {
    tab_id: u64,
    revision: u64,
    pattern: String,
    options: SearchOptions,
    /// 매치 문자 범위 (오름차순), 또는 검색어 오류
    result: Result<Arc<Vec<Range<usize>>>, SearchError>,
    /// 계산 당시 문서 텍스트 길이 (바이트)
    text_len: usize,
}

/// 찾기 바 상태 (앱에 하나, 활성 탭을 대상으로 찾는다).
#[derive(Debug)]
pub struct SearchBar {
    open: bool,
    pattern: String,
    options: SearchOptions,
    /// 다음 프레임에 입력란으로 포커스를 옮기고 내용을 모두 선택한다.
    focus_input: bool,
    cache: Option<MatchCache>,
}

impl SearchBar {
    pub fn new(options: SearchOptions) -> Self {
        Self {
            open: false,
            pattern: String::new(),
            options,
            focus_input: false,
            cache: None,
        }
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    /// 현재 찾기 옵션 (설정 저장용).
    pub fn options(&self) -> SearchOptions {
        self.options
    }

    pub fn has_pattern(&self) -> bool {
        !self.pattern.is_empty()
    }

    /// 입력란 위젯 ID.
    pub fn input_id() -> Id {
        Id::unique("search_bar_input")
    }

    /// 입력란이 키보드 포커스를 가졌는지.
    pub fn input_has_focus(&self, ctx: &egui::Context) -> bool {
        self.open && ctx.memory(|m| m.has_focus(Self::input_id()))
    }

    /// 찾기 바를 열고 입력란에 포커스를 준다 (`Ctrl+F`).
    ///
    /// `doc`의 선택이 한 줄 안이고 비어 있지 않으면 그 텍스트를 검색어로 채운다. 입력란 내용은 모두 선택해
    /// 바로 새 검색어를 칠 수 있게 한다.
    pub fn open(&mut self, doc: Option<&Document>) {
        if let Some(doc) = doc {
            let text = doc.text();
            let selected = &text[char_range_to_byte_range(text, doc.cursor_char_range())];
            if !selected.is_empty() && !selected.contains('\n') {
                self.pattern = selected.to_owned();
            }
        }
        self.open = true;
        self.focus_input = true;
    }

    /// 찾기 바를 닫는다 (강조도 사라진다).
    pub fn close(&mut self) {
        self.open = false;
        self.focus_input = false;
    }

    pub fn toggle_wrap_around(&mut self) {
        self.options.wrap_around = !self.options.wrap_around;
    }

    /// 활성 탭 문서에 맞춰 매치를 다시 찾는다 (조건이 그대로면 아무것도 하지 않음).
    ///
    /// 검색어나 옵션이 지난 계산과 달라졌으면 `true` — 호출하는 쪽이 첫 매치로 이동한다.
    /// 탭 전환이나 문서 편집으로만 다시 찾은 경우는 `false` (커서를 빼앗지 않는다).
    /// `searchable`이 `false`(읽기 전용 뷰어)면 매치 없음으로 둔다.
    pub fn refresh(&mut self, tab_id: u64, doc: &Document, searchable: bool) -> bool {
        let fresh = self.cache.as_ref().is_some_and(|c| {
            c.tab_id == tab_id
                && c.revision == doc.revision()
                && c.pattern == self.pattern
                && c.options == self.options
        });
        if fresh {
            return false;
        }
        let query_changed = self
            .cache
            .as_ref()
            .is_none_or(|c| c.pattern != self.pattern || c.options != self.options);
        let text = doc.text();
        let result = SearchQuery::new(&self.pattern, &self.options).map(|query| {
            let bytes = if searchable {
                query.find_all(text)
            } else {
                Vec::new()
            };
            Arc::new(byte_ranges_to_char_ranges(text, &bytes))
        });
        self.cache = Some(MatchCache {
            tab_id,
            revision: doc.revision(),
            pattern: self.pattern.clone(),
            options: self.options,
            result,
            text_len: text.len(),
        });
        query_changed && self.has_pattern()
    }

    /// 지난 [`Self::refresh`]로 찾은 매치 (오류면 `None`).
    fn matches(&self) -> Option<&Arc<Vec<Range<usize>>>> {
        self.cache.as_ref().and_then(|c| c.result.as_ref().ok())
    }

    /// 에디터에 그릴 매치 (찾기 바가 열려 있을 때만).
    pub fn marks(&self) -> Option<SearchMarks> {
        let cache = self.cache.as_ref().filter(|_| self.open)?;
        let ranges = cache.result.as_ref().ok()?;
        Some(SearchMarks {
            ranges: Arc::clone(ranges),
            text_len: cache.text_len,
        })
    }

    /// 커서(선택 시작) 위치 이후 첫 매치 — 검색어를 입력하는 동안 이동할 곳.
    pub fn first_from_cursor(&self, doc: &Document) -> Option<Range<usize>> {
        let matches = self.matches()?;
        let from = doc.cursor_char_range().start;
        next_match_index(matches, from, self.options.wrap_around).map(|i| matches[i].clone())
    }

    /// 다음(`forward`) 또는 이전 매치를 찾는다. 기준은 문서의 선택 범위다.
    ///
    /// 선택이 현재 매치면 그 다음/이전 매치로, 아니면 커서 이후 첫 매치/커서 앞 마지막 매치로 간다.
    pub fn step(&self, doc: &Document, forward: bool) -> Step {
        let Some(matches) = self.matches().filter(|m| !m.is_empty()) else {
            return Step::NoMatch;
        };
        let selection = doc.cursor_char_range();
        let wrap = self.options.wrap_around;
        let found = if forward {
            let on_match = current_match_index(matches, &selection).is_some();
            let from = selection.start + usize::from(on_match);
            next_match_index(matches, from, wrap).map(|i| (i, matches[i].start < from))
        } else {
            prev_match_index(matches, selection.start, wrap)
                .map(|i| (i, matches[i].start >= selection.start))
        };
        match found {
            Some((i, false)) => Step::Moved(matches[i].clone()),
            Some((i, true)) => Step::Wrapped(matches[i].clone()),
            None => Step::End,
        }
    }

    /// 찾기 바를 그린다. `selection`은 활성 문서의 선택(문자 범위), `viewer`는 읽기 전용 뷰어 탭 여부.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        palette: &Palette,
        selection: &Range<usize>,
        viewer: bool,
    ) -> Option<BarEvent> {
        let ctx = ui.ctx().clone();
        let id = Self::input_id();
        let mut event = None;

        // 입력란의 `Enter`·`Esc`는 위젯보다 먼저 소비한다 (한 줄 입력란은 `Enter`에 포커스를 놓는다).
        // `consume_key`는 `Shift`를 따로 보지 않으므로 `Shift+Enter`를 먼저 확인한다.
        if ctx.memory(|m| m.has_focus(id)) {
            ctx.input_mut(|i| {
                if i.consume_key(Modifiers::SHIFT, Key::Enter) {
                    event = Some(BarEvent::Prev);
                } else if i.consume_key(Modifiers::NONE, Key::Enter) {
                    event = Some(BarEvent::Next);
                } else if i.consume_key(Modifiers::NONE, Key::Escape) {
                    event = Some(BarEvent::Close);
                }
            });
        }
        if std::mem::take(&mut self.focus_input) {
            let mut state = TextEditState::load(&ctx, id).unwrap_or_default();
            let len = self.pattern.chars().count();
            state
                .cursor
                .set_char_range(Some(CCursorRange::two(CCursor::new(0), CCursor::new(len))));
            state.store(&ctx, id);
            ctx.memory_mut(|m| m.request_focus(id));
        }

        let error = self
            .cache
            .as_ref()
            .filter(|c| c.pattern == self.pattern)
            .and_then(|c| c.result.as_ref().err())
            .map(SearchError::to_string);
        ui.horizontal(|ui| {
            let mut edit = egui::TextEdit::singleline(&mut self.pattern)
                .id(id)
                .hint_text("찾기")
                .desired_width(INPUT_WIDTH);
            if error.is_some() {
                edit = edit.text_color(palette.status_error);
            }
            let mut input = ui.add(edit);
            if let Some(error) = &error {
                ui.painter().rect_stroke(
                    input.rect,
                    2.0,
                    Stroke::new(1.5, palette.status_error),
                    StrokeKind::Inside,
                );
                input = input.on_hover_text(error);
            }
            input.on_hover_text("Enter 다음 · Shift+Enter 이전 · Esc 닫기");

            let toggles = [
                (
                    &mut self.options.regex,
                    ".*",
                    "정규식 사용 (Rust regex 문법 — 전후방 탐색·역참조는 지원하지 않음)",
                ),
                (&mut self.options.match_case, "Aa", "대소문자 구분"),
                (&mut self.options.whole_word, "\\b", "단어 단위 일치"),
            ];
            for (on, label, tip) in toggles {
                if ui
                    .add(egui::Button::selectable(
                        *on,
                        RichText::new(label).monospace(),
                    ))
                    .on_hover_text(tip)
                    .clicked()
                {
                    *on = !*on;
                }
            }

            ui.add_space(8.0);
            let count = self.count_label(selection, viewer);
            let no_result = self.has_pattern() && self.matches().is_some_and(|m| m.is_empty());
            ui.label(if no_result || viewer {
                RichText::new(count).color(palette.status_warning)
            } else {
                RichText::new(count)
            });

            let can_move = self.matches().is_some_and(|m| !m.is_empty());
            let icon = |g: &str| RichText::new(g).family(FontFamily::Name(ICON_FAMILY.into()));
            if ui
                .add_enabled(
                    can_move,
                    egui::Button::new(icon(glyph::ARROW_UP)).frame(false),
                )
                .on_hover_text("이전 찾기 (Shift+Enter)")
                .clicked()
            {
                event = Some(BarEvent::Prev);
            }
            if ui
                .add_enabled(
                    can_move,
                    egui::Button::new(icon(glyph::ARROW_DOWN)).frame(false),
                )
                .on_hover_text("다음 찾기 (Enter)")
                .clicked()
            {
                event = Some(BarEvent::Next);
            }

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .add(egui::Button::new(icon(glyph::X)).frame(false))
                    .on_hover_text("닫기 (Esc)")
                    .clicked()
                {
                    event = Some(BarEvent::Close);
                }
            });
        });
        event
    }

    /// 매치 카운트 문구: `3 / 28`, 선택이 매치가 아니면 `? / 28`, 매치가 없으면 `결과 없음`.
    fn count_label(&self, selection: &Range<usize>, viewer: bool) -> String {
        if viewer {
            return "큰 파일 뷰어에서는 아직 찾을 수 없습니다".to_owned();
        }
        if !self.has_pattern() {
            return String::new();
        }
        match self.matches() {
            None => "정규식 오류".to_owned(),
            Some(m) if m.is_empty() => "결과 없음".to_owned(),
            Some(m) => match current_match_index(m, selection) {
                Some(i) => format!("{} / {}", i + 1, m.len()),
                None => format!("? / {}", m.len()),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::encoding::TextEncoding;
    use crate::core::line_ending::LineEnding;

    fn doc(text: &str) -> Document {
        let mut doc = Document::new_untitled_with_defaults(1, TextEncoding::Utf8, LineEnding::Lf);
        doc.set_text(text);
        doc
    }

    fn bar(pattern: &str) -> SearchBar {
        let mut bar = SearchBar::new(SearchOptions::default());
        bar.open = true;
        bar.pattern = pattern.to_owned();
        bar
    }

    #[test]
    fn open_prefills_single_line_selection() {
        let mut d = doc("한글 검색 단어\n둘째 줄");
        d.set_cursor_char_range(3..5);
        let mut b = SearchBar::new(SearchOptions::default());
        b.open(Some(&d));
        assert!(b.is_open());
        assert_eq!(b.pattern, "검색");

        // 여러 줄 선택·빈 선택은 이전 검색어를 그대로 둔다
        d.set_cursor_char_range(3..10);
        b.open(Some(&d));
        assert_eq!(b.pattern, "검색");
        d.set_cursor_char_range(0..0);
        b.open(Some(&d));
        assert_eq!(b.pattern, "검색");
        b.close();
        assert!(!b.is_open());
        assert!(b.marks().is_none());
    }

    #[test]
    fn refresh_only_reports_query_changes() {
        let mut d = doc("ab ab ab");
        let mut b = bar("ab");
        assert!(b.refresh(1, &d, true), "처음 찾기");
        assert!(!b.refresh(1, &d, true), "조건 그대로");
        assert_eq!(b.marks().unwrap().ranges.len(), 3);

        // 문서 편집·탭 전환으로 다시 찾을 때는 이동하지 않는다
        d.replace_range(0..0, "ab ");
        assert!(!b.refresh(1, &d, true));
        assert_eq!(b.marks().unwrap().ranges.len(), 4);
        assert!(!b.refresh(2, &d, true));

        // 검색어·옵션이 바뀌면 이동
        b.pattern = "AB".to_owned();
        assert!(b.refresh(2, &d, true));
        b.options.match_case = true;
        assert!(b.refresh(2, &d, true));
        assert!(b.marks().unwrap().ranges.is_empty());

        // 빈 검색어는 이동할 곳이 없다
        b.pattern.clear();
        assert!(!b.refresh(2, &d, true));
    }

    #[test]
    fn marks_are_char_ranges() {
        let d = doc("가나 검색 다라 검색");
        let mut b = bar("검색");
        b.refresh(1, &d, true);
        assert_eq!(*b.marks().unwrap().ranges, [3..5, 9..11]);
        assert_eq!(b.marks().unwrap().text_len, d.text().len());
    }

    #[test]
    fn step_moves_from_selection_and_wraps() {
        let mut d = doc("ab ab ab");
        let mut b = bar("ab");
        b.refresh(1, &d, true);

        // 커서 위치 이후 첫 매치
        d.set_cursor_char_range(1..1);
        assert_eq!(b.first_from_cursor(&d), Some(3..5));
        assert_eq!(b.step(&d, true), Step::Moved(3..5));
        assert_eq!(b.step(&d, false), Step::Moved(0..2));

        // 선택이 현재 매치면 다음·이전 매치로
        d.set_cursor_char_range(3..5);
        assert_eq!(b.count_label(&(3..5), false), "2 / 3");
        assert_eq!(b.step(&d, true), Step::Moved(6..8));
        assert_eq!(b.step(&d, false), Step::Moved(0..2));

        // 끝에서 처음으로, 처음에서 끝으로
        d.set_cursor_char_range(6..8);
        assert_eq!(b.step(&d, true), Step::Wrapped(0..2));
        d.set_cursor_char_range(0..2);
        assert_eq!(b.step(&d, false), Step::Wrapped(6..8));

        // Wrap Around 끔
        b.toggle_wrap_around();
        b.refresh(1, &d, true);
        assert_eq!(b.step(&d, false), Step::End);
        d.set_cursor_char_range(6..8);
        assert_eq!(b.step(&d, true), Step::End);
    }

    #[test]
    fn count_label_states() {
        let d = doc("ab ab");
        let mut b = bar("");
        b.refresh(1, &d, true);
        assert_eq!(b.count_label(&(0..0), false), "");

        b.pattern = "ab".to_owned();
        b.refresh(1, &d, true);
        assert_eq!(b.count_label(&(0..2), false), "1 / 2");
        assert_eq!(b.count_label(&(1..1), false), "? / 2");

        b.pattern = "zz".to_owned();
        b.refresh(1, &d, true);
        assert_eq!(b.count_label(&(0..0), false), "결과 없음");
        assert_eq!(b.step(&d, true), Step::NoMatch);

        b.pattern = "(".to_owned();
        b.options.regex = true;
        b.refresh(1, &d, true);
        assert_eq!(b.count_label(&(0..0), false), "정규식 오류");
        assert!(b.marks().is_none());
        assert_eq!(b.step(&d, true), Step::NoMatch);
    }

    #[test]
    fn viewer_tabs_are_not_searched() {
        let d = doc("ab ab");
        let mut b = bar("ab");
        b.refresh(1, &d, false);
        assert!(b.marks().unwrap().ranges.is_empty());
        assert!(b.count_label(&(0..0), true).contains("뷰어"));
    }

    #[test]
    fn icon_glyphs_exist_in_bundled_font() {
        let face = ttf_parser::Face::parse(
            include_bytes!("../../assets/file-icons/phosphor/Phosphor.ttf"),
            0,
        )
        .unwrap();
        for g in [glyph::ARROW_UP, glyph::ARROW_DOWN, glyph::X] {
            let c = g.chars().next().unwrap();
            assert!(face.glyph_index(c).is_some(), "U+{:04X}", c as u32);
        }
    }
}
