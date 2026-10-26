//! 찾기/바꾸기 바 UI (PLAN.md §3.2-2, ui-spec §1 — P4-04, P4-03, P4-05).
//!
//! - 탭 바 아래·사이드바 오른쪽, 에디터 바로 위에 펼쳐진다 (VS Code 찾기 위젯 배치):
//!   `[▸] [검색어 입력란] [.*] [Aa] [\b]  3 / 28  [↑] [↓] … [×]`
//!   바꾸기 줄(`Ctrl+H` 또는 `▸`): `    [바꿀 내용 입력란] [바꾸기] [모두 바꾸기]  N개 바꿈`
//! - 바꾸기 입력란 키: `Enter` 바꾸기 · `Ctrl+Alt+Enter`(macOS `Cmd+Option+Enter`) 모두 바꾸기 · `Esc` 닫기.
//!   바꾸기는 편집할 수 있는 탭에서만 (읽기 전용 문서·뷰어 탭은 버튼이 꺼진다).
//! - 탭·문서 `revision`·탭 종류(편집/뷰어)·검색어·옵션이 바뀔 때만 다시 찾고, 매치는 캐시해 강조([`SearchMarks`])와
//!   이동에 같이 쓴다. 단위는 편집 탭이면 문자 인덱스, 읽기 전용 뷰어 탭이면 바이트다.
//! - [`SYNC_MAX_BYTES`] 이하 문서는 UI 스레드에서 바로 찾는다 (입력하는 즉시 강조 — UX-6 확인).
//!   더 큰 문서는 입력이 [`DEBOUNCE`] 동안 멈추면 작업 스레드([`SearchJob`])에서 찾는다 (P4-03). 조건이 바뀌면
//!   진행 중인 작업을 버려 취소하고, 도착한 결과는 지금 조건과 같을 때만 받는다 — 빠르게 바꿔 입력해도 이전
//!   검색어의 결과가 표시되지 않는다. 찾는 동안 카운트는 `찾는 중…`, 강조는 검색어가 같을 때만 이전 결과를 쓴다.
//! - 현재 매치는 선택 범위와 똑같은 매치다 (번호를 따로 들고 있지 않는다). 커서를 옮기면 `? / 28`.
//! - 검색어나 옵션이 바뀌면 결과가 나온 뒤 커서(선택 시작) 이후 첫 매치를 선택해 보여 준다.
//! - 입력란 키: `Enter` 다음 · `Shift+Enter` 이전 · `Esc` 닫기. `F3`/`Shift+F3`(macOS `Cmd+G`/`Cmd+Shift+G`)는 전역 단축키.

use std::ops::Range;
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui::{
    self, Align, FontFamily, Id, Key, Layout, Modifiers, RichText, Stroke, StrokeKind,
    text::{CCursor, CCursorRange},
    text_edit::TextEditState,
};

use crate::core::document::Document;
use crate::core::search::{
    JobPoll, RangeUnit, SearchError, SearchJob, SearchOptions, SearchQuery,
    byte_ranges_to_char_ranges, current_match_index, next_match_index, prev_match_index,
};
use crate::fonts::ICON_FAMILY;
use crate::ui::editor::SearchMarks;
use crate::ui::theme::Palette;

/// 검색어 입력란 폭 (논리 픽셀).
const INPUT_WIDTH: f32 = 260.0;

/// 이 크기(바이트) 이하 문서는 UI 스레드에서 바로 찾는다. 1MB 매치 카운트는 수 ms (P4-01 측정 10MB 최대 41.5ms).
pub const SYNC_MAX_BYTES: usize = 1 << 20;

/// 큰 문서에서 조건이 바뀐 뒤 검색을 시작하기까지 기다리는 시간 (PLAN.md §3.2-2).
pub const DEBOUNCE: Duration = Duration::from_millis(150);

mod glyph {
    pub const ARROW_UP: &str = "\u{E08E}";
    pub const ARROW_DOWN: &str = "\u{E03E}";
    pub const X: &str = "\u{E4F6}";
    pub const CARET_RIGHT: &str = "\u{E13A}";
    pub const CARET_DOWN: &str = "\u{E136}";
}

/// 찾기 바에서 사용자가 고른 동작.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BarEvent {
    Next,
    Prev,
    Close,
    /// 현재 매치를 바꾸고 다음 매치로 (선택이 매치가 아니면 다음 매치로만)
    Replace,
    /// 모든 매치를 바꾼다 (Undo 한 단계)
    ReplaceAll,
}

/// 다음/이전 찾기 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// 이 매치로 옮긴다 (탭 단위: 편집 탭은 문자, 뷰어 탭은 바이트 범위).
    Moved(Range<usize>),
    /// 문서 끝(이전 찾기는 처음)을 지나 반대쪽 끝에서 이어 찾았다.
    Wrapped(Range<usize>),
    /// Wrap Around를 꺼 둔 상태에서 더 찾을 매치가 없다.
    End,
    /// 매치가 없다 (빈 검색어·오류 포함).
    NoMatch,
}

/// 검색 조건: 이 값이 같으면 같은 결과다.
#[derive(Debug, Clone, PartialEq, Eq)]
struct QueryKey {
    tab_id: u64,
    revision: u64,
    /// 읽기 전용 뷰어 탭 (결과 단위가 바이트)
    viewer: bool,
    pattern: String,
    options: SearchOptions,
}

impl QueryKey {
    fn same_query(&self, other: &Self) -> bool {
        self.pattern == other.pattern && self.options == other.options
    }

    fn unit(&self) -> RangeUnit {
        if self.viewer {
            RangeUnit::Bytes
        } else {
            RangeUnit::Chars
        }
    }
}

/// 매치 캐시: 이 조건으로 찾은 결과.
#[derive(Debug)]
struct MatchCache {
    key: QueryKey,
    /// 매치 범위 (오름차순), 또는 검색어 오류
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
    /// 바꾸기 줄이 열려 있는지
    replace_open: bool,
    /// 바꿀 내용
    replacement: String,
    /// 다음 프레임에 바꾸기 입력란으로 포커스를 옮기고 내용을 모두 선택한다.
    focus_replace: bool,
    /// 마지막 모두 바꾸기 결과 문구 (`N개 바꿈`) — 검색어·바꿀 내용이 바뀌면 지운다
    replace_note: Option<String>,
    /// 지난 [`Self::refresh`]에서 원한 조건
    wanted: Option<QueryKey>,
    cache: Option<MatchCache>,
    /// 작업 스레드에서 찾는 중인 조건과 작업 (버리면 취소)
    job: Option<(QueryKey, SearchJob)>,
    /// 큰 문서: 아직 검색을 시작하지 않은 조건과 그 조건을 처음 본 시각 (디바운스)
    dirty: Option<(QueryKey, Instant)>,
    /// 검색어·옵션이 바뀌어, 결과가 나오면 첫 매치로 이동해야 한다.
    reveal_pending: bool,
    /// 미리보기 단독 모드면 미리보기 찾기 상태 `Some(아직 안 셌으면 None | (현재 번호, 매치 수))` (P4-06)
    preview: Option<Option<(Option<usize>, usize)>>,
}

impl SearchBar {
    pub fn new(options: SearchOptions) -> Self {
        Self {
            open: false,
            pattern: String::new(),
            options,
            focus_input: false,
            replace_open: false,
            replacement: String::new(),
            focus_replace: false,
            replace_note: None,
            wanted: None,
            cache: None,
            job: None,
            dirty: None,
            reveal_pending: false,
            preview: None,
        }
    }

    /// 지금 검색어 (미리보기 찾기 조건용).
    pub fn pattern(&self) -> &str {
        &self.pattern
    }

    /// 미리보기 단독 모드에서는 카운트에 미리보기 찾기 상태를 쓴다 (`preview`가 `false`면 소스 매치).
    /// `status`는 `(현재 매치 번호, 매치 수)`, 아직 세지 않았으면 `None`.
    pub fn set_preview_status(&mut self, preview: bool, status: Option<(Option<usize>, usize)>) {
        self.preview = preview.then_some(status);
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

    /// 바꾸기 입력란 위젯 ID.
    fn replace_id() -> Id {
        Id::unique("search_bar_replace")
    }

    /// 찾기·바꾸기 입력란 중 하나가 키보드 포커스를 가졌는지.
    pub fn input_has_focus(&self, ctx: &egui::Context) -> bool {
        self.open
            && ctx.memory(|m| m.has_focus(Self::input_id()) || m.has_focus(Self::replace_id()))
    }

    /// 찾기 바를 열고 입력란에 포커스를 준다 (`Ctrl+F` — 바꾸기 줄은 접는다, `Ctrl+H` — 바꾸기 줄도 연다).
    ///
    /// `selected`(활성 탭의 선택 텍스트)가 한 줄 안이고 비어 있지 않으면 검색어로 채운다. 입력란 내용은 모두
    /// 선택해 바로 새로 칠 수 있게 한다. 바꾸기로 열 때 검색어가 있으면 바꾸기 입력란에 포커스를 준다.
    pub fn open(&mut self, selected: Option<&str>, replace: bool) {
        if let Some(selected) = selected
            && !selected.is_empty()
            && !selected.contains('\n')
        {
            self.pattern = selected.to_owned();
        }
        self.open = true;
        self.replace_open = replace;
        if replace && self.has_pattern() {
            self.focus_replace = true;
        } else {
            self.focus_input = true;
        }
    }

    pub fn is_replace_open(&self) -> bool {
        self.open && self.replace_open
    }

    /// 바꿀 내용 (입력한 그대로 — 해석은 [`SearchQuery`]가 한다).
    pub fn replacement(&self) -> &str {
        &self.replacement
    }

    /// 지금 검색어·옵션의 검색 조건 (빈 검색어·오류면 `None`).
    pub fn query(&self) -> Option<SearchQuery> {
        SearchQuery::new(&self.pattern, &self.options)
            .ok()
            .filter(|q| !q.is_empty())
    }

    /// 모두 바꾸기 결과를 바꾸기 줄에 보여 준다.
    pub fn set_replace_note(&mut self, note: impl Into<String>) {
        self.replace_note = Some(note.into());
    }

    /// 찾기 바를 닫는다 (강조도 사라진다).
    pub fn close(&mut self) {
        self.open = false;
        self.focus_input = false;
    }

    pub fn toggle_wrap_around(&mut self) {
        self.options.wrap_around = !self.options.wrap_around;
    }

    /// 활성 탭 문서에 맞춰 매치를 찾는다 — 매 프레임 불러도 된다 (조건이 그대로면 아무것도 하지 않음).
    ///
    /// 검색어·옵션이 바뀐 뒤 그 결과가 이번에 나왔으면 `true` — 호출하는 쪽이 첫 매치로 이동한다.
    /// 탭 전환이나 문서 편집으로만 다시 찾은 경우는 `false` (커서를 빼앗지 않는다).
    pub fn refresh(
        &mut self,
        ctx: &egui::Context,
        tab_id: u64,
        doc: &Document,
        viewer: bool,
    ) -> bool {
        let key = QueryKey {
            tab_id,
            revision: doc.revision(),
            viewer,
            pattern: self.pattern.clone(),
            options: self.options,
        };
        if self.wanted.as_ref().is_none_or(|w| !w.same_query(&key)) {
            self.reveal_pending = !key.pattern.is_empty();
            self.replace_note = None;
        }
        self.wanted = Some(key.clone());
        if self.cache.as_ref().is_some_and(|c| c.key == key) {
            self.job = None;
            self.dirty = None;
            return std::mem::take(&mut self.reveal_pending);
        }

        let text = doc.text();
        let query = match SearchQuery::new(&key.pattern, &key.options) {
            Ok(query) => query,
            Err(err) => {
                self.install(ctx, key, Err(err), text.len());
                return std::mem::take(&mut self.reveal_pending);
            }
        };
        if query.is_empty() || text.len() <= SYNC_MAX_BYTES {
            let bytes = query.find_all(text);
            let ranges = match key.unit() {
                RangeUnit::Bytes => bytes,
                RangeUnit::Chars => byte_ranges_to_char_ranges(text, &bytes),
            };
            self.install(ctx, key, Ok(ranges), text.len());
            return std::mem::take(&mut self.reveal_pending);
        }

        // 큰 문서: 같은 조건의 작업이 있으면 결과를 확인한다 (다른 조건의 작업은 버려 취소).
        if let Some((job_key, job)) = &self.job
            && *job_key == key
        {
            match job.poll() {
                JobPoll::Running => return false,
                JobPoll::Done(ranges) => {
                    self.job = None;
                    self.install(ctx, key, Ok(ranges), text.len());
                    return std::mem::take(&mut self.reveal_pending);
                }
                // 스레드가 결과 없이 끝났다: 아래에서 다시 시작한다.
                JobPoll::Lost => self.job = None,
            }
        }
        self.job = None;

        let since = match &self.dirty {
            Some((dirty_key, since)) if *dirty_key == key => *since,
            _ => {
                let now = Instant::now();
                self.dirty = Some((key.clone(), now));
                now
            }
        };
        let waited = since.elapsed();
        if waited < DEBOUNCE {
            ctx.request_repaint_after(DEBOUNCE - waited);
            return false;
        }
        self.dirty = None;
        let notify = ctx.clone();
        let unit = key.unit();
        let job = SearchJob::spawn(query, text.to_owned(), unit, move || {
            notify.request_repaint()
        });
        self.job = Some((key, job));
        false
    }

    fn install(
        &mut self,
        ctx: &egui::Context,
        key: QueryKey,
        result: Result<Vec<Range<usize>>, SearchError>,
        text_len: usize,
    ) {
        self.cache = Some(MatchCache {
            key,
            result: result.map(Arc::new),
            text_len,
        });
        ctx.request_repaint();
    }

    /// 지난 [`Self::refresh`]에서 원한 조건의 결과가 나와 있는지 (찾는 중이 아님).
    pub fn is_ready(&self) -> bool {
        match (&self.wanted, &self.cache) {
            (Some(wanted), Some(cache)) => cache.key == *wanted,
            _ => false,
        }
    }

    /// 지금 조건의 매치 (찾는 중·오류면 `None`).
    fn matches(&self) -> Option<&Arc<Vec<Range<usize>>>> {
        self.cache
            .as_ref()
            .filter(|_| self.is_ready())
            .and_then(|c| c.result.as_ref().ok())
    }

    /// 지금 조건의 검색어 오류.
    fn error(&self) -> Option<&SearchError> {
        let (wanted, cache) = (self.wanted.as_ref()?, self.cache.as_ref()?);
        cache
            .result
            .as_ref()
            .err()
            .filter(|_| cache.key.same_query(wanted))
    }

    /// 강조할 매치 (찾기 바가 열려 있을 때만). 같은 탭·같은 검색어면 문서 편집 뒤 다시 찾는 동안에도
    /// 이전 결과를 쓴다 (그리는 쪽이 텍스트 길이로 다시 확인). 검색어·옵션이 바뀌어 찾는 중이면 없다.
    pub fn marks(&self) -> Option<SearchMarks> {
        let wanted = self.wanted.as_ref().filter(|_| self.open)?;
        let cache = self.cache.as_ref()?;
        let same = cache.key.same_query(wanted)
            && cache.key.tab_id == wanted.tab_id
            && cache.key.viewer == wanted.viewer;
        let ranges = cache.result.as_ref().ok().filter(|_| same)?;
        Some(SearchMarks {
            ranges: Arc::clone(ranges),
            text_len: cache.text_len,
        })
    }

    /// 선택 시작 이후 첫 매치 — 검색어를 입력하는 동안 이동할 곳.
    pub fn first_from(&self, selection: &Range<usize>) -> Option<Range<usize>> {
        let matches = self.matches()?;
        next_match_index(matches, selection.start, self.options.wrap_around)
            .map(|i| matches[i].clone())
    }

    /// 다음(`forward`) 또는 이전 매치를 찾는다. 기준은 활성 탭의 선택 범위(탭 단위)다.
    ///
    /// 선택이 현재 매치면 그 다음/이전 매치로, 아니면 커서 이후 첫 매치/커서 앞 마지막 매치로 간다.
    pub fn step(&self, selection: &Range<usize>, forward: bool) -> Step {
        let Some(matches) = self.matches().filter(|m| !m.is_empty()) else {
            return Step::NoMatch;
        };
        let wrap = self.options.wrap_around;
        let found = if forward {
            let on_match = current_match_index(matches, selection).is_some();
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

    /// 찾기 바를 그린다. `selection`은 활성 탭의 선택 범위(탭 단위), `can_replace`는 활성 탭을 편집할 수 있는지.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        palette: &Palette,
        selection: &Range<usize>,
        can_replace: bool,
    ) -> Option<BarEvent> {
        let ctx = ui.ctx().clone();
        let (id, replace_id) = (Self::input_id(), Self::replace_id());
        let mut event = None;

        // 입력란의 `Enter`·`Esc`는 위젯보다 먼저 소비한다 (한 줄 입력란은 `Enter`에 포커스를 놓는다).
        // `consume_key`는 `Shift`·`Alt`를 따로 보지 않으므로 수식키가 붙은 조합을 먼저 확인한다.
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
        } else if self.replace_open && ctx.memory(|m| m.has_focus(replace_id)) {
            ctx.input_mut(|i| {
                if i.consume_key(Modifiers::COMMAND | Modifiers::ALT, Key::Enter) {
                    event = can_replace.then_some(BarEvent::ReplaceAll);
                } else if i.consume_key(Modifiers::NONE, Key::Enter) {
                    event = can_replace.then_some(BarEvent::Replace);
                } else if i.consume_key(Modifiers::NONE, Key::Escape) {
                    event = Some(BarEvent::Close);
                }
            });
        }
        if std::mem::take(&mut self.focus_input) {
            focus_select_all(&ctx, id, &self.pattern);
        }
        if std::mem::take(&mut self.focus_replace) {
            focus_select_all(&ctx, replace_id, &self.replacement);
        }

        let error = self
            .error()
            .filter(|_| {
                self.wanted
                    .as_ref()
                    .is_some_and(|w| w.pattern == self.pattern)
            })
            .map(SearchError::to_string);
        let count = self.count_label(selection);
        let no_result = count == "결과 없음";
        let can_move = self.has_pattern() && self.error().is_none();
        let icon = |g: &str| RichText::new(g).family(FontFamily::Name(ICON_FAMILY.into()));
        let mut toggle_width = 0.0;
        ui.horizontal(|ui| {
            // 바꾸기 줄 펼치기/접기 (VS Code 찾기 위젯의 `▸`)
            let toggle = ui
                .add(
                    egui::Button::new(icon(if self.replace_open {
                        glyph::CARET_DOWN
                    } else {
                        glyph::CARET_RIGHT
                    }))
                    .frame(false),
                )
                .on_hover_text(if self.replace_open {
                    "바꾸기 닫기"
                } else {
                    "바꾸기 열기 (Ctrl+H)"
                });
            toggle_width = toggle.rect.width() + ui.spacing().item_spacing.x;
            if toggle.clicked() {
                self.replace_open = !self.replace_open;
                if self.replace_open {
                    self.focus_replace = true;
                }
            }

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
            ui.label(if no_result {
                RichText::new(count).color(palette.status_warning)
            } else {
                RichText::new(count)
            });

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

        if self.replace_open {
            ui.horizontal(|ui| {
                ui.add_space(toggle_width);
                let tip = if self.options.regex {
                    "정규식: $1·${name} 캡처 그룹, $$는 $ 글자, \\n 줄바꿈, \\t 탭\n\
                     그룹 뒤에 글자를 붙일 때는 ${1}a처럼 씁니다\n\
                     Enter 바꾸기 · Ctrl+Alt+Enter 모두 바꾸기 · Esc 닫기"
                } else {
                    "글자 그대로 바꿉니다 ($1·\\n은 정규식 모드에서만 해석)\n\
                     Enter 바꾸기 · Ctrl+Alt+Enter 모두 바꾸기 · Esc 닫기"
                };
                let response = ui
                    .add(
                        egui::TextEdit::singleline(&mut self.replacement)
                            .id(replace_id)
                            .hint_text("바꿀 내용")
                            .desired_width(INPUT_WIDTH),
                    )
                    .on_hover_text(tip);
                if response.changed() {
                    self.replace_note = None;
                }
                let enabled = can_replace && can_move;
                let disabled_tip = if can_replace {
                    "찾을 내용을 입력하세요"
                } else {
                    "읽기 전용 문서는 바꿀 수 없습니다"
                };
                if ui
                    .add_enabled(enabled, egui::Button::new("바꾸기"))
                    .on_hover_text("현재 매치를 바꾸고 다음 매치로 (Enter)")
                    .on_disabled_hover_text(disabled_tip)
                    .clicked()
                {
                    event = Some(BarEvent::Replace);
                }
                if ui
                    .add_enabled(enabled, egui::Button::new("모두 바꾸기"))
                    .on_hover_text(
                        "모든 매치를 바꿉니다 — Ctrl+Z 한 번으로 되돌림 (Ctrl+Alt+Enter)",
                    )
                    .on_disabled_hover_text(disabled_tip)
                    .clicked()
                {
                    event = Some(BarEvent::ReplaceAll);
                }
                if let Some(note) = &self.replace_note {
                    ui.add_space(8.0);
                    ui.label(note);
                }
            });
        }
        event
    }

    /// 매치 카운트 문구: `3 / 28`, 선택이 매치가 아니면 `? / 28`, 매치가 없으면 `결과 없음`, 찾는 중이면 `찾는 중…`.
    fn count_label(&self, selection: &Range<usize>) -> String {
        if !self.has_pattern() {
            return String::new();
        }
        if self.error().is_some() {
            return "정규식 오류".to_owned();
        }
        // 미리보기 단독 모드: 미리보기 글자의 매치 수 (P4-06)
        if let Some(status) = self.preview {
            return match status {
                None => "찾는 중…".to_owned(),
                Some((_, 0)) => "결과 없음".to_owned(),
                Some((Some(i), total)) => format!("{} / {total}", i + 1),
                Some((None, total)) => format!("? / {total}"),
            };
        }
        match self.matches() {
            None => "찾는 중…".to_owned(),
            Some(m) if m.is_empty() => "결과 없음".to_owned(),
            Some(m) => match current_match_index(m, selection) {
                Some(i) => format!("{} / {}", i + 1, m.len()),
                None => format!("? / {}", m.len()),
            },
        }
    }
}

/// 입력란에 포커스를 주고 내용을 모두 선택한다 (바로 새로 칠 수 있게).
fn focus_select_all(ctx: &egui::Context, id: Id, text: &str) {
    let mut state = TextEditState::load(ctx, id).unwrap_or_default();
    let len = text.chars().count();
    state
        .cursor
        .set_char_range(Some(CCursorRange::two(CCursor::new(0), CCursor::new(len))));
    state.store(ctx, id);
    ctx.memory_mut(|m| m.request_focus(id));
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

    /// 결과가 나올 때까지 `refresh`를 되풀이한다 (큰 문서는 디바운스 + 작업 스레드). 이동 요청 여부를 돌려준다.
    fn settle(
        b: &mut SearchBar,
        ctx: &egui::Context,
        tab: u64,
        d: &Document,
        viewer: bool,
    ) -> bool {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut reveal = false;
        loop {
            reveal |= b.refresh(ctx, tab, d, viewer);
            if b.is_ready() {
                return reveal;
            }
            assert!(Instant::now() < deadline, "검색이 끝나지 않음");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn open_prefills_single_line_selection() {
        let mut b = SearchBar::new(SearchOptions::default());
        b.open(Some("검색"), false);
        assert!(b.is_open());
        assert_eq!(b.pattern, "검색");

        // 여러 줄 선택·빈 선택·선택 없음은 이전 검색어를 그대로 둔다
        b.open(Some("검색\n둘째"), false);
        b.open(Some(""), false);
        b.open(None, false);
        assert_eq!(b.pattern, "검색");
        b.close();
        assert!(!b.is_open());
        assert!(b.marks().is_none());
    }

    #[test]
    fn replace_opens_row_and_focuses_replace_input_when_pattern_exists() {
        let mut b = SearchBar::new(SearchOptions::default());
        // 검색어가 없으면 찾기 입력란부터
        b.open(None, true);
        assert!(b.is_replace_open());
        assert!(b.focus_input && !b.focus_replace);

        b.focus_input = false;
        b.open(Some("검색"), true);
        assert!(b.is_replace_open());
        assert!(
            b.focus_replace && !b.focus_input,
            "검색어가 있으면 바꾸기 입력란"
        );

        // `Ctrl+F`는 바꾸기 줄을 접는다
        b.open(None, false);
        assert!(!b.is_replace_open());
        assert!(b.focus_input);
        assert_eq!(b.query().map(|q| q.count("검색 검색")), Some(2));
        b.pattern.clear();
        assert!(b.query().is_none(), "빈 검색어는 바꿀 조건이 없다");
    }

    #[test]
    fn replace_note_clears_when_query_changes() {
        let ctx = egui::Context::default();
        let mut d = doc("ab ab");
        let mut b = bar("ab");
        b.refresh(&ctx, 1, &d, false);
        b.set_replace_note("2개 바꿈");
        // 바꾼 뒤 문서가 바뀌어 다시 찾아도 문구는 남는다
        d.set_text("x x");
        b.refresh(&ctx, 1, &d, false);
        assert_eq!(b.replace_note.as_deref(), Some("2개 바꿈"));
        b.pattern = "x".to_owned();
        b.refresh(&ctx, 1, &d, false);
        assert_eq!(b.replace_note, None);
    }

    #[test]
    fn refresh_only_reports_query_changes() {
        let ctx = egui::Context::default();
        let mut d = doc("ab ab ab");
        let mut b = bar("ab");
        assert!(b.refresh(&ctx, 1, &d, false), "처음 찾기");
        assert!(!b.refresh(&ctx, 1, &d, false), "조건 그대로");
        assert_eq!(b.marks().unwrap().ranges.len(), 3);

        // 문서 편집·탭 전환으로 다시 찾을 때는 이동하지 않는다
        d.replace_range(0..0, "ab ");
        assert!(!b.refresh(&ctx, 1, &d, false));
        assert_eq!(b.marks().unwrap().ranges.len(), 4);
        assert!(!b.refresh(&ctx, 2, &d, false));

        // 검색어·옵션이 바뀌면 이동
        b.pattern = "AB".to_owned();
        assert!(b.refresh(&ctx, 2, &d, false));
        b.options.match_case = true;
        assert!(b.refresh(&ctx, 2, &d, false));
        assert!(b.marks().unwrap().ranges.is_empty());

        // 빈 검색어는 이동할 곳이 없다
        b.pattern.clear();
        assert!(!b.refresh(&ctx, 2, &d, false));
    }

    #[test]
    fn marks_use_chars_in_editor_and_bytes_in_viewer() {
        let ctx = egui::Context::default();
        let d = doc("가나 검색 다라 검색");
        let mut b = bar("검색");
        b.refresh(&ctx, 1, &d, false);
        assert_eq!(*b.marks().unwrap().ranges, [3..5, 9..11]);
        assert_eq!(b.marks().unwrap().text_len, d.text().len());
        b.refresh(&ctx, 1, &d, true);
        assert_eq!(*b.marks().unwrap().ranges, [7..13, 21..27]);
    }

    #[test]
    fn step_moves_from_selection_and_wraps() {
        let ctx = egui::Context::default();
        let d = doc("ab ab ab");
        let mut b = bar("ab");
        b.refresh(&ctx, 1, &d, false);

        // 커서 위치 이후 첫 매치
        assert_eq!(b.first_from(&(1..1)), Some(3..5));
        assert_eq!(b.step(&(1..1), true), Step::Moved(3..5));
        assert_eq!(b.step(&(1..1), false), Step::Moved(0..2));

        // 선택이 현재 매치면 다음·이전 매치로
        assert_eq!(b.count_label(&(3..5)), "2 / 3");
        assert_eq!(b.step(&(3..5), true), Step::Moved(6..8));
        assert_eq!(b.step(&(3..5), false), Step::Moved(0..2));

        // 끝에서 처음으로, 처음에서 끝으로
        assert_eq!(b.step(&(6..8), true), Step::Wrapped(0..2));
        assert_eq!(b.step(&(0..2), false), Step::Wrapped(6..8));

        // Wrap Around 끔
        b.toggle_wrap_around();
        b.refresh(&ctx, 1, &d, false);
        assert_eq!(b.step(&(0..2), false), Step::End);
        assert_eq!(b.step(&(6..8), true), Step::End);
    }

    #[test]
    fn count_label_states() {
        let ctx = egui::Context::default();
        let d = doc("ab ab");
        let mut b = bar("");
        b.refresh(&ctx, 1, &d, false);
        assert_eq!(b.count_label(&(0..0)), "");

        b.pattern = "ab".to_owned();
        b.refresh(&ctx, 1, &d, false);
        assert_eq!(b.count_label(&(0..2)), "1 / 2");
        assert_eq!(b.count_label(&(1..1)), "? / 2");

        b.pattern = "zz".to_owned();
        b.refresh(&ctx, 1, &d, false);
        assert_eq!(b.count_label(&(0..0)), "결과 없음");
        assert_eq!(b.step(&(0..0), true), Step::NoMatch);

        b.pattern = "(".to_owned();
        b.options.regex = true;
        b.refresh(&ctx, 1, &d, false);
        assert_eq!(b.count_label(&(0..0)), "정규식 오류");
        assert!(b.marks().is_none());
        assert_eq!(b.step(&(0..0), true), Step::NoMatch);
    }

    /// 1MB를 넘는 문서 (`SYNC_MAX_BYTES` 초과)
    fn large_doc() -> Document {
        let mut text = String::with_capacity(SYNC_MAX_BYTES + 4096);
        let mut i = 0;
        while text.len() <= SYNC_MAX_BYTES {
            text.push_str(&format!("{i}번째 줄 alpha beta\n"));
            i += 1;
        }
        doc(&text)
    }

    #[test]
    fn large_document_is_searched_in_background_after_debounce() {
        let ctx = egui::Context::default();
        let d = large_doc();
        let lines = d.line_count() - 1;
        let mut b = bar("alpha");
        let started = Instant::now();
        assert!(!b.refresh(&ctx, 1, &d, true), "디바운스 동안은 결과 없음");
        assert!(!b.is_ready());
        assert_eq!(b.count_label(&(0..0)), "찾는 중…");
        assert!(b.marks().is_none());
        assert_eq!(b.step(&(0..0), true), Step::NoMatch);

        assert!(
            settle(&mut b, &ctx, 1, &d, true),
            "결과가 나오면 첫 매치로 이동"
        );
        assert!(started.elapsed() >= DEBOUNCE);
        assert_eq!(b.marks().unwrap().ranges.len(), lines);
        assert_eq!(b.count_label(&(0..0)), format!("? / {lines}"));
        // 같은 조건이면 다시 찾지 않는다
        assert!(!b.refresh(&ctx, 1, &d, true));
        assert!(b.is_ready());
    }

    #[test]
    fn changing_query_quickly_discards_old_results() {
        let ctx = egui::Context::default();
        let d = large_doc();
        let mut b = bar("alpha");
        settle(&mut b, &ctx, 1, &d, true);
        let alpha = b.marks().unwrap().ranges.len();

        // 빠르게 바꿔 입력: 디바운스·작업 중에 검색어가 바뀌면 이전 조건의 작업은 버린다
        for pattern in ["b", "be", "bet", "zzz", "beta"] {
            b.pattern = pattern.to_owned();
            b.refresh(&ctx, 1, &d, true);
            assert!(
                b.marks().is_none(),
                "{pattern}: 이전 검색어 강조를 쓰지 않는다"
            );
            assert_eq!(b.count_label(&(0..0)), "찾는 중…");
            std::thread::sleep(Duration::from_millis(60));
        }
        assert!(settle(&mut b, &ctx, 1, &d, true));
        let beta = b.marks().unwrap();
        assert_eq!(beta.ranges.len(), alpha);
        let first = beta.ranges[0].clone();
        assert_eq!(&d.text()[first], "beta", "마지막 검색어의 결과");
    }

    #[test]
    fn editing_large_document_keeps_marks_until_new_results() {
        let ctx = egui::Context::default();
        let mut d = large_doc();
        let mut b = bar("alpha");
        settle(&mut b, &ctx, 1, &d, false);
        let before = b.marks().unwrap();

        d.replace_range(0..0, "alpha ");
        assert!(!b.refresh(&ctx, 1, &d, false));
        // 같은 검색어: 다시 찾는 동안 이전 강조를 쓴다 (그리는 쪽이 텍스트 길이가 달라 그리지 않음)
        let stale = b.marks().unwrap();
        assert_eq!(stale.text_len, before.text_len);
        assert_ne!(stale.text_len, d.text().len());
        assert!(
            !settle(&mut b, &ctx, 1, &d, false),
            "편집으로 다시 찾을 때는 이동하지 않음"
        );
        assert_eq!(b.marks().unwrap().ranges.len(), before.ranges.len() + 1);
        assert_eq!(b.marks().unwrap().text_len, d.text().len());
    }

    /// 10MB 문서를 뷰어로 보며 검색어를 빠르게 바꿔 입력할 때 프레임 시간 (찾기 갱신 + 뷰어 그리기, CPU).
    /// 실행: `cargo test --release --lib -- --ignored --nocapture search_frame_time_10mb`
    #[test]
    #[ignore = "측정용: 릴리스 빌드로 실행"]
    fn search_frame_time_10mb() {
        use crate::ui::viewer::ViewerState;
        use eframe::egui::{FontId, RawInput, Rect, pos2, vec2};

        let mut text = String::with_capacity(10 << 20);
        let mut i = 0u32;
        while text.len() < 10 << 20 {
            text.push_str(&format!(
                "2026-10-26 12:{:02}:{:02} [INFO] 요청 처리 완료 id={i} user=홍길동 error={}\n",
                i / 60 % 60,
                i % 60,
                if i.is_multiple_of(13) {
                    "timeout"
                } else {
                    "none"
                },
            ));
            i += 1;
        }
        let d = doc(&text);
        let ctx = egui::Context::default();
        let palette = Palette::dark();
        let mut viewer = ViewerState::new(1);
        let mut b = bar("");
        let frame = |b: &mut SearchBar, viewer: &mut ViewerState| -> f64 {
            let started = Instant::now();
            if b.refresh(&ctx, 1, &d, true)
                && let Some(range) = b.first_from(&viewer.selection())
            {
                viewer.select_and_reveal(&d, range);
            }
            let marks = b.marks();
            let raw = RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1200.0, 800.0))),
                ..Default::default()
            };
            let mut full = ctx.run_ui(raw, |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    viewer.show(ui, &d, &FontId::monospace(14.0), &palette, marks.as_ref());
                });
            });
            full.textures_delta.clear();
            started.elapsed().as_secs_f64() * 1000.0
        };
        frame(&mut b, &mut viewer);
        frame(&mut b, &mut viewer);

        // 60fps로 한 글자씩 입력 (프레임 사이 16ms), 마지막 입력 뒤 결과가 나올 때까지
        let typed = "error=timeout";
        let mut times = Vec::new();
        for n in 1..=typed.len() {
            b.pattern = typed[..n].to_owned();
            for _ in 0..3 {
                times.push(frame(&mut b, &mut viewer));
                std::thread::sleep(Duration::from_millis(16));
            }
        }
        let last_input = Instant::now();
        while !b.is_ready() {
            times.push(frame(&mut b, &mut viewer));
            std::thread::sleep(Duration::from_millis(16));
            assert!(last_input.elapsed() < Duration::from_secs(10));
        }
        let to_result = last_input.elapsed().as_secs_f64() * 1000.0;
        times.push(frame(&mut b, &mut viewer));
        let count = b.marks().unwrap().ranges.len();
        let avg = times.iter().sum::<f64>() / times.len() as f64;
        let max = times.iter().copied().fold(0.0, f64::max);
        println!(
            "10MB({} B) 입력 중 프레임 {}개: 평균 {avg:.2}ms · 최대 {max:.2}ms, 마지막 입력 → 결과 {to_result:.0}ms, 매치 {count}개",
            text.len(),
            times.len()
        );
        assert!(max <= 16.0, "검색 중 UI 프레임 ≤ 16ms — 최대 {max:.2}ms");
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
