//! 앱 루트 상태 및 화면 조합 (`NotepadApp` — PLAN.md §4.2).
//!
//! - P1-01: 메뉴바·상태 표시줄·본문 영역 골격
//! - P1-02: OS별 시스템 폰트 Fallback 체인(`fonts.rs`) 적용, 로드 실패 시 상태 표시줄/로그 경고
//! - P1-03: 전역 `Command` 열거형, OS별 `Keymap`, 위젯 선처리 `KeyDispatcher`, 상단 7개 메뉴바 연결
//! - P1-09: 본문을 실제 에디터(`ui::editor`)와 문서 모델(`Document`)로 교체, 편집 커맨드·상태 표시줄 커서 연결
//! - P1-10: 멀티 탭(`tabs::Tabs`, `ui::tab_bar`), 파일 열기/저장/다른 이름으로 저장(`rfd` 대화상자 + `core::file_io`),
//!   탭 이동·정렬·닫기 커맨드, 창 메뉴 탭 목록, 창 제목
//! - P1-11: 닫기 대기열 — 탭 닫기·일괄 닫기·창 목록·앱 종료 시 수정된 탭마다 `저장 / 저장 안 함 / 취소` 확인,
//!   `취소`하면 남은 대기열과 종료를 모두 중단. 창 닫기(X·`Alt+F4`·`Cmd+Q`)는 가로채서 같은 대기열로 처리
//! - P1-12: 상태 표시줄 인코딩·줄바꿈 팝업, 인코딩으로 다시 읽기(수정 중이면 확인), 변환 저장,
//!   표현 불가 문자 경고(UTF-8로 저장 · `?`로 바꿔 저장 · 취소), 손상된 문서 저장 경고, 명령줄 파일 인자
//! - P1-13: 파일을 창에 끌어다 놓아 열기(놓기 전 안내 덧그림, 폴더는 건너뜀), 명령줄 인자 해석은 `cli.rs`
//! - P1-14: 5MB 초과 파일은 읽기 전용 뷰어(`ui::viewer`), 1~5MB는 안내 막대, 뷰어 ↔ 편집 모드 전환(큰 파일은 경고)

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::time::Instant;

use eframe::egui::{self, FontId, ViewportCommand};

use crate::commands::{Command, KeyDispatcher, Keymap, ThemeMode, ViewMode};
use crate::core::document::Document;
use crate::core::encoding::{TextEncoding, decode_with_encoding, find_unmappable_chars};
use crate::core::file_io::{self, FileIoErrorKind, OpenMode, OpenOutcome, SaveError, SizePolicy};
use crate::core::line_ending::LineEnding;
use crate::fonts;
use crate::tabs::Tabs;
use crate::ui::{
    dialogs::{self, Dialog, DialogOutcome},
    editor::{self, EditorOptions},
    menu_bar::{self, MenuBarState, MenuTabEntry},
    status_bar::{self, StatusBarInfo},
    tab_bar::{self, TabAction, TabBarState, UNSAVED_PATH_HINT},
    theme::Palette,
    viewer,
};

/// 상태 표시줄 안내 문구를 보여 주는 시간(초).
const NOTICE_SECONDS: f64 = 4.0;

/// 측정 중 문서 전체를 훑으며 프레임 시간을 재는 횟수.
const MEASURE_SCROLL_FRAMES: u64 = 60;

/// `--measure-out` 측정 상태 (`cargo xtask measure --viewer`, P1-14).
struct MeasureRun {
    out: PathBuf,
    started: Instant,
    frame: u64,
    first_frame_ms: f64,
    /// 스크롤 중 프레임별 CPU 시간 (ms, `eframe` `cpu_usage` — 앱 로직 + 그리기, vsync 대기 제외)
    scroll_ms: Vec<f32>,
}

/// My-Notepad 메인 애플리케이션 상태.
pub struct NotepadApp {
    frames: u64,
    font_warning: Option<String>,

    // P1-10 탭·파일 입출력
    tabs: Tabs,
    tab_bar: TabBarState,
    size_policy: SizePolicy,
    dialogs: VecDeque<Dialog>,
    /// 상태 표시줄에 잠시 보여 줄 안내 (문구, 사라질 시각).
    notice: Option<(String, f64)>,
    window_title: String,

    // P1-11 닫기 대기열·종료 가로채기
    /// 닫을 탭 ID (앞에서부터 하나씩 처리)
    close_queue: VecDeque<u64>,
    /// 대기열을 모두 처리하면 앱을 종료한다.
    quit_after_close: bool,
    /// 저장 확인을 마쳤으므로 다음 창 닫기 요청은 가로채지 않는다.
    allow_close: bool,

    // P1-03 커맨드·단축키 및 메뉴 상태
    keymap: Keymap,
    key_dispatcher: KeyDispatcher,
    sidebar_open: bool,
    word_wrap: bool,
    zoom: f32,
    scroll_sync: bool,
    theme: ThemeMode,
    last_command: Option<String>,

    measure: Option<MeasureRun>,
}

impl NotepadApp {
    /// `files`는 명령줄로 받은 파일 경로 — 시작하면서 탭으로 연다.
    pub fn new(cc: &eframe::CreationContext<'_>, files: &[PathBuf]) -> Self {
        let (defs, report) = fonts::explicit_chain();
        cc.egui_ctx.set_fonts(defs);

        log::info!(
            "폰트 체인 로드 완료 ({:.2} ms, 성공 {}개, 실패 {}개)",
            report.total.as_secs_f64() * 1000.0,
            report.loaded.len(),
            report.failures.len()
        );
        for failure in &report.failures {
            log::warn!("시스템 폰트 로드 실패 (기본 폰트로 대체): {failure}");
        }

        let font_warning = report.status_warning();
        let mut app = Self {
            frames: 0,
            font_warning,
            // 첫 실행 화면은 포커스를 받은 빈 `Untitled-1` (UX-2: 바로 글을 쓸 수 있는 상태)
            tabs: Tabs::new(),
            tab_bar: TabBarState::default(),
            size_policy: SizePolicy::default(),
            dialogs: VecDeque::new(),
            notice: None,
            window_title: String::new(),
            close_queue: VecDeque::new(),
            quit_after_close: false,
            allow_close: false,
            keymap: Keymap::current(),
            key_dispatcher: KeyDispatcher::new(),
            sidebar_open: true,
            word_wrap: true,
            zoom: 1.0,
            scroll_sync: true,
            theme: ThemeMode::System,
            last_command: None,
            measure: None,
        };
        for file in files {
            app.open_path(&cc.egui_ctx, file);
        }
        app
    }

    fn execute_command(&mut self, ctx: &egui::Context, command: Command, source: &'static str) {
        let shortcut_hint = self
            .keymap
            .format_shortcut_for(command, ctx)
            .map(|s| format!(" [{s}]"))
            .unwrap_or_default();
        let summary = format!("{} ({source}{shortcut_hint})", command.label());
        log::info!("커맨드 실행: {summary}");

        self.last_command = Some(summary);

        let tab = self.tabs.active_mut();
        if let Some(viewer) = &mut tab.viewer {
            // 뷰어는 읽기 전용: 복사·모두 선택만 처리하고 편집 커맨드는 무시한다.
            match command {
                Command::Copy | Command::Cut => {
                    if let Some(text) = viewer.selected_text(&tab.doc) {
                        ctx.copy_text(text.to_owned());
                    }
                    return;
                }
                Command::SelectAll => {
                    viewer.select_all(&tab.doc);
                    viewer.request_focus();
                    return;
                }
                Command::Undo | Command::Redo | Command::Paste => return,
                _ => {}
            }
        } else if tab.editor.handle_command(ctx, &mut tab.doc, command) {
            return;
        }

        let active = self.tabs.active_index();
        let count = self.tabs.len();
        match command {
            Command::Exit => {
                ctx.send_viewport_cmd(ViewportCommand::Close);
            }

            // ── 파일 ──
            Command::NewTab => {
                self.tabs.new_untitled();
            }
            Command::OpenFile => self.open_with_dialog(ctx),
            Command::Save => {
                self.save_active(ctx, false);
            }
            Command::SaveAs => {
                self.save_active(ctx, true);
            }

            // ── 탭 이동·정렬·닫기 ──
            Command::NextTab => self.tabs.next(),
            Command::PrevTab => self.tabs.prev(),
            Command::GotoTab(i) => {
                self.tabs.activate(i);
            }
            Command::LastTab => self.tabs.last(),
            Command::MoveTabLeft => self.tabs.move_active(false),
            Command::MoveTabRight => self.tabs.move_active(true),
            Command::CloseTab => self.close_tabs(&[active]),
            Command::CloseOtherTabs => {
                let others: Vec<usize> = (0..count).filter(|&i| i != active).collect();
                self.close_tabs(&others);
            }
            Command::CloseTabsToRight => {
                let right: Vec<usize> = (active + 1..count).collect();
                self.close_tabs(&right);
            }
            Command::CloseSavedTabs => {
                let all: Vec<usize> = (0..count).collect();
                self.tabs.close_unmodified(&all);
            }
            Command::CloseAllTabs => {
                let all: Vec<usize> = (0..count).collect();
                self.close_tabs(&all);
            }
            Command::CopyTabPath => self.copy_tab_path(ctx, active),
            Command::ShowWindowList => self.dialogs.push_back(Dialog::window_list()),

            // ── 보기 ──
            Command::ToggleSidebar => {
                self.sidebar_open = !self.sidebar_open;
            }
            Command::ToggleWordWrap => {
                self.word_wrap = !self.word_wrap;
            }
            Command::ZoomIn => {
                self.zoom = (self.zoom + 0.1).min(3.0);
            }
            Command::ZoomOut => {
                self.zoom = (self.zoom - 0.1).max(0.5);
            }
            Command::ZoomReset => {
                self.zoom = 1.0;
            }
            Command::SetViewMode(mode) => {
                self.tabs.active_mut().view_mode = mode;
            }
            Command::ToggleMdPreview => {
                let tab = self.tabs.active_mut();
                tab.view_mode = if tab.view_mode == ViewMode::Preview {
                    ViewMode::Source
                } else {
                    ViewMode::Preview
                };
            }
            Command::ToggleMdSplit => {
                let tab = self.tabs.active_mut();
                tab.view_mode = if tab.view_mode == ViewMode::Split {
                    ViewMode::Source
                } else {
                    ViewMode::Split
                };
            }
            Command::ToggleScrollSync => {
                self.scroll_sync = !self.scroll_sync;
            }
            Command::SetTheme(theme) => {
                self.theme = theme;
                ctx.set_theme(theme.to_egui());
            }

            // ── 인코딩·줄바꿈 (다시 읽기는 P1-12) ──
            Command::ReopenWithEncoding(enc) => self.request_reopen(ctx, enc),
            Command::ConvertToEncoding(enc) => self.convert_encoding(ctx, enc),
            Command::SetLineEnding(le) => {
                self.tabs.active_mut().doc.set_line_ending(le);
            }
            _ => {}
        }
    }

    fn run_tab_action(&mut self, ctx: &egui::Context, action: TabAction) {
        let count = self.tabs.len();
        match action {
            TabAction::Activate(i) => {
                self.tabs.activate(i);
            }
            TabAction::New => {
                self.tabs.new_untitled();
            }
            TabAction::Close(i) => self.close_tabs(&[i]),
            TabAction::CloseOthers(i) => {
                let others: Vec<usize> = (0..count).filter(|&j| j != i).collect();
                self.close_tabs(&others);
            }
            TabAction::CloseRight(i) => {
                let right: Vec<usize> = (i + 1..count).collect();
                self.close_tabs(&right);
            }
            TabAction::CloseSaved => {
                let all: Vec<usize> = (0..count).collect();
                self.tabs.close_unmodified(&all);
            }
            TabAction::CloseAll => {
                let all: Vec<usize> = (0..count).collect();
                self.close_tabs(&all);
            }
            TabAction::CopyPath(i) => self.copy_tab_path(ctx, i),
            TabAction::SetViewMode(mode) => {
                self.tabs.active_mut().view_mode = mode;
            }
        }
    }

    // ── 탭 닫기·경로 복사 ──

    /// 탭을 닫기 대기열에 넣는다. 실제로 닫는 것은 [`Self::process_close_queue`].
    fn close_tabs(&mut self, indices: &[usize]) {
        let ids = self.tabs.ids_of(indices);
        self.queue_close(ids);
    }

    fn queue_close(&mut self, ids: Vec<u64>) {
        for id in ids {
            if !self.close_queue.contains(&id) {
                self.close_queue.push_back(id);
            }
        }
    }

    /// 대화상자가 없을 때 매 프레임 호출: 대기열의 탭을 앞에서부터 닫고, 수정된 탭이면 저장 확인을 띄우고 멈춘다.
    fn process_close_queue(&mut self, ctx: &egui::Context) {
        while self.dialogs.is_empty()
            && let Some(id) = self.close_queue.pop_front()
        {
            let Some(index) = self.tabs.index_of(id) else {
                continue;
            };
            let tab = self.tabs.get(index).expect("index_of 결과");
            if tab.doc.is_modified() {
                let title = tab.doc.title();
                // 무엇을 묻는지 보이도록 그 탭으로 이동한다.
                self.tabs.activate(index);
                self.close_queue.push_front(id);
                self.dialogs.push_back(Dialog::Unsaved {
                    tab_id: id,
                    title,
                    remaining: self.close_queue.len() - 1,
                });
                return;
            }
            self.tabs.close_ids(&[id]);
        }
        if self.dialogs.is_empty() && self.close_queue.is_empty() && self.quit_after_close {
            self.quit_after_close = false;
            self.allow_close = true;
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }
    }

    /// 미저장 확인 결과를 처리한다.
    fn resolve_unsaved(&mut self, ctx: &egui::Context, outcome: &DialogOutcome) {
        match *outcome {
            DialogOutcome::Save(id) => {
                let saved = match self.tabs.index_of(id) {
                    Some(index) => {
                        self.tabs.activate(index);
                        self.save_active(ctx, false)
                    }
                    None => false,
                };
                if saved {
                    self.close_queue.retain(|&q| q != id);
                    self.tabs.close_ids(&[id]);
                } else {
                    // 다른 이름으로 저장을 취소했거나 저장에 실패하면 닫지 않고 멈춘다.
                    self.cancel_close();
                }
            }
            DialogOutcome::Discard(id) => {
                self.close_queue.retain(|&q| q != id);
                self.tabs.close_ids(&[id]);
            }
            DialogOutcome::CancelClose => self.cancel_close(),
            _ => {}
        }
    }

    fn cancel_close(&mut self) {
        self.close_queue.clear();
        self.quit_after_close = false;
    }

    /// 창 닫기 요청(X 버튼·`Alt+F4`·`Cmd+Q`·종료 메뉴)을 가로채 수정된 탭마다 저장 확인을 받는다.
    fn intercept_close_request(&mut self, ctx: &egui::Context) {
        if !ctx.input(|i| i.viewport().close_requested()) || self.allow_close {
            return;
        }
        let modified: Vec<u64> = self
            .tabs
            .iter()
            .filter(|t| t.doc.is_modified())
            .map(|t| t.id())
            .collect();
        if modified.is_empty() {
            return;
        }
        ctx.send_viewport_cmd(ViewportCommand::CancelClose);
        if self.quit_after_close {
            return; // 이미 종료 확인 중
        }
        self.quit_after_close = true;
        self.queue_close(modified);
    }

    fn copy_tab_path(&mut self, ctx: &egui::Context, index: usize) {
        let path = self
            .tabs
            .get(index)
            .and_then(|t| t.doc.path())
            .map(|p| p.display().to_string());
        match path {
            Some(path) => {
                ctx.copy_text(path.clone());
                self.show_notice(ctx, format!("경로를 복사했습니다: {path}"));
            }
            None => self.show_notice(ctx, "저장되지 않은 새 문서라 경로가 없습니다."),
        }
    }

    // ── 파일 열기 ──

    /// 대화상자의 시작 폴더: 활성 탭 파일의 폴더, 없으면 경로가 있는 아무 탭의 폴더.
    fn dialog_directory(&self) -> Option<PathBuf> {
        std::iter::once(self.tabs.active())
            .chain(self.tabs.iter())
            .find_map(|t| t.doc.path()?.parent().map(Path::to_path_buf))
    }

    fn open_with_dialog(&mut self, ctx: &egui::Context) {
        let mut picker = rfd::FileDialog::new().set_title("열기");
        if let Some(dir) = self.dialog_directory() {
            picker = picker.set_directory(dir);
        }
        let Some(paths) = picker.pick_files() else {
            return;
        };
        for path in paths {
            self.open_path(ctx, &path);
        }
    }

    /// 파일을 새 탭으로 연다. 이미 열려 있으면 그 탭으로 이동한다.
    pub fn open_path(&mut self, ctx: &egui::Context, path: &Path) {
        if let Some(index) = self.tabs.find_path(path) {
            self.tabs.activate(index);
            return;
        }
        match file_io::open_document(
            path,
            &self.size_policy,
            TextEncoding::Utf8,
            LineEnding::default_for_os(),
        ) {
            Ok(OpenOutcome::Opened(opened)) => {
                let mode = opened.open_mode;
                let mut document = opened.document;
                // 읽기 전용 파일도 메모장처럼 편집은 허용하고, 저장할 때 다른 이름으로 저장을 유도한다 (PLAN.md §3.6).
                if mode != OpenMode::Viewer {
                    document.set_readonly(false);
                }
                let index = self.tabs.open_document(document, opened.file_readonly);
                if let Some(tab) = self.tabs.get_mut(index) {
                    // 5MB 초과는 뷰어, 1~5MB는 안내 막대 (ADR 0001)
                    tab.apply_open_mode(mode);
                }
                if mode == OpenMode::Edit && opened.file_readonly {
                    self.show_notice(
                        ctx,
                        "읽기 전용 파일입니다. 편집 후 저장하면 다른 이름으로 저장합니다.",
                    );
                }
            }
            Ok(OpenOutcome::Binary(loaded)) => {
                self.dialogs.push_back(Dialog::ConfirmBinary(loaded));
            }
            Err(err) => {
                self.dialogs
                    .push_back(Dialog::message("파일을 열 수 없습니다", err.to_string()));
            }
        }
    }

    // ── 파일 저장 ──

    fn save_dialog(&self) -> Option<PathBuf> {
        let tab = self.tabs.active();
        let title = tab.doc.title();
        let text = ("텍스트 문서", &["txt"][..]);
        let markdown = ("Markdown", &["md", "markdown"][..]);
        let filters = if tab.doc.is_markdown() {
            [markdown, text]
        } else {
            [text, markdown]
        };
        let mut picker = rfd::FileDialog::new()
            .set_title("다른 이름으로 저장")
            .set_file_name(&title);
        for (name, exts) in filters {
            picker = picker.add_filter(name, exts);
        }
        picker = picker.add_filter("모든 파일", &["*"]);
        if let Some(dir) = self.dialog_directory() {
            picker = picker.set_directory(dir);
        }
        picker.save_file()
    }

    /// 활성 탭을 저장한다. 새 문서·읽기 전용 파일·`save_as`면 다른 이름으로 저장 대화상자를 연다.
    ///
    /// 저장했으면 `true`. 대화상자를 취소했거나 실패하면 `false`.
    fn save_active(&mut self, ctx: &egui::Context, save_as: bool) -> bool {
        let tab = self.tabs.active();
        let file_readonly = tab.file_readonly;
        let has_path = tab.doc.path().is_some();
        if file_readonly && !save_as {
            self.show_notice(ctx, "읽기 전용 파일이라 다른 이름으로 저장합니다.");
        }
        let target = if save_as || !has_path || file_readonly {
            let Some(path) = self.save_dialog() else {
                return false;
            };
            if let Some(other) = self.tabs.find_path(&path)
                && other != self.tabs.active_index()
            {
                self.dialogs.push_back(Dialog::message(
                    "저장할 수 없습니다",
                    format!(
                        "\"{}\"은(는) 다른 탭에서 열려 있습니다. 그 탭을 닫은 뒤 다시 저장해 주세요.",
                        path.display()
                    ),
                ));
                return false;
            }
            Some(path)
        } else {
            None
        };

        let id = self.tabs.active().id();
        self.save_tab(ctx, id, target, false)
    }

    /// 탭을 저장한다 (`target`이 있으면 그 경로로). 손상된 문서는 `allow_damaged`가 아니면 먼저 경고하고,
    /// 표현 불가 문자가 있으면 저장을 막고 경고 대화상자를 띄운다.
    ///
    /// 저장했으면 `true`. 경고를 띄웠거나 실패하면 `false`.
    fn save_tab(
        &mut self,
        ctx: &egui::Context,
        tab_id: u64,
        target: Option<PathBuf>,
        allow_damaged: bool,
    ) -> bool {
        let Some(index) = self.tabs.index_of(tab_id) else {
            return false;
        };
        self.tabs.activate(index);
        let tab = self.tabs.active_mut();
        if tab.doc.is_damaged() && !allow_damaged {
            self.dialogs
                .push_back(Dialog::DamagedSave { tab_id, target });
            return false;
        }

        let eol_notice = tab.doc.needs_line_ending_conversion_notice();
        let line_ending = tab.doc.line_ending().short_label();
        match file_io::save_document(&mut tab.doc, target.as_deref()) {
            Ok(()) => {
                if target.is_some() {
                    tab.refresh_path_key();
                    tab.file_readonly = false;
                }
                let title = tab.doc.title();
                if eol_notice {
                    tab.doc.mark_line_ending_conversion_notified();
                    self.show_notice(
                        ctx,
                        format!(
                            "\"{title}\" 저장됨 — 섞여 있던 줄바꿈을 {line_ending}로 통일했습니다."
                        ),
                    );
                } else {
                    self.show_notice(ctx, format!("\"{title}\" 저장됨"));
                }
                true
            }
            Err(SaveError::Io(err))
                if err.kind() == FileIoErrorKind::ReadOnly && target.is_none() =>
            {
                tab.file_readonly = true;
                self.save_active(ctx, true)
            }
            Err(SaveError::Unmappable(err)) => {
                self.dialogs.push_back(Dialog::Unmappable {
                    tab_id,
                    target,
                    encoding: err.encoding,
                    chars: err.unmappable,
                });
                false
            }
            Err(SaveError::NoPath) => self.save_active(ctx, true),
            Err(SaveError::Io(err)) => {
                self.dialogs.push_back(Dialog::message(
                    "저장하지 못했습니다",
                    format!("{err}\n원본 파일은 바뀌지 않았습니다."),
                ));
                false
            }
        }
    }

    /// 표현 불가 문자 경고에서 고른 방법으로 다시 저장한다.
    fn resolve_unmappable(
        &mut self,
        ctx: &egui::Context,
        tab_id: u64,
        target: Option<PathBuf>,
        to_utf8: bool,
    ) {
        let Some(index) = self.tabs.index_of(tab_id) else {
            return;
        };
        let doc = &mut self.tabs.get_mut(index).expect("index_of 결과").doc;
        if to_utf8 {
            doc.convert_to_encoding(TextEncoding::Utf8);
        } else {
            // 디스크와 화면이 같도록 문서에서도 `?`로 바꾼다 (Undo 한 번으로 되돌릴 수 있음).
            doc.replace_unmappable();
        }
        // 이 경고까지 왔다면 손상 경고는 이미 통과했다.
        self.save_tab(ctx, tab_id, target, true);
    }

    // ── 인코딩 다시 읽기·변환 ──

    /// 인코딩으로 다시 읽기: 수정된 문서면 변경 내용이 사라진다고 먼저 확인한다.
    fn request_reopen(&mut self, ctx: &egui::Context, encoding: TextEncoding) {
        let tab = self.tabs.active();
        if tab.doc.path().is_none() {
            self.show_notice(ctx, "저장된 파일만 다른 인코딩으로 다시 읽을 수 있습니다.");
            return;
        }
        if tab.doc.is_modified() {
            self.dialogs.push_back(Dialog::ConfirmReopen {
                tab_id: tab.id(),
                title: tab.doc.title(),
                encoding,
            });
        } else {
            self.reopen_tab(ctx, tab.id(), encoding);
        }
    }

    /// 디스크의 파일을 지정한 인코딩으로 다시 해석해 같은 탭에 연다.
    fn reopen_tab(&mut self, ctx: &egui::Context, tab_id: u64, encoding: TextEncoding) {
        let Some(tab) = self
            .tabs
            .index_of(tab_id)
            .and_then(|i| self.tabs.get_mut(i))
        else {
            return;
        };
        let Some(path) = tab.doc.path().map(Path::to_path_buf) else {
            return;
        };
        match file_io::read_file(&path, &self.size_policy) {
            Ok(loaded) => {
                let decoded = decode_with_encoding(&loaded.bytes, encoding);
                let readonly = loaded.open_mode == OpenMode::Viewer;
                let doc = Document::from_decoded(
                    Some(path),
                    decoded,
                    LineEnding::default_for_os(),
                    readonly,
                    Some(loaded.metadata),
                );
                let damaged = doc.is_damaged();
                tab.file_readonly = loaded.readonly;
                tab.replace_document(doc);
                let mut text = format!("{}(으)로 다시 읽었습니다.", encoding.label());
                if damaged {
                    text.push_str(" 해석하지 못한 바이트가 있습니다(�).");
                }
                self.show_notice(ctx, text);
            }
            Err(err) => {
                self.dialogs
                    .push_back(Dialog::message("다시 읽지 못했습니다", err.to_string()));
            }
        }
    }

    /// 변환 저장: 내용은 그대로, 저장할 인코딩만 바꾼다. 표현 불가 문자가 있으면 미리 알려 준다.
    fn convert_encoding(&mut self, ctx: &egui::Context, encoding: TextEncoding) {
        let doc = &mut self.tabs.active_mut().doc;
        if doc.is_readonly() {
            self.show_notice(ctx, "읽기 전용 문서는 인코딩을 바꿀 수 없습니다.");
            return;
        }
        if !doc.convert_to_encoding(encoding) {
            return;
        }
        let unmappable = find_unmappable_chars(doc.text(), encoding, usize::MAX).len();
        let text = if unmappable > 0 {
            format!(
                "저장할 때 {}(으)로 저장합니다 — 이 인코딩에 없는 문자가 {unmappable}개 있어 저장 전에 확인합니다.",
                encoding.label()
            )
        } else {
            format!("저장할 때 {}(으)로 저장합니다.", encoding.label())
        };
        self.show_notice(ctx, text);
    }

    /// 측정 모드를 켠다: 첫 화면 시간을 기록하고, 활성 탭을 처음부터 끝까지 훑으며 프레임 시간을 잰 뒤
    /// `out` 파일에 `key=value` 줄로 쓴다 (마지막 줄 `done=1`).
    pub fn enable_measure(&mut self, out: PathBuf, started: Instant) {
        self.measure = Some(MeasureRun {
            out,
            started,
            frame: 0,
            first_frame_ms: 0.0,
            scroll_ms: Vec::new(),
        });
    }

    fn step_measure(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        let Some(m) = &mut self.measure else {
            return;
        };
        m.frame += 1;
        ctx.request_repaint();
        if m.frame == 2 {
            // 두 번째 프레임이 시작됐다 = 첫 프레임이 화면에 나갔다
            m.first_frame_ms = m.started.elapsed().as_secs_f64() * 1000.0;
        } else if m.frame > 3
            && let Some(cpu) = frame.info().cpu_usage
        {
            // 직전 프레임(스크롤 위치를 바꾼 프레임)의 CPU 시간
            m.scroll_ms.push(cpu * 1000.0);
        }
        if m.frame >= 3 && m.frame < 3 + MEASURE_SCROLL_FRAMES {
            let step = m.frame - 3;
            let tab = self.tabs.active_mut();
            let lines = tab.doc.line_count() as u64;
            if let Some(viewer) = &mut tab.viewer {
                viewer.scroll_to_line((lines * step / MEASURE_SCROLL_FRAMES) as usize);
            }
        } else if m.frame >= 3 + MEASURE_SCROLL_FRAMES {
            let avg = m.scroll_ms.iter().sum::<f32>() / m.scroll_ms.len().max(1) as f32;
            let max = m.scroll_ms.iter().copied().fold(0.0, f32::max);
            let viewer = self.tabs.active().viewer.is_some();
            let report = format!(
                "first_frame_ms={:.0}
scroll_frame_ms_avg={avg:.1}
scroll_frame_ms_max={max:.1}
viewer={viewer}
done=1
",
                m.first_frame_ms
            );
            if let Err(err) = std::fs::write(&m.out, report) {
                log::warn!("측정 결과 쓰기 실패: {err}");
            }
            self.measure = None;
        }
    }

    // ── 끌어다 놓기 ──

    /// 창에 끌어다 놓은 파일을 탭으로 연다. 폴더는 아직 열 수 없으므로 건너뛰고 알려 준다 (사이드바는 Phase 2).
    fn open_dropped_files(&mut self, ctx: &egui::Context) {
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .collect()
        });
        if dropped.is_empty() {
            return;
        }
        let mut folders = 0;
        for path in dropped {
            if path.is_dir() {
                folders += 1;
            } else {
                self.open_path(ctx, &path);
            }
        }
        if folders > 0 {
            self.show_notice(
                ctx,
                format!("폴더 {folders}개는 건너뛰었습니다. 파일을 끌어다 놓아 주세요."),
            );
        }
        // 다른 앱에서 끌어 온 뒤 바로 입력할 수 있게 창을 앞으로 가져온다.
        ctx.send_viewport_cmd(ViewportCommand::Focus);
    }

    /// 파일을 창 위로 끌고 있는 동안 "놓으면 탭으로 엽니다" 안내를 덧그린다.
    fn paint_drop_overlay(ctx: &egui::Context) {
        let count = ctx.input(|i| i.raw.hovered_files.len());
        if count == 0 {
            return;
        }
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::unique("drop-overlay"),
        ));
        let rect = ctx.content_rect();
        painter.rect_filled(rect, 0.0, egui::Color32::from_black_alpha(150));
        let text = if count > 1 {
            format!("여기에 놓으면 파일 {count}개를 탭으로 엽니다")
        } else {
            "여기에 놓으면 탭으로 엽니다".to_owned()
        };
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            text,
            FontId::proportional(22.0),
            egui::Color32::WHITE,
        );
    }

    // ── 안내·창 제목 ──

    fn show_notice(&mut self, ctx: &egui::Context, text: impl Into<String>) {
        let until = ctx.input(|i| i.time) + NOTICE_SECONDS;
        self.notice = Some((text.into(), until));
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(NOTICE_SECONDS));
    }

    fn update_window_title(&mut self, ctx: &egui::Context) {
        let doc = &self.tabs.active().doc;
        let title = format!(
            "{}{} - My-Notepad",
            if doc.is_modified() { "● " } else { "" },
            doc.title()
        );
        if title != self.window_title {
            ctx.send_viewport_cmd(ViewportCommand::Title(title.clone()));
            self.window_title = title;
        }
    }
}

impl eframe::App for NotepadApp {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.frames += 1;
        let ctx = ui.ctx().clone();
        self.step_measure(&ctx, frame);

        // 0. 창 닫기 요청 가로채기 (수정된 탭이 있으면 저장 확인), 끌어다 놓은 파일 열기
        self.intercept_close_request(&ctx);
        self.open_dropped_files(&ctx);

        // 1. 위젯보다 먼저 전역 단축키를 소비·디스패치한다 (PLAN.md §3.5). 대화상자가 떠 있으면 막는다.
        if self.dialogs.is_empty()
            && let Some(cmd) = self.key_dispatcher.dispatch(&ctx, &self.keymap)
        {
            self.execute_command(&ctx, cmd, "단축키");
        }

        let palette = Palette::from_visuals(ui.visuals());

        // 2. 상단 메뉴바 렌더링 (`파일 · 편집 · 검색 · 보기 · 인코딩 · 창 · 도움말`)
        let tab_infos: Vec<(String, String, bool)> = self
            .tabs
            .iter()
            .map(|t| {
                (
                    t.doc.title(),
                    t.doc
                        .path()
                        .map_or_else(|| UNSAVED_PATH_HINT.to_owned(), |p| p.display().to_string()),
                    t.doc.is_modified(),
                )
            })
            .collect();
        let menu_tabs: Vec<MenuTabEntry<'_>> = tab_infos
            .iter()
            .map(|(title, path_hint, modified)| MenuTabEntry {
                title,
                path_hint,
                modified: *modified,
            })
            .collect();
        let active = self.tabs.active();
        let menu_state = MenuBarState {
            sidebar_open: self.sidebar_open,
            word_wrap: self.word_wrap,
            is_markdown: active.doc.is_markdown(),
            view_mode: active.view_mode,
            scroll_sync: self.scroll_sync,
            theme: self.theme,
            encoding: active.doc.encoding(),
            line_ending: active.doc.line_ending(),
            mixed_line_endings: active.doc.has_mixed_line_endings(),
            // 최근 파일 목록은 설정 저장(P5)과 함께 구현
            recent_files: &[],
            tabs: &menu_tabs,
            active_tab: self.tabs.active_index(),
            modified_marker_color: palette.modified_marker,
        };

        let mut menu_cmd = None;
        egui::Panel::top("menu_bar").show(ui, |ui| {
            menu_cmd = menu_bar::show(ui, &self.keymap, &menu_state);
        });
        if let Some(cmd) = menu_cmd {
            self.execute_command(&ctx, cmd, "메뉴");
        }

        // 3. 탭 바
        let mut tab_action = None;
        egui::Panel::top("tab_bar").show(ui, |ui| {
            tab_action = tab_bar::show(ui, &mut self.tab_bar, &self.tabs, &self.keymap);
        });
        if let Some(action) = tab_action {
            self.run_tab_action(&ctx, action);
        }

        // 4. 하단 상태 표시줄
        let now = ctx.input(|i| i.time);
        if self.notice.as_ref().is_some_and(|(_, until)| *until <= now) {
            self.notice = None;
        }
        let chord_hint = self.key_dispatcher.is_chord_k_pending().then(|| {
            format!(
                "{} 입력됨 — 다음 키(V) 대기 중…",
                ctx.format_shortcut(&self.keymap.chord_k())
            )
        });
        let active_tab = self.tabs.active();
        let doc = &active_tab.doc;
        let (line, column, selected_chars) = match &active_tab.viewer {
            Some(viewer) => viewer.cursor_status(doc),
            None => editor::cursor_status(doc),
        };
        let status_info = StatusBarInfo {
            line,
            column,
            selected_chars,
            line_count: doc.line_count(),
            zoom: self.zoom,
            encoding: doc.encoding(),
            encoding_estimated: doc.is_encoding_estimated(),
            encoding_fallback_from: doc.encoding_fallback_from(),
            damaged: doc.is_damaged(),
            line_ending: doc.line_ending(),
            mixed_line_endings: doc.has_mixed_line_endings(),
            font_warning: self.font_warning.as_deref(),
            chord_hint: chord_hint.as_deref(),
            notice: self.notice.as_ref().map(|(text, _)| text.as_str()),
            last_command: self.last_command.as_deref(),
        };
        let mut status_cmd = None;
        egui::Panel::bottom("status_bar").show(ui, |ui| {
            status_cmd = status_bar::show(ui, &palette, &status_info);
        });
        if let Some(cmd) = status_cmd {
            self.execute_command(&ctx, cmd, "상태 표시줄");
        }

        // 5. 본문 영역: 활성 탭 에디터
        let font_size = ui.style().text_styles[&egui::TextStyle::Monospace].size * self.zoom;
        let options = EditorOptions {
            word_wrap: self.word_wrap,
            font: FontId::monospace(font_size),
        };
        let mut switch_to_edit = false;
        let mut edit_large = None;
        egui::CentralPanel::default()
            .frame(egui::Frame::central_panel(ui.style()).inner_margin(0))
            .show(ui, |ui| {
                let viewer_above = self.size_policy.viewer_above();
                let tab = self.tabs.active_mut();
                let tab_id = tab.id();
                if let Some(viewer) = &mut tab.viewer {
                    let size = tab
                        .doc
                        .disk_metadata()
                        .map_or(tab.doc.text().len() as u64, |m| m.byte_len);
                    if viewer::readonly_bar(ui, &palette, size) {
                        if size > viewer_above {
                            edit_large = Some(Dialog::ConfirmEditLarge {
                                tab_id,
                                title: tab.doc.title(),
                                size_bytes: size,
                                limit_bytes: viewer_above,
                            });
                        } else {
                            switch_to_edit = true;
                        }
                    }
                    viewer.show(ui, &tab.doc, &options.font, &palette);
                } else {
                    if tab.size_notice {
                        match viewer::size_banner(ui, &palette) {
                            Some(viewer::BannerAction::ViewReadOnly) => tab.switch_to_viewer(),
                            Some(viewer::BannerAction::Close) => tab.size_notice = false,
                            None => {}
                        }
                    }
                    if tab.viewer.is_none() {
                        tab.editor.show(ui, &mut tab.doc, &options);
                    }
                }
            });
        if switch_to_edit {
            self.tabs.active_mut().switch_to_editor();
        }
        if let Some(dialog) = edit_large {
            self.dialogs.push_back(dialog);
        }

        // 6. 대화상자 (한 번에 하나씩)
        let mut current = self.dialogs.pop_front();
        let outcome = dialogs::show(&ctx, &mut current, &self.tabs);
        if let Some(dialog) = current {
            self.dialogs.push_front(dialog);
        }
        match outcome {
            Some(DialogOutcome::OpenBinary(loaded)) => {
                let opened = file_io::open_binary_anyway(*loaded, LineEnding::default_for_os());
                let index = self
                    .tabs
                    .open_document(opened.document, opened.file_readonly);
                if let Some(tab) = self.tabs.get_mut(index) {
                    tab.binary = true;
                    tab.apply_open_mode(opened.open_mode);
                }
            }
            Some(DialogOutcome::OpenEditMode(tab_id)) => {
                if let Some(tab) = self
                    .tabs
                    .index_of(tab_id)
                    .and_then(|i| self.tabs.get_mut(i))
                {
                    tab.switch_to_editor();
                }
            }
            Some(DialogOutcome::Activate(id)) => {
                if let Some(index) = self.tabs.index_of(id) {
                    self.tabs.activate(index);
                }
            }
            Some(DialogOutcome::CloseTabs(ids)) => self.queue_close(ids),
            Some(
                ref unsaved @ (DialogOutcome::Save(_)
                | DialogOutcome::Discard(_)
                | DialogOutcome::CancelClose),
            ) => self.resolve_unsaved(&ctx, unsaved),
            Some(DialogOutcome::SaveAsUtf8 { tab_id, target }) => {
                self.resolve_unmappable(&ctx, tab_id, target, true);
            }
            Some(DialogOutcome::SaveReplacing { tab_id, target }) => {
                self.resolve_unmappable(&ctx, tab_id, target, false);
            }
            Some(DialogOutcome::SaveDamaged { tab_id, target }) => {
                self.save_tab(&ctx, tab_id, target, true);
            }
            Some(DialogOutcome::Reopen { tab_id, encoding }) => {
                self.reopen_tab(&ctx, tab_id, encoding);
            }
            Some(DialogOutcome::Dismissed) | None => {}
        }

        // 7. 닫기 대기열 (대화상자가 없을 때만 진행)
        self.process_close_queue(&ctx);

        // 8. 끌어다 놓기 안내 (맨 위에 덧그림)
        Self::paint_drop_overlay(&ctx);

        self.update_window_title(&ctx);
    }
}
