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

use std::ops::Range;
use std::sync::Arc;

use eframe::egui::{
    self, Event, FontId, Galley, Id, Key, Pos2, Rect, UiBuilder, ViewportCommand,
    text::{CCursor, CCursorRange, LayoutJob},
    text_edit::TextEditState,
};

use crate::commands::Command;
use crate::core::buffer::char_to_byte_offset;
use crate::core::document::Document;
use crate::core::line_ending::normalize_to_lf;
use crate::ui::theme::Palette;

/// 에디터 표시 옵션.
#[derive(Debug, Clone)]
pub struct EditorOptions {
    /// 자동 줄바꿈 여부 (`false`면 가로 스크롤).
    pub word_wrap: bool,
    /// 본문 글꼴 (확대/축소 반영).
    pub font: FontId,
}

/// 한 프레임의 에디터 결과.
#[derive(Debug, Clone, Copy, Default)]
pub struct EditorOutput {
    /// 이번 프레임에 문서 내용이 바뀌었는지 여부.
    pub changed: bool,
    /// 에디터가 키보드 포커스를 가졌는지 여부.
    pub has_focus: bool,
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
    focus_requested: bool,
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
            focus_requested: false,
        }
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

        if self.synced_revision != Some(doc.revision()) {
            self.sync_from(doc);
        }

        let focused_before = ctx.memory(|m| m.has_focus(self.id));
        // 이번 프레임에 포커스를 받는 경우(메뉴 `붙여넣기` 직후 등)에도 위젯보다 먼저 입력을 고친다.
        if focused_before || self.focus_requested {
            ctx.input_mut(|i| filter_input_events(&mut i.events));
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
        let scratch = &mut self.scratch;
        // 기본 레이아웃과 같되, 자동 줄바꿈을 끄면 줄바꿈 폭을 무한대로 한다
        // (`desired_width(INFINITY)`만으로는 보이는 폭에서 접혀 가로 스크롤이 생기지 않는다).
        let wrap = options.word_wrap;
        let font = options.font.clone();
        let mut layouter = move |ui: &egui::Ui, buf: &dyn egui::TextBuffer, wrap_width: f32| {
            let mut job = LayoutJob::simple(
                buf.as_str().to_owned(),
                font.clone(),
                ui.visuals().text_color(),
                if wrap { wrap_width } else { f32::INFINITY },
            );
            job.keep_trailing_whitespace = true;
            ui.fonts_mut(|f| f.layout_job(job))
        };
        let scroll_output = ui
            .scope_builder(UiBuilder::new().max_rect(text_rect), |ui| {
                scroll.show(ui, |ui| {
                    let min_size = ui.available_size();
                    if readonly {
                        // 읽기 전용 문서도 선택·복사는 되도록 `&str`로 그린다.
                        let mut text = scratch.as_str();
                        text_edit(&mut text, id, options, min_size, &mut layouter).show(ui)
                    } else {
                        text_edit(scratch, id, options, min_size, &mut layouter).show(ui)
                    }
                })
            })
            .inner;
        doc.set_scroll_offset_y(scroll_output.state.clamped_offset().y);

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
        EditorOutput { changed, has_focus }
    }

    fn sync_from(&mut self, doc: &Document) {
        self.scratch.clear();
        self.scratch.push_str(doc.text());
        self.synced_revision = Some(doc.revision());
    }
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

/// 줄 번호 Gutter 폭. 최소 3자리이고 줄 수 자릿수(1,000·10,000줄 …)에 맞춰 늘어난다 (뷰어와 같은 폭).
pub fn gutter_width(line_count: usize, char_width: f32) -> f32 {
    let digits = line_count.max(1).to_string().len().max(3);
    digits as f32 * char_width + 20.0
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
    for (line, row_rect) in logical_line_rows(galley) {
        let row_rect = row_rect.translate(galley_pos.to_vec2());
        if row_rect.max.y < rect.min.y {
            continue;
        }
        if row_rect.min.y > rect.max.y {
            break;
        }
        painter.text(
            Pos2::new(rect.max.x - 10.0, row_rect.center().y),
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
    painter.vline(
        rect.max.x - 4.0,
        rect.y_range(),
        egui::Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color),
    );
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
        }
    }

    /// 헤드리스 `egui` 프레임 하나를 돌린다.
    fn frame(
        ctx: &egui::Context,
        editor: &mut EditorState,
        doc: &mut Document,
        events: Vec<Event>,
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
                out = editor.show(ui, doc, &options(), &Palette::light());
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
        assert!(numbers.iter().all(|(_, right, _)| *right <= body.min.x));
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
