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
//! - P1-13: 파일을 창에 끌어다 놓아 열기(놓기 전 안내 덧그림), 명령줄 인자 해석은 `cli.rs`
//! - P2-08: 설정 영속화(`config.rs` — 창 위치·크기, 사이드바, 글꼴 확대, 줄바꿈, 탭, 강조 옵션, 테마, 최근 파일).
//!   바뀌면 1초 뒤 자동 저장하고, 종료할 때 한 번 더 저장한다
//! - P2-07: 앱이 포커스를 얻을 때 열린 파일의 외부 변경·삭제를 확인 (다시 불러오기 확인, 삭제된 탭 표시)
//! - P2-06: 좌측 폴더 탐색 사이드바 — `폴더 열기`·끌어다 놓은 폴더·명령줄 폴더 인자로 열고, 파일 클릭 시 탭으로 연다
//! - P1-14: 5MB 초과 파일은 읽기 전용 뷰어(`ui::viewer`), 1~5MB는 안내 막대, 뷰어 ↔ 편집 모드 전환(큰 파일은 경고)

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::time::Instant;

use eframe::egui::{self, FontId, ViewportCommand};

use crate::commands::{Command, KeyDispatcher, Keymap, ThemeMode, ViewMode};
use crate::config::{Settings, WindowGeometry};
use crate::core::document::Document;
use crate::core::encoding::{TextEncoding, decode_with_encoding, find_unmappable_chars};
use crate::core::file_io::{self, FileIoErrorKind, OpenMode, OpenOutcome, SaveError, SizePolicy};
use crate::core::highlight::HighlightOptions;
use crate::core::line_ending::LineEnding;
use crate::fonts;
use crate::tabs::{ExternalChange, Tabs};
use crate::ui::{
    dialogs::{self, Dialog, DialogOutcome},
    doc_view,
    editor::{self, EditorOptions},
    markdown_view::LinkAction,
    menu_bar::{self, MenuBarState, MenuTabEntry},
    sidebar::{self, SidebarAction, SidebarState},
    status_bar::{self, StatusBarInfo},
    tab_bar::{self, TabAction, TabBarState, UNSAVED_PATH_HINT},
    theme::{self, Palette},
    viewer,
};

/// 설정이 바뀐 뒤 저장까지 기다리는 시간(초) — 창을 끄는 동안 여러 번 쓰지 않게.
const SETTINGS_SAVE_DELAY: f64 = 1.0;

/// 상태 표시줄 안내 문구를 보여 주는 시간(초).
const NOTICE_SECONDS: f64 = 4.0;

/// 저장 시도 결과.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SaveResult {
    /// 디스크에 저장했다.
    Saved,
    /// 손상·표현 불가 문자 경고를 띄웠다 — 사용자가 고른 뒤 저장될 수 있다.
    Pending,
    /// 취소했거나 실패했다 (오류 대화상자를 띄웠을 수 있음).
    NotSaved,
}

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
    sidebar: SidebarState,
    word_wrap: bool,
    /// 탭 너비 (공백 칸 수)
    tab_width: u8,
    /// `Tab` 키로 공백 넣기
    tab_spaces: bool,
    /// 적용 중인 글꼴 정의 (탭 너비를 바꿀 때 다시 적용)
    font_defs: egui::FontDefinitions,
    /// 지난 프레임에 창이 포커스를 가졌는지 (포커스를 다시 얻으면 외부 변경 확인)
    window_focused: bool,
    zoom: f32,
    /// Markdown 분할 비율 (P3-06)
    split_ratio: f32,
    scroll_sync: bool,
    /// 선택 단어 일괄 강조 옵션
    highlight: HighlightOptions,
    theme: ThemeMode,
    last_command: Option<String>,

    // P2-08 설정 영속화
    /// 설정 파일 경로 (`None`이면 읽지도 저장하지도 않음 — 성능 측정 실행)
    settings_path: Option<PathBuf>,
    /// 마지막으로 저장한 설정 (지금 상태와 다르면 저장 대기)
    saved_settings: Settings,
    /// 최근 파일·창 위치·사이드바 폭 등 앱 필드로 따로 두지 않는 설정
    settings: Settings,
    /// 설정이 처음 바뀐 시각 (1초 뒤 자동 저장)
    settings_dirty_since: Option<f64>,

    measure: Option<MeasureRun>,
}

impl NotepadApp {
    /// `files`는 명령줄로 받은 파일 경로 — 시작하면서 탭으로 연다.
    /// `settings`는 저장된 설정(`settings_path`에서 읽음), `settings_problem`은 설정을 읽지 못한 이유.
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        files: &[PathBuf],
        settings: Settings,
        settings_path: Option<PathBuf>,
        settings_problem: Option<String>,
    ) -> Self {
        let (mut font_defs, report) = fonts::explicit_chain();
        fonts::set_tab_size(&mut font_defs, f32::from(settings.tab_width));
        cc.egui_ctx.set_fonts(font_defs.clone());
        theme::configure_visuals(&cc.egui_ctx);
        cc.egui_ctx.set_theme(settings.theme.to_egui());
        // `Ctrl/Cmd + +/-/0`은 에디터 글꼴 확대/축소(P2-02)로 쓴다. egui 기본 동작(화면 전체 확대)은 끈다.
        cc.egui_ctx.options_mut(|o| o.zoom_with_keyboard = false);

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
            sidebar_open: settings.sidebar_open,
            sidebar: SidebarState::default(),
            word_wrap: settings.word_wrap,
            tab_width: settings.tab_width,
            tab_spaces: settings.tab_spaces,
            font_defs,
            window_focused: false,
            zoom: settings.zoom,
            split_ratio: settings.split_ratio,
            scroll_sync: true,
            highlight: HighlightOptions {
                whole_word: settings.highlight_whole_word,
                match_case: settings.highlight_match_case,
                ..HighlightOptions::default()
            },
            theme: settings.theme,
            last_command: None,
            measure: None,
            settings_path,
            saved_settings: settings.clone(),
            settings: settings.clone(),
            settings_dirty_since: None,
        };
        // 지난번 사이드바 폴더 (없어졌으면 조용히 넘어간다). 명령줄 폴더 인자가 있으면 아래에서 바뀐다.
        if let Some(folder) = &settings.sidebar_folder
            && folder.is_dir()
            && let Err(e) = app.sidebar.open_folder(folder)
        {
            log::warn!(
                "지난번 사이드바 폴더를 열지 못함 ({}): {e}",
                folder.display()
            );
        }
        if let Some(problem) = settings_problem {
            app.dialogs
                .push_back(Dialog::message("설정을 불러오지 못했습니다", problem));
        }
        for file in files {
            app.open_any(&cc.egui_ctx, file);
        }
        app
    }

    /// 미리보기에서 누른 링크를 처리한다 (PLAN.md §3.8-5, P3-09).
    fn handle_link(&mut self, ctx: &egui::Context, action: LinkAction) {
        match action {
            LinkAction::OpenUrl(url) => ctx.open_url(egui::OpenUrl::new_tab(url)),
            LinkAction::OpenFile { path, anchor } => {
                let text_like = path.extension().is_some_and(|e| {
                    ["md", "markdown", "txt"]
                        .iter()
                        .any(|t| e.eq_ignore_ascii_case(t))
                });
                if !text_like || path.is_dir() {
                    // 문서가 아닌 파일·폴더는 OS 기본 프로그램으로
                    ctx.open_url(egui::OpenUrl::new_tab(format!(
                        "file:///{}",
                        path.display().to_string().replace('\\', "/")
                    )));
                    return;
                }
                self.open_path(ctx, &path);
                if let Some(anchor) = anchor {
                    let tab = self.tabs.active_mut();
                    if tab.doc.path().is_some_and(|p| self::same_path(p, &path)) {
                        tab.preview.jump_to_anchor(anchor);
                    }
                }
            }
            LinkAction::Notice(text) => self.show_notice(ctx, text),
        }
    }

    /// 활성 탭의 보기 모드를 바꾸고, 파일이면 다음에 열 때 쓰도록 기억한다 (P3-06).
    fn set_view_mode(&mut self, mode: ViewMode) {
        let tab = self.tabs.active_mut();
        tab.view_mode = mode;
        if let Some(path) = tab.doc.path().map(Path::to_path_buf) {
            self.settings.remember_view_mode(&path, mode);
        }
    }

    // ── 설정 영속화 (P2-08) ──

    /// 지금 앱 상태를 설정으로 모은다.
    fn settings_snapshot(&self) -> Settings {
        Settings {
            sidebar_open: self.sidebar_open,
            sidebar_folder: self.sidebar.folder().map(Path::to_path_buf),
            zoom: self.zoom,
            split_ratio: self.split_ratio,
            word_wrap: self.word_wrap,
            tab_width: self.tab_width,
            tab_spaces: self.tab_spaces,
            highlight_whole_word: self.highlight.whole_word,
            highlight_match_case: self.highlight.match_case,
            theme: self.theme,
            ..self.settings.clone()
        }
    }

    /// 창 위치·크기를 기억한다. 최대화 중에는 최대화 전 크기를 그대로 두고, 최소화 중에는 건드리지 않는다.
    fn track_window_geometry(&mut self, ctx: &egui::Context) {
        let (outer, inner, maximized, minimized) = ctx.input(|i| {
            let v = i.viewport();
            (v.outer_rect, v.inner_rect, v.maximized, v.minimized)
        });
        if minimized == Some(true) {
            return;
        }
        if maximized == Some(true) {
            if let Some(geometry) = &mut self.settings.window {
                geometry.maximized = true;
            }
        } else if let (Some(outer), Some(inner)) = (outer, inner) {
            self.settings.window = Some(WindowGeometry {
                x: outer.min.x,
                y: outer.min.y,
                width: inner.width(),
                height: inner.height(),
                maximized: false,
            });
        }
    }

    /// 설정이 바뀌었으면 1초 동안 더 바뀌지 않을 때 저장한다 (창을 끄는 중에도 [`Self::save_settings_now`]).
    fn autosave_settings(&mut self, ctx: &egui::Context) {
        if self.settings_path.is_none() {
            return;
        }
        let snapshot = self.settings_snapshot();
        if snapshot == self.saved_settings {
            self.settings_dirty_since = None;
            return;
        }
        let now = ctx.input(|i| i.time);
        match self.settings_dirty_since {
            Some(since) if now - since >= SETTINGS_SAVE_DELAY => self.save_settings_now(),
            Some(_) => {}
            None => {
                self.settings_dirty_since = Some(now);
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(
                    SETTINGS_SAVE_DELAY + 0.05,
                ));
            }
        }
    }

    /// 지금 설정을 바로 저장한다. 실패하면 로그만 남기고, 같은 내용으로 다시 시도하지 않는다.
    fn save_settings_now(&mut self) {
        let Some(path) = &self.settings_path else {
            return;
        };
        let snapshot = self.settings_snapshot();
        if snapshot == self.saved_settings {
            return;
        }
        match snapshot.save(path) {
            Ok(()) => log::info!("설정 저장: {}", path.display()),
            Err(e) => log::warn!("설정 저장 실패 ({}): {e}", path.display()),
        }
        self.saved_settings = snapshot;
        self.settings_dirty_since = None;
    }

    /// 최근 파일 열기. 파일이 없어졌으면 목록에서 지우고 알린다.
    fn open_recent(&mut self, ctx: &egui::Context, index: usize) {
        let Some(path) = self.settings.recent_files.get(index).cloned() else {
            return;
        };
        if path.is_file() {
            self.open_path(ctx, &path);
        } else {
            self.settings.recent_files.remove(index);
            self.show_notice(
                ctx,
                format!(
                    "\"{}\" 파일이 없어 최근 파일 목록에서 지웠습니다.",
                    path.file_name().unwrap_or_default().to_string_lossy()
                ),
            );
        }
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
        match command {
            Command::Exit => {
                ctx.send_viewport_cmd(ViewportCommand::Close);
            }

            // ── 파일 ──
            Command::NewTab => {
                self.tabs.new_untitled();
            }
            Command::OpenFile => self.open_with_dialog(ctx),
            Command::OpenRecent(index) => self.open_recent(ctx, index),
            Command::ClearRecentFiles => self.settings.recent_files.clear(),
            Command::OpenFolder => self.open_folder_with_dialog(),
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
            // 닫기·경로 복사는 탭 바(우클릭 메뉴)와 같은 동작을 활성 탭 기준으로 실행한다.
            Command::CloseTab => self.run_tab_action(ctx, TabAction::Close(active)),
            Command::CloseOtherTabs => self.run_tab_action(ctx, TabAction::CloseOthers(active)),
            Command::CloseTabsToRight => self.run_tab_action(ctx, TabAction::CloseRight(active)),
            Command::CloseSavedTabs => self.run_tab_action(ctx, TabAction::CloseSaved),
            Command::CloseAllTabs => self.run_tab_action(ctx, TabAction::CloseAll),
            Command::CopyTabPath => self.run_tab_action(ctx, TabAction::CopyPath(active)),
            Command::ShowWindowList => self.dialogs.push_back(Dialog::window_list()),

            // ── 보기 ──
            Command::ToggleSidebar => {
                self.sidebar_open = !self.sidebar_open;
            }
            Command::ToggleWordWrap => {
                self.word_wrap = !self.word_wrap;
            }
            Command::SetTabWidth(width) => {
                if self.tab_width != width {
                    self.tab_width = width;
                    fonts::set_tab_size(&mut self.font_defs, f32::from(width));
                    ctx.set_fonts(self.font_defs.clone());
                }
            }
            Command::ToggleTabSpaces => {
                self.tab_spaces = !self.tab_spaces;
            }
            Command::GoToLine => {
                let tab = self.tabs.active();
                let line = match &tab.viewer {
                    Some(viewer) => viewer.cursor_status(&tab.doc).0,
                    None => editor::cursor_status(&tab.doc).0,
                };
                self.dialogs
                    .push_back(Dialog::go_to_line(tab.id(), tab.doc.line_count(), line));
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
            Command::SetViewMode(mode) => self.set_view_mode(mode),
            Command::ToggleMdPreview => {
                let tab = self.tabs.active_mut();
                if tab.doc_view.is_narrow_split(tab.view_mode) {
                    // 좁은 창의 분할은 모드를 그대로 두고 보이는 쪽만 바꾼다 (넓히면 분할로 복귀)
                    tab.doc_view.toggle_narrow_pane();
                } else if tab.view_mode == ViewMode::Preview {
                    self.set_view_mode(ViewMode::Source);
                } else {
                    self.set_view_mode(ViewMode::Preview);
                }
            }
            Command::ToggleMdSplit => {
                if self.tabs.active().view_mode == ViewMode::Split {
                    self.set_view_mode(ViewMode::Source);
                } else {
                    self.set_view_mode(ViewMode::Split);
                }
            }
            Command::ToggleScrollSync => {
                self.scroll_sync = !self.scroll_sync;
            }
            Command::ToggleHighlightWholeWord => {
                self.highlight.whole_word = !self.highlight.whole_word;
            }
            Command::ToggleHighlightMatchCase => {
                self.highlight.match_case = !self.highlight.match_case;
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
            TabAction::SetViewMode(mode) => self.set_view_mode(mode),
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
            if tab.needs_save() {
                let title = tab.doc.title();
                let deleted = tab.disk_deleted;
                // 무엇을 묻는지 보이도록 그 탭으로 이동한다.
                self.tabs.activate(index);
                self.close_queue.push_front(id);
                self.dialogs.push_back(Dialog::Unsaved {
                    tab_id: id,
                    title,
                    remaining: self.close_queue.len() - 1,
                    deleted,
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
                let result = match self.tabs.index_of(id) {
                    Some(index) => {
                        self.tabs.activate(index);
                        self.save_active(ctx, false)
                    }
                    None => SaveResult::NotSaved,
                };
                match result {
                    SaveResult::Saved => {
                        self.close_queue.retain(|&q| q != id);
                        self.tabs.close_ids(&[id]);
                    }
                    // 손상·표현 불가 문자 경고 중: 탭이 대기열 맨 앞에 남아 있으므로, 경고에서 저장하면
                    // 수정되지 않은 탭으로 닫히고 대기열(종료 포함)이 이어진다. 경고를 취소하면 다시 묻는다.
                    SaveResult::Pending => {}
                    // 다른 이름으로 저장을 취소했거나 저장에 실패하면 닫지 않고 멈춘다.
                    SaveResult::NotSaved => self.cancel_close(),
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
            .filter(|t| t.needs_save())
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

    /// 폴더 선택 대화상자로 사이드바에 폴더를 연다.
    fn open_folder_with_dialog(&mut self) {
        let mut picker = rfd::FileDialog::new().set_title("폴더 열기");
        if let Some(dir) = self
            .sidebar
            .folder()
            .map(Path::to_path_buf)
            .or_else(|| self.dialog_directory())
        {
            picker = picker.set_directory(dir);
        }
        if let Some(folder) = picker.pick_folder() {
            self.open_folder(&folder);
        }
    }

    /// 사이드바에 폴더를 열고 사이드바를 펼친다.
    pub fn open_folder(&mut self, path: &Path) {
        match self.sidebar.open_folder(path) {
            Ok(()) => self.sidebar_open = true,
            Err(err) => self.dialogs.push_back(Dialog::message(
                "폴더를 열 수 없습니다",
                format!("{}\n{err}", path.display()),
            )),
        }
    }

    /// 경로가 폴더면 사이드바에, 파일이면 탭으로 연다 (명령줄 인자).
    pub fn open_any(&mut self, ctx: &egui::Context, path: &Path) {
        if path.is_dir() {
            self.open_folder(path);
        } else {
            self.open_path(ctx, path);
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
                self.settings.add_recent(path);
                if let Some(tab) = self.tabs.get_mut(index) {
                    // 5MB 초과는 뷰어, 1~5MB는 안내 막대 (ADR 0001)
                    tab.apply_open_mode(mode);
                    // 지난번에 이 파일에서 쓴 보기 모드 (P3-06)
                    if let Some(view_mode) = self.settings.view_mode_for(path) {
                        tab.view_mode = view_mode;
                    }
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
    /// 읽기 전용으로 보는 문서(뷰어·바이너리)는 수정되지 않았으면 다시 쓰지 않는다 — 다른 이름으로 저장만 한다.
    fn save_active(&mut self, ctx: &egui::Context, save_as: bool) -> SaveResult {
        let tab = self.tabs.active();
        if !save_as && !tab.doc.is_modified() && (tab.viewer.is_some() || tab.doc.is_readonly()) {
            self.show_notice(
                ctx,
                "읽기 전용으로 보는 문서라 저장하지 않았습니다. 사본은 다른 이름으로 저장할 수 있습니다.",
            );
            return SaveResult::NotSaved;
        }
        let file_readonly = tab.file_readonly;
        let has_path = tab.doc.path().is_some();
        if file_readonly && !save_as {
            self.show_notice(ctx, "읽기 전용 파일이라 다른 이름으로 저장합니다.");
        }
        let target = if save_as || !has_path || file_readonly {
            let Some(path) = self.save_dialog() else {
                return SaveResult::NotSaved;
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
                return SaveResult::NotSaved;
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
    fn save_tab(
        &mut self,
        ctx: &egui::Context,
        tab_id: u64,
        target: Option<PathBuf>,
        allow_damaged: bool,
    ) -> SaveResult {
        let Some(index) = self.tabs.index_of(tab_id) else {
            return SaveResult::NotSaved;
        };
        self.tabs.activate(index);
        let tab = self.tabs.active_mut();
        if tab.doc.is_damaged() && !allow_damaged {
            self.dialogs
                .push_back(Dialog::DamagedSave { tab_id, target });
            return SaveResult::Pending;
        }

        let eol_notice = tab.doc.needs_line_ending_conversion_notice();
        let line_ending = tab.doc.line_ending().short_label();
        match file_io::save_document(&mut tab.doc, target.as_deref()) {
            Ok(()) => {
                if let Some(new_path) = &target {
                    tab.refresh_path_key();
                    tab.file_readonly = false;
                    self.settings.add_recent(new_path);
                }
                // 저장으로 디스크 파일이 다시 생겼고 디스크 버전도 지금 내용과 같아졌다.
                tab.disk_deleted = false;
                tab.dismissed_disk = None;
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
                SaveResult::Saved
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
                SaveResult::Pending
            }
            Err(SaveError::NoPath) => self.save_active(ctx, true),
            Err(SaveError::Io(err)) => {
                self.dialogs.push_back(Dialog::message(
                    "저장하지 못했습니다",
                    format!("{err}\n원본 파일은 바뀌지 않았습니다."),
                ));
                SaveResult::NotSaved
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

    // ── 외부 변경 감지 (P2-07) ──

    /// 창이 포커스를 다시 얻으면 열린 파일의 디스크 상태를 확인한다.
    fn check_external_changes(&mut self, ctx: &egui::Context) {
        let focused = ctx.input(|i| i.focused);
        let gained = focused && !self.window_focused;
        self.window_focused = focused;
        if !gained {
            return;
        }
        for change in self.tabs.check_external_changes() {
            match change {
                ExternalChange::Modified {
                    tab_id,
                    title,
                    conflict,
                    disk,
                } => self.dialogs.push_back(Dialog::ExternalChange {
                    tab_id,
                    title,
                    conflict,
                    disk,
                }),
                ExternalChange::Deleted { title, .. } => self.show_notice(
                    ctx,
                    format!(
                        "\"{title}\" 파일이 디스크에서 삭제되었습니다. 저장하면 다시 만듭니다."
                    ),
                ),
            }
        }
    }

    /// 외부 변경 확인 결과를 처리한다.
    fn resolve_external_change(&mut self, ctx: &egui::Context, outcome: &DialogOutcome) {
        match *outcome {
            DialogOutcome::ReloadFromDisk(tab_id) => {
                let Some(encoding) = self
                    .tabs
                    .index_of(tab_id)
                    .and_then(|i| self.tabs.get(i))
                    .map(|t| t.doc.encoding())
                else {
                    return;
                };
                if self.reload_tab(tab_id, encoding).is_ok() {
                    self.show_notice(ctx, "디스크의 내용으로 다시 불러왔습니다.");
                }
            }
            DialogOutcome::KeepMine { tab_id, disk } => {
                if let Some(tab) = self
                    .tabs
                    .index_of(tab_id)
                    .and_then(|i| self.tabs.get_mut(i))
                {
                    tab.external_pending = false;
                    tab.dismissed_disk = Some(disk);
                }
            }
            _ => {}
        }
    }

    /// 디스크의 파일을 지정한 인코딩으로 다시 해석해 같은 탭에 연다.
    fn reopen_tab(&mut self, ctx: &egui::Context, tab_id: u64, encoding: TextEncoding) {
        if let Ok(damaged) = self.reload_tab(tab_id, encoding) {
            let mut text = format!("{}(으)로 다시 읽었습니다.", encoding.label());
            if damaged {
                text.push_str(" 해석하지 못한 바이트가 있습니다(�).");
            }
            self.show_notice(ctx, text);
        }
    }

    /// 디스크에서 다시 읽어 같은 탭의 문서를 바꾼다. 해석하지 못한 바이트가 있으면 `Ok(true)`.
    /// 읽지 못하면 오류 대화상자를 띄우고 `Err(())`.
    fn reload_tab(&mut self, tab_id: u64, encoding: TextEncoding) -> Result<bool, ()> {
        let Some(tab) = self
            .tabs
            .index_of(tab_id)
            .and_then(|i| self.tabs.get_mut(i))
        else {
            return Err(());
        };
        let Some(path) = tab.doc.path().map(Path::to_path_buf) else {
            return Err(());
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
                Ok(damaged)
            }
            Err(err) => {
                tab.external_pending = false;
                self.dialogs
                    .push_back(Dialog::message("다시 읽지 못했습니다", err.to_string()));
                Err(())
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

    /// 창에 끌어다 놓은 파일을 탭으로 연다. 폴더는 사이드바에 연다 (여러 개면 첫 폴더만, P2-06).
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
        let mut folders = Vec::new();
        for path in dropped {
            if path.is_dir() {
                folders.push(path);
            } else {
                self.open_path(ctx, &path);
            }
        }
        if let Some(folder) = folders.first() {
            self.open_folder(folder);
            if folders.len() > 1 {
                self.show_notice(
                    ctx,
                    format!(
                        "폴더는 하나만 열 수 있어 \"{}\"를 열었습니다.",
                        folder.file_name().unwrap_or_default().to_string_lossy()
                    ),
                );
            }
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
        self.check_external_changes(&ctx);

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
        // 최근 파일: "이름 — 폴더"
        let recent_strings: Vec<String> = self
            .settings
            .recent_files
            .iter()
            .map(|p| {
                let name = p.file_name().unwrap_or_default().to_string_lossy();
                match p.parent() {
                    Some(dir) if !dir.as_os_str().is_empty() => {
                        format!("{name}  —  {}", dir.display())
                    }
                    _ => name.into_owned(),
                }
            })
            .collect();
        let recent_labels: Vec<&str> = recent_strings.iter().map(String::as_str).collect();
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
            tab_width: self.tab_width,
            tab_spaces: self.tab_spaces,
            is_markdown: active.doc.is_markdown(),
            view_mode: active.view_mode,
            scroll_sync: self.scroll_sync,
            highlight_whole_word: self.highlight.whole_word,
            highlight_match_case: self.highlight.match_case,
            theme: self.theme,
            encoding: active.doc.encoding(),
            line_ending: active.doc.line_ending(),
            mixed_line_endings: active.doc.has_mixed_line_endings(),
            recent_files: &recent_labels,
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

        // 5. 좌측 사이드바 (닫혀 있으면 다시 여는 좁은 띠). 패널 폭은 egui가 기억해 닫았다 열어도 유지된다.
        let toggle_shortcut = self
            .keymap
            .format_shortcut_for(Command::ToggleSidebar, &ctx)
            .unwrap_or_default();
        if self.sidebar_open {
            let current = self.tabs.active().doc.path().map(Path::to_path_buf);
            let mut sidebar_action = None;
            let panel = egui::Panel::left("sidebar")
                .resizable(true)
                .default_size(self.settings.sidebar_width)
                .size_range(140.0..=480.0)
                .show(ui, |ui| {
                    sidebar_action = sidebar::show(
                        ui,
                        &mut self.sidebar,
                        &palette,
                        &toggle_shortcut,
                        current.as_deref(),
                    );
                });
            self.settings.sidebar_width = panel.response.rect.width().round();
            match sidebar_action {
                Some(SidebarAction::OpenFile(path)) => self.open_path(&ctx, &path),
                Some(SidebarAction::OpenFolderDialog) => self.open_folder_with_dialog(),
                Some(SidebarAction::Close) => self.sidebar_open = false,
                None => {}
            }
        } else {
            let mut open = false;
            egui::Panel::left("sidebar-rail")
                .resizable(false)
                .exact_size(sidebar::RAIL_WIDTH)
                .show(ui, |ui| open = sidebar::rail(ui, &toggle_shortcut));
            if open {
                self.sidebar_open = true;
            }
        }

        // 6. 본문 영역: 활성 탭 에디터
        let font_size = ui.style().text_styles[&egui::TextStyle::Monospace].size * self.zoom;
        let options = EditorOptions {
            word_wrap: self.word_wrap,
            font: FontId::monospace(font_size),
            highlight: self.highlight,
            tab_width: usize::from(self.tab_width),
            tab_spaces: self.tab_spaces,
        };
        let zoom = self.zoom;
        let mut switch_to_edit = false;
        let mut switch_to_split = false;
        let mut link_action = None;
        let mut edit_large = None;
        let mut highlight_exceeded = false;
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
                        let output = doc_view::show(
                            ui,
                            tab,
                            &mut self.split_ratio,
                            self.scroll_sync,
                            &options,
                            zoom,
                            &palette,
                        );
                        highlight_exceeded = output.highlight_exceeded;
                        switch_to_split = output.switch_to_split;
                        link_action = output.link;
                    }
                }
            });
        if switch_to_edit {
            self.tabs.active_mut().switch_to_editor();
        }
        if switch_to_split {
            // 미리보기 블록 더블클릭 → 소스로 이동하므로 분할로 바꾼다 (P3-08)
            self.set_view_mode(ViewMode::Split);
        }
        if let Some(action) = link_action {
            self.handle_link(&ctx, action);
        }
        if highlight_exceeded {
            self.show_notice(
                &ctx,
                format!(
                    "같은 단어가 {}개를 넘어 강조하지 않습니다",
                    self.highlight.max_matches
                ),
            );
        }
        if let Some(dialog) = edit_large {
            self.dialogs.push_back(dialog);
        }

        // 7. 대화상자 (한 번에 하나씩)
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
            Some(
                external @ (DialogOutcome::ReloadFromDisk(_) | DialogOutcome::KeepMine { .. }),
            ) => self.resolve_external_change(&ctx, &external),
            Some(DialogOutcome::GoToLine { tab_id, line }) => {
                if let Some(index) = self.tabs.index_of(tab_id)
                    && let Some(tab) = self.tabs.get_mut(index)
                {
                    match &mut tab.viewer {
                        Some(viewer) => viewer.go_to_line(&tab.doc, line - 1),
                        None => tab.editor.go_to_line(&mut tab.doc, line),
                    }
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

        // 8. 닫기 대기열 (대화상자가 없을 때만 진행)
        self.process_close_queue(&ctx);

        // 9. 끌어다 놓기 안내 (맨 위에 덧그림)
        Self::paint_drop_overlay(&ctx);

        self.update_window_title(&ctx);

        // 10. 설정 자동 저장 (P2-08)
        self.track_window_geometry(&ctx);
        self.autosave_settings(&ctx);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.save_settings_now();
    }
}

/// 두 경로가 같은 파일인지 (실제 경로로 비교, 실패하면 그대로 비교).
fn same_path(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}
